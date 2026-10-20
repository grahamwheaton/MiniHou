use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::geometry::{AttrData, Geometry};
use crate::nodes::rbd::RBD_SOLVER;
use crate::nodes::sim_common::{float, int, vec3};
use crate::nodes::Inputs;
use crate::params::Params;
use crate::timectx;

use super::world::{Settings, World};

pub(crate) const BACKEND: &str = "minihou-native";
pub(crate) const BACKEND_VERSION: &str = "1";
/// Seconds after a (re)start in which links can not break: touching pieces settle first.
pub(crate) const SETTLE_TIME: f64 = 0.1;
pub(crate) const MAX_WORLDS: usize = 8;

struct Resident {
    signature: u64,
    world: Arc<Mutex<World>>,
}

static WORLDS: Mutex<Vec<Resident>> = Mutex::new(Vec::new());

fn hash_geometry(g: Option<&Geometry>, h: &mut DefaultHasher) {
    let Some(g) = g else { "none".hash(h); return };
    g.npoints.hash(h);
    g.topology.vertex_point.hash(h);
    g.topology.prim_offsets.hash(h);
    g.topology.closed.hash(h);
    for (dom, table) in [("point", &g.point), ("vertex", &g.vertex), ("prim", &g.prim)] {
        for (name, a) in table {
            (dom, name, a.width).hash(h);
            match &a.data {
                AttrData::F32(v) => v.iter().for_each(|x| x.to_bits().hash(h)),
                AttrData::I32(v) => v.hash(h),
            }
        }
    }
    Value::Object(g.detail.clone()).to_string().hash(h);
}

/// Forget every resident world (tests, or to free memory).
pub fn clear_worlds() {
    WORLDS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

pub(crate) fn rbd_solver(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let bodies = RBD_SOLVER.one(inputs, 0)?;
    let colliders = inputs[1].first().cloned();
    let constraints = inputs[2].first().cloned();
    let settings = Settings {
        gravity: vec3(p, "gravity")?,
        start_frame: p.i("start_frame")?,
        substeps: int(p, "substeps", Some(1), Some(128))?,
        iterations: int(p, "iterations", Some(1), Some(500))?,
        margin: float(p, "margin", Some(0.0), Some(1.0))?,
        cache_mb: int(p, "cache_mb", Some(1), Some(1024))? as f64,
        include_colliders: p.b("include_colliders")?,
    };
    let reset = p.i("reset")?;
    let (frame, fps) = (timectx::frame(), timectx::fps());
    if !fps.is_finite() || fps <= 0.0 || !frame.is_finite() {
        return Err(Error::new("RBD frame must be finite and FPS positive"));
    }
    let mut h = DefaultHasher::new();
    (settings.gravity.map(f64::to_bits), settings.start_frame, settings.substeps, settings.iterations, settings.margin.to_bits(),
     settings.cache_mb.to_bits(), settings.include_colliders, reset, fps.to_bits(), BACKEND_VERSION).hash(&mut h);
    for g in [Some(&*bodies), colliders.as_deref(), constraints.as_deref()] {
        hash_geometry(g, &mut h);
    }
    let signature = h.finish();
    let target = settings.start_frame.max(frame.floor() as i64);
    if target - settings.start_frame > 100000 {
        return Err(Error::new("RBD frame range exceeds 100000 frames"));
    }
    let world = {
        let mut worlds = WORLDS.lock().unwrap_or_else(|e| e.into_inner());
        match worlds.iter().position(|r| r.signature == signature) {
            Some(i) => {
                let r = worlds.remove(i);
                let w = r.world.clone();
                worlds.push(r);
                w
            }
            None => {
                drop(worlds);
                let w = Arc::new(Mutex::new(World::new(bodies.clone(), colliders.clone(), constraints.as_deref(), settings)?));
                let mut worlds = WORLDS.lock().unwrap_or_else(|e| e.into_inner());
                worlds.push(Resident { signature, world: w.clone() });
                while worlds.len() > MAX_WORLDS {
                    worlds.remove(0);
                }
                w
            }
        }
    };
    let result = world.lock().unwrap_or_else(|e| e.into_inner()).output(target);
    if result.is_err() {
        WORLDS.lock().unwrap_or_else(|e| e.into_inner()).retain(|r| r.signature != signature);
    }
    result
}
