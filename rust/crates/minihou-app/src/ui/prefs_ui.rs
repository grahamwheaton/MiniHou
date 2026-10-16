//! Preferences window. Its tab, search and pending key capture are egui-only state.
use eframe::egui::{self, RichText};
use super::{Chord, Keymap, Preferences, ThemePref};
use super::super::theme::Theme;
use crate::units::LengthUnit;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab { #[default] Interface, Units, Viewport, Keymap, Files }
#[derive(Clone, Default)]
struct State { tab: Tab, search: String, capture: Option<usize>, conflict: String }

pub fn show(ctx: &egui::Context, open: &mut bool, prefs: &mut Preferences, keymap: &mut Keymap, theme: &Theme) -> bool {
    if !*open { return false; }
    let id = egui::Id::new("minihou_preferences_ui");
    let mut state = ctx.data_mut(|d| d.get_temp::<State>(id).unwrap_or_default());
    let mut changed = false;
    egui::Window::new("Preferences").open(open).default_size([640., 480.]).resizable(true)
        .frame(egui::Frame::new().fill(theme.panel).stroke(egui::Stroke::new(1.0_f32,theme.stroke)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(130.);
                    for (tab,label) in [(Tab::Interface,"Interface"),(Tab::Units,"Units"),(Tab::Viewport,"Viewport"),(Tab::Keymap,"Keymap"),(Tab::Files,"Files")] {
                        if ui.selectable_label(state.tab == tab, label).clicked() { state.tab = tab; state.capture = None; state.conflict.clear(); }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_min_width(450.);
                    match state.tab {
                        Tab::Interface => {
                            heading(ui,"Interface",theme);
                            ui.horizontal(|ui| { ui.label("Theme");
                                for value in ThemePref::ALL { changed |= ui.selectable_value(&mut prefs.theme,value,value.label()).changed(); }
                            });
                            changed |= ui.add(egui::Slider::new(&mut prefs.ui_scale,0.5..=3.).text("UI scale").step_by(0.05)).changed();
                            changed |= ui.add(egui::DragValue::new(&mut prefs.snap_increment).range(0.0001..=1000.).speed(0.01).prefix("Snap increment  ")).changed();
                            changed |= ui.checkbox(&mut prefs.show_stats,"Show statistics").changed();
                            changed |= ui.add(egui::Slider::new(&mut prefs.node_physics_snappiness,0.0..=1.0).text("Node physics snappiness"))
                                .on_hover_text("Higher values make connected nodes follow Shift-drag faster and settle sooner").changed();
                        }
                        Tab::Units => {
                            heading(ui,"Units",theme);
                            ui.horizontal(|ui| {
                                ui.label("Length display");
                                egui::ComboBox::from_id_salt("length_unit").selected_text(prefs.units.display.symbol()).show_ui(ui, |ui| {
                                    for unit in LengthUnit::ALL {
                                        changed |= ui.selectable_value(&mut prefs.units.display, unit, unit.symbol()).changed();
                                    }
                                });
                            });
                            changed |= ui.add(egui::DragValue::new(&mut prefs.units.unit_scale).range(1e-9..=1e9).speed(0.01).prefix("Unit scale  ")).changed();
                            ui.add_space(4.);
                            ui.label(RichText::new("Display only: scale 1 means one model unit is one metre (1000 mm). Existing geometry is never resized. Explicit length suffixes are accepted by measurement editing.").color(theme.dim));
                        }
                        Tab::Viewport => {
                            heading(ui,"Viewport",theme);
                            ui.horizontal(|ui| { ui.label("Navigation"); egui::ComboBox::from_id_salt("navigation").selected_text(match prefs.viewport.navigation { super::prefs::Navigation::Blender => "Blender (middle mouse orbit)", super::prefs::Navigation::Houdini => "Houdini / MiniHou" }).show_ui(ui, |ui| {
                                changed |= ui.selectable_value(&mut prefs.viewport.navigation, super::prefs::Navigation::Blender, "Blender (middle mouse orbit)").changed();
                                changed |= ui.selectable_value(&mut prefs.viewport.navigation, super::prefs::Navigation::Houdini, "Houdini / MiniHou").changed();
                            }); });
                            ui.horizontal(|ui| { ui.label("Default shading");
                                egui::ComboBox::from_id_salt("default_shading").selected_text(&prefs.viewport.default_shading).show_ui(ui, |ui| {
                                    for mode in crate::viewport::Mode::ALL.map(|m| m.label()) {
                                        changed |= ui.selectable_value(&mut prefs.viewport.default_shading,mode.to_owned(),mode).changed();
                                    }
                                });
                            });
                            ui.horizontal(|ui| { ui.label("Other objects");
                                egui::ComboBox::from_id_salt("object_visibility_pref").selected_text(prefs.viewport.object_visibility.label()).show_ui(ui,|ui| {
                                    for visibility in crate::viewport::ObjectVisibility::ALL {
                                        changed |= ui.selectable_value(&mut prefs.viewport.object_visibility,visibility,visibility.label()).changed();
                                    }
                                });
                            });
                            changed |= ui.checkbox(&mut prefs.viewport.fly_navigation,"Fly navigation (RMB + WASD)").changed();
                            changed |= ui.checkbox(&mut prefs.viewport.show_grid,"Show grid").changed();
                            changed |= ui.checkbox(&mut prefs.viewport.show_axes,"Show axes").changed();
                            changed |= ui.add(egui::DragValue::new(&mut prefs.viewport.auto_budget).range(1..=u32::MAX).speed(1_000.).prefix("Auto proxy triangle budget  ")).changed();
                            changed |= ui.add(egui::DragValue::new(&mut prefs.viewport.clip_start).range(1e-6..=1e8).speed(0.01).prefix("Clip Start  ")).changed();
                            changed |= ui.add(egui::DragValue::new(&mut prefs.viewport.clip_end).range(1e-6..=1e9).speed(1.0).prefix("Clip End  ")).changed();
                            (prefs.viewport.clip_start, prefs.viewport.clip_end) = minihou_app::sanitize_clip_planes(prefs.viewport.clip_start, prefs.viewport.clip_end);
                        }
                        Tab::Keymap => {
                            heading(ui,"Keymap",theme);
                            ui.label(RichText::new("Numpad fallback: Alt+digit, Alt+Period, and Alt+Plus/Minus. Emulate Numpad uses plain digits to navigate and Shift+1/2/3 for edit mode selection.").color(theme.dim));
                            if ui.checkbox(&mut prefs.emulate_numpad,"Emulate Numpad").changed() { keymap.emulate_numpad = prefs.emulate_numpad; changed = true; }
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut state.search).hint_text("Search commands"));
                                if ui.button("Reset keymap").clicked() { keymap.reset(); keymap.emulate_numpad = prefs.emulate_numpad; prefs.keymap_overrides = keymap.overrides_json(); state.conflict.clear(); changed = true; }
                            });
                            if !state.conflict.is_empty() { ui.label(RichText::new(&state.conflict).color(theme.err)); }
                            ui.separator();
                            ui.horizontal(|ui| { ui.add_sized([205.,20.],egui::Label::new(RichText::new("COMMAND").color(theme.dim)));
                                ui.add_sized([115.,20.],egui::Label::new(RichText::new("CHORD").color(theme.dim)));
                                ui.label(RichText::new("CONTEXT").color(theme.dim)); });
                            egui::ScrollArea::vertical().max_height(280.).show(ui, |ui| {
                                for (index,b) in keymap.bindings.clone().into_iter().enumerate() {
                                    if !b.command.label().to_ascii_lowercase().contains(&state.search.to_ascii_lowercase()) { continue; }
                                    ui.horizontal(|ui| {
                                        ui.add_sized([205.,24.],egui::Label::new(b.command.label()));
                                        let label = if state.capture == Some(index) { "Press a key…".to_owned() } else { b.chord.label() };
                                        if ui.add_sized([115.,24.],egui::Button::new(RichText::new(label).monospace())).clicked() {
                                            state.capture = Some(index); state.conflict.clear();
                                        }
                                        ui.label(format!("{:?}",b.context));
                                    });
                                }
                            });
                            if let Some(index) = state.capture {
                                let pressed = ctx.input(|i| i.events.iter().find_map(|event| match event {
                                    egui::Event::Key { key, pressed: true, modifiers, .. } => Some(Chord::new(*key,modifiers.ctrl,modifiers.shift,modifiers.alt)), _ => None,
                                }));
                                if let Some(chord) = pressed {
                                    if chord.key == egui::Key::Escape { state.capture = None; }
                                    else { match keymap.rebind_at(index,chord) {
                                        Ok(()) => { state.capture = None; state.conflict.clear(); prefs.keymap_overrides = keymap.overrides_json(); changed = true; }
                                        Err(c) => state.conflict = format!("{} already uses {} in {:?}",c.command.label(),c.chord.label(),c.context),
                                    } }
                                }
                            }
                        }
                        Tab::Files => {
                            heading(ui,"Files",theme);
                            let mut enabled = prefs.autosave_minutes.is_some();
                            if ui.checkbox(&mut enabled,"Autosave").changed() { prefs.autosave_minutes = enabled.then_some(5); changed = true; }
                            if let Some(minutes) = &mut prefs.autosave_minutes { changed |= ui.add(egui::DragValue::new(minutes).range(1..=120).prefix("Every ").suffix(" minutes")).changed(); }
                            ui.separator(); ui.label(RichText::new("Recent files").color(theme.dim));
                            if prefs.recent_files.is_empty() { ui.label("No recent files"); }
                            for path in prefs.recent_files.iter().take(10) { ui.label(RichText::new(path.display().to_string()).monospace()); }
                        }
                    }
                });
            });
        });
    ctx.data_mut(|d| d.insert_temp(id,state));
    changed
}
fn heading(ui: &mut egui::Ui, title: &str, theme: &Theme) {
    ui.label(RichText::new(title).size(15.).strong().color(theme.text)); ui.add_space(8.);
}
