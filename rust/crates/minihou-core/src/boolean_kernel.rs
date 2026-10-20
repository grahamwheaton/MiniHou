//! Symbolic surface intersections, planar cut reconstruction and welding.
//! Port of `minihou/boolean.py`; point and polygon order is deliberately stable.

pub(crate) mod assembly;
pub(crate) mod cuts;
pub(crate) mod intersections;
pub(crate) mod polygons;

pub(crate) use assembly::boolean;
#[allow(unused_imports)]
pub(crate) use cuts::inner_point;
#[allow(unused_imports)]
pub(crate) use intersections::orient3d;
pub(crate) use polygons::{earclip, holes_into};

pub(crate) type V = [f64; 3];
pub(crate) type V2 = [f64; 2];
pub(crate) type Key = [usize; 4];

pub(crate) fn add(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] + b[i])
}
pub(crate) fn sub(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(crate) fn mul(a: V, s: f64) -> V {
    a.map(|v| v * s)
}
pub(crate) fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn len(a: V) -> f64 {
    dot(a, a).sqrt()
}
pub(crate) fn unit(a: V) -> V {
    mul(a, 1. / len(a).max(1e-300))
}
pub(crate) fn basis(n: V) -> (V, V) {
    let u = unit(cross(
        n,
        if n[0].abs() < 0.9 {
            [1., 0., 0.]
        } else {
            [0., 1., 0.]
        },
    ));
    (u, cross(n, u))
}
pub(crate) fn cross2(a: V2, b: V2, c: V2) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
pub(crate) fn area2(poly: &[usize], xy: &[V2]) -> f64 {
    (0..poly.len())
        .map(|k| {
            let a = xy[poly[(k + poly.len() - 1) % poly.len()]];
            let b = xy[poly[k]];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum()
}

#[derive(Clone)]
pub(crate) struct Piece {
    pub v: Vec<usize>,
    pub bary: Vec<V>,
    pub owner: usize,
    pub src: usize,
    pub whole: bool,
}
pub(crate) struct Output {
    pub p: Vec<V>,
    pub pieces: Vec<Piece>,
    pub keys: Vec<Key>,
    pub edge_t: Vec<f64>,
    pub remap: Vec<usize>,
}
pub(crate) struct Dsu(Vec<usize>);
impl Dsu {
    pub fn new(n: usize) -> Self {
        Self((0..n).collect())
    }
    pub fn root(&mut self, i: usize) -> usize {
        if self.0[i] != i {
            self.0[i] = self.root(self.0[i]);
        }
        self.0[i]
    }
    pub fn join(&mut self, a: usize, b: usize) {
        let a = self.root(a);
        let b = self.root(b);
        self.0[a.max(b)] = a.min(b);
    }
}
