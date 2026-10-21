use super::*;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct Element { pub(super) a:[f32;3], pub(super) b:[f32;3], pub(super) kind:u32, pub(super) selected:u32 }
/// Instance data is expanded from stable point/segment indices, never from the LOD triangles.
/// Surface points (kind 2) draw in point selection mode, or when the overlays are on and the points overlay is on.
pub(super) fn points_visible(o:&crate::ui::Overlays, mode:crate::ui::SelectionMode) -> bool { match mode { crate::ui::SelectionMode::Points=>true, crate::ui::SelectionMode::Object=>false, _=>o.enabled && o.points } }
/// Object-mode rule, in one place: points that are a face corner or an edge end are kind 2 (hidden in object mode);
/// only points used by no face and no edge are kind 1 (stray). Stray edges are the kind 0 segments.
fn point_kind(id:u32, loose:&std::collections::HashSet<u32>, edge_ends:&std::collections::HashSet<u32>) -> u32 {
    if loose.contains(&id) && !edge_ends.contains(&id) {1} else {2}
}

pub(super) struct Elements { pub(super) buffer:wgpu::Buffer, pub(super) selected:u32, pub(super) total:u32,
    pub(super) first_selected:u32, pub(super) first_total:u32, pub(super) chunks:Vec<(wgpu::Buffer,u32,u32)> }
/// Point previews arrive as three axis-aligned segments per point. Keep real edges
/// as lines and turn only that recognisable cross geometry into disc instances.
pub(super) fn split_point_crosses(lines:&[f32]) -> (Vec<f32>,Vec<f32>) {
    let mut points=Vec::new(); let mut outlines=Vec::new();
    let mut offset=0;
    while offset+18<=lines.len() {
        let chunk=&lines[offset..offset+18];
        let centers:Vec<[f32;3]>=(0..3).map(|axis| std::array::from_fn(|k| (chunk[axis*6+k]+chunk[axis*6+3+k])*0.5)).collect();
        let cross=centers.iter().all(|c| c.iter().zip(centers[0]).all(|(a,b)| (a-b).abs()<1e-5))
            && (0..3).all(|axis| (0..3).all(|k| ((chunk[axis*6+k]-chunk[axis*6+3+k]).abs()>1e-6) == (k==axis)));
        if cross { points.extend_from_slice(&centers[0]); offset+=18; }
        else { outlines.extend_from_slice(&lines[offset..offset+6]); offset+=6; }
    }
    outlines.extend_from_slice(&lines[offset..]);
    (points,outlines)
}
#[cfg(test)]
#[test]
fn viewport_point_highlights_become_discs_without_losing_edges() {
    let edge=[0.,0.,0.,1.,0.,0.];
    let cross=[0.9,2.,3.,1.1,2.,3.,1.,1.9,3.,1.,2.1,3.,1.,2.,2.9,1.,2.,3.1];
    let mut lines=edge.to_vec(); lines.extend_from_slice(&cross);
    let (points,outlines)=split_point_crosses(&lines);
    assert_eq!(points,vec![1.,2.,3.]);
    assert_eq!(outlines,edge);
}
impl Elements {
    pub(super) fn new(device:&wgpu::Device, scene:&Scene, ranges:&[std::ops::Range<u32>], selected_points:&[u32]) -> Self {
        let instances=Self::instances(scene,ranges,selected_points);
        let count=instances.1; let instances=instances.0;
        let limit=device.limits().max_buffer_size.min(device.limits().max_storage_buffer_binding_size as u64);
        let capacity=(limit / std::mem::size_of::<Element>() as u64).max(1) as usize;
        let upload=|slice:&[Element]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some("point and curve instances"),contents:if slice.is_empty() { &[0u8;32] } else { bytemuck::cast_slice(slice) },usage:wgpu::BufferUsages::VERTEX });
        let mut chunks=Vec::new();
        for (start, slice) in instances.chunks(capacity).enumerate() {
            let start=start*capacity;
            let selected=(count as usize).saturating_sub(start).min(slice.len()) as u32;
            chunks.push((upload(slice),selected,slice.len() as u32));
        }
        let (buffer,first_selected,first_total)=if chunks.is_empty() {(upload(&[]),0,0)} else {chunks.remove(0)};
        Self {buffer,selected:count,total:instances.len() as u32,first_selected,first_total,chunks}
    }
    pub(super) fn instances(scene:&Scene, ranges:&[std::ops::Range<u32>], selected_points:&[u32]) -> (Vec<Element>,u32) {
        let mut instances=Vec::new(); let mut count=0;
        let edge_ends:std::collections::HashSet<u32>=scene.line_primitive_indices().iter().copied().collect();
        let loose:std::collections::HashSet<_>=scene.point_indices().iter().copied().collect();
        let picked:std::collections::HashSet<_>=selected_points.iter().copied().collect();
        let at=|id:u32| { let k=id as usize*3; [scene.p[k],scene.p[k+1],scene.p[k+2]] };
        for selected in [true,false] {
            let belongs=|ids:&[u32]| ranges.iter().any(|r| ids.iter().all(|id| r.contains(id)))==selected;
            for id in 0..(scene.p.len()/3) as u32 { if belongs(&[id]) {
                instances.push(Element { a:at(id), b:at(id), kind:point_kind(id,&loose,&edge_ends), selected:picked.contains(&id) as u32 });
            } }
            for pair in scene.line_primitive_indices().chunks_exact(2) { if belongs(pair) {
                instances.push(Element { a:at(pair[0]), b:at(pair[1]), kind:0, selected:0 });
            } }
            if selected { count=instances.len() as u32; }
        }
        (instances,count)
    }
}

