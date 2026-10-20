use super::*;
use openusd::gf::{vec3d, Mat4d as Matrix4d};
use openusd_schemas::geom::{BBoxCache, Purpose, XformCache};

#[derive(Clone, Debug)]
pub struct ReadOptions {
    pub prim_path: String,
    pub purposes: String,
    pub path_attrib: String,
    pub up_axis: String,
    pub apply_units: bool,
    pub time: Option<f64>,
}
impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            prim_path: String::new(),
            purposes: "default render".into(),
            path_attrib: String::new(),
            up_axis: "z".into(),
            apply_units: true,
            time: None,
        }
    }
}

mod values;
use values::*;

mod materials;
use materials::*;

mod instances;
use instances::*;

mod traverse;
pub use traverse::{read_geometry, read_graph};
