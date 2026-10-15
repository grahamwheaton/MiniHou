//! Design tokens. The values are the `--var`s of the design canvas (project/Main.dc.html, `.ws-dark` / `.ws-light`, and the
//! viewport's `.vp-dark` / `.vp-light`); keep the names the same so the design and the code can be compared one to one.
use eframe::egui::{self, Color32};

const fn hex(v: u32) -> Color32 { Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8) }

#[derive(Clone, Debug)]
pub struct Theme {
    pub dark: bool,
    pub base: Color32,     // network canvas background
    pub grid: Color32,     // network canvas grid lines
    pub panel: Color32,
    pub panel2: Color32,   // panel headers, group bars, status bar
    pub input: Color32,    // value boxes, slider tracks
    pub stroke: Color32,
    pub stroke2: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_bg: Color32,
    pub card: Color32,     // node card fill
    pub sel: Color32,      // selection outline
    pub wire: Color32,
    pub c_src: Color32,    // node category colours
    pub c_mod: Color32,
    pub c_attr: Color32,
    pub c_comb: Color32,
    pub c_out: Color32,
    pub ok: Color32,
    pub ok_bg: Color32,
    pub warn: Color32,
    pub warn_bg: Color32,
    pub err: Color32,
    pub err_bg: Color32,
    pub tpl: Color32,      // Template flag
    pub x_axis: Color32,
    pub y_axis: Color32,
    pub z_axis: Color32,
    // viewport (the `.vp-*` set)
    pub vp_bg: Color32,
    pub vp_floor: Color32,
    pub vp_wire: Color32,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            dark: true, base: hex(0x1b1d21), grid: hex(0x212429), panel: hex(0x23262b), panel2: hex(0x2b2f35), input: hex(0x181a1e),
            stroke: hex(0x3a3f47), stroke2: hex(0x555c67), text: hex(0xd8dce2), dim: hex(0x9aa1ab), faint: hex(0x6f7681),
            accent: hex(0x4f9cf0), accent_bg: hex(0x243a57), card: hex(0x2e323a), sel: hex(0xf2f4f7), wire: hex(0x6b727e),
            c_src: hex(0x3fb59a), c_mod: hex(0x8b7cf0), c_attr: hex(0x9bb84a), c_comb: hex(0xd4688a), c_out: hex(0x8693a3),
            ok: hex(0x4cb782), ok_bg: hex(0x1d3a2e), warn: hex(0xe8a23a), warn_bg: hex(0x3a2f1d), err: hex(0xef5b52), err_bg: hex(0x3b2224),
            tpl: hex(0x5bb8c9), x_axis: hex(0xc4584f), y_axis: hex(0x6cb56c), z_axis: hex(0x4f86d0),
            vp_bg: hex(0x2b2e34), vp_floor: hex(0x3b3f47), vp_wire: hex(0xc4c9d1),
        }
    }
    pub fn light() -> Self {
        Self {
            dark: false, base: hex(0xe4e7eb), grid: hex(0xd8dce1), panel: hex(0xf4f5f7), panel2: hex(0xe9ebee), input: hex(0xffffff),
            stroke: hex(0xc4c9d1), stroke2: hex(0xa1a9b4), text: hex(0x1f2329), dim: hex(0x566070), faint: hex(0x8a919c),
            accent: hex(0x2a74d6), accent_bg: hex(0xdbe8fb), card: hex(0xffffff), sel: hex(0x1f2329), wire: hex(0x8a929e),
            c_src: hex(0x1f8f77), c_mod: hex(0x6a58d6), c_attr: hex(0x6f8a1f), c_comb: hex(0xbf3f6a), c_out: hex(0x5f6b7a),
            ok: hex(0x1f8a55), ok_bg: hex(0xd9f0e4), warn: hex(0xa86a05), warn_bg: hex(0xfbefd6), err: hex(0xc8362d), err_bg: hex(0xfbe3e1),
            tpl: hex(0x2a8ea1), x_axis: hex(0xc4584f), y_axis: hex(0x3f9a45), z_axis: hex(0x3d76c4),
            vp_bg: hex(0xb3b8c0), vp_floor: hex(0xc6cbd2), vp_wire: hex(0x2a2e34),
        }
    }

    /// Category colour of a node type, as the design's cards use it (source / modifier / attribute / combine / output).
    pub fn category_color(&self, node_type: &str) -> Color32 {
        match node_type {
            "grid" | "box" | "sphere" | "tube" | "torus" | "line" | "circle" | "null" | "point" | "curve" | "bezier" | "helix" | "lsystem" => self.c_src,
            "normal" | "attribute_create" | "attribute_transfer" | "attribute_promote" | "attribute_randomize" | "measure" | "color" => self.c_attr,
            "merge" | "boolean" | "copy_to_points" | "switch" => self.c_comb,
            "output" => self.c_out,
            _ => self.c_mod,
        }
    }

    /// Install the colours into egui so stock widgets (sliders, combo boxes, scroll bars) match the design.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut v = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.override_text_color = Some(self.text);
        v.panel_fill = self.panel;
        v.window_fill = self.panel2;
        v.window_stroke = egui::Stroke::new(1.0_f32, self.stroke2);
        v.extreme_bg_color = self.input;
        v.faint_bg_color = self.panel2;
        v.selection.bg_fill = self.accent_bg;
        v.selection.stroke = egui::Stroke::new(1.0_f32, self.accent);
        v.hyperlink_color = self.accent;
        v.warn_fg_color = self.warn;
        v.error_fg_color = self.err;
        let stroke = egui::Stroke::new(1.0_f32, self.stroke);
        v.widgets.noninteractive.bg_fill = self.panel;
        v.widgets.noninteractive.bg_stroke = stroke;
        v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, self.text);
        v.widgets.inactive.bg_fill = self.input;
        v.widgets.inactive.weak_bg_fill = self.panel2;
        v.widgets.inactive.bg_stroke = stroke;
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, self.text);
        v.widgets.hovered.bg_fill = self.input;
        v.widgets.hovered.weak_bg_fill = self.panel2;
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, self.stroke2);
        v.widgets.active.bg_fill = self.accent_bg;
        v.widgets.active.weak_bg_fill = self.accent_bg;
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, self.accent);
        v.widgets.open.bg_fill = self.accent_bg;
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = egui::CornerRadius::same(4);
        }
        ctx.set_visuals(v);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(6., 4.);
            s.spacing.button_padding = egui::vec2(8., 3.);
            s.spacing.interact_size.y = 24.;
        });
    }
}
