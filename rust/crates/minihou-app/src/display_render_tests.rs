use super::*;

pub(super) fn adapter() -> Option<(wgpu::Device,wgpu::Queue)> {
    // One shared device/queue per test binary: creating one per test caused flaky aborts.
    static SHARED: std::sync::OnceLock<Option<(wgpu::Device,wgpu::Queue)>> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| {
        let instance=wgpu::Instance::default();
        let adapter=pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }).clone()
}
fn scene() -> Scene {
    let (mut p,mut n,mut tri,mut edge)=(Vec::new(),Vec::new(),Vec::new(),Vec::new());
    // Two separate tetrahedra: object bounds differ visibly from their surfaces.
    for (object,x) in [-1.25f32,1.25].into_iter().enumerate() {
        for q in [[-0.8,-0.7,-0.6],[0.8,-0.7,-0.6],[0.,0.9,-0.6],[0.,0.,0.9]] {
            p.extend_from_slice(&[q[0]+x,q[1],q[2]]);
            let len=(q[0]*q[0]+q[1]*q[1]+q[2]*q[2]).sqrt();
            n.extend(q.map(|v| v/len));
        }
        let start=object as u32*4;
        tri.extend([0,2,1,0,1,3,1,2,3,2,0,3].map(|i| i+start));
        edge.extend([0,1,1,2,2,0,0,3,1,3,2,3].map(|i| i+start));
    }
    let (mut nlp,mut nln)=(Vec::new(),Vec::new());
    for (p,n) in p.chunks_exact(3).zip(n.chunks_exact(3)) {
        nlp.extend_from_slice(p); nlp.extend((0..3).map(|k| p[k]+n[k]*0.2));
        nln.extend_from_slice(n); nln.extend_from_slice(n);
    }
    Scene { p,n,tri,edge,point:Vec::new(),line:Vec::new(),pick_bvh:std::sync::Arc::new(crate::picking::Bvh::new(&[],&[])),nlp,nln,normal_points:(0..8).collect(),bbp:[0.;24],center:[0.;3],radius:2.5,textured:Vec::new(),uv_triangles:Vec::new(),cd:None,packed_sources:Vec::new(),auto_lod:None }
}
pub(super) fn render(device:&wgpu::Device,queue:&wgpu::Queue,gpu:&Gpu,mode:Mode,overlay:Overlay,outline:bool)->Vec<[u8;4]> {
    render_selection(device,queue,gpu,mode,overlay,outline,crate::ui::SelectionMode::Object)
}
pub(super) fn render_selection(device:&wgpu::Device,queue:&wgpu::Queue,gpu:&Gpu,mode:Mode,overlay:Overlay,outline:bool,selection:crate::ui::SelectionMode)->Vec<[u8;4]> {
    render_selection_from_camera(device,queue,gpu,&Camera::frame([0.;3],2.5),mode,overlay,outline,selection)
}
pub(super) fn render_selection_from_camera(device:&wgpu::Device,queue:&wgpu::Queue,gpu:&Gpu,camera:&Camera,mode:Mode,overlay:Overlay,outline:bool,selection:crate::ui::SelectionMode)->Vec<[u8;4]> {
    let size=[128,128];
    let (target,texture)=Target::new_headless(device,size);
    gpu.draw_raw(device,queue,&target,camera,mode,overlay,selection,outline);
    let buffer=device.create_buffer(&wgpu::BufferDescriptor {
        label:Some("display regression pixels"),size:128*128*4,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false,
    });
    let mut encoder=device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo { texture:&texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer:&buffer,layout:wgpu::TexelCopyBufferLayout { offset:0,bytes_per_row:Some(512),rows_per_image:Some(128) } },
        wgpu::Extent3d { width:128,height:128,depth_or_array_layers:1 });
    queue.submit([encoder.finish()]);
    let (tx,rx)=std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read,move |r| tx.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap(); rx.recv().unwrap().unwrap();
    let pixels=buffer.slice(..).get_mapped_range().chunks_exact(4).map(|p| [p[0],p[1],p[2],p[3]]).collect();
    buffer.unmap(); pixels
}
pub(super) fn object_outline(p:&[u8;4])->bool { (212..=222).contains(&p[0]) && (115..=125).contains(&p[1]) && (13..=23).contains(&p[2]) }
fn outline(p:&&[u8;4])->bool { object_outline(p) }
pub(super) fn changed(a:&[[u8;4]],b:&[[u8;4]])->usize { a.iter().zip(b).filter(|(a,b)| a!=b).count() }

