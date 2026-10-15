//! `minihou_rs`: the Rust core as a Python module (build it with `maturin`; see `rust/README.md`).
//!
//! Kernels take contiguous numpy arrays of the exact dtype (the wrappers in `minihou/rust.py` convert), read them without
//! copying, run with the GIL released, and hand their results back as new numpy arrays that own the Rust buffers (no copy).
//! `cook` evaluates a graph JSON with the Rust nodes and returns plain arrays the Python side turns into a `Geometry`.

use std::sync::Arc;

use minihou_core::geometry::{validate_topology, AttrData, AttrTable, Geometry, Topology};
use minihou_core::{noise, nodes, normals, topo, Graph};
use numpy::{PyArray1, PyArrayMethods, PyReadonlyArray1, PyReadonlyArray2, PyUntypedArrayMethods};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

fn slice1<'a, T: numpy::Element>(a: &'a PyReadonlyArray1<'_, T>, what: &str) -> PyResult<&'a [T]> {
    a.as_slice().map_err(|_| PyValueError::new_err(format!("{what} must be a contiguous array")))
}

fn slice2<'a, T: numpy::Element>(a: &'a PyReadonlyArray2<'_, T>, width: usize, what: &str) -> PyResult<&'a [T]> {
    if a.shape()[1] != width {
        return Err(PyValueError::new_err(format!("{what} must have {width} columns")));
    }
    a.as_slice().map_err(|_| PyValueError::new_err(format!("{what} must be a contiguous array")))
}

/// Reject a malformed topology before any kernel indexes into it (the kernels trust their input).
fn check(vertex_point: &[i32], prim_offsets: &[i32], closed: Option<&[bool]>, npoints: usize) -> PyResult<()> {
    validate_topology(vertex_point, prim_offsets, closed, npoints).map_err(|e| PyValueError::new_err(e.0))
}

/// A flat buffer as a numpy array of shape `(len / width, width)` (or `(len,)` for width 0), without copying.
fn array<'py, T: numpy::Element>(py: Python<'py>, v: Vec<T>, width: usize) -> PyResult<Bound<'py, PyAny>> {
    let n = v.len();
    let a = PyArray1::from_vec(py, v);
    Ok(match n.checked_div(width) {
        None => a.into_any(),
        Some(rows) => a.reshape([rows, width])?.into_any(),
    })
}

/// Unique edges as `(points (E, 2) int64, of_vertex (V,) int64, count (E,) int64)`, numbered like `minihou.topo.edges`.
#[pyfunction]
#[pyo3(signature = (vertex_point, prim_offsets, closed, npoints))]
fn edges<'py>(py: Python<'py>, vertex_point: PyReadonlyArray1<'py, i32>, prim_offsets: PyReadonlyArray1<'py, i32>,
              closed: Option<PyReadonlyArray1<'py, bool>>, npoints: usize) -> PyResult<Bound<'py, PyAny>> {
    let vp = slice1(&vertex_point, "vertex_point")?;
    let offs = slice1(&prim_offsets, "prim_offsets")?;
    let cl = match &closed { Some(c) => Some(slice1(c, "closed")?), None => None };
    check(vp, offs, cl, npoints)?;
    let e = py.detach(|| topo::edges_raw(vp, offs, cl, npoints));
    Ok((array(py, e.points, 2)?, array(py, e.of_vertex, 0)?, array(py, e.count, 0)?).into_pyobject(py)?.into_any())
}

/// Newell normal per primitive, `(nprims, 3)` float64 (length 2 x area).
#[pyfunction]
fn prim_normals<'py>(py: Python<'py>, p: PyReadonlyArray2<'py, f32>, vertex_point: PyReadonlyArray1<'py, i32>,
                     prim_offsets: PyReadonlyArray1<'py, i32>) -> PyResult<Bound<'py, PyAny>> {
    let pp = slice2(&p, 3, "P")?;
    let (vp, offs) = (slice1(&vertex_point, "vertex_point")?, slice1(&prim_offsets, "prim_offsets")?);
    check(vp, offs, None, pp.len() / 3)?;
    let n = py.detach(|| normals::prim_normals_raw(pp, vp, offs));
    array(py, n.into_iter().flatten().collect::<Vec<f64>>(), 3)
}

