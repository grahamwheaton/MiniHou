use super::*;
use super::display_render_tests::{adapter,render,changed};

#[test]
fn grid_spacing_transition_is_continuous_at_far_zoom() {
    let Some((device,queue))=adapter() else { eprintln!("No adapter: skipping grid render test"); return; };
    let scene=Scene::new(&Geometry::points_only(Vec::new()).unwrap());
    let gpu=Gpu::new(&device,&scene);
    let overlay=Overlay {axes:false,geometry:crate::ui::Overlays {points:false,lines:false,faces:false,..Default::default()},..Default::default()};
    let image_at=|distance| {
        let mut camera=Camera::frame([0.;3],1.);
        camera.set_dist(distance);
        camera.set_clip_planes(0.01,10000.);
        super::display_render_tests::render_selection_from_camera(&device,&queue,&gpu,&camera,Mode::SmoothShaded,overlay,false,crate::ui::SelectionMode::Object)
    };
    let before=image_at(99.99);
    let after=image_at(100.01);
    let change:usize=before.iter().zip(&after).map(|(a,b)|a.iter().zip(b).map(|(x,y)|x.abs_diff(*y) as usize).sum::<usize>()).sum();
    assert!(change<3000,"grid popped at decade boundary: total channel change {change}");
}

#[test]
fn point_mode_draws_box_inner_corner_where_three_faces_meet() {
    let Some((device,queue))=adapter() else { eprintln!("No adapter: skipping box point render test"); return; };
    let p=vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.,
        0.,0.,1., 1.,0.,1., 1.,1.,1., 0.,1.,1.];
    let v=vec![0,3,2,1, 4,5,6,7, 0,1,5,4, 2,3,7,6, 0,4,7,3, 1,2,6,5];
    let box_geo=Geometry::from_arrays(p,v,vec![0,4,8,12,16,20,24],Some(vec![true;6])).unwrap();
    let scene=Scene::new(&box_geo);
    let mut gpu=Gpu::new(&device,&scene);
    gpu.select_points(&device,&scene,&[6]);
    let mut overlay=Overlay::default();
    overlay.geometry.points=false;
    overlay.geometry.point_size=0.5;
    for dx in [0.,2.,4.,6.] { for dy in [0.,2.,4.,6.] {
        let mut view=Camera::frame([1.;3],1.);
        view.orbit_degrees(79.38,-63.9);
        view.set_dist(1.2);
        view.pan(dx,dy);
        let image=super::display_render_tests::render_selection_from_camera(&device,&queue,&gpu,&view,Mode::SmoothShaded,overlay,false,crate::ui::SelectionMode::Points);
        let mut inner=Vec::new();
        for y in 58..=70 { for x in 58..=70 { inner.push(image[y*128+x]); } }
        assert!(inner.iter().any(|p|p[0]>245 && (150..=170).contains(&p[1]) && p[2]<60),"inner corner point was hidden at {dx},{dy}");
        if dx==0. && dy==0. {
            let (projection,_)=view.matrices(1.);
            let (ray_origin,ray_forward)=view.ray(0.,0.,1.);
            let (_,right)=view.ray(1.,0.,1.);
            let (_,up)=view.ray(0.,1.,1.);
            let camera=crate::ui::CameraInfo {projection,ray_origin,ray_forward,
                ray_right:[0,1,2].map(|i|right[i]-ray_forward[i]),
                ray_up:[0,1,2].map(|i|up[i]-ray_forward[i]),..Default::default()};
            let hit=crate::element_pick_glue::pick_displayed_wire(&scene,camera,
                eframe::egui::Rect::from_min_size(eframe::egui::Pos2::ZERO,eframe::egui::vec2(128.,128.)),
                eframe::egui::pos2(66.,66.),ObjectVisibility::ShowAll,&[],&Default::default(),crate::ui::SelectionMode::Points,false);
            let picked=match hit {Some(crate::picking::WireHit::Point(id))=>Some(id),_=>None};
            assert_eq!(picked,Some(6),"box inner corner could not be picked next to its three faces");
        }
    } }
}

