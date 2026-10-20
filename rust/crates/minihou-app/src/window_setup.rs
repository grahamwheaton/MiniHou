use super::*;

/// Open a graph given on the command line; the error names the path (a bare "os error 2" says nothing to someone who double-clicked).
fn open_graph(path: &std::path::Path) -> Result<Document> {
    Document::open(path).map_err(|e| format!("Cannot open {}: {e}

Run the app without arguments to open the built-in demo graph.", std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()).display()).into())
}
/// The app icon (window and taskbar), decoded from the embedded PNG; None if it cannot be decoded.
fn app_icon() -> Option<egui::IconData> {
    let image=image::load_from_memory(include_bytes!("../assets/app_icon.png")).ok()?.to_rgba8();
    let (width,height)=image.dimensions();
    Some(egui::IconData { rgba: image.into_raw(), width, height })
}

pub(super) fn run() -> Result<()> {
    let args:Vec<_>=std::env::args_os().skip(1).collect();
    // Started with no arguments (a double-click): open the built-in demo graph rather than exit with a usage line nobody sees.
    let (headless,path,doc)=match args.as_slice() {
        [flag,path] if flag=="--selftest" => { let path=PathBuf::from(path); let doc=open_graph(&path)?; (true,path,doc) }
        [path] => { let path=PathBuf::from(path); let doc=open_graph(&path)?; (false,path,doc) }
        [] => (false,PathBuf::from("untitled.json"),Document::parse(include_str!("default_graph.json").to_owned())?),
        _ => return Err("usage: minihou-app [--selftest] [GRAPH.json]".into()),
    };
    if headless { println!("{}",minihou_app::selftest(&doc)?); return Ok(()); }
    let mut viewport=egui::ViewportBuilder::default().with_inner_size([1280.,800.]).with_maximized(true);
    if let Some(icon)=app_icon() { viewport=viewport.with_icon(std::sync::Arc::new(icon)); }
    let options=eframe::NativeOptions { renderer:eframe::Renderer::Wgpu,viewport,..Default::default() };
    eframe::run_native("MiniHou",options,Box::new(move |cc| {
        // The builder's maximized flag alone can leave a Windows window at its restored size while
        // the OS believes it is maximized (the first Maximize click then does nothing visible), so
        // apply the real state explicitly on the first frame.
        cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
        cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        let state=cc.wgpu_render_state.clone().ok_or("wgpu renderer unavailable")?;
        Ok(Box::new(App::new(doc,path,state)?))
    })).map_err(|e|e.to_string())?;
    Ok(())
}
