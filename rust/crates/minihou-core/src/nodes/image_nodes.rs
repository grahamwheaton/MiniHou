//! Python image_nodes.py and images.py's in-memory pixel/grid operations.
use std::sync::Arc;
use serde_json::{json,Value};
use crate::{Geometry, Attr, Error, Result, params::Params};
use super::{Inputs,NodeSpec,blender_input::{ExternalInput,ExternalInputs,numbers}};
pub const BLENDER_IMAGE: NodeSpec=NodeSpec{name:"blender_image",inputs:&[],params:crate::params!("image":"";"max_size":1024;"colorspace":"auto";"size":10.;"center":[0.,0.,0.];"height":"none";"height_scale":1.),cook:waiting};
pub const IMAGE_OUT: NodeSpec=NodeSpec{name:"image_out",inputs:super::GEO,params:crate::params!("image":"MiniHou";"mode":"lattice";"source":"Cd";"width":512;"height":512;"normalize":false;"encoding":"linear";"file":"",unsupported),cook:image_out};
fn waiting(_: &Inputs,p: &Params)->Result<Geometry>{blender_image(p,&ExternalInputs::new())}
pub fn srgb_to_linear(x:f32)->f32{if x<=0.04045{x/12.92}else{((x+0.055)/1.055).powf(2.4)}}
pub fn linear_to_srgb(x:f32)->f32{let x=x.max(0.);if x<=0.0031308{x*12.92}else{1.055*x.powf(1./2.4)-0.055}}
pub fn blender_image(p:&Params,store:&ExternalInputs)->Result<Geometry>{
    let name=p.s("image")?;
    if name.is_empty(){return Err(Error::new("set 'image' to the name of a Blender image"));}
    let data=store.get(&format!("image:{name}")).ok_or_else(||Error::new(format!("waiting for Blender image {name:?} (is Blender connected to this engine?)")))?;
    let ExternalInput::Image{pixels,width,height,meta}=data else{return Err(Error::new(format!("Blender image {name:?}: {}",match data{ExternalInput::Missing(e)=>e.as_str(),_=>"invalid input"})));};
    let max_size=p.i("max_size")?.max(1) as usize;
    let factor=(*width).max(*height).div_ceil(max_size).max(1);
    let w=width/factor;let h=height/factor;
    if w<2||h<2{return Err(Error::new("an image needs at least 2 x 2 pixels to make a grid"));}
    if w.checked_mul(h).is_none_or(|n|n>16_000_000){return Err(Error::new("image exceeds 16,000,000 pixels; lower max_size"));}
    if width.checked_mul(*height).and_then(|n|n.checked_mul(4))!=Some(pixels.len()){return Err(Error::new("invalid pixel dimensions"));}
    let linear=p.e("colorspace",&["auto","stored"])?=="auto"&&!meta["is_float"].as_bool().unwrap_or(false)&&meta["colorspace"].as_str().unwrap_or("").to_lowercase().starts_with("srgb");
    let mut pix=vec![0f32;w*h*4];
    // Convert before block averaging, matching Python's _linearise -> downsample order.
    for y in 0..h{for x in 0..w{for dy in 0..factor{for dx in 0..factor{for c in 0..4{
        let v=pixels[((y*factor+dy)*width+x*factor+dx)*4+c];pix[(y*w+x)*4+c]+=if linear&&c<3{srgb_to_linear(v)}else{v};
    }}}for c in 0..4{pix[(y*w+x)*4+c]/=(factor*factor) as f32;}}}
    grid_from_pixels(&pix,w,h,p,&name)
}
fn grid_from_pixels(pix:&[f32],w:usize,h:usize,p:&Params,name:&str)->Result<Geometry>{
    let size=p.f("size")? as f32;let sy=(p.f("size")?*h as f64/w as f64) as f32;let center=p.v3("center")?;
    let height=p.e("height",&["none","luminance","red","green","blue","alpha"])?;let hs=p.f("height_scale")? as f32;
    let mut positions=Vec::with_capacity(w*h*3);let mut uv=Vec::with_capacity(w*h*2);let mut cd=Vec::with_capacity(w*h*3);let mut heights=Vec::new();
    for y in 0..h{for x in 0..w{
        let u=(x as f64/(w-1) as f64) as f32;let v=(y as f64/(h-1) as f64) as f32;let rgba=&pix[(y*w+x)*4..(y*w+x+1)*4];
        let hv=match height.as_str(){"red"=>rgba[0],"green"=>rgba[1],"blue"=>rgba[2],"alpha"=>rgba[3],"luminance"=>rgba[0]*0.2126+rgba[1]*0.7152+rgba[2]*0.0722,_=>0.};
        positions.extend([(u-0.5)*size+center[0] as f32,(v-0.5)*sy+center[1] as f32,center[2] as f32+hv*hs]);uv.extend([u,v]);cd.extend_from_slice(&rgba[..3]);heights.push(hv);
    }}
    let mut vp=Vec::new();for y in 0..h-1{for x in 0..w-1{let b=(y*w+x) as i32;vp.extend([b,b+1,b+w as i32+1,b+w as i32]);}}
    let offsets=(0..=vp.len()/4).map(|i|(i*4) as i32).collect();let mut g=Geometry::from_arrays(positions,vp,offsets,None)?;
    g.point.insert("Cd".into(),Arc::new(Attr::f32(3,cd)));g.point.insert("uv".into(),Arc::new(Attr::f32(2,uv)));
    if pix.chunks_exact(4).any(|c|(c[3]-1.).abs()>1e-8+1e-5){g.point.insert("Alpha".into(),Arc::new(Attr::f32(1,pix.chunks_exact(4).map(|c|c[3]).collect())));}
    if height!="none"{g.point.insert("height".into(),Arc::new(Attr::f32(1,heights)));}
    g.detail.insert("grid_columns".into(),json!(w));g.detail.insert("grid_rows".into(),json!(h));if !name.is_empty(){g.detail.insert("image".into(),json!(name));}Ok(g)
}
fn image_out(inputs:&Inputs,p:&Params)->Result<Geometry>{let g=inputs[0].first().ok_or_else(||Error::new("input 'geo' is not connected"))?;pixels(g,p)?;p.e("encoding",&["linear","srgb","data"])?;Ok((**g).clone())}
/// RGBA pixels, bottom row first. Image alpha comes from the chosen source, as in Python (not the separate Alpha attribute).
pub fn pixels(g:&Geometry,p:&Params)->Result<(usize,usize,Vec<f32>)>{
    let source=p.s("source")?;let name=if source.trim().is_empty(){"Cd"}else{source.trim()};
    let a=g.point.get(name).ok_or_else(||Error::new(format!("no point attribute {name:?} to make an image from; available: {}",g.point.keys().cloned().collect::<Vec<_>>().join(", "))))?;
    if a.width==0{return Err(Error::new("image source has no channels"));}let mut values:Vec<f32>=numbers(a).into_iter().map(|v|v as f32).collect();
    if p.b("normalize")?{if values.is_empty(){return Err(Error::new("cannot normalize an empty attribute"));}let lo=values.iter().copied().fold(f32::INFINITY,f32::min);let hi=values.iter().copied().fold(f32::NEG_INFINITY,f32::max);for x in &mut values{*x=if hi>lo{(*x-lo)/(hi-lo)}else{0.};}}
    let mut rgba=Vec::with_capacity(g.npoints*4);for v in values.chunks_exact(a.width){rgba.extend(match a.width{1=>[v[0],v[0],v[0],1.],2=>[v[0],v[1],0.,1.],3=>[v[0],v[1],v[2],1.],_=>[v[0],v[1],v[2],v[3]]});}
    if p.e("mode",&["lattice","bake"])?=="lattice"{
        let w=g.detail.get("grid_columns").and_then(Value::as_u64).unwrap_or(0) as usize;let h=g.detail.get("grid_rows").and_then(Value::as_u64).unwrap_or(0) as usize;
        if w==0||h==0||w.checked_mul(h)!=Some(g.npoints){return Err(Error::new("this geometry is not a pixel grid (no grid_columns / grid_rows for its points): use mode 'bake' to raster points by their position, or start from a grid or an image"));}return Ok((w,h,rgba));
    }
    let wi=p.i("width")?;let he=p.i("height")?;if wi<1||he<1||wi.checked_mul(he).is_none_or(|n|n>16_000_000){return Err(Error::new("image size is out of range (1 to 16,000,000 pixels)"));}if g.npoints==0{return Err(Error::new("there are no points to bake"));}let w=wi as usize;let h=he as usize;
    let mut lo=[f64::INFINITY;2];let mut hi=[f64::NEG_INFINITY;2];for v in g.p().chunks_exact(3){for i in 0..2{lo[i]=lo[i].min(v[i] as f64);hi[i]=hi[i].max(v[i] as f64);}}
    let mut sums=vec![[0f64;4];w*h];let mut count=vec![0usize;w*h];for (i,v) in g.p().chunks_exact(3).enumerate(){let xy:[usize;2]=std::array::from_fn(|k|{let dim=if k==0{w}else{h};let span=if hi[k]>lo[k]{hi[k]-lo[k]}else{1.};(((v[k] as f64-lo[k])/span*dim as f64) as usize).min(dim-1)});let cell=xy[1]*w+xy[0];count[cell]+=1;for c in 0..4{sums[cell][c]+=rgba[i*4+c] as f64;}}
    let out=sums.iter().zip(count).flat_map(|(v,n)|v.map(|x|if n==0{0.}else{(x/n as f64) as f32})).collect();Ok((w,h,out))
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn bake_means_and_transparency(){let mut g=Geometry::points_only(vec![0.,0.,0.,0.,0.,0.,1.,1.,0.]).unwrap();g.point.insert("Cd".into(),Arc::new(Attr::f32(1,vec![0.,1.,1.])));let p=Params::resolve(IMAGE_OUT.params,json!({"mode":"bake","width":2,"height":2}).as_object()).unwrap();let(_,_,pix)=pixels(&g,&p).unwrap();assert_eq!(&pix[..4],&[0.5,0.5,0.5,1.]);assert_eq!(&pix[4..8],&[0.;4]);assert_eq!(&pix[12..],&[1.;4]);}
    #[test]fn image_grid_height_and_alpha(){let mut store=ExternalInputs::new();store.insert("image:A".into(),ExternalInput::Image{pixels:Arc::new(vec![0.5;16]),width:2,height:2,meta:json!({"is_float":true})});let p=Params::resolve(BLENDER_IMAGE.params,json!({"image":"A","height":"red","height_scale":2.}).as_object()).unwrap();let g=blender_image(&p,&store).unwrap();assert_eq!(g.p(),&[-5.,-5.,1.,5.,-5.,1.,-5.,5.,1.,5.,5.,1.]);assert_eq!(g.topology.vertex_point,vec![0,1,3,2]);assert_eq!(g.point["Alpha"].rows(),4);}
}
