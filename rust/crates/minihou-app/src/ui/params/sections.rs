use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use minihou_app::{param_meta, Edit};
use crate::ui::{Actions, UiAction, View};
use minihou_core::nodes;
use serde_json::{json, Value};

use super::paint_text;
use crate::ui::curve_tool;
use crate::ui::deform_gizmo;

pub fn deformer_section(typ: &str, name: &str) -> Option<&'static str> {
    if typ == "polyextrude" {
        return Some(if matches!(name, "group" | "edges" | "points" | "divide_into" | "split_group") { "Selection" }
            else if name.starts_with("outgrp_") || matches!(name, "output_front" | "output_back" | "output_side" | "preserve_groups" | "copy_edge_groups" | "copy_point_groups" | "copy_primitive_groups") { "Output Geometry and Groups" }
            else if matches!(name, "front_magnitude" | "back_magnitude" | "front_stiffness" | "back_stiffness" | "interpolation" | "division_spacing" | "reverse_spine_direction" | "axial_rotation" | "front_blend" | "back_blend" | "thickness_scale" | "thickness_ramp" | "thickness_attribute" | "twist_range_half_width" | "local_twist_ramp" | "twist_attribute") { "Spine Control" }
            else if name.ends_with("_attribute") { "Local Control" }
            else { "Extrusion" });
    }
    if !deform_gizmo::is_deformer(typ) { return None; }
    Some(if matches!(name, "group" | "outside" | "max_points") { "Selection" }
        else if matches!(name, "origin" | "center" | "direction" | "up" | "length" | "radius" | "axis" | "frame" | "offset" | "stretch" | "symmetric" | "up_vector_angle") { "Capture" }
        else { "Deformation" })
}

pub fn param_section<'a>(typ: &str, name: &str, meta: Option<&'a Value>) -> Option<&'a str> {
    meta.and_then(|m| m["section"].as_str()).or_else(|| deformer_section(typ, name))
}

#[cfg(test)]
mod deformer_group_tests {
    use super::{deformer_section, param_section};
    use serde_json::json;
    #[test]
    fn capture_and_deformation_fields_are_grouped() {
        assert_eq!(deformer_section("bend", "origin"), Some("Capture"));
        assert_eq!(deformer_section("linear_taper", "taper_x"), Some("Deformation"));
        assert_eq!(deformer_section("point_deform", "group"), Some("Selection"));
        assert_eq!(deformer_section("box", "size"), None);
        assert_eq!(param_section("bend", "origin", Some(&json!({"section": "Manifest"}))), Some("Manifest"));
        assert_eq!(param_section("bend", "origin", None), Some("Capture"));
    }
}

#[cfg(test)]
mod polyextrude_panel_tests {
    use super::*;
    #[test]
    fn polyextrude_metadata_has_spine_local_and_houdini_output_group_names() {
        assert_eq!(deformer_section("polyextrude", "thickness_ramp"), Some("Spine Control"));
        assert_eq!(deformer_section("polyextrude", "distance_scale_attribute"), Some("Local Control"));
        assert_eq!(deformer_section("polyextrude", "outgrp_front_name"), Some("Output Geometry and Groups"));
        assert_eq!(param_meta("polyextrude", "offset_even").unwrap()["default"], json!(true));
        assert_eq!(param_meta("polyextrude", "outgrp_front_name").unwrap()["default"], json!("extrudeFront"));
        assert_eq!(param_meta("polyextrude", "outgrp_back_name").unwrap()["default"], json!("extrudeBack"));
        assert_eq!(param_meta("polyextrude", "outgrp_side_name").unwrap()["default"], json!("extrudeSide"));
    }
}

pub fn unsupported(ui: &mut egui::Ui, v: &View<'_>, id: &str, typ: &str) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.), Sense::hover());
    ui.painter().rect_filled(Rect::from_min_size(r.min + Vec2::new(12., 18.), Vec2::splat(10.)), 2., v.theme.dim);
    paint_text(ui, r.min + Vec2::new(30., 23.), id, 13., true, v.theme.text, Align2::LEFT_CENTER);
    paint_text(ui, Pos2::new(r.right() - 12., r.top() + 23.), typ, 10., true, v.theme.dim, Align2::RIGHT_CENTER);
    let (warn, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 90.), Sense::hover());
    let box_rect = Rect::from_min_max(warn.min + Vec2::new(12., 0.), warn.max - Vec2::new(12., 0.));
    ui.painter().rect(box_rect, 6., v.theme.warn_bg, Stroke::new(1.0_f32, v.theme.warn), StrokeKind::Inside);
    paint_text(ui, box_rect.min + Vec2::new(10., 13.), "Unsupported node type", 12., false, v.theme.text, Align2::LEFT_TOP);
    paint_text(ui, box_rect.min + Vec2::new(10., 35.), "This build has no implementation.", 11., false, v.theme.dim, Align2::LEFT_TOP);
    paint_text(ui, box_rect.min + Vec2::new(10., 53.), "The node is saved unchanged.", 11., false, v.theme.dim, Align2::LEFT_TOP);
    ui.add_space(12.);
    ui.colored_label(v.theme.dim, "STORED PARAMETERS (READ ONLY)");
    if let Some(params) = v.doc["nodes"][id]["params"].as_object() {
        for (name, value) in params {
            let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.), Sense::hover());
            let row = Rect::from_min_max(row.min + Vec2::new(12., 0.), row.max - Vec2::new(12., 0.));
            ui.painter().rect_filled(row, 0., v.theme.input);
            paint_text(ui, row.min + Vec2::new(8., 11.), &format!("{name}: {value}"), 11., true, v.theme.dim, Align2::LEFT_CENTER);
        }
    }
}