#[test]
fn uv_preview_shader_renders_checker_missing_and_minified_uvs() {
    let Some((device,queue))=adapter() else {eprintln!("No adapter: skipping UV preview shader test");return;};
    let scene=scene();
    let gpu=Gpu::new(&device,&scene);
    let camera=Camera::frame([0.;3],2.5);
    let (_,view)=camera.matrices(1.);
    let right=[view[0][0],view[1][0],view[2][0]];
    let up=[view[0][1],view[1][1],view[2][1]];
    let positions=[[-1.,-1.],[1.,-1.],[1.,1.],[-1.,1.]];
    let sample=|uvs:[[f32;2];4],valid:f32| -> [u8;4] {
        let vertices:Vec<TexVertex>=[0usize,1,2,0,2,3].map(|i|TexVertex {
            p:std::array::from_fn(|k|right[k]*positions[i][0]+up[k]*positions[i][1]),
            n:[0.,0.,1.],color:[0.72;3],uv:uvs[i],valid,
        }).to_vec();
        let buffer=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("synthetic UV preview"),contents:bytemuck::cast_slice(&vertices),usage:wgpu::BufferUsages::VERTEX});
        let (target,texture)=Target::new_headless(&device,[128,128]);
        gpu.draw_raw(&device,&queue,&target,&camera,Mode::SmoothShaded,Overlay {geometry:crate::ui::Overlays {uv_preview:true,..Default::default()},..Default::default()},crate::ui::SelectionMode::Object,false);
        let output=device.create_buffer(&wgpu::BufferDescriptor {label:Some("UV preview pixels"),size:128*128*4,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let mut encoder=device.create_command_encoder(&Default::default());
        {
            let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("UV preview synthetic pass"),color_attachments:&[Some(wgpu::RenderPassColorAttachment {view:&target.color,depth_slice:None,resolve_target:None,ops:wgpu::Operations {load:wgpu::LoadOp::Clear(wgpu::Color::BLACK),store:wgpu::StoreOp::Store}})],depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {view:&target.depth,depth_ops:Some(wgpu::Operations {load:wgpu::LoadOp::Clear(1.),store:wgpu::StoreOp::Store}),stencil_ops:None}),timestamp_writes:None,occlusion_query_set:None});
            pass.set_pipeline(&gpu.pipe_uv_preview);
            pass.set_bind_group(0,&gpu.bind,&[]);
            pass.set_bind_group(1,&gpu.default_tex_bind,&[]);
            pass.set_vertex_buffer(0,buffer.slice(..));
            pass.draw(0..6,0..1);
        }
        encoder.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo {texture:&texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},wgpu::TexelCopyBufferInfo {buffer:&output,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(512),rows_per_image:Some(128)}},wgpu::Extent3d {width:128,height:128,depth_or_array_layers:1});
        queue.submit([encoder.finish()]);
        let (tx,rx)=std::sync::mpsc::channel();
        output.slice(..).map_async(wgpu::MapMode::Read,move |r|tx.send(r).unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();rx.recv().unwrap().unwrap();
        let pixels=output.slice(..).get_mapped_range();
        let at=(64*128+54)*4;
        let value=[pixels[at],pixels[at+1],pixels[at+2],pixels[at+3]];
        drop(pixels);output.unmap();value
    };
    let light=sample([[0.01,0.01];4],1.);
    let dark=sample([[0.08,0.01];4],1.);
    assert!(light[0]>dark[0]+50,"light {light:?}, dark {dark:?}");
    let missing=sample([[0.01,0.01];4],0.);
    assert!(missing[0]>missing[1]+70 && missing[2]>missing[1]+40,"missing {missing:?}");
    let minified=sample([[0.,0.],[100.,0.],[100.,100.],[0.,100.]],1.);
    assert!(minified[0]>dark[0]+30 && minified[0]<light[0]-30,"minified {minified:?}");
}

