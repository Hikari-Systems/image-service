//! The background pass that finishes variants an upload deferred.
//!
//! `resize.processing: "deferred"` — the shipped default — tells an upload to
//! store the source and return. Until this existed there was nothing to finish
//! the job: only an explicit `POST /api/image/{id}/transcode` ever produced the
//! variants, so in deferred mode they never appeared at all. Callers worked around
//! it by always passing `forceImmediateResize`, which put every derivative on the
//! request's critical path — the thing `?defer=` was added to avoid.
//!
//! ## Running on more than one node
//!
//! Every replica runs this loop, so the claim must be exclusive or two nodes pay
//! for the same transcode and then race each other writing `resized_files`.
//! Exclusivity lives entirely in [`ImageBackend::claim_for_transcode`]'s single
//! `UPDATE … FOR UPDATE SKIP LOCKED` statement; see the note there.
//!
//! The claim is a **lease**, not an in-progress flag. A flag is only correct if
//! whoever sets it always lives to clear it, and on a spot fleet a node can vanish
//! mid-transcode at any moment — which would strand that image as permanently
//! "in progress". A lease that expires needs no cleanup and no operator.
//!
//! ## Pacing
//!
//! `batchSize` is small by default because transcoding competes with live uploads
//! for the same cores; the point of deferring was to keep that work off the
//! request path, not to relocate a stampede. Each pass claims at most `batchSize`
//! and transcodes them **one at a time**, for the same reason `transcode_image`
//! scales sizes sequentially.

use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use tracing::{debug, error, info, warn};

use crate::helpers::transcode::{full_transcode, ResizeSelection, ORIGINAL_KEY};
use crate::config::ResizeConfig;
use crate::models::image::{ExpectedVariants, ImageRecord};
use crate::state::AppState;

/// How long to park an image the sweep claimed but found nothing to do for.
///
/// Only reachable when the row's variant *count* is short but every key the
/// category configures is already present — a shape mismatch rather than missing
/// work. Clearing the lease would re-claim it on the next pass forever, so park it
/// well beyond any plausible pass and log it once.
const NOTHING_TO_DO_PARK_HOURS: i64 = 24;

/// Spawn the sweep loop. Returns immediately; the task runs for the process's life.
pub fn spawn(state: Arc<AppState>) {
    let cfg = state.config.resize.transcode_sweep.clone();
    let interval = Duration::from_secs(cfg.interval_seconds.max(1) as u64);

    info!(
        "transcode sweep: on — every {}s, up to {} image(s) per pass, {}s lease",
        cfg.interval_seconds, cfg.batch_size, cfg.lease_seconds
    );

    tokio::spawn(async move {
        // Offset the first pass so a whole fleet restarting together does not
        // stampede the same rows (they would not collide — SKIP LOCKED sees to
        // that — but they would all wake into the same database at once).
        tokio::time::sleep(interval).await;
        loop {
            if let Err(e) = pass(&state).await {
                // Never fatal: a sweep that fails is a sweep that runs again.
                error!("transcode sweep: pass failed: {e:#}");
            }
            tokio::time::sleep(interval).await;
        }
    });
}

/// Run a single pass and return, for the `server transcode-sweep [n]` subcommand.
///
/// Deliberately **not** gated on `transcodeSweep.enabled`: that flag decides whether
/// every replica runs the loop on its own, which is a different question from whether
/// an operator (or an external scheduler) may ask for one pass now. Gating it would
/// make the obvious "turn the loop off and drive it from cron" setup impossible.
///
/// It is still gated on the backend, because without an exclusive claim a second
/// caller would duplicate the work rather than skip it.
pub async fn run_once(
    state: &AppState,
    batch: Option<u32>,
    max_load: Option<f64>,
) -> anyhow::Result<()> {
    // Yield to the machine before claiming anything. Transcoding is pure CPU and
    // competes directly with the uploads it exists to keep work away from, so on a
    // busy box the right amount of background work is none: skipping costs a minute
    // of latency on a variant nobody is waiting for, while pushing through costs
    // the seller sitting in front of a photo upload right now.
    //
    // Checked BEFORE the claim so a skipped tick takes no lease and leaves the rows
    // free for a quieter node — with two replicas that is a real effect, not a
    // formality.
    if let Some(limit) = max_load {
        match current_load_average() {
            Some(load) if load > limit => {
                info!("transcode sweep: skipping, load {load:.2} is over the {limit:.2} ceiling");
                return Ok(());
            }
            Some(load) => debug!("transcode sweep: load {load:.2} within the {limit:.2} ceiling"),
            // Unreadable /proc/loadavg means we cannot tell, and refusing to work
            // on a machine that might be idle is worse than the throttle not
            // applying. Warn and continue.
            None => warn!("transcode sweep: could not read /proc/loadavg; ignoring the load ceiling"),
        }
    }

    if state.config.image_metadata.storage.trim() != "db" {
        anyhow::bail!(
            "transcode sweep needs imageMetadata.storage = \"db\": only the Postgres \
             backend can claim an image exclusively, so on {:?} two callers would \
             transcode the same image rather than skip it",
            state.config.image_metadata.storage
        );
    }
    let n = batch.unwrap_or(state.config.resize.transcode_sweep.batch_size);
    debug!("transcode sweep: one-shot pass, up to {n} image(s)");
    let done = pass_with(state, n).await?;
    // Silent on an idle pass, and deliberately so: this runs from cron as often
    // as once a minute, and a job that says "nothing to do" 1,440 times a day
    // buries the times it did something. An empty log is the healthy steady state.
    if done > 0 {
        info!("transcode sweep: one-shot pass complete, {done} image(s) processed");
    } else {
        debug!("transcode sweep: one-shot pass complete, nothing pending");
    }
    Ok(())
}