pub fn modifier_inputs_panel(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, node: &str) {
    use minihou_app::modifier_inputs::{validate, KINDS};
    let entries = v.doc["nodes"][node]["modifier_inputs"].as_array().cloned().unwrap_or_default();
    ui.separator();
    ui.label("Blender inputs");
    for (index, entry) in entries.iter().enumerate() {
        let ident = entry["id"].as_str().unwrap_or("");
        let key = egui::Id::new(("modifier_input_name", node, ident));
        let mut name = ui.ctx().data_mut(|d| d.get_persisted::<String>(key)).unwrap_or_else(|| entry["name"].as_str().unwrap_or("").to_owned());
        ui.horizontal(|ui| {
            let response = ui.add(egui::TextEdit::singleline(&mut name).desired_width(110.));
            if response.changed() { ui.ctx().data_mut(|d| d.insert_persisted(key, name.clone())); }
            if response.lost_focus() && name != entry["name"] {
                let mut next = entries.clone(); next[index]["name"] = json!(name.trim());
                if validate(&next).is_ok() {
                    out.push(UiAction::Edit { edit: Edit::ModifierInputs { node: node.into(), inputs: next }, continuous: false });
                }
            }
            ui.label(entry["kind"].as_str().unwrap_or("?"));
            for (label, dest) in [("↑", index.checked_sub(1)), ("↓", (index + 1 < entries.len()).then_some(index + 1))] {
                if ui.add_enabled(dest.is_some(), egui::Button::new(label)).clicked() {
                    let mut next = entries.clone(); next.swap(index, dest.unwrap());
                    out.push(UiAction::Edit { edit: Edit::ModifierInputs { node: node.into(), inputs: next }, continuous: false });
                }
            }
            if ui.button("×").on_hover_text("Remove input").clicked() {
                let mut next = entries.clone(); next.remove(index);
                out.push(UiAction::Edit { edit: Edit::ModifierInputs { node: node.into(), inputs: next }, continuous: false });
            }
        });
        ui.small(format!("id: {ident} · default: {}", entry["default"]));
    }
    let key = egui::Id::new(("modifier_input_draft", node));
    let mut draft = ui.ctx().data_mut(|d| d.get_persisted::<String>(key)).unwrap_or_default();
    let kind_key = egui::Id::new(("modifier_input_kind", node));
    let mut kind = ui.ctx().data_mut(|d| d.get_persisted::<String>(kind_key)).unwrap_or_else(|| "float".into());
    let default_key = egui::Id::new(("modifier_input_default", node));
    let mut default = ui.ctx().data_mut(|d| d.get_persisted::<String>(default_key)).unwrap_or_default();
    let min_key = egui::Id::new(("modifier_input_min", node));
    let mut min = ui.ctx().data_mut(|d| d.get_persisted::<String>(min_key)).unwrap_or_default();
    let max_key = egui::Id::new(("modifier_input_max", node));
    let mut max = ui.ctx().data_mut(|d| d.get_persisted::<String>(max_key)).unwrap_or_default();
    let subtype_key = egui::Id::new(("modifier_input_subtype", node));
    let mut subtype = ui.ctx().data_mut(|d| d.get_persisted::<String>(subtype_key)).unwrap_or_default();
    let error_key = egui::Id::new(("modifier_input_error", node));
    let mut error = ui.ctx().data_mut(|d| d.get_persisted::<String>(error_key)).unwrap_or_default();
    ui.horizontal(|ui| { ui.label("Name"); ui.text_edit_singleline(&mut draft); });
    ui.horizontal(|ui| {
        ui.label("Type");
        egui::ComboBox::from_id_salt((node, "input_type")).selected_text(&kind).show_ui(ui, |ui| {
            for option in KINDS { ui.selectable_value(&mut kind, (*option).to_owned(), *option); }
        });
    });
    ui.horizontal(|ui| { ui.label("Default JSON"); ui.text_edit_singleline(&mut default); });
    if matches!(kind.as_str(), "float" | "int" | "vector" | "rotation") {
        ui.horizontal(|ui| { ui.label("Min"); ui.text_edit_singleline(&mut min); ui.label("Max"); ui.text_edit_singleline(&mut max); });
    }
    if matches!(kind.as_str(), "vector" | "float" | "int" | "string" | "menu") {
        ui.horizontal(|ui| { ui.label(if kind == "menu" { "Options (comma separated)" } else { "Subtype" }); ui.text_edit_singleline(&mut subtype); });
    }
    for (key, value) in [(key, &draft), (kind_key, &kind), (default_key, &default), (min_key, &min), (max_key, &max), (subtype_key, &subtype)] {
        ui.ctx().data_mut(|d| d.insert_persisted(key, value.clone()));
    }
    if ui.button("Add input").clicked() {
        // Same path as Blender's input_define_v1 (ids, collision suffixes, validation).
        let options: Vec<String> = subtype.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
        let parsed = if default.trim().is_empty() { Ok(None) } else { serde_json::from_str::<Value>(&default).map(Some) };
        match parsed {
            Ok(value) => {
                let draft = minihou_app::input_define::Draft { name: draft.trim().to_owned(), kind: kind.clone(), default: value,
                    min: min.trim().parse().ok(), max: max.trim().parse().ok(),
                    subtype: if kind == "menu" { String::new() } else { subtype.trim().to_owned() }, options, doc: String::new() };
                match minihou_app::input_define::add_entry(v.doc, node, &draft) {
                    Ok((next, _)) => {
                        out.push(UiAction::Edit { edit: Edit::ModifierInputs { node: node.into(), inputs: next }, continuous: false });
                        error.clear();
                        ui.ctx().data_mut(|d| { d.insert_persisted(key, String::new()); d.insert_persisted(default_key, String::new()); });
                    }
                    Err(e) => error = e.to_string(),
                }
            }
            Err(_) => error = "Default must be valid JSON".into(),
        }
        ui.ctx().data_mut(|d| d.insert_persisted(error_key, error.clone()));
    }
    if !error.is_empty() { ui.colored_label(v.theme.err, error); }
    ui.small("Link a parameter with this input's stable id in the node parameter menu.");
}

