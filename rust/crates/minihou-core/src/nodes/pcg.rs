//! PCG nodes: point samplers, point booleans, density tools and spawning (minihou/nodes/pcg.py).

pub(crate) const DEFAULT_EXTENT: f64 = 0.5;

#[allow(unused_imports)]
pub(crate) mod boxes;
pub(crate) mod cloud;
pub(crate) mod mesh;
pub(crate) mod random;

pub use boxes::greedy_independent;
pub(crate) use boxes::{
    compute_aabbs, compute_boxes, covered_fraction, obb_overlap_one, overlapping_pairs,
    OrientedBoxes,
};
pub(crate) use cloud::{cloud, gather_attr, new_points_cloud, read_pcg, NewAttrVal, PcgData};
pub(crate) use mesh::{
    coverage, fan_triangles, is_mesh, mesh_coverage, point_inside_mesh, ray_tri_hit, PARITY_DIRS,
};
pub use random::{euler_xyz_to_matrix, hash32, matrix_to_euler_xyz, position_seeds, random01};
pub(crate) use random::{mat_mul, rotation_from_z_to};

// -- Euler XYZ & 3x3 helpers ----------------------------------------------------------------------------------

#[inline]
pub(crate) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
pub(crate) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
pub(crate) fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

#[inline]
pub(crate) fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = norm(a);
    if l < 1e-20 {
        a
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}

// =============================================================================================================
// NODE IMPLEMENTATIONS
// =============================================================================================================
