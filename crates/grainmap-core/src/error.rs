#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("cannot read {0}: {1}")]
    Read(String, String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;