/// Unit point normals `(npoints, 3)` float32 (area-weighted face normals, unused points 0). With `point_prims` (the
/// `(start, prims)` arrays from [`point_prims`]) the per-topology table is not rebuilt.
#[pyfunction]
#[pyo3(signature = (p, vertex_point, prim_offsets, point_prims=None))]
fn point_normals<'py>(py: Python<'py>, p: PyReadonlyArray2<'py, f32>, vertex_point: PyReadonlyArray1<'py, i32>,
                      prim_offsets: PyReadonlyArray1<'py, i32>,
                      point_prims: Option<(PyReadonlyArray1<'py, u32>, PyReadonlyArray1<'py, u32>)>) -> PyResult<Bound<'py, PyAny>> {
    let pp = slice2(&p, 3, "P")?;
    let npoints = pp.len() / 3;
    let (vp, offs) = (slice1(&vertex_point, "vertex_point")?, slice1(&prim_offsets, "prim_offsets")?);
    check(vp, offs, None, npoints)?;
    let n = match &point_prims {
        Some((start, prims)) => {
            let (st, pr) = (slice1(start, "start")?, slice1(prims, "prims")?);
            let nprims = offs.len() as u32 - 1;
            if st.len() != npoints + 1 || st.first() != Some(&0) || st.windows(2).any(|w| w[1] < w[0])
                || *st.last().unwrap() as usize != pr.len() || pr.len() != vp.len() || pr.iter().any(|&x| x >= nprims) {
                return Err(PyValueError::new_err("point_prims does not belong to this topology"));
            }
            py.detach(|| normals::point_normals_with(pp, vp, offs, st, pr))
        }
        None => py.detach(|| normals::point_normals_raw(pp, vp, offs, npoints)),
    };
    array(py, n, 3)
}

/// The primitives around each point (CSR, primitive order): `(start u32 (npoints + 1,), prims u32 (nvertices,))`.
#[pyfunction]
fn point_prims<'py>(py: Python<'py>, vertex_point: PyReadonlyArray1<'py, i32>, prim_offsets: PyReadonlyArray1<'py, i32>,
                    npoints: usize) -> PyResult<Bound<'py, PyAny>> {
    let (vp, offs) = (slice1(&vertex_point, "vertex_point")?, slice1(&prim_offsets, "prim_offsets")?);
    check(vp, offs, None, npoints)?;
    let (start, prims) = py.detach(|| normals::point_prims(vp, offs, npoints));
    Ok((array(py, start, 0)?, array(py, prims, 0)?).into_pyobject(py)?.into_any())
}

/// Fractal Perlin noise at each row of `pos` `(n, 3)` float32: `(n,)` float32, as `minihou.nodes._util.fbm3_numpy`.
#[pyfunction]
fn fbm3<'py>(py: Python<'py>, pos: PyReadonlyArray2<'py, f32>, seed: i64, octaves: u32, lacunarity: f64, gain: f64)
             -> PyResult<Bound<'py, PyAny>> {
    let pp = slice2(&pos, 3, "pos")?;
    let n = py.detach(|| noise::fbm3(pp, seed, octaves, lacunarity, gain));
    array(py, n, 0)
}

/// The `noise` node's fused vector pass (`minihou.nodes.noise`): `(n, 3)` float32, `P + noise` when `add`, else the noise.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn noise_vec<'py>(py: Python<'py>, p: PyReadonlyArray2<'py, f32>, frequency: f32, offset: [f32; 3], amplitude: [f32; 3], seed: i64,
                  octaves: u32, lacunarity: f64, gain: f64, vector: bool, add: bool) -> PyResult<Bound<'py, PyAny>> {
    let pp = slice2(&p, 3, "P")?;
    let out = py.detach(|| noise::noise_vec(pp, frequency, offset, amplitude, seed, octaves, lacunarity, gain, vector, add));
    array(py, out, 3)
}