pub fn controller_panel(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str) {
    let node = &v.doc["nodes"][id];
    let mut sections = node["controller_sections"].as_array().cloned().unwrap_or_default();
    let mut changed = false;
    ui.heading(format!("Controller: {id}"));
    ui.small("Use ch('../controller/parameter') to drive another node.");
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading("Parameters");
        let parameters = minihou_core::controller::parameters(node);
        if parameters.is_empty() { ui.small("Add a section and parameter below to create a control."); }
        for section in node["controller_sections"].as_array().into_iter().flatten() {
            if let Some(name) = section["name"].as_str() { ui.label(egui::RichText::new(name).strong()); }
            for entry in section["parameters"].as_array().into_iter().flatten() {
            let Some(name) = entry["name"].as_str() else { continue; };
            let kind = entry["kind"].as_str().unwrap_or("float");
            let old = node["params"].get(name).unwrap_or(&entry["default"]);
            let mut value = old.clone();
            ui.push_id(("controller_value", name), |ui| {
                ui.horizontal(|ui| {
                    ui.label(name);
                    let edited = match kind {
                        "float" => { let mut n=value.as_f64().unwrap_or(0.); let yes=ui.add(egui::DragValue::new(&mut n).speed(0.01)).changed(); value=json!(n); yes },
                        "int" => { let mut n=value.as_i64().unwrap_or(0); let yes=ui.add(egui::DragValue::new(&mut n)).changed(); value=json!(n); yes },
                        "bool" => { let mut b=value.as_bool().unwrap_or(false); let yes=ui.checkbox(&mut b, "").changed(); value=json!(b); yes },
                        "vector" | "color" => { let mut any=false; let count=if kind=="vector" {3} else {4}; let mut xs=(0..count).map(|i|value[i].as_f64().unwrap_or(0.)).collect::<Vec<_>>(); for x in &mut xs {any|=ui.add(egui::DragValue::new(x).speed(0.01)).changed();} value=json!(xs); any },
                        "menu" => { let mut s=value.as_str().unwrap_or("").to_owned(); let before=s.clone(); egui::ComboBox::from_id_salt("choice").selected_text(&s).show_ui(ui, |ui| {for option in entry["options"].as_array().into_iter().flatten().filter_map(Value::as_str) {ui.selectable_value(&mut s,option.into(),option);}}); value=json!(s); s!=before },
                        _ => {let mut s=value.as_str().unwrap_or("").to_owned(); let yes=ui.text_edit_singleline(&mut s).changed(); value=json!(s); yes},
                    };
                    if edited { out.push(UiAction::Edit {edit: Edit::Param {node:id.into(),name:name.into(),value},continuous:true}); }
                });
            });
            }
        }
        ui.separator();
        ui.heading("Edit Sections");
        let section_drag_key=egui::Id::new(("controller_section_drag",id));
        let mut section_dragging=ui.ctx().data(|d|d.get_temp::<usize>(section_drag_key));
        let mut section_move=None;
        for si in 0..sections.len() {
            ui.push_id(si, |ui| {
                ui.separator();
                let mut title = sections[si]["name"].as_str().unwrap_or("").to_owned();
                let heading=ui.horizontal(|ui| {
                    let handle=ui.add(egui::Label::new("☰").sense(Sense::drag()));
                    if handle.drag_started() {section_dragging=Some(si);}
                    if ui.text_edit_singleline(&mut title).changed() && !title.trim().is_empty() {
                        sections[si]["name"] = json!(title.trim()); changed = true;
                    }
                    if ui.add_enabled(si > 0, egui::Button::new("↑")).clicked() { sections.swap(si, si-1); changed = true; }
                    if ui.add_enabled(si+1 < sections.len(), egui::Button::new("↓")).clicked() { sections.swap(si, si+1); changed = true; }
                    if ui.button("Remove section").clicked() { sections.remove(si); changed = true; }
                });
                if ui.ctx().input(|i|i.pointer.any_released()) && heading.response.hovered() {
                    if let Some(from)=section_dragging {if from!=si {section_move=Some((from,si));}}
                }
                if si >= sections.len() { return; }
                let mut entries = sections[si]["parameters"].as_array().cloned().unwrap_or_default();
                let drag_key=egui::Id::new(("controller_drag",id,si));
                let mut dragging=ui.ctx().data(|d|d.get_temp::<usize>(drag_key));
                let mut move_to=None;
                for pi in 0..entries.len() {
                    ui.push_id(pi, |ui| {
                        let name = entries[pi]["name"].as_str().unwrap_or("").to_owned();
                        let kind = entries[pi]["kind"].as_str().unwrap_or("float").to_owned();
                        let row=ui.horizontal(|ui| {
                            let handle=ui.add(egui::Label::new("☰").sense(Sense::drag()));
                            if handle.drag_started() {dragging=Some(pi);}
                            ui.label(&name);
                            ui.small(&kind);
                            let mut expose = entries[pi]["expose_to_blender"].as_bool().unwrap_or(false);
                            if matches!(kind.as_str(),"float"|"int"|"bool"|"vector"|"color") && ui.checkbox(&mut expose,"Expose to Blender").changed() {
                                entries[pi]["expose_to_blender"]=json!(expose); changed=true;
                            }
                            if ui.add_enabled(pi>0,egui::Button::new("↑")).clicked() {entries.swap(pi,pi-1);changed=true;}
                            if ui.add_enabled(pi+1<entries.len(),egui::Button::new("↓")).clicked() {entries.swap(pi,pi+1);changed=true;}
                            if ui.button("×").clicked() {entries.remove(pi);changed=true;}
                        });
                        if ui.ctx().input(|i|i.pointer.any_released()) && row.response.hovered() {
                            if let Some(from)=dragging {if from!=pi {move_to=Some((from,pi));}}
                        }
                        if pi < entries.len() && kind == "menu" {
                            let mut options=entries[pi]["options"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
                            ui.horizontal(|ui| {ui.label("Choices");
                                if ui.text_edit_singleline(&mut options).lost_focus() {
                                    let choices=options.split(',').map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>();
                                    if !choices.is_empty() {
                                        entries[pi]["options"]=json!(choices);
                                        if !choices.iter().any(|s|entries[pi]["default"]==*s) {entries[pi]["default"]=json!(choices[0]);}
                                        changed=true;
                                    }
                                }
                            });
                        }
                    });
                    if pi>=entries.len() {break;}
                }
                if let Some((from,to))=move_to {let entry=entries.remove(from);entries.insert(to,entry);changed=true;}
                if ui.ctx().input(|i|i.pointer.any_released()) {dragging=None;}
                ui.ctx().data_mut(|d|if let Some(from)=dragging {d.insert_temp(drag_key,from)} else {d.remove::<usize>(drag_key)});
                sections[si]["parameters"]=json!(entries);
                let key=egui::Id::new(("controller_draft",id,si));
                let mut draft=ui.ctx().data_mut(|d|d.get_persisted::<String>(key)).unwrap_or_default();
                let kind_key=key.with("kind");
                let mut kind=ui.ctx().data_mut(|d|d.get_persisted::<String>(kind_key)).unwrap_or_else(||"float".into());
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut draft);
                    egui::ComboBox::from_id_salt("new_kind").selected_text(&kind).show_ui(ui,|ui|for option in minihou_core::controller::KINDS {ui.selectable_value(&mut kind,(*option).into(),*option);});
                    if ui.button("Add parameter").clicked() {
                        let name=draft.trim();
                        if !name.is_empty() && !sections.iter().any(|s|s["parameters"].as_array().into_iter().flatten().any(|p|p["name"]==name)) {
                            let options=if kind=="menu" {json!(["Option"])} else {json!([])};
                            let default=if kind=="menu" {json!("Option")} else {minihou_core::controller::default(&kind)};
                            sections[si]["parameters"].as_array_mut().unwrap().push(json!({"name":name,"kind":kind,"default":default,"options":options,"expose_to_blender":false}));
                            changed=true; draft.clear();
                        }
                    }
                });
                ui.ctx().data_mut(|d|{d.insert_persisted(key,draft);d.insert_persisted(kind_key,kind);});
            });
        }
        if let Some((from,to))=section_move {let entry=sections.remove(from);sections.insert(to,entry);changed=true;}
        if ui.ctx().input(|i|i.pointer.any_released()) {section_dragging=None;}
        ui.ctx().data_mut(|d|if let Some(from)=section_dragging {d.insert_temp(section_drag_key,from)} else {d.remove::<usize>(section_drag_key)});
        if ui.button("Add section").clicked() {
            let mut i=sections.len()+1;
            while sections.iter().any(|s|s["name"]==format!("Section {i}")) {i+=1;}
            sections.push(json!({"name":format!("Section {i}"),"parameters":[]}));changed=true;
        }
    });
    if changed {
        let mut next=v.doc.clone();next["nodes"][id]["controller_sections"]=json!(sections);
        let kept:std::collections::HashSet<String>=minihou_core::controller::parameters(&next["nodes"][id]).iter().filter_map(|e|e["name"].as_str().map(str::to_owned)).collect();
        if let Some(values)=next["nodes"][id].get_mut("params").and_then(Value::as_object_mut) {values.retain(|name,_|kept.contains(name));}
        if let Some(channels)=next["nodes"][id].get_mut("channels").and_then(Value::as_object_mut) {channels.retain(|name,_|kept.contains(name.split('.').next().unwrap_or("")));}
        if minihou_core::controller::validate(&next["nodes"][id]).is_ok() {
            out.push(UiAction::Edit {edit:Edit::Replace(next),continuous:false});
        }
    }
}

