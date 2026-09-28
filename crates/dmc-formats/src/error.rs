use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FormatError {
    #[error("read of {len} bytes at 0x{offset:x} runs past the end (size 0x{size:x})")]
    OutOfBounds {
        offset: usize,
        len: usize,
        size: usize,
    },

    #[error("bad magic: {0}")]
    BadMagic(String),

    #[error("invalid {what}: {detail}")]
    Invalid { what: &'static str, detail: String },
}

impl FormatError {
    pub fn invalid(what: &'static str, detail: impl Into<String>) -> Self {
        Self::Invalid {
            what,
            detail: detail.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, FormatError>;
