use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::Utc;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::SignatureEncoding;
use rsa::signature::Signer;
use rsa::RsaPrivateKey;
use sha1::Sha1;
use std::fs;
use tracing::debug;

use crate::config::CloudfrontConfig;

/// A pre-configured CloudFront URL signer. If no key is configured,
/// URLs are returned unsigned (useful for local/dev).
pub struct CloudfrontService {
    cf_url: String,
    expiry_seconds: i64,
    /// Pre-loaded private key, or None if signing is disabled.
    signing_key: Option<(String, RsaPrivateKey)>,
}

impl CloudfrontService {
    pub fn new(cfg: &CloudfrontConfig) -> Result<Self> {
        let signing_key = if cfg.keypair_id.is_empty() {
            None
        } else {
            let key_text = cfg.private_key.trim().to_string();
            let pem = if !key_text.is_empty() {
                debug!("CloudFront: loading private key from config text");
                rebuild_pem(&key_text)
            } else if !cfg.private_key_file.is_empty() {
                debug!("CloudFront: loading private key from file {}", cfg.private_key_file);
                fs::read_to_string(&cfg.private_key_file)
                    .with_context(|| format!("Failed to read CF key file: {}", cfg.private_key_file))?
            } else {
                bail!("CloudFront keypairId is set but no privateKey or privateKeyFile provided");
            };
            let private_key = RsaPrivateKey::from_pkcs8_pem(&pem)
                .or_else(|_| {
                    // Try PKCS#1 PEM as fallback
                    use rsa::pkcs1::DecodeRsaPrivateKey;
                    RsaPrivateKey::from_pkcs1_pem(&pem)
                })
                .context("Failed to parse CloudFront private key")?;
            Some((cfg.keypair_id.clone(), private_key))
        };

        Ok(Self {
            cf_url: cfg.url.trim_end_matches('/').to_string(),
            expiry_seconds: cfg.expiry_seconds,
            signing_key,
        })
    }

    pub fn get_signed_url(&self, s3_path: &str) -> Result<String> {
        let full_url = format!("{}/{}", self.cf_url, s3_path);
        let Some((key_pair_id, private_key)) = &self.signing_key else {
            return Ok(full_url);
        };

        let expires_at = Utc::now().timestamp() + self.expiry_seconds;

        // Build the canned policy JSON
        let policy = format!(
            r#"{{"Statement":[{{"Resource":"{}","Condition":{{"DateLessThan":{{"AWS:EpochTime":{}}}}}}}]}}"#,
            full_url, expires_at
        );

        // Sign the policy with RSA-SHA1 (CloudFront canned-policy format).
        // PKCS1v15 is deterministic so no RNG is needed.
        let signing_key: SigningKey<Sha1> = SigningKey::new(private_key.clone());
        let signature = Signer::sign(&signing_key, policy.as_bytes());

        let encoded_sig = cf_base64(BASE64.encode(signature.to_bytes().as_ref()));

        let signed = format!(
            "{}?Expires={}&Signature={}&Key-Pair-Id={}",
            full_url,
            expires_at,
            encoded_sig,
            key_pair_id
        );

        debug!(
            "Signed CF URL for {}: expires={}",
            s3_path, expires_at
        );
        Ok(signed)
    }
}

/// Re-wrap a raw base64 private key string into a PKCS#8 PEM block.
fn rebuild_pem(raw_b64: &str) -> String {
    // Insert line breaks every 64 chars to produce valid PEM
    let wrapped: String = raw_b64
        .chars()
        .collect::<Vec<_>>()
        .chunks(64)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
        wrapped
    )
}

/// CloudFront uses a slightly different base64 alphabet:
/// replaces `+` with `-`, `/` with `_`, and `=` with `~`.
fn cf_base64(s: String) -> String {
    s.replace('+', "-").replace('/', "_").replace('=', "~")
}
