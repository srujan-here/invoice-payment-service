//! Domain entities, enums, and the request/response DTOs that cross the API
//! boundary. No SQL and no HTTP types live here — just the shapes and the
//! invoice state-machine rules.

pub mod api_key;
pub mod customer;
pub mod invoice;
pub mod payment;
pub mod webhook;
