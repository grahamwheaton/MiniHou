//! Viewport chrome drawn over the 3-D view (design: Viewport.dc.html): header bar, left tool strip, timeline, stats chip,
//! axis gizmo, error / dense-mesh notes. `rect` is the viewport's area; the 3-D image is already painted into it.
//! Baseline: nothing drawn.
use super::{Actions, View};
use eframe::egui;

pub fn overlay(_ui: &mut egui::Ui, _rect: egui::Rect, _v: &View, _out: &mut Actions) {}
