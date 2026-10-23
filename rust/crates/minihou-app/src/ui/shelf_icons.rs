//! Flat painter versions of the remaining drawings in the web shelf's ICONS table.
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};

fn at(o: Pos2, x: f32, y: f32) -> Pos2 { o + Vec2::new(x, y) }
fn line(p: &egui::Painter, o: Pos2, pts: &[(f32, f32)], c: Color32) {
    for pair in pts.windows(2) { p.line_segment([at(o, pair[0].0, pair[0].1), at(o, pair[1].0, pair[1].1)], Stroke::new(1.7_f32, c)); }
}
#[allow(clippy::too_many_arguments)] // coordinates mirror the compact SVG rectangles
fn rect(p: &egui::Painter, o: Pos2, x: f32, y: f32, w: f32, h: f32, c: Color32, fill: bool) {
    let r = Rect::from_min_size(at(o, x, y), Vec2::new(w, h));
    if fill { p.rect_filled(r, 0., c); } else { p.rect_stroke(r, 0., Stroke::new(1.5_f32, c), StrokeKind::Inside); }
}
fn dot(p: &egui::Painter, o: Pos2, x: f32, y: f32, r: f32, c: Color32) { p.circle_filled(at(o, x, y), r, c); }
fn ellipse(p: &egui::Painter, o: Pos2, x: f32, y: f32, rx: f32, ry: f32, c: Color32) {
    p.add(egui::Shape::ellipse_stroke(at(o, x, y), Vec2::new(rx, ry), Stroke::new(1.5_f32, c)));
}
fn polygon(p: &egui::Painter, o: Pos2, pts: &[(f32, f32)], c: Color32) {
    p.add(egui::Shape::convex_polygon(pts.iter().map(|&(x, y)| at(o, x, y)).collect(), c, Stroke::NONE));
}

