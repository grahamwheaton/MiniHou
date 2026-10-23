//! Parameters panel: Main.dc.html `.par` and Params.dc.html.
use super::{theme::Theme, Actions, UiAction, View};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use super::number_step;
use minihou_app::{clamp_parameter, metadata_kind, param_kind, param_meta, slider_range, Edit, ParamKind};
use minihou_core::{nodes, params::ParamSpec};
use serde_json::{json, Value};
use minihou_app::value_link_ui::{self, CopiedParam, PasteMode};

const DISABLED: &str = "The Rust backend does not support it yet";
pub mod attribute_model;
pub mod sections;
pub use sections::*;
pub mod output_groups;
pub use output_groups::{is_output_group_field, output_group_label, output_group_toggle, target_field, TARGET_TEAL};
pub mod output_targets;
pub mod pick;
mod group_fuzzy;
pub use pick::{domain_edit_mode, is_input_group_field, pick_domain, pick_domain_for, pick_field, PICK_PURPLE, SELECT_ORANGE};
pub mod widgets;
pub use widgets::{boxed, color_widget, drag_number, file_widget, number_field, ramp_widget, render_vector_fields, widget, widget_styled, widget_with_units};
pub mod value_links;
pub(crate) use value_links::*;

pub mod header;
pub(crate) use header::*;


#[path = "param_values.rs"]
mod param_values;
#[allow(unused_imports)] // Keep the existing params module names available to callers.
pub use param_values::{parse_color_value, serialize_color_value, is_color_param, is_file_param, is_ramp_param, RampInterpolation, RampValue, RampStop, RampFormat, RampData};

pub mod row;
pub use row::ParamRowLayout;
pub(crate) use row::*;

const SCOPE_KEY: &str = "param_panel_scope";

/// Set (or clear with `None`) the subnet scope the next `show` call resolves against.
pub fn set_scope(ctx: &egui::Context, root: Option<(&serde_json::Value, &[String])>) {
    let scope = root.and_then(|(doc, path)| minihou_core::channels::resolve_scope(doc, path));
    ctx.data_mut(|d| match scope {
        Some(scope) => d.insert_temp(egui::Id::new(SCOPE_KEY), std::sync::Arc::new(scope)),
        None => d.remove::<std::sync::Arc<minihou_core::channels::ScopeResolved>>(egui::Id::new(SCOPE_KEY)),
    });
}

pub(crate) fn promoted_override(ctx: &egui::Context, id: &str, param: &str) -> bool {
    ctx.data(|d| d.get_temp::<std::sync::Arc<minihou_core::channels::ScopeResolved>>(egui::Id::new(SCOPE_KEY)))
        .is_some_and(|scope| scope.overridden.contains(&(id.to_owned(), param.to_owned())))
}

