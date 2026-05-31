//! API-key authentication: key generation/hashing and the Axum middleware +
//! extractor that turn a bearer token into an authenticated `business_id`.

pub mod api_key;
pub mod middleware;

pub use api_key::{derive_prefix, generate_key, hash_key, GeneratedKey};
pub use middleware::{require_api_key, AuthBusiness};
