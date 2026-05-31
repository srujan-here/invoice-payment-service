//! Webhook delivery: HMAC signing and the background dispatcher that drains the
//! outbox without ever blocking the API request path.

pub mod dispatcher;
pub mod signing;