#[test]
fn packed_uv_preview_renders_full_source_at_each_instance_transform() {
    let Some((device,queue))=adapter() else {eprintln!("No adapter: skipping packed UV preview test");return;};
    use minihou_core::{geometry::Attr,nodes::packed::instance_geometry,math::IDENTITY};
    use std::sync::Arc;
    let camera=Camera::frame([0.;3],2.5);
    let (_,view)=camera.matrices(1.);
    let right=[view[0][0],view[1][0],view[2][0]];
    let up=[view[0][1],view[1][1],view[2][1]];
    let mut p=Vec::new();
    for (x,y) in [(-0.45,-0.65),(0.45,-0.65),(0.45,0.65),(-0.45,0.65)] {
        p.extend((0..3).map(|k|right[k]*x+up[k]*y));
    }
    let mut source=minihou_core::Geometry::from_arrays(p,vec![0,1,2,3],vec![0,4],None).unwrap();
    source.point.insert("uv".into(),Arc::new(Attr::f32(2,vec![0.01,0.01].repeat(4))));
    let positions=[-0.6,0.6].into_iter().flat_map(|x|right.map(|v|v*x)).collect();
    let packed=instance_geometry(&[Arc::new(source)],&[0,0],positions,&[IDENTITY;2],Default::default()).unwrap();
    let scene=Scene::new(&packed);
    let gpu=Gpu::new(&device,&scene);
    assert_eq!(gpu.uv_packed.len(),1);
    assert_eq!(gpu.uv_packed[0].total,2);
    assert_eq!(gpu.uv_packed[0].batch.chunks.iter().map(|c|c.total).sum::<u32>(),6);
    let options=Overlay {geometry:crate::ui::Overlays {uv_preview:true,lines:false,points:false,wireframe:false,..Default::default()},..Default::default()};
    let checked=render(&device,&queue,&gpu,Mode::SmoothShaded,options,false);
    let plain=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {geometry:crate::ui::Overlays {uv_preview:false,..options.geometry},..options},false);
    for x in [46,82] {
        let index=64*128+x;
        assert!(checked[index][0]>plain[index][0]+20,"instance at {x}: {:?} vs {:?}",checked[index],plain[index]);
    }
}

#[test]
fn display_modes_and_object_visibility_headless() {
    let Some((device,queue))=adapter() else { eprintln!("No adapter: skipping display render test"); return; };
    let scene=scene(); let mut gpu=Gpu::new(&device,&scene);
    gpu.select_objects(&device,&scene,vec![0..4]);
    gpu.set_object_ranges(&device,&scene,vec![0..4,4..8]);
    let options=Overlay {geometry:crate::ui::Overlays {points:false,..Default::default()},..Default::default()};
    let images:Vec<_>=Mode::ALL.into_iter().map(|m| render(&device,&queue,&gpu,m,options,false)).collect();
    if let Some(directory)=std::env::var_os("MINIHOU_DISPLAY_CAPTURE_DIR") {
        let directory=std::path::PathBuf::from(directory); std::fs::create_dir_all(&directory).unwrap();
        for (i,image) in images.iter().enumerate() {
            let mut ppm=b"P6\n128 128\n255\n".to_vec();
            ppm.extend(image.iter().flat_map(|p| p[..3].iter()).copied());
            std::fs::write(directory.join(format!("mode-{i:02}.ppm")),ppm).unwrap();
        }
    }
    assert_eq!(changed(&images[1],&render(&device,&queue,&gpu,Mode::Wireframe,Overlay { xray:true,..options },false)),0,"X-ray must keep pure wireframe surface-free");
    // Surface, topology and shading semantics must produce actual pixel differences.
    for (a,b) in [(0,1),(1,2),(1,3),(3,4),(5,7),(6,7)] {
        assert!(changed(&images[a],&images[b])>3,"{} and {} are indistinguishable",Mode::ALL[a].label(),Mode::ALL[b].label());
    }
    let background=images[0][0];
    let covered=|i:usize| images[i].iter().filter(|p| **p!=background).count();
    assert!(covered(3)<covered(1),"hidden-line depth must remove occluded edges");
    for mode in Mode::ALL {
        let show=render(&device,&queue,&gpu,mode,options,true);
        assert!(show.iter().filter(outline).count()>0,"{} lost its selected silhouette",mode.label());
        let hide=render(&device,&queue,&gpu,mode,Overlay { visibility:ObjectVisibility::HideOthers,..options },true);
        let ghost=render(&device,&queue,&gpu,mode,Overlay { visibility:ObjectVisibility::GhostOthers,..options },true);
        assert!(changed(&show,&hide)>3,"{} did not hide other object",mode.label());
        assert!(changed(&ghost,&hide)>3,"{} did not ghost other object",mode.label());
        assert_eq!(hide.iter().filter(outline).count(),ghost.iter().filter(outline).count(),"other-object ghost changed selected outline in {}",mode.label());
        // Every mode also exercises x-ray and both mesh overlays without GPU validation errors.
        render(&device,&queue,&gpu,mode,Overlay { xray:true,normals:true,bounds:true,wireframe:true,..options },true);
    }
    // Empty selection must hide everything, including box geometry and outlines.
    gpu.select_objects(&device,&scene,vec![]); gpu.set_object_ranges(&device,&scene,vec![0..4,4..8]);
    for mode in Mode::ALL {
        let hide=render(&device,&queue,&gpu,mode,Overlay { normals:true,bounds:true,visibility:ObjectVisibility::HideOthers,..options },true);
        assert!(hide.iter().all(|p| *p==background),"empty selection leaked geometry in {}",mode.label());
    }
}

