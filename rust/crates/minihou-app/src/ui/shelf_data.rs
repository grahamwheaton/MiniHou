//! Shelf presentation order. Node details come from the package catalogue;
//! the layout records existing placements and legacy display overrides.
use minihou_core::nodes::{package_catalogue, NodeCatalogueEntry};
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub(super) struct Tool { pub label: &'static str, pub node_type: &'static str, pub icon: &'static str }
#[derive(Clone, Copy)]
pub(super) struct Tab { pub name: &'static str, pub tools: &'static [Tool] }

#[derive(Deserialize)]
struct Placement { id: String, label: Option<String>, icon: Option<String> }
#[derive(Deserialize)]
struct LayoutTab { name: String, tools: Vec<Placement> }

fn layout() -> &'static [LayoutTab] {
    static LAYOUT: OnceLock<Vec<LayoutTab>> = OnceLock::new();
    LAYOUT.get_or_init(|| serde_json::from_str(include_str!("shelf_layout.json")).expect("shelf layout"))
}

fn permanent(value: &str) -> &'static str { Box::leak(value.to_owned().into_boxed_str()) }

fn build(catalogue: &[NodeCatalogueEntry], append_new: bool) -> Vec<Tab> {
    layout().iter().map(|tab| {
        let mut tools: Vec<Tool> = tab.tools.iter().map(|placement| {
            let package = catalogue.iter().find(|node| node.id == placement.id);
            Tool {
                label: package.map(|node| node.name).or_else(|| placement.label.as_deref()).map(permanent).unwrap_or_else(|| permanent(&placement.id)),
                node_type: permanent(&placement.id),
                icon: placement.icon.as_deref().map(permanent).unwrap_or_else(|| permanent(&placement.id)),
            }
        }).collect();
        if append_new {
            for node in catalogue.iter().filter(|node| node.category == tab.name) {
                if tools.iter().any(|tool| tool.node_type == node.id) { continue; }
                tools.push(Tool { label: node.name, node_type: node.id, icon: node.id });
            }
        }
        Tab { name: permanent(&tab.name), tools: Box::leak(tools.into_boxed_slice()) }
    }).collect()
}

pub(super) fn legacy_tabs() -> &'static [Tab] {
    static LEGACY: OnceLock<Vec<Tab>> = OnceLock::new();
    LEGACY.get_or_init(|| build(package_catalogue(), false))
}

pub(super) fn tabs() -> &'static [Tab] {
    static TABS: OnceLock<Vec<Tab>> = OnceLock::new();
    TABS.get_or_init(|| build(package_catalogue(), true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_shelf_matches_existing_positions_labels_and_icons() {
        let expected: serde_json::Value = serde_json::from_str(include_str!("shelf_snapshot.json")).unwrap();
        let actual: Vec<_> = tabs().iter().map(|tab| serde_json::json!({
            "name": tab.name,
            "tools": tab.tools.iter().map(|tool| [tool.label, tool.node_type, tool.icon]).collect::<Vec<_>>()
        })).collect();
        assert_eq!(serde_json::json!(actual), expected);
    }

    #[test]
    fn newly_registered_package_appears_without_a_shelf_entry() {
        let added = NodeCatalogueEntry { id: "future_package", name: "Future Package", category: "Create", description: "", keywords: "" };
        let mut catalogue: Vec<_> = package_catalogue().iter().map(|node| NodeCatalogueEntry {
            id: node.id, name: node.name, category: node.category, description: node.description, keywords: node.keywords,
        }).collect();
        catalogue.push(added);
        let generated = build(&catalogue, true);
        let create = generated.iter().find(|tab| tab.name == "Create").unwrap();
        assert_eq!(create.tools.last().map(|tool| (tool.label, tool.node_type, tool.icon)),
            Some(("Future Package", "future_package", "future_package")));
    }
}