/// `(P - pivot) @ m.T + shift`, float32 `(n, 3)` (the transform node's matrix step).
#[pyfunction]
fn affine<'py>(py: Python<'py>, p: PyReadonlyArray2<'py, f32>, m: [[f32; 3]; 3], pivot: [f32; 3], shift: [f32; 3])
               -> PyResult<Bound<'py, PyAny>> {
    let pp = slice2(&p, 3, "P")?;
    let out = py.detach(|| minihou_core::xform::affine(pp, m, pivot, shift));
    array(py, out, 3)
}

/// `normalize_rows(v @ m.T)` in float32 `(n, 3)` (normals through the transform node).
#[pyfunction]
fn linear_normalized<'py>(py: Python<'py>, v: PyReadonlyArray2<'py, f32>, m: [[f32; 3]; 3]) -> PyResult<Bound<'py, PyAny>> {
    let vv = slice2(&v, 3, "v")?;
    let out = py.detach(|| minihou_core::xform::linear_normalized(vv, m));
    array(py, out, 3)
}

fn table<'py>(py: Python<'py>, t: AttrTable) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (name, a) in t {
        // the arrays move into numpy without a copy unless another geometry of this cook still shares them
        let a = Arc::try_unwrap(a).unwrap_or_else(|shared| (*shared).clone());
        let width = if a.width == 1 { 0 } else { a.width };
        let v = match a.data {
            AttrData::F32(v) => array(py, v, width)?,
            AttrData::I32(v) => array(py, v, width)?,
        };
        d.set_item(name, v)?;
    }
    Ok(d)
}

/// Cook a graph (JSON text) with the Rust nodes. Returns `{"npoints", "vertex_point", "prim_offsets", "closed" (or None),
/// "point", "vertex", "prim" (name -> array), "detail_json"}`; raises RuntimeError with the cook error otherwise.
#[pyfunction]
#[pyo3(signature = (graph_json, node=None))]
fn cook<'py>(py: Python<'py>, graph_json: &str, node: Option<String>) -> PyResult<Bound<'py, PyDict>> {
    let text = graph_json.to_string();
    let geo = py.detach(move || Graph::from_json(&text).and_then(|g| g.cook(node.as_deref())))
        .map_err(|e| PyRuntimeError::new_err(e.0))?;
    // the cook's other results are gone, so this is normally the only reference: take it apart without copying
    let geo = Arc::try_unwrap(geo).unwrap_or_else(|shared| (*shared).clone());
    geometry_dict(py, geo)
}

/// The dict `cook` returns, from a geometry this call owns (moves the arrays into numpy without copying).
fn geometry_dict<'py>(py: Python<'py>, geo: Geometry) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("npoints", geo.npoints)?;
    let t: Topology = Arc::try_unwrap(geo.topology).unwrap_or_else(|shared| (*shared).clone());
    out.set_item("vertex_point", array(py, t.vertex_point, 0)?)?;
    out.set_item("prim_offsets", array(py, t.prim_offsets, 0)?)?;
    match t.closed {
        Some(c) => out.set_item("closed", array(py, c, 0)?)?,
        None => out.set_item("closed", py.None())?,
    }
    out.set_item("point", table(py, geo.point)?)?;
    out.set_item("vertex", table(py, geo.vertex)?)?;
    out.set_item("prim", table(py, geo.prim)?)?;
    out.set_item("detail_json", serde_json_string(&geo.detail))?;
    Ok(out)
}

/// An incremental evaluator (`minihou_core::session`): edit a parameter, cook again, and only the nodes whose recipe
/// changed run. `cook` returns the same dict as the module-level `cook`; the arrays are copies (the cache keeps its own), made
/// with the GIL released.
#[pyclass]
struct Session {
    inner: minihou_core::session::Session,
}

#[pymethods]
impl Session {
    #[new]
    fn new(graph_json: &str) -> PyResult<Self> {
        let inner = minihou_core::session::Session::from_json(graph_json).map_err(|e| PyValueError::new_err(e.0))?;
        Ok(Session { inner })
    }

