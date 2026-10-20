use super::*;

impl App {
    pub(super) fn poll_frame_jobs(&mut self, ctx: &egui::Context) {
        self.poll_http(ctx);
        if self.blender_job.as_ref().is_some_and(|job| job.is_finished()) {
            let result = self.blender_job.take().unwrap().join();
            self.blender.busy = false;
            match result {
                Ok(Ok((message, installed))) => {
                    self.blender.message = message;
                    if installed { self.blender.addon_installed = true; }
                }
                Ok(Err(e)) => self.blender.message = format!("Blender operation failed: {e}"),
                Err(_) => self.blender.message = "Blender setup worker panicked.".into(),
            }
        }
        if self.blender_job.is_some() { ctx.request_repaint_after(Duration::from_millis(50)); }
    }
    pub(super) fn poll_frame_links(&mut self, ctx: &egui::Context) {
        let requests = self.link.as_ref().map(|link| link.poll()).unwrap_or_default();
        if self.link.as_ref().is_some_and(|link|link.apply_asset_updates(&mut self.materials)) {
            if let (Some(gpu),Some(scene))=(&mut self.gpu,&self.scene) {
                gpu.upload_textures(&self.state.device,&self.state.queue,scene,&mut self.materials);
            }
            ctx.request_repaint();
        }
        for request in requests {
            let result: Result<()> = match request.message["type"].as_str() {
                Some("live_activate" | "live_update" | "live_remove" | "live_replace") => {
                    let mut link=self.link.take().unwrap();
                    let edits=link.handle_live(&request,&mut self.editor);
                    self.link=Some(link);
                    match edits {
                        Ok(edits)=>{
                            if request.message["type"]=="live_activate" { ctx.send_viewport_cmd(egui::ViewportCommand::Focus); }
                            self.display=Graph::from_value(self.editor.doc.value()).ok().and_then(|g|g.main_output().ok()).unwrap_or_default();
                            self.selected=self.display.clone();
                            self.selected_set=std::collections::BTreeSet::from([self.display.clone()]);
                            for edit in edits { self.send(edit); }
                            Ok(())
                        },
                        Err(e)=>Err(e.into()),
                    }
                },
                Some("embed_conflict") => {
                    // Never decided silently: the modal below asks (graph_embed_v1).
                    self.embed_conflict = Some(request.message.clone());
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    Ok(())
                },
                Some("input_define") => {
                    let live = self.link.as_ref().and_then(|l| l.bound_path().map(str::to_owned));
                    let (reply, changed) = minihou_app::input_define::handle(&mut self.editor, live.as_deref(), &request.message);
                    if changed { self.send(Edit::Replace(self.editor.doc.value().clone())); }
                    if let Some(link) = &self.link { link.reply(request.peer, reply); }
                    Ok(())
                },
                Some("set_param") => minihou_app::link_param_edit(self.editor.doc.value(), &request.message)
                    .map(|edit| {
                        self.finish_history();
                        self.edit(edit, false);
                    }),
                Some("object_geo" | "object_missing" | "image_data" | "image_missing") => {
                    match minihou_app::link_protocol::external_edit(&request.message,&request.arrays) {
                        Ok(edit)=>{self.send(edit);Ok(())},
                        Err(e)=>{
                            // Invalidate stale input geometry even when a replacement frame is malformed.
                            let image=request.message["type"].as_str().unwrap_or("").starts_with("image_");
                            if let Some(name)=request.message[if image{"image"}else{"object"}].as_str(){
                                self.send(Edit::External{name:format!("{}:{name}",if image{"image"}else{"object"}),input:minihou_core::nodes::blender_input::ExternalInput::Missing(format!("could not read input: {e}"))});
                            }
                            Err(e.into())
                        }
                    }
                },
                Some("set_frame") => match self.link.as_mut().unwrap().set_frame(&request.message) {
                    Ok(changed) => {
                        // Timeline updates stay out of saved parameters and undo history: the session
                        // recooks only time-dependent nodes (solvers, @Frame/@Time) at the new frame.
                        if changed {
                            let (frame, fps) = { let link = self.link.as_ref().unwrap(); (link.frame, link.fps) };
                            self.send(Edit::Time { frame, fps });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e.into()),
                },
                _ => Ok(()),
            };
            if let Err(e) = result {
                if let Some(link) = &self.link {
                    link.reply_error(request.peer, request.message["type"].as_str().unwrap_or("request"), &e.to_string());
                }
            }
        }
        self.embed_conflict_modal(ctx);
        if let Some(link) = &self.link {
            link.set_open_dirty(!link.live_active() && self.editor.dirty());
            link.document_changed(&self.editor.doc, &self.editor.path);
            let clients = link.clients();
            let disconnected = !self.blender.clients.is_empty() && clients.is_empty();
            self.blender.clients = clients;
            self.blender.link_message = ui::blender_ui::connection_message(
                link.listening(), &self.blender.clients, &self.blender.link_message, disconnected);
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}
