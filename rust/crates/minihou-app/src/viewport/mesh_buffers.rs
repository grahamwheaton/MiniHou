use super::*;

pub(super) fn object_mask_indices(triangles: &[u32], ranges: &[std::ops::Range<u32>]) -> Vec<u32> {
    triangles.chunks_exact(3).filter(|tri| ranges.iter().any(|range| tri.iter().all(|id| range.contains(id))))
        .flatten().copied().collect()
}

#[test]
fn object_mask_excludes_unselected_outputs() {
    let triangles = [0, 1, 2, 3, 4, 5];
    assert_eq!(object_mask_indices(&triangles, &[0..3]), [0, 1, 2]);
    assert_eq!(object_mask_indices(&triangles, &[3..6]), [3, 4, 5]);
    assert_eq!(object_mask_indices(&triangles, &[0..3, 3..6]), triangles);
    assert!(object_mask_indices(&triangles, &[]).is_empty());
}

pub(super) fn edge_world_of(sc: &Scene) -> f32 {
    (2.0 * sc.radius) / ((sc.edge.len().max(2) as f32) / 2.0).sqrt()
}

/// Keep only polygon edges with adjacent surface normals above the crease angle.
/// Match endpoints by position because box faces may have separate point ids at
/// the same corner. The ordinary wire buffer remains unfiltered for edit modes.
pub(super) fn sharp_edges(sc: &Scene, threshold_degrees: f32) -> Vec<u32> {
    type Key = ([i64; 3], [i64; 3]);
    let tolerance = sc.radius.max(1e-3) * 1e-6;
    let point = |id: u32| -> Option<[f32; 3]> {
        let p = sc.p.get(id as usize * 3..id as usize * 3 + 3)?;
        Some([p[0], p[1], p[2]])
    };
    let position_key = |p: [f32; 3]| -> [i64; 3] {
        [0, 1, 2].map(|i| ((p[i] - sc.center[i]) / tolerance).round() as i64)
    };
    let edge_key = |a: u32, b: u32| -> Option<Key> {
        let a = position_key(point(a)?);
        let b = position_key(point(b)?);
        (a != b).then_some(if a < b { (a, b) } else { (b, a) })
    };
    let candidates: HashSet<Key> = sc.edge.chunks_exact(2)
        .filter_map(|edge| edge_key(edge[0], edge[1])).collect();
    let mut faces: HashMap<Key, Vec<[f32; 3]>> = HashMap::new();
    for tri in sc.tri.chunks_exact(3) {
        let (Some(a), Some(b), Some(c)) = (point(tri[0]), point(tri[1]), point(tri[2])) else { continue };
        let ab = [0, 1, 2].map(|i| b[i] - a[i]);
        let ac = [0, 1, 2].map(|i| c[i] - a[i]);
        let cross = [ab[1]*ac[2]-ab[2]*ac[1], ab[2]*ac[0]-ab[0]*ac[2], ab[0]*ac[1]-ab[1]*ac[0]];
        let length = cross.iter().map(|x| x*x).sum::<f32>().sqrt();
        if length <= 1e-12 { continue; }
        let normal = cross.map(|x| x / length);
        for (u, v) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            if let Some(key) = edge_key(u, v).filter(|key| candidates.contains(key)) {
                faces.entry(key).or_default().push(normal);
            }
        }
    }
    let limit = threshold_degrees.to_radians().cos();
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for edge in sc.edge.chunks_exact(2) {
        let Some(key) = edge_key(edge[0], edge[1]) else { continue };
        if !seen.insert(key) { continue; }
        let Some(normals) = faces.get(&key) else { continue };
        if normals.iter().enumerate().any(|(i,a)| normals[i+1..].iter()
            .any(|b| a.iter().zip(b).map(|(x,y)| x*y).sum::<f32>() < limit)) {
            result.extend_from_slice(edge);
        }
    }
    result
}

