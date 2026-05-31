//! Shared building blocks for the invoice service and the mock PSP.
//!
//! Nothing here knows about HTTP routing or business rules — it is the DRY
//! substrate both binaries depend on: the money type that makes floats
//! unrepresentable, the single error envelope, id generation, config loading,
//! and cursor pagination.

pub mod config;
pub mod error;
pub mod ids;
pub mod money;
pub mod pagination;

pub use error::{AppError, AppResult, ErrorBody};
pub use money::Money;
