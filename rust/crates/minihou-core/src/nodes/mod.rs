//! The nodes this backend can cook. Each one mirrors the Python node of the same name: same parameters and defaults, same
//! output (exact topology and attribute names; floats within rounding). Parameters it does not implement yet are declared
//! `unsupported`, so a graph that sets one fails loudly instead of cooking something different.

use std::sync::{Arc, OnceLock};

use crate::error::{Error, Result};
use crate::geometry::Geometry;
use crate::params::{ParamSpec, Params};

pub mod blender_input;
pub mod image_nodes;
pub mod scene_objects;
mod generate;
mod groups;
mod group;
mod curves;
mod fence;
mod lsystem;
mod copy_to_points;
mod instance;
mod foreach;
mod attribs;
mod attrib_tools;
pub mod attrib_paint;
mod falloff;
mod uv;
mod merge;
pub mod packed;
mod modify;
mod output;
mod model;
pub(crate) mod polyfill;
mod mirror;
mod array;
mod planar_offset;
mod pushpull;
mod boolean;
mod boolean_fracture;
mod reduce;
#[cfg(test)]
mod boolean_parity;
#[cfg(test)]
mod oracle_material_detail;
mod solidify;
mod edge_tools;
mod subdivide;
mod modeling_tools;
mod profiles;
mod bevel;
mod knife;
mod volume;
mod fracture_voronoi;
mod voronoi_points;
mod fracture_tools;
mod ray;
mod occlusion;
pub(crate) mod r#loop;
pub(crate) mod scalar_expr;
mod wrangle;
mod blend;
mod exploded_view;
pub mod usd_nodes;
mod np_random;
mod pcg;
mod procedural_systems;
// time / physics family (docs/RUST_SOLVER_DESIGN.md)
mod sim_common;
mod goals;
pub mod solver;
mod rbd;
pub mod rbd_engine;
mod rbd_fracture_tools;
mod rbd_material_fracture;
mod rbd_interior_detail;
mod convex_decomposition;
mod python_node;
#[cfg(test)]
mod solver_parity;
mod delaunay;
mod surface;
mod capture_deform;
mod contour;
mod heightfield;
mod scatter;
mod scatteralign;
mod primitives;
mod transform;
mod material_tables;
#[cfg(test)]
mod stage1_provenance_tests;
mod dissolve;
mod select_nodes;
mod mesh_common;
mod grid_fill;
mod recalc_normals;
mod tris_to_quads;
mod inset_ext;

pub use modify::{match_size_points, MatchSizeOut};
pub use modeling_tools::{edge_slide_preview, loop_cut_fractions, loop_cut_preview, loop_slide_fractions, LoopRing, LoopTopology, LOOP_CUT_MAX};
pub(crate) use copy_to_points::copy_limit_bytes;

/// One input port.
pub struct InputSpec {
    pub name: &'static str,
    /// Takes any number of connections (Python's `Input(multi=True)`).
    pub multi: bool,
    pub optional: bool,
}

/// What a node's function receives for each input port, in declaration order: the connected geometries (none, one, or for
/// a multi input any number).
pub type Inputs = [Vec<Arc<Geometry>>];

#[derive(Clone, Copy)]
pub struct NodeSpec {
    pub name: &'static str,
    pub inputs: &'static [InputSpec],
    pub params: &'static [ParamSpec],
    pub cook: fn(&Inputs, &Params) -> Result<Geometry>,
}

impl NodeSpec {
    /// The single geometry on input port `i` (an error naming the port when nothing is connected).
    pub fn one(&self, inputs: &Inputs, i: usize) -> Result<Arc<Geometry>> {
        inputs[i].first().cloned().ok_or_else(|| Error::new(format!("input '{}' is not connected", self.inputs[i].name)))
    }
}

const GEO: &[InputSpec] = &[InputSpec { name: "geo", multi: false, optional: false }];

/// Display information supplied by a self-registering package.
pub struct NodeCatalogueEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub keywords: &'static str,
}

mod packages {
    include!(concat!(env!("OUT_DIR"), "/node_packages.rs"));
}

pub fn package_catalogue() -> &'static [NodeCatalogueEntry] { packages::CATALOGUE }

