mod ui;
mod viewport;
use eframe::{egui, egui_wgpu};
use minihou_app::{Camera, Document, Result, Edit, EditorDocument, CookWorker, CookRequest, CookState};
use minihou_core::Graph;
use serde_json::json;
use std::{collections::HashMap, path::PathBuf, time::Duration};
use ui::{theme::Theme, Actions, NodeFlags, UiAction, View};
use viewport::{Gpu, Mode, Scene, Target};

struct App {
    editor: EditorDocument, worker: CookWorker<Scene>, cooking: CookState,
    selected: String, display: String,
    state: egui_wgpu::RenderState, gpu: Option<Gpu>, target: Option<Target>,
    camera: Camera, bounds: ([f32;3], f32), mode: Mode,
    file_status: String,
    theme: Theme, flags: HashMap<String, NodeFlags>, themed: bool,
}
impl App {
    fn new(doc: Document, path: PathBuf, state: egui_wgpu::RenderState) -> Result<Self> {
        let display = Graph::from_value(doc.value())?.main_output()?;
        let worker = CookWorker::start(&doc, Scene::new)?;
        let mut app = Self { selected:display.clone(), display, editor:EditorDocument::new(doc, path), worker,
            cooking:CookState::default(), state,gpu:None,target:None,
            camera:Camera::frame([0.;3],1.),bounds:([0.;3],1.),mode:Mode::Shaded,file_status:String::new(),
            theme:Theme::dark(),flags:HashMap::new(),themed:false };
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
impl App {
    /// A shelf tool: a new node of `node_type` wired to `after` (its first input) and made the displayed node; one undo step.
    fn add_node(&mut self, node_type: &str, after: Option<&str>) {
        let Some(spec) = minihou_core::nodes::get(node_type) else {
            self.file_status = format!("The Rust core has no {node_type} node yet");
            return;
        };
        let mut value = self.editor.doc.value().clone();
        let Some(nodes) = value.get_mut("nodes").and_then(|n| n.as_object_mut()) else { return };
        let id = (1..).map(|n| format!("{node_type}{n}")).find(|id| !nodes.contains_key(id)).unwrap();
        let mut node = json!({ "type": node_type });
        if let (Some(after), Some(port)) = (after.filter(|a| nodes.contains_key(*a)), spec.inputs.first()) {
            node["inputs"] = if port.multi { json!({ port.name: [after] }) } else { json!({ port.name: after }) };
        }
        nodes.insert(id.clone(), node);
        value["display"] = json!(id);
        let doc = serde_json::to_string_pretty(&value).map_err(|e| e.to_string()).and_then(|t| Document::parse(t).map_err(|e| e.to_string()));
        match doc {
            Ok(new) => {
                self.editor.history.replace(&mut self.editor.doc, new);
                self.send(Edit::Replace(self.editor.doc.value().clone()));
                self.selected = id.clone();
                self.display = id;
            }
            Err(e) => self.file_status = format!("Could not add {node_type}: {e}"),
        }
    }
    /// Apply what the panels asked for this frame.
    fn apply(&mut self, ctx: &egui::Context, actions: Vec<UiAction>) {
        for action in actions {
            match action {
                UiAction::Open => if let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["json"]).pick_file() { self.open(path); },
                UiAction::Save => self.save_to(self.editor.path.clone()),
                UiAction::SaveAs => {
                    if let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["json"]).set_file_name(self.editor.path.file_name().unwrap_or_default().to_string_lossy()).save_file() { self.save_to(path); }
                }
                UiAction::Undo | UiAction::Redo => {
                    let undo = matches!(action, UiAction::Undo);
                    let command = if undo { self.editor.history.undo(&mut self.editor.doc) } else { self.editor.history.redo(&mut self.editor.doc) };
                    if let Some(command) = command {
                        self.display = Graph::from_value(self.editor.doc.value()).unwrap().main_output().unwrap();
                        self.send(command);
                    }
                }
                UiAction::Frame => self.frame_mesh(),
                UiAction::Select(id) => self.selected = id,
                UiAction::SetDisplay(id) => {
                    self.display = id.clone();
                    self.editor.history.finish(&self.editor.doc);
                    self.edit(Edit::Display(id), false);
                }
                UiAction::Edit { edit, continuous } => {
                    if !continuous { self.editor.history.finish(&self.editor.doc); }
                    self.edit(edit, continuous);
                }
                UiAction::AddNode { node_type, after } => self.add_node(&node_type, after.as_deref()),
                UiAction::SetMode(mode) => self.mode = mode,
                UiAction::ToggleLocked(id) => { let f = self.flags.entry(id).or_default(); f.locked = !f.locked; }
                UiAction::ToggleTemplate(id) => { let f = self.flags.entry(id).or_default(); f.template = !f.template; }
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.editor.title()));
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if !self.themed { self.theme.apply(ctx); self.themed = true; }
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
        let mut out = Actions::default();
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Z)) { out.push(UiAction::Undo); }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Y)) { out.push(UiAction::Redo); }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S)) { out.push(UiAction::Save); }
        if !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::F)) { out.push(UiAction::Frame); }
        {
            let v = View { graph: &graph, doc: self.editor.doc.value(), path: &self.editor.path, dirty: self.editor.dirty(),
                can_undo: self.editor.history.can_undo(), can_redo: self.editor.history.can_redo(),
                selected: &self.selected, display: &self.display, cooking: &self.cooking, file_status: &self.file_status,
                mode: self.mode, theme: &self.theme, flags: &self.flags };
            egui::TopBottomPanel::top("toolbar").show(ctx, |ui| ui::topbar::show(ui, &v, &mut out));
            egui::TopBottomPanel::top("shelf").show(ctx, |ui| ui::shelf::show(ui, &v, &mut out));
            egui::TopBottomPanel::bottom("stats").show(ctx, |ui| ui::status::show(ui, &v, &mut out));
            egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| ui::network::show(ui, &v, &mut out));
            egui::SidePanel::right("parameters").default_width(320.).show(ctx, |ui| ui::params::show(ui, &v, &mut out));
            egui::CentralPanel::default().show(ctx, |ui| {
                let size = ui.available_size().max(egui::vec2(1., 1.));
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                let delta = ctx.input(|i| i.pointer.delta());
                if response.dragged() {
                    if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary) || ctx.input(|i| i.modifiers.shift) {
                        self.camera.pan(delta.x, delta.y);
                    } else { self.camera.orbit(delta.x, delta.y); }
                }
                if response.hovered() { self.camera.zoom(ctx.input(|i| i.smooth_scroll_delta.y)); }
                let limit = self.state.device.limits().max_texture_dimension_2d;
                let pixels = [(size.x * ctx.pixels_per_point()).round() as u32, (size.y * ctx.pixels_per_point()).round() as u32].map(|x| x.clamp(1, limit));
                if self.target.as_ref().is_none_or(|t| t.size != pixels) {
                    if let Some(old) = self.target.take() { self.state.renderer.write().free_texture(&old.id); }
                    self.target = Some(Target::new(&self.state, pixels));
                }
                if let (Some(gpu), Some(target)) = (&self.gpu, &self.target) {
                    gpu.draw(&self.state, target, &self.camera, self.mode);
                    ui.painter().image(target.id, rect, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), egui::Color32::WHITE);
                }
                ui::viewport_ui::overlay(ui, rect, &v, &mut out);
            });
        }
        if !ctx.input(|i| i.pointer.primary_down()) { self.editor.history.finish(&self.editor.doc); }
        self.apply(ctx, out.0);
        if self.cooking.cooking() { ctx.request_repaint_after(Duration::from_millis(16)); }
    }
}
/// Open a graph given on the command line; the error names the path (a bare "os error 2" says nothing to someone who double-clicked).
fn open_graph(path: &std::path::Path) -> Result<Document> {
    Document::open(path).map_err(|e| format!("Cannot open {}: {e}

Run the app without arguments to open the built-in demo graph.", std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()).display()).into())
}
fn run() -> Result<()> {
    let args:Vec<_>=std::env::args_os().skip(1).collect();
    // Started with no arguments (a double-click): open the built-in demo graph rather than exit with a usage line nobody sees.
    let (headless,path,doc)=match args.as_slice() {
        [flag,path] if flag=="--selftest" => { let path=PathBuf::from(path); let doc=open_graph(&path)?; (true,path,doc) }
        [path] => { let path=PathBuf::from(path); let doc=open_graph(&path)?; (false,path,doc) }
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
        // (MINIHOU_NO_DIALOG: the tests start the app with bad input and must not wait for someone to click OK.)
        if std::env::var_os("MINIHOU_NO_DIALOG").is_none() {
            rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("MiniHou").set_description(e.to_string()).show();
        }
        std::process::exit(1);
    }
}



