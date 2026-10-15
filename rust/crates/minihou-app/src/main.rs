mod viewport;
use eframe::{egui, egui_wgpu};
use minihou_app::{Camera, Document, ParamKind, Result, param_kind};
use minihou_core::{nodes, session::Session};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};
use viewport::{Gpu, Mode, Scene, Target};

struct App {
    doc: Document, session: Session, path: PathBuf, selected: String, display: String,
    state: egui_wgpu::RenderState, gpu: Option<Gpu>, target: Option<Target>,
    camera: Camera, bounds: ([f32;3], f32), mode: Mode,
    cook_ms: f64, cook_error: Option<String>, file_status: String, dirty: bool,
}
impl App {
    fn new(doc: Document, path: PathBuf, state: egui_wgpu::RenderState) -> Result<Self> {
        let session = doc.session()?;
        let display = session.graph().main_output()?;
        let mut app = Self { selected:display.clone(),display,doc,session,path,state,gpu:None,target:None,
            camera:Camera::frame([0.;3],1.),bounds:([0.;3],1.),mode:Mode::Shaded,
            cook_ms:0.,cook_error:None,file_status:String::new(),dirty:false };
        app.cook();
        app.frame_mesh();
        Ok(app)
    }
    fn cook(&mut self) {
        let t = Instant::now();
        let result = self.session.cook(Some(&self.display));
        self.cook_ms = t.elapsed().as_secs_f64()*1000.;
        match result {
            Ok(g) => {
                let scene = Scene::new(&g);
                self.bounds = (scene.center,scene.radius);
                if let Some(gpu)=&mut self.gpu { gpu.upload(&self.state.device,&scene); } else { self.gpu = Some(Gpu::new(&self.state.device,&scene)); }
                self.cook_error = None;
            }
            Err(e) => { self.cook_error = Some(e.to_string()); self.gpu = None; }
        }
    }
    fn frame_mesh(&mut self) { self.camera = Camera::frame(self.bounds.0,self.bounds.1); }
    fn save(&mut self) {
        match self.doc.save(&self.path) {
            Ok(()) => { self.dirty = false; self.file_status = "Saved".into(); }
            Err(e) => self.file_status = format!("Save failed: {e}"),
        }
    }
}
fn widget(ui: &mut egui::Ui, kind: ParamKind, value: &mut Value) -> bool {
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
            ui.horizontal(|ui| { for x in &mut v { changed |= ui.add(egui::DragValue::new(x).speed(0.01)).changed(); } });
            if changed { *value=json!(v); } changed
        }
        ParamKind::Unsupported => { ui.label(value.to_string()); false }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let mut recook = false;
        let mut save = ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL,egui::Key::S));
        let mut frame = !ctx.wants_keyboard_input() && ctx.input(|i|i.key_pressed(egui::Key::F));
        egui::TopBottomPanel::top("toolbar").show(ctx,|ui| {
            ui.horizontal(|ui| {
                for (mode,label) in [(Mode::Solid,"Solid"),(Mode::Shaded,"Shaded"),(Mode::Wire,"Wire"),(Mode::ShadedWire,"Shaded+Wire")] {
                    ui.selectable_value(&mut self.mode,mode,label);
                }
                frame |= ui.button("Frame (F)").clicked();
                save |= ui.button("Save (Ctrl+S)").clicked();
                ui.label(format!("{}{}",self.path.display(),if self.dirty {" *"} else {""}));
            });
        });
        egui::SidePanel::left("nodes").default_width(220.).show(ctx,|ui| {
            ui.heading("Nodes");
            egui::ScrollArea::vertical().show(ui,|ui| {
                for (id,node) in &self.session.graph().nodes {
                    ui.horizontal(|ui| {
                        if ui.radio(self.display==*id,"").on_hover_text("Display this node").clicked() {
                            self.display=id.clone();
                            if let Err(e)=self.doc.set_display(id) { self.file_status=e.to_string(); }
                            self.dirty=true; recook=true;
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
            let node = self.session.graph().nodes.iter().find(|(id,_)|id==&selected).map(|(_,n)|(n.node_type.clone(),n.bypass));
            if let Some((typ,mut bypass))=node {
                ui.label(&typ);
                if ui.checkbox(&mut bypass,"Bypass").changed() {
                    match self.doc.set_bypass(&mut self.session,&selected,bypass) {
                        Ok(()) => {recook=true;self.dirty=true;},
                        Err(e) => self.file_status=e.to_string(),
                    }
                }
                egui::ScrollArea::vertical().show(ui,|ui| {
                    if let Some(spec)=nodes::get(&typ) {
                        for param in spec.params {
                            ui.push_id(param.name,|ui| {
                                ui.label(param.name);
                                let default=(param.default)();
                                let mut value=self.doc.value()["nodes"][&selected]["params"].get(param.name).unwrap_or(&default).clone();
                                let response = ui.add_enabled_ui(param.supported,|ui|widget(ui,param_kind(&default),&mut value)).response;
                                if !param.supported { response.on_hover_text("The Rust backend does not support it yet").on_disabled_hover_text("The Rust backend does not support it yet"); }
                                // Response.changed is not propagated by a scope; inspect the value instead.
                                let old=self.doc.value()["nodes"][&selected]["params"].get(param.name).unwrap_or(&default);
                                if param.supported && &value!=old {
                                    match self.doc.set_param(&mut self.session,&selected,param.name,value) {
                                        Ok(()) => {recook=true;self.dirty=true;},
                                        Err(e) => self.file_status=e.to_string(),
                                    }
                                }

                            });
                        }
                    } else { ui.colored_label(egui::Color32::RED,"Node type is not supported by the Rust backend yet"); }
                });
            }
        });
        // Batch all edits made in this egui frame into one synchronous cook.
        if recook { self.cook(); }
        if frame { self.frame_mesh(); }
        if save { self.save(); }
        egui::TopBottomPanel::bottom("stats").show(ctx,|ui| {
            let stats=self.session.last_stats();
            ui.label(format!("cooked: {} in {:.1} ms; cached: {}",stats.cooked.join(", "),self.cook_ms,stats.cached.join(", ")));
            if let Some(error)=&self.cook_error { ui.colored_label(egui::Color32::RED,error); }
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
    let (headless,path)=match args.as_slice() {
        [flag,path] if flag=="--selftest" => (true,PathBuf::from(path)),
        [path] => (false,PathBuf::from(path)),
        _ => return Err("usage: minihou-app [--selftest] GRAPH.json".into()),
    };
    let doc=Document::open(&path)?;
    if headless { println!("{}",minihou_app::selftest(&doc)?); return Ok(()); }
    let options=eframe::NativeOptions { renderer:eframe::Renderer::Wgpu,viewport:egui::ViewportBuilder::default().with_inner_size([1280.,800.]),..Default::default() };
    eframe::run_native("MiniHou",options,Box::new(move |cc| {
        let state=cc.wgpu_render_state.clone().ok_or("wgpu renderer unavailable")?;
        Ok(Box::new(App::new(doc,path,state)?))
    })).map_err(|e|e.to_string())?;
    Ok(())
}
fn main() {
    if let Err(e)=run() { eprintln!("{e}"); std::process::exit(1); }
}