#[test]
fn lod_and_proxy_ownership_headless() {
    use minihou_core::lod::{Cluster,ClusterLod,Sphere};
    let Some((device,queue))=adapter() else { eprintln!("No adapter: skipping LOD render test"); return; };
    let scene=scene();
    let cluster=|triangles| Cluster { triangles, bounds:Sphere { center:[0.;3],radius:3. },error:0.,level:0 };
    let lod=ClusterLod { clusters:vec![cluster(scene.tri[..12].chunks_exact(3).map(|t| [t[0],t[1],t[2]]).collect()),
        cluster(scene.tri[12..].chunks_exact(3).map(|t| [t[0],t[1],t[2]]).collect())],..Default::default() };
    let mut gpu=Gpu::new_lod(&device,&scene);
    gpu.select_objects(&device,&scene,vec![0..4]);
    gpu.set_object_ranges(&device,&scene,vec![0..4,4..8]);
    // Reordered cluster packing must not turn cluster ranges into object ownership.
    gpu.select_lod(&device,&lod,&[1,0],true);
    assert_eq!((gpu.partition_tri.selected,gpu.partition_tri.total),(12,24));
    assert_eq!((gpu.partition_edge.selected,gpu.partition_edge.total),(12,24));
    let hide=Overlay { visibility:ObjectVisibility::HideOthers,..Default::default() };
    let lod_image=render(&device,&queue,&gpu,Mode::SmoothShaded,hide,true);
    let mut full=Gpu::new(&device,&scene);
    full.select_objects(&device,&scene,vec![0..4]); full.set_object_ranges(&device,&scene,vec![0..4,4..8]);
    let full_image=render(&device,&queue,&full,Mode::SmoothShaded,hide,true);
    // Sorting LOD line endpoints can round an interpolated channel by one UNORM unit.
    assert!(lod_image.iter().zip(&full_image).all(|(a,b)| a.iter().zip(b).all(|(a,b)| a.abs_diff(*b)<=1)),"LOD/full render exceeded one-unit rounding tolerance");
    gpu.upload_proxy(&device,&scene,0.5);
    gpu.select_objects(&device,&scene,vec![4..8]); gpu.set_object_ranges(&device,&scene,vec![0..4,4..8]);
    assert!(gpu.partition_tri.selected>0 && gpu.partition_tri.selected<gpu.partition_tri.total);
    // Full-resolution bounds stay identical across proxy levels.
    full.select_objects(&device,&scene,vec![4..8]); full.set_object_ranges(&device,&scene,vec![0..4,4..8]);
    assert_eq!(changed(&render(&device,&queue,&gpu,Mode::WireBoundingBox,hide,true),&render(&device,&queue,&full,Mode::WireBoundingBox,hide,true)),0);
}

#[test]
fn bounds_of_edge_only_objects_keep_selection_outline_headless() {
    let Some((device,queue))=adapter() else { eprintln!("No adapter: skipping curve bounds test"); return; };
    let mut scene=scene(); scene.tri.clear();
    let mut gpu=Gpu::new(&device,&scene);
    assert!(render(&device,&queue,&gpu,Mode::WireBoundingBox,Overlay::default(),true).iter().filter(outline).count()>0,
        "a standalone scene defaults to one object box");
    gpu.select_objects(&device,&scene,vec![0..4]); gpu.set_object_ranges(&device,&scene,vec![0..4,4..8]);
    assert_eq!(gpu.object_mask_n,0);
    assert_eq!(gpu.nl_selected,8);
    for mode in [Mode::WireBoundingBox] {
        let image=render(&device,&queue,&gpu,mode,Overlay { visibility:ObjectVisibility::HideOthers,..Default::default() },true);
        assert!(image.iter().filter(outline).count()>0,"edge-only object lost bounds silhouette");
    }
}