pub fn show(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions) {
    // Inside a subnet the caller stores values resolved against the whole document, so promoted
    // overrides from the subnet show on the inner nodes; see `set_scope`.
    let scope = ui.ctx().data(|d| d.get_temp::<std::sync::Arc<minihou_core::channels::ScopeResolved>>(egui::Id::new(SCOPE_KEY)));
    if let Some(scope) = scope {
        let resolved = minihou_core::channels::ResolvedChannels { params: scope.resolved.params.clone(), diagnostics: scope.resolved.diagnostics.clone(), time_dependent: scope.resolved.time_dependent.clone() };
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("param_panel_resolved"),std::sync::Arc::new(resolved)));
    } else if let Ok(graph)=minihou_core::Graph::from_value(v.doc) {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("param_panel_resolved"),std::sync::Arc::new(minihou_core::channels::resolve_graph(&graph))));
    }
    expression_dialog(ui.ctx(), v, out);
    // No title bar: the dock tab already names the panel.
    let id = v.selected;
    let Some(node) = v.graph.nodes.iter().find(|(name, _)| name == id).map(|(_, node)| node) else {
        ui.add_space(20.);
        ui.colored_label(v.theme.dim, "Nothing selected. Click a node in the network to edit its parameters.");
        return;
    };
    let typ = &node.node_type;
    if matches!(typ.as_str(), "subnet_input" | "subnet_output" | "input" | "output")
        && v.doc["nodes"][id]["port_index"].is_number() {
        ui.heading(if matches!(typ.as_str(), "subnet_input" | "input") { "Subnet Input" } else { "Subnet Output" });
        ui.menu_button("Parameter options", |ui| {
            if ui.button("Add output").clicked() {
                out.push(UiAction::AddSubnetOutput(id.to_owned()));
                ui.close();
            }
        });
        return;
    }
    if typ == "subnet" {
        promoted_panel(ui, v, out, id);
        return;
    }
    if typ == "controller" {
        controller_panel(ui, v, out, id);
        return;
    }
    if nodes::get(typ).is_none() {
        unsupported(ui, v, id, typ);
        return;
    }
    let flags = v.flags_of(id);
    header(ui, v, out, id, typ, node.bypass, flags.locked, flags.template);
    if typ == "polysplit" && v.doc["nodes"][id]["params"]["mode"] == "edge_loop" {
        loop_cut_redo(ui, v, out, id, flags.locked);
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        let (r, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.), Sense::click());
        ui.painter().rect_filled(r, 0., v.theme.panel2);
        ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0_f32, v.theme.stroke));
        ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, v.theme.stroke));
        let key = egui::Id::new(("params_group", typ));
        let mut open = ui.ctx().data_mut(|d| d.get_persisted::<bool>(key).unwrap_or(true));
        if response.clicked() { open = !open; ui.ctx().data_mut(|d| d.insert_persisted(key, open)); }
        let (cx, cy) = (r.left() + 14., r.center().y);
        let chevron: [Pos2; 3] = if open { [Pos2::new(cx - 3., cy - 1.5), Pos2::new(cx, cy + 1.5), Pos2::new(cx + 3., cy - 1.5)] }
            else { [Pos2::new(cx - 1.5, cy - 3.), Pos2::new(cx + 1.5, cy), Pos2::new(cx - 1.5, cy + 3.)] };
        ui.painter().add(egui::Shape::line(chevron.to_vec(), Stroke::new(1.5_f32, v.theme.text)));
        paint_text(ui, Pos2::new(r.left() + 26., r.center().y), &title(typ), 12., false, v.theme.text, Align2::LEFT_CENTER);
        if open {
            ui.add_space(6.);
            let mut errors = Vec::new();
            if let Some(spec) = nodes::get(typ) {
                let outputs: Vec<_> = spec.params.iter().filter(|param| is_output_group_field(param.name, param_meta(typ, param.name))).collect();
                let output_toggles: Vec<_> = outputs.iter().filter_map(|param| output_group_toggle(typ, param.name, param_meta(typ, param.name))).collect();
                let mut section=None;
                if typ == "attribute_create" { attribute_domain_row(ui,v,out,id,flags.locked); }
                for param in spec.params {
                    if typ == "switchif" && matches!(param.name,
                        "mergecondition" | "testinput" | "numtests" | "tests" |
                        "enable" | "test_type" | "type" | "enable_value" | "expr") { continue; }
                    if typ == "bezier" && param.name == "controls" { continue; }
                    if typ == "knife" && param.name == "cuts" { continue; }
                    if typ == "attribute_create" && param.name != "group" { continue; }
                    if outputs.iter().any(|output| output.name == param.name) || output_toggles.contains(&param.name) { continue; }
                    let next=param_section(typ,param.name,param_meta(typ,param.name));
                    if next.is_some() && next!=section {
                        section=next;
                        ui.add_space(5.);
                        ui.label(egui::RichText::new(next.unwrap()).strong().color(v.theme.accent));
                    }
                    ui.push_id(param.name, |ui| row(ui, v, out, id, typ, param, flags.locked, &mut errors));
                }
                if !outputs.is_empty() {
                    ui.add_space(5.);
                    ui.label(egui::RichText::new("Output Groups").strong().color(v.theme.accent));
                    for param in outputs {
                        ui.push_id(param.name, |ui| row(ui, v, out, id, typ, param, flags.locked, &mut errors));
                    }
                }
                output_targets::show(ui, id, spec.outputs, v.theme.accent);
                if typ == "attribute_create" { attribute_create_panel(ui,v,out,id,flags.locked); }
                if typ == "bezier" { bezier_controls_panel(ui,v,out,id,flags.locked); }
                if typ == "knife" { knife_chains_panel(ui,v,out,id,flags.locked); }
                if typ == "switchif" { switchif_tests_panel(ui,v,out,id,flags.locked); }
            }
            if typ == "modifier_input" { modifier_inputs_panel(ui, v, out, id); }
            if let Some(message) = super::waiting::waiting_of(&v.cooking.stats, id) { ui.add_space(4.); super::waiting::panel_note(ui, v.theme, message); }
            ui.add_space(6.);
            for error in errors {
                let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.), Sense::hover());
                let r = Rect::from_min_max(r.min + Vec2::new(12., 0.), r.max - Vec2::new(12., 0.));
                ui.painter().rect(r, 6., v.theme.err_bg, Stroke::new(1.0_f32, v.theme.err), StrokeKind::Inside);
                paint_text(ui, Pos2::new(r.left() + 10., r.center().y), &error, 11., true, v.theme.err, Align2::LEFT_CENTER);
            }
        }
    });
}

