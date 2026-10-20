//! The network menu shares the catalogue and labels with the shelf. Its
//! layout keeps the few historical ordering and membership differences.
#[path = "shelf_data.rs"]
mod shelf_data;

use minihou_core::nodes::package_catalogue;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub(super) struct Tool { pub label: &'static str, pub node_type: &'static str }
#[derive(Clone, Copy)]
pub(super) struct Category { pub name: &'static str, pub tools: &'static [Tool] }

#[derive(Deserialize)]
struct Placement { id: String, label: Option<String> }
#[derive(Deserialize)]
struct LayoutTab { name: String, tools: Vec<Placement> }

fn layout() -> &'static [LayoutTab] {
    static LAYOUT: OnceLock<Vec<LayoutTab>> = OnceLock::new();
    LAYOUT.get_or_init(|| serde_json::from_str(include_str!("network_menu_layout.json")).expect("network menu layout"))
}
fn permanent(value: &str) -> &'static str { Box::leak(value.to_owned().into_boxed_str()) }

fn build(append_new: bool) -> Vec<Category> {
    layout().iter().map(|tab| {
        let mut tools: Vec<Tool> = tab.tools.iter().map(|placement| {
            let label = placement.label.as_deref()
                .or_else(|| shelf_data::tabs().iter().find(|shelf| shelf.name == tab.name)
                    .and_then(|shelf| shelf.tools.iter().find(|tool| tool.node_type == placement.id)).map(|tool| tool.label))
                .or_else(|| package_catalogue().iter().find(|node| node.id == placement.id).map(|node| node.name))
                .unwrap_or(&placement.id);
            Tool { label: permanent(label), node_type: permanent(&placement.id) }
        }).collect();
        if append_new {
            for node in package_catalogue().iter().filter(|node| node.category == tab.name) {
                if tools.iter().any(|tool| tool.node_type == node.id) { continue; }
                tools.push(Tool { label: node.name, node_type: node.id });
            }
        }
        Category { name: permanent(&tab.name), tools: Box::leak(tools.into_boxed_slice()) }
    }).collect()
}

pub(super) fn legacy_tabs() -> &'static [Category] {
    static LEGACY: OnceLock<Vec<Category>> = OnceLock::new();
    LEGACY.get_or_init(|| build(false))
}
pub(super) fn tabs() -> &'static [Category] {
    static TABS: OnceLock<Vec<Category>> = OnceLock::new();
    TABS.get_or_init(|| build(true))
}