#[test]
fn random_objects_and_normal_matcap_headless() {
    let Some((device,queue))=adapter() else { eprintln!("No adapter: skipping MatCap render test"); return; };
    let scene=scene(); let mut gpu=Gpu::new(&device,&scene);
    gpu.set_named_object_ranges(&device,&scene,vec![("left".into(),0..4),("right".into(),4..8)]);
    let options=Overlay {geometry:crate::ui::Overlays {points:false,..Default::default()},..Default::default()};
    let single=render(&device,&queue,&gpu,Mode::SmoothShaded,options,false);
    let random=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {object_color:ObjectColor::Random,..options},false);
    for half in [0..64,64..128] {
        assert!((0..128).flat_map(|y|half.clone().map(move |x|y*128+x)).filter(|&i|single[i]!=random[i]).count()>50);
    }
    let mut plane=Scene {p:Vec::new(),n:Vec::new(),tri:vec![0,1,2,0,2,3],edge:vec![0,1,1,2,2,3,3,0],point:Vec::new(),line:Vec::new(),pick_bvh:std::sync::Arc::new(crate::picking::Bvh::new(&[],&[])),nlp:Vec::new(),nln:Vec::new(),normal_points:Vec::new(),bbp:[0.;24],center:[0.;3],radius:2.5,textured:Vec::new(),uv_triangles:Vec::new(),cd:None,packed_sources:Vec::new(),auto_lod:None};
    let cam=Camera::frame([0.;3],2.5); let (_,view)=cam.matrices(1.);
    let right=[view[0][0],view[1][0],view[2][0]];
    let up=[view[0][1],view[1][1],view[2][1]];
    let toward=cam.forward().map(|v|-v);
    for (x,y) in [(-1.,-1.),(1.,-1.),(1.,1.),(-1.,1.)] {
        plane.p.extend((0..3).map(|i|right[i]*x+up[i]*y)); plane.n.extend(toward);
    }
    gpu.upload(&device,&plane);
    let capture=render(&device,&queue,&gpu,Mode::MatCapShaded,Overlay {matcap:MatCap::Normal,..options},false);
    let centre=capture[64*128+64];
    assert!(centre[0].abs_diff(186)<65 && centre[1].abs_diff(186)<65 && centre[2]>235,"normal centre {centre:?}");
    for normal in plane.n.chunks_exact_mut(3) { normal.copy_from_slice(&right); }
    gpu.upload(&device,&plane);
    let smooth=render(&device,&queue,&gpu,Mode::MatCapShaded,Overlay {matcap:MatCap::Normal,..options},false);
    let flat=render(&device,&queue,&gpu,Mode::MatCapShaded,Overlay {matcap:MatCap::Normal,matcap_flat:true,..options},false);
    assert!(changed(&smooth,&flat)>100,"flat MatCap ignored face normal");
    assert!(flat[64*128+64][2]>235,"flat normal centre {:?}",flat[64*128+64]);
}

#[test]
fn sketchup_clean_matcap_uses_greyscale_faces_and_dark_edges_headless() {
    let Some((device,queue))=adapter() else { eprintln!("No adapter: skipping SketchUp clean MatCap render test"); return; };
    let scene=scene(); let gpu=Gpu::new(&device,&scene);
    let options=Overlay {matcap:MatCap::SketchUpClean,geometry:crate::ui::Overlays {points:false,..Default::default()},..Default::default()};
    let clean=render(&device,&queue,&gpu,Mode::MatCapShaded,options,false);
    let random=render(&device,&queue,&gpu,Mode::MatCapShaded,Overlay {object_color:ObjectColor::Random,..options},false);
    assert_eq!(clean,random,"object colours leaked into clean MatCap");
    let clay=render(&device,&queue,&gpu,Mode::MatCapShaded,Overlay {matcap:MatCap::Clay,..options},false);
    assert!(changed(&clean,&clay)>100,"clean MatCap capture was not selected");
    let no_edges=render(&device,&queue,&gpu,Mode::MatCapShaded,Overlay {geometry:crate::ui::Overlays {lines:false,points:false,..Default::default()},..options},false);
    assert!(changed(&clean,&no_edges)>10,"clean MatCap did not draw mesh edges");
    assert!(clean.iter().zip(&no_edges).any(|(edge,face)| edge[0]<face[0] && edge[1]<face[1] && edge[2]<face[2]
        && edge[0]==edge[1] && edge[1]==edge[2]),"clean edge shader did not produce neutral dark lines");
}

#[test]
fn sketchup_clean_matcap_hides_flat_wire_edges_headless() {
    let Some((device,queue))=adapter() else { eprintln!("No adapter: skipping SketchUp edge render test"); return; };
    let p=vec![-1.,-1.,0., 0.,-1.,0., 0.,1.,0., -1.,1.,0.,
        0.,-1.,0., 1.,-1.,0., 1.,1.,0., 0.,1.,0.];
    let g=Geometry::from_arrays(p,vec![0,1,2,3,4,5,6,7],vec![0,4,8],Some(vec![true;2])).unwrap();
    let scene=Scene::new(&g);
    let gpu=Gpu::new(&device,&scene);
    let with=Overlay {matcap:MatCap::SketchUpClean,geometry:crate::ui::Overlays {points:false,..Default::default()},..Default::default()};
    let without=Overlay {geometry:crate::ui::Overlays {lines:false,..with.geometry},..with};
    assert_eq!(render(&device,&queue,&gpu,Mode::MatCapShaded,with,false),
        render(&device,&queue,&gpu,Mode::MatCapShaded,without,false),"flat seam or open boundary leaked into clean MatCap");
    assert!(changed(&render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {wireframe:true,..with},false),
        &render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {wireframe:true,..without},false))>10,"ordinary wireframe lost its edges");
}

