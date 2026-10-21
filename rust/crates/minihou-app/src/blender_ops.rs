use super::*;

impl App {
    pub(super) fn restart_blender_link(&mut self) {
        let mut link_time=None;
        if let Some(mut link)=self.link.take() {
            for edit in link.restore_editor(&mut self.editor) { self.send(edit); }
            link_time=Some((link.frame,link.fps));
        }
        self.blender.clients.clear();
        match ui::blender_ui::blender_link::LinkServer::start(&self.editor.doc, &self.editor.path) {
            Ok(mut link) => {
                if let Some((frame,fps))=link_time {link.frame=frame;link.fps=fps;}
                self.link = Some(link);
                self.blender.link_message = "Listening on 127.0.0.1:8765; waiting for Blender.".into();
                if !display_is_empty(self.editor.doc.value()) {
                    self.send(Edit::Replace(self.editor.doc.value().clone()));
                }
            }
            Err(e) => {
                let hint = if e.kind() == std::io::ErrorKind::AddrInUse {
                    " Port is already in use. Close the other link server, then click Restart link."
                } else { "" };
                self.blender.link_message = format!("Cannot listen on 127.0.0.1:8765: {e}.{hint}");
                self.blender_bar = true;
            }
        }
    }
    pub(super) fn blender_task(&mut self, status: &str,
        task: impl FnOnce() -> std::io::Result<(String, bool)> + Send + 'static) {
        if self.blender_job.is_some() { return; }
        self.blender.message = status.into();
        match std::thread::Builder::new().name("minihou-blender-setup".into()).spawn(task) {
            Ok(job) => { self.blender_job = Some(job); self.blender.busy = true; }
            Err(e) => self.blender.message = format!("Could not start Blender task: {e}"),
        }
    }
    pub(super) fn blender_action(&mut self, action: UiAction, ctx: &egui::Context) {
        use ui::blender_ui::{self, blender_link as bl};
        self.blender_bar = true;
        if matches!(action, UiAction::BlenderRestartLink) {
            self.restart_blender_link();
            self.blender.message = if self.link.is_some() { "Link restarted; listening on 127.0.0.1:8765. Blender with Auto-connect enabled will reconnect.".into() }
                else { self.blender.link_message.clone() };
            return;
        }
        if self.blender_job.is_some() { return; }
        match action {
            UiAction::BlenderDetect => {
                let installs = bl::find_installs();
                self.blender.installs = installs.iter().map(|(name, path)| (name.clone(), path.display().to_string())).collect();
                self.blender_paths = installs.into_iter().map(|(_, path)| path).collect();
                self.blender.message = if self.blender_paths.is_empty() {
                    "No Blender installations found. Set BLENDER_EXE to the executable path and restart MiniHou, or install Blender and detect again.".into()
                } else { format!("Detected {} Blender installation(s).", self.blender_paths.len()) };
            }
            UiAction::BlenderInstall(index) => {
                if let Some(exe) = self.blender_paths.get(index).cloned() {
                    self.blender_task("Installing add-on...", move || {
                        let zip = bl::prepare_addon_zip()?;
                        bl::install_addon(&exe, &zip).map(|message| (message, true))
                    });
                } else { self.blender.message = "No Blender installation selected. Click Detect installs.".into(); }
            }
            UiAction::BlenderOpen => {
                if !self.link.as_ref().is_some_and(|link| link.listening()) { self.restart_blender_link(); }
                if self.link.is_none() { return; }
                let index = blender_ui::selected_index(ctx, self.blender_paths.len());
                if let Some(exe) = self.blender_paths.get(index).cloned() {
                    let graph = match bl::prepare_launch_graph(&self.editor.doc, &self.editor.path, self.editor.dirty()) {
                        Ok(graph) => graph,
                        Err(e) => { self.blender.message = format!("Could not prepare current document for Blender: {e}"); return; }
                    };
                    self.blender_task("Starting Blender...", move || {
                        let mut child = bl::open_blender(&exe, &graph.path)?;
                        // Reap the process without holding the UI's setup task for Blender's lifetime.
                        std::thread::spawn(move || { let _ = child.wait(); drop(graph); });
                        Ok(("Blender started and connected to 127.0.0.1:8765.".into(), false))
                    });
                } else { self.blender.message = "No Blender installation selected. Click Detect installs.".into(); }
            }
            UiAction::BlenderDownloadAddon => {
                if let Some(destination) = rfd::FileDialog::new().set_file_name("MiniHou_Link.zip")
                    .add_filter("ZIP archive", &["zip"]).save_file() {
                    self.blender_task("Preparing add-on ZIP...", move || {
                        let zip = bl::prepare_addon_zip()?;
                        std::fs::copy(zip, &destination)?;
                        Ok((format!("Saved add-on ZIP to {}", destination.display()), false))
                    });
                } else { self.blender.message = "Save ZIP cancelled.".into(); }
            }
            _ => {}
        }
    }

}
