//! Document/session material descriptions and immutable decoded image assets.
//! Image IDs are content hashes; file paths are only a local source hint.
use std::{collections::{HashMap, HashSet}, path::{Path, PathBuf}};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const CPU_BUDGET: usize = 1024 * 1024 * 1024;
pub const GPU_BUDGET: usize = 512 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;

fn one() -> [f32; 4] { [1.; 4] }
fn opaque() -> String { "OPAQUE".into() }
fn uv() -> String { "uv".into() }
fn linear() -> String { "linear".into() }
fn repeat() -> String { "repeat".into() }
fn srgb() -> String { "srgb".into() }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Sampler { pub min: String, pub mag: String, pub wrap_u: String, pub wrap_v: String }
impl Default for Sampler { fn default() -> Self { Self { min:"linear_mipmap_linear".into(), mag:linear(), wrap_u:repeat(), wrap_v:repeat() } } }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BaseColorTexture {
    #[serde(default)] pub image_id: Option<String>,
    #[serde(default)] pub path: Option<PathBuf>,
    #[serde(default="uv")] pub uv_set: String,
    #[serde(default)] pub sampler: Sampler,
    #[serde(default="srgb")] pub color_space: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Textures { #[serde(default)] pub base_color: Option<BaseColorTexture> }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Descriptor {
    pub version: u32,
    #[serde(default)] pub name: String,
    #[serde(default="one")] pub base_color_factor: [f32; 4],
    #[serde(default)] pub textures: Textures,
    #[serde(default="opaque")] pub alpha_mode: String,
    #[serde(default)] pub alpha_cutoff: Option<f32>,
    #[serde(default)] pub double_sided: bool,
}
impl Descriptor {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let descriptor:Self=serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        if descriptor.version != 1 { return Err("material version must be 1".into()); }
        if descriptor.alpha_mode != "OPAQUE" { return Err("only OPAQUE materials are supported".into()); }
        if descriptor.base_color_factor.iter().any(|v| !v.is_finite()) { return Err("non-finite base colour factor".into()); }
        if let Some(t) = &descriptor.textures.base_color {
            if t.image_id.is_none() && t.path.is_none() { return Err("base colour image needs image_id or path".into()); }
            if !matches!(t.color_space.as_str(),"srgb"|"linear") { return Err("unsupported colour space".into()); }
            if !matches!(t.sampler.mag.as_str(),"nearest"|"linear") || !matches!(t.sampler.min.as_str(),"nearest"|"linear"|"nearest_mipmap_nearest"|"nearest_mipmap_linear"|"linear_mipmap_nearest"|"linear_mipmap_linear")
                || ![&t.sampler.wrap_u,&t.sampler.wrap_v].into_iter().all(|s| matches!(s.as_str(),"repeat"|"clamp"|"mirror")) { return Err("unsupported sampler".into()); }
        }
        Ok(descriptor)
    }
}

#[derive(Clone, Debug)]
pub struct ImageAsset { pub width:u32, pub height:u32, pub rgba:Vec<u8>, pub hash:String }
impl ImageAsset { pub fn resident_bytes(&self) -> usize { self.rgba.len() } }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State { Ok, Missing, Unsupported(String), CheckerFallback(String) }
#[derive(Clone, Debug)]
pub struct MaterialBinding { pub slot:String, pub descriptor_id:Option<String>, pub scope:Option<String>, pub descriptor:Option<Descriptor>, pub image:Option<ImageAsset>, pub state:State }

