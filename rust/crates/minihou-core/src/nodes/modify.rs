//! `transform` (`minihou/nodes/transform.py`), `normal` (`normal.py`) and `noise` (`noise.py`).

use std::sync::Arc;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use rayon::prelude::*;

use super::{InputSpec, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, AttrTable, Geometry, Topology};
use crate::params::Params;
use crate::{math, normals, topo, xform};
use super::groups;

mod transform;
use transform::transform;

pub const TRANSFORM: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "transform",
    inputs: GEO,
    params: crate::params!(
        "translate": [0.0, 0.0, 0.0]; "rotate": [0.0, 0.0, 0.0]; "scale": [1.0, 1.0, 1.0]; "uniform_scale": 1.0;
        "pivot": [0.0, 0.0, 0.0]; "axis_rotation": [0.0, 0.0, 0.0];
        "group": "";
        "proportional_enabled": false;
        "proportional_radius": 1.0;
        "proportional_falloff": "smooth";
        "proportional_connected": false;
        "proportional_projected": false;
        "proportional_view_direction": [0.0, 0.0, 1.0];
    ),
    cook: transform,
};

mod noise;
use noise::noise_node;

pub const NOISE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "noise",
    inputs: GEO,
    params: crate::params!(
        "attrib": "P"; "mode": "scalar"; "amplitude": [0.0, 0.0, 1.0]; "frequency": 1.0; "offset": [0.0, 0.0, 0.0];
        "octaves": 1; "lacunarity": 2.0; "gain": 0.5; "seed": 0;
        "group": "";
        "blend": 1.0;
        "class": "point";
        "channel_x": true; "channel_y": true; "channel_z": true;
        "noise_along_vector": false;
        "vector_attribute": "N";
        "operation": "default";
        "range_values": "zero_centered";
        "output_raw": false;
        "enable_remap": false;
        "remap_min": 0.0;
        "remap_max": 1.0;
        "noise_type": "perlin";
        "location_attribute": "P";
        "element_size": 1.0;
        "use_vexpression": false;
        "vexpression": "";
        "animate": false;
        "animation_speed": 1.0;
        "animation_time": 0.0;
        "fractal_type": "standard";
        "warping": false;
        "warp_amplitude": 1.0;
        "warp_frequency": 1.0;
        "clamp_min": false;
        "clamp_max": false;
        "min_value": 0.0;
        "max_value": 1.0;
    ),
    cook: noise_node,
};

pub const MOUNTAIN: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "mountain",
    inputs: GEO,
    params: crate::params!(
        "attrib": "P"; "mode": "scalar"; "amplitude": [0.0, 0.0, 1.0]; "frequency": 1.0; "offset": [0.0, 0.0, 0.0];
        "octaves": 3; "lacunarity": 2.0; "gain": 0.5; "seed": 0;
        "group": "";
        "blend": 1.0;
        "class": "point";
        "channel_x": true; "channel_y": true; "channel_z": true;
        "noise_along_vector": true;
        "vector_attribute": "N";
        "operation": "add";
        "range_values": "zero_centered";
        "output_raw": false;
        "enable_remap": false;
        "remap_min": 0.0;
        "remap_max": 1.0;
        "noise_type": "perlin";
        "location_attribute": "P";
        "element_size": 1.0;
        "use_vexpression": false;
        "vexpression": "";
        "animate": false;
        "animation_speed": 1.0;
        "animation_time": 0.0;
        "fractal_type": "standard";
        "warping": false;
        "warp_amplitude": 1.0;
        "warp_frequency": 1.0;
        "clamp_min": false;
        "clamp_max": false;
        "min_value": 0.0;
        "max_value": 1.0;
    ),
    cook: noise_node,
};

mod match_size;
use match_size::match_size;
pub use match_size::{MatchSizeOut, match_size_points};

pub const MATCH_SIZE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    // Retained for the Python fast path; the package owns the public match_size ID.
    name: "match_size_legacy",
    inputs: &[
        InputSpec { name: "geo", multi: false, optional: false },
        InputSpec { name: "target", multi: false, optional: true },
    ],
    params: crate::params!(
        "translate": true; "center": [0.0, 0.0, 0.0]; "justify_z": "center";
        "scale": false; "size": [1.0, 1.0, 1.0]; "uniform": true;
    ),
    cook: match_size,
};

mod delete;
use delete::delete_node;
pub(super) use delete::keep_points;

pub const DELETE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "delete",
    inputs: GEO,
    params: crate::params!("expr": "@P.z < 0"; "invert": false;),
    cook: delete_node,
};

