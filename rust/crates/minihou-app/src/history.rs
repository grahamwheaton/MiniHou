use crate::{Document, Edit, Result};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionSnapshot {
    pub mode: u8,
    pub points: Vec<u32>,
    pub edges: Vec<(u32, u32)>,
    pub tris: Vec<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionState {
    pub selection: SelectionSnapshot,
    pub stashed: SelectionSnapshot,
}

pub enum HistoryOutcome {
    Selection(SelectionState),
    DocEdit { edit: Edit, selection: Option<SelectionState> },
}

struct HistoryStep {
    before: Document,
    after: Document,
    selection_before: Option<SelectionState>,
    selection_after: Option<SelectionState>,
    selection_only: bool,
}

#[derive(Default)]
pub struct History {
    undo: Vec<HistoryStep>,
    redo: Vec<HistoryStep>,
    pending: Option<(Document, Option<SelectionState>)>,
}
impl History {
    pub fn edit(&mut self, doc: &mut Document, edit: &Edit) -> Result<()> {
        self.edit_with_selection(doc, edit, None)
    }
    pub fn edit_with_selection(&mut self, doc: &mut Document, edit: &Edit, selection: Option<SelectionState>) -> Result<()> {
        let before = doc.clone();
        doc.edit(edit)?;
        if before.text != doc.text {
            self.pending.get_or_insert((before, selection));
            self.redo.clear();
        }
        Ok(())
    }
    /// A structural change (adding a node): `doc` becomes `new` as one undo step.
    pub fn replace(&mut self, doc: &mut Document, new: Document) {
        self.replace_with_selection(doc, new, None, None);
    }
    pub fn replace_with_selection(&mut self, doc: &mut Document, new: Document, before: Option<SelectionState>, after: Option<SelectionState>) {
        self.finish_with_selection(doc, before.clone());
        if doc.text != new.text {
            self.undo.push(HistoryStep { before: doc.clone(), after: new.clone(), selection_before: before, selection_after: after, selection_only: false });
            self.redo.clear();
            *doc = new;
        }
    }
    /// Record a completed drag whose document was already updated in place.
    pub fn record(&mut self, before: Document, after: &Document) {
        self.record_with_selection(before, after, None, None);
    }
    pub fn record_with_selection(&mut self, before: Document, after: &Document, selection_before: Option<SelectionState>, selection_after: Option<SelectionState>) {
        self.finish_with_selection(after, selection_before.clone());
        if before.text != after.text {
            self.undo.push(HistoryStep { before, after: after.clone(), selection_before, selection_after, selection_only: false });
            self.redo.clear();
        }
    }
    /// Called at the end of a drag, or immediately for discrete edits.
    pub fn finish(&mut self, doc: &Document) {
        self.finish_with_selection(doc, None);
    }
    pub fn finish_with_selection(&mut self, doc: &Document, selection: Option<SelectionState>) {
        if let Some((before, selection_before)) = self.pending.take() {
            if before.text != doc.text { self.undo.push(HistoryStep { before, after: doc.clone(), selection_before, selection_after: selection, selection_only: false }); }
        }
    }
    /// A viewport selection change: one undo step that leaves the document as it is (the caller keeps the selections).
    pub fn record_selection(&mut self, doc: &Document) {
        self.record_selection_state(doc, None, None);
    }
    pub fn record_selection_state(&mut self, doc: &Document, before: Option<SelectionState>, after: Option<SelectionState>) {
        self.finish_with_selection(doc, before.clone());
        self.undo.push(HistoryStep { before: doc.clone(), after: doc.clone(), selection_before: before, selection_after: after, selection_only: true });
        self.redo.clear();
    }
    /// Forget an in-progress edit without recording it (a modal tool records its own single step).
    pub fn drop_pending(&mut self) { self.pending = None; }
    pub fn can_undo(&self) -> bool { !self.undo.is_empty() || self.pending.is_some() }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }
    pub fn undo(&mut self, doc: &mut Document) -> Option<Edit> {
        self.undo_outcome(doc).map(|outcome| match outcome { HistoryOutcome::Selection(_) => Edit::Replace(doc.value.clone()), HistoryOutcome::DocEdit { edit, .. } => edit })
    }
    pub fn redo(&mut self, doc: &mut Document) -> Option<Edit> {
        self.redo_outcome(doc).map(|outcome| match outcome { HistoryOutcome::Selection(_) => Edit::Replace(doc.value.clone()), HistoryOutcome::DocEdit { edit, .. } => edit })
    }
    pub fn undo_outcome(&mut self, doc: &mut Document) -> Option<HistoryOutcome> {
        self.finish(doc);
        let step = self.undo.pop()?;
        *doc = step.before.clone();
        let outcome = if step.selection_only {
            HistoryOutcome::Selection(step.selection_before.clone().unwrap_or_default())
        } else {
            HistoryOutcome::DocEdit { edit: Edit::Replace(doc.value.clone()), selection: step.selection_before.clone() }
        };
        self.redo.push(step);
        Some(outcome)
    }
    pub fn redo_outcome(&mut self, doc: &mut Document) -> Option<HistoryOutcome> {
        self.finish(doc);
        let step = self.redo.pop()?;
        *doc = step.after.clone();
        let outcome = if step.selection_only {
            HistoryOutcome::Selection(step.selection_after.clone().unwrap_or_default())
        } else {
            HistoryOutcome::DocEdit { edit: Edit::Replace(doc.value.clone()), selection: step.selection_after.clone() }
        };
        self.undo.push(step);
        Some(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minihou_core::Graph;
    use serde_json::json;

    #[test]
    fn history_selection_steps_interleave_with_graph_edits_in_order() {
        let mut doc=Document::parse("{
  \"nodes\": {\"out\": {\"type\": \"box\", \"params\": {\"size\": [1,2,3]}}},
  \"display\": \"out\"
}".into()).unwrap();
        let mut history=History::default();
        let s0=SelectionState::default();
        let s1=SelectionState { selection: SelectionSnapshot { mode:1, points:vec![2], ..Default::default() }, ..Default::default() };
        let s2=SelectionState { selection: SelectionSnapshot { mode:3, tris:vec![5], ..Default::default() }, ..Default::default() };
        history.record_selection_state(&doc,Some(s0.clone()),Some(s1.clone()));
        history.edit_with_selection(&mut doc,&Edit::Param{node:"out".into(),name:"size".into(),value:json!([4,5,6])},Some(s1.clone())).unwrap();
        history.finish_with_selection(&doc,Some(s1.clone()));
        history.record_selection_state(&doc,Some(s1.clone()),Some(s2.clone()));
        let edited=doc.text().to_string();
        assert!(matches!(history.undo_outcome(&mut doc),Some(HistoryOutcome::Selection(s)) if s==s1));
        assert_eq!(doc.text(),edited);
        assert!(matches!(history.undo_outcome(&mut doc),Some(HistoryOutcome::DocEdit { selection:Some(s),.. }) if s==s1));
        assert_ne!(doc.text(),edited);
        assert!(matches!(history.undo_outcome(&mut doc),Some(HistoryOutcome::Selection(s)) if s==s0));
        assert!(!history.can_undo());
        assert!(matches!(history.redo_outcome(&mut doc),Some(HistoryOutcome::Selection(s)) if s==s1));
        assert!(matches!(history.redo_outcome(&mut doc),Some(HistoryOutcome::DocEdit { selection:Some(s),.. }) if s==s1));
        assert_eq!(doc.text(),edited);
        history.record_selection_state(&doc,Some(s1),Some(s2));
        assert!(!history.can_redo());
    }

    #[test]
    fn history_drag_keeps_first_document_and_selection_baseline() {
        let mut doc=Document::parse(json!({"nodes":{"out":{"type":"box","params":{"size":[1,2,3]}}},"display":"out"}).to_string()).unwrap();
        let original=doc.text().to_string();
        let first=SelectionState { selection: SelectionSnapshot { mode:1, points:vec![1], ..Default::default() }, ..Default::default() };
        let later=SelectionState { selection: SelectionSnapshot { mode:1, points:vec![2], ..Default::default() }, ..Default::default() };
        let mut history=History::default();
        for (size,selection) in [(4,first.clone()),(5,later.clone())] {
            history.edit_with_selection(&mut doc,&Edit::Param{node:"out".into(),name:"size".into(),value:json!([size,2,3])},Some(selection)).unwrap();
        }
        history.finish_with_selection(&doc,Some(later.clone()));
        assert!(matches!(history.undo_outcome(&mut doc),Some(HistoryOutcome::DocEdit { selection:Some(s),.. }) if s==first));
        assert_eq!(doc.text(),original);
        assert!(matches!(history.redo_outcome(&mut doc),Some(HistoryOutcome::DocEdit { selection:Some(s),.. }) if s==later));
        assert_eq!(doc.value()["nodes"]["out"]["params"]["size"],json!([5,2,3]));
    }

    #[test]
    fn history_replace_and_record_keep_selection_on_both_sides() {
        let mut doc=Document::parse(json!({"nodes":{},"display":""}).to_string()).unwrap();
        let selected=SelectionState { selection: SelectionSnapshot { mode:3, tris:vec![4], ..Default::default() }, ..Default::default() };
        let empty=SelectionState::default();
        let mut history=History::default();
        let added=Document::parse(json!({"nodes":{"out":{"type":"box"}},"display":"out"}).to_string()).unwrap();
        history.replace_with_selection(&mut doc,added,Some(empty.clone()),Some(selected.clone()));
        let before=doc.clone();
        doc.edit(&Edit::Bypass{node:"out".into(),flag:true}).unwrap();
        history.record_with_selection(before,&doc,Some(selected.clone()),Some(empty.clone()));
        assert!(matches!(history.undo_outcome(&mut doc),Some(HistoryOutcome::DocEdit { selection:Some(s),.. }) if s==selected));
        assert!(matches!(history.undo_outcome(&mut doc),Some(HistoryOutcome::DocEdit { selection:Some(s),.. }) if s==empty));
        assert!(matches!(history.redo_outcome(&mut doc),Some(HistoryOutcome::DocEdit { selection:Some(s),.. }) if s==selected));
        assert!(matches!(history.redo_outcome(&mut doc),Some(HistoryOutcome::DocEdit { selection:Some(s),.. }) if s==empty));
    }

    #[test]
    fn channels_and_locks_roundtrip_through_lossless_document_history() {
        let original="{\n  \"nodes\": {\"driver\": {\"type\": \"box\"}, \"out\": {\"type\": \"box\", \"params\": {\"size\": [1,2,3]}, \"custom\": 42}},\n  \"display\": \"out\", \"unknown\": \"stay\"\n}";
        let mut doc=Document::parse(original.into()).unwrap();
        let mut history=History::default();
        history.edit(&mut doc,&Edit::Channel{node:"out".into(),path:"size.y".into(),expression:Some("ch('../driver/size.y')".into())}).unwrap();
        history.edit(&mut doc,&Edit::LockParam{node:"out".into(),path:"size.y".into(),locked:true}).unwrap();
        history.finish(&doc);
        assert!(doc.text().contains("\"custom\": 42"));
        assert!(doc.text().contains("\"unknown\": \"stay\""));
        let loaded=Document::parse(doc.text().into()).unwrap();
        assert_eq!(loaded.value()["nodes"]["out"]["channels"]["size.y"],json!("ch('../driver/size.y')"));
        assert_eq!(loaded.value()["nodes"]["out"]["locked_params"],json!(["size.y"]));
        let graph=Graph::from_value(loaded.value()).unwrap();
        assert_eq!(graph.to_value()["nodes"]["out"]["channels"]["size.y"],json!("ch('../driver/size.y')"));
        history.undo(&mut doc);
        assert_eq!(doc.text(),original);
        history.redo(&mut doc);
        assert_eq!(doc.value()["nodes"]["out"]["locked_params"],json!(["size.y"]));
    }
}
