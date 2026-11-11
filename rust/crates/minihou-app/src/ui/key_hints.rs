//! Blender-style key hints for the status bar: what the keyboard and mouse do right now.
//! The app publishes the current context once a frame; the status bar reads it back.
use super::modal_transform::Kind;
use eframe::egui;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Context { #[default] Idle, Transform(Kind), Modelling }

/// Keys shown as caps, then what they do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hint { pub keys: &'static [&'static str], pub label: &'static str }
const fn h(keys: &'static [&'static str], label: &'static str) -> Hint { Hint { keys, label } }

pub fn hints(context: Context) -> Vec<Hint> {
    match context {
        Context::Idle => vec![
            h(&["G"], "Move"), h(&["R"], "Rotate"), h(&["S"], "Scale"),
            h(&["1", "2", "3", "4"], "Points / Edges / Faces / Objects"), h(&["Home"], "Frame"),
        ],
        Context::Modelling => vec![h(&["Enter"], "Confirm"), h(&["Esc"], "Cancel")],
        Context::Transform(kind) => {
            let mut hints = vec![
                h(&["Enter"], "Confirm"), h(&["Esc"], "Cancel"),
                h(&["X", "Y", "Z"], "Axis"), h(&["Shift", "X", "Y", "Z"], "Plane"),
                h(&["0-9"], "Type value"), h(&["B"], "Set Snap Base"), h(&["Ctrl"], "Snap Invert"),
                h(&["O"], "Proportional"), h(&["Shift"], "Precision Mode"),
            ];
            match kind {
                Kind::Rotate => hints.push(h(&["R"], "Trackball")),
                Kind::Scale => hints.push(h(&["V"], "Cage")),
                Kind::Move => {}
            }
            hints
        }
    }
}

fn id() -> egui::Id { egui::Id::new("status_key_hints") }
pub fn set(ctx: &egui::Context, context: Context) { ctx.data_mut(|d| d.insert_temp(id(), context)); }
pub fn current(ctx: &egui::Context) -> Context { ctx.data(|d| d.get_temp(id())).unwrap_or_default() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_move_lists_confirm_cancel_and_axis_keys() {
        let list = hints(Context::Transform(Kind::Move));
        assert_eq!(list[0].label, "Confirm");
        assert!(list.iter().any(|h| h.label == "Axis" && h.keys == ["X", "Y", "Z"]));
        assert!(!list.iter().any(|h| h.label == "Trackball"));
    }
    #[test]
    fn rotate_and_scale_add_their_own_key() {
        assert!(hints(Context::Transform(Kind::Rotate)).iter().any(|h| h.label == "Trackball"));
        assert!(hints(Context::Transform(Kind::Scale)).iter().any(|h| h.label == "Cage"));
    }
    #[test]
    fn the_context_round_trips_through_the_egui_context() {
        let ctx = egui::Context::default();
        assert_eq!(current(&ctx), Context::Idle);
        set(&ctx, Context::Transform(Kind::Scale));
        assert_eq!(current(&ctx), Context::Transform(Kind::Scale));
    }
}
