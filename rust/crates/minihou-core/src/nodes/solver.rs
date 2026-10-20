//! Solver: feed each frame's result into the next (Houdini's Solver SOP), ported from `minihou/nodes/solver.py`.
//!
//! `body` (a node type) runs on the previous result once per substep from `start_frame` to the current frame, inside
//! [`crate::timectx::at`] (the substep's frame) and [`crate::timectx::step`] (`@TimeInc`). Deterministic: a frame gives the
//! same result however it is reached. Recent frames are remembered per input geometry (by `Arc` identity, held with a
//! `Weak` so a dead input can not be mistaken for a new one at the same address) and settings; playback steps from the
//! nearest remembered frame at or before the target, and going back before it restarts from the input.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};

use serde_json::Value;

use super::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::Geometry;
use crate::params::Params;
use crate::timectx;

/// Frames remembered per solver input (playback steps from the nearest one).
const MAX_FRAMES: usize = 1000;
/// Inputs remembered at once (least recently used dropped first).
const MAX_MEMOS: usize = 32;

pub const SOLVER: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "solver",
    inputs: GEO,
    params: crate::params!("body": ""; "params": "{}"; "start_frame": 1; "substeps": 1),
    cook: solver,
};

struct Memo {
    input: Weak<Geometry>,
    settings: String,
    frames: BTreeMap<i64, Arc<Geometry>>,
    order: VecDeque<i64>,
    used: u64,
}

struct Memory {
    memos: Vec<Memo>,
    clock: u64,
}

static MEMORY: Mutex<Memory> = Mutex::new(Memory { memos: Vec::new(), clock: 0 });

fn memory() -> std::sync::MutexGuard<'static, Memory> {
    MEMORY.lock().unwrap_or_else(|e| e.into_inner())
}

/// Forget every remembered solver frame (tests, or to free memory).
pub fn clear_memory() {
    memory().memos.clear();
}

fn body_spec(body: &str) -> Result<&'static NodeSpec> {
    let spec = super::get(body).ok_or_else(|| Error::new(format!("no node type '{body}' for the solver body")))?;
    if spec.inputs.is_empty() {
        return Err(Error::new(format!("'{body}' has no geometry input to feed the previous frame into")));
    }
    Ok(spec)
}