/// Every node type, by name.
pub fn registry() -> &'static [NodeSpec] {
    static LEGACY: &[NodeSpec] = &[
        blender_input::BLENDER_OBJECT,
        blender_input::MODIFIER_INPUT,
        blender_input::OBJECT_INPUT,
        wrangle::WRANGLE,
        blend::BLEND,
        exploded_view::EXPLODED_VIEW,
        usd_nodes::USD_EXPORT,
        usd_nodes::USD_IMPORT,
        r#loop::LOOP_BEGIN,
        r#loop::LOOP_END,
        image_nodes::BLENDER_IMAGE,
        image_nodes::IMAGE_FILE,
        image_nodes::IMAGE_OUT,
        scene_objects::SCENE_OBJECTS,
        generate::GRID,
        generate::BOX,
        generate::CURVE,
        fence::FENCE,
        generate::CIRCLE,
        generate::SPHERE,
        generate::TUBE,
        generate::TORUS,
        volume::SDF_SHAPE,
        volume::SDF_BOOLEAN,
        fracture_voronoi::VORONOI_FRACTURE,
        fracture_voronoi::VORONOI_SPLIT,
        voronoi_points::VORONOI_FRACTURE_POINTS,
        fracture_tools::VORONOI_ADJACENCY,
        ray::RAY,
        occlusion::OCCLUSION,
        modify::TRANSFORM,
        modify::NORMAL,
        modify::NOISE,
        modify::MATCH_SIZE,
        modify::DELETE,
        merge::MERGE,
        lsystem::LSYSTEM,
        curves::CONVERT_LINE,
        curves::POLYPATH,
        curves::CARVE,
        curves::CURVE_SIMPLIFY,
        copy_to_points::COPY_TO_POINTS,
        instance::INSTANCE,
        foreach::FOR_EACH,
        group::GROUP_CREATE,
        select_nodes::GROW,
        select_nodes::SELECT_PATH,
        select_nodes::SELECT_LOOP,
        select_nodes::SELECT_RING,
        grid_fill::GRID_FILL,
        recalc_normals::RECALC_NORMALS,
        tris_to_quads::TRIS_TO_QUADS,
        group::BLAST,
        attrib_tools::ATTRIBUTE_CREATE,
        attrib_paint::ATTRIB_PAINT,
        attrib_tools::ATTRIBUTE_PROMOTE,
        attrib_tools::ATTRIBUTE_BLUR,
        attrib_tools::CONNECTIVITY,
        attrib_tools::SMOOTH,
        attrib_tools::MEASURE,
        falloff::FALLOFF,
        uv::UV_PROJECT,
        uv::UV_FLATTEN,
        uv::UV_TRANSFORM,
        model::DIVIDE,
        model::MIRROR,
        array::ARRAY,
        model::CLIP,
        model::POLYFILL,
        model::POLYEXTRUDE,
        pushpull::INSET,
        pushpull::OUTSET,
        boolean::BOOLEAN,
        boolean_fracture::BOOLEAN_FRACTURE,
        reduce::POLYREDUCE,
        pushpull::PUSHPULL,
        solidify::SOLIDIFY,
        edge_tools::CREASE,
        edge_tools::EDGE_GROUP,
        edge_tools::EDGE_GROUP_PROMOTE,
        edge_tools::POLYBRIDGE,
        edge_tools::DISSOLVE,
        subdivide::SUBDIVIDE,
        modeling_tools::TWIST,
        modeling_tools::LINEAR_TAPER,
        modeling_tools::SQUASH_STRETCH,
        modeling_tools::SOFT_PEAK,
        modeling_tools::GROUP_RANGE,
        modeling_tools::EDGE_COLLAPSE,
        modeling_tools::EDGE_FLIP,
        modeling_tools::EDGE_CUSP,
        modeling_tools::EDGE_SLIDE,
        modeling_tools::POLYSPLIT,
        modeling_tools::LOOP_CUT,
        profiles::SKIN_PROFILES,
        profiles::POLYEXPAND2D,
        bevel::POLYBEVEL,
        knife::KNIFE,
        pcg::PCG_UNION,
        pcg::PCG_DIFFERENCE,
        pcg::PCG_INTERSECTION,
        pcg::PCG_DISTANCE,
        pcg::PCG_COPY_POINTS,
        pcg::PCG_SELF_PRUNE,
        pcg::PCG_PROJECTION,
        pcg::PCG_SPLINE_SAMPLER,
        pcg::PCG_SPAWNER,
        pcg::PCG_VOLUME_SAMPLER,
        pcg::PCG_SURFACE_SAMPLER,
        pcg::PCG_DEBUG,
        procedural_systems::BUILDING_RULES,
        procedural_systems::REGION_IDENTIFY,
        procedural_systems::REGION_TRANSFORM,
        fracture_tools::ASSEMBLE,
        fracture_tools::GLUE_CLUSTER,
        goals::GOAL_LENGTH,
        goals::GOAL_COLLISION,
        goals::GOAL_SURFACE,
        goals::GOAL_GRAVITY,
        goals::GOAL_PIN,
        goals::GOAL_SOLVE,
        solver::SOLVER,
        rbd::RBD_CONFIGURE,
        rbd::RBD_CONSTRAINT,
        rbd::RBD_SOLVER,
        rbd_fracture_tools::RBD_CONSTRAINT_PROPERTIES,
        rbd_material_fracture::RBD_MATERIAL_FRACTURE,
        convex_decomposition::CONVEX_DECOMPOSITION,
        python_node::PYTHON,
        NodeSpec { name: "lod_bake", inputs: GEO, params: crate::params!("cluster_size": 128; "error_px": 1.0; "enabled": true; "min_triangles": 50000), cook: lod_bake },
        NodeSpec { name: "null", inputs: GEO, params: crate::params!("label": ""), cook: pass_through },
        delaunay::DELAUNAY,
        surface::BRIDGE,
        surface::REVOLVE,
        curves::SWEEP,
        surface::SKIN,
        capture_deform::LATTICE,
        capture_deform::LATTICE_GRID,
        curves::PATH_DEFORM,
        capture_deform::POINT_DEFORM,
        volume::POINTS_TO_SURFACE,
        contour::CONTOUR,
        volume::REMESH,
        heightfield::HEIGHTFIELD_ERODE,
        scatter::SCATTER,
        scatteralign::SCATTERALIGN,
        attrib_tools::SORT,
        merge::SWITCH,
        primitives::FUSE,
        capture_deform::SOFT_TRANSFORM,
        transform::COPY_TRANSFORM,
        attrib_tools::ATTRIBUTE_RANDOMIZE,
        attrib_tools::ATTRIBUTE_TRANSFER,
        attrib_tools::COLOR,
        surface::CONVEX_HULL,
        contour::SLICE,
        dissolve::DISSOLVE,
        output::OUTPUT,
        output::INPUT,
    ];
    static MERGED: OnceLock<Vec<NodeSpec>> = OnceLock::new();
    MERGED.get_or_init(|| {
        let mut nodes = LEGACY.to_vec();
        for (spec, info) in packages::SPECS.iter().zip(packages::CATALOGUE) {
            assert_eq!(spec.name, info.id, "package node.rs NODE.name differs from node.toml id");
            assert!(!nodes.iter().any(|n| n.name == spec.name), "duplicate node id: {}", spec.name);
            nodes.push(*spec);
        }
        nodes
    })
}

