use std::fmt;

/// A cook or input error, worded for the person using MiniHou (the Python engine shows these as they are).
#[derive(Clone, Debug, PartialEq)]
pub struct Error(pub String);

impl Error {
    pub fn new(msg: impl Into<String>) -> Self {
        Error(msg.into())
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
