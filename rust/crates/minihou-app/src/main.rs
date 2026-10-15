mod viewport;
use eframe::{egui, egui_wgpu};
use minihou_app::{Camera, Document, ParamKind, Result, param_kind, param_meta, metadata_kind, slider_range, clamp_parameter, Edit, EditorDocument, CookWorker, CookRequest, CookState};
use minihou_core::{nodes, Graph};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use viewport::{Gpu, Mode, Scene, Target};

struct App {
    editor: EditorDocument, worker: CookWorker<Scene>, cooking: CookState,
    selected: String, display: String,
    state: egui_wgpu::RenderState, gpu: Option<Gpu>, target: Option<Target>,
    camera: Camera, bounds: ([f32;3], f32), mode: Mode,
    file_status: String,
}
impl App {
    fn new(doc: Document, path: PathBuf, state: egui_wgpu::RenderState) -> Result<Self> {
        let display = Graph::from_value(doc.value())?.main_output()?;
        let worker = CookWorker::start(&doc, Scene::new)?;
        let mut app = Self { selected:display.clone(), display, editor:EditorDocument::new(doc, path), worker,
            cooking:CookState::default(), state,gpu:None,target:None,
            camera:Camera::frame([0.;3],1.),bounds:([0.;3],1.),mode:Mode::Shaded,file_status:String::new() };
        app.send(Edit::Display(app.display.clone()));
        Ok(app)
    }
    fn send(&mut self, edit: Edit) {
        self.cooking.requested += 1;
        if let Err(e) = self.worker.send(CookRequest { revision:self.cooking.requested, edit }) {
            self.cooking.completed = self.cooking.requested;
            self.cooking.error = Some(e.to_string());
        }
    }
    fn edit(&mut self, edit: Edit, continuous: bool) {
        match self.editor.history.edit(&mut self.editor.doc, &edit) {
            Ok(()) => { self.send(edit); if !continuous { self.editor.history.finish(&self.editor.doc); } }
            Err(e) => self.file_status = e.to_string(),
        }
    }
    fn frame_mesh(&mut self) { self.camera = Camera::frame(self.bounds.0,self.bounds.1); }
    fn save_to(&mut self, path: PathBuf) {
        self.editor.history.finish(&self.editor.doc);
        match self.editor.save_to(path) {
            Ok(()) => { self.file_status = "Saved".into(); }
            Err(e) => self.file_status = format!("Save failed: {e}"),
        }
    }
    fn open(&mut self, path: PathBuf) {
        let replacement = Document::open(&path).and_then(|doc| Self::new(doc, path, self.state.clone()));
        match replacement {
            Ok(mut app) => { app.target = self.target.take(); *self = app; }
            Err(e) => self.file_status = format!("Open failed: {e}"),
        }
    }
}
fn widget(ui: &mut egui::Ui, kind: ParamKind, meta: Option<&Value>, value: &mut Value) -> bool {
    if let Some(meta) = meta {
        if meta["kind"] == "enum" {
            let before = value.clone();
            egui::ComboBox::from_id_salt("enum").selected_text(value.as_str().unwrap_or(""))
                .show_ui(ui, |ui| {
                    if let Some(options) = meta["options"].as_array() {
                        for option in options { ui.selectable_value(value, option.clone(), option.as_str().unwrap_or("")); }
                    }
                });
            return *value != before;
        }
        if matches!(kind, ParamKind::Integer | ParamKind::Float) {
            let mut v = value.as_f64().unwrap_or(0.);
            let integer = kind == ParamKind::Integer;
            let mut changed = false;
            if let Some(range) = slider_range(meta) {
                let mut slider = egui::Slider::new(&mut v, range).show_value(false).clamping(egui::SliderClamping::Edits);
                if integer { slider = slider.integer(); }
                changed |= ui.add(slider).changed();
            }
            changed |= ui.add(egui::DragValue::new(&mut v).speed(if integer {1.} else {0.01})
                .range(meta["min"].as_f64().unwrap_or(f64::NEG_INFINITY)..=meta["max"].as_f64().unwrap_or(f64::INFINITY))).changed();
            if changed {
                v = clamp_parameter(meta, v);
                *value = if integer { json!(v.round() as i64) } else { json!(v) };
            }
            return changed;
        }
    }
    match kind {
        ParamKind::Bool => {
            let mut v = value.as_bool().unwrap_or(false);
            if ui.checkbox(&mut v,"").changed() { *value=json!(v); true } else { false }
        }
        ParamKind::Integer => {
            let mut v = value.as_i64().or_else(|| value.as_f64().map(|f|f as i64)).unwrap_or(0);
            if ui.add(egui::DragValue::new(&mut v).speed(1)).changed() { *value=json!(v); true } else { false }
        }
        ParamKind::Float => {
            let mut v = value.as_f64().unwrap_or(0.);
            if ui.add(egui::DragValue::new(&mut v).speed(0.01)).changed() { *value=json!(v); true } else { false }
        }
        ParamKind::String => {
            let mut v = value.as_str().unwrap_or("").to_owned();
            if ui.text_edit_singleline(&mut v).changed() { *value=json!(v); true } else { false }
        }
        ParamKind::Vec3 => {
            let mut v = [0,1,2].map(|i|value.get(i).and_then(Value::as_f64).unwrap_or(0.));
            let mut changed=false;
            ui.horizontal(|ui| { for x in &mut v { changed |= ui.add(egui::DragValue::new(x).speed(0.01).range(
                meta.and_then(|m| m["min"].as_f64()).unwrap_or(f64::NEG_INFINITY)..=meta.and_then(|m| m["max"].as_f64()).unwrap_or(f64::INFINITY))).changed(); } });
            if changed { *value=json!(v); } changed
        }
        ParamKind::Unsupported => { ui.label(value.to_string()); false }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(response) = self.worker.poll() {
            if let Some(scene) = self.cooking.accept(response) {
                let first = self.gpu.is_none();
                self.bounds = (scene.center, scene.radius);
                if let Some(gpu) = &mut self.gpu { gpu.upload(&self.state.device, &scene); }
                else { self.gpu = Some(Gpu::new(&self.state.device, &scene)); }
                if first { self.frame_mesh(); }
            }
        }
        if self.cooking.cooking() { ctx.request_repaint_after(Duration::from_millis(16)); }
        let graph = Graph::from_value(self.editor.doc.value()).expect("validated document");
        let mut open = false;
        let mut save_as = false;
        let mut undo = ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Z));
        let mut redo = ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Y));
        let mut save = ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL,egui::Key::S));
        let mut frame = !ctx.wants_keyboard_input() && ctx.input(|i|i.key_pressed(egui::Key::F));
        egui::TopBottomPanel::top("toolbar").show(ctx,|ui| {
            ui.horizontal(|ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open...").clicked() { open = true; ui.close(); }
                    if ui.button("Save").clicked() { save = true; ui.close(); }
                    if ui.button("Save As...").clicked() { save_as = true; ui.close(); }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.add_enabled(self.editor.history.can_undo(), egui::Button::new("Undo (Ctrl+Z)")).clicked() { undo = true; ui.close(); }
                    if ui.add_enabled(self.editor.history.can_redo(), egui::Button::new("Redo (Ctrl+Y)")).clicked() { redo = true; ui.close(); }
                });
                for (mode,label) in [(Mode::Solid,"Solid"),(Mode::Shaded,"Shaded"),(Mode::Wire,"Wire"),(Mode::ShadedWire,"Shaded+Wire")] {
                    ui.selectable_value(&mut self.mode,mode,label);
                }
                frame |= ui.button("Frame (F)").clicked();
                save |= ui.button("Save (Ctrl+S)").clicked();
                ui.label(format!("{}{}",self.editor.path.display(),if self.editor.dirty() {" *"} else {""}));
            });
        });
        egui::SidePanel::left("nodes").default_width(220.).show(ctx,|ui| {
            ui.heading("Nodes");
            egui::ScrollArea::vertical().show(ui,|ui| {
                for (id,node) in &graph.nodes {
                    ui.horizontal(|ui| {
                        if ui.radio(self.display==*id,"").on_hover_text("Display this node").clicked() {
                            self.display=id.clone();
                            self.editor.history.finish(&self.editor.doc);
                            self.edit(Edit::Display(id.clone()), false);
                        }
                        if ui.selectable_label(self.selected==*id,format!("{id}  {}",node.node_type)).clicked() { self.selected=id.clone(); }
                        if node.bypass { ui.small("[bypass]"); }
                    });
                }
            });
        });
        egui::SidePanel::right("parameters").default_width(300.).show(ctx,|ui| {
            ui.heading(&self.selected);
            let selected=self.selected.clone();
            let node = graph.nodes.iter().find(|(id,_)|id==&selected).map(|(_,n)|(n.node_type.clone(),n.bypass));
            if let Some((typ,mut bypass))=node {
                ui.label(&typ);
                if ui.checkbox(&mut bypass,"Bypass").changed() {
                    self.editor.history.finish(&self.editor.doc);
                    self.edit(Edit::Bypass { node:selected.clone(), flag:bypass }, false);
                }
                egui::ScrollArea::vertical().show(ui,|ui| {
                    if let Some(spec)=nodes::get(&typ) {
                        for param in spec.params {
                            ui.push_id(param.name,|ui| {
                                let meta = param_meta(&typ, param.name);
                                let doc = meta.and_then(|m| m["doc"].as_str()).unwrap_or("");
                                ui.label(param.name).on_hover_text(doc);
                                let default=(param.default)();
                                let mut value=self.editor.doc.value()["nodes"][&selected]["params"].get(param.name).unwrap_or(&default).clone();
                                let response = ui.add_enabled_ui(param.supported,|ui|widget(ui,meta.map_or_else(|| param_kind(&default), |m| metadata_kind(m, &default)),meta,&mut value)).response;
                                let response = response.on_hover_text(doc);
                                if !param.supported { response.on_hover_text("The Rust backend does not support it yet").on_disabled_hover_text("The Rust backend does not support it yet"); }
                                // Response.changed is not propagated by a scope; inspect the value instead.
                                let old=self.editor.doc.value()["nodes"][&selected]["params"].get(param.name).unwrap_or(&default);
                                if param.supported && &value!=old {
                                    self.edit(Edit::Param { node:selected.clone(), name:param.name.into(), value }, ctx.input(|i| i.pointer.primary_down()));
                                }

                            });
                        }
                    } else { ui.colored_label(egui::Color32::RED,"Node type is not supported by the Rust backend yet"); }
                });
            }
        });
        if !ctx.input(|i| i.pointer.primary_down()) { self.editor.history.finish(&self.editor.doc); }
        let command = if undo { self.editor.history.undo(&mut self.editor.doc) } else if redo { self.editor.history.redo(&mut self.editor.doc) } else { None };
        if let Some(command) = command {
            self.display = Graph::from_value(self.editor.doc.value()).unwrap().main_output().unwrap();
            self.send(command);
        }
        if frame { self.frame_mesh(); }
        if save { self.save_to(self.editor.path.clone()); }
        if save_as {
            if let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["json"]).set_file_name(self.editor.path.file_name().unwrap_or_default().to_string_lossy()).save_file() { self.save_to(path); }
        }
        if open {
            if let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["json"]).pick_file() { self.open(path); }
        }
        if self.cooking.cooking() { ctx.request_repaint_after(Duration::from_millis(16)); }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.editor.title()));
        egui::TopBottomPanel::bottom("stats").show(ctx,|ui| {
            let stats=&self.cooking.stats;
            if self.cooking.cooking() { ui.label("cooking..."); }
            ui.label(format!("cooked: {} in {:.1} ms; cached: {}",stats.cooked.join(", "),self.cooking.milliseconds,stats.cached.join(", ")));
            if let Some(error)=&self.cooking.error { ui.colored_label(egui::Color32::RED,error); }
            if !self.file_status.is_empty() { ui.label(&self.file_status); }
        });
        egui::CentralPanel::default().show(ctx,|ui| {
            let size=ui.available_size().max(egui::vec2(1.,1.));
            let (rect,response)=ui.allocate_exact_size(size,egui::Sense::click_and_drag());
            let delta=ctx.input(|i|i.pointer.delta());
            if response.dragged() {
                if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary) || ctx.input(|i|i.modifiers.shift) {
                    self.camera.pan(delta.x,delta.y);
                } else { self.camera.orbit(delta.x,delta.y); }
            }
            if response.hovered() { self.camera.zoom(ctx.input(|i|i.smooth_scroll_delta.y)); }
            let limit=self.state.device.limits().max_texture_dimension_2d;
            let pixels=[(size.x*ctx.pixels_per_point()).round() as u32,(size.y*ctx.pixels_per_point()).round() as u32].map(|x|x.clamp(1,limit));
            if self.target.as_ref().is_none_or(|t|t.size!=pixels) {
                if let Some(old)=self.target.take() { self.state.renderer.write().free_texture(&old.id); }
                self.target=Some(Target::new(&self.state,pixels));
            }
            if let (Some(gpu),Some(target))=(&self.gpu,&self.target) {
                gpu.draw(&self.state,target,&self.camera,self.mode);
                ui.painter().image(target.id,rect,egui::Rect::from_min_max(egui::pos2(0.,0.),egui::pos2(1.,1.)),egui::Color32::WHITE);
            }
        });
    }
}
fn run() -> Result<()> {
    let args:Vec<_>=std::env::args_os().skip(1).collect();
    // Started with no arguments (a double-click): open the built-in demo graph rather than exit with a usage line nobody sees.
    let (headless,path,doc)=match args.as_slice() {
        [flag,path] if flag=="--selftest" => { let path=PathBuf::from(path); let doc=Document::open(&path)?; (true,path,doc) }
        [path] => { let path=PathBuf::from(path); let doc=Document::open(&path)?; (false,path,doc) }
        [] => (false,PathBuf::from("untitled.json"),Document::parse(include_str!("default_graph.json").to_owned())?),
        _ => return Err("usage: minihou-app [--selftest] [GRAPH.json]".into()),
    };
    if headless { println!("{}",minihou_app::selftest(&doc)?); return Ok(()); }
    let options=eframe::NativeOptions { renderer:eframe::Renderer::Wgpu,viewport:egui::ViewportBuilder::default().with_inner_size([1280.,800.]),..Default::default() };
    eframe::run_native("MiniHou",options,Box::new(move |cc| {
        let state=cc.wgpu_render_state.clone().ok_or("wgpu renderer unavailable")?;
        Ok(Box::new(App::new(doc,path,state)?))
    })).map_err(|e|e.to_string())?;
    Ok(())
}
fn main() {
    if let Err(e)=run() {
        eprintln!("{e}");
        // A double-clicked build has no terminal: say why it did not start.
        rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("MiniHou").set_description(e.to_string()).show();
        std::process::exit(1);
    }
}



