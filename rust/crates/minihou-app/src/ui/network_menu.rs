//! Local network context and Tab menu.
#[path = "network_menu_data.rs"]
mod shelf_data;

use super::{Actions, UiAction, View};
use eframe::egui::{self, pos2, Color32, FontId, Id, Key, Order, Pos2, RichText, Stroke};
use shelf_data::TABS;
use std::{collections::HashSet, sync::{Mutex, OnceLock}};

#[derive(Default)]
struct MenuPrefs { favs: Vec<String>, recent: Vec<String> }
fn prefs() -> &'static Mutex<MenuPrefs> {
    static PREFS: OnceLock<Mutex<MenuPrefs>> = OnceLock::new();
    PREFS.get_or_init(|| Mutex::new(MenuPrefs::default()))
}

fn web_data() -> &'static serde_json::Value {
    static DATA: OnceLock<serde_json::Value> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("network_menu_web.json")).expect("web node menu metadata"))
}
fn meta(ty: &str, field: &str) -> &'static str {
    web_data()[ty][field].as_str().unwrap_or("")
}
fn words(s: &str) -> Vec<String> {
    s.to_ascii_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}
fn token_score(label: &str, name: &str, keywords: &str, category: &str, doc: &str, tok: &str) -> Option<u32> {
    let label = label.to_ascii_lowercase();
    let name = name.replace('_', " ").to_ascii_lowercase();
    if label == tok || name == tok { Some(0) }
    else if label.starts_with(tok) || name.starts_with(tok) { Some(1) }
    else if words(&label).iter().chain(words(&name).iter()).any(|w| w.starts_with(tok)) { Some(2) }
    else if label.contains(tok) || name.contains(tok) { Some(3) }
    else if words(keywords).iter().any(|w| w == tok) { Some(4) }
    else if words(keywords).iter().any(|w| w.starts_with(tok)) { Some(5) }
    else if category.to_ascii_lowercase().contains(tok) { Some(6) }
    else if doc.to_ascii_lowercase().contains(tok) { Some(7) }
    else { None }
}
fn search_score(ty: &str, query: &str) -> Option<u32> {
    let label = meta(ty, "label");
    let label = if label.is_empty() { ty } else { label };
    let tokens = query.to_ascii_lowercase().split(|c: char| c.is_ascii_whitespace() || c == ',' || c == '_')
        .filter(|t| !t.is_empty()).map(str::to_owned).collect::<Vec<_>>();
    let category = format!("{} {}", meta(ty, "menu"), meta(ty, "category"));
    let mut score = 0;
    for tok in tokens { score += token_score(label, ty, meta(ty, "keywords"), &category, meta(ty, "doc"), &tok)?; }
    Some(score * 100 + label.len().min(40) as u32 + if meta(ty, "label").is_empty() { 50 } else { 0 })
}

#[derive(Clone)]
pub(super) struct MenuState { at: Pos2, target: Option<String>, add: bool, pie: bool, category: Option<usize>, open: HashSet<usize>, query: String, active: usize, focus: bool,
    pub(super) connect_to: Option<(crate::graph_edit::AddConnection, Pos2)> }
