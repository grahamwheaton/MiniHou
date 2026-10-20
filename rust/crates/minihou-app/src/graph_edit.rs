//! Pure document changes used by the network editor.
use minihou_app::Document;
use minihou_core::nodes;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

mod document;
pub use document::validated;
use document::{fresh_id, nodes, nodes_mut};
mod layout;
use layout::position_in_stream;
mod add;
pub use add::*;
mod remove;
pub use remove::*;
mod wires;
pub use wires::*;
mod clipboard;
pub use clipboard::*;
mod node_edit;
pub use node_edit::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod noodle_tests;