#[cfg(test)]
mod controller_panel_tests {
    use super::*;
    use crate::ui::testing::Fixture;

    #[test]
    fn controller_values_are_visible_in_parameter_panel() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({"nodes":{"driver":{"type":"controller",
            "controller_sections":[{"name":"Shape","parameters":[{"name":"width","kind":"float","default":1.0}]}],
            "params":{"width":2.0}}},"display":"driver"}).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "driver".into();
        f.display = "driver".into();
        let ctx = egui::Context::default();
        let mut out = Actions::default();
        let frame = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| super::super::show(ui, &f.view(), &mut out));
        });
        assert!(frame.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.text() == "Parameters")));
        assert!(frame.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.text() == "width")));
        assert!(frame.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.text() == "Shape")));
        assert!(out.0.is_empty());
    }

    #[test]
    fn add_section_click_emits_persisted_controller_section() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({"nodes":{"controller1":{"type":"controller"}},"display":"controller1"}).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "controller1".into();
        f.display = "controller1".into();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let mut click = None;
        let mut actions = Actions::default();
        for phase in 0..3 {
            let events = match (phase, click) {
                (1, Some(pos)) => vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }],
                (2, Some(pos)) => vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 600.))), events, ..Default::default() };
            let output = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| super::super::show(ui, &f.view(), &mut actions));
            });
            if phase == 0 {
                click = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) if t.galley.text() == "Add section" => Some(t.pos + t.galley.size() * 0.5),
                    _ => None,
                });
            }
        }
        assert!(click.is_some(), "Add section button must be drawn");
        let next = actions.0.into_iter().find_map(|action| match action {
            UiAction::Edit { edit: Edit::Replace(next), .. } => Some(next),
            _ => None,
        }).expect("Add section click must emit a document edit");
        assert_eq!(next["nodes"]["controller1"]["controller_sections"], json!([{"name":"Section 1","parameters":[]}]));
        assert!(next["nodes"]["controller1"].get("channels").is_none());
        f.doc = minihou_app::Document::parse(next.to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        let rendered = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| super::super::show(ui, &f.view(), &mut Actions::default()));
        });
        assert!(rendered.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.text() == "Section 1")));
    }
}