    /// Set one parameter; `value_json` is the value as JSON text. ValueError for an unknown node or parameter.
    fn set_param(&mut self, node: &str, name: &str, value_json: &str) -> PyResult<()> {
        let value: serde_json::Value = serde_json::from_str(value_json)
            .map_err(|e| PyValueError::new_err(format!("value is not valid JSON: {e}")))?;
        self.inner.set_param(node, name, value).map_err(|e| PyValueError::new_err(e.0))
    }

    fn set_bypass(&mut self, node: &str, flag: bool) -> PyResult<()> {
        self.inner.set_bypass(node, flag).map_err(|e| PyValueError::new_err(e.0))
    }

    #[pyo3(signature = (target=None))]
    fn cook<'py>(&mut self, py: Python<'py>, target: Option<String>) -> PyResult<Bound<'py, PyDict>> {
        let inner = &mut self.inner;
        let geo = py.detach(move || inner.cook(target.as_deref()).map(|g| (*g).clone()))
            .map_err(|e| PyRuntimeError::new_err(e.0))?;
        geometry_dict(py, geo)
    }

    /// `{"cooked": [ids], "cached": [ids], "node_ms": {id: ms}}` of the last cook.
    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let s = self.inner.last_stats();
        let out = PyDict::new(py);
        out.set_item("cooked", s.cooked.clone())?;
        out.set_item("cached", s.cached.clone())?;
        let ms = PyDict::new(py);
        for (id, t) in &s.node_ms {
            ms.set_item(id, *t)?;
        }
        out.set_item("node_ms", ms)?;
        Ok(out)
    }

    fn cache_bytes(&self) -> usize {
        self.inner.cache_bytes()
    }

    /// LRU budget in bytes; `None` or 0 means unlimited.
    #[pyo3(signature = (n=None))]
    fn set_budget_bytes(&mut self, n: Option<usize>) {
        self.inner.set_budget_bytes(n);
    }

    fn clear(&mut self) {
        self.inner.clear();
    }

    /// Results kept per node (default 2: current and previous).
    fn set_keep_versions(&mut self, n: usize) {
        self.inner.set_keep_versions(n);
    }
}

fn serde_json_string(m: &serde_json::Map<String, serde_json::Value>) -> String {
    serde_json::Value::Object(m.clone()).to_string()
}

/// The viewport layout of a big mesh (`minihou.preview`): `(tri u32 (T*3,), tri_prim u32, edge u32 (E*2,), table float32
/// (rows*8,) or None, truncated)`.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn preview_layout<'py>(py: Python<'py>, p: PyReadonlyArray2<'py, f32>, vertex_point: PyReadonlyArray1<'py, i32>,
                       prim_offsets: PyReadonlyArray1<'py, i32>, closed: Option<PyReadonlyArray1<'py, bool>>, max_tris: usize,
                       max_edges: usize, chunk_min: usize, chunk: usize) -> PyResult<Bound<'py, PyAny>> {
    let pp = slice2(&p, 3, "P")?;
    let npoints = pp.len() / 3;
    let (vp, offs) = (slice1(&vertex_point, "vertex_point")?, slice1(&prim_offsets, "prim_offsets")?);
    let cl = match &closed { Some(c) => Some(slice1(c, "closed")?), None => None };
    check(vp, offs, cl, npoints)?;
    if chunk == 0 {
        return Err(PyValueError::new_err("chunk must be at least 1"));
    }
    let lay = py.detach(|| minihou_core::preview::layout(pp, vp, offs, cl, npoints, max_tris, max_edges, chunk_min, chunk));
    let table = match lay.table { Some(t) => array(py, t, 0)?, None => py.None().into_bound(py) };
    Ok((array(py, lay.tri, 0)?, array(py, lay.tri_prim, 0)?, array(py, lay.edge, 0)?, table, lay.truncated)
        .into_pyobject(py)?.into_any())
}

/// The run table for triangles already in drawing order (`preview.chunk_table`): float32 `(rows * 8,)`.
#[pyfunction]
fn chunk_bounds<'py>(py: Python<'py>, p: PyReadonlyArray2<'py, f32>, tri: PyReadonlyArray1<'py, u32>, chunk: usize)
                     -> PyResult<Bound<'py, PyAny>> {
    let pp = slice2(&p, 3, "P")?;
    let t = slice1(&tri, "tri")?;
    let npoints = (pp.len() / 3) as u32;
    if chunk == 0 || t.len() % 3 != 0 || t.iter().any(|&q| q >= npoints) {
        return Err(PyValueError::new_err("tri must be point triples within P, and chunk at least 1"));
    }
    let table = py.detach(|| minihou_core::preview::chunk_bounds(pp, t, chunk));
    array(py, table, 0)
}

