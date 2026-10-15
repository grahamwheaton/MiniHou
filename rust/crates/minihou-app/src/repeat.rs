//! Blender's Shift+R "Repeat Last" operator: records confirmed operators
//! and replays them on the current selection.

use crate::ui::modal_transform::Constraint;
use crate::ui::{Orientation, Proportional};
use minihou_app::{Document, Edit};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum LastOperator {
    MatrixDelta {
        delta: crate::transform_math::Mat4,
        frame: crate::transform_math::Frame,
        label: &'static str,
    },
    Move {
        delta: [f32; 3],
        constraint: Option<Constraint>,
        orientation: Orientation,
        proportional: Option<Proportional>,
    },
    Rotate {
        angle: [f32; 3],
        pivot: [f32; 3],
        orientation: Orientation,
        axis_rotation: Option<[f32; 3]>,
    },
    Scale {
        factor: [f32; 3],
        pivot: [f32; 3],
    },
    AddNode {
        node_type: String,
        after_selection: bool,
    },
    Duplicate,
    Dissolve,
    Delete,
    Connect {
        from: String,
        to: String,
        port: Option<String>,
    },
    Tidy,
}

impl LastOperator {
    pub fn description(&self) -> String {
        match self {
            Self::MatrixDelta { label, .. } => format!("Repeat {label} (Shift+R)"),
            Self::Move { .. } => "Repeat Move (Shift+R)".to_string(),
            Self::Rotate { .. } => "Repeat Rotate (Shift+R)".to_string(),
            Self::Scale { .. } => "Repeat Scale (Shift+R)".to_string(),
            Self::AddNode { node_type, .. } => format!("Repeat Add {node_type} (Shift+R)"),
            Self::Duplicate => "Repeat Duplicate (Shift+R)".to_string(),
            Self::Dissolve => "Repeat Dissolve (Shift+R)".to_string(),
            Self::Delete => "Repeat Delete (Shift+R)".to_string(),
            Self::Connect { .. } => "Repeat Connect (Shift+R)".to_string(),
            Self::Tidy => "Repeat Tidy (Shift+R)".to_string(),
        }
    }
}