/// A subnet's public numeric controls live in its own `params`, with a mapping
/// to the target node inside `inner.nodes`.
pub fn promoted_panel(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str) {
    ui.heading(format!("Subnet: {id}"));
    let subnet=&v.doc["nodes"][id];
    let mut candidates=Vec::<(String,String,Value)>::new();
    if let Some(inner)=subnet["inner"]["nodes"].as_object() {
        for (target,node) in inner {
            if let Some(spec)=node["type"].as_str().and_then(nodes::get) {
                for param in spec.params {
                    let value=node["params"].get(param.name).cloned().unwrap_or_else(||(param.default)());
                    if value.is_number() {candidates.push((target.clone(),param.name.into(),value));}
                    else if let Some(xs)=value.as_array().filter(|xs|xs.len()==3 && xs.iter().all(Value::is_number)) {
                        for (i,axis) in ["x","y","z"].iter().enumerate() {candidates.push((target.clone(),format!("{}.{}",param.name,axis),xs[i].clone()));}
                    }
                }
            }
        }
    }
    let choice_id=egui::Id::new(("promote_choice",id));
    let mut choice=ui.ctx().data(|d|d.get_temp::<usize>(choice_id)).unwrap_or(0).min(candidates.len().saturating_sub(1));
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(choice_id).selected_text(candidates.get(choice).map(|(node,param,_)|format!("{node}.{param}")).unwrap_or_else(||"No numeric parameters".into()))
            .show_ui(ui,|ui|for (i,(node,param,_)) in candidates.iter().enumerate(){ui.selectable_value(&mut choice,i,format!("{node}.{param}"));});
        if ui.add_enabled(!candidates.is_empty(),egui::Button::new("Promote")).clicked() {
            let (target,param,value)=&candidates[choice];
            let mut next=v.doc.clone();
            let node=&mut next["nodes"][id];
            let mut name=param.replace('.',"_");
            if nodes::subnet_public_params(node).iter().any(|e|e["name"]==name) {
                let base=name.clone(); let mut i=2;
                while nodes::subnet_public_params(node).iter().any(|e|e["name"]==name) {name=format!("{base}_{i}");i+=1;}
            }
            if !node["promoted_params"].is_array(){node["promoted_params"]=json!([]);}
            node["promoted_params"].as_array_mut().unwrap().push(json!({"name":name.clone(),"node":target,"param":param,"default":value,"min":0.0,"max":10.0}));
            node["params"][&name]=value.clone();
            out.push(UiAction::Edit {edit:Edit::Replace(next),continuous:false});
        }
    });
    ui.ctx().data_mut(|d|d.insert_temp(choice_id,choice));
    if let Some(inner)=subnet["inner"]["nodes"].as_object() {
        for (boundary, _) in inner.iter().filter(|(_, node)| matches!(node["type"].as_str(), Some("input" | "output" | "subnet_input" | "subnet_output"))) {
            ui.horizontal(|ui| {
                ui.label(format!("{boundary} parameters"));
                if ui.add_enabled(!candidates.is_empty(), egui::Button::new("Add parameter")).clicked() {
                    let (target,param,value)=&candidates[choice];
                    if let Some(next)=add_boundary_param(v.doc,id,boundary,target,param,value) {
                        out.push(UiAction::Edit {edit:Edit::Replace(next),continuous:false});
                    }
                }
            });
        }
    }
    for entry in nodes::subnet_public_params(subnet) {
        let Some(name)=entry["name"].as_str() else {continue};
        let default=entry.get("default").cloned().unwrap_or(json!(0.0));
        let old=subnet["params"].get(name).unwrap_or(&default);
        if let Some(values)=old.as_array().filter(|values| values.len()==3 && values.iter().all(Value::is_number)) {
            let mut numbers: Vec<f64>=values.iter().map(|value| value.as_f64().unwrap()).collect();
            let changed=ui.horizontal(|ui| {
                ui.label(entry["label"].as_str().unwrap_or(name));
                let mut changed=false;
                for number in &mut numbers { changed |= ui.add(egui::DragValue::new(number).speed(0.01)).changed(); }
                changed
            }).inner;
            if changed {
                let mut next=v.doc.clone(); next["nodes"][id]["params"][name]=json!(numbers);
                out.push(UiAction::Edit {edit:Edit::Replace(next),continuous:true});
            }
            continue;
        }
        if let Some(mut checked)=old.as_bool() {
            if ui.checkbox(&mut checked,entry["label"].as_str().unwrap_or(name)).changed() {
                let mut next=v.doc.clone(); next["nodes"][id]["params"][name]=json!(checked);
                out.push(UiAction::Edit {edit:Edit::Replace(next),continuous:false});
            }
            continue;
        }
        let Some(mut number)=old.as_f64() else {continue};
        let min=entry["min"].as_f64().unwrap_or(0.0);
        let max=entry["max"].as_f64().unwrap_or(10.0).max(min+f64::EPSILON);
        ui.horizontal(|ui| {
            ui.label(entry["label"].as_str().unwrap_or(name));
            let slider=ui.add(egui::Slider::new(&mut number,min..=max).show_value(false));
            let field=ui.add(egui::DragValue::new(&mut number).speed((max-min)/200.0));
            if slider.changed() || field.changed() {
                out.push(promoted_value_edit(v.doc,id,name,number,slider.dragged() || field.dragged()));
            }
            let channel=&subnet["channels"][name];
            let key=egui::Id::new(("promoted_expression",id,name));
            if ui.button(if channel.is_string() {"ƒx ●"} else {"ƒx"}).on_hover_text("Channel expression").clicked() {
                ui.ctx().data_mut(|d|d.insert_temp(key,channel.as_str().unwrap_or("").to_owned()));
            }
            if let Some(mut expr)=ui.ctx().data(|d|d.get_temp::<String>(key)) {
                let mut open=true;
                let mut applied=false;
                egui::Window::new(format!("Expression: {id}.{name}")).id(key.with("window")).open(&mut open).show(ui.ctx(),|ui| {
                    ui.text_edit_singleline(&mut expr);
                    if ui.button("Apply").clicked() {
                        let mut next=v.doc.clone();
                        if expr.trim().is_empty() {next["nodes"][id]["channels"].as_object_mut().map(|c|c.remove(name));}
                        else {next["nodes"][id]["channels"][name]=json!(expr);}
                        out.push(UiAction::Edit {edit:Edit::Replace(next),continuous:false});
                        applied=true;
                    }
                });
                if open && !applied {ui.ctx().data_mut(|d|d.insert_temp(key,expr));}else{ui.ctx().data_mut(|d|d.remove::<String>(key));}
            }
        });
    }
}

fn add_boundary_param(doc: &Value, id: &str, boundary: &str, target: &str, param: &str, default: &Value) -> Option<Value> {
    let mut next=doc.clone();
    let subnet=&mut next["nodes"][id];
    let existing=nodes::subnet_public_params(subnet);
    let base=param.replace('.',"_");
    let mut name=base.clone();
    let mut suffix=2;
    while existing.iter().any(|entry| entry["name"] == name) {
        name=format!("{base}_{suffix}"); suffix+=1;
    }
    let node=subnet["inner"]["nodes"].get_mut(boundary)?;
    if !matches!(node["type"].as_str(), Some("input" | "output" | "subnet_input" | "subnet_output")) { return None; }
    if !node["custom_params"].is_array() {node["custom_params"]=json!([]);}
    node["custom_params"].as_array_mut()?.push(json!({"name":name,"node":target,"param":param,"default":default,"min":0.0,"max":10.0}));
    Some(next)
}

pub fn promoted_value_edit(doc: &Value, id: &str, name: &str, value: f64, continuous: bool) -> UiAction {
    let mut next=doc.clone(); next["nodes"][id]["params"][name]=json!(value);
    UiAction::Edit {edit:Edit::Replace(next),continuous}
}