/// Per-axis `(min, max)` of `P` `(n, 3)` float32, as two float32 arrays of 3 (numpy's `P.min(0)` / `P.max(0)`, NaN included).
/// Empty `P` gives infinities.
#[pyfunction]
fn bounds<'py>(py: Python<'py>, p: PyReadonlyArray2<'py, f32>) -> PyResult<Bound<'py, PyAny>> {
    let pp = slice2(&p, 3, "P")?;
    let (lo, hi) = py.detach(|| minihou_core::preview::bounds(pp));
    Ok((array(py, lo.to_vec(), 0)?, array(py, hi.to_vec(), 0)?).into_pyobject(py)?.into_any())
}

/// `(fan triangles of the closed polygons, vertices of the open curves, points no primitive uses)`: `proxy.draw_stats`.
#[pyfunction]
fn draw_stats(py: Python<'_>, vertex_point: PyReadonlyArray1<'_, i32>, prim_offsets: PyReadonlyArray1<'_, i32>,
              closed: Option<PyReadonlyArray1<'_, bool>>, npoints: usize) -> PyResult<(u64, u64, u64)> {
    let (vp, offs) = (slice1(&vertex_point, "vertex_point")?, slice1(&prim_offsets, "prim_offsets")?);
    let cl = match &closed { Some(c) => Some(slice1(c, "closed")?), None => None };
    check(vp, offs, cl, npoints)?;
    Ok(py.detach(|| minihou_core::preview::draw_stats(vp, offs, cl, npoints)))
}

/// Node types this build can cook.
fn check_grid(len: usize, dims: [usize; 3]) -> PyResult<()> {
    let size = dims.iter().try_fold(1usize, |n, &d| n.checked_mul(d));
    if size != Some(len) {
        return Err(PyValueError::new_err("grid dimensions do not match values"));
    }
    Ok(())
}

#[pyfunction]
fn sdf_from_mesh<'py>(py: Python<'py>, tris: PyReadonlyArray1<'py, f64>, origin: [f64; 3],
                     dims: [usize; 3], h: f64, band: f64) -> PyResult<Bound<'py, PyAny>> {
    let t = slice1(&tris, "triangles")?;
    let out = py.detach(|| minihou_core::sdf_mesh::from_mesh_with_ties(t, origin, dims, h, band))
        .map_err(PyValueError::new_err)?;
    Ok((array(py, out.values, 0)?, array(py, out.ambiguous.into_iter().map(|i| i as i64).collect(), 0)?)
        .into_pyobject(py)?.into_any())
}

#[pyfunction]
fn sdf_resolve_seed_ties<'py>(py: Python<'py>, tris: PyReadonlyArray1<'py, f64>, points: PyReadonlyArray1<'py, f64>,
                            seeds: PyReadonlyArray1<'py, i64>, distances: PyReadonlyArray1<'py, f64>, band: f64)
                            -> PyResult<Bound<'py, PyAny>> {
    let t = slice1(&tris, "triangles")?;
    let p = slice1(&points, "points")?;
    let s = slice1(&seeds, "seeds")?;
    let d = slice1(&distances, "distances")?;
    if t.is_empty() || !t.len().is_multiple_of(9) || p.len() != s.len() * 3 || s.len() != d.len()
        || s.iter().any(|&i| i < 0 || i as usize >= t.len() / 3) {
        return Err(PyValueError::new_err("invalid nearest-corner seeds"));
    }
    array(py, py.detach(|| minihou_core::sdf_mesh::resolve_seed_ties(t, p, s, d, band)), 0)
}

