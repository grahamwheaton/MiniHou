use super::*;

pub(super) struct MeshSource { pub(super) p: Vec<f32>, pub(super) n: Vec<f32>, pub(super) colors: Vec<f32>, pub(super) cd: Option<Vec<f32>> }

pub(super) struct MeshChunk {
    pub(super) p: wgpu::Buffer, pub(super) n: wgpu::Buffer, pub(super) colors: wgpu::Buffer, pub(super) cd: Option<wgpu::Buffer>,
    pub(super) indices: wgpu::Buffer, pub(super) selected: u32, pub(super) total: u32,
}

#[derive(Default)]
struct CpuChunk {
    vertices: Vec<u32>, indices: Vec<u32>, selected: u32,
}

pub(super) fn capped_normal_vertices(float_count: usize, byte_limit: u64) -> usize {
    (float_count / 3).min((byte_limit / 12) as usize) & !1
}

pub(super) fn chunked_scene(sc: &Scene, byte_limit: u64) -> bool {
    [sc.p.len().saturating_mul(4), sc.n.len(), sc.cd.as_ref().map_or(0, Vec::len),
        sc.tri.len(), sc.edge.len(), sc.nlp.len(), sc.nln.len(), sc.bbp.len()]
        .into_iter().any(|n| (n as u64).saturating_mul(4) > byte_limit)
}

pub(super) fn neutral_color_len(sc: &Scene, oversized: bool, byte_limit: u64) -> usize {
    let floats = if oversized { sc.nlp.len().max(24) }
        else { sc.p.len().saturating_mul(4).max(sc.tri.len()).max(sc.nlp.len()).max(24) };
    floats.min((byte_limit / 4) as usize)
}

pub(super) fn box_chunks(device: &wgpu::Device, sc: &Scene, byte_limit: u64) -> Vec<MeshChunk> {
    if (sc.bbp.len() as u64) * 4 <= byte_limit { return Vec::new(); }
    let source = MeshSource { p: sc.bbp.to_vec(), n: vec![0.; sc.bbp.len()],
        colors: vec![0.72; sc.bbp.len()], cd: None };
    upload_chunks(device, &source, &BOX_EDGES, 2, &[], byte_limit)
}

/// Partition complete primitives, duplicating shared vertices at a boundary.
/// The selected portion precedes the remaining primitives in each GPU chunk.
fn chunk_primitives(indices: &[u32], width: usize, selected: &[std::ops::Range<u32>],
                    vertex_count: usize, byte_limit: u64) -> Vec<CpuChunk> {
    let max_vertices = (byte_limit / 12).min(u32::MAX as u64) as usize;
    let max_indices = (byte_limit / 4).min(u32::MAX as u64) as usize;
    assert!(max_vertices >= width && max_indices >= width, "GPU buffer limit cannot hold one primitive");
    let (selected_indices, other_indices) = split_indices(indices, width, selected);
    let mut chunks = Vec::new();
    let mut chunk = CpuChunk::default();
    let mut map = HashMap::new();
    for (is_selected, primitives) in [(true, selected_indices.as_slice()), (false, other_indices.as_slice())] {
        if !is_selected && !chunk.indices.is_empty() {
            chunks.push(std::mem::take(&mut chunk)); map.clear();
        }
        for primitive in primitives.chunks_exact(width) {
            let new_vertices = primitive.iter().filter(|&&id| !map.contains_key(&id)).count();
            if !chunk.indices.is_empty() &&
                (chunk.vertices.len() + new_vertices > max_vertices || chunk.indices.len() + width > max_indices) {
                chunks.push(std::mem::take(&mut chunk)); map.clear();
            }
            for &id in primitive {
                assert!((id as usize) < vertex_count, "mesh index outside vertex array");
                let local = *map.entry(id).or_insert_with(|| {
                    let local = chunk.vertices.len() as u32;
                    chunk.vertices.push(id);
                    local
                });
                chunk.indices.push(local);
            }
            if is_selected { chunk.selected += width as u32; }
        }
    }
    if !chunk.indices.is_empty() { chunks.push(chunk); }
    chunks
}