#[test]
fn element_indices_points_lines_mixed_and_proxy() {
    let p=Geometry::points_only(vec![0.;9]).unwrap();
    let s=Scene::new(&p); assert_eq!(s.point,[0,1,2]); assert!(s.line.is_empty() && s.tri.is_empty());
    let g=Geometry::from_arrays(vec![0.;18],vec![0,1,2,3,4],vec![0,3,5],Some(vec![true,false])).unwrap();
    let s=Scene::new(&g); assert_eq!(s.point,[3,4,5]); assert_eq!(s.line,[3,4]);
    assert!(s.edge.chunks_exact(2).any(|e|e==[3,4]),"core edges include open primitives");
    let (tri, edge)=s.proxy_indices(0.); assert!(tri.is_empty() && edge.is_empty());
    assert_eq!(s.line,[3,4]); assert_eq!(s.point,[3,4,5]);
    // Explicitly closed curve, distinguished from a polygon by source metadata.
    let mut g=Geometry::from_arrays(vec![0.;9],vec![0,1,2],vec![0,3],None).unwrap();
    g.prim.insert("__viewport_curve".into(),std::sync::Arc::new(minihou_core::geometry::Attr::i32(1,vec![1])));
    assert_eq!(Scene::new(&g).line,[0,1,1,2,2,0]);
}

#[test]
fn points_and_polylines_visible_headless() {
    let Some((device,queue))=adapter() else {eprintln!("No adapter: skipping geometry render test");return;};
    let p=vec![-1.,0.,0., 0.,0.,1., 1.,0.,0.];
    let geos=[Geometry::points_only(p.clone()).unwrap(),Geometry::from_arrays(p,vec![0,1,2],vec![0,3],Some(vec![false])).unwrap()];
    for (kind,g) in geos.iter().enumerate() {
        let scene=Scene::new(g); let mut gpu=Gpu::new(&device,&scene);
        let mut options=Overlay::default(); options.geometry.points=kind==0;
        let image=render(&device,&queue,&gpu,Mode::SmoothShaded,options,false);
        let background=image[0]; let visible=image.iter().filter(|p|**p!=background).count();
        assert!(visible>8,"geometry {kind} produced no visible pixels ({visible})");
        let mut off=options; off.geometry.points=false; off.geometry.lines=false;
        let empty=render(&device,&queue,&gpu,Mode::SmoothShaded,off,false);
        assert!(empty.iter().all(|p|*p==empty[0]));
        let mut larger=options; larger.geometry.point_size=10.; larger.geometry.line_width=6.;
        let large=render(&device,&queue,&gpu,Mode::SmoothShaded,larger,false);
        assert!(large.iter().filter(|p|**p!=background).count()>visible);
        let mut master=options; master.geometry.enabled=false;
        assert_eq!(changed(&image,&render(&device,&queue,&gpu,Mode::SmoothShaded,master,false)),0,"intrinsic geometry must survive overlay master");
        gpu.select_objects(&device,&scene,vec![0..1]);
        let hide=Overlay {visibility:ObjectVisibility::HideOthers,..options};
        let ghost=Overlay {visibility:ObjectVisibility::GhostOthers,..options};
        let hidden=render(&device,&queue,&gpu,Mode::SmoothShaded,hide,false);
        assert!(changed(&image,&hidden)>0); assert!(changed(&hidden,&render(&device,&queue,&gpu,Mode::SmoothShaded,ghost,false))>0);
        gpu.upload_lod(&device,&scene);
        gpu.select_lod(&device,&ClusterLod::default(),&[],false);
        assert_eq!(changed(&image,&render(&device,&queue,&gpu,Mode::SmoothShaded,options,false)),0,"LOD discarded standalone elements");
    }
}


