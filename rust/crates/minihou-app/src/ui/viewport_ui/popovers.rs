use super::*;

pub(super) fn note(p: &egui::Painter, th: &Theme, at: Pos2, (edge, fill): (Color32, Color32), (title, body): (&str, &str), max_w: f32) {
    let t = p.layout_no_wrap(title.to_owned(), egui::FontId::proportional(12.), th.text);
    let b = p.layout(body.to_owned(), egui::FontId::monospace(11.), th.dim, (max_w - 24.).max(80.));
    let w = t.size().x.max(b.size().x) + 24.;
    let r = Rect::from_min_size(at, Vec2::new(w, t.size().y + b.size().y + 16.));
    p.rect(r, 6., fill, Stroke::new(1.0_f32, edge), StrokeKind::Inside);
    p.galley(r.min + Vec2::new(12., 6.), t.clone(), th.text);
    p.galley(r.min + Vec2::new(12., 8. + t.size().y), b, th.dim);
}

pub(super) fn caret_button(ui: &mut egui::Ui, th: &Theme, whole: Rect, id: &'static str, active: bool) -> bool {
    let r = Rect::from_min_max(Pos2::new(whole.right() - 16., whole.top()), whole.max);
    let resp = ui.interact(r, egui::Id::new(id), Sense::click());
    if active { ui.painter().rect_filled(r, 4., th.accent_bg); }
    else if resp.hovered() { ui.painter().rect_filled(r, 4., th.panel2); }
    let c = r.center();
    ui.painter().add(egui::Shape::line(vec![c + Vec2::new(-3., -1.5), c + Vec2::new(0., 1.5), c + Vec2::new(3., -1.5)], Stroke::new(1.3_f32, th.dim)));
    resp.clicked()
}

#[allow(clippy::too_many_arguments)] // one compact painter call keeps the menu rows consistent
fn pop_row(ui: &mut egui::Ui, r: Rect, id: impl std::hash::Hash, label: &str, selected: bool, check: bool, key: &str, th: &Theme) -> bool {
    let resp = ui.interact(r, egui::Id::new(("vp_pop_row", id)), Sense::click());
    let p = ui.painter();
    if resp.hovered() { p.rect_filled(r, 3., th.panel2); }
    let mark = Rect::from_center_size(Pos2::new(r.left() + 13., r.center().y), Vec2::splat(14.));
    if check {
        p.rect(mark, 3., if selected { th.accent } else { th.panel }, Stroke::new(1.0_f32, if selected { th.accent } else { th.dim }), StrokeKind::Inside);
        if selected { icon(p, Icon::Frame, mark.shrink(2.), th.panel, true); }
    } else if selected { p.text(mark.center(), egui::Align2::CENTER_CENTER, "✓", egui::FontId::proportional(13.), th.accent); }
    p.text(Pos2::new(r.left() + 30., r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.), th.text);
    if !key.is_empty() { p.text(Pos2::new(r.right() - 8., r.center().y), egui::Align2::RIGHT_CENTER, key, egui::FontId::monospace(11.), th.dim); }
    resp.clicked()
}

fn numeric_row(ui: &mut egui::Ui, r: Rect, id: &'static str, label: &str, value: f32, th: &Theme, units: Option<crate::units::UnitSettings>) -> Option<f32> {
    let p = ui.painter();
    p.text(Pos2::new(r.left() + 8., r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.), th.text);
    let box_r = Rect::from_min_size(Pos2::new(r.right() - 88., r.center().y - 11.), Vec2::new(80., 22.));
    p.rect(box_r, 4., th.input, Stroke::new(1.0_f32, th.stroke), StrokeKind::Inside);
    let edit_id = egui::Id::new(("vp_number_edit", id));
    if let Some(mut text) = ui.ctx().data(|d| d.get_temp::<String>(edit_id)) {
        let resp = ui.put(box_r.shrink2(Vec2::new(3., 1.)), egui::TextEdit::singleline(&mut text).font(egui::FontId::monospace(12.)).frame(false).id(edit_id.with("text")));
        if !resp.has_focus() && !resp.lost_focus() { resp.request_focus(); }
        if resp.lost_focus() {
            ui.ctx().data_mut(|d| d.remove::<String>(edit_id));
            let parsed = if let Some(u) = units {
                u.parse(&text).map(|x| x as f32)
            } else {
                text.trim().parse::<f32>().ok()
            };
            return parsed.filter(|x| x.is_finite()).map(|x| x.max(1e-4));
        }
        ui.ctx().data_mut(|d| d.insert_temp(edit_id, text));
    } else {
        let resp = ui.interact(box_r, edit_id, Sense::click_and_drag());
        if resp.double_clicked() {
            let initial = if let Some(u) = units {
                u.format(value as f64)
            } else {
                format!("{value}")
            };
            ui.ctx().data_mut(|d| d.insert_temp(edit_id, initial));
        }
        if resp.dragged() {
            let dx = ui.ctx().input(|i| i.pointer.delta().x);
            if dx != 0. { return Some((value + dx * value.max(0.1) * 0.01).max(1e-4)); }
        }
        let display_str = if let Some(u) = units {
            u.format(value as f64)
        } else {
            format!("{value:.2}")
        };
        p.text(Pos2::new(box_r.right() - 6., box_r.center().y), egui::Align2::RIGHT_CENTER, display_str, egui::FontId::monospace(12.), th.text);
    }
    None
}