fn switchif_tests_panel(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, locked: bool) {
    let params = &v.doc["nodes"][id]["params"];
    let count = params["number_of_tests"].as_u64().unwrap_or(2).min(256) as usize;
    let mut tests = params["tests"].as_array().cloned().unwrap_or_default();
    tests.resize_with(count, || json!({"enable": true, "test_type": "expression", "enable_value": "1"}));
    let mut changed = false;
    ui.add_space(5.);
    ui.label(egui::RichText::new("Tests").strong().color(v.theme.accent));
    for (index, test) in tests.iter_mut().take(count).enumerate() {
        ui.push_id(("switchif_test", index), |ui| {
            ui.add_enabled_ui(!locked, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("Test {}", index + 1));
                    let mut enabled = test["enable"].as_bool().unwrap_or(true);
                    if ui.checkbox(&mut enabled, "Enable").changed() {
                        test["enable"] = json!(enabled);
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Test Type");
                    egui::ComboBox::from_id_salt("test_type")
                        .selected_text("Expression")
                        .show_ui(ui, |ui| { ui.label("Expression"); });
                });
                ui.horizontal(|ui| {
                    ui.label("Enable value");
                    let mut expression = test["enable_value"].as_str()
                        .or_else(|| test["expr"].as_str()).unwrap_or("1").to_owned();
                    if ui.text_edit_singleline(&mut expression).changed() {
                        test["enable_value"] = json!(expression);
                        changed = true;
                    }
                });
                ui.add_space(4.);
            });
        });
    }
    if changed {
        out.push(UiAction::Edit { edit: Edit::Param {
            node: id.to_owned(), name: "tests".to_owned(), value: json!(tests),
        }, continuous: false });
    }
}