#[test]
fn sketchup_clean_matcap_keeps_sharp_edges_with_chunked_uploads_headless() {
    let Some((device,queue))=adapter() else { eprintln!("No adapter: skipping chunked SketchUp render test"); return; };
    let scene=scene();
    let regular=Gpu::new(&device,&scene);
    let mut chunked=Gpu::new(&device,&scene);
    chunked.limits.set_gpu_limits(48,48);
    chunked.upload(&device,&scene);
    assert!(chunked.mesh_source.is_some());
    assert!(!chunked.sharp_edge_chunks.is_empty());
    let overlay=Overlay {matcap:MatCap::SketchUpClean,geometry:crate::ui::Overlays {points:false,..Default::default()},..Default::default()};
    assert_eq!(render(&device,&queue,&regular,Mode::MatCapShaded,overlay,false),
        render(&device,&queue,&chunked,Mode::MatCapShaded,overlay,false));
}

#[test]
fn textured_two_slots_factor_checker_and_diagnostic_modes_headless() {
    let Some((device,queue))=adapter() else {eprintln!("No adapter: skipping texture render test");return;};
    use std::sync::Arc;
    use minihou_core::geometry::Attr;
    use minihou_app::materials::{Descriptor,Registry};
    let cam=Camera::frame([0.;3],2.5); let (_,view)=cam.matrices(1.);
    let right=[view[0][0],view[1][0],view[2][0]]; let up=[view[0][1],view[1][1],view[2][1]];
    let mut p=Vec::new();
    for (x,y) in [(-1.,-1.),(0.,-1.),(0.,1.),(-1.,1.),(0.,-1.),(1.,-1.),(1.,1.),(0.,1.)] {
        p.extend((0..3).map(|i|right[i]*x+up[i]*y));
    }
    let mut g=minihou_core::Geometry::from_arrays(p,vec![0,1,2,3,4,5,6,7],vec![0,4,8],None).unwrap();
    g.vertex.insert("uv".into(),Arc::new(Attr::f32(2,vec![0.,0.,1.,0.,1.,1.,0.,1., 0.,0.,1.,0.,1.,1.,0.,1.])));
    g.prim.insert("material_index".into(),Arc::new(Attr::i32(1,vec![0,1])));
    g.detail.insert("materials".into(),serde_json::json!(["Red","Green"]));
    let scene=Scene::new(&g);
    assert_eq!(scene.textured.len(),4); assert_eq!(scene.textured[0].vertices[0].uv,[0.,0.]);
    let mut registry=Registry::new(".");
    let red=registry.register_pixels(2,2,vec![255,0,0,255].repeat(4),false).unwrap();
    let green=registry.register_pixels(2,2,vec![0,255,0,255].repeat(4),false).unwrap();
    for (name,id,factor) in [("Red",red.clone(),[0.5,1.,1.,1.]),("Green",green.clone(),[1.;4])] {
        registry.register_descriptor(name,Descriptor::parse(&serde_json::json!({"version":1,"base_color_factor":factor,"textures":{"base_color":{"image_id":id}}})).unwrap());
    }
    let mut gpu=Gpu::new(&device,&scene); gpu.upload_textures(&device,&queue,&scene,&mut registry);
    let options=Overlay {textures:true,geometry:crate::ui::Overlays {points:false,lines:false,wireframe:false,..Default::default()},..Default::default()};
    let image=render(&device,&queue,&gpu,Mode::SmoothShaded,options,false);
    let left=image[64*128+46]; let right=image[64*128+82];
    assert!(left[0]>left[1]+25 && left[0]>left[2]+25,"left {left:?}");
    assert!(right[1]>right[0]+25 && right[1]>right[2]+25,"right {right:?}");
    gpu.select_objects(&device,&scene,vec![0..4]);
    let hidden=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {visibility:ObjectVisibility::HideOthers,..options},true);
    assert!(hidden[64*128+46][0]>hidden[64*128+46][1]+25);
    assert!(hidden[64*128+82][1]<right[1]/2,"unselected textured object should be hidden");
    registry.register_descriptor("Red",Descriptor::parse(&serde_json::json!({"version":1,"textures":{"base_color":{"image_id":red}}})).unwrap());
    gpu.upload_textures(&device,&queue,&scene,&mut registry);
    let full=render(&device,&queue,&gpu,Mode::SmoothShaded,options,false);
    assert!(full[64*128+46][0]>left[0]+20,"factor should darken red: {:?} vs {left:?}",full[64*128+46]);
    let matcap=render(&device,&queue,&gpu,Mode::MatCapShaded,Overlay {matcap:MatCap::Normal,..options},false);
    let matcap_off=render(&device,&queue,&gpu,Mode::MatCapShaded,Overlay {textures:false,matcap:MatCap::Normal,..options},false);
    assert_eq!(matcap,matcap_off);
    let random=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {object_color:ObjectColor::Random,..options},false);
    let random_off=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {textures:false,object_color:ObjectColor::Random,..options},false);
    assert_eq!(random,random_off);
    registry.register_descriptor("Green",Descriptor::parse(&serde_json::json!({"version":1,"textures":{"base_color":{"image_id":"sha256:missing"}}})).unwrap());
    gpu.upload_textures(&device,&queue,&scene,&mut registry);
    let missing=render(&device,&queue,&gpu,Mode::SmoothShaded,options,false);
    assert!(changed(&image,&missing)>100); assert!(gpu.texture_status().iter().any(|s|s.contains("checker fallback")));
    let mut broken=g.clone();
    broken.vertex.insert("uv".into(),Arc::new(Attr::f32(2,vec![f32::NAN;16])));
    let broken=Scene::new(&broken);gpu.upload(&device,&broken);gpu.upload_textures(&device,&queue,&broken,&mut registry);
    assert!(gpu.texture_status().iter().any(|s|s.contains("missing UV checker")));
    let invalid=render(&device,&queue,&gpu,Mode::SmoothShaded,options,false);
    assert!(changed(&missing,&invalid)>50);
}