fn falloff_value(f: Falloff, x: f32) -> f32 {
    let t = (1. - x).clamp(0., 1.);
    match f {
        Falloff::Smooth => t * t * (3. - 2. * t), Falloff::Sphere => (1. - x * x).max(0.).sqrt(),
        Falloff::Root => t.sqrt(), Falloff::InverseSquare => t * t, Falloff::Sharp => t * t * t,
        Falloff::Linear => t, Falloff::Constant => if x < 1. { 1. } else { 0. },
        Falloff::Random => ((x * 37.).sin() * 0.35 + 0.55) * t,
    }
}
pub(super) fn falloff_samples(f: Falloff) -> [f32; 13] { std::array::from_fn(|i| falloff_value(f, i as f32 / 12.)) }

pub(super) fn show_popover(ui: &mut egui::Ui, viewport: Rect, anchor: Rect, which: Popover, v: &View, out: &mut Actions) -> Rect {
    let th = v.theme;
    let ed = v.editor;
    let rows = match which { Popover::Orientation => 6, Popover::Pivot => 5, Popover::Snap => 11, Popover::Proportional => 12, Popover::Overlays => 19, Popover::Shading => 19, Popover::Visibility => 3 };
    let height = 30. + rows as f32 * 24. + 10.;
    let x = (anchor.right() - 232.).clamp(viewport.left() + 6., (viewport.right() - 238.).max(viewport.left() + 6.));
    let pop = Rect::from_min_size(Pos2::new(x, anchor.bottom() + 4.), Vec2::new(232., height));
    let p = ui.painter();
    p.rect(pop, 6., th.panel, Stroke::new(1.0_f32, th.stroke), StrokeKind::Inside);
    let title = match which { Popover::Orientation => "ORIENTATION", Popover::Pivot => "PIVOT POINT", Popover::Snap => "SNAPPING", Popover::Proportional => "PROPORTIONAL EDITING", Popover::Overlays => "OVERLAYS", Popover::Shading => "DISPLAY MODE", Popover::Visibility => "OBJECT VISIBILITY" };
    p.text(pop.min + Vec2::new(12., 17.), egui::Align2::LEFT_CENTER, title, egui::FontId::proportional(11.), th.dim);
    let row = |i: usize| Rect::from_min_size(pop.min + Vec2::new(6., 28. + i as f32 * 24.), Vec2::new(220., 24.));
    match which {
        Popover::Shading => {
            for (i,mode) in Mode::ALL.into_iter().enumerate() {
                if display_row(ui,row(i),("display",i),mode.label(),v.mode==mode,Icon::Display(mode),th) { out.push(UiAction::SetMode(mode)); }
            }
            section(ui,row(12),"MATCAP",th);
            for (i,matcap) in crate::viewport::MatCap::ALL.into_iter().enumerate() {
                let r=row(13+i);
                if pop_row(ui,r,("matcap",i),matcap.label(),ed.matcap==matcap,false,"",th) {
                    out.push(UiAction::Editor(EditorChange::MatCap(matcap)));
                }
                matcap_swatch(ui,r,matcap);
            }
            if pop_row(ui,row(16),"matcap_flat","Flat normals",ed.matcap_flat,true,"",th) {
                out.push(UiAction::Editor(EditorChange::MatCapFlat(!ed.matcap_flat)));
            }
            let active=matches!(v.mode,Mode::FlatShaded|Mode::FlatWireShaded|Mode::SmoothShaded|Mode::SmoothWireShaded);
            if pop_row(ui,row(17),"materials_textures","Materials / Textures",ed.textures,true,if active {""}else{"Shaded modes only"},th) {
                out.push(UiAction::Editor(EditorChange::Textures(!ed.textures)));
            }
            if pop_row(ui,row(18),"texture_inspector","Texture inspector",ed.texture_inspector,true,"",th) {
                out.push(UiAction::Editor(EditorChange::TextureInspector(!ed.texture_inspector)));
            }
        },
        Popover::Visibility => for (i,visibility) in ObjectVisibility::ALL.into_iter().enumerate() {
            if display_row(ui,row(i),("visibility",i),visibility.label(),ed.object_visibility==visibility,Icon::Visibility(visibility),th) { out.push(UiAction::Editor(EditorChange::ObjectVisibility(visibility))); }
        },
        Popover::Orientation => for (i, o) in Orientation::ALL.iter().enumerate() {
            if pop_row(ui, row(i), ("orientation", i), o.label(), *o == ed.orientation, false, "", th) { out.push(UiAction::Editor(EditorChange::Orientation(*o))); }
        },
        Popover::Pivot => for (i, pivot) in Pivot::ALL.iter().enumerate() {
            if pop_row(ui, row(i), ("pivot", i), pivot.label(), *pivot == ed.pivot, false, "", th) { out.push(UiAction::Editor(EditorChange::Pivot(*pivot))); }
        },
        Popover::Snap => {
            if pop_row(ui, row(0), ("snap_enabled", 0), "Enable snapping", ed.snap.enabled, true, "", th) { out.push(UiAction::Editor(EditorChange::SnapEnabled(!ed.snap.enabled))); }
            section(ui, row(1), "SNAP TO", th);
            for (i, to) in SnapTo::ALL.iter().enumerate() {
                if pop_row(ui, row(2 + i), ("snap_to", i), to.label(), ed.snap.to.contains(*to), true, "", th) { out.push(UiAction::Editor(EditorChange::SnapTo(*to))); }
            }
            if let Some(x) = numeric_row(ui, row(8), "snap_increment", "Increment", ed.snap.increment, th, Some(v.units)) { out.push(UiAction::Editor(EditorChange::SnapIncrement(x))); }
            // The final two rows extend the standard spacing so both toggles remain readable.
            let extra = row(9);
            if pop_row(ui, extra, ("snap_align", 0), "Align rotation to target", ed.snap.align_rotation, true, "", th) { out.push(UiAction::Editor(EditorChange::SnapAlignRotation(!ed.snap.align_rotation))); }
            if pop_row(ui, row(10), ("snap_backface", 0), "Backface culling", ed.snap.backface_culling, true, "", th) { out.push(UiAction::Editor(EditorChange::SnapBackfaceCulling(!ed.snap.backface_culling))); }
        }
        Popover::Proportional => {
            if pop_row(ui, row(0), ("prop_enabled", 0), "Enable proportional editing", ed.proportional.enabled, true, "", th) { out.push(UiAction::Editor(EditorChange::ProportionalEnabled(!ed.proportional.enabled))); }
            section(ui, row(1), "FALLOFF", th);
            for (i, falloff) in Falloff::ALL.iter().enumerate() {
                let r = row(2 + i);
                if pop_row(ui, r, ("falloff", i), falloff.label(), *falloff == ed.proportional.falloff, false, "", th) { out.push(UiAction::Editor(EditorChange::ProportionalFalloff(*falloff))); }
                let pts = falloff_samples(*falloff).iter().enumerate().map(|(j, y)| Pos2::new(r.right() - 34. + j as f32 * 2., r.center().y + 6. - y * 12.)).collect();
                ui.painter().add(egui::Shape::line(pts, Stroke::new(1.0_f32, th.dim)));
            }
            if let Some(x) = numeric_row(ui, row(10), "prop_radius", "Radius", ed.proportional.radius, th, Some(v.units)) { out.push(UiAction::Editor(EditorChange::ProportionalRadius(x))); }
            if pop_row(ui, row(11), ("prop_connected", 0), "Connected only", ed.proportional.connected_only, true, "", th) { out.push(UiAction::Editor(EditorChange::ProportionalConnected(!ed.proportional.connected_only))); }
        }
        Popover::Overlays => {
            section(ui,row(0),"GENERAL",th);
            let mut o=ed.overlays;
            let values=[o.grid,o.axes,o.stats,o.bounds,o.normals,o.wireframe];
            for (i,label) in ["Grid","Axes / Floor","Statistics","Bounds","Normals","Wireframe overlay"].iter().enumerate() {
                if pop_row(ui,row(i+1),("general_overlay",i),label,values[i],true,"",th) {
                    match i {0=>o.grid=!o.grid,1=>o.axes=!o.axes,2=>o.stats=!o.stats,3=>o.bounds=!o.bounds,4=>o.normals=!o.normals,_=>o.wireframe=!o.wireframe}
                }
            }
            if pop_row(ui,row(7),"overlay_xray","X-ray",ed.xray,true,"Alt+Z",th) { out.push(UiAction::Editor(EditorChange::Xray(!ed.xray))); }
            section(ui,row(8),"GEOMETRY",th);
            for (i,(label,value)) in [("Points",o.points),("Lines (curves / edges)",o.lines),("Faces",o.faces)].into_iter().enumerate() {
                if pop_row(ui,row(i+9),("geometry_overlay",i),label,value,true,"",th) {
                    match i {0=>o.points=!o.points,1=>o.lines=!o.lines,_=>o.faces=!o.faces}
                }
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(row(12)),|ui| {ui.add(egui::Slider::new(&mut o.point_size,1.0..=16.0).text("Point size"));});
            ui.scope_builder(egui::UiBuilder::new().max_rect(row(13)),|ui| {ui.add(egui::Slider::new(&mut o.line_width,1.0..=10.0).text("Line width"));});
            if o!=ed.overlays { out.push(UiAction::Editor(EditorChange::Overlays(o))); }
            section(ui,row(14),"COLOR",th);
            for (i,color) in crate::viewport::ObjectColor::ALL.into_iter().enumerate() {
                if pop_row(ui,row(15+i),("object_color",i),color.label(),ed.object_color==color,false,"",th) {
                    out.push(UiAction::Editor(EditorChange::ObjectColor(color)));
                }
            }
            if pop_row(ui,row(17),"overlay_materials","Materials / Textures",ed.textures,true,"",th) {
                out.push(UiAction::Editor(EditorChange::Textures(!ed.textures)));
            }
            if pop_row(ui,row(18),"overlay_texture_inspector","Texture inspector",ed.texture_inspector,true,"",th) {
                out.push(UiAction::Editor(EditorChange::TextureInspector(!ed.texture_inspector)));
            }
        }
    }
    pop
}