#[cfg(test)]
mod subnet_parameter_tests {
    use super::*;
    use crate::ui::testing::Fixture;
    #[test]
    fn promoted_slider_edit_writes_outer_param() {
        let doc=json!({"nodes":{"subnet1":{"type":"subnet","params":{"width":1.0},"inner":{"nodes":{"box1":{"type":"box"}},"output":"box1"}}}});
        let UiAction::Edit {edit:Edit::Replace(next),continuous:true}=promoted_value_edit(&doc,"subnet1","width",4.0,true) else {panic!("expected continuous document edit")};
        assert_eq!(next["nodes"]["subnet1"]["params"]["width"],json!(4.0));
        assert_eq!(doc["nodes"]["subnet1"]["params"]["width"],json!(1.0));
    }
    #[test]
    fn boundary_parameter_is_visible_and_can_be_added() {
        let doc=json!({"display":"sub","nodes":{"sub":{"type":"subnet","inner":{"nodes":{
            "input1":{"type":"subnet_input","port_index":0,"custom_params":[
                {"name":"offset","node":"move","param":"translate.x","default":1.0}]},
            "move":{"type":"transform","inputs":{"geo":"input1"}},
            "output1":{"type":"subnet_output","port_index":0,"inputs":{"geo":"move"}}
        },"output":"output1"},"input_sockets":[{"index":0,"name":"Input 1"}],
            "output_sockets":[{"index":0,"name":"Output 1"}]}}});
        let mut f=Fixture::new();
        f.doc=minihou_app::Document::parse(doc.to_string()).unwrap();
        f.graph=minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected="sub".into();
        let ctx=egui::Context::default();
        let mut out=Actions::default();
        let frame=ctx.run(egui::RawInput::default(),|ctx| {
            egui::CentralPanel::default().show(ctx,|ui| promoted_panel(ui,&f.view(),&mut out,"sub"));
        });
        assert!(frame.shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(t) if t.galley.text()=="offset")));
        let next=add_boundary_param(&doc,"sub","output1","move","translate.y",&json!(2.0)).unwrap();
        assert_eq!(next["nodes"]["sub"]["inner"]["nodes"]["output1"]["custom_params"][0]["name"],"translate_y");
        assert_eq!(nodes::subnet_public_params(&next["nodes"]["sub"]).len(),2);
    }
}

pub fn redo_param(out: &mut Actions, id: &str, name: &str, value: Value, continuous: bool) {
    out.push(UiAction::Edit { edit: Edit::Param { node: id.into(), name: name.into(), value }, continuous });
}

/// Selection Domain dropdown of Attribute Create. A change converts the node's selection group to the new domain.
pub fn attribute_domain_row(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, locked: bool) {
    use super::attribute_model::{domain_label, DOMAINS};
    let current = v.doc["nodes"][id]["params"]["domain"].as_str().unwrap_or("points").to_owned();
    let mut next = current.clone();
    ui.add_enabled_ui(!locked, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(12.);
            ui.label("Domain");
            egui::ComboBox::from_id_salt((id, "attribute_create_domain")).selected_text(domain_label(&current))
                .show_ui(ui, |ui| {
                    for (value, label) in DOMAINS { ui.selectable_value(&mut next, value.to_owned(), label); }
                });
        });
    });
    if next != current { out.push(UiAction::SetAttributeDomain { node: id.into(), domain: next }); }
}

pub fn attribute_create_panel(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, locked: bool) {
    use super::attribute_model::{blank_entry, channels, domain_label, entry_domain, set_alpha, set_type, resolve, DOMAINS, TYPES};
    let mut attrs = resolve(&v.doc["nodes"][id]["params"]);
    let before = attrs.clone();
    ui.add_space(5.);
    ui.label(egui::RichText::new("Attributes").strong().color(v.theme.accent));
    ui.add_enabled_ui(!locked, |ui| {
        let mut remove = None;
        for (i, a) in attrs.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                if i > 0 { ui.separator(); }
                ui.horizontal(|ui| { ui.strong(format!("Attribute {}", i + 1)); if ui.button("Remove").clicked() { remove = Some(i); } });
                let mut name = a["name"].as_str().unwrap_or("").to_owned();
                ui.horizontal(|ui| {
                    ui.label("Name");
                    if ui.text_edit_singleline(&mut name).changed() { a["name"] = json!(name); }
                    // The attribute name stays a name; picking beside it fills the node's input group.
                    if let Some(mode) = super::pick_domain_for("attribute_create", "group",
                        param_meta("attribute_create", "group"), &json!({"attributes":[a.clone()]})) {
                        let active = ui.ctx().data(|d| d.get_temp::<(String, String)>(egui::Id::new("param_pick_active")))
                            .is_some_and(|(node, field)| node == id && field == "group");
                        let arrow = super::pick::pick_button(ui, active, !locked);
                        if arrow.clicked() {
                            out.push(UiAction::PickParameter { node: id.to_owned(), name: "group".into(), mode, pairs: false });
                        }
                    }
                });
                let domain = entry_domain(a).to_owned();
                let mut next_domain = domain.clone();
                ui.horizontal(|ui| {
                    ui.label("Domain");
                    egui::ComboBox::from_id_salt("domain").selected_text(domain_label(&domain)).show_ui(ui, |ui| {
                        for (value, label) in DOMAINS { ui.selectable_value(&mut next_domain, value.to_owned(), label); }
                        // Stored graphs may hold vertices or detail; keep showing them.
                        if !DOMAINS.iter().any(|(value, _)| *value == domain) { ui.selectable_value(&mut next_domain, domain.clone(), &domain); }
                    });
                });
                if next_domain != domain {
                    if let Some(object) = a.as_object_mut() { object.remove("class"); }
                    a["domain"] = json!(next_domain);
                }
                let ty = a["type"].as_str().unwrap_or("float").to_owned();
                let mut next_ty = ty.clone();
                ui.horizontal(|ui| {
                    ui.label("Type");
                    egui::ComboBox::from_id_salt("type").selected_text(&ty).show_ui(ui, |ui| {
                        for option in TYPES { ui.selectable_value(&mut next_ty, option.to_owned(), option); }
                        if !TYPES.contains(&ty.as_str()) { ui.selectable_value(&mut next_ty, ty.clone(), &ty); }
                    });
                });
                if next_ty != ty { set_type(a, &next_ty); }
                let channel_count = channels(a);
                if a["type"] == "string" {
                    let mut value = a["value"].as_str().unwrap_or("").to_owned();
                    ui.horizontal(|ui| { ui.label("Value"); if ui.text_edit_singleline(&mut value).changed() { a["value"] = json!(value); } });
                } else {
                    let mut values = a["value"].as_array().cloned().unwrap_or_default();
                    values.resize(channel_count.max(values.len()), json!(0.0));
                    let mut touched = false;
                    let mut alpha = channel_count == 4;
                    ui.horizontal(|ui| {
                        ui.label("Value");
                        let is_colour = a["type"] == "color";
                        if is_colour {
                            // The chip opens the colour selector; the channel boxes stay for exact numbers.
                            let c: Vec<f32> = values.iter().map(|x| x.as_f64().unwrap_or(0.) as f32).collect();
                            let _changed = if alpha {
                                let mut rgba = [c[0], c[1], c[2], c[3]];
                                let r = ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed();
                                if r { touched = true; for (k, x) in rgba.iter().enumerate() { values[k] = json!(*x as f64); } }
                                r
                            } else {
                                let mut rgb = [c[0], c[1], c[2]];
                                let r = ui.color_edit_button_rgb(&mut rgb).changed();
                                if r { touched = true; for (k, x) in rgb.iter().enumerate() { values[k] = json!(*x as f64); } }
                                r
                            };
                        }
                        let int = a["type"] == "int";
                        for component in values.iter_mut().take(channel_count) {
                            let mut x = component.as_f64().unwrap_or(0.);
                            let drag = if int { egui::DragValue::new(&mut x).speed(0.1).max_decimals(0) } else { egui::DragValue::new(&mut x).speed(0.1) };
                            if ui.add(drag).changed() { touched = true; *component = json!(if int { x.round() } else { x }); }
                        }
                        if is_colour && ui.checkbox(&mut alpha, "A").changed() { set_alpha(a, alpha); values = a["value"].as_array().cloned().unwrap_or_default(); touched = false; }
                    });
                    if touched { values.truncate(channel_count.max(1)); a["value"] = json!(values); }
                }
            });
        }
        if let Some(i) = remove { attrs.remove(i); }
        if !attrs.is_empty() { ui.separator(); }
        if ui.button("Add attribute").clicked() { attrs.push(blank_entry(attrs.len() + 1)); }
    });
    if attrs != before { redo_param(out, id, "attributes", json!(attrs), false); }
}

