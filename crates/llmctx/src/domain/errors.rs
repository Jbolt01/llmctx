//! Domain-specific errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid selection")]
    InvalidSelection,
}