fn matcap_swatch(ui:&egui::Ui, row:Rect, matcap:crate::viewport::MatCap) {
    let centre=Pos2::new(row.left()+17.,row.center().y);
    let mut mesh=egui::Mesh::default();
    let rings=4; let sectors=20; let radius=9.;
    for ring in 0..=rings { for sector in 0..sectors {
        let r=ring as f32/rings as f32;
        let angle=std::f32::consts::TAU*sector as f32/sectors as f32;
        let (x,y)=(r*angle.cos(),r*angle.sin());
        let rgb=match matcap {
            crate::viewport::MatCap::Normal => crate::viewport::display_modes::normal_color([x,-y,(1.-r*r).sqrt()]),
            crate::viewport::MatCap::SketchUpClean => { let v=(0.76+0.19*(-0.35*x-0.55*y+0.76*(1.-r*r).sqrt()).max(0.)-0.04*y).clamp(0.,1.); [v;3] },
            crate::viewport::MatCap::Clay => { let v=0.68+0.23*(1.-r*r).sqrt()-0.14*y; [v*0.99,v*0.95,v*0.9] },
        };
        mesh.vertices.push(egui::epaint::Vertex {pos:centre+Vec2::new(x*radius,y*radius),uv:egui::epaint::WHITE_UV,color:Color32::from_rgb((rgb[0]*255.) as u8,(rgb[1]*255.) as u8,(rgb[2]*255.) as u8)});
    } }
    for ring in 0..rings { for sector in 0..sectors {
        let a=(ring*sectors+sector) as u32;
        let b=(ring*sectors+(sector+1)%sectors) as u32;
        let c=((ring+1)*sectors+sector) as u32;
        let d=((ring+1)*sectors+(sector+1)%sectors) as u32;
        mesh.indices.extend([a,c,b,b,c,d]);
    } }
    ui.painter().add(egui::Shape::mesh(mesh));
}


/// Compact Blender-style status strip for the active viewport transform.
