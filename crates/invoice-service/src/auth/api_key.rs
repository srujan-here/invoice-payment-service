//! Key format, generation, and hashing.
//!
//! Format: `dpk_<env>_<random>` where `<env>` is `live`|`test` and `<random>`
//! is 36 chars of CSPRNG alphanumerics (~214 bits). We store the SHA-256 of the
//! full key plus a non-secret `prefix` (the lookup handle). SHA-256 — not a slow
//! KDF — is correct because keys are high-entropy: there is nothing to brute
//! force, and the check runs on every request. (Defended in DESIGN.md §5.)

use rand::distributions::Alphanumeric;
use rand::Rng;
use sha2::{Digest, Sha256};

/// Namespace (`dpk_`) + env (`live`/`test`, 4 chars) + `_` = 9, plus the first
/// 8 chars of the random body = a 17-char prefix.
pub const PREFIX_LEN: usize = 17;
const RANDOM_LEN: usize = 36;

pub struct GeneratedKey {
    /// The full secret. Returned to the caller exactly once, never stored.
    pub full: String,
    pub prefix: String,
    pub last_four: String,
    pub key_hash: Vec<u8>,
}

/// Generate a fresh key for the given environment.
pub fn generate_key(env: &str) -> GeneratedKey {
    let env = if env == "live" { "live" } else { "test" };
    let random: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(RANDOM_LEN)
        .map(char::from)
        .collect();
    let full = format!("dpk_{env}_{random}");
    GeneratedKey {
        prefix: derive_prefix(&full).to_string(),
        last_four: full[full.len() - 4..].to_string(),
        key_hash: hash_key(&full),
        full,
    }
}

/// The lookup handle: the first [`PREFIX_LEN`] chars of the key. Deterministic
/// so we can derive it from a presented token and hit the unique index.
pub fn derive_prefix(key: &str) -> &str {
    let end = key
        .char_indices()
        .nth(PREFIX_LEN)
        .map(|(i, _)| i)
        .unwrap_or(key.len());
    &key[..end]
}

/// SHA-256 of the full key.
pub fn hash_key(key: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hasher.finalize().to_vec()
}

/// Constant-time byte comparison, so hash verification doesn't leak via timing.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_is_self_consistent() {
        let k = generate_key("test");
        assert!(k.full.starts_with("dpk_test_"));
        assert_eq!(k.prefix.len(), PREFIX_LEN);
        assert_eq!(derive_prefix(&k.full), k.prefix);
        assert_eq!(hash_key(&k.full), k.key_hash);
        assert!(k.full.ends_with(&k.last_four));
    }

    #[test]
    fn constant_time_eq_works() {
        let h = hash_key("dpk_test_abc");
        assert!(constant_time_eq(&h, &hash_key("dpk_test_abc")));
        assert!(!constant_time_eq(&h, &hash_key("dpk_test_abd")));
        assert!(!constant_time_eq(&h, b"short"));
    }
}
