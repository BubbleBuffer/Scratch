use serde::{Deserialize, Serialize};
use std::fmt;

/// The one error type of the crate. It serializes (for Tauri commands),
/// displays as a readable message and converts into `String`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VaultError {
    /// A page or block id that does not exist.
    NotFound(String),
    /// An argument or op that cannot be applied (nothing was changed).
    Invalid(String),
    /// File system error.
    Io(String),
    /// SQLite index error.
    Index(String),
    /// Loro CRDT error (corrupt document or internal failure).
    Crdt(String),
}

pub type Result<T> = std::result::Result<T, VaultError>;

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VaultError::NotFound(m) => write!(f, "not found: {m}"),
            VaultError::Invalid(m) => write!(f, "invalid: {m}"),
            VaultError::Io(m) => write!(f, "io error: {m}"),
            VaultError::Index(m) => write!(f, "index error: {m}"),
            VaultError::Crdt(m) => write!(f, "crdt error: {m}"),
        }
    }
}

impl std::error::Error for VaultError {}

impl From<VaultError> for String {
    fn from(e: VaultError) -> String {
        e.to_string()
    }
}

impl From<std::io::Error> for VaultError {
    fn from(e: std::io::Error) -> Self {
        VaultError::Io(e.to_string())
    }
}

impl From<rusqlite::Error> for VaultError {
    fn from(e: rusqlite::Error) -> Self {
        VaultError::Index(e.to_string())
    }
}

impl From<loro::LoroError> for VaultError {
    fn from(e: loro::LoroError) -> Self {
        VaultError::Crdt(e.to_string())
    }
}

impl From<loro::LoroEncodeError> for VaultError {
    fn from(e: loro::LoroEncodeError) -> Self {
        VaultError::Crdt(e.to_string())
    }
}

pub(crate) fn not_found(what: &str, id: &str) -> VaultError {
    VaultError::NotFound(format!("{what} {id}"))
}

pub(crate) fn invalid(msg: impl Into<String>) -> VaultError {
    VaultError::Invalid(msg.into())
}