impl MenuState {
    pub(super) fn new(at: Pos2, target: Option<String>, add: bool) -> Self {
        Self { at, target, add, pie: false, category: None, open: HashSet::from([0, 3]), query: String::new(), active: 1, focus: true, connect_to: None }
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
enum Entry { Item(Item), Category(usize), Tool(&'static str), Operator(&'static str) }
fn entries(menu: &MenuState) -> Vec<(String, Entry)> {
    if !menu.add { return items_for(menu.target.as_deref()).into_iter().map(|item| (item_label(item).to_owned(), Entry::Item(item))).collect(); }
    if !menu.query.trim().is_empty() {
        let query = menu.query.trim();
        let mut seen = HashSet::new();
        let mut ranked: Vec<_> = TABS.iter().flat_map(|tab| tab.tools.iter())
            .filter(|tool| seen.insert(tool.node_type) && accepts(menu, tool.node_type))
            .filter_map(|tool| search_score(tool.node_type, query).map(|score| (score, tool))).collect();
        ranked.sort_by(|a,b| a.0.cmp(&b.0).then_with(|| meta(a.1.node_type, "label").cmp(meta(b.1.node_type, "label"))));
        let mut matches: Vec<_> = ranked.into_iter().map(|(_, tool)|
            (format!("{} · {}", tool.label, meta(tool.node_type, "menu")), Entry::Tool(tool.node_type))).collect();
        if menu.connect_to.is_none() && "select coplanar".contains(&query) {matches.insert(0,("Select Coplanar · Selection".into(),Entry::Operator("select_coplanar")));}
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
    let mut result = Vec::new();
    let saved = prefs().lock().unwrap();
    for (names, hint) in [(&saved.favs, "★ Favourites"), (&saved.recent, "Recent")] {
        for ty in names.iter().take(6).filter(|ty| accepts(menu, ty)) {
            if let Some(tool) = TABS.iter().flat_map(|tab| tab.tools.iter()).find(|tool| tool.node_type == ty) {
                if hint != "Recent" || !saved.favs.contains(ty) { result.push((format!("{} · {hint}", tool.label), Entry::Tool(tool.node_type))); }
            }
        }
    }
    for (index, tab) in TABS.iter().enumerate().filter(|(_, tab)| !tab.tools.is_empty()) {
        result.push((format!("{}  {}", tab.name, if menu.open.contains(&index) { "▾" } else { "▸" }), Entry::Category(index)));
        if menu.open.contains(&index) {
            result.extend(tab.tools.iter().filter(|tool| accepts(menu, tool.node_type)).map(|tool| (tool.label.to_owned(), Entry::Tool(tool.node_type))));
        }
    }
    result
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
        Entry::Operator(name)=>{out.push(UiAction::Command(name.into()));Some(Choice::Close)}
        Entry::Category(index) => {
            if menu.pie { menu.category = Some(index); menu.active = 0; }
            else if !menu.open.insert(index) { menu.open.remove(&index); }
            None
        }
        Entry::Tool(node_type) => {
            { let mut saved = prefs().lock().unwrap();
              saved.recent.retain(|ty| ty != node_type);
              saved.recent.insert(0, node_type.to_owned()); saved.recent.truncate(12); }
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
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::default(), Key::Escape);
        });
        return Some(Choice::Close);
    }
    // The web menu consumes held/repeated Tab while it is open.
    ctx.input_mut(|i| { i.consume_key(egui::Modifiers::default(), Key::Tab); });
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
    if menu.query.is_empty() && !menu.pie && ctx.input(|i| i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::ArrowRight)) {
        let open = ctx.input(|i| i.key_pressed(Key::ArrowRight));
        if let Some((_, Entry::Category(index))) = shown.get(menu.active) {
            if open { menu.open.insert(*index); } else { menu.open.remove(index); }
            shown = entries(menu);
        }
    }
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
                    let available = !matches!(entry, Entry::Tool(ty) if minihou_core::nodes::get(ty).is_none());
                    let starred = matches!(entry, Entry::Tool(ty) if prefs().lock().unwrap().favs.iter().any(|fav| fav == ty));
                    let shown_label = format!("{}{}{}", if starred { "★ " } else { "" }, if available { "" } else { "● " }, label);
                    let response = ui.add_enabled(!disabled, egui::Button::new(RichText::new(shown_label).color(color)).selected(index == menu.active)
                        .fill(if index == menu.active { v.theme.accent_bg } else { Color32::TRANSPARENT }).frame(false).sense(egui::Sense::click_and_drag()));
                    if let Entry::Tool(ty) = entry { response.dnd_set_drag_payload(crate::ui::ToolDrag((*ty).into())); }
                    if response.secondary_clicked() { if let Entry::Tool(ty) = entry {
                        let mut saved = prefs().lock().unwrap();
                        if saved.favs.iter().any(|fav| fav == ty) { saved.favs.retain(|fav| fav != ty); }
                        else { saved.favs.push((*ty).to_owned()); }
                    } }
                    if response.hovered() { menu.active = index; }
                    if response.clicked() { chosen = Some(*entry); }
                    if let Entry::Tool(ty) = entry {
                        let description = meta(ty, "doc").lines().next().unwrap_or("");
                        if !description.is_empty() { ui.label(RichText::new(description).small().color(v.theme.faint)); }
                    }
                }
            });
            if menu.add { ui.separator(); ui.label(RichText::new("↑ ↓ choose   Enter add   Esc close   • missing Rust node").small().color(v.theme.faint)); }
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
    #[test] fn web_ranked_queries_match_original_node_menu() {
        for (query, expected) in [("box", "box"), ("extrude", "polyextrude"), ("bevel", "polybevel"),
            ("move", "transform"), ("rotate", "twist"), ("scale", "transform"),
            ("loop", "loop_end"), ("python", "python"), ("donut", "torus"), ("image", "image_out")] {
            let mut menu = MenuState::new(Pos2::ZERO, None, true);
            menu.query = query.into();
            assert!(matches!(entries(&menu).first(), Some((_, Entry::Tool(ty))) if *ty == expected), "{query}");
        }
    }
    #[test] fn search_navigation_wraps_and_resets_on_query_edit() {
        let mut menu = MenuState::new(Pos2::ZERO, None, true);
        menu.query = "bevel".into();
        let rows = entries(&menu);
        assert!(matches!(rows.first(), Some((_, Entry::Tool("polybevel")))));
        menu.active = (menu.active + 1) % rows.len();
        menu.active = (menu.active + rows.len() - 1) % rows.len();
        assert_eq!(menu.active, 0);
        menu.query = "box".into(); menu.active = 0;
        assert!(matches!(entries(&menu).first(), Some((_, Entry::Tool("box")))));
    }
    #[test] fn egui_search_enter_dispatches_selected_node() {
        let f = crate::ui::testing::Fixture::new();
        let ctx = egui::Context::default();
        let mut menu = MenuState::new(pos2(30., 30.), None, true);
        menu.query = "bevel".into(); menu.focus = false;
        let mut actions = Actions::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| { show(ctx, Id::new("search_harness"), &mut menu, &f.view(), &mut actions); });
        let input = egui::RawInput { events: vec![egui::Event::Key { key: Key::Enter, physical_key: None,
            pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }], ..Default::default() };
        let _ = ctx.run(input, |ctx| { show(ctx, Id::new("search_harness"), &mut menu, &f.view(), &mut actions); });
        assert!(matches!(actions.0.as_slice(), [UiAction::AddNode { node_type, .. }] if node_type == "polybevel"));
    }
    #[test] fn coplanar_search_runs_a_viewport_operator_not_an_add_node() {
        let f=crate::ui::testing::Fixture::new();let mut menu=MenuState::new(pos2(0.,0.),None,true);
        menu.query="select coplanar".into();let shown=entries(&menu);
        let entry=shown.iter().find(|(_,entry)|matches!(entry,Entry::Operator("select_coplanar"))).unwrap().1;
        let mut actions=Actions::default();assert!(matches!(activate(entry,&mut menu,&f.view(),&mut actions),Some(Choice::Close)));
        assert!(matches!(actions.0.as_slice(),[UiAction::Command(name)] if name=="select_coplanar"));
    }
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