/// JSON text with object keys sorted (the settings key).
fn canonical(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys.iter().map(|k| format!("{}:{}", Value::String((*k).clone()), canonical(&m[*k]))).collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

fn solver(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = SOLVER.one(inputs, 0)?;
    let body = p.s("body")?;
    let spec = body_spec(&body)?;
    let text = p.s("params")?;
    let overrides: Value = serde_json::from_str(if text.is_empty() { "{}" } else { &text })
        .map_err(|e| Error::new(format!("params must be JSON: {e}")))?;
    let overrides = overrides.as_object().cloned().unwrap_or_default();
    let values = Params::resolve(spec.params, Some(&overrides)).map_err(|e| Error::new(format!("solver body '{body}': {e}")))?;
    let start_frame = p.i("start_frame")?;
    let substeps = super::sim_common::int(p, "substeps", Some(1), Some(100))?;
    let target = timectx::frame().round_ties_even() as i64;
    if target <= start_frame {
        return Ok((*geo).clone());
    }
    let fps = timectx::fps();
    let settings = format!("{body}|{}|{start_frame}|{substeps}|{fps:?}", canonical(&Value::Object(overrides)));
    let (mut start, mut state) = (start_frame, geo.clone());
    {
        let mut mem = memory();
        mem.clock += 1;
        let clock = mem.clock;
        mem.memos.retain(|m| m.input.strong_count() > 0);
        if let Some(m) = mem.memos.iter_mut().find(|m| m.settings == settings && std::ptr::eq(m.input.as_ptr(), Arc::as_ptr(&geo))) {
            m.used = clock;
            if let Some((&f, g)) = m.frames.range(..=target).next_back() {
                start = f;
                state = g.clone();
            }
        } else {
            mem.memos.push(Memo { input: Arc::downgrade(&geo), settings: settings.clone(), frames: BTreeMap::new(), order: VecDeque::new(), used: clock });
            if mem.memos.len() > MAX_MEMOS {
                let oldest = mem.memos.iter().enumerate().min_by_key(|(_, m)| m.used).map(|(i, _)| i).unwrap();
                mem.memos.remove(oldest);
            }
        }
    }
    if start == target {
        return Ok((*state).clone());
    }
    let dt = 1.0 / (fps * substeps as f64);
    let extra = spec.inputs.len() - 1;
    for f in start + 1..=target {
        for k in 0..substeps {
            let sub = (f - 1) as f64 + (k + 1) as f64 / substeps as f64;
            let mut ins: Vec<Vec<Arc<Geometry>>> = vec![vec![state.clone()]];
            ins.extend((0..extra).map(|_| Vec::new()));
            let next = timectx::at(Some(sub), None, || timectx::step(dt, || (spec.cook)(&ins, &values)))
                .and_then(|g| g.validate().map(|_| g))
                .map_err(|e| Error::new(format!("solver body '{body}' at frame {sub}: {e}")))?;
            state = Arc::new(next);
        }
        let mut mem = memory();
        if let Some(m) = mem.memos.iter_mut().find(|m| m.settings == settings && std::ptr::eq(m.input.as_ptr(), Arc::as_ptr(&geo))) {
            if m.frames.insert(f, state.clone()).is_none() {
                m.order.push_back(f);
            }
            while m.frames.len() > MAX_FRAMES {
                if let Some(old) = m.order.pop_front() {
                    m.frames.remove(&old);
                }
            }
        }
    }
    Ok((*state).clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use crate::session::Session;
    use serde_json::json;

    fn graph(params: Value, start: i64, substeps: i64) -> Value {
        json!({"nodes": {
            "p": {"type": "grid", "params": {"columns": 2, "rows": 2}},
            "s": {"type": "solver", "inputs": {"geo": "p"},
                  "params": {"body": "transform", "params": params.to_string(), "start_frame": start, "substeps": substeps}},
            "o": {"type": "output", "inputs": {"geo": "s"}}}, "display": "o"})
    }

    fn up() -> Value {
        json!({"translate": [0, 0, 1]})
    }

    fn at(g: &Value, frame: f64) -> Vec<f32> {
        timectx::at(Some(frame), None, || Graph::from_value(g).unwrap().cook(None).unwrap().p().to_vec())
    }

    #[test]
    fn growth_start_frame_and_errors() {
        let g = graph(up(), 1, 1);
        assert_eq!(at(&g, 1.0)[2], 0.0);
        assert_eq!(at(&g, 7.0)[2], 6.0);
        assert_eq!(at(&g, 7.4)[2], 6.0);
        assert_eq!(at(&g, 0.0)[2], 0.0);
        assert_eq!(at(&graph(up(), 10, 1), 12.0)[2], 2.0);
        assert_eq!(at(&graph(up(), 1, 4), 3.0)[2], 8.0);
        for (body, params) in [("nope", "{}"), ("grid", "{}"), ("transform", "{bad")] {
            let mut g = graph(up(), 1, 1);
            g["nodes"]["s"]["params"]["body"] = json!(body);
            g["nodes"]["s"]["params"]["params"] = json!(params);
            let err = timectx::at(Some(3.0), None, || Graph::from_value(&g).unwrap().cook(None)).unwrap_err();
            assert!(err.0.contains("'s' (solver)"), "{err}");
        }
    }

    #[test]
    fn session_playback_scrub_and_time_keys() {
        let g = graph(up(), 1, 1);
        let mut s = Session::new(Graph::from_value(&g).unwrap());
        let direct = at(&g, 30.0);
        for f in 1..=30 {
            s.set_time(f as f64, 24.0).unwrap();
            s.cook(None).unwrap();
        }
        assert_eq!(s.cook(None).unwrap().p(), &direct[..]);
        // only the solver branch recooks when the frame changes
        s.set_time(10.0, 24.0).unwrap();
        let back = s.cook(None).unwrap();
        assert_eq!(back.p(), &at(&g, 10.0)[..]);
        assert!(!s.last_stats().cooked.contains(&"p".to_string()));
        assert!(s.last_stats().cooked.contains(&"s".to_string()));
        assert!(s.set_time(f64::NAN, 24.0).is_err());
    }
}
