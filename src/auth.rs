use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use base64::{Engine, engine::general_purpose::STANDARD};
use constant_time_eq::constant_time_eq;
use getrandom::fill;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tracing::warn;

const DUMMY_PASSWORD: &str = "__dummy_password_for_timing_attack_prevention__";

fn sha256_hash(salt: &[u8; 16], password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(password.as_bytes());
    hasher.finalize().into()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

fn verify_sha256_hash(stored_hash: &str, password: &str) -> bool {
    if !stored_hash.starts_with("$sha256$") {
        return false;
    }
    let rest = &stored_hash["$sha256$".len()..];
    let Some((salt_hex, expected_hash_hex)) = rest.split_once('$') else {
        return false;
    };

    let salt_bytes = match decode_hex(salt_hex) {
        Some(b) => b,
        None => return false,
    };
    if salt_bytes.len() != 16 {
        return false;
    }

    let mut salt = [0u8; 16];
    salt.copy_from_slice(&salt_bytes);

    let computed = sha256_hash(&salt, password);
    let expected = match decode_hex(expected_hash_hex) {
        Some(e) => e,
        None => return false,
    };

    constant_time_eq(&computed, &expected)
}

pub fn verify_auth(
    req: &Request<axum::body::Body>,
    users: &HashMap<String, String>,
) -> Option<String> {
    let auth_header = req.headers().get("Authorization")?;
    let auth_str = auth_header.to_str().ok()?;

    let encoded = auth_str.strip_prefix("Basic ")?;
    let bytes = STANDARD.decode(encoded).ok()?;
    let decoded = String::from_utf8(bytes).ok()?;
    let (username, password) = decoded.split_once(':')?;

    if username.is_empty() {
        warn!("Empty username rejected");
        return None;
    }

    if users.is_empty() {
        warn!("No users configured, authentication denied");
        dummy_verify();
        return None;
    }

    if let Some(hash) = users.get(username) {
        if verify_sha256_hash(hash, password) {
            return Some(username.to_string());
        } else {
            warn!("Password mismatch for user: {}", username);
        }
    } else {
        warn!("Unknown user: {}", username);
        dummy_verify();
    }

    None
}

fn dummy_verify() {
    let mut salt = [0u8; 16];
    fill(&mut salt).unwrap();
    let _ = sha256_hash(&salt, DUMMY_PASSWORD);
}

pub fn hash_password(password: &str) -> Result<String, std::convert::Infallible> {
    let mut salt = [0u8; 16];
    fill(&mut salt).unwrap();
    let hash = sha256_hash(&salt, password);
    Ok(format!(
        "$sha256${}${}",
        encode_hex(&salt),
        encode_hex(&hash)
    ))
}

pub fn unauthorized_response() -> axum::response::Response {
    (
        StatusCode::UNAUTHORIZED,
        [("WWW-Authenticate", "Basic realm=\"Git Login\"")],
        "Unauthorized",
    )
        .into_response()
}