#[pyfunction]
fn sdf_flood<'py>(py: Python<'py>, free: PyReadonlyArray1<'py, bool>, dims: [usize; 3]) -> PyResult<Bound<'py, PyAny>> {
    let v = slice1(&free, "free")?;
    check_grid(v.len(), dims)?;
    array(py, py.detach(|| minihou_core::sdf_mesh::flood(v, dims)), 0)
}

#[pyfunction]
fn sdf_from_points<'py>(py: Python<'py>, points: PyReadonlyArray1<'py, f64>, radius: PyReadonlyArray1<'py, f64>,
                       h: f64, blend: f64) -> PyResult<Bound<'py, PyAny>> {
    let p = slice1(&points, "points")?;
    let r = slice1(&radius, "radius")?;
    let field = py.detach(|| minihou_core::sdf_field::from_points(p, r, h, blend)).map_err(PyValueError::new_err)?;
    Ok((field.origin, field.dims, array(py, field.values, 0)?).into_pyobject(py)?.into_any())
}

#[pyfunction]
fn sdf_blur<'py>(py: Python<'py>, values: PyReadonlyArray1<'py, f32>, dims: [usize; 3], iterations: usize)
                -> PyResult<Bound<'py, PyAny>> {
    let v = slice1(&values, "values")?;
    check_grid(v.len(), dims)?;
    array(py, py.detach(|| minihou_core::sdf_field::blur(v, dims, iterations)), 0)
}

#[pyfunction]
fn sdf_resample_to<'py>(py: Python<'py>, values: PyReadonlyArray1<'py, f32>, src_dims: [usize; 3], offset: [i64; 3],
                       dims: [usize; 3], fill: f32) -> PyResult<Bound<'py, PyAny>> {
    let v = slice1(&values, "values")?;
    check_grid(v.len(), src_dims)?;
    if dims.iter().try_fold(1usize, |n, &d| n.checked_mul(d)).is_none_or(|n| n > 24_000_000)
        || (0..3).any(|a| offset[a].checked_add(dims[a] as i64).is_none()) {
        return Err(PyValueError::new_err("resampled grid is too large"));
    }
    array(py, py.detach(|| minihou_core::sdf_field::resample_to(v, src_dims, offset, dims, fill)), 0)
}

#[pyfunction]
fn sdf_smooth_min_f32<'py>(py: Python<'py>, a: PyReadonlyArray1<'py, f32>, b: PyReadonlyArray1<'py, f32>, k: f32)
                         -> PyResult<Bound<'py, PyAny>> {
    let a = slice1(&a, "a")?;
    let b = slice1(&b, "b")?;
    if a.len() != b.len() { return Err(PyValueError::new_err("broadcast input sizes differ")); }
    array(py, py.detach(|| minihou_core::sdf_field::smooth_min_f32(a, b, k)), 0)
}

#[pyfunction]
fn sdf_smooth_min_f64<'py>(py: Python<'py>, a: PyReadonlyArray1<'py, f64>, b: PyReadonlyArray1<'py, f64>, k: f64)
                         -> PyResult<Bound<'py, PyAny>> {
    let a = slice1(&a, "a")?;
    let b = slice1(&b, "b")?;
    if a.len() != b.len() { return Err(PyValueError::new_err("broadcast input sizes differ")); }
    array(py, py.detach(|| minihou_core::sdf_field::smooth_min_f64(a, b, k)), 0)
}

#[pyfunction]
fn sdf_smooth_min_f32_k64<'py>(py: Python<'py>, a: PyReadonlyArray1<'py, f64>, b: PyReadonlyArray1<'py, f64>, k: f64)
                             -> PyResult<Bound<'py, PyAny>> {
    let a = slice1(&a, "a")?;
    let b = slice1(&b, "b")?;
    if a.len() != b.len() { return Err(PyValueError::new_err("broadcast input sizes differ")); }
    array(py, py.detach(|| minihou_core::sdf_field::smooth_min_f32_k64(a, b, k)), 0)
}

#[pyfunction]
fn sdf_resolve_face_saddles<'py>(py: Python<'py>, values: PyReadonlyArray1<'py, f64>, dims: [usize; 3], max_rounds: usize)
                               -> PyResult<Bound<'py, PyAny>> {
    let v = slice1(&values, "values")?;
    check_grid(v.len(), dims)?;
    array(py, py.detach(|| minihou_core::sdf_nets::resolve_face_saddles(v, dims, max_rounds)), 0)
}

