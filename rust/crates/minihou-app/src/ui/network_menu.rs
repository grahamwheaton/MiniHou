//! Local network context and Tab menu.
#[path = "network_menu_data.rs"]
mod shelf_data;

use super::{Actions, UiAction, View};
use eframe::egui::{self, pos2, Color32, FontId, Id, Key, Order, Pos2, RichText, Stroke};
use shelf_data::TABS;

#[derive(Clone)]
pub(super) struct MenuState { at: Pos2, target: Option<String>, add: bool, pie: bool, category: Option<usize>, query: String, active: usize, focus: bool,
    pub(super) connect_to: Option<(crate::graph_edit::AddConnection, Pos2)> }
impl MenuState {
    pub(super) fn new(at: Pos2, target: Option<String>, add: bool) -> Self {
        Self { at, target, add, pie: false, category: None, query: String::new(), active: 0, focus: true, connect_to: None }
    }
    pub(super) fn pie(at: Pos2) -> Self { let mut menu = Self::new(at, None, true); menu.pie = true; menu }
    pub(super) fn query(&self) -> &str { &self.query }
    pub(super) fn at(&self) -> Pos2 { self.at }
    pub(super) fn connected(at: Pos2, connection: crate::graph_edit::AddConnection, world: Pos2) -> Self {
        let mut menu = Self::new(at, None, true);
        menu.connect_to = Some((connection, world));
        menu
    }
}

