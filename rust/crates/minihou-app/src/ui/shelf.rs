//! Tool shelf (design: Main.dc.html `.shelf`): tabs (Create, Modify, ...) and one-click tools. A click raises
//! `UiAction::AddNode { node_type, after: selected }`. The tab and tool lists are the ones in the old editor's
//! `minihou/web/static/shelf.js` (the design shows the Create row). Baseline: nothing drawn.
use super::{Actions, View};
use eframe::egui;

pub fn show(_ui: &mut egui::Ui, _v: &View, _out: &mut Actions) {}
