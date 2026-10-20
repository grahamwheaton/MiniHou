//! Python MCP edit vocabulary. Work on a private snapshot; commit only after validation.
use crate::{param_kind, param_meta, Document, ParamKind, Result};
use minihou_core::{nodes, params::Params};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

pub const OPS: &[&str] = &[
    "add",
    "insert",
    "remove",
    "rename",
    "connect",
    "disconnect",
    "set",
    "paint_assign",
    "display",
    "expose",
    "unexpose",
    "move",
    "meta",
    "replace",
    "layout",
    "loop",
    "box_add",
    "box_set",
    "box_remove",
];
mod validation;
mod operations;
pub use operations::apply_ops;
mod layout;
use layout::{connect, layout, rewire};
pub use validation::validate;
use validation::{clean_box, entries, ensure_object, fresh, ids, node, object, order, pos, refs, spec, string, valid_id, validate_param};
