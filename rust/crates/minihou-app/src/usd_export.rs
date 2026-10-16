//! USD dialog; all geometry work goes to the existing full-resolution cook worker.
use super::*;
use minihou_app::{
    http_api::Response,
    http_geometry::{Query, QueryKind},
};
use std::sync::{mpsc, Arc, Mutex};
#[derive(Clone, Default)]
struct Dialog {
    params: Value,
    receiver: Option<Arc<Mutex<mpsc::Receiver<Response>>>>,
}
fn id() -> egui::Id {
    egui::Id::new("usd_export_dialog")
}
pub fn open(ctx: &egui::Context, path: &std::path::Path) {
    let mut p = serde_json::Map::new();
    for s in minihou_core::nodes::get("usd_export").unwrap().params {
        p.insert(s.name.into(), (s.default)());
    }
    p.insert(
        "file".into(),
        json!(path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("exports/scene.usdc")
            .to_string_lossy()),
    );
    ctx.data_mut(|d| {
        d.insert_temp(
            id(),
            Dialog {
                params: Value::Object(p),
                receiver: None,
            },
        )
    });
}
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut state) = ctx.data_mut(|d| d.remove_temp::<Dialog>(id())) else {
        return;
    };
    let mut keep = true;
    let mut export = false;
    egui::Window::new("Export USD")
        .collapsible(false)
        .resizable(false)
        .open(&mut keep)
        .show(ctx, |ui| {
            if state.receiver.is_some() {
                ui.label("Exporting full-resolution geometry…");
                ui.spinner();
                return;
            }
            ui.label("Display output and additional outputs");
            egui::Grid::new("usd_export_fields")
                .num_columns(2)
                .show(ui, |ui| {
                    for name in [
                        "file",
                        "format",
                        "name",
                        "root",
                        "frames",
                        "start",
                        "end",
                        "step",
                        "up_axis",
                        "units",
                        "materials",
                        "embed_graph",
                        "overwrite",
                    ] {
                        if ["start", "end", "step"].contains(&name)
                            && state.params["frames"] != "range"
                        {
                            continue;
                        }
                        if name == "name" {
                            continue;
                        } // Multiple outputs keep their own node names.
                        let label = match name {
                            "file" => "File",
                            "format" => "Format",
                            "root" => "Root prim",
                            "frames" => "Frames",
                            "up_axis" => "Up axis",
                            "embed_graph" => "Embed graph",
                            "materials" => "Materials",
                            "overwrite" => "Overwrite",
                            "units" => "Units",
                            "start" => "Start",
                            "end" => "End",
                            _ => "Step",
                        };
                        ui.label(label);
                        let v = &mut state.params[name];
                        if let Some(b) = v.as_bool() {
                            let mut b = b;
                            ui.checkbox(&mut b, "");
                            *v = json!(b);
                        } else if let Some(n) = v.as_i64() {
                            let mut n = n;
                            ui.add(egui::DragValue::new(&mut n));
                            *v = json!(n);
                        } else {
                            let mut text = v.as_str().unwrap_or("").to_owned();
                            let choices: &[&str] = match name {
                                "format" => &["auto", "usdc", "usda"],
                                "frames" => &["current", "range"],
                                "up_axis" => &["z", "y"],
                                "units" => {
                                    &["meters", "centimeters", "millimeters", "inches", "feet"]
                                }
                                _ => &[],
                            };
                            if choices.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.text_edit_singleline(&mut text);
                                    if name == "file" && ui.button("Browse…").clicked() {
                                        if let Some(path) = rfd::FileDialog::new()
                                            .add_filter("USD", &["usd", "usda", "usdc"])
                                            .set_file_name(&text)
                                            .save_file()
                                        {
                                            text = path.to_string_lossy().into();
                                        }
                                    }
                                });
                            } else {
                                egui::ComboBox::from_id_salt(("usd", name))
                                    .selected_text(&text)
                                    .show_ui(ui, |ui| {
                                        for s in choices {
                                            ui.selectable_value(&mut text, (*s).to_owned(), *s);
                                        }
                                    });
                            }
                            *v = json!(text);
                        }
                        ui.end_row();
                    }
                });
            export = ui.button("Export").clicked();
        });
    if export {
        let result = (|| -> Result<()> {
            let p = minihou_core::params::Params::resolve(
                minihou_core::nodes::get("usd_export").unwrap().params,
                state.params.as_object(),
            )?;
            let path = minihou_core::nodes::usd_nodes::export_target(&p)?;
            minihou_core::nodes::usd_nodes::export_frames(&p)?;
            let (tx, rx) = mpsc::channel();
            app.worker.query(Query {
                kind: QueryKind::Export {
                    path,
                    node: None,
                    graph: p.b("embed_graph")?.then(|| app.editor.doc.value().clone()),
                    overwrite: p.b("overwrite")?,
                    options: state.params.clone(),
                    root: app
                        .editor
                        .path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .to_owned(),
                },
                revision: app.cooking.requested,
                reply: tx,
            })?;
            state.receiver = Some(Arc::new(Mutex::new(rx)));
            Ok(())
        })();
        if let Err(e) = result {
            app.file_status = format!("USD export: {e}");
        }
    }
    if let Some(rx) = &state.receiver {
        let result = rx.lock().unwrap().try_recv();
        match result {
            Ok(response) => {
                if response.status < 400 {
                    app.file_status = response.body["message"]
                        .as_str()
                        .unwrap_or("USD saved")
                        .into();
                    keep = false;
                } else {
                    app.file_status = format!("USD export: {}", response.body["error"]);
                    state.receiver = None;
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                app.file_status = "USD export worker stopped".into();
                state.receiver = None;
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(50)),
        }
    }
    if keep {
        ctx.data_mut(|d| d.insert_temp(id(), state));
    }
}