pub(super) fn upload_chunks(device: &wgpu::Device, source: &MeshSource, indices: &[u32], width: usize,
                 selected: &[std::ops::Range<u32>], limit: u64) -> Vec<MeshChunk> {
    chunk_primitives(indices, width, selected, source.p.len()/3, limit).into_iter().map(|chunk| {
        let gather = |values: &[f32]| -> Vec<f32> {
            chunk.vertices.iter().flat_map(|&id| values[id as usize*3..id as usize*3+3].iter().copied()).collect()
        };
        let buffer = |label, values: &[f32]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label), contents: bytemuck::cast_slice(values), usage: wgpu::BufferUsages::VERTEX,
        });
        let p = buffer("chunk P", &gather(&source.p));
        let n = buffer("chunk N", &gather(&source.n));
        let colors = buffer("chunk colors", &gather(&source.colors));
        let cd = source.cd.as_ref().map(|c| buffer("chunk Cd", &gather(c)));
        let indices = index_buffer(device, "chunk indices", &chunk.indices);
        MeshChunk { p, n, colors, cd, indices, selected: chunk.selected, total: chunk.indices.len() as u32 }
    }).collect()
}

#[cfg(test)]
#[test]
fn primitive_chunks_cover_mesh_with_local_triangle_indices() {
    let triangles: Vec<u32> = (0..8000).flat_map(|i| [i, i+1, i+2]).collect();
    let chunks = chunk_primitives(&triangles, 3, &[], 8002, 64 * 1024);
    assert!(chunks.len() > 1);
    let reconstructed: Vec<u32> = chunks.iter().flat_map(|c| {
        assert!(c.vertices.len()*12 <= 64*1024);
        assert!(c.indices.len()*4 <= 64*1024);
        c.indices.chunks_exact(3).flat_map(|tri| {
            tri.iter().map(|&id| { assert!((id as usize) < c.vertices.len()); c.vertices[id as usize] })
        }).collect::<Vec<_>>()
    }).collect();
    assert_eq!(reconstructed, triangles);
}

#[cfg(test)]
#[test]
fn primitive_chunks_preserve_selection_and_edge_pairs() {
    let triangles = [0,1,2, 2,3,4, 5,6,7, 7,8,9];
    let selected = [0..5];
    let chunks = chunk_primitives(&triangles, 3, &selected, 10, 36);
    let selected_triangles: Vec<[u32;3]> = chunks.iter().flat_map(|chunk| {
        chunk.indices[..chunk.selected as usize].chunks_exact(3).map(|tri| [chunk.vertices[tri[0] as usize],chunk.vertices[tri[1] as usize],chunk.vertices[tri[2] as usize]]).collect::<Vec<_>>()
    }).collect();
    assert_eq!(selected_triangles, [[0,1,2],[2,3,4]]);
    let edges = [0,1, 1,2, 2,3, 3,4, 4,5];
    let chunks = chunk_primitives(&edges, 2, &[], 6, 36);
    let rebuilt: Vec<u32> = chunks.iter().flat_map(|chunk| chunk.indices.iter().map(|&id| chunk.vertices[id as usize])).collect();
    assert_eq!(rebuilt, edges);
}

#[cfg(test)]
#[test]
fn oversized_normal_lines_and_box_fit_lowered_limit() {
    let limit = 48;
    assert_eq!(capped_normal_vertices(60, limit), 4);
    assert_eq!(capped_normal_vertices(6, limit), 2);
    let boxes = chunk_primitives(&BOX_EDGES, 2, &[], 8, limit);
    assert!(boxes.len() > 1);
    assert!(boxes.iter().all(|c| c.vertices.len() * 12 <= limit as usize && c.indices.len() * 4 <= limit as usize));
    let rebuilt: Vec<u32> = boxes.iter().flat_map(|c| c.indices.iter().map(|&i| c.vertices[i as usize])).collect();
    assert_eq!(rebuilt, BOX_EDGES);
}

