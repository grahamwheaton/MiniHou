//! Opt-in timing probe for the actual startup mesh and viewport paths.
use super::*;
#[test]
#[ignore = "manual viewport performance probe"]
fn profile_face_mode() {
    let graph = Graph::from_json(include_str!("default_graph.json")).unwrap();
    let scene = Scene::new(&graph.cook(Some("output1")).unwrap());
    let topo = element_select::Topology::new(scene.positions(),scene.triangle_indices(),scene.edge_indices());
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
    eprintln!("Adapter: {:?}",adapter.get_info());
    let (device,queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let camera = Camera::frame(scene.center,scene.radius);
    let (target,_texture) = Target::new_headless(&device,[1280,720]);
    for selected in [false,true] {
        for mode in [ui::SelectionMode::Object,ui::SelectionMode::Points,ui::SelectionMode::Edges,ui::SelectionMode::Faces] {
            let mut sel = picking::Selection {mode,..Default::default()};
            if selected { match mode {
                ui::SelectionMode::Points => sel.points=(0..scene.positions().len() as u32/3).collect(),
                ui::SelectionMode::Edges => sel.edges=scene.edge_indices().chunks_exact(2).map(|e|(e[0],e[1])).collect(),
                ui::SelectionMode::Faces => sel.tris=(0..scene.triangles() as u32).collect(),
                _ => {},
            } }
            let start=Instant::now();
            for _ in 0..20 {
                // View builds both the selection centre and normal frame every redraw.
                for _ in 0..2 {
                    let ids=selected_point_ids(&sel,mode,scene.triangle_indices());
                    let mut sum=[0f32;3];
                    for id in ids { for k in 0..3 { sum[k]+=scene.positions()[id as usize*3+k]; } }
                    std::hint::black_box(sum);
                }
            }
            let cpu=start.elapsed()/20;
            let cache=selection_metrics::Cache::default();
            let owners=std::collections::BTreeSet::new();
            cache.get(0,&sel,mode,&scene,viewport::ObjectVisibility::ShowAll,&[],&owners);
            let start=Instant::now();
            for _ in 0..100 {
                for _ in 0..2 { std::hint::black_box(cache.get(0,&sel,mode,&scene,viewport::ObjectVisibility::ShowAll,&[],&owners)); }
            }
            let cached=start.elapsed()/100;
            let mut gpu=Gpu::new(&device,&scene);
            gpu.select_points(&device,&scene,&sel.points);
            gpu.set_face_markers(&device,&scene,&topo,&sel.tris,&[],viewport::ObjectVisibility::ShowAll);
            let mut highlight=sel.clone(); highlight.points.clear();
            let (faces,lines)=picking::highlight(scene.positions(),scene.triangle_indices(),&highlight,0.01);
            gpu.set_selection(&device,&faces,&lines);
            for _ in 0..2 { gpu.draw_raw(&device,&queue,&target,&camera,Mode::Solid,Overlay::default(),mode,false);device.poll(wgpu::PollType::wait_indefinitely()).unwrap(); }
            let start=Instant::now();
            for _ in 0..10 { gpu.draw_raw(&device,&queue,&target,&camera,Mode::Solid,Overlay::default(),mode,false);device.poll(wgpu::PollType::wait_indefinitely()).unwrap(); }
            eprintln!("{mode:?}, selected={selected}: CPU selection {:?}, cached {:?}, GPU render+wait {:?}",cpu,cached,start.elapsed()/10);
        }
    }
}
