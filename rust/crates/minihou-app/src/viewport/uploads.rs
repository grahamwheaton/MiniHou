use super::*;

impl Gpu {
    pub fn upload(&mut self, device: &wgpu::Device, sc: &Scene) {
        self.face_markers.invalidate();
        self.tex_batches.clear(); self.texture_status.clear();
        self.elements=Elements::new(device,sc,&[],&self.element_points);
        self.current_tri=sc.tri.clone(); self.current_edge=sc.edge.clone();
        self.partition_tri=Partition::new(device,&sc.tri,3,&[]);
        self.partition_edge=Partition::new(device,&sc.edge,2,&[]);
        self.boxes=Some(Boxes::new(device,sc,&[],&[],&[0..(sc.p.len()/3) as u32])); self.object_ranges.clear(); self.object_ids.clear();
        self.object_mask = None; self.object_mask_n = 0; self.object_mask_points = None;
        let buf = |label, data: &[u8], usage| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label), contents:if data.is_empty() { &[0;4] } else { data }, usage
        });
        self.vp=buf("P",bytemuck::cast_slice(&sc.p),wgpu::BufferUsages::VERTEX);
        self.vn=buf("N",bytemuck::cast_slice(&sc.n),wgpu::BufferUsages::VERTEX);
        self.vc=buf("object colors",bytemuck::cast_slice(&vec![0.72f32;sc.p.len().max(3)]),wgpu::BufferUsages::VERTEX);
        self.vcd=sc.cd.as_ref().map(|c|buf("Cd colors",bytemuck::cast_slice(c),wgpu::BufferUsages::VERTEX));
        self.neutral_color=buf("neutral color",bytemuck::cast_slice(&vec![0.72f32;(sc.p.len()*4).max(sc.tri.len()).max(24)]),wgpu::BufferUsages::VERTEX);
        self.ntri_idx=sc.tri.len() as u32;
        self.edge_world=edge_world_of(sc);
        self.nl_p=buf("nl_p",bytemuck::cast_slice(&sc.nlp),wgpu::BufferUsages::VERTEX);
        self.nl_n=buf("nl_n",bytemuck::cast_slice(&sc.nln),wgpu::BufferUsages::VERTEX);
        self.nl_count=(sc.nlp.len()/3) as u32; self.nl_selected=0;
        self.bb_p=buf("bb_p",bytemuck::cast_slice(&sc.bbp),wgpu::BufferUsages::VERTEX);
    }
    /// Use full-detail selected-object triangles for a stable silhouette at every proxy/LOD level.
    pub fn select_objects(&mut self, device: &wgpu::Device, scene: &Scene, ranges: Vec<std::ops::Range<u32>>) {
        if self.object_mask_points.as_ref() == Some(&ranges) { return; }
        self.face_markers.invalidate();
        self.elements=Elements::new(device,scene,&ranges,&self.element_points);
        let indices = object_mask_indices(scene.triangle_indices(), &ranges);
        self.object_mask_n = indices.len() as u32;
        self.object_mask = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("selected object mask"),
            contents: if indices.is_empty() { &[0u8; 4] } else { bytemuck::cast_slice(&indices) },
            usage: wgpu::BufferUsages::INDEX,
        }));
        self.partition_tri=Partition::new(device,&self.current_tri,3,&ranges);
        for batch in &mut self.tex_batches {batch.repartition(device,&ranges);}
        self.partition_edge=Partition::new(device,&self.current_edge,2,&ranges);
        self.boxes=None;
        let (mut p,mut n)=(Vec::with_capacity(scene.nlp.len()),Vec::with_capacity(scene.nln.len()));
        self.nl_selected=0;
        for selected in [true,false] {
            for (i,id) in scene.normal_points.iter().enumerate() {
                if ranges.iter().any(|r| r.contains(id))==selected {
                    p.extend_from_slice(&scene.nlp[i*6..i*6+6]); n.extend_from_slice(&scene.nln[i*6..i*6+6]);
                    if selected { self.nl_selected+=2; }
                }
            }
        }
        let buf=|label,data:&[f32]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label:Some(label),contents:if data.is_empty() { &[0u8;12] } else { bytemuck::cast_slice(data) },usage:wgpu::BufferUsages::VERTEX });
        self.nl_p=buf("partitioned normal lines",&p); self.nl_n=buf("partitioned normal normals",&n);
        self.object_mask_points = Some(ranges);
    }
    /// Bounds use full-resolution object ownership, independently of the current proxy/LOD cut.
    pub fn set_object_ranges(&mut self, device: &wgpu::Device, scene: &Scene, ranges: Vec<std::ops::Range<u32>>) {
        let named=ranges.into_iter().enumerate().map(|(i,r)| (i.to_string(),r)).collect();
        self.set_named_object_ranges(device,scene,named);
    }
    pub fn set_named_object_ranges(&mut self, device:&wgpu::Device, scene:&Scene, objects:Vec<(String,std::ops::Range<u32>)>) {
        let (ids,ranges):(Vec<_>,Vec<_>)=objects.into_iter().unzip();
        if self.boxes.is_some() && self.object_ranges==ranges && self.object_ids==ids { return; }
        let mut colors=vec![0.72f32;scene.p.len().max(3)];
        for (id,range) in ids.iter().zip(&ranges) {
            let color=display_modes::random_object_color(id);
            for i in range.clone() { if let Some(dst)=colors.get_mut(i as usize*3..i as usize*3+3) { dst.copy_from_slice(&color); } }
        }
        self.vc=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("object colors"),contents:bytemuck::cast_slice(&colors),usage:wgpu::BufferUsages::VERTEX});
        self.boxes=Some(Boxes::new(device,scene,&ranges,&ids,self.object_mask_points.as_deref().unwrap_or(&[])));
        self.object_ranges=ranges; self.object_ids=ids;
    }
    /// Retain the source vertex attributes but start with no triangles; selection uploads a cut.
    pub fn upload_lod(&mut self, device: &wgpu::Device, sc: &Scene) {
        self.face_markers.invalidate();
        self.upload(device,&sc.proxied(0.0));
    }
    pub fn select_lod(&mut self, device: &wgpu::Device, lod: &ClusterLod, ids: &[ClusterId], wire: bool) {
        let (indices, _)=packed_cut(lod, ids);
        self.ntri_idx=indices.len() as u32;
        self.current_tri=indices;
        self.partition_tri=Partition::new(device,&self.current_tri,3,self.object_mask_points.as_deref().unwrap_or(&[]));
        if wire {
            let mut edges = std::collections::BTreeSet::new();
            for &id in ids { for tri in &lod.clusters[id as usize].triangles {
                for (a,b) in [(tri[0],tri[1]),(tri[1],tri[2]),(tri[2],tri[0])] { edges.insert((a.min(b),a.max(b))); }
            } }
            let mut indices=Vec::with_capacity(edges.len()*2);
            for (a,b) in edges { indices.extend_from_slice(&[a,b]); }
            self.current_edge=indices;
        } else { self.current_edge.clear(); }
        self.partition_edge=Partition::new(device,&self.current_edge,2,self.object_mask_points.as_deref().unwrap_or(&[]));
    }

}