#[test]
fn points_depth_selection_and_geometry_switches_headless() {
    use super::display_render_tests::render_selection;
    let Some((device,queue))=adapter() else {eprintln!("No adapter: skipping depth render test");return;};
    // A large plane perpendicular to the default camera. Extra elements sit behind it.
    let cam=Camera::frame([0.;3],2.5); let (vp,_)=cam.matrices(1.);
    let right=[vp[0][0],vp[1][0],vp[2][0]];
    let up=[vp[0][1],vp[1][1],vp[2][1]];
    let normalize=|v:[f32;3]| {let len=v.iter().map(|x|x*x).sum::<f32>().sqrt();v.map(|x|x/len)};
    let (right,up)=(normalize(right),normalize(up));
    let mut p=Vec::new();
    for (x,y) in [(-2.,-2.),(2.,-2.),(2.,2.),(-2.,2.)] {
        p.extend((0..3).map(|i|right[i]*x+up[i]*y));
    }
    let forward=cam.forward();
    for x in [-0.5,0.,0.5] {p.extend((0..3).map(|i|forward[i]+right[i]*x));}
    let g=Geometry::from_arrays(p,vec![0,1,2,3,4,6],vec![0,4,6],Some(vec![true,false])).unwrap();
    let scene=Scene::new(&g); let mut gpu=Gpu::new(&device,&scene);
    let mut options=Overlay::default(); options.geometry.enabled=false;
    let shaded=render(&device,&queue,&gpu,Mode::SmoothShaded,options,false);
    let mut no_elements=options; no_elements.geometry.points=false;no_elements.geometry.lines=false;
    assert_eq!(changed(&shaded,&render(&device,&queue,&gpu,Mode::SmoothShaded,no_elements,false)),0,"elements behind faces leaked through");
    let mut no_faces=options; no_faces.geometry.faces=false;
    let revealed=render(&device,&queue,&gpu,Mode::SmoothShaded,no_faces,false);
    assert!(changed(&shaded,&revealed)>100);
    assert!(revealed.iter().filter(|p|**p!=revealed[0]).count()>10,"faces off did not reveal points and curves");
    gpu.select_objects(&device,&scene,vec![0..7]);
    let outlined=render(&device,&queue,&gpu,Mode::SmoothShaded,no_faces,true);
    assert!(outlined.iter().any(super::display_render_tests::object_outline),"faces off lost silhouette");
    gpu.select_points(&device,&scene,&[5]);
    let selected=render_selection(&device,&queue,&gpu,Mode::SmoothShaded,no_faces,false,crate::ui::SelectionMode::Points);
    assert!(selected.iter().any(|p|p[0]==255 && (155..=165).contains(&p[1]) && (35..=45).contains(&p[2])),"selected points must be orange");
    let mut light=no_faces;light.light_background=true;
    let image=render(&device,&queue,&gpu,Mode::SmoothShaded,light,false);
    assert!(image.iter().any(|p|p[0]<15 && p[1]<15 && p[2]<15),"light theme needs dark elements");
    let floor=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay{grid:true,axes:true,..no_elements},false);
    assert!(changed(&shaded,&floor)>0,"floor/axis switches must draw pixels");
}


#[test]
fn merged_source_points_and_closed_curves_keep_indices() {
    let doc=minihou_app::Document::parse(serde_json::json!({"nodes":{
        "box":{"type":"box"}, "point":{"type":"point"},
        "curve":{"type":"curve","params":{"closed":true,"points":"0,0,0 1,0,0 1,1,0"}},
        "circle":{"type":"circle","params":{"divisions":8}},
        "bezier":{"type":"bezier","params":{"closed":true}},
        "merged":{"type":"merge","inputs":{"geos":["box","point","curve","circle","bezier"]}}
    },"display":"merged"}).to_string()).unwrap();
    let mut session=doc.session().unwrap();
    let g=session.cook(Some("merged")).unwrap(); let scene=Scene::new(&g);
    assert_eq!(scene.point,[8],"standalone point offset must survive merged display");
    let line=|a,b|scene.line.chunks_exact(2).any(|pair|pair==[a,b]);
    assert!(line(9,10) && line(10,11) && line(11,9),"closed sketch curve lost its closing segment");
    assert!(line(12,13) && line(19,12),"Circle must remain a line primitive after merge");
    assert!(scene.line.len()>22,"sampled Bezier segments are missing");
    assert!(scene.tri.len()>=36,"mesh surfaces must remain intact");
}
