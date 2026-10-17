use super::*;
#[test] fn purple_group_preview_renders_large_membership_and_clears() {
    let Some((device,queue))=display_render_tests::adapter() else {eprintln!("No GPU adapter");return;};
    let doc=serde_json::json!({"nodes":{"grid":{"type":"grid","params":{"rows":160,"columns":160}},"group":{"type":"group_create","inputs":{"geo":"grid"},"params":{"name":"flat","domain":"primitives","method":"pattern","pattern":"*"}}},"display":"group"});
    let g=minihou_core::Graph::from_value(&doc).unwrap().cook(Some("group")).unwrap();
    let scene=Scene::new(&g);let mut gpu=Gpu::new(&device,&scene);let mut state=crate::group_preview::State::default();
    let options=Overlay {geometry:crate::ui::Overlays {points:false,lines:false,..Default::default()},..Default::default()};
    let baseline=display_render_tests::render(&device,&queue,&gpu,Mode::Solid,options,false);
    state.update(&mut gpu,&device,Some(&g),&doc,"group",1);
    assert_eq!(gpu.group_face_n as usize,scene.triangle_indices().len());
    let purple=display_render_tests::render(&device,&queue,&gpu,Mode::Solid,options,false);
    assert!(display_render_tests::changed(&baseline,&purple)>0);
    assert!(purple.iter().any(|p|p[2]>p[1].saturating_add(15)&&p[0]>p[1].saturating_add(10)),"preview must be visibly purple");
    let mut changed_doc=doc.clone();changed_doc["nodes"]["group"]["params"]["pattern"]=serde_json::json!("0");
    let changed_geometry=minihou_core::Graph::from_value(&changed_doc).unwrap().cook(Some("group")).unwrap();
    state.update(&mut gpu,&device,Some(&changed_geometry),&changed_doc,"group",2);
    assert_eq!(gpu.group_face_n,6,"a group parameter edit must refresh its membership preview");
    state.update(&mut gpu,&device,Some(&g),&doc,"grid",2);
    assert_eq!(gpu.group_face_n,0);assert_eq!(gpu.group_line_n,0);
    let cleared=display_render_tests::render(&device,&queue,&gpu,Mode::Solid,options,false);
    assert_eq!(cleared,baseline,"leaving Group Create must remove the overlay");
}
