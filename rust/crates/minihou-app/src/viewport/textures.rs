use super::*;

pub(super) struct TextureBatch { pub(super) vertex:wgpu::Buffer, pub(super) bind:wgpu::BindGroup, pub(super) selected:u32, pub(super) total:u32,
    pub(super) first_selected:u32, pub(super) first_total:u32, pub(super) chunks:Vec<(wgpu::Buffer,u32,u32)>, pub(super) triangles:Vec<SurfaceTri> }
impl TextureBatch {
    pub(super) fn repartition(&mut self,device:&wgpu::Device,ranges:&[std::ops::Range<u32>]) {
        let mut vertices=Vec::with_capacity(self.triangles.len()*3);
        for selected in [true,false] { for tri in &self.triangles {
            if ranges.iter().any(|r|tri.points.iter().all(|p|r.contains(p)))==selected {vertices.extend_from_slice(&tri.vertices);}
        } if selected {self.selected=vertices.len() as u32;} }
        self.total=vertices.len() as u32;
        let limits=device.limits();
        let limit=limits.max_buffer_size.min(limits.max_storage_buffer_binding_size as u64);
        let capacity=((limit / std::mem::size_of::<TexVertex>() as u64) as usize / 3).max(1)*3;
        let upload=|slice:&[TexVertex]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some("material vertices"),contents:if slice.is_empty(){&[0u8;48]}else{bytemuck::cast_slice(slice)},usage:wgpu::BufferUsages::VERTEX});
        let mut chunks=Vec::new();
        for (index,slice) in vertices.chunks(capacity).enumerate() {
            let start=index*capacity;
            chunks.push((upload(slice),(self.selected as usize).saturating_sub(start).min(slice.len()) as u32,slice.len() as u32));
        }
        let (vertex,first_selected,first_total)=if chunks.is_empty() {(upload(&[]),0,0)} else {chunks.remove(0)};
        self.vertex=vertex;self.first_selected=first_selected;self.first_total=first_total;self.chunks=chunks;
    }
}

fn texture_address(s:&str)->wgpu::AddressMode { match s {"clamp"=>wgpu::AddressMode::ClampToEdge,"mirror"=>wgpu::AddressMode::MirrorRepeat,_=>wgpu::AddressMode::Repeat} }
fn texture_filter(s:&str)->wgpu::FilterMode {if s.starts_with("nearest") {wgpu::FilterMode::Nearest} else {wgpu::FilterMode::Linear} }

