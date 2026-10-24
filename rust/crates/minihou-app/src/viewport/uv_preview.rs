use super::*;

pub(super) struct UvChunk { pub(super) vertex:wgpu::Buffer, pub(super) selected:u32, pub(super) total:u32 }
pub(super) struct UvBatch { triangles:Vec<UvTri>, pub(super) chunks:Vec<UvChunk> }

impl UvBatch {
    pub(super) fn empty() -> Self { Self {triangles:Vec::new(),chunks:Vec::new()} }
    pub(super) fn new(device:&wgpu::Device,triangles:Vec<UvTri>,ranges:&[std::ops::Range<u32>])->Self {
        let mut batch=Self {triangles,chunks:Vec::new()};batch.repartition(device,ranges);batch
    }
    pub(super) fn repartition(&mut self,device:&wgpu::Device,ranges:&[std::ops::Range<u32>]) {
        let mut vertices=Vec::with_capacity(self.triangles.len()*3);
        let mut selected_count=0usize;
        for selected in [true,false] {
            for tri in &self.triangles {
                if ranges.iter().any(|r|tri.points.iter().all(|p|r.contains(p)))==selected {
                    vertices.extend_from_slice(&tri.vertices);
                }
            }
            if selected {selected_count=vertices.len();}
        }
        let limit=device.limits().max_buffer_size.min(device.limits().max_storage_buffer_binding_size as u64);
        let capacity=triangle_capacity(limit);
        self.chunks=vertices.chunks(capacity).enumerate().map(|(index,slice)| {
            let start=index*capacity;
            UvChunk {vertex:device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("UV preview corners"),contents:bytemuck::cast_slice(slice),usage:wgpu::BufferUsages::VERTEX}),
                selected:selected_count.saturating_sub(start).min(slice.len()) as u32,total:slice.len() as u32}
        }).collect();
    }
}

fn triangle_capacity(limit:u64)->usize {
    ((limit/std::mem::size_of::<TexVertex>() as u64) as usize/3).max(1)*3
}

#[cfg(test)]
#[test]
fn uv_chunk_capacity_keeps_complete_corner_triangles() {
    let stride=std::mem::size_of::<TexVertex>() as u64;
    assert_eq!(triangle_capacity(stride*3),3);
    assert_eq!(triangle_capacity(stride*5),3);
    assert_eq!(triangle_capacity(stride*9),9);
}

pub(super) struct PackedUvGpu {
    pub(super) batch:UvBatch, instances:Vec<PackedInstance>,
    pub(super) instance_buffer:wgpu::Buffer,pub(super) selected:u32,pub(super) total:u32,
}
impl Gpu {
    pub(super) fn upload_uv_preview(&mut self,device:&wgpu::Device,scene:&Scene) {
        let mut ranges:Vec<_>=scene.packed_sources.iter().flat_map(|s|s.instances.iter().map(|i|i.range.clone())).collect();
        ranges.sort_by_key(|r|r.start);
        let ordinary=scene.uv_triangles.iter().filter(|tri| {
            let point=tri.points[0];
            let before=ranges.partition_point(|r|r.start<=point);
            before==0 || !ranges[before-1].contains(&point)
        }).cloned().collect();
        self.uv_batch=UvBatch::new(device,ordinary,self.object_mask_points.as_deref().unwrap_or(&[]));
        self.uv_packed=scene.packed_sources.iter().map(|source|PackedUvGpu {
            batch:UvBatch::new(device,source.scene.uv_triangles.clone(),&[]),
            instances:source.instances.clone(),
            instance_buffer:device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("UV preview packed instances"),contents:&[0u8;64],usage:wgpu::BufferUsages::VERTEX}),
            selected:0,total:0,
        }).collect();
        self.update_uv_packed_instances(device);
    }
    pub(super) fn update_uv_packed_instances(&mut self,device:&wgpu::Device) {
        for source in &mut self.uv_packed {
            let selected=|instance:&PackedInstance|self.object_mask_points.as_ref().is_some_and(|ranges|
                ranges.iter().any(|range|range.start<=instance.range.start && range.end>=instance.range.end));
            let mut vertices=Vec::with_capacity(source.instances.len());
            for chosen in [true,false] { for instance in &source.instances {
                if selected(instance)==chosen {vertices.push(InstanceVertex {rows:instance.rows,color:[1.;4]});}
            }}
            source.selected=source.instances.iter().filter(|instance|selected(instance)).count() as u32;
            source.total=vertices.len() as u32;
            source.instance_buffer=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("UV preview packed instances"),contents:if vertices.is_empty(){&[0u8;64]}else{bytemuck::cast_slice(&vertices)},usage:wgpu::BufferUsages::VERTEX});
        }
    }
}