#[test]
fn asymmetric_texture_rows_render_top_to_bottom_headless() {
    let Some((device,queue))=adapter() else {eprintln!("No adapter: skipping UV orientation render test");return;};
    use std::sync::Arc;
    use minihou_core::geometry::Attr;
    use minihou_app::materials::{Descriptor,Registry};
    let cam=Camera::frame([0.;3],2.5); let (_,view)=cam.matrices(1.);
    let right=[view[0][0],view[1][0],view[2][0]];let up=[view[0][1],view[1][1],view[2][1]];
    let mut p=Vec::new();for (x,y) in [(-1.,-1.),(1.,-1.),(1.,1.),(-1.,1.)] {p.extend((0..3).map(|i|right[i]*x+up[i]*y));}
    let mut g=minihou_core::Geometry::from_arrays(p,vec![0,1,2,3],vec![0,4],None).unwrap();
    // Blender 5.2's default plane: lower-left (0,0), upper-right (1,1).
    g.vertex.insert("uv".into(),Arc::new(Attr::f32(2,vec![0.,0.,1.,0.,1.,1.,0.,1.])));
    g.prim.insert("material_index".into(),Arc::new(Attr::i32(1,vec![0])));
    g.detail.insert("materials".into(),serde_json::json!(["Corners"]));
    let scene=Scene::new(&g); let mut registry=Registry::new(".");
    let png=include_bytes!("test_data/blender_orientation_source.png");
    let hash=registry.register_image(png).unwrap();
    registry.register_descriptor("Corners",Descriptor::parse(&serde_json::json!({"version":1,"textures":{"base_color":{"image_id":hash,"color_space":"linear","sampler":{"min":"nearest","mag":"nearest","wrap_u":"clamp","wrap_v":"clamp"}}}})).unwrap());
    let mut gpu=Gpu::new(&device,&scene);gpu.upload_textures(&device,&queue,&scene,&mut registry);
    let pixels=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {textures:true,geometry:crate::ui::Overlays {points:false,lines:false,wireframe:false,..Default::default()},..Default::default()},false);
    for (x,y,channel) in [(56,56,0),(80,48,1),(48,80,2)] {
        let p=pixels[y*128+x];assert!(u16::from(p[channel])>u16::from(p[(channel+1)%3])+40 && u16::from(p[channel])>u16::from(p[(channel+2)%3])+40,"{x},{y}: {p:?}");
    }
    let yellow=pixels[80*128+80];assert!(u16::from(yellow[0])>u16::from(yellow[2])+40 && u16::from(yellow[1])>u16::from(yellow[2])+40,"80,80: {yellow:?}");
    let dot=pixels[48*128+48];assert!(dot[..3].iter().all(|c|*c>230),"white dot: {dot:?}");
    // A descriptor path and transferred PNG bytes must decode to identical rows.
    registry.register_descriptor("Corners",Descriptor::parse(&serde_json::json!({"version":1,"textures":{"base_color":{"path":concat!(env!("CARGO_MANIFEST_DIR"),"/src/test_data/blender_orientation_source.png"),"color_space":"linear","sampler":{"min":"nearest","mag":"nearest","wrap_u":"clamp","wrap_v":"clamp"}}}})).unwrap());
    gpu.upload_textures(&device,&queue,&scene,&mut registry);
    let local=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {textures:true,geometry:crate::ui::Overlays {points:false,lines:false,wireframe:false,..Default::default()},..Default::default()},false);
    assert_eq!(pixels,local,"local-file texture differs from image asset");
}