pub struct Registry {
    descriptors:HashMap<String,Descriptor>, images:HashMap<String,(ImageAsset,u64)>,
    errors:HashMap<String,String>,
    scopes:HashMap<String,String>,
    tick:u64, cpu_budget:usize, pub gpu_budget:usize, base_path:PathBuf,
}
impl Registry {
    pub fn new(base_path:impl Into<PathBuf>) -> Self { Self { descriptors:HashMap::new(), images:HashMap::new(), errors:HashMap::new(), scopes:HashMap::new(), tick:0, cpu_budget:CPU_BUDGET, gpu_budget:GPU_BUDGET, base_path:base_path.into() } }
    pub fn set_budgets(&mut self, cpu:usize, gpu:usize) { self.cpu_budget=cpu; self.gpu_budget=gpu; self.evict(&HashSet::new()); }
    pub fn register_descriptor(&mut self, id:impl Into<String>, descriptor:Descriptor) { let id=id.into(); self.errors.remove(&id); self.descriptors.insert(id,descriptor); }
    pub fn register_scoped_descriptor(&mut self, scope:&str, id:impl Into<String>, descriptor:Descriptor) {
        let id=id.into(); self.scopes.insert(id.clone(),scope.into()); self.register_descriptor(id,descriptor);
    }
    pub fn clear_scope(&mut self, scope:&str) {
        self.scopes.retain(|id,owned| {
            if owned==scope {self.descriptors.remove(id);self.errors.remove(id);false} else {true}
        });
    }
    pub fn register_error(&mut self, id:impl Into<String>, reason:String) { let id=id.into(); self.descriptors.remove(&id); self.errors.insert(id,reason); }
    pub fn register_scoped_error(&mut self, scope:&str, id:impl Into<String>, reason:String) {
        let id=id.into(); self.scopes.insert(id.clone(),scope.into()); self.register_error(id,reason);
    }
    /// Register encoded PNG/JPEG bytes. The returned ID includes pixels, dimensions, encoding and top-row origin.
    pub fn register_image(&mut self, bytes:&[u8]) -> Result<String,String> {
        let format=image::guess_format(bytes).map_err(|e| e.to_string())?;
        if !matches!(format,image::ImageFormat::Png|image::ImageFormat::Jpeg) { return Err("only PNG/JPEG image bytes are supported".into()); }
        let (width,height)=image::ImageReader::with_format(std::io::Cursor::new(bytes),format).into_dimensions().map_err(|e|e.to_string())?;
        if u64::from(width)*u64::from(height)>MAX_PIXELS {return Err("image exceeds 16 Mpx".into());}
        let source=image::load_from_memory_with_format(bytes,format).map_err(|e| e.to_string())?;
        if matches!(source,image::DynamicImage::ImageLuma16(_)|image::DynamicImage::ImageLumaA16(_)|image::DynamicImage::ImageRgb16(_)|image::DynamicImage::ImageRgba16(_)|image::DynamicImage::ImageRgb32F(_)|image::DynamicImage::ImageRgba32F(_)) {return Err("high precision image needs a float asset path (not yet supported)".into());}
        let rgba=source.to_rgba8().into_raw();
        self.register_pixels(width,height,rgba,false)
    }
    /// Future transport entry point: verify the immutable ID before publication.
    pub fn register_image_with_id(&mut self,expected:&str,bytes:&[u8])->Result<String,String>{
        let actual=self.register_image(bytes)?;
        if actual!=expected {self.images.remove(&actual);return Err(format!("image hash mismatch: expected {expected}, got {actual}"));}
        Ok(actual)
    }
    /// RGBA8 pixels. Set bottom_up for Blender-style rows; internally rows always start at the top.
    pub fn register_pixels(&mut self,width:u32,height:u32,mut rgba:Vec<u8>,bottom_up:bool)->Result<String,String>{
        if width==0 || height==0 || u64::from(width)*u64::from(height)>MAX_PIXELS || rgba.len()!=width as usize*height as usize*4 { return Err("invalid image dimensions or payload".into()); }
        if bottom_up { let row=width as usize*4; for y in 0..height as usize/2 { for x in 0..row { rgba.swap(y*row+x,(height as usize-1-y)*row+x); } } }
        let mut h=Sha256::new(); h.update(b"RGBA8/top-row/v1"); h.update(width.to_le_bytes()); h.update(height.to_le_bytes()); h.update(&rgba);
        let hash=format!("sha256:{:x}",h.finalize());
        self.tick+=1;
        self.images.insert(hash.clone(),(ImageAsset {width,height,rgba,hash:hash.clone()},self.tick));
        self.evict(&HashSet::from([hash.clone()]));
        Ok(hash)
    }
    pub fn register_graph(&mut self, graph:&Value) -> Vec<String> {
        let mut errors=Vec::new();
        if let Some(table)=graph.get("materials").and_then(Value::as_object) {
            for (name,value) in table { match Descriptor::parse(value) { Ok(d)=>self.register_descriptor(name.clone(),d),Err(e)=>{self.descriptors.remove(name);self.errors.insert(name.clone(),e.clone());errors.push(format!("{name}: {e}"));} } }
        }
        errors
    }
    /// Sidecar `<graph>.materials.json` maps slot names to the same descriptors; a string value is shorthand for a local PNG/JPEG path.
    pub fn register_sidecar(&mut self,graph_path:&Path)->Result<Vec<String>,String>{
        let path=PathBuf::from(format!("{}.materials.json",graph_path.display()));
        if !path.exists() { return Ok(Vec::new()); }
        let data=std::fs::read(&path).map_err(|e| e.to_string())?;
        let mut value:Value=serde_json::from_slice(&data).map_err(|e| e.to_string())?;
        if let Some(table)=value.as_object_mut() { for (name,d) in table { if let Some(p)=d.as_str() { *d=serde_json::json!({"version":1,"name":name,"textures":{"base_color":{"path":p}}}); } } }
        Ok(self.register_graph(&serde_json::json!({"materials":value})))
    }
    pub fn resolve(&mut self,slot:&str)->MaterialBinding {
        let id = if self.descriptors.contains_key(slot) {Some(slot.to_owned())} else {
            let mut matches=self.descriptors.iter().filter(|(id,d)|self.scopes.contains_key(*id) && d.name==slot).map(|(id,_)|id.clone());
            matches.next().filter(|_|matches.next().is_none())
        };
        let scope=id.as_ref().and_then(|id|self.scopes.get(id)).cloned();
        let Some(descriptor)=id.as_ref().and_then(|id|self.descriptors.get(id)).cloned() else { return MaterialBinding {slot:slot.into(),descriptor_id:self.errors.contains_key(slot).then(||slot.into()),scope:self.scopes.get(slot).cloned(),descriptor:None,image:None,state:self.errors.get(slot).map_or(State::Missing,|e|State::Unsupported(e.clone()))}; };
        let result=|image,state| MaterialBinding {slot:slot.into(),descriptor_id:id.clone(),scope:scope.clone(),descriptor:Some(descriptor.clone()),image,state};
        let Some(tex)=descriptor.textures.base_color.as_ref() else { return result(None,State::Ok); };
        let mut id=tex.image_id.clone();
        if id.as_ref().is_none_or(|id| !self.images.contains_key(id)) {
            if let Some(path)=&tex.path {
                let path=if path.is_absolute() {path.clone()} else {self.base_path.join(path)};
                match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b|self.register_image(&b)) {
                    Ok(hash)=>{ if id.as_ref().is_some_and(|expected| expected!=&hash) { return result(None,State::CheckerFallback("image hash mismatch".into())); } id=Some(hash); },
                    Err(e)=>return result(None,State::CheckerFallback(e)),
                }
            }
        }
        let image=id.and_then(|id| { self.tick+=1; self.images.get_mut(&id).map(|(asset,last)| {*last=self.tick;asset.clone()}) });
        let state=if image.is_some() {State::Ok} else {State::CheckerFallback("image asset missing".into())};
        result(image,state)
    }
    pub fn cpu_resident_bytes(&self)->usize { self.images.values().map(|(i,_)|i.resident_bytes()).sum() }
    pub fn evict(&mut self,pinned:&HashSet<String>) {
        while self.cpu_resident_bytes()>self.cpu_budget { let Some(id)=self.images.iter().filter(|(id,_)| !pinned.contains(*id) && !self.descriptors.values().any(|d|d.textures.base_color.as_ref().and_then(|t|t.image_id.as_ref())==Some(*id))).min_by_key(|(_,(_,last))|last).map(|(id,_)|id.clone()) else {break}; self.images.remove(&id); }
    }
}

