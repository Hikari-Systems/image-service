use actix_web::{web, HttpResponse};
use serde::Serialize;
use tracing::error;

use crate::state::AppState;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/api/category/list", web::get().to(list_categories));
}

#[derive(Serialize)]
struct SizeEntry {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<i32>,
    #[serde(rename = "mimeType")]
    mime_type: String,
}

#[derive(Serialize)]
struct CategoryEntry {
    name: String,
    sizes: Vec<SizeEntry>,
}

async fn list_categories(state: web::Data<AppState>) -> HttpResponse {
    match build_category_list(&state) {
        Ok(cats) => HttpResponse::Ok().json(cats),
        Err(e) => {
            error!("Error building category list: {}", e);
            HttpResponse::InternalServerError().finish()
        }
    }
}

fn build_category_list(state: &AppState) -> anyhow::Result<Vec<CategoryEntry>> {
    let cfg = &state.config;
    let mut categories: Vec<CategoryEntry> = Vec::new();

    // Default category from resize.sizeKeys
    if !cfg.resize.size_keys.is_empty() {
        let sizes = collect_sizes(&cfg.resize.size_keys, cfg);
        if !sizes.is_empty() {
            categories.push(CategoryEntry {
                name: "default".to_string(),
                sizes,
            });
        }
    }

    // Named categories from resize.scalingSets
    for (cat_name, keys_str) in &cfg.resize.scaling_sets {
        if !cat_name.is_empty() && !keys_str.is_empty() {
            let sizes = collect_sizes(keys_str, cfg);
            if !sizes.is_empty() {
                categories.push(CategoryEntry {
                    name: cat_name.clone(),
                    sizes,
                });
            }
        }
    }

    Ok(categories)
}

fn collect_sizes(keys_str: &str, cfg: &crate::config::AppConfig) -> Vec<SizeEntry> {
    keys_str
        .split(',')
        .map(|k| k.trim())
        .filter(|k| !k.is_empty())
        .filter_map(|size_key| {
            let sc = cfg.resize.get_size(size_key)?;
            let mime_type = sc.mime_type.as_deref()?.to_string();
            Some(SizeEntry {
                name: size_key.to_string(),
                width: sc.width,
                height: sc.height,
                mime_type,
            })
        })
        .collect()
}