/// The 1-minute load average, or `None` if it cannot be read.
///
/// `/proc` is not namespaced by the container runtime, so this is the **host's**
/// load — which is the number that matters: the ceiling exists to protect the box
/// the transcode shares with every other container, not this process's own view.
///
/// The 1-minute figure rather than 5 or 15, because the thing being avoided is a
/// burst of uploads happening *now*; a longer window would keep throttling for
/// minutes after the burst passed, and would react too late when one began.
fn current_load_average() -> Option<f64> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// One pass at the configured batch size.
async fn pass(state: &AppState) -> anyhow::Result<()> {
    pass_with(state, state.config.resize.transcode_sweep.batch_size).await?;
    Ok(())
}

/// One pass: claim up to `batch`, complete each image's missing variants. Returns
/// how many were claimed, so the one-shot entry point can report it.
async fn pass_with(state: &AppState, batch: u32) -> anyhow::Result<usize> {
    let cfg = &state.config.resize.transcode_sweep;
    let (expected, default_expected) = expected_variants(&state.config.resize);

    let claimed = state
        .backend
        .claim_for_transcode(
            batch.max(1) as i64,
            cfg.lease_seconds.max(1) as i64,
            &expected,
            default_expected,
        )
        .await?;

    if claimed.is_empty() {
        debug!("transcode sweep: nothing pending");
        return Ok(0);
    }
    let claimed_count = claimed.len();

    info!("transcode sweep: claimed {} image(s)", claimed.len());
    for image in claimed {
        let Some(id) = image.id else { continue };
        match missing_variants(&state.config.resize, &image) {
            m if m.is_empty() => {
                warn!(
                    "transcode sweep: {id} was claimed but has every configured variant \
                     — parking it for {NOTHING_TO_DO_PARK_HOURS}h"
                );
                let park = Utc::now() + ChronoDuration::hours(NOTHING_TO_DO_PARK_HOURS);
                let _ = state.backend.set_resize_lease(id, Some(park)).await;
            }
            missing => {
                info!("transcode sweep: {id} completing {missing:?}");
                // An explicit allowlist, so the pass produces exactly what is
                // absent and never redoes work an earlier pass already paid for.
                let selection = ResizeSelection::parse(Some(&missing.join(",")), None, false);
                match full_transcode(image.clone(), &selection, state).await {
                    Ok(_) => {
                        // Clear the lease rather than parking it: the row should now
                        // satisfy the completeness test and stop being claimed on its
                        // own. If it does not, that is a real gap worth seeing again.
                        if let Err(e) = state.backend.set_resize_lease(id, None).await {
                            warn!("transcode sweep: {id} done but lease not cleared: {e:#}");
                        } else {
                            info!("transcode sweep: {id} done");
                        }
                    }
                    Err(e) => {
                        // Leave the lease alone — the claim already set it to
                        // now + leaseSeconds, so it doubles as the retry backoff.
                        error!("transcode sweep: {id} failed, will retry after lease: {e:#}");
                    }
                }
            }
        }
    }
    Ok(claimed_count)
}

