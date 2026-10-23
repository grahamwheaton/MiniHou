//! Geometry summary and Houdini-style Spreadsheet beneath Parameters.
#[path = "../geo_summary.rs"]
pub mod geo_summary;

use super::{theme::Theme, Actions, AttrInfo, GeoSummary, View};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use minihou_app::geo_info::{self, ElementDomain};

const HEIGHT: f32 = 220.;
const HEADER: f32 = 26.;
const ROW: f32 = 18.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GeoTab {
    Info,
    Spreadsheet,
}

fn line(ui: &mut egui::Ui, height: f32) -> Rect {
    ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover()).0
}

fn text(ui: &egui::Ui, x: f32, y: f32, value: &str, color: egui::Color32) {
    ui.painter().text(Pos2::new(x, y), Align2::LEFT_CENTER, value, FontId::monospace(11.), color);
}

fn fmt(value: f64) -> String { format!("{value:.3}") }

fn vec3(v: [f32; 3], units: crate::units::UnitSettings) -> String {
    format!("{}, {}, {}", crate::units::format(v[0] as f64, units), crate::units::format(v[1] as f64, units), crate::units::format(v[2] as f64, units))
}

fn stats(ui: &mut egui::Ui, s: &GeoSummary, theme: &Theme, units: crate::units::UnitSettings) {
    let r = line(ui, ROW + 4.);
    text(ui, r.left() + 10., r.center().y, "POINTS", theme.dim);
    text(ui, r.left() + 68., r.center().y, &s.points.to_string(), theme.text);
    text(ui, r.left() + 105., r.center().y, "VERTS", theme.dim);
    text(ui, r.left() + 155., r.center().y, &s.vertices.to_string(), theme.text);
    text(ui, r.left() + 197., r.center().y, "PRIMS", theme.dim);
    text(ui, r.left() + 245., r.center().y, &s.prims.to_string(), theme.text);
    if let Some((min, max)) = s.bounds {
        for (label, value) in [("Min", min), ("Max", max), ("Size", [max[0] - min[0], max[1] - min[1], max[2] - min[2]])] {
            let r = line(ui, ROW);
            text(ui, r.left() + 10., r.center().y, label, theme.dim);
            text(ui, r.left() + 48., r.center().y, &vec3(value, units), theme.text);
        }
    }
}

fn attribute_table(ui: &mut egui::Ui, label: &str, attrs: &[AttrInfo], ranges: bool, theme: &Theme) {
    let r = line(ui, 20.);
    ui.painter().rect_filled(r, 0., theme.panel2);
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, theme.stroke));
    text(ui, r.left() + 10., r.center().y, &format!("{label} ({})", attrs.len()), theme.text);
    if attrs.is_empty() { return; }
    let r = line(ui, ROW);
    for (x, heading) in [(10., "NAME"), (112., "TYPE"), (196., "SIZE")] {
        text(ui, r.left() + x, r.center().y, heading, theme.faint);
    }
    if ranges {
        text(ui, r.left() + 240., r.center().y, "MIN", theme.faint);
        text(ui, r.left() + 320., r.center().y, "MAX", theme.faint);
    }
    for a in attrs {
        let r = line(ui, ROW);
        ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, theme.stroke));
        text(ui, r.left() + 10., r.center().y, &a.name, theme.text);
        text(ui, r.left() + 112., r.center().y, &a.kind, theme.dim);
        text(ui, r.left() + 196., r.center().y, &a.components.to_string(), theme.dim);
        if ranges {
            text(ui, r.left() + 240., r.center().y, &a.min.map_or_else(|| "—".into(), fmt), theme.dim);
            text(ui, r.left() + 320., r.center().y, &a.max.map_or_else(|| "—".into(), fmt), theme.dim);
        }
    }
}

