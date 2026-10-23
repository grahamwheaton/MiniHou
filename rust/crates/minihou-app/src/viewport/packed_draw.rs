use super::*;

#[derive(Clone)]
pub(super) struct PackedCut {
    pub tri:wgpu::Buffer,pub wire:wgpu::Buffer,pub tri_count:u32,pub wire_count:u32,
}

impl Gpu {
    pub(super) fn upload_packed(&mut self, device:&wgpu::Device, scene:&Scene) {
        *self.packed_cut_cache.lock().unwrap()=None;
        let vertex=|label,data:&[f32]|device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label),contents:if data.is_empty(){&[0u8;12]}else{bytemuck::cast_slice(data)},usage:wgpu::BufferUsages::VERTEX,
        });
        self.packed=scene.packed_sources.iter().filter_map(|source| {
            let lod=source.lod.as_ref()?;
            let p=vertex("packed source P",&source.scene.p);
            let n=vertex("packed source N",&source.scene.n);
            let c=vertex("packed source color",&vec![0.72;source.scene.p.len()]);
            Some(PackedGpuSource {lod:lod.clone(),instances:source.instances.clone(),p,n,c,
                instance_buffer:device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label:Some("packed instances"),contents:&[0u8;64],usage:wgpu::BufferUsages::VERTEX,
                }),selected:0,total:0})
        }).collect();
        self.update_packed_instances(device);
    }

    pub(super) fn update_packed_instances(&mut self,device:&wgpu::Device) {
        for source in &mut self.packed {
            let selected=|instance:&PackedInstance|self.object_mask_points.as_ref().is_some_and(|ranges|
                ranges.iter().any(|range|range.start<=instance.range.start && range.end>=instance.range.end));
            let mut vertices=Vec::with_capacity(source.instances.len());
            for chosen in [true,false] {for instance in &source.instances {
                if selected(instance)!=chosen {continue;}
                let color=self.object_ranges.iter().zip(&self.object_ids)
                    .find(|(range,_)|range.start<=instance.range.start && range.end>=instance.range.end)
                    .map_or([1.;3],|(_,id)|display_modes::random_object_color(id));
                vertices.push(InstanceVertex{rows:instance.rows,color:[color[0],color[1],color[2],1.]});
            }}
            source.selected=source.instances.iter().filter(|instance|selected(instance)).count() as u32;
            source.total=vertices.len() as u32;
            source.instance_buffer=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label:Some("packed instances"),contents:if vertices.is_empty(){&[0u8;64]}else{bytemuck::cast_slice(&vertices)},usage:wgpu::BufferUsages::VERTEX,
            });
        }
    }

    pub(super) fn packed_cuts(&self,device:&wgpu::Device,cam:&Camera,size:[u32;2],wire:bool)->Vec<PackedCut> {
        if self.packed.is_empty(){return Vec::new();}
        let lod_camera=cam.lod_camera(size);
        let mut cache=self.packed_cut_cache.lock().unwrap();
        if let Some((vp,eye,pixels,was_wire,cuts))=cache.as_ref() {
            if *vp==lod_camera.view_projection && *eye==lod_camera.eye && *pixels==size && *was_wire==wire {
                return cuts.clone();
            }
        }
        let cuts=self.packed.iter().map(|source| {
            let eye=cam.eye();
            let instance=source.instances.iter().min_by(|a,b| {
                let dist=|v:&PackedInstance|(0..3).map(|k|(eye[k]-v.rows[k][3]).powi(2)).sum::<f32>();
                dist(a).total_cmp(&dist(b))
            });
            let mut lod_cam=cam.lod_camera(size);
            if let Some(instance)=instance {
                let model:[[f32;4];4]=std::array::from_fn(|c|std::array::from_fn(|r|if r==3 {if c==3 {1.} else {0.}}
                    else if c==3 {instance.rows[r][3]} else {instance.rows[r][c]}));
                let vp=lod_cam.view_projection;
                lod_cam.view_projection=std::array::from_fn(|c|std::array::from_fn(|r|(0..4).map(|k|vp[k][r]*model[c][k]).sum()));
                lod_cam.eye=[0,1,2].map(|k|eye[k]-instance.rows[k][3]);
            }
            let mut ids=source.lod.select(lod_cam,source.lod.error_px.max(1.));
            if ids.is_empty() && source.total>0 {
                ids=source.lod.clusters.iter().enumerate().filter(|(_,cluster)|cluster.level==0).map(|(id,_)|id as u32).collect();
            }
            let (tri,_)=packed_cut(&source.lod,&ids);
            let edges=if wire {uploads::cut_wire_indices(&source.lod,&ids)}else{Vec::new()};
            PackedCut {tri:index_buffer(device,"packed cut triangles",&tri),wire:index_buffer(device,"packed cut wires",&edges),
                tri_count:tri.len() as u32,wire_count:edges.len() as u32}
        }).collect::<Vec<_>>();
        *cache=Some((lod_camera.view_projection,lod_camera.eye,size,wire,cuts.clone()));
        cuts
    }
}