/// Selected primitives first, followed by other objects. Vertex IDs never change.
pub(super) struct Partition { pub(super) buffer: wgpu::Buffer, pub(super) selected: u32, pub(super) total: u32 }
impl Partition {
    pub(super) fn new(device: &wgpu::Device, indices: &[u32], width: usize, ranges: &[std::ops::Range<u32>]) -> Self {
        let (mut selected, other)=split_indices(indices,width,ranges);
        let count=selected.len() as u32;
        selected.extend(other);
        Self { buffer:index_buffer(device,"object partition",&selected), selected:count, total:selected.len() as u32 }
    }
    pub(super) fn visible(&self, visibility: ObjectVisibility) -> std::ops::Range<u32> {
        0..if visibility == ObjectVisibility::ShowAll { self.total } else { self.selected }
    }
}
pub(super) fn index_buffer(device: &wgpu::Device, label: &str, indices: &[u32]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:Some(label), contents:if indices.is_empty() { &[0u8;4] } else { bytemuck::cast_slice(indices) }, usage:wgpu::BufferUsages::INDEX,
    })
}
const BOX_TRIANGLES: [u32;36]=[0,2,3,0,3,1, 4,5,7,4,7,6, 0,1,5,0,5,4, 2,6,7,2,7,3, 0,4,6,0,6,2, 1,3,7,1,7,5];
pub(super) struct Boxes { pub(super) p:wgpu::Buffer, pub(super) n:wgpu::Buffer, pub(super) c:wgpu::Buffer, pub(super) tri:Partition, pub(super) edge:Partition }
impl Boxes {
    pub(super) fn new(device:&wgpu::Device, scene:&Scene, objects:&[std::ops::Range<u32>], ids:&[String], selected:&[std::ops::Range<u32>]) -> Self {
        let (mut points,mut colors,mut triangles,mut edges,mut selected_boxes)=(Vec::new(),Vec::new(),Vec::new(),Vec::new(),Vec::new());
        let fallback=[0..(scene.p.len()/3) as u32];
        let objects=if objects.is_empty() { &fallback[..] } else { objects };
        for (i,range) in objects.iter().enumerate() {
            let Some(p)=scene.p.get(range.start as usize*3..range.end as usize*3).filter(|p| !p.is_empty()) else { continue; };
            let (lo,hi)=preview::bounds(p);
            let start=(points.len()/3) as u32;
            for c in 0..8 { for k in 0..3 { points.push(if c>>k&1==1 { hi[k] } else { lo[k] }); } }
            let color=display_modes::random_object_color(ids.get(i).map(String::as_str).unwrap_or("object"));
            for _ in 0..8 { colors.extend(color); }
            triangles.extend(BOX_TRIANGLES.iter().map(|i| start+i));
            edges.extend(BOX_EDGES.iter().map(|i| start+i));
            if selected.iter().any(|s| s.start<=range.start && s.end>=range.end) { selected_boxes.push(start..start+8); }
        }
        let vertex=|label,data:&[f32]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label),contents:if data.is_empty() { &[0u8;12] } else { bytemuck::cast_slice(data) },usage:wgpu::BufferUsages::VERTEX,
        });
        Self { p:vertex("object bounds",&points),n:vertex("bounds normals",&vec![0.;points.len()]),c:vertex("bounds colors",&colors),
            tri:Partition::new(device,&triangles,3,&selected_boxes),edge:Partition::new(device,&edges,2,&selected_boxes) }
    }
}
/// Only the selected clusters enter the GPU index buffer; ranges stay contiguous.
pub(super) fn packed_cut(lod: &ClusterLod, ids: &[ClusterId]) -> (Vec<u32>, Vec<std::ops::Range<u32>>) {
    let mut indices=Vec::with_capacity(lod.selected_triangles(ids)*3);
    let mut ranges=Vec::with_capacity(ids.len());
    for &id in ids {
        let start=indices.len() as u32;
        for tri in &lod.clusters[id as usize].triangles { indices.extend_from_slice(tri); }
        ranges.push(start..indices.len() as u32);
    }
    (indices,ranges)
}