impl Gpu {
    fn chunk_limit(&self) -> u64 {
        self.limits.max_buffer_size.unwrap().min(self.limits.max_storage_buffer_binding_size.unwrap())
    }

    fn rebuild_chunks(&mut self, device: &wgpu::Device) {
        if let Some(source) = &self.mesh_source {
            let limit = self.chunk_limit();
            let ranges = self.object_mask_points.as_deref().unwrap_or(&[]);
            self.face_chunks = upload_chunks(device, source, &self.current_tri, 3, ranges, limit);
            self.edge_chunks = upload_chunks(device, source, &self.current_edge, 2, ranges, limit);
            self.sharp_edge_chunks = upload_chunks(device, source, &self.sharp_edge_indices, 2, ranges, limit);
        } else {
            self.face_chunks.clear(); self.edge_chunks.clear(); self.sharp_edge_chunks.clear();
        }
    }

    pub fn upload(&mut self, device: &wgpu::Device, sc: &Scene) {
        let started = std::time::Instant::now();
        let limit = self.chunk_limit();
        let oversized = chunked_scene(sc, limit);
        self.face_markers.invalidate();
        self.tex_batches.clear(); self.texture_status.clear();
        self.elements=Elements::new(device,sc,&[],&self.element_points);
        self.current_tri=sc.tri.clone(); self.current_edge=sc.edge.clone();
        self.sharp_edge_indices=sharp_edges(sc,SKETCHUP_CREASE_DEGREES);
        self.partition_tri=Partition::new(device,if oversized {&[]} else {&sc.tri},3,&[]);
        self.partition_edge=Partition::new(device,if oversized {&[]} else {&sc.edge},2,&[]);
        self.sharp_edge=Partition::new(device,if oversized {&[]} else {&self.sharp_edge_indices},2,&[]);
        self.boxes=Some(Boxes::new(device,sc,&[],&[],&[0..(sc.p.len()/3) as u32])); self.object_ranges.clear(); self.object_ids.clear();
        self.object_mask = None; self.object_mask_n = 0; self.object_mask_points = None;
        let buf = |label, data: &[u8], usage| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label), contents:if data.is_empty() { &[0;4] } else { data }, usage
        });
        let fallback=[0f32;3];
        self.vp=buf("P",bytemuck::cast_slice(if oversized {&fallback[..]} else {&sc.p}),wgpu::BufferUsages::VERTEX);
        self.vn=buf("N",bytemuck::cast_slice(if oversized {&fallback[..]} else {&sc.n}),wgpu::BufferUsages::VERTEX);
        self.vc=buf("object colors",bytemuck::cast_slice(&vec![0.72f32;if oversized {3} else {sc.p.len().max(3)}]),wgpu::BufferUsages::VERTEX);
        self.vcd=sc.cd.as_ref().map(|c|buf("Cd colors",bytemuck::cast_slice(if oversized {&fallback[..]} else {c}),wgpu::BufferUsages::VERTEX));
        self.neutral_color=buf("neutral color",bytemuck::cast_slice(&vec![0.72f32;neutral_color_len(sc,oversized,limit)]),wgpu::BufferUsages::VERTEX);
        self.mesh_source=oversized.then(|| MeshSource {p:sc.p.clone(),n:sc.n.clone(),colors:vec![0.72;sc.p.len()],cd:sc.cd.clone()});
        self.mask_chunks.clear();
        self.rebuild_chunks(device);
        self.ntri_idx=sc.tri.len() as u32;
        self.edge_world=edge_world_of(sc);
        let normal_floats=capped_normal_vertices(sc.nlp.len().min(sc.nln.len()),limit)*3;
        self.nl_p=buf("nl_p",bytemuck::cast_slice(&sc.nlp[..normal_floats]),wgpu::BufferUsages::VERTEX);
        self.nl_n=buf("nl_n",bytemuck::cast_slice(&sc.nln[..normal_floats]),wgpu::BufferUsages::VERTEX);
        self.nl_count=(normal_floats/3) as u32; self.nl_selected=0;
        self.bb_chunks=box_chunks(device,sc,limit);
        self.bb_p=buf("bb_p",bytemuck::cast_slice(if self.bb_chunks.is_empty() {&sc.bbp[..]} else {&[0.;3][..]}),wgpu::BufferUsages::VERTEX);
        if std::env::var_os("MINIHOU_VIEWPORT_TIMING").is_some() {
            eprintln!("viewport Gpu::upload: points={} triangles={} ms={:.3}", sc.p.len()/3, sc.tri.len()/3, started.elapsed().as_secs_f64()*1000.0);
        }
    }
    /// Use full-detail selected-object triangles for a stable silhouette at every proxy/LOD level.
    pub fn select_objects(&mut self, device: &wgpu::Device, scene: &Scene, ranges: Vec<std::ops::Range<u32>>) {
        if self.object_mask_points.as_ref() == Some(&ranges) { return; }
        self.face_markers.invalidate();
        self.elements=Elements::new(device,scene,&ranges,&self.element_points);
        let indices = object_mask_indices(scene.triangle_indices(), &ranges);
        self.object_mask_n = indices.len() as u32;
        self.object_mask = (!self.mesh_source.is_some()).then(|| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("selected object mask"),
            contents: if indices.is_empty() { &[0u8; 4] } else { bytemuck::cast_slice(&indices) },
            usage: wgpu::BufferUsages::INDEX,
        }));
        self.mask_chunks = self.mesh_source.as_ref().map_or_else(Vec::new, |source| upload_chunks(device,source,&indices,3,&[],self.chunk_limit()));
        self.partition_tri=Partition::new(device,if self.mesh_source.is_some() {&[]} else {&self.current_tri},3,&ranges);
        for batch in &mut self.tex_batches {batch.repartition(device,&ranges);}
        self.partition_edge=Partition::new(device,if self.mesh_source.is_some() {&[]} else {&self.current_edge},2,&ranges);
        self.sharp_edge=Partition::new(device,if self.mesh_source.is_some() {&[]} else {&self.sharp_edge_indices},2,&ranges);
        self.object_mask_points = Some(ranges.clone());
        self.rebuild_chunks(device);
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
        let normal_floats=capped_normal_vertices(p.len().min(n.len()),self.chunk_limit())*3;
        self.nl_selected=self.nl_selected.min((normal_floats/3) as u32);
        self.nl_count=(normal_floats/3) as u32;
        self.nl_p=buf("partitioned normal lines",&p[..normal_floats]); self.nl_n=buf("partitioned normal normals",&n[..normal_floats]);
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
        if let Some(source)=&mut self.mesh_source {source.colors=colors.clone(); self.rebuild_chunks(device);
            if let Some(selected)=&self.object_mask_points {
                let indices=object_mask_indices(scene.triangle_indices(),selected);
                self.mask_chunks=upload_chunks(device,self.mesh_source.as_ref().unwrap(),&indices,3,&[],self.chunk_limit());
            }
        }
        else {self.vc=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("object colors"),contents:bytemuck::cast_slice(&colors),usage:wgpu::BufferUsages::VERTEX});}
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
        self.partition_tri=Partition::new(device,if self.mesh_source.is_some() {&[]} else {&self.current_tri},3,self.object_mask_points.as_deref().unwrap_or(&[]));
        if wire {
            let mut edges = std::collections::BTreeSet::new();
            for &id in ids { for tri in &lod.clusters[id as usize].triangles {
                for (a,b) in [(tri[0],tri[1]),(tri[1],tri[2]),(tri[2],tri[0])] { edges.insert((a.min(b),a.max(b))); }
            } }
            let mut indices=Vec::with_capacity(edges.len()*2);
            for (a,b) in edges { indices.extend_from_slice(&[a,b]); }
            self.current_edge=indices;
        } else { self.current_edge.clear(); }
        self.partition_edge=Partition::new(device,if self.mesh_source.is_some() {&[]} else {&self.current_edge},2,self.object_mask_points.as_deref().unwrap_or(&[]));
        self.rebuild_chunks(device);
    }

}
