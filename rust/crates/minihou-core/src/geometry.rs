//! Columnar geometry, laid out like `minihou/geometry.py`.
//!
//! * Points carry attributes, `P` (float32 x 3) among them.
//! * Primitives are a CSR topology: `vertex_point` (one entry per corner, the point it uses) and `prim_offsets`
//!   (`nprims + 1` entries, the first 0). `closed` marks polygons (true) and open curves (false); `None` means all closed.
//! * Attributes live in four tables (point, vertex, prim, detail). Arrays are shared (`Arc`) between geometries, so a node
//!   that changes one attribute copies only that one, as in Python.
//!
//! Indices are `i32`, as in Python and Blender: [`MAX_ELEMENTS`] is the limit and building past it is an error.

use std::collections::BTreeMap;
use std::sync::Arc;

use rayon::prelude::*;
use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// Largest point / vertex / primitive count an `i32` index can address.
pub const MAX_ELEMENTS: usize = i32::MAX as usize;

#[derive(Clone, Debug, PartialEq)]
pub enum AttrData {
    F32(Vec<f32>),
    I32(Vec<i32>),
}

/// One attribute: `width` values per element, row by row (`width` 1 is a scalar column).
#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub width: usize,
    pub data: AttrData,
}

impl Attr {
    pub fn f32(width: usize, data: Vec<f32>) -> Self {
        Attr { width, data: AttrData::F32(data) }
    }

    pub fn i32(width: usize, data: Vec<i32>) -> Self {
        Attr { width, data: AttrData::I32(data) }
    }

    pub fn rows(&self) -> usize {
        let n = match &self.data {
            AttrData::F32(v) => v.len(),
            AttrData::I32(v) => v.len(),
        };
        n.checked_div(self.width).unwrap_or(0)
    }

