//! Native USD interchange. OpenUSD's Rust implementation supplies the file formats
//! and composition; this module supplies MiniHou's geometry and metadata conventions.
use crate::{Attr, AttrData, Error, Geometry, Result};
use openusd::{
    sdf::{self, AbstractData, Value as U},
    usd,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

mod reader;
pub use reader::{read_geometry, read_graph, ReadOptions};

pub const UNITS: &[(&str, f64)] = &[
    ("meters", 1.),
    ("centimeters", 0.01),
    ("millimeters", 0.001),
    ("inches", 0.0254),
    ("feet", 0.3048),
];
#[derive(Clone, Debug)]
pub struct WriteOptions {
    pub up_axis: String,
    pub units: String,
    pub root: String,
    pub materials: bool,
    pub overwrite: bool,
    pub graph: Option<Value>,
    pub fps: f64,
}
impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            up_axis: "z".into(),
            units: "meters".into(),
            root: "MiniHou".into(),
            materials: true,
            overwrite: false,
            graph: None,
            fps: 24.,
        }
    }
}
pub type Outputs = BTreeMap<String, Arc<Geometry>>;
pub(crate) fn err(e: impl std::fmt::Display) -> Error {
    Error::new(e.to_string())
}
mod format;
pub use format::valid_name;
use format::*;

mod geometry;
use geometry::geometry;

mod stage;
pub use stage::{export_session, resolve_path, write_animation, write_stage};