#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::Fixture;

    #[test] fn channel_link_changes_only_the_selected_axis() {
        let mut doc = minihou_app::Document::parse(json!({"nodes":{"a":{"type":"box"},"b":{"type":"box","channels":{"size.y":"ch('../a/size.y')"}}}}).to_string()).unwrap();
        doc.edit(&channel_reference_edit("b", "size.x", "a", "size.x")).unwrap();
        assert_eq!(doc.value()["nodes"]["b"]["channels"]["size.x"], "ch(\"../a/size.x\")");
        assert_eq!(doc.value()["nodes"]["b"]["channels"]["size.y"], "ch('../a/size.y')");
        assert!(doc.value()["nodes"]["b"]["channels"].get("size.z").is_none());
    }

    #[test] fn edit_expression_prefills_the_existing_channel_relationship() {
        let ctx = egui::Context::default();
        let doc = json!({"nodes":{"b":{"channels":{"size.x":"ch('../a/size.x')"}}}});
        open_expression(&ctx, &doc, "b", "size.x");
        assert_eq!(ctx.data(|d| d.get_temp::<String>(egui::Id::new("link_expression_text"))).as_deref(), Some("ch('../a/size.x')"));
        assert_eq!(ctx.data(|d| d.get_temp::<(String,String)>(egui::Id::new("link_expression_open"))), Some(("b".into(), "size.x".into())));
    }


    #[test] fn linked_row_context_menu_opens_in_egui() {
        let mut f=Fixture::new();
        f.doc=minihou_app::Document::parse(json!({"display":"b","nodes":{"a":{"type":"box"},"b":{"type":"box","channels":{"size.x":"ch('../a/size.x')"}}}}).to_string()).unwrap();
        f.graph=minihou_core::Graph::from_value(f.doc.value()).unwrap(); f.selected="b".into(); f.display="b".into();
        let ctx=egui::Context::default(); f.theme.apply(&ctx);
        let mut label=Pos2::ZERO; let mut found=false; let mut tinted=false;
        for phase in 0..4 {
            let events=match phase {
                1=>vec![egui::Event::PointerMoved(label),egui::Event::PointerButton{pos:label,button:egui::PointerButton::Secondary,pressed:true,modifiers:egui::Modifiers::NONE}],
                2=>vec![egui::Event::PointerButton{pos:label,button:egui::PointerButton::Secondary,pressed:false,modifiers:egui::Modifiers::NONE}],
                _=>vec![],
            };
            let input=egui::RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(800.,600.))),events,..Default::default()};
            let v=f.view(); let mut out=Actions::default();
            let output=ctx.run(input,|ctx|{egui::CentralPanel::default().show(ctx,|ui|{
                let param=minihou_core::nodes::get("box").unwrap().params.iter().find(|p|p.name=="size").unwrap();
                row(ui,&v,&mut out,"b","box",param,false,&mut vec![]);
                if let Some(r)=ui.ctx().read_response(ui.id().with(("b","size","label"))) {label=r.rect.center();}
            });});
            found|=output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(t) if t.galley.text().contains("Copy Parameter")));
            tinted|=output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Rect(rect) if rect.fill == Color32::from_rgba_unmultiplied(48,164,102,42)));
            assert!(out.0.is_empty());
        }
        assert!(found,"right click should reveal parameter actions");
        assert!(tinted,"linked field should carry the green tint");
    }

    #[test] fn vector_channel_right_click_opens_channel_link_menu() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({"display":"b","nodes":{"a":{"type":"box"},"b":{"type":"box"}}}).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap(); f.selected = "b".into(); f.display = "b".into();
        let ctx = egui::Context::default(); f.theme.apply(&ctx);
        let mut at = Pos2::ZERO;
        let mut found = false;
        for phase in 0..4 {
            let events = match phase {
                1 => vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE }],
                2 => vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: egui::Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.,600.))), events, ..Default::default() };
            let v = f.view(); let mut out = Actions::default();
            let output = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                let param = minihou_core::nodes::get("box").unwrap().params.iter().find(|p| p.name == "size").unwrap();
                row(ui, &v, &mut out, "b", "box", param, false, &mut vec![]);
            }); });
            if phase == 0 {
                at = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) if t.galley.text() == "X" => Some(t.pos + Vec2::new(4., 5.)),
                    _ => None,
                }).expect("X channel label");
            }
            found |= output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.text().contains("Edit Expression")));
            assert!(out.0.is_empty());
        }
        assert!(found, "right-clicking X should open its channel menu");
    }

    #[test]
    fn bezier_inspector_uses_control_model_with_default_param() {
        let mut f=Fixture::new();
        f.doc=minihou_app::Document::parse(json!({"display":"bezier1","nodes":{"bezier1":{"type":"bezier"}}}).to_string()).unwrap();
        f.graph=minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected="bezier1".into();f.display="bezier1".into();
        let actions=f.run(|ui,v,out| bezier_controls_panel(ui,v,out,"bezier1",false));
        assert!(actions.is_empty());
        let controls=super::super::curve_tool::node_controls(f.doc.value(),"bezier1");
        assert_eq!(super::super::curve_tool::parse(&controls).unwrap().len(),3);
    }

    #[test]
    fn clicking_expose_emits_saved_document_edit_for_the_link_interface() {
        let mut f=Fixture::new();
        f.doc=minihou_app::Document::parse(json!({"display":"box1","nodes":{"box1":{"type":"box"}}}).to_string()).unwrap();
        f.graph=minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected="box1".into();f.display="box1".into();
        let ctx=egui::Context::default();f.theme.apply(&ctx);
        let mut click=None;let mut actions=Actions::default();
        for phase in 0..3 {
            let events=match (phase,click) {
                (1,Some(pos))=>vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE}],
                (2,Some(pos))=>vec![egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE}],
                _=>vec![],
            };
            let input=egui::RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(800.,600.))),events,..Default::default()};
            let v=f.view();
            let _=ctx.run(input,|ctx|{egui::CentralPanel::default().show(ctx,|ui|{
                let param=minihou_core::nodes::get("box").unwrap().params.iter().find(|p|p.name=="size").unwrap();
                row(ui,&v,&mut actions,"box1","box",param,false,&mut vec![]);
                click=ui.ctx().read_response(ui.id().with(("box1","size","expose"))).map(|r|r.rect.center());
            });});
        }
        let edit=actions.0.into_iter().find_map(|a|match a {UiAction::Edit{edit:edit @ Edit::Exposed(_),..}=>Some(edit),_=>None}).expect("Expose click must emit a document edit");
        let mut history=minihou_app::History::default();history.edit(&mut f.doc,&edit).unwrap();history.finish(&f.doc);
        assert_eq!(f.doc.value()["exposed"][0]["param"],"size");
        assert_eq!(minihou_app::link_interface(f.doc.value())["params"][0]["name"],"box1_size");
        history.undo(&mut f.doc);assert!(!minihou_app::is_exposed(f.doc.value(),"box1","size"));
    }

    #[test]
    fn clicking_group_arrow_emits_viewport_pick_action() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({"display":"subdivide1","nodes":{
            "box1":{"type":"box"},"subdivide1":{"type":"subdivide","inputs":{"geo":"box1"}}
        }}).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "subdivide1".into(); f.display = "subdivide1".into();
        let ctx = egui::Context::default(); f.theme.apply(&ctx);
        let mut click = None;
        let mut actions = Actions::default();
        for phase in 0..3 {
            let events = match (phase, click) {
                (1,Some(pos)) => vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:true, modifiers:egui::Modifiers::NONE }],
                (2,Some(pos)) => vec![egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:false, modifiers:egui::Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(800.,600.))), events, ..Default::default() };
            let v = f.view();
            let output = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                let param = minihou_core::nodes::get("subdivide").unwrap().params.iter().find(|p|p.name=="creases").unwrap();
                row(ui,&v,&mut actions,"subdivide1","subdivide",param,false,&mut vec![]);
            }); });
            if phase == 0 {
                for shape in output.shapes { if let egui::Shape::Text(t) = shape.shape {
                    if t.galley.text() == "↖" { click = Some(t.pos + t.galley.size() * 0.5); }
                }}
            }
        }
        assert!(click.is_some(), "purple arrow must be drawn");
        assert!(actions.0.iter().any(|a| matches!(a, UiAction::PickParameter {node,name,mode,..}
            if node=="subdivide1" && name=="creases" && *mode==super::super::SelectionMode::Edges)));
        // the arrow click must not touch the node selection (no Select / ToggleSelect / SelectSet / Display actions)
        assert!(!actions.0.iter().any(|a| matches!(a, UiAction::Select(_) | UiAction::ToggleSelect(_) | UiAction::SelectSet { .. })));
    }

    #[test]
    fn outgroup_checkbox_edits_outset_enable_parameter() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({"display":"outset1","nodes":{
            "box1":{"type":"box"},"outset1":{"type":"outset","inputs":{"geo":"box1"}}
        }}).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "outset1".into(); f.display = "outset1".into();
        let ctx = egui::Context::default(); f.theme.apply(&ctx);
        let mut actions = Actions::default();
        let mut click = None;
        for phase in 0..3 {
            let events = match (phase, click) {
                (1, Some(pos)) => vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:true, modifiers:egui::Modifiers::NONE }],
                (2, Some(pos)) => vec![egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:false, modifiers:egui::Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(800.,600.))), events, ..Default::default() };
            let v = f.view();
            let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                let param = minihou_core::nodes::get("outset").unwrap().params.iter().find(|p| p.name == "output_group_name").unwrap();
                let row_rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 28.));
                click = Some(ParamRowLayout::compute(row_rect).label_rect.min + Vec2::new(13., 14.));
                row(ui, &v, &mut actions, "outset1", "outset", param, false, &mut vec![]);
            }); });
        }
        assert!(actions.0.iter().any(|action| matches!(action, UiAction::Edit { edit: Edit::Param { node, name, value }, .. }
            if node == "outset1" && name == "output_group" && *value == json!(true))), "actions: {:?}; click: {:?}", actions.0, click);
    }

    fn arrow_click_mode(typ: &str, field: &str) -> Option<super::super::SelectionMode> {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({"display":"n1","nodes":{
            "box1":{"type":"box"},"n1":{"type":typ,"inputs":{"geo":"box1"}}
        }}).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "n1".into(); f.display = "n1".into();
        let ctx = egui::Context::default(); f.theme.apply(&ctx);
        let mut click = None;
        let mut actions = Actions::default();
        for phase in 0..3 {
            let events = match (phase, click) {
                (1,Some(pos)) => vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:true, modifiers:egui::Modifiers::NONE }],
                (2,Some(pos)) => vec![egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:false, modifiers:egui::Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(800.,600.))), events, ..Default::default() };
            let v = f.view();
            let output = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                let param = minihou_core::nodes::get(typ).unwrap().params.iter().find(|p|p.name==field).unwrap();
                row(ui,&v,&mut actions,"n1",typ,param,false,&mut vec![]);
            }); });
            if phase == 0 {
                for shape in output.shapes { if let egui::Shape::Text(t) = shape.shape {
                    if t.galley.text() == "↖" { click = Some(t.pos + t.galley.size() * 0.5); }
                }}
            }
        }
        actions.0.iter().find_map(|a| match a {
            UiAction::PickParameter {node,name,mode,..} if node=="n1" && name==field => Some(*mode),
            _ => None })
    }
    #[test]
    fn clicking_arrow_works_on_many_nodes() {
        use super::super::SelectionMode::*;
        assert_eq!(arrow_click_mode("blast", "group"), Some(Faces));
        assert_eq!(arrow_click_mode("group_create", "pattern"), Some(Points));
        assert_eq!(arrow_click_mode("edge_slide", "group"), Some(Edges));
        assert_eq!(arrow_click_mode("bend", "group"), Some(Points));
        assert_eq!(arrow_click_mode("select_path", "waypoints"), Some(Points));
        assert_eq!(arrow_click_mode("select_loop", "seeds"), Some(Edges));
        assert_eq!(arrow_click_mode("select_ring", "seeds"), Some(Edges));
    }

    #[test]
    fn titles_and_invalid_ranges() {
        assert_eq!(title("attribute_create"), "Attribute Create");
        assert_eq!(invalid_reason(Some(&json!({"min": 1, "max": 4})), &json!(0)), Some("Value must be at least 1.".into()));
        assert_eq!(invalid_reason(Some(&json!({"min": 1, "max": 4})), &json!(3)), None);
    }

    #[test]
    fn reset_all_uses_only_modified_values() {
        let mut f = Fixture::new();
        f.selected = "noise1".into();
        let before = modified_params(&f.view(), "noise1", "noise");
        f.doc.edit(&Edit::Param { node: "noise1".into(), name: "gain".into(), value: json!(0.7) }).unwrap();
        let after = modified_params(&f.view(), "noise1", "noise");
        assert!(after.contains(&("gain".into(), json!(0.5))));
        assert_eq!(after.len(), before.len() + 1);
    }

    #[test]
    fn both_themes_render() {
        let mut f = Fixture::new();
        f.selected = "noise1".into();
        assert!(f.run(show).is_empty());
        f.theme = Theme::light();
        assert!(f.run(show).is_empty());
        f.flags.insert("noise1".into(), super::super::NodeFlags { locked: true, template: false });
        assert!(f.run(show).is_empty());
    }

    #[test]
    fn parameter_panel_shows_mm_for_length_params() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({
            "version": 1,
            "display": "box1",
            "nodes": {
                "box1": {
                    "type": "box",
                    "params": {
                        "size": [1.0, 1.0, 1.0]
                    }
                }
            }
        }).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "box1".into();
        f.display = "box1".into();
        f.units = crate::units::UnitSettings { display: crate::units::LengthUnit::Mm, unit_scale: 1.0 };

        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(1280., 800.))), ..Default::default() };
        let mut out = Actions::default();
        let v = f.view();
        let full_output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(ui, &v, &mut out));
        });

        fn extract_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
            for s in shapes {
                match &s.shape {
                    egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                    egui::epaint::Shape::Vec(subs) => {
                        let clipped: Vec<egui::epaint::ClippedShape> = subs.iter().map(|sub| egui::epaint::ClippedShape { clip_rect: s.clip_rect, shape: sub.clone() }).collect();
                        extract_texts(&clipped, out);
                    }
                    _ => {}
                }
            }
        }
        let mut texts = Vec::new();
        extract_texts(&full_output.shapes, &mut texts);
        assert!(texts.iter().any(|t| t == "1000 mm"), "expected '1000 mm' in painted texts, got: {texts:?}");
    }

    #[test]
    fn uniform_scale_slider_and_vector_scrub() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({
            "version": 1,
            "display": "transform1",
            "nodes": {
                "transform1": {
                    "type": "transform",
                    "params": {
                        "translate": [1.0, 2.0, 3.0],
                        "scale": [1.0, 1.0, 1.0],
                        "uniform_scale": 1.0
                    }
                }
            }
        }).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "transform1".into();
        f.display = "transform1".into();
        assert!(f.run(show).is_empty());
    }

    #[test]
    fn houdini_row_layout_geometry() {
        // Standard width 400px: label width is ~30% (120px)
        let r400 = Rect::from_min_size(Pos2::ZERO, Vec2::new(400., 28.));
        let l400 = ParamRowLayout::compute(r400);
        assert!((l400.label_rect.width() - 120.0).abs() < 1e-3);
        assert_eq!(l400.label_rect.left(), 16.0);
        assert_eq!(l400.control_rect.left(), l400.label_rect.right() + 8.0);
        assert!(l400.control_rect.width() > 60.0);

        // Narrow width 350px: label width clamps to min 110px
        let r350 = Rect::from_min_size(Pos2::ZERO, Vec2::new(350., 28.));
        let l350 = ParamRowLayout::compute(r350);
        assert_eq!(l350.label_rect.width(), 110.0);
        assert_eq!(l350.control_rect.left(), l350.label_rect.right() + 8.0);

        // Wide width 1200px: label width clamps to max 240px
        let r1200 = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200., 28.));
        let l1200 = ParamRowLayout::compute(r1200);
        assert_eq!(l1200.label_rect.width(), 240.0);
        assert_eq!(l1200.control_rect.left(), l1200.label_rect.right() + 8.0);

        // Very narrow width 180px: fields shrink gracefully to minimum
        let r180 = Rect::from_min_size(Pos2::ZERO, Vec2::new(180., 28.));
        let l180 = ParamRowLayout::compute(r180);
        assert!(l180.label_rect.width() >= 30.0);
        assert!(l180.control_rect.width() >= 10.0);
        assert_eq!(l180.control_rect.left(), l180.label_rect.right() + 8.0);
    }

    #[test]
    fn label_truncation_helper() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let font = FontId::proportional(12.);
                let full = "Consolidate Corner Points";
                let untruncated = truncate_with_ellipsis(ui, full, &font, 300.0);
                assert_eq!(untruncated, full);

                let truncated = truncate_with_ellipsis(ui, full, &font, 50.0);
                assert!(truncated.ends_with('…'));
                assert!(truncated.len() < full.len());
            });
        });
    }

    #[test]
    fn boolean_row_omits_label_column_and_renders_toggle_at_field_column() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({
            "version": 1,
            "display": "box1",
            "nodes": {
                "box1": {
                    "type": "box",
                    "params": {
                        "consolidate": true
                    }
                }
            }
        }).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "box1".into();
        f.display = "box1".into();

        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(500., 400.))), ..Default::default() };
        let mut out = Actions::default();
        let v = f.view();
        let full_output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(ui, &v, &mut out));
        });

        let mut label_texts = Vec::new();
        fn collect_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<(String, Pos2)>) {
            for s in shapes {
                match &s.shape {
                    egui::epaint::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.pos)),
                    egui::epaint::Shape::Vec(subs) => {
                        let clipped: Vec<egui::epaint::ClippedShape> = subs.iter().map(|sub| egui::epaint::ClippedShape { clip_rect: s.clip_rect, shape: sub.clone() }).collect();
                        collect_texts(&clipped, out);
                    }
                    _ => {}
                }
            }
        }
        collect_texts(&full_output.shapes, &mut label_texts);

        let row_layout = ParamRowLayout::compute(Rect::from_min_size(Pos2::ZERO, Vec2::new(500., 28.)));
        let consolidate_text = label_texts.iter().find(|(t, _)| t.contains("Consolidate"));
        assert!(consolidate_text.is_some(), "Consolidate toggle text should be present");
        let (_, pos) = consolidate_text.unwrap();
        assert!(pos.x >= row_layout.label_rect.right(), "Checkbox text pos ({}) must sit at/after field column start ({})", pos.x, row_layout.label_rect.right());
    }
}
#[cfg(test)]
mod wrangle_layout_tests {
    use super::*;
    #[test]
    fn wrangle_code_is_labelled_and_last() {
        let params = nodes::get("wrangle").unwrap().params;
        assert_eq!(params.last().unwrap().name, "snippet");
        assert_eq!(param_meta("wrangle", "snippet").unwrap()["label"], "code");
    }
}
