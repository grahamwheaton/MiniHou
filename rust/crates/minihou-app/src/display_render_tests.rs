use super::*;

pub(super) fn adapter() -> Option<(wgpu::Device,wgpu::Queue)> {
    let instance=wgpu::Instance::default();
    let adapter=pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
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
    Scene { p,n,tri,edge,point:Vec::new(),line:Vec::new(),nlp,nln,normal_points:(0..8).collect(),bbp:[0.;24],center:[0.;3],radius:2.5,textured:Vec::new() }
}
pub(super) fn render(device:&wgpu::Device,queue:&wgpu::Queue,gpu:&Gpu,mode:Mode,overlay:Overlay,outline:bool)->Vec<[u8;4]> {
    render_selection(device,queue,gpu,mode,overlay,outline,crate::ui::SelectionMode::Object)
}
pub(super) fn render_selection(device:&wgpu::Device,queue:&wgpu::Queue,gpu:&Gpu,mode:Mode,overlay:Overlay,outline:bool,selection:crate::ui::SelectionMode)->Vec<[u8;4]> {
    let size=[128,128];
    let (target,texture)=Target::new_headless(device,size);
    gpu.draw_raw(device,queue,&target,&Camera::frame([0.;3],2.5),mode,overlay,selection,outline);
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
fn outline(p:&&[u8;4])->bool { p[0]==255 && (155..=165).contains(&p[1]) && (35..=45).contains(&p[2]) }
pub(super) fn changed(a:&[[u8;4]],b:&[[u8;4]])->usize { a.iter().zip(b).filter(|(a,b)| a!=b).count() }

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
    assert_eq!(changed(&images[2],&render(&device,&queue,&gpu,Mode::Wireframe,Overlay { xray:true,..options },false)),0,"X-ray must keep pure wireframe surface-free");
    // Surface, topology and shading semantics must produce actual pixel differences.
    for (a,b) in [(0,2),(1,8),(2,3),(2,4),(4,5),(6,10),(8,10),(8,9),(10,11)] {
        assert!(changed(&images[a],&images[b])>3,"{} and {} are indistinguishable",Mode::ALL[a].label(),Mode::ALL[b].label());
    }
    let background=images[0][0];
    let covered=|i:usize| images[i].iter().filter(|p| **p!=background).count();
    assert!(covered(4)<covered(2),"hidden-line depth must remove occluded edges");
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
    let lod_image=render(&device,&queue,&gpu,Mode::SmoothWireShaded,hide,true);
    let mut full=Gpu::new(&device,&scene);
    full.select_objects(&device,&scene,vec![0..4]); full.set_object_ranges(&device,&scene,vec![0..4,4..8]);
    let full_image=render(&device,&queue,&full,Mode::SmoothWireShaded,hide,true);
    // Sorting LOD line endpoints can round an interpolated channel by one UNORM unit.
    assert!(lod_image.iter().zip(&full_image).all(|(a,b)| a.iter().zip(b).all(|(a,b)| a.abs_diff(*b)<=1)),"LOD/full render exceeded one-unit rounding tolerance");
    gpu.upload(&device,&scene.proxied(0.5));
    gpu.select_objects(&device,&scene,vec![4..8]); gpu.set_object_ranges(&device,&scene,vec![0..4,4..8]);
    assert!(gpu.partition_tri.selected>0 && gpu.partition_tri.selected<gpu.partition_tri.total);
    // Full-resolution bounds stay identical across proxy levels.
    full.select_objects(&device,&scene,vec![4..8]); full.set_object_ranges(&device,&scene,vec![0..4,4..8]);
    assert_eq!(changed(&render(&device,&queue,&gpu,Mode::ShadedBoundingBox,hide,true),&render(&device,&queue,&full,Mode::ShadedBoundingBox,hide,true)),0);
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
    for mode in [Mode::WireBoundingBox,Mode::ShadedBoundingBox] {
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
    let mut plane=Scene {p:Vec::new(),n:Vec::new(),tri:vec![0,1,2,0,2,3],edge:vec![0,1,1,2,2,3,3,0],point:Vec::new(),line:Vec::new(),nlp:Vec::new(),nln:Vec::new(),normal_points:Vec::new(),bbp:[0.;24],center:[0.;3],radius:2.5,textured:Vec::new()};
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
    g.vertex.insert("uv".into(),Arc::new(Attr::f32(2,vec![0.,1.,1.,1.,1.,0.,0.,0.])));
    g.prim.insert("material_index".into(),Arc::new(Attr::i32(1,vec![0])));
    g.detail.insert("materials".into(),serde_json::json!(["Corners"]));
    let scene=Scene::new(&g); let mut registry=Registry::new(".");
    let hash=registry.register_pixels(2,2,vec![255,0,0,255,0,255,0,255,0,0,255,255,255,255,0,255],false).unwrap();
    registry.register_descriptor("Corners",Descriptor::parse(&serde_json::json!({"version":1,"textures":{"base_color":{"image_id":hash,"color_space":"linear","sampler":{"min":"nearest","mag":"nearest","wrap_u":"clamp","wrap_v":"clamp"}}}})).unwrap());
    let mut gpu=Gpu::new(&device,&scene);gpu.upload_textures(&device,&queue,&scene,&mut registry);
    let pixels=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {textures:true,geometry:crate::ui::Overlays {points:false,lines:false,wireframe:false,..Default::default()},..Default::default()},false);
    for (x,y,channel) in [(48,48,0),(80,48,1),(48,80,2)] {
        let p=pixels[y*128+x];assert!(p[channel]>p[(channel+1)%3]+40 && p[channel]>p[(channel+2)%3]+40,"{x},{y}: {p:?}");
    }
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
        ("vertex/uv","<f4",vec![4,2],f32bytes(vec![0.,1.,1.,1.,1.,0.,0.,0.])),
        ("prim/material_index","<i4",vec![1],i32bytes(vec![0])),
    ] {arrays.insert(key.into(),Array {dtype:dtype.into(),shape,bytes:Arc::new(bytes)});}
    let header=serde_json::json!({"type":"object_geo","object":"Seam","materials":["Paint"],
        "manifest_revision":1,"material_bindings":{"version":1,"slots":["peer1:Paint"]}});
    let frame=link_protocol::encode_message(&header,&arrays).unwrap();
    let (header,arrays)=link_protocol::read_message(&mut std::io::Cursor::new(frame),2*1024*1024).unwrap();
    let geometry=link_protocol::decode_object(&header,&arrays).unwrap();
    let scene=Scene::new(&geometry);
    let mut png=Vec::new();
    let image=image::RgbaImage::from_raw(2,2,vec![255,0,0,255,0,255,0,255,0,0,255,255,255,255,0,255]).unwrap();
    image::DynamicImage::ImageRgba8(image).write_to(&mut std::io::Cursor::new(&mut png),image::ImageFormat::Png).unwrap();
    let hash=format!("sha256:{:x}",Sha256::digest(&png));
    let mut service=Service::default();service.manifest("peer1:object:Seam",1,serde_json::json!({
        "descriptors":{"peer1:Paint":{"version":1,"textures":{"base_color":{"image_id":hash,"color_space":"linear",
        "sampler":{"min":"nearest","mag":"nearest","wrap_u":"clamp","wrap_v":"clamp"}}}}}}));
    service.begin("x",&hash,png.len()).unwrap();service.chunk("x",0,&png).unwrap();service.end("x").unwrap();
    let mut registry=Registry::new(".");service.bind("peer1:object:Seam",&mut registry).unwrap();
    let mut gpu=Gpu::new(&device,&scene);gpu.upload_textures(&device,&queue,&scene,&mut registry);
    let pixels=render(&device,&queue,&gpu,Mode::SmoothShaded,Overlay {textures:true,geometry:crate::ui::Overlays {points:false,lines:false,wireframe:false,..Default::default()},..Default::default()},false);
    for (x,y,channel) in [(48,48,0),(80,48,1),(48,80,2)] {
        let p=pixels[y*128+x];assert!(p[channel]>p[(channel+1)%3]+40 && p[channel]>p[(channel+2)%3]+40,"{x},{y}: {p:?}");
    }
}
