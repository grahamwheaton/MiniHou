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
pub struct MaterialBinding { pub slot:String, pub descriptor:Option<Descriptor>, pub image:Option<ImageAsset>, pub state:State }

pub struct Registry {
    descriptors:HashMap<String,Descriptor>, images:HashMap<String,(ImageAsset,u64)>,
    tick:u64, cpu_budget:usize, pub gpu_budget:usize, base_path:PathBuf,
}
impl Registry {
    pub fn new(base_path:impl Into<PathBuf>) -> Self { Self { descriptors:HashMap::new(), images:HashMap::new(), tick:0, cpu_budget:CPU_BUDGET, gpu_budget:GPU_BUDGET, base_path:base_path.into() } }
    pub fn set_budgets(&mut self, cpu:usize, gpu:usize) { self.cpu_budget=cpu; self.gpu_budget=gpu; self.evict(&HashSet::new()); }
    pub fn register_descriptor(&mut self, id:impl Into<String>, descriptor:Descriptor) { self.descriptors.insert(id.into(),descriptor); }
    /// Register encoded PNG/JPEG bytes. The returned ID includes pixels, dimensions, encoding and top-row origin.
    pub fn register_image(&mut self, bytes:&[u8]) -> Result<String,String> {
        let format=image::guess_format(bytes).map_err(|e| e.to_string())?;
        if !matches!(format,image::ImageFormat::Png|image::ImageFormat::Jpeg) { return Err("only PNG/JPEG image bytes are supported".into()); }
        let source=image::load_from_memory_with_format(bytes,format).map_err(|e| e.to_string())?;
        let (width,height)=(source.width(),source.height());
        if u64::from(width)*u64::from(height)>MAX_PIXELS { return Err("image exceeds 16 Mpx".into()); }
        let rgba=source.to_rgba8().into_raw();
        self.register_pixels(width,height,rgba,false)
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
            for (name,value) in table { match Descriptor::parse(value) { Ok(d)=>self.register_descriptor(name.clone(),d),Err(e)=>errors.push(format!("{name}: {e}")) } }
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
        let Some(descriptor)=self.descriptors.get(slot).cloned() else { return MaterialBinding {slot:slot.into(),descriptor:None,image:None,state:State::Missing}; };
        let Some(tex)=descriptor.textures.base_color.as_ref() else { return MaterialBinding {slot:slot.into(),descriptor:Some(descriptor),image:None,state:State::Ok}; };
        let mut id=tex.image_id.clone();
        if id.as_ref().is_none_or(|id| !self.images.contains_key(id)) {
            if let Some(path)=&tex.path {
                let path=if path.is_absolute() {path.clone()} else {self.base_path.join(path)};
                match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b|self.register_image(&b)) {
                    Ok(hash)=>{ if id.as_ref().is_some_and(|expected| expected!=&hash) { return MaterialBinding {slot:slot.into(),descriptor:Some(descriptor),image:None,state:State::CheckerFallback("image hash mismatch".into())}; } id=Some(hash); },
                    Err(e)=>return MaterialBinding {slot:slot.into(),descriptor:Some(descriptor),image:None,state:State::CheckerFallback(e)},
                }
            }
        }
        let image=id.and_then(|id| { self.tick+=1; self.images.get_mut(&id).map(|(asset,last)| {*last=self.tick;asset.clone()}) });
        let state=if image.is_some() {State::Ok} else {State::CheckerFallback("image asset missing".into())};
        MaterialBinding {slot:slot.into(),descriptor:Some(descriptor),image,state}
    }
    pub fn cpu_resident_bytes(&self)->usize { self.images.values().map(|(i,_)|i.resident_bytes()).sum() }
    pub fn evict(&mut self,pinned:&HashSet<String>) {
        while self.cpu_resident_bytes()>self.cpu_budget { let Some(id)=self.images.iter().filter(|(id,_)| !pinned.contains(*id)).min_by_key(|(_,(_,last))|last).map(|(id,_)|id.clone()) else {break}; self.images.remove(&id); }
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
        let hash=r.register_pixels(1,2,vec![255,0,0,255,0,0,255,255],true).unwrap();
        assert_eq!(&r.images[&hash].0.rgba[..4],&[0,0,255,255]);
        assert_eq!(hash,r.register_pixels(1,2,vec![0,0,255,255,255,0,0,255],false).unwrap());
        r.set_budgets(0,0); assert_eq!(r.cpu_resident_bytes(),0);
    }
    #[test] fn color_transfer_round_trip() { for v in [0.,0.02,0.18,0.5,1.] { assert!((srgb_to_linear(linear_to_srgb(v))-v).abs()<1e-5); } }
}
