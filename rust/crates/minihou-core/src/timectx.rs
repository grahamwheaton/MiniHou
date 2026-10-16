//! Time: the frame a graph cooks at (Houdini's $F / $T), mirroring `minihou/timectx.py`.
//!
//! The evaluator sets the frame for the duration of a cook. Python keeps it in context variables; here it is a thread-local
//! with scoped overrides ([`at`], [`step`]), so nested cooks on the same thread see it and cooks on other threads do not
//! interfere. Read it on the cooking thread (rayon workers do not inherit it). `Time` is in seconds with frame 1 at time 0.

use std::cell::Cell;

use serde_json::{Map, Value};

pub const DEFAULT_FPS: f64 = 24.0;

thread_local! {
    static STATE: Cell<(f64, f64)> = const { Cell::new((1.0, DEFAULT_FPS)) };
    static STEP: Cell<Option<f64>> = const { Cell::new(None) };
}

pub fn frame() -> f64 {
    STATE.with(|s| s.get().0)
}

pub fn fps() -> f64 {
    STATE.with(|s| s.get().1)
}

/// Seconds: `(frame - 1) / fps`.
pub fn seconds() -> f64 {
    let (f, rate) = STATE.with(|s| s.get());
    (f - 1.0) / rate
}

/// Seconds per step: `1 / fps`, or a solver substep's share of it (Houdini's @TimeInc).
pub fn time_inc() -> f64 {
    STEP.with(|s| s.get()).unwrap_or_else(|| 1.0 / fps())
}

struct Restore<T: Copy + 'static>(&'static std::thread::LocalKey<Cell<T>>, T);

impl<T: Copy + 'static> Drop for Restore<T> {
    fn drop(&mut self) {
        self.0.with(|s| s.set(self.1));
    }
}

/// Run `f` at a frame and/or fps (`None` keeps the current value).
pub fn at<R>(frame_: Option<f64>, fps_: Option<f64>, f: impl FnOnce() -> R) -> R {
    let old = STATE.with(|s| s.get());
    let new = (frame_.unwrap_or(old.0), fps_.unwrap_or(old.1));
    STATE.with(|s| s.set(new));
    let _restore = Restore(&STATE, old);
    f()
}

/// Run `f` with `@TimeInc` = `seconds_per_step`.
pub fn step<R>(seconds_per_step: f64, f: impl FnOnce() -> R) -> R {
    let old = STEP.with(|s| s.get());
    STEP.with(|s| s.set(Some(seconds_per_step)));
    let _restore = Restore(&STEP, old);
    f()
}

/// Python `repr` of a float (shortest round-trip, always with a `.0` or exponent).
fn py_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let s = format!("{x:?}");
    s
}

/// Cache-key material for time-dependent nodes (`t{frame!r}@{fps!r}/{step!r}` in Python).
pub fn key() -> String {
    let (f, rate) = STATE.with(|s| s.get());
    let step = STEP.with(|s| s.get()).map_or_else(|| "None".to_string(), py_repr);
    format!("t{}@{}/{}", py_repr(f), py_repr(rate), step)
}

fn mentions_time(s: &str) -> bool {
    // `@(Frame|Time|TimeInc)\b`
    let b = s.as_bytes();
    let mut i = 0;
    while let Some(pos) = s[i..].find('@') {
        let start = i + pos + 1;
        for word in ["TimeInc", "Frame", "Time"] {
            if s[start..].starts_with(word) {
                let end = start + word.len();
                if end >= b.len() || !(b[end].is_ascii_alphanumeric() || b[end] == b'_') {
                    return true;
                }
            }
        }
        i = start;
    }
    false
}

/// True if a string parameter mentions @Frame/@Time/@TimeInc, or the node says it is `animated`.
pub fn params_use_time(params: &Map<String, Value>) -> bool {
    params.get("animated") == Some(&Value::Bool(true))
        || params.values().any(|v| v.as_str().is_some_and(mentions_time))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_frame_step_and_key() {
        assert_eq!((frame(), fps()), (1.0, 24.0));
        assert_eq!(key(), "t1.0@24.0/None");
        at(Some(12.5), None, || {
            assert_eq!(frame(), 12.5);
            assert!((seconds() - 11.5 / 24.0).abs() < 1e-15);
            step(0.01, || {
                assert_eq!(time_inc(), 0.01);
                assert_eq!(key(), "t12.5@24.0/0.01");
            });
            assert_eq!(time_inc(), 1.0 / 24.0);
            at(None, Some(30.0), || assert_eq!((frame(), fps()), (12.5, 30.0)));
        });
        assert_eq!(frame(), 1.0);
        let r = std::panic::catch_unwind(|| at(Some(5.0), None, || panic!("x")));
        assert!(r.is_err());
        assert_eq!(frame(), 1.0);
    }

    #[test]
    fn detects_time_parameters() {
        let p = |v: Value| v.as_object().unwrap().clone();
        assert!(params_use_time(&p(serde_json::json!({"snippet": "@P.z += @Time"}))));
        assert!(params_use_time(&p(serde_json::json!({"snippet": "@TimeInc"}))));
        assert!(params_use_time(&p(serde_json::json!({"animated": true}))));
        assert!(!params_use_time(&p(serde_json::json!({"snippet": "@Timer + @Frames", "n": 3}))));
    }
}
