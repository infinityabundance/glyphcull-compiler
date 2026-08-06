//! Errors produced by the atlas generator.

/// An atlas-generation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The font bytes could not be parsed as a font face.
    FontParseFailed,
    /// An option value is out of range.
    InvalidOption(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::FontParseFailed => write!(f, "font parse failed"),
            Error::InvalidOption(what) => write!(f, "invalid atlas option: {what}"),
        }
    }
}

impl std::error::Error for Error {}