/// Downsample in linear light, then encode into the selected GPU byte format.
fn mip_chain(asset:&materials::ImageAsset,srgb:bool)->Vec<(u32,u32,Vec<u8>)> {
    let mut levels=vec![(asset.width,asset.height,asset.rgba.clone())];
    while levels.last().is_some_and(|(w,h,_)| *w>1 || *h>1) {
        let (w,h,src)=levels.last().unwrap(); let (nw,nh)=((*w/2).max(1),(*h/2).max(1));
        let mut dst=vec![0u8;nw as usize*nh as usize*4];
        for y in 0..nh {for x in 0..nw {for c in 0..4 {
            let mut sum=0.; let mut count=0.;
            for yy in 0..2 {for xx in 0..2 {let sx=(x*2+xx).min(*w-1); let sy=(y*2+yy).min(*h-1);
                let v=src[((sy*w+sx)*4+c) as usize] as f32/255.; sum+=if srgb && c<3 {materials::srgb_to_linear(v)} else {v}; count+=1.; }}
            let v=sum/count; let v=if srgb && c<3 {materials::linear_to_srgb(v)} else {v};
            dst[((y*nw+x)*4+c) as usize]=(v.clamp(0.,1.)*255.).round() as u8;
        }}}
        levels.push((nw,nh,dst));
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mip_levels_keep_alpha() {
        let asset=materials::ImageAsset {width:2,height:2,rgba:vec![255,0,0,0, 0,255,0,64, 0,0,255,128, 255,255,255,255],hash:"test".into()};
        for srgb in [false,true] {
            let levels=mip_chain(&asset,srgb);
            assert_eq!(levels[0].2,asset.rgba);
            assert_eq!(levels[1].2[3],112);
        }
    }
}

impl Gpu {
    pub fn upload_textures(&mut self,device:&wgpu::Device,queue:&wgpu::Queue,sc:&Scene,registry:&mut Registry) {
        self.tex_batches.clear(); self.texture_status.clear();
        let mut grouped:BTreeMap<String,Vec<SurfaceTri>>=BTreeMap::new();
        for tri in &sc.textured { grouped.entry(tri.slot.clone()).or_default().push(tri.clone()); }
        let mut pinned=HashSet::new(); let mut gpu_used=0usize;
        for (slot,mut triangles) in grouped {
            let binding=registry.resolve(&slot);
            let tex=binding.descriptor.as_ref().and_then(|d|d.textures.base_color.as_ref());
            if let Some(tex)=tex {for tri in &mut triangles {for k in 0..3 {
                let uv=tri.uv_sets.get(&tex.uv_set).map(|uv|uv[k]).unwrap_or([f32::NAN;2]);
                tri.vertices[k].valid=(uv[0].is_finite()&&uv[1].is_finite()) as u8 as f32;
                tri.vertices[k].uv=if tri.vertices[k].valid>0. {uv}else{[0.;2]};
            }}}
            else {for tri in &mut triangles {for v in &mut tri.vertices {v.valid=1.;v.uv=[0.;2];}}}
            let srgb=tex.is_some_and(|t|t.color_space=="srgb");
            let image=binding.image.clone().unwrap_or_else(|| {
                let checker=matches!(binding.state,State::CheckerFallback(_)|State::Unsupported(_));
                let rgba=if checker {[220,220,220,255,90,30,110,255,90,30,110,255,220,220,220,255].to_vec()} else {[255,255,255,255].to_vec()};
                materials::ImageAsset {width:if checker {2}else{1},height:if checker {2}else{1},rgba,hash:if checker {"checker".into()}else{"white".into()}}
            });
            let key=format!("{}:{}",image.hash,if srgb {"srgb"}else{"linear"}); pinned.insert(key.clone());
            let (view,resident,dimensions,downsampled)=if let Some((view,bytes,last))=self.gpu_images.get_mut(&key) {
                self.gpu_tick+=1; *last=self.gpu_tick; gpu_used+=*bytes; (view.clone(),*bytes,(image.width,image.height),false)
            } else {
                let mut levels=mip_chain(&image,srgb);
                let mut skipped=0;
                while levels.len()>1 && gpu_used+levels.iter().map(|(_,_,b)|b.len()).sum::<usize>()>registry.gpu_budget {levels.remove(0);skipped+=1;}
                let bytes=levels.iter().map(|(_,_,b)|b.len()).sum::<usize>(); gpu_used+=bytes;
                let dimensions=(levels[0].0,levels[0].1);
                let format=if srgb {wgpu::TextureFormat::Rgba8UnormSrgb}else{wgpu::TextureFormat::Rgba8Unorm};
                let texture=device.create_texture(&wgpu::TextureDescriptor {label:Some("base colour"),size:wgpu::Extent3d {width:dimensions.0,height:dimensions.1,depth_or_array_layers:1},mip_level_count:levels.len() as u32,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
                for (mip,(w,h,bytes)) in levels.iter().enumerate() {
                    queue.write_texture(wgpu::TexelCopyTextureInfo {texture:&texture,mip_level:mip as u32,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},bytes,wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(w*4),rows_per_image:Some(*h)},wgpu::Extent3d {width:*w,height:*h,depth_or_array_layers:1});
                }
                let view=texture.create_view(&Default::default());
                self.gpu_tick+=1; self.gpu_images.insert(key.clone(),(view.clone(),bytes,self.gpu_tick));
                (view,bytes,dimensions,skipped>0)
            };
            let sampler_desc=tex.map(|t|&t.sampler);
            let sampler=device.create_sampler(&wgpu::SamplerDescriptor {address_mode_u:sampler_desc.map_or(wgpu::AddressMode::Repeat,|s|texture_address(&s.wrap_u)),address_mode_v:sampler_desc.map_or(wgpu::AddressMode::Repeat,|s|texture_address(&s.wrap_v)),mag_filter:sampler_desc.map_or(wgpu::FilterMode::Linear,|s|texture_filter(&s.mag)),min_filter:sampler_desc.map_or(wgpu::FilterMode::Linear,|s|texture_filter(&s.min)),mipmap_filter:if sampler_desc.is_some_and(|s|s.min.ends_with("nearest")){wgpu::FilterMode::Nearest}else{wgpu::FilterMode::Linear},..Default::default()});
            let factor=binding.descriptor.as_ref().map_or([1.;4],|d|d.base_color_factor);
            let uniform=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("base colour factor"),contents:bytemuck::cast_slice(&factor),usage:wgpu::BufferUsages::UNIFORM});
            let bind=device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("material"),layout:&self.tex_bgl,entries:&[
                wgpu::BindGroupEntry {binding:0,resource:uniform.as_entire_binding()},wgpu::BindGroupEntry {binding:1,resource:wgpu::BindingResource::TextureView(&view)},wgpu::BindGroupEntry {binding:2,resource:wgpu::BindingResource::Sampler(&sampler)}]});
            let mut warning=match &binding.state {State::Ok=>"ok".into(),State::Missing=>"neutral".into(),State::Unsupported(e)|State::CheckerFallback(e)=>format!("checker fallback: {e}")};
            if triangles.iter().any(|t|t.vertices.iter().any(|v|v.valid==0.)) {warning.push_str("; missing UV checker");}
            if downsampled {warning.push_str("; downsampled for GPU budget");}
            self.texture_status.push(format!("{}: descriptor {} · scope {} · image hash {} · state {:?} · {}x{} · UV {} · {} / {} · {} bytes · {}",if slot.is_empty(){"(default)"}else{&slot},binding.descriptor_id.as_deref().unwrap_or("none"),binding.scope.as_deref().unwrap_or("none"),tex.and_then(|t|t.image_id.as_deref()).unwrap_or("none"),binding.state,dimensions.0,dimensions.1,tex.map_or("uv",|t|&t.uv_set),tex.map_or("linear",|t|&t.sampler.min),tex.map_or("repeat",|t|&t.sampler.wrap_u),resident,warning));
            let mut batch=TextureBatch {vertex:device.create_buffer(&wgpu::BufferDescriptor {label:Some("empty material vertices"),size:48,usage:wgpu::BufferUsages::VERTEX,mapped_at_creation:false}),bind,selected:0,total:0,first_selected:0,first_total:0,chunks:Vec::new(),triangles};
            batch.repartition(device,&self.object_mask_points.clone().unwrap_or_default());
            self.tex_batches.push(batch);
        }
        while self.gpu_images.values().map(|(_,bytes,_)|*bytes).sum::<usize>()>registry.gpu_budget {
            let Some(id)=self.gpu_images.iter().filter(|(id,_)|!pinned.contains(*id)).min_by_key(|(_,(_,_,last))|last).map(|(id,_)|id.clone()) else {break};
            self.gpu_images.remove(&id);
        }
    }
    pub fn texture_status(&self)->&[String] {&self.texture_status}
}