#[cfg(test)]
mod attribute_create_pick_tests {
    use super::*;
    use crate::ui::testing::Fixture;
    use eframe::egui::{Pos2, Rect, Vec2};

    #[test]
    fn name_row_arrow_picks_group_without_replacing_attribute_name() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({"display":"attr1","nodes":{
            "box1":{"type":"box"},"attr1":{"type":"attribute_create","inputs":{"geo":"box1"},
                "params":{"attributes":[{"name":"my_attr","class":"primitives","type":"float","size":1,
                    "default":[0,0,0,0],"value":[0,0,0,0]}]}}
        }}).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "attr1".into(); f.display = "attr1".into();
        let ctx = egui::Context::default(); f.theme.apply(&ctx);
        let mut click = None;
        let mut actions = Actions::default();
        for phase in 0..3 {
            let events = match (phase, click) {
                (1, Some(pos)) => vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos, button:egui::PointerButton::Primary, pressed:true, modifiers:egui::Modifiers::NONE }],
                (2, Some(pos)) => vec![egui::Event::PointerButton {
                    pos, button:egui::PointerButton::Primary, pressed:false, modifiers:egui::Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(800.,600.))), events, ..Default::default() };
            let output = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                attribute_create_panel(ui, &f.view(), &mut actions, "attr1", false);
            }); });
            if phase == 0 {
                for shape in output.shapes { if let egui::Shape::Text(t) = shape.shape {
                    if t.galley.text() == "↖" { click = Some(t.pos + t.galley.size() * 0.5); }
                }}
            }
        }
        assert!(click.is_some(), "purple arrow must be drawn beside Name");
        assert!(actions.0.iter().any(|a| matches!(a, UiAction::PickParameter {node,name,mode,..}
            if node == "attr1" && name == "group" && *mode == crate::ui::SelectionMode::Faces)));
        assert!(!actions.0.iter().any(|a| matches!(a, UiAction::Edit { edit:Edit::Param {name,..},.. }
            if name == "attributes")));
    }
}

/// The most common Loop Cut adjustments stay visible above the full node parameters.
pub fn loop_cut_redo(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, locked: bool) {
    let params = &v.doc["nodes"][id]["params"];
    let dragging = ui.ctx().input(|i| i.pointer.primary_down());
    ui.add_enabled_ui(!locked, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Loop Cut redo");
            let mut cuts = params["loops"].as_u64().unwrap_or(1) as i64;
            ui.label("Number of Cuts");
            if ui.add(egui::DragValue::new(&mut cuts).range(1..=64).speed(0.1)).changed() {
                redo_param(out, id, "loops", json!(cuts), dragging);
            }
            let mut smoothness = params["smoothness"].as_f64().unwrap_or(0.);
            ui.label("Smoothness");
            if ui.add(egui::DragValue::new(&mut smoothness).range(-1. ..=1.).speed(0.01)).changed() {
                redo_param(out, id, "smoothness", json!(smoothness), dragging);
            }
            let mut falloff = params["falloff"].as_str().unwrap_or("inverse_square").to_owned();
            ui.label("Falloff");
            egui::ComboBox::from_id_salt((id,"loop_cut_redo_falloff")).selected_text(&falloff)
                .show_ui(ui, |ui| { for option in ["inverse_square", "linear", "smooth"] {
                    ui.selectable_value(&mut falloff, option.into(), option);
                } });
            if params["falloff"].as_str().unwrap_or("inverse_square") != falloff {
                redo_param(out, id, "falloff", json!(falloff), false);
            }
            let mut factor = params["factor"].as_f64().unwrap_or(0.);
            ui.label("Factor");
            if ui.add(egui::DragValue::new(&mut factor).range(-1. ..=1.).speed(0.01)).changed() {
                redo_param(out, id, "factor", json!(factor), dragging);
            }
            for (label, name, default) in [("Even", "even", false), ("Flipped", "flipped", false), ("Clamp", "clamp", true)] {
                let mut value = params[name].as_bool().unwrap_or(default);
                if ui.checkbox(&mut value, label).changed() { redo_param(out, id, name, json!(value), false); }
            }
        });
    });
    ui.separator();
}

