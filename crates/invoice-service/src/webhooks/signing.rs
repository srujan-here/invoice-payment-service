//! Webhook signing (Stripe-style).
//!
//! We sign `"{timestamp}.{raw_body}"` with HMAC-SHA256 using the per-endpoint
//! secret. `timestamp` is fresh per delivery attempt; `raw_body` is the exact
//! bytes sent. The receiver recomputes the HMAC, constant-time compares, and
//! rejects timestamps outside a tolerance window (replay protection). The stable
//! `X-Webhook-Id` (the event id) lets receivers dedupe across retries.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Returns the hex-encoded HMAC of `"{timestamp}.{body}"`.
pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// The `X-Webhook-Signature` header value: `t=<unix>,v1=<hex>`.
pub fn signature_header(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    format!("t={timestamp},v1={}", sign(secret, timestamp, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_stable_and_verifiable() {
        let secret = b"whsec_test";
        let body = br#"{"id":"evt_1"}"#;
        let sig = sign(secret, 1_700_000_000, body);
        // Recomputing with the same inputs yields the same signature.
        assert_eq!(sig, sign(secret, 1_700_000_000, body));
        // A different body changes it.
        assert_ne!(sig, sign(secret, 1_700_000_000, br#"{"id":"evt_2"}"#));
        // A different timestamp changes it.
        assert_ne!(sig, sign(secret, 1_700_000_001, body));
        assert!(signature_header(secret, 1_700_000_000, body).starts_with("t=1700000000,v1="));
    }
}
