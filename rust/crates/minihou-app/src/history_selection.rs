use super::*;

pub(crate) fn snapshot(selection:&picking::Selection)->SelectionSnapshot {
    SelectionSnapshot { mode: match selection.mode { ui::SelectionMode::Object=>0, ui::SelectionMode::Points=>1, ui::SelectionMode::Edges=>2, ui::SelectionMode::Faces=>3 },
        points:selection.points.clone(), edges:selection.edges.clone(), tris:selection.tris.clone() }
}
pub(crate) fn restore(snapshot:SelectionSnapshot)->picking::Selection {
    picking::Selection { mode:match snapshot.mode { 1=>ui::SelectionMode::Points, 2=>ui::SelectionMode::Edges, 3=>ui::SelectionMode::Faces, _=>ui::SelectionMode::Object },
        points:snapshot.points, edges:snapshot.edges, tris:snapshot.tris }
}
pub(crate) fn selection_state(selection:&picking::Selection,stash:&picking::Selection)->SelectionState {
    SelectionState { selection:snapshot(selection), stashed:snapshot(stash) }
}
impl App {
    pub(crate) fn sync_selection_baseline(&mut self) { self.sel_baseline = (self.selection.clone(), self.stashed_selection.clone()); }
    pub(crate) fn selection_state(&self)->SelectionState { selection_state(&self.selection,&self.stashed_selection) }
    pub(crate) fn finish_history(&mut self) {
        let selection=self.selection_state();
        self.editor.history.finish_with_selection(&self.editor.doc,Some(selection));
    }
    pub(crate) fn record_history(&mut self,before:Document) {
        let selection=self.selection_state();
        self.editor.history.record_with_selection(before,&self.editor.doc,Some(selection.clone()),Some(selection));
    }
    pub(crate) fn apply_selection_state(&mut self,state:SelectionState) {
        self.selection=restore(state.selection); self.stashed_selection=restore(state.stashed);
        if self.selection.mode!=ui::SelectionMode::Object { self.editor_state.apply(ui::EditorChange::Selection(self.selection.mode)); }
        self.sync_selection_baseline(); self.selection_dirty=true;
    }
    /// A viewport selection change becomes one step on the shared undo stack.
    pub(crate) fn record_selection_step(&mut self) {
        if self.selection == self.sel_baseline.0 { return; }
        let before = std::mem::replace(&mut self.sel_baseline, (self.selection.clone(), self.stashed_selection.clone()));
        let before=selection_state(&before.0,&before.1);
        let after=self.selection_state();
        self.editor.history.record_selection_state(&self.editor.doc,Some(before),Some(after));
    }

}
