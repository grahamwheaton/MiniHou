//! MiniHou's Rust core: the first steps of a native compute backend (see `rust/README.md` and `RustPlan.md`).
//!
//! * [`geometry`]: columnar geometry with the same layout as `minihou/geometry.py`.
//! * [`topo`], [`normals`], [`noise`]: kernels (edges, face/point normals, fractal noise) that match the Python results.
//! * [`nodes`]: a handful of nodes (grid, box, transform, normal, noise, merge, null, output).
//! * [`graph`]: reads MiniHou graph JSON and cooks it.
//! * [`session`]: a persistent evaluator: edit a parameter, recook only what changed.
//!
//! The Python engine stays the reference: `tests/test_rust_parity.py` cooks the same graphs in both and compares.

pub mod error;
mod boolean_kernel;
pub mod geometry;
pub mod graph;
pub mod group_pattern;
pub mod channels;
pub mod controller;
pub mod coplanar;
pub mod heightfield;
pub mod lod;
pub mod limits;
pub mod loop_topology;
pub mod math;
pub mod noise;
pub mod nodes;
pub mod normals;
pub mod params;
pub mod preview;
pub mod python_bridge;
pub mod session;
pub mod sdf_mesh;
pub mod sdf_nets;
pub mod sdf_field;
pub mod select_topo;
pub mod timectx;
pub mod topo;
pub mod xform;
pub mod usd_io;

pub use error::{Error, Result};
pub use geometry::{Attr, AttrData, Geometry, Topology};
pub use graph::Graph;