/// The (category, expected-size-count) pairs the claim query judges against, plus
/// the count for a category with no configured scaling set.
///
/// These are **size keys only, and deliberately exclude the original**: the query
/// compares them against `jsonb_array_length(resized_files)`, and the original is
/// not a size key — it lives in its own `original_s3_path` column and is tested by
/// its own `IS NULL` clause. Counting it here would make every fully-transcoded
/// row look one short and re-claim it forever.
fn expected_variants(resize: &ResizeConfig) -> (Vec<ExpectedVariants>, i32) {
    let expected = resize
        .scaling_sets
        .keys()
        .map(|category| ExpectedVariants {
            category: category.to_lowercase(),
            count: resize.size_keys_for_category(&category.to_lowercase()).len() as i32,
        })
        .collect();
    let default_expected = resize.size_keys_for_category("").len() as i32;
    (expected, default_expected)
}

/// Which variants this image still lacks: the category's size keys that have no
/// entry in `resized_files`, plus `original` when it was never produced.
fn missing_variants(resize: &ResizeConfig, image: &ImageRecord) -> Vec<String> {
    let category = image
        .category
        .as_deref()
        .filter(|c| !c.is_empty())
        .unwrap_or("image")
        .to_lowercase();

    let present: Vec<&str> = image
        .resized_files
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|s| s.size.as_str())
        .collect();

    let mut missing: Vec<String> = resize
        .size_keys_for_category(&category)
        .into_iter()
        .filter(|k| !present.iter().any(|p| p == k))
        .collect();

    if image.original_s3_path.is_none() {
        missing.push(ORIGINAL_KEY.to_string());
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::image::ScaledImage;

    fn resize_config(size_keys: &str, sets: &[(&str, &str)]) -> ResizeConfig {
        ResizeConfig {
            size_keys: size_keys.to_string(),
            scaling_sets: sets
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    fn image(category: &str, sizes: &[&str], original: bool) -> ImageRecord {
        ImageRecord {
            id: Some(uuid::Uuid::nil()),
            category: Some(category.to_string()),
            downloaded_s3_path: Some("cat-id.jpg".to_string()),
            original_s3_path: original.then(|| "cat-id-original.jpg".to_string()),
            resized_files: Some(
                sizes
                    .iter()
                    .map(|s| ScaledImage { size: s.to_string(), s3_path: format!("p-{s}") })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    /// The case the sell chat creates on every upload: all display sizes baked,
    /// `?defer=original`. The sweep must ask for the original and nothing else.
    #[test]
    fn a_deferred_original_is_the_only_thing_left_to_do() {
        let cfg = resize_config("small,medium", &[("auctionphoto", "auctionThumbnail,auctionMedium,auctionLarge")]);
        let img = image(
            "auctionPhoto",
            &["auctionThumbnail", "auctionMedium", "auctionLarge"],
            false,
        );
        assert_eq!(missing_variants(&cfg, &img), vec![ORIGINAL_KEY.to_string()]);
    }

    /// A fully transcoded image must come back empty, or the sweep would rebuild
    /// work it already paid for on every pass.
    #[test]
    fn a_complete_image_has_nothing_missing() {
        let cfg = resize_config("small,medium", &[("auctionphoto", "auctionThumbnail,auctionMedium")]);
        let img = image("auctionPhoto", &["auctionThumbnail", "auctionMedium"], true);
        assert!(missing_variants(&cfg, &img).is_empty());
    }

    /// Category lookup is case-insensitive — categories are lower-cased before the
    /// scaling set is resolved, so `auctionPhoto` must find `auctionphoto`.
    #[test]
    fn the_category_is_matched_case_insensitively() {
        let cfg = resize_config("small", &[("auctionphoto", "auctionThumbnail,auctionMedium")]);
        let img = image("AuctionPhoto", &["auctionThumbnail"], true);
        assert_eq!(missing_variants(&cfg, &img), vec!["auctionMedium".to_string()]);
    }

    /// An unconfigured category falls back to the global `sizeKeys`.
    #[test]
    fn an_unknown_category_falls_back_to_the_global_size_keys() {
        let cfg = resize_config("small,medium", &[]);
        let img = image("somethingElse", &["small"], true);
        assert_eq!(missing_variants(&cfg, &img), vec!["medium".to_string()]);
    }

    /// The counts handed to the claim query are size keys only. Including the
    /// original would leave every finished row one short and re-claim it forever.
    #[test]
    fn expected_counts_exclude_the_original() {
        let cfg = resize_config("small,medium", &[("auctionphoto", "auctionThumbnail,auctionMedium,auctionLarge")]);
        let (expected, default_expected) = expected_variants(&cfg);
        assert_eq!(default_expected, 2, "global sizeKeys has two entries");
        let ap = expected.iter().find(|e| e.category == "auctionphoto").unwrap();
        assert_eq!(ap.count, 3, "three size keys, original not counted");
    }
}
