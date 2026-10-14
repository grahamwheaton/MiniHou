//! `minihou_rs`: the Rust core as a Python module (build it with `maturin`; see `rust/README.md`).
//!
//! Kernels take contiguous numpy arrays of the exact dtype (the wrappers in `minihou/rust.py` convert), read them without
//! copying, run with the GIL released, and hand their results back as new numpy arrays that own the Rust buffers (no copy).
//! `cook` evaluates a graph JSON with the Rust nodes and returns plain arrays the Python side turns into a `Geometry`.

use std::sync::Arc;

use minihou_core::geometry::{validate_topology, AttrData, AttrTable, Topology};
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

/// Unit point normals `(npoints, 3)` float32 (area-weighted face normals, unused points 0).
#[pyfunction]
fn point_normals<'py>(py: Python<'py>, p: PyReadonlyArray2<'py, f32>, vertex_point: PyReadonlyArray1<'py, i32>,
                      prim_offsets: PyReadonlyArray1<'py, i32>) -> PyResult<Bound<'py, PyAny>> {
    let pp = slice2(&p, 3, "P")?;
    let npoints = pp.len() / 3;
    let (vp, offs) = (slice1(&vertex_point, "vertex_point")?, slice1(&prim_offsets, "prim_offsets")?);
    check(vp, offs, None, npoints)?;
    let n = py.detach(|| normals::point_normals_raw(pp, vp, offs, npoints));
    array(py, n, 3)
}

/// Fractal Perlin noise at each row of `pos` `(n, 3)` float32: `(n,)` float32, as `minihou.nodes._util.fbm3_numpy`.
#[pyfunction]
fn fbm3<'py>(py: Python<'py>, pos: PyReadonlyArray2<'py, f32>, seed: i64, octaves: u32, lacunarity: f64, gain: f64)
             -> PyResult<Bound<'py, PyAny>> {
    let pp = slice2(&pos, 3, "pos")?;
    let n = py.detach(|| noise::fbm3(pp, seed, octaves, lacunarity, gain));
    array(py, n, 0)
}

fn table<'py>(py: Python<'py>, t: &AttrTable) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (name, a) in t {
        let width = if a.width == 1 { 0 } else { a.width };
        let v = match &a.data {
            AttrData::F32(v) => array(py, v.clone(), width)?,
            AttrData::I32(v) => array(py, v.clone(), width)?,
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
    let out = PyDict::new(py);
    out.set_item("npoints", geo.npoints)?;
    let t: &Arc<Topology> = &geo.topology;
    out.set_item("vertex_point", array(py, t.vertex_point.clone(), 0)?)?;
    out.set_item("prim_offsets", array(py, t.prim_offsets.clone(), 0)?)?;
    match &t.closed {
        Some(c) => out.set_item("closed", array(py, c.clone(), 0)?)?,
        None => out.set_item("closed", py.None())?,
    }
    out.set_item("point", table(py, &geo.point)?)?;
    out.set_item("vertex", table(py, &geo.vertex)?)?;
    out.set_item("prim", table(py, &geo.prim)?)?;
    out.set_item("detail_json", serde_json_string(&geo.detail))?;
    Ok(out)
}

fn serde_json_string(m: &serde_json::Map<String, serde_json::Value>) -> String {
    serde_json::Value::Object(m.clone()).to_string()
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
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(edges, m)?)?;
    m.add_function(wrap_pyfunction!(prim_normals, m)?)?;
    m.add_function(wrap_pyfunction!(point_normals, m)?)?;
    m.add_function(wrap_pyfunction!(fbm3, m)?)?;
    m.add_function(wrap_pyfunction!(cook, m)?)?;
    m.add_function(wrap_pyfunction!(node_types, m)?)?;
    m.add_function(wrap_pyfunction!(node_params, m)?)?;
    Ok(())
}
