//! Cached face-centre instances: camera motion is handled entirely by the shader.
use super::*;

#[derive(PartialEq)]
struct State { selected: Vec<u32>, hovered: Vec<u32>, visibility: ObjectVisibility, ranges: Vec<std::ops::Range<u32>> }
pub(super) struct FaceMarkers {
    pub buffer: wgpu::Buffer, pub count: u32,
    pub hover_buffer: wgpu::Buffer, pub hover_count: u32,
    state: Option<State>,
}
impl FaceMarkers {
    pub fn new(device: &wgpu::Device) -> Self {
        let empty = || device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("face centre instances"), contents: &[0;32], usage: wgpu::BufferUsages::VERTEX,
        });
        Self { buffer: empty(), count: 0, hover_buffer: empty(), hover_count: 0, state: None }
    }
    pub fn invalidate(&mut self) { self.state = None; }
    pub fn update(&mut self, device: &wgpu::Device, scene: &Scene, topo: &crate::element_select::Topology,
        selected: &[u32], hovered: &[u32], visibility: ObjectVisibility, ranges: &[std::ops::Range<u32>]) -> bool {
        if self.state.as_ref().is_some_and(|s| s.selected == selected && s.hovered == hovered && s.visibility == visibility && s.ranges == ranges) { return false; }
        let base_changed = self.state.as_ref().is_none_or(|s| s.selected != selected || s.visibility != visibility || s.ranges != ranges);
        let marker = |poly: u32, state: u32| {
            let vertices = topo.polygon_vertices(poly);
            if vertices.is_empty() || (visibility != ObjectVisibility::ShowAll && !vertices.iter().all(|id| ranges.iter().any(|r| r.contains(id)))) { return None; }
            let center = [0,1,2].map(|k| vertices.iter().map(|&id| scene.p[id as usize*3+k]).sum::<f32>() / vertices.len() as f32);
            Some(Element { a: center, b: center, kind: 0, selected: state })
        };
        let buffer = |instances: &[Element]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("face centre instances"), contents: if instances.is_empty() { &[0;32] } else { bytemuck::cast_slice(instances) }, usage: wgpu::BufferUsages::VERTEX,
        });
        if base_changed {
            let selected_mask = topo.polygon_selection_mask(selected);
            let instances: Vec<_> = topo.polygons().filter_map(|poly| marker(poly, selected_mask[poly as usize] as u32)).collect();
            self.buffer = buffer(&instances);
            self.count = instances.len() as u32;
        }
        // Pointer movement updates only the hovered polygons, not the full mesh.
        let hover: Vec<_> = topo.polys_of(hovered).into_iter().filter_map(|poly| marker(poly, 2)).collect();
        self.hover_buffer = buffer(&hover);
        self.hover_count = hover.len() as u32;
        self.state = Some(State { selected: selected.to_vec(), hovered: hovered.to_vec(), visibility, ranges: ranges.to_vec() });
        true
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn face_markers_are_cached_and_render_selection() {
        let Some((device, queue)) = super::super::display_render_tests::adapter() else { eprintln!("No GPU adapter"); return; };
        let graph = minihou_core::Graph::from_json(include_str!("test_data/demo_graph.json")).unwrap();
        let scene = Scene::new(&graph.cook(Some("output1")).unwrap());
        let topo = crate::element_select::Topology::new(scene.positions(),scene.triangle_indices(),scene.edge_indices());
        let selected: Vec<_> = (0..scene.triangles() as u32).collect();
        let mut gpu = Gpu::new(&device,&scene);
        let start = std::time::Instant::now();
        assert!(gpu.face_markers.update(&device,&scene,&topo,&selected,&[],ObjectVisibility::ShowAll,&[]));
        assert_eq!(gpu.face_markers.count as usize,topo.polygons().len());
        eprintln!("Upload {} face markers: {:?}",gpu.face_markers.count,start.elapsed());
        let start = std::time::Instant::now();
        for _ in 0..100 { assert!(!gpu.face_markers.update(&device,&scene,&topo,&selected,&[],ObjectVisibility::ShowAll,&[])); }
        eprintln!("100 cached face-marker frames: {:?}",start.elapsed());
        let image = super::super::display_render_tests::render_selection(&device,&queue,&gpu,Mode::Solid,Overlay::default(),false,crate::ui::SelectionMode::Faces);
        assert!(image.iter().any(|p| p[0]>230 && (130..=180).contains(&p[1]) && p[2]<70),"selected centres must render orange");
        assert!(gpu.face_markers.update(&device,&scene,&topo,&[],&[],ObjectVisibility::ShowAll,&[]));
        let base_buffer = gpu.face_markers.buffer.clone();
        assert!(gpu.face_markers.update(&device,&scene,&topo,&[],&[0],ObjectVisibility::ShowAll,&[]));
        assert_eq!(gpu.face_markers.buffer,base_buffer,"hover must not rebuild the base instances");
        assert_eq!(gpu.face_markers.hover_count,1);
        let hover = super::super::display_render_tests::render_selection(&device,&queue,&gpu,Mode::Solid,Overlay::default(),false,crate::ui::SelectionMode::Faces);
        assert!(super::super::display_render_tests::changed(&image,&hover)>0);
        assert!(gpu.face_markers.update(&device,&scene,&topo,&[],&[],ObjectVisibility::HideOthers,&[]));
        assert_eq!(gpu.face_markers.count,0);
        gpu.upload_lod(&device,&scene);
        assert!(gpu.face_markers.update(&device,&scene,&topo,&selected,&[],ObjectVisibility::ShowAll,&[]));
    }
}
