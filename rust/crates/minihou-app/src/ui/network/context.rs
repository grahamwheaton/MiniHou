use super::*;

pub(super) fn node_context_menu(response: &egui::Response, node_id: &String, v: &View, out: &mut Actions) {
            response.clone().context_menu(|ui| {
                if ui.button("Rename node...").clicked() {
                    ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("rename_link_node"),(node_id.clone(),node_id.clone())));
                    ui.close();
                }
                let links = minihou_app::value_link_ui::links(v.doc);
                for link in links.iter().filter(|l| &l.target == node_id && !l.broken) {
                    if ui.button(format!("Go to source: {}.{}",link.source,link.source_param)).clicked() {
                        out.push(UiAction::NavigateLink {node:link.source.clone(),param:link.source_param.clone()}); ui.close();
                    }
                }
                for link in links.iter().filter(|l| &l.source == node_id) {
                    if ui.button(format!("Go to target: {}.{}",link.target,link.target_param)).clicked() {
                        out.push(UiAction::NavigateLink {node:link.target.clone(),param:link.target_param.clone()}); ui.close();
                    }
                }
            });
}

pub(super) fn canvas_menu(ctx: &egui::Context, id: Id, state: &mut State, v: &View, out: &mut Actions, r: Rect, positions: &[(String, Pos2)]) {
    if let Some(menu) = &mut state.menu {
        let canvas_pos = menu.canvas_pos();
        if let Some(choice) = network_menu::show(ctx, id.with("menu"), menu, v, out) {
            match choice {
                network_menu::Choice::Close => {
                    if matches!(out.0.last(), Some(UiAction::AddNode { after: None, .. })) {
                        state.add_pos = canvas_pos;
                    } else if !matches!(out.0.last(), Some(UiAction::AddNode { .. })) {
                        state.add_pos = None;
                    }
                    state.menu = None;
                }
                network_menu::Choice::FrameAll => { frame_all(state, r, content_bounds(&positions)); state.menu = None; state.add_pos = None; },
                network_menu::Choice::Tidy => { tidy_state(state); out.push(UiAction::TidyNetwork); state.menu = None; state.add_pos = None; },
            }
        }
    }
}