    pub fn as_f32(&self) -> Option<&[f32]> {
        match &self.data {
            AttrData::F32(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<&[i32]> {
        match &self.data {
            AttrData::I32(v) => Some(v),
            _ => None,
        }
    }

    pub fn dtype(&self) -> &'static str {
        match &self.data {
            AttrData::F32(_) => "float32",
            AttrData::I32(_) => "int32",
        }
    }

    /// `n` rows of zeros with this attribute's type and width.
    pub fn zeros_like(&self, n: usize) -> Attr {
        match &self.data {
            AttrData::F32(_) => Attr::f32(self.width, vec![0.0; n * self.width]),
            AttrData::I32(_) => Attr::i32(self.width, vec![0; n * self.width]),
        }
    }
}

pub type AttrTable = BTreeMap<String, Arc<Attr>>;

#[derive(Clone, Debug, PartialEq)]
pub struct Topology {
    pub vertex_point: Vec<i32>,
    pub prim_offsets: Vec<i32>,
    pub closed: Option<Vec<bool>>,
}

impl Default for Topology {
    fn default() -> Self {
        Topology { vertex_point: Vec::new(), prim_offsets: vec![0], closed: None }
    }
}

impl Topology {
    pub fn nprims(&self) -> usize {
        self.prim_offsets.len().saturating_sub(1)
    }

    pub fn nvertices(&self) -> usize {
        self.vertex_point.len()
    }

    pub fn is_closed(&self, prim: usize) -> bool {
        self.closed.as_ref().is_none_or(|c| c[prim])
    }

    pub fn has_open(&self) -> bool {
        self.closed.as_ref().is_some_and(|c| c.iter().any(|&x| !x))
    }

    pub fn closed_mask(&self) -> Vec<bool> {
        self.closed.clone().unwrap_or_else(|| vec![true; self.nprims()])
    }

    /// See [`validate_topology`].
    pub fn validate(&self, npoints: usize) -> Result<()> {
        validate_topology(&self.vertex_point, &self.prim_offsets, self.closed.as_deref(), npoints)
    }
}

/// The structural checks Python's `Topology` makes: offsets start at 0, never decrease and end at the vertex count, `closed`
/// has one entry per primitive, and every corner names a point that exists.
pub fn validate_topology(vertex_point: &[i32], prim_offsets: &[i32], closed: Option<&[bool]>, npoints: usize) -> Result<()> {
    if prim_offsets.first() != Some(&0) {
        return Err(Error::new("prim_offsets must start with 0"));
    }
    if prim_offsets.par_windows(2).any(|w| w[1] < w[0]) {
        return Err(Error::new("prim_offsets must not decrease"));
    }
    if *prim_offsets.last().unwrap() as usize != vertex_point.len() {
        return Err(Error::new("prim_offsets must end at the number of vertices"));
    }
    if closed.is_some_and(|c| c.len() != prim_offsets.len() - 1) {
        return Err(Error::new("closed needs one entry per primitive"));
    }
    if vertex_point.par_iter().any(|&p| p < 0 || p as usize >= npoints) {
        return Err(Error::new(format!("vertex_point has indices outside [0, {npoints})")));
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct Geometry {
    pub npoints: usize,
    pub topology: Arc<Topology>,
    pub point: AttrTable,
    pub vertex: AttrTable,
    pub prim: AttrTable,
    pub detail: Map<String, Value>,
}

impl Geometry {
    /// Points `p` (x, y, z per point) and primitives given as a CSR topology.
    pub fn from_arrays(p: Vec<f32>, vertex_point: Vec<i32>, prim_offsets: Vec<i32>, closed: Option<Vec<bool>>) -> Result<Self> {
        if !p.len().is_multiple_of(3) {
            return Err(Error::new("P must have 3 values per point"));
        }
        let npoints = p.len() / 3;
        if npoints > MAX_ELEMENTS || vertex_point.len() > MAX_ELEMENTS {
            return Err(Error::new(format!("more than {MAX_ELEMENTS} elements")));
        }
        let topology = Topology { vertex_point, prim_offsets, closed };
        topology.validate(npoints)?;
        let mut g = Geometry { npoints, topology: Arc::new(topology), ..Default::default() };
        g.point.insert("P".into(), Arc::new(Attr::f32(3, p)));
        Ok(g)
    }

    pub fn points_only(p: Vec<f32>) -> Result<Self> {
        Self::from_arrays(p, Vec::new(), vec![0], None)
    }

    pub fn p(&self) -> &[f32] {
        self.point.get("P").and_then(|a| a.as_f32()).expect("geometry has no P")
    }

    pub fn set_p(&mut self, p: Vec<f32>) {
        debug_assert_eq!(p.len(), self.npoints * 3);
        self.point.insert("P".into(), Arc::new(Attr::f32(3, p)));
    }

    pub fn nprims(&self) -> usize {
        self.topology.nprims()
    }

    pub fn nvertices(&self) -> usize {
        self.topology.nvertices()
    }

    pub fn table(&self, domain: &str) -> &AttrTable {
        match domain {
            "point" => &self.point,
            "vertex" => &self.vertex,
            _ => &self.prim,
        }
    }

    /// Every attribute has the right number of rows for its domain.
    pub fn validate(&self) -> Result<()> {
        self.topology.validate(self.npoints)?;
        if let Some(bindings) = self.detail.get("material_bindings") {
            if bindings.get("version").and_then(Value::as_u64) != Some(1) {
                return Err(Error::new("material_bindings version must be 1"));
            }
            let slots = bindings.get("slots").and_then(Value::as_array)
                .ok_or_else(|| Error::new("material_bindings slots must be an array"))?;
            if slots.iter().any(|v| !v.is_null() && !v.is_string()) {
                return Err(Error::new("material_bindings slots must be IDs or null"));
            }
            let names = self.detail.get("materials").and_then(Value::as_array)
                .ok_or_else(|| Error::new("material_bindings requires materials"))?;
            if slots.len() != names.len() {
                return Err(Error::new("material_bindings slots must parallel materials"));
            }
        }
        if let Some(a) = self.prim.get("material_index") {
            if a.width != 1 || a.as_i32().is_none() {
                return Err(Error::new("prim material_index must be scalar int32"));
            }
        }
        for (domain, table, n) in [("point", &self.point, self.npoints), ("vertex", &self.vertex, self.nvertices()),
                                   ("prim", &self.prim, self.nprims())] {
            for (name, a) in table {
                if a.rows() != n {
                    return Err(Error::new(format!("{domain} attribute {name:?} has {} rows, expected {n}", a.rows())));
                }
            }
        }
        Ok(())
    }
}