#[test]
fn mhl1_image_asset_and_seam_geometry_render_together_headless() {
    let Some((device,queue))=adapter() else {eprintln!("No adapter: skipping linked texture render test");return;};
    use minihou_app::{image_assets::Service,link_protocol::{self,Array,Arrays},materials::Registry};
    use sha2::{Digest,Sha256};
    use std::sync::Arc;
    let cam=Camera::frame([0.;3],2.5);let (_,view)=cam.matrices(1.);
    let right=[view[0][0],view[1][0],view[2][0]];let up=[view[0][1],view[1][1],view[2][1]];
    let mut p=Vec::new();for (x,y) in [(-1.,-1.),(1.,-1.),(1.,1.),(-1.,1.)] {p.extend((0..3).map(|i|right[i]*x+up[i]*y));}
    let f32bytes=|v:Vec<f32>|v.iter().flat_map(|x|x.to_le_bytes()).collect::<Vec<_>>();
    let i32bytes=|v:Vec<i32>|v.iter().flat_map(|x|x.to_le_bytes()).collect::<Vec<_>>();
    let mut arrays=Arrays::new();
    for (key,dtype,shape,bytes) in [
        ("point/P","<f4",vec![4,3],f32bytes(p)),
        ("topo/vertex_point","<i4",vec![4],i32bytes(vec![0,1,2,3])),
        ("topo/prim_offsets","<i4",vec![2],i32bytes(vec![0,4])),
        ("vertex/uv","<f4",vec![4,2],f32bytes(vec![0.,0.,1.,0.,1.,1.,0.,1.])),
        ("prim/material_index","<i4",vec![1],i32bytes(vec![0])),
    ] {arrays.insert(key.into(),Array {dtype:dtype.into(),shape,bytes:Arc::new(bytes)});}
    let header=serde_json::json!({"type":"object_geo","object":"Seam","materials":["Paint"],
        "manifest_revision":1,"material_bindings":{"version":1,"slots":["peer1:Paint"]}});
    let frame=link_protocol::encode_message(&header,&arrays).unwrap();
    let (header,arrays)=link_protocol::read_message(&mut std::io::Cursor::new(frame),2*1024*1024).unwrap();
    let geometry=link_protocol::decode_object(&header,&arrays).unwrap();
    let scene=Scene::new(&geometry);
    assert_eq!(scene.uv_triangles.len(),2);
    assert!(scene.uv_triangles.iter().all(|t|t.vertices.iter().all(|v|v.valid==1.)));
    let png=include_bytes!("test_data/blender_orientation_source.png");
    let hash=format!("sha256:{:x}",Sha256::digest(&png));
    let mut service=Service::default();service.manifest("peer1:object:Seam",1,serde_json::json!({
        "descriptors":{"peer1:Paint":{"version":1,"textures":{"base_color":{"image_id":hash,"color_space":"linear",
        "sampler":{"min":"nearest","mag":"nearest","wrap_u":"clamp","wrap_v":"clamp"}}}}}}));
    service.begin("x",&hash,png.len()).unwrap();service.chunk("x",0,png).unwrap();service.end("x").unwrap();
    let mut registry=Registry::new(".");service.bind("peer1:object:Seam",&mut registry).unwrap();
    let mut gpu=Gpu::new(&device,&scene);gpu.upload_textures(&device,&queue,&scene,&mut registry);
    assert_eq!(gpu.uv_batch.chunks.iter().map(|c|c.total).sum::<u32>(),6);
    let pixels=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {textures:true,geometry:crate::ui::Overlays {points:false,lines:false,wireframe:false,..Default::default()},..Default::default()},false);
    let checked=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {textures:true,geometry:crate::ui::Overlays {uv_preview:true,points:false,lines:false,wireframe:false,..Default::default()},..Default::default()},false);
    assert!(changed(&pixels,&checked)>100);
    for (x,y,channel) in [(56,56,0),(80,48,1),(48,80,2)] {
        let p=pixels[y*128+x];assert!(u16::from(p[channel])>u16::from(p[(channel+1)%3])+40 && u16::from(p[channel])>u16::from(p[(channel+2)%3])+40,"{x},{y}: {p:?}");
    }
    let yellow=pixels[80*128+80];assert!(u16::from(yellow[0])>u16::from(yellow[2])+40 && u16::from(yellow[1])>u16::from(yellow[2])+40,"80,80: {yellow:?}");
    let dot=pixels[48*128+48];assert!(dot[..3].iter().all(|c|*c>230),"white dot: {dot:?}");
}
