use std::fmt;

/// Marker of [`Error::waiting`] messages.
pub const WAITING_MARK: &str = "waiting: ";

/// A cook or input error, worded for the person using MiniHou (the Python engine shows these as they are).
#[derive(Clone, Debug, PartialEq)]
pub struct Error(pub String);

impl Error {
    pub fn new(msg: impl Into<String>) -> Self {
        Error(msg.into())
    }

    /// A neutral "waiting for external data" state (no object picked, Blender not connected yet): the node
    /// cooks empty geometry and is reported in `CookStats::waiting`, never as a failed cook.
    pub fn waiting(msg: impl Into<String>) -> Self {
        Error(format!("{WAITING_MARK}{}", msg.into()))
    }

    /// The waiting message (without the marker), or None for a real error.
    pub fn waiting_text(&self) -> Option<&str> {
        self.0.strip_prefix(WAITING_MARK)
    }

    /// The same error with `node 'id' (type): ` in front, as the Python evaluator words node failures.
    pub fn in_node(self, id: &str, node_type: &str) -> Self {
        Error(format!("node '{id}' ({node_type}): {}", self.0))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