#[cfg(test)]
mod mesh_cut_redo_tests {
    use super::*;

    #[test]
    fn redo_values_write_to_polysplit_node_params() {
        let mut doc = minihou_app::Document::parse(json!({"display":"cut1","nodes":{
            "box1":{"type":"box"},
            "cut1":{"type":"polysplit","inputs":{"geo":"box1"},"params":{"mode":"edge_loop","split_edges":"p0-1"}}
        }}).to_string()).unwrap();
        let mut actions = Actions::default();
        for (name, value) in [
            ("loops",json!(4)), ("smoothness",json!(0.25)), ("falloff",json!("linear")),
            ("factor",json!(-0.4)), ("even",json!(true)), ("flipped",json!(true)), ("clamp",json!(false)),
        ] { redo_param(&mut actions,"cut1",name,value,false); }
        for action in actions.0 {
            let UiAction::Edit { edit, .. } = action else { panic!("expected parameter edit") };
            doc.edit(&edit).unwrap();
        }
        let params = &doc.value()["nodes"]["cut1"]["params"];
        assert_eq!(params["loops"],4);
        assert_eq!(params["smoothness"],0.25);
        assert_eq!(params["falloff"],"linear");
        assert_eq!(params["factor"],-0.4);
        assert_eq!(params["even"],true);
        assert_eq!(params["flipped"],true);
        assert_eq!(params["clamp"],false);
    }
}

pub fn knife_chains_panel(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, locked: bool) {
    let raw=v.doc["nodes"][id]["params"]["cuts"].as_str().unwrap_or("[]");
    let Ok(mut chains)=serde_json::from_str::<Vec<Value>>(raw) else { ui.colored_label(v.theme.err,"Invalid Knife chains"); return; };
    ui.add_space(5.); ui.horizontal(|ui| {ui.add_space(12.);ui.strong(format!("Cut chains ({})",chains.len()));});
    let mut clear=None;
    for (i,chain) in chains.iter().enumerate() {
        let count=chain.get("points").and_then(Value::as_array).map_or(2,Vec::len);
        ui.horizontal(|ui| {ui.add_space(12.);ui.label(format!("Chain {} · {} points",i+1,count));
            if ui.add_enabled(!locked,egui::Button::new("Clear")).clicked() {clear=Some(i);} });
    }
    if let Some(i)=clear {chains.remove(i);out.push(UiAction::Edit{edit:Edit::Param{node:id.into(),name:"cuts".into(),value:json!(Value::Array(chains).to_string())},continuous:false});}
}

pub fn bezier_controls_panel(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, locked: bool) {
    use curve_tool::{self, Part, Selected};
    let Some(cs)=curve_tool::visible_controls(ui.ctx(),v.doc,id) else {
        ui.colored_label(v.theme.err,"Invalid Bezier control data");return;
    };
    let selected=curve_tool::selected(ui.ctx(),id);
    let index=selected.index.min(cs.len()-1);
    ui.add_space(5.);
    ui.horizontal(|ui| {ui.add_space(12.);ui.strong(format!("Control point {} of {}",index+1,cs.len()));});
    let mut mode=cs[index].mode.clone();
    ui.add_enabled_ui(!locked,|ui| {
        ui.horizontal(|ui| {
            ui.add_space(12.);ui.label("Handle mode");
            egui::ComboBox::from_id_salt((id,"bezier_mode")).selected_text(&mode).show_ui(ui,|ui| {
                for choice in ["AUTO","VECTOR","ALIGNED","MIRRORED","FREE"] {ui.selectable_value(&mut mode,choice.into(),choice);}
            });
        });
        if mode!=cs[index].mode {
            let closed=v.doc["nodes"][id]["params"]["closed"].as_bool().unwrap_or(false);
            let mut next=curve_tool::resolved(&cs,closed);next[index].mode=mode;
            next=curve_tool::resolved(&next,closed);
            out.push(UiAction::Edit{edit:Edit::Param{node:id.into(),name:"controls".into(),value:curve_tool::write(&next)},continuous:false});
        }
        ui.horizontal(|ui| {
            ui.add_space(12.);
            let append=ui.button("Append point").clicked();
            let closed=v.doc["nodes"][id]["params"]["closed"].as_bool().unwrap_or(false);
            let split=ui.add_enabled(closed||index+1<cs.len(),egui::Button::new("Split segment")).clicked();
            let delete=ui.add_enabled(cs.len()>2,egui::Button::new("Delete point")).clicked();
            if append||split||delete {
                let next=if append {curve_tool::append(&cs,[cs.last().unwrap().p[0]+1.,cs.last().unwrap().p[1],cs.last().unwrap().p[2]])}
                    else if split {curve_tool::split(&cs,index,closed)} else {curve_tool::delete(&cs,index)};
                let selected_index=if append {next.len()-1} else if split {index+1} else {index.min(next.len()-1)};
                curve_tool::set_selected(ui.ctx(),id,Selected{index:selected_index,part:Part::Anchor});
                out.push(UiAction::Edit{edit:Edit::Param{node:id.into(),name:"controls".into(),value:curve_tool::write(&next)},continuous:false});
            }
        });
        let mut p=cs[index].p;
        ui.horizontal(|ui| {
            ui.add_space(12.);
            for (axis,label) in ["X","Y","Z"].into_iter().enumerate() {
                ui.label(label);
                let response=ui.add(egui::DragValue::new(&mut p[axis]).speed(0.1).max_decimals(5));
                if response.changed() && p[axis].is_finite() {
                    let next=curve_tool::move_part(&cs,index,Part::Anchor,p);
                    out.push(UiAction::Edit{edit:Edit::Param{node:id.into(),name:"controls".into(),value:curve_tool::write(&next)},continuous:response.dragged()});
                }
            }
        });
    });
}
