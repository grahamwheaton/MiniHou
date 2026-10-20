//! Rigid-body worlds and pose caches for `rbd_solver`, ported from `minihou/rbd.py`.
//!
//! What is a straight port: input validation (`pieces`, `convex_part`, `link_table`, `validate_link_values`), the
//! world/frame semantics (integer frames with fixed substeps, fractional frames hold the preceding integer frame,
//! going back restarts from the initial state, never from a snapshot; byte-bounded pose cache; at most 8 resident
//! worlds), velocity damping, the 0.1 s settle time before links may break, legacy glue's one-break-per-substep rule,
//! contact exclusion between linked pairs until all their non-colliding links break, and the output geometry
//! (transformed points and normals, `rbd_v` / `rbd_w`, colliders merged, detail `rbd`).
//!
//! What is not: Python delegates the dynamics to MuJoCo 3.15.0, an optional dependency that is not in the Rust build.
//! The dynamics here are a native sequential-impulse solver: convex-convex contacts from SAT with face clipping
//! (`margin` is the contact distance), Coulomb friction (pair friction = max), restitution (pair = mean, applied above a
//! small approach speed), split position correction, and joints as velocity constraints (weld for glue, ball for pin,
//! ball + two angular rows + limit + damping for hinge, explicit Hooke spring + length stops for spring). Masses and
//! inertias are those of the closed (convex) part meshes at uniform density, like MuJoCo's `inertia="convex"`. Results
//! therefore behave like the Python solver (bodies fall, rest, stack, tumble, links hold and break) but are not
//! bit-identical to MuJoCo. `iterations` is the velocity iteration count per substep.

mod collision;
mod input;
mod joints;
mod math;
mod solver;
mod world;

#[cfg(test)]
mod tests;

pub(super) const BACKEND: &str = solver::BACKEND;
pub(super) const BACKEND_VERSION: &str = solver::BACKEND_VERSION;
pub(crate) const SETTLE_TIME: f64 = solver::SETTLE_TIME;
pub use solver::clear_worlds;
pub(super) use solver::rbd_solver;

pub(super) use input::{link_table, pieces, Link, Piece, PieceValues};
pub(crate) use input::check_static_inputs;
pub(super) use world::World;