fn accepts(menu: &MenuState, ty: &str) -> bool {
    use crate::graph_edit::AddConnection;
    match &menu.connect_to {
        Some((AddConnection::Output(_), _)) => minihou_core::nodes::get(ty).is_some_and(|s| !s.inputs.is_empty()),
        Some((AddConnection::Input { .. }, _)) => ty != "output" && minihou_core::nodes::get(ty).is_some(),
        None => true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item { Add, Paste, Tidy, FrameAll, Display, Mute, Lock, Template, Duplicate, Delete, Rename }
fn items_for(target: Option<&str>) -> Vec<Item> {
    if target.is_some() { vec![Item::Display, Item::Mute, Item::Lock, Item::Template, Item::Duplicate, Item::Delete, Item::Rename] }
    else { vec![Item::Add, Item::Paste, Item::Tidy, Item::FrameAll] }
}

#[derive(Clone, Copy)]
enum Entry { Item(Item), Category(usize), Tool(&'static str) }
fn entries(menu: &MenuState) -> Vec<(String, Entry)> {
    if !menu.add { return items_for(menu.target.as_deref()).into_iter().map(|item| (item_label(item).to_owned(), Entry::Item(item))).collect(); }
    if !menu.query.trim().is_empty() {
        let query = menu.query.to_lowercase();
        let mut matches: Vec<_> = TABS.iter().flat_map(|tab| tab.tools.iter().map(move |tool| (tab.name, tool)))
            .filter(|(category, tool)| accepts(menu, tool.node_type) && (tool.label.to_lowercase().contains(&query) || tool.node_type.contains(&query) || category.to_lowercase().contains(&query)))
            .map(|(category, tool)| (format!("{} · {}", tool.label, category), Entry::Tool(tool.node_type))).collect();
        matches.truncate(80);
        return matches;
    }
    if let Some(index) = menu.category {
        if let Some(tab) = TABS.get(index) {
            return tab.tools.iter().filter(|tool| accepts(menu, tool.node_type)).map(|tool| (tool.label.to_owned(), Entry::Tool(tool.node_type))).collect();
        }
    }
    if menu.pie {
        return ["Adjust", "Deform", "Polygons", "Group", "Create", "Build", "Edges", "Modify"].into_iter().enumerate()
            .map(|(slot, name)| {
                let catalog = ["Model", "Deform", "Polygon", "Groups", "Create", "Model", "Edges", "Modify"][slot];
                let index = TABS.iter().position(|tab| tab.name == catalog).unwrap_or(0);
                (format!("{}  {name}", slot + 1), Entry::Category(index))
            }).collect();
    }
    TABS.iter().enumerate().filter(|(_, tab)| !tab.tools.is_empty())
        .map(|(index, tab)| (format!("{}  ›", tab.name), Entry::Category(index))).collect()
}
fn item_label(item: Item) -> &'static str {
    match item {
        Item::Add => "Add node  ›", Item::Paste => "Paste", Item::Tidy => "Tidy network", Item::FrameAll => "Frame all",
        Item::Display => "Display (Visible)", Item::Mute => "Mute / Unmute", Item::Lock => "Lock / Unlock",
        Item::Template => "Template", Item::Duplicate => "Duplicate", Item::Delete => "Delete", Item::Rename => "Rename",
    }
}

pub(super) enum Choice { Close, FrameAll, Tidy }
fn activate(entry: Entry, menu: &mut MenuState, v: &View, out: &mut Actions) -> Option<Choice> {
    match entry {
        Entry::Category(index) => { menu.category = Some(index); menu.active = 0; None }
        Entry::Tool(node_type) => {
            if let Some((connection, at)) = &menu.connect_to {
                out.push(UiAction::AddConnectedNode { node_type: node_type.to_owned(), connection: connection.clone(), pos: [at.x, at.y] });
                return Some(Choice::Close);
            }
            let after = (!v.selected.is_empty() && v.graph.nodes.iter().any(|(id, _)| id == v.selected)).then(|| v.selected.to_owned());
            out.push(UiAction::AddNode { node_type: node_type.to_owned(), after });
            Some(Choice::Close)
        }
        Entry::Item(Item::Add) => { menu.add = true; menu.category = None; menu.active = 0; menu.focus = true; None }
        Entry::Item(Item::Paste) => { out.push(UiAction::Command("paste".into())); Some(Choice::Close) }
        Entry::Item(Item::Tidy) => Some(Choice::Tidy),
        Entry::Item(Item::FrameAll) => Some(Choice::FrameAll),
        Entry::Item(Item::Rename) => None,
        Entry::Item(item) => {
            let Some(id) = &menu.target else { return None };
            let action = match item {
                Item::Display => UiAction::SetDisplay(id.clone()),
                Item::Mute => {
                    let bypass = v.graph.nodes.iter().find(|(node, _)| node == id).is_some_and(|(_, def)| def.bypass);
                    UiAction::Edit { edit: minihou_app::Edit::Bypass { node: id.clone(), flag: !bypass }, continuous: false }
                }
                Item::Lock => UiAction::ToggleLocked(id.clone()),
                Item::Template => UiAction::ToggleTemplate(id.clone()),
                Item::Duplicate => UiAction::DuplicateNode(id.clone()),
                Item::Delete => UiAction::DeleteNode(id.clone()),
                _ => return None,
            };
            out.push(action);
            Some(Choice::Close)
        }
    }
}

pub(super) fn show(ctx: &egui::Context, id: Id, menu: &mut MenuState, v: &View, out: &mut Actions) -> Option<Choice> {
    if ctx.input(|i| i.key_pressed(Key::Escape)) || (!menu.focus && ctx.input(|i| i.key_pressed(Key::Tab))) {
        ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::default(), Key::Escape);
            i.consume_key(egui::Modifiers::default(), Key::Tab);
        });
        return Some(Choice::Close);
    }
    let mut chosen = None;
    let mut shown = entries(menu);
    if menu.pie {
        for (slot, key) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8].into_iter().enumerate() {
            if ctx.input(|i| i.key_pressed(key)) { chosen = shown.get(slot).map(|(_, entry)| *entry); break; }
        }
        if let Some(entry) = chosen { return activate(entry, menu, v, out); }
    }
    if !shown.is_empty() { menu.active = menu.active.min(shown.len() - 1); }
    if ctx.input(|i| i.key_pressed(Key::ArrowDown)) && !shown.is_empty() { menu.active = (menu.active + 1) % shown.len(); }
    if ctx.input(|i| i.key_pressed(Key::ArrowUp)) && !shown.is_empty() { menu.active = (menu.active + shown.len() - 1) % shown.len(); }
    if ctx.input(|i| i.key_pressed(Key::Enter)) { chosen = shown.get(menu.active).map(|(_, entry)| *entry); }
    let screen = ctx.input(|i| i.viewport_rect());
    let at = pos2(menu.at.x.clamp(screen.left() + 4.0, (screen.right() - 258.0).max(screen.left() + 4.0)),
        menu.at.y.clamp(screen.top() + 4.0, (screen.bottom() - 390.0).max(screen.top() + 4.0)));
    let area = egui::Area::new(id).order(Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        egui::Frame::new().fill(v.theme.panel2).stroke(Stroke::new(1.0_f32, v.theme.stroke)).corner_radius(6.0).inner_margin(6.0).show(ui, |ui| {
            ui.set_width(240.0);
            if menu.add {
                if menu.category.is_some() && menu.query.is_empty() && ui.button("‹  Categories").clicked() { menu.category = None; menu.active = 0; }
                let response = ui.add(egui::TextEdit::singleline(&mut menu.query).hint_text("Search nodes…").desired_width(230.0).font(FontId::proportional(12.0)));
                if menu.focus { ui.memory_mut(|m| m.request_focus(response.id)); menu.focus = false; }
                if response.changed() { menu.active = 0; shown = entries(menu); }
                ui.separator();
            }
            egui::ScrollArea::vertical().max_height(350.0).show(ui, |ui| {
                for (index, (label, entry)) in shown.iter().enumerate() {
                    let disabled = matches!(entry, Entry::Item(Item::Rename));
                    let color = if disabled { v.theme.faint } else if index == menu.active { v.theme.accent } else { v.theme.text };
                    let response = ui.add_enabled(!disabled, egui::Button::new(RichText::new(label).color(color)).selected(index == menu.active)
                        .fill(if index == menu.active { v.theme.accent_bg } else { Color32::TRANSPARENT }).frame(false).sense(egui::Sense::click_and_drag()));
                    if let Entry::Tool(ty) = entry { response.dnd_set_drag_payload(crate::ui::ToolDrag((*ty).into())); }
                    if response.hovered() { menu.active = index; }
                    if response.clicked() { chosen = Some(*entry); }
                }
            });
        });
    });
    if let Some(entry) = chosen { return activate(entry, menu, v, out); }
    let (any_pressed, interact_pos) = ctx.input(|i| (i.pointer.any_pressed(), i.pointer.interact_pos()));
    if !menu.focus && any_pressed && !area.response.rect.contains(interact_pos.unwrap_or(Pos2::ZERO)) {
        return Some(Choice::Close);
    }
    if !menu.add { menu.focus = false; }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn noodle_search_filters_and_keeps_connection_through_categories() {
        use crate::graph_edit::AddConnection;
        let mut menu = MenuState::connected(pos2(100., 200.), AddConnection::Output("source".into()), pos2(96., 192.));
        assert!(!accepts(&menu, "point"));
        assert!(accepts(&menu, "transform"));
        let f = crate::ui::testing::Fixture::new();
        let mut out = Actions::default();
        assert!(activate(Entry::Category(0), &mut menu, &f.view(), &mut out).is_none());
        assert!(out.0.is_empty());
        menu.query = "transform".into();
        assert!(entries(&menu).iter().any(|(_, e)| matches!(e, Entry::Tool("transform"))));
        assert!(matches!(activate(Entry::Tool("transform"), &mut menu, &f.view(), &mut out), Some(Choice::Close)));
        assert!(matches!(out.0.as_slice(), [UiAction::AddConnectedNode { connection: AddConnection::Output(source), pos, .. }] if source == "source" && *pos == [96., 192.]));
        let menu = MenuState::connected(Pos2::ZERO, AddConnection::Input { node: "target".into(), port: "geo".into() }, Pos2::ZERO);
        assert!(accepts(&menu, "box"));
        assert!(!accepts(&menu, "output"));
    }
    #[test]
    fn menu_target_model() {
        assert_eq!(items_for(None), vec![Item::Add, Item::Paste, Item::Tidy, Item::FrameAll]);
        assert_eq!(items_for(Some("box1")), vec![Item::Display, Item::Mute, Item::Lock, Item::Template, Item::Duplicate, Item::Delete, Item::Rename]);
    }
    #[test]
    fn tab_search_uses_shelf_types() {
        let mut menu = MenuState::new(Pos2::ZERO, None, true);
        menu.query = "grid".into();
        assert!(entries(&menu).iter().any(|(_, entry)| matches!(entry, Entry::Tool("grid"))));
    }
    #[test] fn pie_number_slots_follow_web_main_page() {
        let menu = MenuState::pie(Pos2::ZERO);
        let labels: Vec<_> = entries(&menu).into_iter().map(|(label, _)| label).collect();
        assert_eq!(labels, ["1  Adjust", "2  Deform", "3  Polygons", "4  Group", "5  Create", "6  Build", "7  Edges", "8  Modify"]);
    }
}
