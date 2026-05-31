//! Business logic. Services own validation, the invoice state machine, and
//! transaction orchestration (including writing the webhook outbox atomically
//! with the state change). They depend on repositories, never on SQL strings.

pub mod customer_service;
pub mod invoice_service;
pub mod payment_service;
pub mod webhook_service;