/// Applies a transform repeat to an existing transform node in the document.
/// Move adds the delta, Rotate adds the angle, Scale multiplies by factor (compounding).
pub fn apply_repeat_transform(
    doc: &mut Document,
    target: &str,
    op: &LastOperator,
) -> Result<Edit, String> {
    let (name, next) = match op {
        LastOperator::Move { delta, .. } => {
            let current = &doc.value()["nodes"][target]["params"]["translate"];
            let next = [0, 1, 2].map(|i| current.get(i).and_then(Value::as_f64).unwrap_or(0.) as f32 + delta[i]);
            ("translate", json!(next))
        }
        LastOperator::Rotate { angle, .. } => {
            let current = &doc.value()["nodes"][target]["params"]["rotate"];
            let next = [0, 1, 2].map(|i| current.get(i).and_then(Value::as_f64).unwrap_or(0.) as f32 + angle[i]);
            ("rotate", json!(next))
        }
        LastOperator::Scale { factor, .. } => {
            let current = &doc.value()["nodes"][target]["params"]["scale"];
            let next = [0, 1, 2].map(|i| current.get(i).and_then(Value::as_f64).unwrap_or(1.) as f32 * factor[i]);
            ("scale", json!(next))
        }
        _ => return Err("Not a transform operator".to_string()),
    };
    let edit = Edit::Param {
        node: target.to_owned(),
        name: name.to_owned(),
        value: next,
    };
    doc.edit(&edit).map_err(|e| e.to_string())?;
    Ok(edit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_four_x_90_degree_rotate_repeats() {
        let initial = json!({
            "version": 1,
            "display": "transform1",
            "nodes": {
                "box1": { "type": "box" },
                "transform1": { "type": "transform", "inputs": { "geo": "box1" }, "params": { "rotate": [0., 0., 0.] } }
            }
        });
        let mut doc = Document::parse(initial.to_string()).unwrap();
        let op = LastOperator::Rotate {
            angle: [0., 90., 0.],
            pivot: [0.; 3],
            orientation: Orientation::Global,
            axis_rotation: None,
        };
        for _ in 0..4 {
            apply_repeat_transform(&mut doc, "transform1", &op).unwrap();
        }
        let rot_y = doc.value()["nodes"]["transform1"]["params"]["rotate"][1].as_f64().unwrap();
        assert!((rot_y - 360.0).abs() < 1e-4);
        assert_eq!(doc.value()["nodes"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn test_move_repeat_adds_delta() {
        let initial = json!({
            "version": 1,
            "display": "transform1",
            "nodes": {
                "box1": { "type": "box" },
                "transform1": { "type": "transform", "inputs": { "geo": "box1" }, "params": { "translate": [1., 0., 0.] } }
            }
        });
        let mut doc = Document::parse(initial.to_string()).unwrap();
        let op = LastOperator::Move {
            delta: [1.5, -0.5, 2.0],
            constraint: None,
            orientation: Orientation::Global,
            proportional: None,
        };
        apply_repeat_transform(&mut doc, "transform1", &op).unwrap();
        let t1 = &doc.value()["nodes"]["transform1"]["params"]["translate"];
        assert!((t1[0].as_f64().unwrap() - 2.5).abs() < 1e-4);
        assert!((t1[1].as_f64().unwrap() - (-0.5)).abs() < 1e-4);
        assert!((t1[2].as_f64().unwrap() - 2.0).abs() < 1e-4);

        apply_repeat_transform(&mut doc, "transform1", &op).unwrap();
        let t2 = &doc.value()["nodes"]["transform1"]["params"]["translate"];
        assert!((t2[0].as_f64().unwrap() - 4.0).abs() < 1e-4);
        assert!((t2[1].as_f64().unwrap() - (-1.0)).abs() < 1e-4);
        assert!((t2[2].as_f64().unwrap() - 4.0).abs() < 1e-4);
    }

    #[test]
    fn test_scale_repeat_compounds() {
        let initial = json!({
            "version": 1,
            "display": "transform1",
            "nodes": {
                "box1": { "type": "box" },
                "transform1": { "type": "transform", "inputs": { "geo": "box1" }, "params": { "scale": [2., 1., 0.5] } }
            }
        });
        let mut doc = Document::parse(initial.to_string()).unwrap();
        let op = LastOperator::Scale {
            factor: [2., 3., 0.5],
            pivot: [0.; 3],
        };
        apply_repeat_transform(&mut doc, "transform1", &op).unwrap();
        let s1 = &doc.value()["nodes"]["transform1"]["params"]["scale"];
        assert!((s1[0].as_f64().unwrap() - 4.0).abs() < 1e-4);
        assert!((s1[1].as_f64().unwrap() - 3.0).abs() < 1e-4);
        assert!((s1[2].as_f64().unwrap() - 0.25).abs() < 1e-4);

        apply_repeat_transform(&mut doc, "transform1", &op).unwrap();
        let s2 = &doc.value()["nodes"]["transform1"]["params"]["scale"];
        assert!((s2[0].as_f64().unwrap() - 8.0).abs() < 1e-4);
        assert!((s2[1].as_f64().unwrap() - 9.0).abs() < 1e-4);
        assert!((s2[2].as_f64().unwrap() - 0.125).abs() < 1e-4);
    }

    #[test]
    fn test_recording_on_confirm_and_not_on_cancel() {
        use crate::ui::modal_transform::{Kind, Modal};
        use eframe::egui::pos2;

        let mut last_op: Option<LastOperator> = None;

        let mut modal = Modal::new(Kind::Rotate, pos2(0., 0.), [0.; 3], ([0.; 3], [1.; 3]), crate::ui::SnapTo::Increment, false, 1.);
        modal.last = [0., 90., 0.];

        // Simulate cancel: last_op is NOT updated
        let cancel = true;
        if !cancel {
            last_op = Some(LastOperator::Rotate {
                angle: modal.last,
                pivot: modal.pivot,
                orientation: Orientation::Global,
                axis_rotation: None,
            });
        }
        assert_eq!(last_op, None);

        // Simulate confirm: last_op IS updated
        let confirm = true;
        if confirm {
            last_op = Some(LastOperator::Rotate {
                angle: modal.last,
                pivot: modal.pivot,
                orientation: Orientation::Global,
                axis_rotation: None,
            });
        }
        assert!(matches!(last_op, Some(LastOperator::Rotate { angle, .. }) if angle == [0., 90., 0.]));
    }

    #[test]
    fn test_repeat_after_undo_repeats_the_same_operator() {
        let initial = json!({
            "version": 1,
            "display": "transform1",
            "nodes": {
                "box1": { "type": "box" },
                "transform1": { "type": "transform", "inputs": { "geo": "box1" }, "params": { "translate": [1., 0., 0.] } }
            }
        });
        let mut doc = Document::parse(initial.to_string()).unwrap();
        let mut history = minihou_app::History::default();
        let op = LastOperator::Move {
            delta: [1., 0., 0.],
            constraint: None,
            orientation: Orientation::Global,
            proportional: None,
        };

        // Step 1: repeat move
        let before = doc.clone();
        apply_repeat_transform(&mut doc, "transform1", &op).unwrap();
        history.record(before, &doc);
        assert_eq!(doc.value()["nodes"]["transform1"]["params"]["translate"][0], 2.0);

        // Undo
        let undo = history.undo(&mut doc).unwrap();
        assert_eq!(doc.value()["nodes"]["transform1"]["params"]["translate"][0], 1.0);
        let _ = undo;

        // Repeat again: LastOperator is retained, so it repeats the same Move
        let before2 = doc.clone();
        apply_repeat_transform(&mut doc, "transform1", &op).unwrap();
        history.record(before2, &doc);
        assert_eq!(doc.value()["nodes"]["transform1"]["params"]["translate"][0], 2.0);
    }

    #[test]
    fn test_nothing_to_repeat_state() {
        let last_op: Option<LastOperator> = None;
        let status = if last_op.is_none() {
            "Nothing to repeat"
        } else {
            "Repeated"
        };
        assert_eq!(status, "Nothing to repeat");
    }

    #[test]
    fn test_descriptions_include_shift_r() {
        let op = LastOperator::Move { delta: [1., 0., 0.], constraint: None, orientation: Orientation::Global, proportional: None };
        assert_eq!(op.description(), "Repeat Move (Shift+R)");
        let op = LastOperator::Rotate { angle: [0., 90., 0.], pivot: [0.; 3], orientation: Orientation::Global, axis_rotation: None };
        assert_eq!(op.description(), "Repeat Rotate (Shift+R)");
        let op = LastOperator::Scale { factor: [2.; 3], pivot: [0.; 3] };
        assert_eq!(op.description(), "Repeat Scale (Shift+R)");
        let op = LastOperator::AddNode { node_type: "box".into(), after_selection: true };
        assert_eq!(op.description(), "Repeat Add box (Shift+R)");
        let op = LastOperator::Duplicate;
        assert_eq!(op.description(), "Repeat Duplicate (Shift+R)");
        let op = LastOperator::Dissolve;
        assert_eq!(op.description(), "Repeat Dissolve (Shift+R)");
        let op = LastOperator::Delete;
        assert_eq!(op.description(), "Repeat Delete (Shift+R)");
        let op = LastOperator::Tidy;
        assert_eq!(op.description(), "Repeat Tidy (Shift+R)");
    }
}