/// Returns false for the web shelf's initials fallback.
pub fn draw(p: &egui::Painter, o: Pos2, kind: &str, ink: Color32, accent: Color32) -> bool {
    let a = accent;
    let c = ink;
    match kind {
        "transform" => { line(p,o,&[(12.,2.),(12.,22.)],c); line(p,o,&[(2.,12.),(22.,12.)],c);
            polygon(p,o,&[(12.,1.),(9.,6.),(15.,6.)],a); polygon(p,o,&[(22.,12.),(17.,9.),(17.,15.)],a); }
        "merge" => { line(p,o,&[(3.,5.),(9.,5.),(16.,12.),(9.,19.),(3.,19.)],c); line(p,o,&[(16.,12.),(22.,12.),(19.,9.)],a); line(p,o,&[(22.,12.),(19.,15.)],a); }
        "duplicate" => { rect(p,o,3.,3.,12.,12.,c,false); rect(p,o,9.,9.,12.,12.,a,false); }
        "copy_to_points" => { for (x,y) in [(4.,6.),(12.,4.),(20.,7.)] { dot(p,o,x,y,2.,a); rect(p,o,x-3.,y+6.,6.,6.,c,false); } }
        "instance" => { for (x,y) in [(2.,2.),(16.,2.),(9.,16.)] { rect(p,o,x,y,7.,7.,c,false); } line(p,o,&[(9.,6.),(16.,6.)],a); line(p,o,&[(6.,9.),(12.,16.),(19.,9.)],a); }
        "switch" | "switchif" => { rect(p,o,3.,9.,18.,7.,c,false); dot(p,o,17.,12.5,3.,a); }
        "controller" => { rect(p,o,3.,3.,18.,18.,c,false); for (y,x) in [(7.,10.),(12.,16.),(17.,8.)] { line(p,o,&[(6.,y),(18.,y)],c); dot(p,o,x,y,2.,a); } }
        "match_size" => { rect(p,o,2.,2.,20.,20.,a,false); rect(p,o,7.,7.,10.,10.,c,false); }
        "bound" => { line(p,o,&[(12.,2.),(21.,7.),(21.,18.),(12.,22.),(3.,18.),(3.,7.),(12.,2.),(12.,12.),(21.,7.)],a); line(p,o,&[(3.,7.),(12.,12.),(12.,22.)],a); }
        "blast" | "delete" => { if kind == "blast" { rect(p,o,4.,6.,15.,14.,c,false); } line(p,o,&[(7.,4.),(21.,20.)],a); line(p,o,&[(21.,4.),(7.,20.)],a); }
        "fuse" => { for (x,y) in [(4.,5.),(20.,5.),(4.,19.),(20.,19.)] { dot(p,o,x,y,2.,c); line(p,o,&[(x,y),(12.,12.)],a); } dot(p,o,12.,12.,3.,a); }
        "sort" => { for (y,w) in [(4.,18.),(10.,13.),(16.,8.)] { rect(p,o,3.,y,w,3.,a,true); } }
        "output" => { line(p,o,&[(3.,14.),(3.,21.),(21.,21.),(21.,14.)],c); line(p,o,&[(12.,3.),(12.,17.),(7.,12.)],a); line(p,o,&[(12.,17.),(17.,12.)],a); }
        "boolean" | "blend" => { ellipse(p,o,9.,12.,7.,8.,a); if kind == "boolean" { ellipse(p,o,15.,12.,7.,8.,c); } else { rect(p,o,12.,5.,10.,13.,c,false); } }
        "mirror" => { line(p,o,&[(12.,2.),(12.,22.)],a); polygon(p,o,&[(10.,6.),(3.,19.),(10.,19.)],c); polygon(p,o,&[(14.,6.),(21.,19.),(14.,19.)],c); }
        "subdivide" => { rect(p,o,4.,4.,16.,16.,c,false); line(p,o,&[(12.,4.),(12.,20.)],c); line(p,o,&[(4.,12.),(20.,12.)],c); p.circle_stroke(at(o,12.,12.),6.,Stroke::new(1.7_f32,a)); }
        "skin" => { ellipse(p,o,12.,5.,6.,2.,c); ellipse(p,o,12.,20.,9.,2.5,c); line(p,o,&[(6.,5.),(3.,20.)],a); line(p,o,&[(18.,5.),(21.,20.)],a); }
        "revolve" => { line(p,o,&[(12.,2.),(12.,22.)],c); ellipse(p,o,15.,12.,7.,9.,a); line(p,o,&[(3.,6.),(9.,6.)],a); }
        "sweep" => { line(p,o,&[(3.,21.),(8.,9.),(15.,18.),(21.,5.)],c); line(p,o,&[(4.,20.),(9.,11.),(15.,16.),(21.,6.)],a); }
        "fillet" | "polybevel" => { line(p,o,&[(4.,21.),(4.,10.),(10.,4.),(21.,4.)],c); line(p,o,&[(4.,10.),(10.,4.)],a); dot(p,o,10.,10.,1.5,a); }
        "polyreduce" | "facet" => { polygon(p,o,&[(12.,2.),(22.,9.),(19.,21.),(5.,21.),(2.,9.)],c); line(p,o,&[(12.,2.),(12.,21.)],a); line(p,o,&[(2.,9.),(22.,9.)],a); }
        "convex_hull" | "hull" => { line(p,o,&[(12.,2.),(22.,10.),(19.,21.),(5.,21.),(2.,10.),(12.,2.)],a); for (x,y) in [(12.,10.),(8.,16.),(16.,15.)] { dot(p,o,x,y,1.7,c); } }
        "remesh" | "terrain" => { polygon(p,o,&[(3.,20.),(12.,3.),(21.,20.)],c); line(p,o,&[(7.,13.),(17.,13.)],a); line(p,o,&[(5.,17.),(19.,17.)],a); line(p,o,&[(12.,3.),(12.,20.)],a); }
        "polyextrude" => { polygon(p,o,&[(3.,13.),(12.,18.),(21.,13.),(12.,8.)],c); line(p,o,&[(3.,13.),(3.,20.),(12.,23.),(21.,20.),(21.,13.)],c); line(p,o,&[(12.,18.),(12.,23.)],c); line(p,o,&[(12.,7.),(12.,1.),(9.,4.)],a); line(p,o,&[(12.,1.),(15.,4.)],a); }
        "polysplit" => { rect(p,o,4.,4.,16.,16.,c,false); line(p,o,&[(4.,12.),(20.,12.)],a); dot(p,o,4.,12.,2.,a); dot(p,o,20.,12.,2.,a); }
        "polyfill" => { polygon(p,o,&[(4.,6.),(16.,3.),(22.,12.),(16.,21.),(4.,17.)],a); line(p,o,&[(4.,6.),(16.,3.),(22.,12.),(16.,21.),(4.,17.),(4.,6.)],c); }
        "polybridge" => { rect(p,o,2.,5.,5.,15.,c,false); rect(p,o,17.,5.,5.,15.,c,false); rect(p,o,7.,8.,10.,9.,a,false); }
        "polyexpand2d" => { rect(p,o,2.,2.,20.,20.,a,false); rect(p,o,7.,7.,10.,10.,c,false); }
        "dissolve" => { rect(p,o,3.,3.,18.,18.,c,false); line(p,o,&[(12.,3.),(12.,21.)],a); }
        "divide" => { polygon(p,o,&[(3.,21.),(21.,21.),(12.,3.)],c); line(p,o,&[(12.,3.),(12.,21.)],a); line(p,o,&[(3.,21.),(17.,13.)],a); }
        "reverse" => { line(p,o,&[(3.,8.),(20.,8.),(16.,4.)],a); line(p,o,&[(20.,8.),(16.,12.)],a); line(p,o,&[(21.,17.),(4.,17.),(8.,13.)],a); line(p,o,&[(4.,17.),(8.,21.)],a); }
        "normal" => { rect(p,o,3.,14.,18.,7.,c,false); for x in [7.,12.,17.] { line(p,o,&[(x,14.),(x,4.)],a); } }
        "edge_collapse" => { line(p,o,&[(2.,12.),(9.,12.),(12.,12.),(15.,12.),(22.,12.)],c); line(p,o,&[(8.,7.),(12.,12.),(8.,17.)],a); line(p,o,&[(16.,7.),(12.,12.),(16.,17.)],a); }
        "edge_flip" => { rect(p,o,3.,3.,18.,18.,c,false); line(p,o,&[(3.,3.),(21.,21.)],c); line(p,o,&[(21.,3.),(3.,21.)],a); }
        "edge_cusp" => { line(p,o,&[(3.,18.),(12.,5.),(21.,18.)],c); dot(p,o,12.,5.,2.5,a); }
        "crease" => { rect(p,o,3.,6.,18.,12.,c,false); line(p,o,&[(3.,12.),(21.,12.)],a); }
        "group" => { rect(p,o,3.,3.,18.,18.,a,false); for (x,y) in [(8.,9.),(16.,9.),(12.,16.)] { dot(p,o,x,y,2.,c); } }
        "code" => { line(p,o,&[(9.,6.),(4.,12.),(9.,18.)],a); line(p,o,&[(15.,6.),(20.,12.),(15.,18.)],a); }
        "attr" => { for y in [4.,10.,16.] { rect(p,o,3.,y,5.,3.,c,true); rect(p,o,10.,y,11.,3.,a,true); } }
        "measure" => { line(p,o,&[(3.,17.),(17.,3.),(21.,7.),(7.,21.),(3.,17.)],c); for i in 0..3 { let q=i as f32*4.; line(p,o,&[(8.+q,14.-q),(10.+q,16.-q)],a); } }
        "connectivity" => { for (x,y,col) in [(6.,7.,a),(18.,7.,c),(12.,19.,c)] { dot(p,o,x,y,3.,col); } line(p,o,&[(6.,7.),(18.,7.),(12.,19.),(6.,7.)],a); }
        "ray" => { line(p,o,&[(3.,4.),(18.,19.)],a); line(p,o,&[(8.,21.),(21.,21.)],c); dot(p,o,18.,19.,2.,c); }
        "resample" => { line(p,o,&[(3.,18.),(7.,9.),(12.,5.),(17.,9.),(21.,18.)],c); for (x,y) in [(3.,18.),(7.,9.),(12.,5.),(17.,9.),(21.,18.)] { dot(p,o,x,y,1.8,a); } }
        "carve" => { line(p,o,&[(3.,20.),(7.,8.),(12.,4.),(17.,8.),(21.,20.)],c); line(p,o,&[(7.,11.),(12.,6.),(17.,11.)],a); }
        "scatter" => { for (x,y) in [(4.,5.),(10.,3.),(19.,6.),(7.,13.),(16.,14.),(4.,20.),(12.,21.),(21.,20.)] { dot(p,o,x,y,1.7,a); } }
        "noise" => { line(p,o,&[(2.,17.),(6.,9.),(10.,14.),(14.,4.),(18.,13.),(22.,7.)],a); line(p,o,&[(2.,21.),(22.,21.)],c); }
        "bend" => { line(p,o,&[(4.,21.),(4.,11.),(8.,5.),(20.,4.)],c); line(p,o,&[(5.,19.),(6.,10.),(10.,6.),(20.,5.)],a); }
        "twist" => { line(p,o,&[(8.,3.),(16.,9.),(8.,15.),(16.,21.)],c); line(p,o,&[(16.,3.),(8.,9.),(16.,15.),(8.,21.)],a); }
        "taper" => { polygon(p,o,&[(4.,21.),(20.,21.),(16.,4.),(8.,4.)],c); line(p,o,&[(8.,4.),(16.,4.)],a); }
        "squash" => { rect(p,o,5.,9.,14.,7.,c,true); line(p,o,&[(12.,2.),(12.,7.),(9.,5.)],a); line(p,o,&[(12.,7.),(15.,5.)],a); line(p,o,&[(12.,22.),(12.,17.),(9.,19.)],a); line(p,o,&[(12.,17.),(15.,19.)],a); }
        "peak" => { line(p,o,&[(2.,19.),(8.,19.),(12.,5.),(16.,19.),(22.,19.)],c); dot(p,o,12.,5.,2.,a); }
        "smooth" => { line(p,o,&[(2.,17.),(6.,5.),(11.,17.),(16.,5.),(22.,17.)],c); line(p,o,&[(2.,15.),(6.,10.),(11.,17.),(16.,8.),(22.,15.)],a); }
        "fracture" => { p.circle_stroke(at(o,12.,12.),9.,Stroke::new(1.5_f32,c)); line(p,o,&[(12.,3.),(10.,10.),(15.,13.),(9.,17.),(12.,21.)],a); line(p,o,&[(10.,10.),(4.,11.)],a); line(p,o,&[(15.,13.),(20.,10.)],a); }
        "solver" => { p.circle_stroke(at(o,12.,12.),9.,Stroke::new(1.5_f32,c)); polygon(p,o,&[(10.,7.),(18.,12.),(10.,17.)],a); }
        "loop" => { line(p,o,&[(6.,8.),(18.,8.),(21.,12.),(18.,17.),(6.,17.)],a); line(p,o,&[(9.,5.),(5.,8.),(9.,11.)],a); line(p,o,&[(15.,14.),(19.,17.),(15.,20.)],a); }
        "loop_circle" => { ellipse(p,o,12.,12.,9.,8.,c); for (x,y) in [(12.,4.),(21.,12.),(12.,20.),(3.,12.)] { dot(p,o,x,y,1.8,a); } }
        "loop_space" => { line(p,o,&[(3.,7.),(8.,5.),(14.,6.),(21.,8.)],c); line(p,o,&[(3.,17.),(9.,17.),(15.,17.),(21.,17.)],a); for x in [3.,9.,15.,21.] { dot(p,o,x,17.,1.8,a); } }
        "loop_flatten" => { line(p,o,&[(3.,8.),(8.,5.),(12.,9.),(17.,6.),(21.,10.)],c); line(p,o,&[(3.,17.),(21.,17.)],a); for x in [3.,9.,15.,21.] { dot(p,o,x,17.,1.8,a); } }
        "loop_relax" => { line(p,o,&[(3.,15.),(7.,6.),(12.,17.),(17.,6.),(21.,15.)],c); line(p,o,&[(3.,15.),(7.,10.),(12.,13.),(17.,10.),(21.,15.)],a); for x in [3.,12.,21.] { dot(p,o,x,15.,1.6,a); } }
        "loop_curve" => { line(p,o,&[(3.,18.),(7.,10.),(12.,6.),(17.,10.),(21.,18.)],c); line(p,o,&[(3.,17.),(8.,14.),(12.,8.),(16.,14.),(21.,17.)],a); dot(p,o,12.,8.,1.8,a); }
        "pack" => { rect(p,o,3.,3.,18.,18.,c,false); for (x,y) in [(7.,7.),(14.,7.),(7.,14.),(14.,14.)] { rect(p,o,x,y,3.,3.,a,true); } }
        "unpack" => { rect(p,o,6.,6.,12.,12.,c,false); for (x,y) in [(2.,2.),(19.,2.),(2.,19.),(19.,19.)] { rect(p,o,x,y,3.,3.,a,true); } }
        "extract_centroid" => { polygon(p,o,&[(3.,20.),(12.,4.),(21.,20.)],c); dot(p,o,12.,14.,3.,a); }
        "split" => { polygon(p,o,&[(3.,5.),(11.,5.),(11.,19.),(3.,19.)],c); polygon(p,o,&[(14.,5.),(22.,5.),(22.,19.),(14.,19.)],a); }
        "vertex_bevel" => { line(p,o,&[(3.,20.),(9.,6.),(15.,6.),(21.,20.)],c); line(p,o,&[(9.,6.),(15.,6.)],a); dot(p,o,9.,6.,1.7,a); dot(p,o,15.,6.,1.7,a); }
        "clean" => { line(p,o,&[(3.,18.),(8.,7.),(17.,7.),(21.,18.),(3.,18.)],c); line(p,o,&[(8.,7.),(12.,12.),(17.,7.)],a); dot(p,o,12.,12.,2.,a); }
        "dissolve_flat_edges" => { rect(p,o,3.,5.,18.,14.,c,false); line(p,o,&[(12.,5.),(12.,19.)],a); line(p,o,&[(9.,12.),(15.,12.)],a); }
        "boundary_select" => { line(p,o,&[(3.,19.),(3.,5.),(21.,5.),(21.,19.),(3.,19.)],a); for (x,y) in [(3.,5.),(21.,5.),(21.,19.),(3.,19.)] { dot(p,o,x,y,2.,c); } }
        "select_sharp_edges" => { line(p,o,&[(3.,19.),(12.,5.),(21.,19.)],c); line(p,o,&[(8.,13.),(12.,5.),(16.,13.)],a); dot(p,o,12.,5.,2.,a); }
        "example_offset" => { rect(p,o,3.,8.,10.,10.,c,false); rect(p,o,10.,4.,10.,10.,a,false); line(p,o,&[(5.,21.),(17.,21.),(14.,18.)],a); }
        "cache" => { rect(p,o,4.,3.,16.,18.,c,false); line(p,o,&[(7.,7.),(17.,7.)],a); line(p,o,&[(7.,11.),(17.,11.)],a); dot(p,o,12.,17.,2.,a); }
        "bulge" => { line(p,o,&[(3.,17.),(7.,14.),(12.,5.),(17.,14.),(21.,17.)],c); line(p,o,&[(3.,20.),(21.,20.)],a); }
        "fractal" => { line(p,o,&[(2.,20.),(7.,11.),(12.,20.),(17.,11.),(22.,20.)],c); line(p,o,&[(7.,11.),(9.,5.),(12.,11.),(15.,5.),(17.,11.)],a); }
        "waves" | "ripple" => { for y in [7.,12.,17.] { line(p,o,&[(2.,y+2.),(7.,y-2.),(12.,y+2.),(17.,y-2.),(22.,y+2.)],if y == 12. { a } else { c }); } }
        "point_jitter" => { line(p,o,&[(3.,12.),(21.,12.)],c); for (x,y) in [(4.,9.),(9.,15.),(15.,8.),(20.,16.)] { dot(p,o,x,y,2.,a); } }
        "point_relax" => { for (x,y) in [(5.,7.),(19.,7.),(5.,18.),(19.,18.),(12.,12.)] { dot(p,o,x,y,2.,a); } line(p,o,&[(5.,7.),(12.,12.),(19.,7.)],c); line(p,o,&[(5.,18.),(12.,12.),(19.,18.)],c); }
        "group_expand" => { rect(p,o,7.,7.,10.,10.,c,false); line(p,o,&[(7.,7.),(3.,3.)],a); line(p,o,&[(17.,7.),(21.,3.)],a); line(p,o,&[(7.,17.),(3.,21.)],a); line(p,o,&[(17.,17.),(21.,21.)],a); }
        "group_promote" => { rect(p,o,3.,13.,18.,8.,c,false); line(p,o,&[(12.,18.),(12.,4.),(8.,8.)],a); line(p,o,&[(12.,4.),(16.,8.)],a); }
        "flood_fill" => { rect(p,o,3.,3.,18.,18.,c,false); for (x,y) in [(7.,7.),(12.,7.),(17.,7.),(7.,12.),(12.,12.),(17.,12.),(7.,17.),(12.,17.),(17.,17.)] { dot(p,o,x,y,1.6,if x+y <= 24. { a } else { c }); } }
        "attribute_create" => { rect(p,o,3.,5.,18.,14.,c,false); line(p,o,&[(12.,8.),(12.,16.)],a); line(p,o,&[(8.,12.),(16.,12.)],a); }
        "attribute_delete" => { rect(p,o,3.,5.,18.,14.,c,false); line(p,o,&[(8.,12.),(16.,12.)],a); }
        "attribute_rename" => { rect(p,o,3.,5.,18.,14.,c,false); line(p,o,&[(7.,15.),(17.,15.)],a); dot(p,o,8.,10.,1.5,a); dot(p,o,16.,10.,1.5,a); }
        "attribute_promote" => { for y in [7.,15.] { rect(p,o,3.,y,7.,3.,c,true); } line(p,o,&[(12.,17.),(19.,10.),(15.,10.)],a); line(p,o,&[(19.,10.),(19.,14.)],a); }
        "attribute_randomize" => { for (x,y) in [(4.,5.),(15.,4.),(8.,12.),(19.,13.),(4.,19.),(14.,20.)] { rect(p,o,x,y,3.,3.,a,true); } }
        "attribute_transfer" => { rect(p,o,2.,5.,7.,14.,c,false); rect(p,o,15.,5.,7.,14.,c,false); line(p,o,&[(9.,12.),(15.,12.),(12.,9.)],a); line(p,o,&[(15.,12.),(12.,15.)],a); }
        "attribute_blur" => { dot(p,o,6.,12.,3.,c); dot(p,o,12.,12.,2.5,a); dot(p,o,18.,12.,2.,a); line(p,o,&[(3.,18.),(21.,18.)],c); }
        "ambient_occlusion" => { line(p,o,&[(3.,19.),(21.,19.)],c); dot(p,o,12.,17.,2.,a); for x in [5.,9.,15.,19.] { line(p,o,&[(12.,16.),(x,5.)],a); } }
        "set_material" => { rect(p,o,3.,4.,18.,16.,c,false); polygon(p,o,&[(3.,20.),(12.,7.),(21.,20.)],a); }
        "image_density" => { rect(p,o,3.,3.,18.,18.,c,false); for (x,y) in [(7.,8.),(13.,6.),(17.,11.),(8.,17.),(14.,16.)] { dot(p,o,x,y,1.8,a); } }
        "measure_area" => { polygon(p,o,&[(4.,19.),(10.,4.),(20.,9.),(19.,19.)],c); line(p,o,&[(4.,19.),(20.,9.)],a); }
        "measure_bounds" => { rect(p,o,5.,6.,14.,12.,c,false); line(p,o,&[(3.,3.),(7.,3.)],a); line(p,o,&[(3.,3.),(3.,7.)],a); line(p,o,&[(21.,21.),(17.,21.)],a); line(p,o,&[(21.,21.),(21.,17.)],a); }
        "measure_distance" => { dot(p,o,4.,18.,2.,c); dot(p,o,20.,6.,2.,c); line(p,o,&[(6.,16.),(18.,8.)],a); line(p,o,&[(7.,13.),(9.,16.)],a); line(p,o,&[(15.,8.),(17.,11.)],a); }
        "measure_length" => { line(p,o,&[(3.,17.),(8.,7.),(15.,10.),(21.,4.)],c); for (x,y) in [(3.,17.),(8.,7.),(15.,10.),(21.,4.)] { dot(p,o,x,y,1.7,a); } }
        "measure_volume" => { line(p,o,&[(12.,3.),(20.,7.),(20.,17.),(12.,21.),(4.,17.),(4.,7.),(12.,3.)],c); line(p,o,&[(4.,7.),(12.,11.),(20.,7.)],a); line(p,o,&[(12.,11.),(12.,21.)],a); }
        "subnet" => { rect(p,o,3.,4.,18.,16.,c,false); for (x,y) in [(7.,9.),(17.,9.),(12.,16.)] { dot(p,o,x,y,2.,a); } line(p,o,&[(7.,9.),(12.,16.),(17.,9.)],c); }
        "pcg_points" => { for (x,y) in [(5.,5.),(17.,5.),(11.,11.),(5.,19.),(18.,18.)] { dot(p,o,x,y,2.,a); } }
        "pcg_bounds" => { rect(p,o,3.,3.,18.,18.,c,false); for (x,y) in [(7.,8.),(16.,9.),(11.,16.)] { dot(p,o,x,y,1.8,a); } }
        "pcg_copy_points" => { for (x,y) in [(4.,5.),(14.,5.),(4.,15.),(14.,15.)] { rect(p,o,x,y,6.,6.,c,false); dot(p,o,x+3.,y+3.,1.7,a); } }
        "pcg_spawner" => { dot(p,o,12.,12.,2.5,c); for (x,y) in [(12.,3.),(21.,12.),(12.,21.),(3.,12.)] { line(p,o,&[(12.,12.),(x,y)],c); dot(p,o,x,y,2.,a); } }
        "pcg_transform" => { for (x,y) in [(5.,7.),(9.,17.),(18.,13.)] { dot(p,o,x,y,2.,c); } line(p,o,&[(5.,7.),(12.,7.),(18.,13.)],a); line(p,o,&[(18.,13.),(14.,12.)],a); }
        "pcg_distance" => { dot(p,o,5.,17.,2.5,a); dot(p,o,19.,7.,2.5,a); line(p,o,&[(7.,15.),(17.,9.)],c); line(p,o,&[(10.,11.),(12.,14.)],c); }
        "pcg_projection" => { dot(p,o,12.,4.,2.,a); line(p,o,&[(12.,6.),(12.,17.)],a); line(p,o,&[(9.,14.),(12.,17.),(15.,14.)],a); line(p,o,&[(3.,20.),(21.,20.)],c); }
        "pcg_surface_sampler" => { line(p,o,&[(2.,17.),(7.,7.),(13.,13.),(21.,5.)],c); for (x,y) in [(4.,13.),(9.,9.),(16.,10.)] { dot(p,o,x,y,2.,a); } }
        "pcg_spline_sampler" => { line(p,o,&[(3.,18.),(8.,6.),(15.,18.),(21.,6.)],c); for (x,y) in [(3.,18.),(8.,6.),(15.,18.),(21.,6.)] { dot(p,o,x,y,1.8,a); } }
        "pcg_volume_sampler" => { line(p,o,&[(12.,3.),(21.,8.),(21.,18.),(12.,22.),(3.,18.),(3.,8.),(12.,3.)],c); for (x,y) in [(9.,9.),(16.,10.),(12.,16.)] { dot(p,o,x,y,1.7,a); } }
        "pcg_density_noise" => { rect(p,o,3.,3.,18.,18.,c,false); for (x,y,r) in [(7.,8.,1.),(14.,6.,2.),(17.,14.,1.5),(8.,17.,2.5)] { dot(p,o,x,y,r,a); } }
        "pcg_density_filter" => { line(p,o,&[(3.,4.),(21.,4.),(15.,12.),(15.,20.),(9.,20.),(9.,12.),(3.,4.)],c); dot(p,o,10.,8.,2.,a); dot(p,o,14.,16.,2.,a); }
        "pcg_self_prune" => { for (x,y) in [(5.,6.),(9.,11.),(15.,8.),(18.,17.),(6.,19.)] { dot(p,o,x,y,2.,c); } line(p,o,&[(7.,9.),(11.,13.)],a); line(p,o,&[(11.,9.),(7.,13.)],a); }
        "pcg_union" => { ellipse(p,o,9.,12.,6.,8.,c); ellipse(p,o,15.,12.,6.,8.,a); dot(p,o,12.,12.,2.,a); }
        "pcg_intersection" => { ellipse(p,o,9.,12.,6.,8.,c); ellipse(p,o,15.,12.,6.,8.,c); line(p,o,&[(12.,5.),(15.,12.),(12.,19.),(9.,12.),(12.,5.)],a); }
        "pcg_difference" => { ellipse(p,o,9.,12.,6.,8.,c); ellipse(p,o,15.,12.,6.,8.,a); line(p,o,&[(4.,12.),(10.,12.)],a); }
        "pcg_debug" => { rect(p,o,4.,3.,16.,18.,c,false); dot(p,o,12.,8.,2.,a); line(p,o,&[(12.,12.),(12.,17.)],a); }
        "constraint" => { ellipse(p,o,6.,12.,5.,3.,c); ellipse(p,o,18.,12.,5.,3.,a); line(p,o,&[(9.,12.),(15.,12.)],c); }
        "uv" => { for (x,y,col) in [(3.,3.,c),(12.,3.,a),(3.,12.,a),(12.,12.,c)] { rect(p,o,x,y,9.,9.,col,false); } }
        "color" => { for (x,y,col) in [(3.,5.,a),(9.,5.,c),(15.,5.,a)] { rect(p,o,x,y,6.,15.,col,false); } }
        _ => return false,
    }
    true
}