pub fn srgb_to_linear(v:f32)->f32 { if v<=0.04045 {v/12.92} else {((v+0.055)/1.055).powf(2.4)} }
pub fn linear_to_srgb(v:f32)->f32 { if v<=0.0031308 {v*12.92} else {1.055*v.powf(1./2.4)-0.055} }

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn descriptor_and_cache() {
        let d=Descriptor::parse(&serde_json::json!({"version":1,"textures":{"base_color":{"image_id":"sha256:missing"}}})).unwrap();
        assert_eq!(d.base_color_factor,[1.;4]); assert!(Descriptor::parse(&serde_json::json!({"version":1,"alpha_mode":"BLEND"})).is_err());
        let mut r=Registry::new("."); r.register_descriptor("a",d); assert!(matches!(r.resolve("a").state,State::CheckerFallback(_)));
        assert_eq!(r.register_graph(&serde_json::json!({"materials":{"bad":{"version":1,"alpha_mode":"BLEND"}}})).len(),1);
        assert!(matches!(r.resolve("bad").state,State::Unsupported(_)));
        let hash=r.register_pixels(1,2,vec![255,0,0,255,0,0,255,255],true).unwrap();
        assert_eq!(&r.images[&hash].0.rgba[..4],&[0,0,255,255]);
        assert_eq!(hash,r.register_pixels(1,2,vec![0,0,255,255,255,0,0,255],false).unwrap());
        r.set_budgets(0,0); assert_eq!(r.cpu_resident_bytes(),0);
    }
    #[test] fn color_transfer_round_trip() { for v in [0.,0.02,0.18,0.5,1.] { assert!((srgb_to_linear(linear_to_srgb(v))-v).abs()<1e-5); } }
    #[test] fn encoded_png_and_verified_id() {
        let bytes=include_bytes!("test_data/textured_cube.png"); let mut r=Registry::new(".");
        let hash=r.register_image(bytes).unwrap();assert_eq!((r.images[&hash].0.width,r.images[&hash].0.height),(64,64));
        assert_eq!(r.register_image_with_id(&hash,bytes).unwrap(),hash);
        assert!(r.register_image_with_id("sha256:wrong",bytes).is_err());
    }
    #[test] fn clip_approximation_parses_as_clamp() {
        let d=Descriptor::parse(&serde_json::json!({"version":1,"textures":{"base_color":{"image_id":"sha256:test",
            "sampler":{"min":"linear","mag":"linear","wrap_u":"clamp","wrap_v":"clamp"}}}})).unwrap();
        let sampler=&d.textures.base_color.unwrap().sampler;
        assert_eq!(sampler.wrap_u,"clamp"); assert_eq!(sampler.wrap_v,"clamp");
    }
    #[test] fn unique_peer_material_name_fallback_is_unambiguous() {
        let mut r=Registry::new(".");
        let d=Descriptor::parse(&serde_json::json!({"version":1,"name":"imagepaste-261017-142014"})).unwrap();
        r.register_scoped_descriptor("peer1:instance:a","peer1:blender:a:0",d.clone());
        let found=r.resolve("imagepaste-261017-142014");
        assert_eq!(found.descriptor_id.as_deref(),Some("peer1:blender:a:0"));
        assert_eq!(found.scope.as_deref(),Some("peer1:instance:a"));
        r.register_scoped_descriptor("peer2:instance:b","peer2:blender:b:0",d);
        assert!(matches!(r.resolve("imagepaste-261017-142014").state,State::Missing));
        assert_eq!(r.resolve("peer1:blender:a:0").descriptor_id.as_deref(),Some("peer1:blender:a:0"));
    }
}