pub fn get(name: &str) -> Option<&'static NodeSpec> {
    registry().iter().find(|n| n.name == name)
}

#[cfg(test)]
mod package_registry_tests {
    use super::*;
    #[test]
    fn legacy_and_package_nodes_are_registered() {
        assert!(get("box").is_some());
        assert!(get("transform").is_some());
        assert!(get("example_offset").is_some());
        assert_eq!(package_catalogue().iter().map(|n| n.id).collect::<Vec<_>>(), ["attribute_delete", "attribute_rename", "bend", "bezier", "bound", "boundary_select", "bulge", "cache", "clean", "example_offset", "facet", "fillet", "flood_fill", "fractal", "group_expand", "group_promote", "helix", "line", "loop_flatten", "loop_space", "measure_area", "measure_bounds", "measure_distance", "measure_length", "measure_volume", "pack", "pcg_bounds", "pcg_density_filter", "pcg_points", "pcg_transform", "peak", "point", "point_jitter", "point_relax", "resample", "reverse", "ripple", "select_sharp_edges", "set_material", "subnet", "unpack", "waves"]);
    }
}

/// Node types whose result depends on the current frame whatever their parameters (Python: `key_extra` uses `timectx.key()`).
pub const TIME_DEPENDENT: &[&str] = &["solver", "rbd_solver"];

/// The node's cache key must include the frame: its type always depends on time, or a string parameter uses
/// `@Frame`/`@Time`/`@TimeInc` (or `animated` is on). See [`crate::timectx`].
pub fn is_time_dependent(node_type: &str, params: Option<&serde_json::Map<String, serde_json::Value>>) -> bool {
    TIME_DEPENDENT.contains(&node_type) || node_type == "usd_export"
        || (node_type == "usd_import" && params.is_some_and(|p| p.get("time").and_then(serde_json::Value::as_str) == Some("frame")))
        || params.is_some_and(crate::timectx::params_use_time)
}

fn pass_through(inputs: &Inputs, _p: &Params) -> Result<Geometry> {
    inputs[0].first().map(|g| (**g).clone()).ok_or_else(|| Error::new("input 'geo' is not connected"))
}

fn lod_bake(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let cluster_size=p.i("cluster_size")?;
    let min_triangles=p.i("min_triangles")?;
    let error_px=p.f("error_px")?;
    p.b("enabled")?;
    if !(16..=128).contains(&cluster_size) || min_triangles < 0 || !(error_px as f32).is_finite() || error_px <= 0.0 {
        return Err(Error::new("cluster_size must be 16..128, min_triangles must be nonnegative, and error_px must be positive and finite"));
    }
    pass_through(inputs,p)
}

/// `np.linspace(start, stop, num)` in float64: `i * step + start`, with the last value exactly `stop`.
pub(crate) fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    if num == 0 {
        return Vec::new();
    }
    if num == 1 {
        return vec![start];
    }
    let step = (stop - start) / (num - 1) as f64;
    let mut v: Vec<f64> = (0..num).map(|i| i as f64 * step + start).collect();
    v[num - 1] = stop;
    v
}