fn spreadsheet_ui(ui: &mut egui::Ui, v: &View<'_>, id: egui::Id) {
    let Some(g) = v.cooking.geometry.as_deref() else {
        let r = line(ui, ROW);
        text(ui, r.left() + 10., r.center().y, "Nothing cooked yet", v.theme.dim);
        return;
    };

    let mut domain = ui.ctx().data_mut(|d| d.get_persisted::<ElementDomain>(id.with("domain")).unwrap_or(ElementDomain::Points));
    let mut page = ui.ctx().data_mut(|d| d.get_persisted::<usize>(id.with("page")).unwrap_or(0));
    let mut page_size = ui.ctx().data_mut(|d| d.get_persisted::<usize>(id.with("page_size")).unwrap_or(500));
    let mut selected_cell = ui.ctx().data_mut(|d| d.get_temp::<(usize, usize)>(id.with("selected_cell")));
    let mut selected_row = ui.ctx().data_mut(|d| d.get_temp::<usize>(id.with("selected_row")));

    // 1. Toolbar (Domain selector, paging controls, copy buttons)
    let bar_rect = line(ui, 24.);
    ui.painter().rect_filled(bar_rect, 0., v.theme.panel2);
    ui.painter().line_segment([bar_rect.left_bottom(), bar_rect.right_bottom()], Stroke::new(1.0_f32, v.theme.stroke));

    let mut cur_x = bar_rect.left() + 6.;
    // Domain selector chips
    for d in ElementDomain::ALL {
        let count = d.row_count(g);
        let label = format!("{} ({count})", d.label());
        let chip_w = (label.len() as f32 * 6.5 + 14.).max(54.);
        let chip_rect = Rect::from_min_size(Pos2::new(cur_x, bar_rect.top() + 2.), Vec2::new(chip_w, 20.));
        let active = d == domain;
        let resp = ui.interact(chip_rect, id.with(("dom_chip", d)), Sense::click());
        if resp.clicked() && !active {
            domain = d;
            page = 0;
            selected_cell = None;
            selected_row = None;
            ui.ctx().data_mut(|dt| {
                dt.insert_persisted(id.with("domain"), domain);
                dt.insert_persisted(id.with("page"), 0usize);
                dt.remove_temp::<(usize, usize)>(id.with("selected_cell"));
                dt.remove_temp::<usize>(id.with("selected_row"));
            });
        }
        let (fill, stroke_col, text_col) = if active {
            (v.theme.accent_bg, v.theme.accent, v.theme.accent)
        } else if resp.hovered() {
            (v.theme.panel, v.theme.stroke2, v.theme.text)
        } else {
            (v.theme.panel, v.theme.stroke, v.theme.dim)
        };
        ui.painter().rect(chip_rect, 3., fill, Stroke::new(1.0_f32, stroke_col), StrokeKind::Inside);
        ui.painter().text(chip_rect.center(), Align2::CENTER_CENTER, &label, FontId::monospace(10.), text_col);
        cur_x += chip_w + 4.;
    }

    let total_rows = domain.row_count(g);
    let total_pgs = geo_info::total_pages(total_rows, page_size);
    if page >= total_pgs {
        page = total_pgs.saturating_sub(1);
    }
    let page_range = geo_info::page_row_range(total_rows, page, page_size);
    let cols = geo_info::flatten_columns(g, domain);

    cur_x += 8.;
    // Paging controls
    if total_rows > 0 {
        let prev_rect = Rect::from_min_size(Pos2::new(cur_x, bar_rect.top() + 2.), Vec2::new(18., 20.));
        let prev_click = ui.interact(prev_rect, id.with("prev_pg"), Sense::click());
        ui.painter().rect(prev_rect, 2., v.theme.panel, Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
        ui.painter().text(prev_rect.center(), Align2::CENTER_CENTER, "◀", FontId::monospace(9.), if page > 0 { v.theme.text } else { v.theme.dim });
        if prev_click.clicked() && page > 0 {
            page -= 1;
            ui.ctx().data_mut(|dt| dt.insert_persisted(id.with("page"), page));
        }
        cur_x += 22.;

        let page_text = format!("{}/{}", page + 1, total_pgs);
        let pw = (page_text.len() as f32 * 6.5 + 8.).max(32.);
        let page_lbl_rect = Rect::from_min_size(Pos2::new(cur_x, bar_rect.top() + 2.), Vec2::new(pw, 20.));
        ui.painter().text(page_lbl_rect.center(), Align2::CENTER_CENTER, &page_text, FontId::monospace(10.), v.theme.dim);
        cur_x += pw + 4.;

        let next_rect = Rect::from_min_size(Pos2::new(cur_x, bar_rect.top() + 2.), Vec2::new(18., 20.));
        let next_click = ui.interact(next_rect, id.with("next_pg"), Sense::click());
        ui.painter().rect(next_rect, 2., v.theme.panel, Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
        ui.painter().text(next_rect.center(), Align2::CENTER_CENTER, "▶", FontId::monospace(9.), if page + 1 < total_pgs { v.theme.text } else { v.theme.dim });
        if next_click.clicked() && page + 1 < total_pgs {
            page += 1;
            ui.ctx().data_mut(|dt| dt.insert_persisted(id.with("page"), page));
        }
        cur_x += 26.;

        for sz in [500usize, 1000, 0] {
            let sz_label = match sz {
                500 => "500",
                1000 => "1k",
                _ => "All",
            };
            let sz_rect = Rect::from_min_size(Pos2::new(cur_x, bar_rect.top() + 3.), Vec2::new(26., 18.));
            let sz_resp = ui.interact(sz_rect, id.with(("page_sz", sz)), Sense::click());
            if sz_resp.clicked() && page_size != sz {
                page_size = sz;
                page = 0;
                ui.ctx().data_mut(|dt| {
                    dt.insert_persisted(id.with("page_size"), page_size);
                    dt.insert_persisted(id.with("page"), 0usize);
                });
            }
            let is_sz = page_size == sz;
            ui.painter().rect(sz_rect, 2., if is_sz { v.theme.accent_bg } else { v.theme.panel }, Stroke::new(1.0_f32, if is_sz { v.theme.accent } else { v.theme.stroke }), StrokeKind::Inside);
            ui.painter().text(sz_rect.center(), Align2::CENTER_CENTER, sz_label, FontId::monospace(9.), if is_sz { v.theme.accent } else { v.theme.dim });
            cur_x += 30.;
        }
    }

    // Copy Row & Copy Cell buttons on right
    let mut right_x = bar_rect.right() - 6.;
    let btn_copy_row = Rect::from_min_size(Pos2::new(right_x - 64., bar_rect.top() + 2.), Vec2::new(64., 20.));
    let resp_copy_row = ui.interact(btn_copy_row, id.with("btn_copy_row"), Sense::click());
    ui.painter().rect(btn_copy_row, 3., v.theme.panel, Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
    ui.painter().text(btn_copy_row.center(), Align2::CENTER_CENTER, "Copy Row", FontId::proportional(10.), v.theme.text);
    if resp_copy_row.clicked() {
        let r = selected_row.or_else(|| selected_cell.map(|c| c.0)).unwrap_or(page_range.start);
        if r < total_rows {
            let tsv = geo_info::format_row_tsv(g, domain, r, &cols);
            ui.ctx().copy_text(tsv);
        }
    }
    right_x -= 70.;

    let btn_copy_cell = Rect::from_min_size(Pos2::new(right_x - 64., bar_rect.top() + 2.), Vec2::new(64., 20.));
    let resp_copy_cell = ui.interact(btn_copy_cell, id.with("btn_copy_cell"), Sense::click());
    ui.painter().rect(btn_copy_cell, 3., v.theme.panel, Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
    ui.painter().text(btn_copy_cell.center(), Align2::CENTER_CENTER, "Copy Cell", FontId::proportional(10.), v.theme.text);
    if resp_copy_cell.clicked() {
        if let Some((r, c)) = selected_cell {
            if r < total_rows && c < cols.len() {
                let val = geo_info::format_cell(g, domain, r, &cols[c]);
                ui.ctx().copy_text(val);
            }
        }
    }

    // Keyboard shortcut Ctrl+C / Cmd+C
    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::C)) {
        if let Some((r, c)) = selected_cell {
            if r < total_rows && c < cols.len() {
                let val = geo_info::format_cell(g, domain, r, &cols[c]);
                ui.ctx().copy_text(val);
            }
        } else if let Some(r) = selected_row {
            if r < total_rows {
                let tsv = geo_info::format_row_tsv(g, domain, r, &cols);
                ui.ctx().copy_text(tsv);
            }
        }
    }

    if total_rows == 0 || cols.is_empty() {
        let r = line(ui, ROW);
        text(ui, r.left() + 10., r.center().y, "0 rows", v.theme.dim);
        return;
    }

    // 2. Table geometry
    let col_w = 84.0_f32;
    let idx_w = 54.0_f32;
    let total_table_width = idx_w + cols.len() as f32 * col_w;
    let table_w = total_table_width.max(ui.available_width());

    let header_height = 28.0_f32;
    let row_height = 20.0_f32;

    // Outer horizontal scroll area keeps sticky header synced horizontally with table body
    egui::ScrollArea::horizontal()
        .id_salt(id.with(("sheet_scroll_h", domain)))
        .show(ui, |ui| {
            ui.set_min_width(table_w);

            // Sticky header
            let (head_rect, _) = ui.allocate_exact_size(Vec2::new(table_w, header_height), Sense::hover());
            ui.painter().rect_filled(head_rect, 0., v.theme.panel2);
            ui.painter().line_segment([head_rect.left_bottom(), head_rect.right_bottom()], Stroke::new(1.0_f32, v.theme.stroke));

            // Column 0: Index header
            let idx_head = Rect::from_min_size(head_rect.left_top(), Vec2::new(idx_w, header_height));
            ui.painter().text(idx_head.center(), Align2::CENTER_CENTER, "#", FontId::monospace(11.), v.theme.dim);
            ui.painter().line_segment([idx_head.right_top(), idx_head.right_bottom()], Stroke::new(1.0_f32, v.theme.stroke));

            // Attribute column headers
            for (k, col) in cols.iter().enumerate() {
                let ch_x = head_rect.left() + idx_w + k as f32 * col_w;
                let ch_rect = Rect::from_min_size(Pos2::new(ch_x, head_rect.top()), Vec2::new(col_w, header_height));
                ui.painter().text(Pos2::new(ch_rect.left() + 6., ch_rect.top() + 7.), Align2::LEFT_TOP, &col.name, FontId::monospace(11.), v.theme.text);
                ui.painter().text(Pos2::new(ch_rect.left() + 6., ch_rect.top() + 18.), Align2::LEFT_TOP, col.dtype, FontId::monospace(9.), v.theme.faint);
                ui.painter().line_segment([ch_rect.right_top(), ch_rect.right_bottom()], Stroke::new(1.0_f32, v.theme.stroke));
            }

            // Virtualized row scrolling (only visible rows are formatted!)
            let avail_h = ui.available_height().max(HEIGHT - HEADER - 24. - header_height);
            egui::ScrollArea::vertical()
                .id_salt(id.with(("sheet_scroll_v", domain)))
                .max_height(avail_h)
                .show_rows(ui, row_height, page_range.len(), |ui, row_range| {
                    ui.set_min_width(table_w);
                    for local_r in row_range {
                        let r = page_range.start + local_r;
                        let (row_rect, row_resp) = ui.allocate_exact_size(Vec2::new(table_w, row_height), Sense::click());
                        let is_row_selected = selected_row == Some(r) || selected_cell.map_or(false, |c| c.0 == r);

                        // Row background
                        let fill = if is_row_selected {
                            v.theme.accent.gamma_multiply(0.2)
                        } else if r % 2 == 1 {
                            v.theme.panel2
                        } else {
                            v.theme.panel
                        };
                        ui.painter().rect_filled(row_rect, 0., fill);
                        ui.painter().line_segment([row_rect.left_bottom(), row_rect.right_bottom()], Stroke::new(0.5_f32, v.theme.stroke));

                        if row_resp.clicked() {
                            selected_row = Some(r);
                            selected_cell = None;
                        }

                        // Column 0: Row index
                        let idx_cell = Rect::from_min_size(row_rect.left_top(), Vec2::new(idx_w, row_height));
                        ui.painter().text(Pos2::new(idx_cell.right() - 8., idx_cell.center().y), Align2::RIGHT_CENTER, &r.to_string(), FontId::monospace(10.), v.theme.dim);
                        ui.painter().line_segment([idx_cell.right_top(), idx_cell.right_bottom()], Stroke::new(0.5_f32, v.theme.stroke));

                        // Attribute cells
                        for (k, col) in cols.iter().enumerate() {
                            let cell_x = row_rect.left() + idx_w + k as f32 * col_w;
                            let cell_rect = Rect::from_min_size(Pos2::new(cell_x, row_rect.top()), Vec2::new(col_w, row_height));
                            let is_cell_selected = selected_cell == Some((r, k));

                            if is_cell_selected {
                                ui.painter().rect(cell_rect, 0., Color32::TRANSPARENT, Stroke::new(1.2_f32, v.theme.accent), StrokeKind::Inside);
                            }

                            let cell_resp = ui.interact(cell_rect, id.with(("cell", r, k)), Sense::click());
                            if cell_resp.clicked() {
                                selected_cell = Some((r, k));
                                selected_row = Some(r);
                            }

                            let val = geo_info::format_cell(g, domain, r, col);
                            let tsv_row = if cell_resp.hovered() { Some(r) } else { None };
                            cell_resp.context_menu(|ui| {
                                if ui.button(format!("Copy Cell: {val}")).clicked() {
                                    ui.ctx().copy_text(val.clone());
                                    ui.close();
                                }
                                if ui.button("Copy Row (TSV)").clicked() {
                                    if let Some(row_idx) = tsv_row {
                                        let tsv = geo_info::format_row_tsv(g, domain, row_idx, &cols);
                                        ui.ctx().copy_text(tsv);
                                    }
                                    ui.close();
                                }
                            });

                            ui.painter().text(Pos2::new(cell_rect.left() + 6., cell_rect.center().y), Align2::LEFT_CENTER, &val, FontId::monospace(11.), v.theme.text);
                            ui.painter().line_segment([cell_rect.right_top(), cell_rect.right_bottom()], Stroke::new(0.5_f32, v.theme.stroke));
                        }
                    }
                });
        });

    ui.ctx().data_mut(|dt| {
        dt.insert_persisted(id.with("domain"), domain);
        dt.insert_persisted(id.with("page"), page);
        dt.insert_persisted(id.with("page_size"), page_size);
        if let Some(c) = selected_cell {
            dt.insert_temp(id.with("selected_cell"), c);
        } else {
            dt.remove_temp::<(usize, usize)>(id.with("selected_cell"));
        }
        if let Some(r) = selected_row {
            dt.insert_temp(id.with("selected_row"), r);
        } else {
            dt.remove_temp::<usize>(id.with("selected_row"));
        }
    });
}

pub fn show(ui: &mut egui::Ui, v: &View<'_>, _out: &mut Actions) {
    let id = ui.id().with("geo_info");
    let mut open = ui.ctx().data_mut(|d| d.get_persisted::<bool>(id.with("open")).unwrap_or(true));
    let mut tab = ui.ctx().data_mut(|d| d.get_persisted::<GeoTab>(id.with("tab")).unwrap_or(GeoTab::Info));
    let mut ranges = ui.ctx().data_mut(|d| d.get_persisted::<bool>(id.with("ranges")).unwrap_or(false));
    let width = ui.available_width();
    let (header, response) = ui.allocate_exact_size(Vec2::new(width, HEADER), Sense::click());
    ui.painter().rect(header, 0., v.theme.panel2, Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);

    // Chevron collapse toggle
    let c = Pos2::new(header.left() + 14., header.center().y);
    let path = if open { [c + Vec2::new(-4., -2.), c + Vec2::new(0., 2.), c + Vec2::new(4., -2.)] }
        else { [c + Vec2::new(-2., -4.), c + Vec2::new(2., 0.), c + Vec2::new(-2., 4.)] };
    ui.painter().line_segment([path[0], path[1]], Stroke::new(1.4_f32, v.theme.dim));
    ui.painter().line_segment([path[1], path[2]], Stroke::new(1.4_f32, v.theme.dim));

    // Chevron click zone
    let chevron_rect = Rect::from_min_size(header.left_top(), Vec2::new(24., HEADER));
    if ui.interact(chevron_rect, id.with("chevron"), Sense::click()).clicked() {
        open = !open;
        ui.ctx().data_mut(|d| d.insert_persisted(id.with("open"), open));
    }

    // Tabs: Info / Spreadsheet
    let mut tab_x = header.left() + 28.;
    for (label, t_val, t_w) in [("Info", GeoTab::Info, 36.0_f32), ("Spreadsheet", GeoTab::Spreadsheet, 84.0_f32)] {
        let tab_rect = Rect::from_min_size(Pos2::new(tab_x, header.top()), Vec2::new(t_w, HEADER));
        let active = tab == t_val;
        let tab_click = ui.interact(tab_rect, id.with(("geo_tab", label)), Sense::click());
        if tab_click.clicked() && !active {
            tab = t_val;
            ui.ctx().data_mut(|d| d.insert_persisted(id.with("tab"), tab));
        }
        ui.painter().text(tab_rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.), if active { v.theme.text } else { v.theme.dim });
        if active {
            ui.painter().line_segment([Pos2::new(tab_rect.left(), header.bottom() - 1.), Pos2::new(tab_rect.right(), header.bottom() - 1.)], Stroke::new(2.0_f32, v.theme.accent));
        }
        tab_x += t_w + 10.;
    }

    // Displayed node chip on far right
    if let Some(s) = v.geo {
        let displayed = s.node == v.display;
        let chip = Rect::from_min_size(Pos2::new(header.right() - 78., header.top() + 4.), Vec2::new(68., 18.));
        let (ink, fill) = if displayed { (v.theme.accent, v.theme.accent_bg) } else { (v.theme.dim, v.theme.panel) };
        ui.painter().rect(chip, 3., fill, Stroke::new(1.0_f32, ink), StrokeKind::Inside);
        ui.painter().text(chip.center(), Align2::CENTER_CENTER, if displayed { "displayed" } else { "other" },
            FontId::monospace(10.), ink);
    }

    if response.clicked() && !ui.rect_contains_pointer(chevron_rect) {
        // Clicking blank header area outside tabs also toggles if needed
    }
    if !open { return; }
    tab_body(ui, v, id, tab, &mut ranges, width, HEIGHT - HEADER);
}

/// Dock id under which the docked Geometry Info and Spreadsheet panels keep their state.
pub fn dock_id() -> egui::Id { egui::Id::new("geo_info_dock") }

/// One docked tab (Geometry Info or Spreadsheet) filling the given ui; the dock draws the tab header.
pub fn show_tab(ui: &mut egui::Ui, v: &View<'_>, tab: GeoTab) {
    let id = dock_id();
    let mut ranges = ui.ctx().data_mut(|d| d.get_persisted::<bool>(id.with("ranges")).unwrap_or(false));
    let (width, height) = (ui.available_width(), ui.available_height().max(HEADER));
    tab_body(ui, v, id, tab, &mut ranges, width, height);
}

fn tab_body(ui: &mut egui::Ui, v: &View<'_>, id: egui::Id, tab: GeoTab, ranges: &mut bool, width: f32, height: f32) {
    ui.allocate_ui_with_layout(Vec2::new(width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
        match tab {
            GeoTab::Info => {
                let r = line(ui, 22.);
                let box_rect = Rect::from_min_size(Pos2::new(r.left() + 10., r.top() + 4.), Vec2::splat(14.));
                let check = ui.interact(box_rect, id.with("ranges_check"), Sense::click());
                ui.painter().rect(box_rect, 3., if *ranges { v.theme.accent } else { v.theme.input },
                    Stroke::new(1.0_f32, if *ranges { v.theme.accent } else { v.theme.stroke2 }), StrokeKind::Inside);
                if *ranges {
                    let c = box_rect.center();
                    let s = Stroke::new(1.4_f32, v.theme.panel);
                    ui.painter().line_segment([c + Vec2::new(-4., 0.), c + Vec2::new(-1., 3.)], s);
                    ui.painter().line_segment([c + Vec2::new(-1., 3.), c + Vec2::new(4., -3.)], s);
                }
                text(ui, r.left() + 30., r.center().y, "ranges", v.theme.dim);
                if check.clicked() {
                    *ranges = !*ranges;
                    ui.ctx().data_mut(|d| d.insert_persisted(id.with("ranges"), *ranges));
                }
                let Some(s) = v.geo else {
                    let r = line(ui, ROW);
                    text(ui, r.left() + 10., r.center().y, "Nothing cooked yet", v.theme.dim);
                    return;
                };
                egui::ScrollArea::both().id_salt("geo_info_scroll").max_height(height - 22.).show(ui, |ui| {
                    ui.set_min_width(if *ranges { 400. } else { width });
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    stats(ui, s, v.theme, v.units);
                    attribute_table(ui, "Point", &s.point_attrs, *ranges, v.theme);
                    attribute_table(ui, "Vertex", &s.vertex_attrs, *ranges, v.theme);
                    attribute_table(ui, "Primitive", &s.prim_attrs, *ranges, v.theme);
                    attribute_table(ui, "Detail", &s.detail_attrs, *ranges, v.theme);
                });
            }
            GeoTab::Spreadsheet => {
                spreadsheet_ui(ui, v, id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::Fixture;
    use minihou_core::Geometry;
    use std::sync::Arc;

    #[test]
    fn empty_and_populated_frames_are_read_only() {
        let mut f = Fixture::new();
        assert!(f.run(show).is_empty());
        f.geo = Some(GeoSummary { node: f.display.clone(), points: 3, vertices: 3, prims: 1,
            bounds: Some(([0.; 3], [1.; 3])), point_attrs: vec![AttrInfo {
                name: "P".into(), kind: "float32".into(), components: 3, min: Some(0.), max: Some(1.)
            }], ..Default::default() });
        assert!(f.run(show).is_empty());
        f.theme = Theme::light();
        assert!(f.run(show).is_empty());
    }

    #[test]
    fn bounds_display_with_units() {
        let mut f = Fixture::new();
        f.geo = Some(GeoSummary {
            node: f.display.clone(), points: 3, vertices: 3, prims: 1,
            bounds: Some(([0.; 3], [1.; 3])), ..Default::default()
        });
        let mut out = Actions::default();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))), ..Default::default() };
        let full = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &f.view(), &mut out);
            });
        });
        fn extract_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
            for s in shapes {
                match &s.shape {
                    egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                    egui::epaint::Shape::Vec(subs) => {
                        let clipped: Vec<egui::epaint::ClippedShape> = subs.iter().map(|sub| egui::epaint::ClippedShape { clip_rect: s.clip_rect, shape: sub.clone() }).collect();
                        extract_texts(&clipped, out);
                    }
                    _ => {}
                }
            }
        }
        let mut texts = Vec::new();
        extract_texts(&full.shapes, &mut texts);
        assert!(texts.iter().any(|t| t.contains("1000 mm")), "expected 1000 mm in bounds: {texts:?}");
    }

    #[test]
    fn spreadsheet_tab_renders_columns_and_rows() {
        let mut f = Fixture::new();
        let g = Geometry::from_arrays(
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            vec![0, 1],
            vec![0, 2],
            None,
        ).unwrap();
        f.cooking.geometry = Some(Arc::new(g));

        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        // Persist Spreadsheet tab
        // The panel keys its state on the enclosing ui id, so set the tab inside the frame.

        let mut out = Actions::default();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))), ..Default::default() };
        let full = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.ctx().data_mut(|d| d.insert_persisted(ui.id().with("geo_info").with("tab"), GeoTab::Spreadsheet));
                show(ui, &f.view(), &mut out);
            });
        });

        fn extract_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
            for s in shapes {
                match &s.shape {
                    egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                    egui::epaint::Shape::Vec(subs) => {
                        let clipped: Vec<egui::epaint::ClippedShape> = subs.iter().map(|sub| egui::epaint::ClippedShape { clip_rect: s.clip_rect, shape: sub.clone() }).collect();
                        extract_texts(&clipped, out);
                    }
                    _ => {}
                }
            }
        }
        let mut texts = Vec::new();
        extract_texts(&full.shapes, &mut texts);

        assert!(texts.iter().any(|t| t == "Spreadsheet"), "Spreadsheet tab header missing: {texts:?}");
        for label in ["Points (2)", "Edges (1)", "Faces (1)", "Objects (1)"] {
            assert!(texts.iter().any(|t| t == label), "{label} tab missing: {texts:?}");
        }
        assert!(texts.iter().any(|t| t == "P.x"), "P.x column missing: {texts:?}");
        assert!(texts.iter().any(|t| t == "P.y"), "P.y column missing: {texts:?}");
        assert!(texts.iter().any(|t| t == "P.z"), "P.z column missing: {texts:?}");
        assert!(texts.iter().any(|t| t == "1.000"), "Row 0 P.x value missing: {texts:?}");
    }
}
