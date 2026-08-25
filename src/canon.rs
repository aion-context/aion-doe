//! Canonicalization and digests. Every digest is taken over JCS (RFC 8785)
//! bytes, so key order and whitespace can never reach the gate.

use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn canonicalize(raw: &[u8]) -> Result<Value> {
    let value: Value = serde_json::from_slice(raw).context("upstream payload is not valid JSON")?;
    let canonical = aion_context::jcs::canonicalize_json_bytes(&serde_json::to_vec(&value)?)
        .map_err(|e| anyhow::anyhow!("JCS canonicalization failed: {e}"))?;
    Ok(serde_json::from_slice(&canonical)?)
}

pub fn canonical_bytes(value: &Value) -> Result<Vec<u8>> {
    aion_context::jcs::canonicalize_json_bytes(&serde_json::to_vec(value)?)
        .map_err(|e| anyhow::anyhow!("JCS canonicalization failed: {e}"))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn digest_value(value: &Value) -> Result<String> {
    Ok(sha256_hex(&canonical_bytes(value)?))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut acc, b| {
        use std::fmt::Write as _;
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

pub fn from_hex(text: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(text.len() % 2 == 0, "hex string has an odd length");
    (0..text.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&text[i..i + 2], 16)
                .map_err(|e| anyhow::anyhow!("invalid hex at byte {}: {e}", i / 2))
        })
        .collect()
}

/// JCS serializes numbers with ECMAScript semantics, so an integer above 2^53
/// silently rounds. Sources are checked before signing rather than after a
/// digest has already been corrupted.
pub fn unsafe_integers(value: &Value) -> Vec<String> {
    fn walk(value: &Value, path: &str, out: &mut Vec<String>) {
        match value {
            Value::Number(n) => {
                let over = n
                    .as_u64()
                    .map(|v| v > (1u64 << 53))
                    .or_else(|| n.as_i64().map(|v| v.unsigned_abs() > (1u64 << 53)))
                    .unwrap_or(false);
                if over {
                    out.push(format!("{path} = {n}"));
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    walk(item, &format!("{path}[{i}]"), out);
                }
            }
            Value::Object(map) => {
                for (key, item) in map {
                    walk(item, &format!("{path}.{key}"), out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, "$", &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonicalization_absorbs_key_order_and_whitespace() {
        let a = canonicalize(br#"{"b":1,  "a":  2}"#).unwrap();
        let b = canonicalize(br#"{"a":2,"b":1}"#).unwrap();
        assert_eq!(digest_value(&a).unwrap(), digest_value(&b).unwrap());
    }

    #[test]
    fn hex_round_trips_and_rejects_malformed_input() {
        let bytes = [0u8, 1, 15, 254, 255];
        assert_eq!(from_hex(&hex(&bytes)).unwrap(), bytes);
        assert!(from_hex("abc").is_err());
        assert!(from_hex("zz").is_err());
    }

    #[test]
    fn html_error_bodies_fail_loudly_rather_than_digesting_to_something() {
        assert!(canonicalize(b"<html>404</html>").is_err());
    }

    #[test]
    fn integers_beyond_jcs_precision_are_found_before_signing() {
        let found = unsafe_integers(&json!({"a": {"file_id": 6_675_964_335_526_256_880u64}}));
        assert_eq!(found.len(), 1);
        assert!(found[0].starts_with("$.a.file_id"));
        assert!(unsafe_integers(&json!({"count": 8127, "id": "34"})).is_empty());
    }
}
