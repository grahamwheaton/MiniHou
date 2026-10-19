//! Python image_nodes.py and images.py's in-memory pixel/grid operations.
use std::sync::Arc;
use serde_json::{json,Value};
use crate::{Geometry, Attr, Error, Result, params::Params};
use super::{Inputs,NodeSpec,blender_input::{ExternalInput,ExternalInputs,numbers}};
pub const BLENDER_IMAGE: NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None,name:"blender_image",inputs:&[],params:crate::params!("image":"";"max_size":1024;"colorspace":"auto";"size":10.;"center":[0.,0.,0.];"height":"none";"height_scale":1.),cook:waiting};
pub const IMAGE_FILE: NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None,name:"image_file",inputs:&[],params:crate::params!("file":"";"max_size":1024;"colorspace":"auto";"size":10.;"center":[0.,0.,0.];"height":"none";"height_scale":1.),cook:image_file};
pub const IMAGE_OUT: NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None,name:"image_out",inputs:super::GEO,params:crate::params!("image":"MiniHou";"mode":"lattice";"source":"Cd";"width":512;"height":512;"normalize":false;"encoding":"linear";"file":"",unsupported),cook:image_out};
fn waiting(_: &Inputs,p: &Params)->Result<Geometry>{blender_image(p,&ExternalInputs::new())}
pub fn srgb_to_linear(x:f32)->f32{if x<=0.04045{x/12.92}else{((x+0.055)/1.055).powf(2.4)}}
pub fn linear_to_srgb(x:f32)->f32{let x=x.max(0.);if x<=0.0031308{x*12.92}else{1.055*x.powf(1./2.4)-0.055}}
pub fn image_file(_: &Inputs, p: &Params) -> Result<Geometry> {
    let file = p.s("file")?;
    if file.trim().is_empty() {
        return Err(Error::new("set 'file' to a PNG file"));
    }
    let path = crate::usd_io::resolve_path(&file);
    if !path.is_file() {
        return Err(Error::new(format!("no such file: {}", path.display())));
    }
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let data = std::fs::read(&path)
        .map_err(|e| Error::new(format!("cannot read {file_name}: {e}")))?;
    let img = image::load_from_memory(&data)
        .map_err(|e| Error::new(format!("cannot read {file_name}: {e}")))?;
    let width = img.width() as usize;
    let height = img.height() as usize;
    let (is_16bit, is_32f) = match &img {
        image::DynamicImage::ImageLuma16(_)
        | image::DynamicImage::ImageLumaA16(_)
        | image::DynamicImage::ImageRgb16(_)
        | image::DynamicImage::ImageRgba16(_) => (true, false),
        image::DynamicImage::ImageRgb32F(_) | image::DynamicImage::ImageRgba32F(_) => (false, true),
        _ => (false, false),
    };
    let mut pix = Vec::with_capacity(width * height * 4);
    if is_16bit {
        let rgba = img.into_rgba16();
        let raw = rgba.as_raw();
        for y in (0..height).rev() {
            let row = &raw[y * width * 4..(y + 1) * width * 4];
            for &val in row {
                pix.push(val as f32 / 65535.0);
            }
        }
    } else if is_32f {
        let rgba = img.into_rgba32f();
        let raw = rgba.as_raw();
        for y in (0..height).rev() {
            let row = &raw[y * width * 4..(y + 1) * width * 4];
            pix.extend_from_slice(row);
        }
    } else {
        let rgba = img.into_rgba8();
        let raw = rgba.as_raw();
        for y in (0..height).rev() {
            let row = &raw[y * width * 4..(y + 1) * width * 4];
            for &val in row {
                pix.push(val as f32 / 255.0);
            }
        }
    }
    let linear = p.e("colorspace", &["auto", "stored"])? == "auto";
    if linear {
        for chunk in pix.chunks_exact_mut(4) {
            chunk[0] = srgb_to_linear(chunk[0]);
            chunk[1] = srgb_to_linear(chunk[1]);
            chunk[2] = srgb_to_linear(chunk[2]);
        }
    }
    let max_size = p.i("max_size")?.max(1) as usize;
    let factor = width.max(height).div_ceil(max_size).max(1);
    let (w, h, pix) = if factor > 1 {
        let w2 = width / factor;
        let h2 = height / factor;
        let mut down = vec![0.0f32; w2 * h2 * 4];
        let divisor = (factor * factor) as f32;
        for y in 0..h2 {
            for x in 0..w2 {
                let mut sums = [0.0f32; 4];
                for dy in 0..factor {
                    for dx in 0..factor {
                        let src = ((y * factor + dy) * width + (x * factor + dx)) * 4;
                        for c in 0..4 {
                            sums[c] += pix[src + c];
                        }
                    }
                }
                let dst = (y * w2 + x) * 4;
                for c in 0..4 {
                    down[dst + c] = sums[c] / divisor;
                }
            }
        }
        (w2, h2, down)
    } else {
        (width, height, pix)
    };
    if w < 2 || h < 2 {
        return Err(Error::new("an image needs at least 2 x 2 pixels to make a grid"));
    }
    if w.checked_mul(h).is_none_or(|n| n > 16_000_000) {
        return Err(Error::new("image exceeds 16,000,000 pixels; lower max_size"));
    }
    grid_from_pixels(&pix, w, h, p, &file_name)
}
/// The image a `blender_image` node reads; waiting conditions are `Error::waiting`.
pub fn resolve_image<'a>(p:&Params,store:&'a ExternalInputs)->Result<&'a ExternalInput>{
    let name=p.s("image")?;
    if name.is_empty(){return Err(Error::waiting("no image set: type the name of a Blender image"));}
    let data=store.get(&format!("image:{name}")).ok_or_else(||Error::waiting(format!("waiting for Blender image {name:?}: connect Blender")))?;
    match data{
        ExternalInput::Image{..}=>Ok(data),
        ExternalInput::Missing(e) if e.starts_with("no image named") => Err(Error::waiting(format!("Blender image {name:?} not found: check the name or connect Blender"))),
        ExternalInput::Missing(e)=>Err(Error::new(format!("Blender image {name:?}: {e}"))),
        _=>Err(Error::new(format!("Blender image {name:?}: invalid input"))),
    }
}
pub fn blender_image(p:&Params,store:&ExternalInputs)->Result<Geometry>{
    let data=resolve_image(p,store)?;
    let ExternalInput::Image{pixels,width,height,meta}=data else{return Err(Error::new("invalid input"));};
    let name=p.s("image")?;
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
    #[test]fn test_image_file_validations(){
        let p_empty=Params::resolve(IMAGE_FILE.params,json!({"file":""}).as_object()).unwrap();
        assert_eq!(image_file(&[],&p_empty).unwrap_err().0,"set 'file' to a PNG file");
        let p_missing=Params::resolve(IMAGE_FILE.params,json!({"file":"does_not_exist_xyz.png"}).as_object()).unwrap();
        assert!(image_file(&[],&p_missing).unwrap_err().0.contains("no such file:"));
    }
    #[test]fn test_image_file_golden_comparison(){
        let manifest_dir=env!("CARGO_MANIFEST_DIR");
        let golden_path=std::path::Path::new(manifest_dir).join("tests/fixtures/image_file_golden.json");
        let golden_text=std::fs::read_to_string(&golden_path).expect("read golden json");
        let golden:Value=serde_json::from_str(&golden_text).expect("parse golden json");

        let file8=std::path::Path::new(manifest_dir).join("tests/fixtures/image_file_test.png");
        let p8=Params::resolve(IMAGE_FILE.params,json!({
            "file":file8.to_str().unwrap(),
            "max_size":1024,
            "colorspace":"auto",
            "size":10.0,
            "center":[1.0,2.0,3.0],
            "height":"red",
            "height_scale":2.0
        }).as_object()).unwrap();
        let g8=image_file(&[],&p8).expect("cook image_file 8bit");
        let want8=&golden["geo8"];
        assert_eq!(g8.npoints,want8["npoints"].as_u64().unwrap() as usize);
        assert_eq!(g8.nprims(),want8["nprims"].as_u64().unwrap() as usize);
        let want_vp:Vec<i32>=want8["vertex_point"].as_array().unwrap().iter().map(|v|v.as_i64().unwrap() as i32).collect();
        assert_eq!(g8.topology.vertex_point,want_vp);
        let want_po:Vec<i32>=want8["prim_offsets"].as_array().unwrap().iter().map(|v|v.as_i64().unwrap() as i32).collect();
        assert_eq!(g8.topology.prim_offsets,want_po);
        let want_p:Vec<Vec<f32>>=serde_json::from_value(want8["P"].clone()).unwrap();
        let got_p=g8.p();
        for (i,pt) in want_p.iter().enumerate(){
            assert!((got_p[i*3]-pt[0]).abs()<1e-4,"P[{}][0]: {} vs {}",i,got_p[i*3],pt[0]);
            assert!((got_p[i*3+1]-pt[1]).abs()<1e-4,"P[{}][1]: {} vs {}",i,got_p[i*3+1],pt[1]);
            assert!((got_p[i*3+2]-pt[2]).abs()<1e-4,"P[{}][2]: {} vs {}",i,got_p[i*3+2],pt[2]);
        }
        let want_cd:Vec<Vec<f32>>=serde_json::from_value(want8["Cd"].clone()).unwrap();
        let got_cd=numbers(&g8.point["Cd"]);
        for (i,cd) in want_cd.iter().enumerate(){
            assert!((got_cd[i*3] as f32-cd[0]).abs()<1e-4);
            assert!((got_cd[i*3+1] as f32-cd[1]).abs()<1e-4);
            assert!((got_cd[i*3+2] as f32-cd[2]).abs()<1e-4);
        }
        assert!(g8.point.contains_key("Alpha"));
        let want_alpha:Vec<f32>=serde_json::from_value(want8["Alpha"].clone()).unwrap();
        let got_alpha=numbers(&g8.point["Alpha"]);
        for (i,&a) in want_alpha.iter().enumerate(){
            assert!((got_alpha[i] as f32-a).abs()<1e-4);
        }
        assert_eq!(g8.detail["grid_columns"],json!(4));
        assert_eq!(g8.detail["grid_rows"],json!(3));
        assert_eq!(g8.detail["image"],json!("image_file_test.png"));

        let file16=std::path::Path::new(manifest_dir).join("tests/fixtures/image_file_16bit.png");
        let p16=Params::resolve(IMAGE_FILE.params,json!({
            "file":file16.to_str().unwrap(),
            "max_size":1024,
            "colorspace":"stored",
            "size":5.0,
            "center":[0.0,0.0,0.0],
            "height":"luminance",
            "height_scale":1.5
        }).as_object()).unwrap();
        let g16=image_file(&[],&p16).expect("cook image_file 16bit");
        let want16=&golden["geo16"];
        assert_eq!(g16.npoints,want16["npoints"].as_u64().unwrap() as usize);
        assert_eq!(g16.nprims(),want16["nprims"].as_u64().unwrap() as usize);
        let want_p16:Vec<Vec<f32>>=serde_json::from_value(want16["P"].clone()).unwrap();
        let got_p16=g16.p();
        for (i,pt) in want_p16.iter().enumerate(){
            assert!((got_p16[i*3]-pt[0]).abs()<1e-4);
            assert!((got_p16[i*3+1]-pt[1]).abs()<1e-4);
            assert!((got_p16[i*3+2]-pt[2]).abs()<1e-4);
        }
        assert!(!g16.point.contains_key("Alpha"));
    }
}