#[pyfunction]
fn sdf_resolve_face_saddles_f32<'py>(py: Python<'py>, values: PyReadonlyArray1<'py, f32>, dims: [usize; 3], max_rounds: usize)
                                   -> PyResult<Bound<'py, PyAny>> {
    let v = slice1(&values, "values")?;
    check_grid(v.len(), dims)?;
    array(py, py.detach(|| minihou_core::sdf_nets::resolve_face_saddles_f32(v, dims, max_rounds)), 0)
}

#[pyfunction]
fn sdf_surface_nets<'py>(py: Python<'py>, values: PyReadonlyArray1<'py, f64>, dims: [usize; 3], origin: [f64; 3], h: f64)
                        -> PyResult<Bound<'py, PyAny>> {
    let v = slice1(&values, "values")?;
    check_grid(v.len(), dims)?;
    let (p, q) = py.detach(|| minihou_core::sdf_nets::surface_nets(v, dims, origin, h));
    Ok((array(py, p, 3)?, array(py, q, 4)?).into_pyobject(py)?.into_any())
}

/// Node types this build can cook.
#[pyfunction]
fn node_types() -> Vec<&'static str> {
    nodes::registry().iter().map(|n| n.name).collect()
}

/// `{name: [default (JSON text), supported]}` for one node type: compared with the Python registry by the parity tests.
#[pyfunction]
fn node_params<'py>(py: Python<'py>, node_type: &str) -> PyResult<Bound<'py, PyDict>> {
    let spec = nodes::get(node_type).ok_or_else(|| PyValueError::new_err(format!("no node type {node_type:?}")))?;
    let d = PyDict::new(py);
    for p in spec.params {
        d.set_item(p.name, ((p.default)().to_string(), p.supported))?;
    }
    Ok(d)
}

#[pymodule]
fn minihou_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(sdf_from_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(sdf_resolve_seed_ties, m)?)?;
    m.add_function(wrap_pyfunction!(sdf_flood, m)?)?;
    m.add_function(wrap_pyfunction!(sdf_from_points, m)?)?;
    m.add_function(wrap_pyfunction!(sdf_blur, m)?)?;
    m.add_function(wrap_pyfunction!(sdf_resample_to, m)?)?;
    m.add_function(wrap_pyfunction!(sdf_smooth_min_f32, m)?)?;
    m.add_function(wrap_pyfunction!(sdf_smooth_min_f64, m)?)?;
    m.add_function(wrap_pyfunction!(sdf_smooth_min_f32_k64, m)?)?;
    m.add_function(wrap_pyfunction!(sdf_resolve_face_saddles, m)?)?;
    m.add_function(wrap_pyfunction!(sdf_resolve_face_saddles_f32, m)?)?;
    m.add_function(wrap_pyfunction!(sdf_surface_nets, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(edges, m)?)?;
    m.add_function(wrap_pyfunction!(prim_normals, m)?)?;
    m.add_function(wrap_pyfunction!(point_normals, m)?)?;
    m.add_function(wrap_pyfunction!(point_prims, m)?)?;
    m.add_function(wrap_pyfunction!(fbm3, m)?)?;
    m.add_function(wrap_pyfunction!(noise_vec, m)?)?;
    m.add_function(wrap_pyfunction!(affine, m)?)?;
    m.add_function(wrap_pyfunction!(linear_normalized, m)?)?;
    m.add_function(wrap_pyfunction!(cook, m)?)?;
    m.add_class::<Session>()?;
    m.add_function(wrap_pyfunction!(preview_layout, m)?)?;
    m.add_function(wrap_pyfunction!(chunk_bounds, m)?)?;
    m.add_function(wrap_pyfunction!(bounds, m)?)?;
    m.add_function(wrap_pyfunction!(draw_stats, m)?)?;
    m.add_function(wrap_pyfunction!(node_types, m)?)?;
    m.add_function(wrap_pyfunction!(node_params, m)?)?;
    Ok(())
}
