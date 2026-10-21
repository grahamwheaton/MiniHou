//! Bounded, connection-independent image transfer and manifest revision state.
use std::collections::{BTreeMap, HashMap, VecDeque};
use serde_json::Value;
use sha2::{Digest, Sha256};
use minihou_core::limits::Limits;
use crate::materials::{Descriptor, Registry};

pub const CHUNK: usize = 1024 * 1024;

struct Transfer { hash:String, total:usize, bytes:Vec<u8>, ranges:BTreeMap<usize,usize> }
#[derive(Default)]
pub struct Service {
    limits: Limits,
    transfers:HashMap<String,Transfer>,
    retained:HashMap<String,Vec<u8>>,
    order:VecDeque<String>,
    pub revisions:HashMap<String,u64>,
    pub manifests:HashMap<String,Value>,
}
impl Service {
    pub fn has(&self, hash:&str)->bool { self.retained.contains_key(hash) }
    pub fn bytes(&self,hash:&str)->Option<&[u8]> {self.retained.get(hash).map(Vec::as_slice)}
    pub fn missing(&self, hashes:&[String])->Vec<String> { hashes.iter().filter(|h| !self.has(h)).cloned().collect() }
    pub fn manifest(&mut self,scope:&str,revision:u64,value:Value)->bool {
        if self.revisions.get(scope).is_some_and(|old| *old>=revision) {return false;}
        self.revisions.insert(scope.into(),revision); self.manifests.insert(scope.into(),value); true
    }
    pub fn remove_peer(&mut self,prefix:&str) {
        self.revisions.retain(|scope,_|!scope.starts_with(prefix));
        self.manifests.retain(|scope,_|!scope.starts_with(prefix));
        self.transfers.retain(|id,_|!id.starts_with(prefix));
    }
    pub fn begin(&mut self,id:&str,hash:&str,total:usize)->Result<(),String> {
        if !hash.starts_with("sha256:") || hash.len()!=71 || !hash[7..].bytes().all(|b|b.is_ascii_hexdigit()) {return Err("invalid image hash".into());}
        if total==0 || total>self.limits.image_decoded_bytes as usize {return Err("image payload exceeds limit".into());}
        let in_flight:usize=self.transfers.values().map(|t|t.total).sum();
        if in_flight.checked_add(total).is_none_or(|n|n>self.limits.image_in_flight_bytes as usize) {return Err("image transfer budget exceeded".into());}
        if self.transfers.contains_key(id) {return Err("duplicate transfer ID".into());}
        self.transfers.insert(id.into(),Transfer {hash:hash.into(),total,bytes:vec![0;total],ranges:BTreeMap::new()}); Ok(())
    }
    pub fn chunk(&mut self,id:&str,offset:usize,bytes:&[u8])->Result<(),String> {
        let t=self.transfers.get_mut(id).ok_or("unknown transfer")?;
        let end=offset.checked_add(bytes.len()).ok_or("offset overflow")?;
        if bytes.is_empty() || bytes.len()>CHUNK || end>t.total {return Err("invalid chunk range".into());}
        if t.ranges.range(..end).next_back().is_some_and(|(_,e)|*e>offset) || t.ranges.range(offset..).next().is_some_and(|(s,_)|*s<end) {return Err("overlapping chunk".into());}
        t.bytes[offset..end].copy_from_slice(bytes); t.ranges.insert(offset,end); Ok(())
    }
    pub fn end(&mut self,id:&str)->Result<String,String> {
        let t=self.transfers.remove(id).ok_or("unknown transfer")?;
        let mut covered=0; for (&start,&end) in &t.ranges {if start!=covered {return Err("incomplete image transfer".into());} covered=end;}
        if covered!=t.total {return Err("incomplete image transfer".into());}
        let hash=format!("sha256:{:x}",Sha256::digest(&t.bytes));
        if hash!=t.hash {return Err("image hash mismatch".into());}
        let format=image::guess_format(&t.bytes).map_err(|e|e.to_string())?;
        if !matches!(format,image::ImageFormat::Png|image::ImageFormat::Jpeg) {return Err("only PNG/JPEG assets are supported".into());}
        let (w,h)=image::ImageReader::with_format(std::io::Cursor::new(&t.bytes),format).into_dimensions().map_err(|e|e.to_string())?;
        if w==0 || h==0 || u64::from(w)*u64::from(h)>self.limits.image_max_pixels || u64::from(w)*u64::from(h)*4>self.limits.image_decoded_bytes {return Err("decoded image exceeds limit".into());}
        let decoded=image::load_from_memory_with_format(&t.bytes,format).map_err(|e|e.to_string())?;
        if matches!(decoded,image::DynamicImage::ImageLuma16(_)|image::DynamicImage::ImageLumaA16(_)|image::DynamicImage::ImageRgb16(_)|image::DynamicImage::ImageRgba16(_)|image::DynamicImage::ImageRgb32F(_)|image::DynamicImage::ImageRgba32F(_)) {return Err("high precision image unsupported".into());}
        if !self.retained.contains_key(&hash) {self.order.push_back(hash.clone());}
        self.retained.insert(hash.clone(),t.bytes);
        let mut bytes:usize=self.retained.values().map(Vec::len).sum();
        while bytes>self.limits.image_in_flight_bytes as usize {if let Some(old)=self.order.pop_front(){if let Some(value)=self.retained.remove(&old){bytes-=value.len();}}else{break;}}
        Ok(hash)
    }
    pub fn cancel(&mut self,id:&str) {self.transfers.remove(id);}
    pub fn publish(&self,hash:&str,registry:&mut Registry)->Result<String,String> {
        let bytes=self.retained.get(hash).ok_or("image not retained")?;
        registry.register_image(bytes)
    }
    pub fn bind(&self,scope:&str,registry:&mut Registry)->Result<(),String> {
        let manifest=self.manifests.get(scope).ok_or("missing manifest")?;
        let descriptors=manifest["descriptors"].as_object().ok_or("missing descriptors")?;
        registry.clear_scope(scope);
        for (id,value) in descriptors {
            let mut d=match Descriptor::parse(value) { Ok(d)=>d, Err(reason)=>{registry.register_scoped_error(scope,id.clone(),reason);continue;} };
            if let Some(t)=d.textures.base_color.as_mut() { if let Some(hash)=t.image_id.as_ref() {if self.has(hash) {t.image_id=Some(self.publish(hash,registry)?);} } }
            registry.register_scoped_descriptor(scope,id.clone(),d);
        }
        Ok(())
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn default_image_budgets_match_previous_values() {
        let service = Service::default();
        assert_eq!(service.limits.image_in_flight_bytes, 512 * 1024 * 1024);
        assert_eq!(service.limits.image_decoded_bytes, 256 * 1024 * 1024);
        assert_eq!(service.limits.image_max_pixels, 16_000_000);
    }
    #[test] fn coverage_hash_and_revision() {
        let mut s=Service::default();
        let mut data=Vec::new();image::DynamicImage::new_rgba8(2,2).write_to(&mut std::io::Cursor::new(&mut data),image::ImageFormat::Png).unwrap();
        let hash=format!("sha256:{:x}",Sha256::digest(&data));let middle=data.len()/2;
        s.begin("a",&hash,data.len()).unwrap(); s.chunk("a",middle,&data[middle..]).unwrap();
        assert!(s.chunk("a",middle+1,b"x").is_err()); s.chunk("a",0,&data[..middle]).unwrap();
        assert_eq!(s.end("a").unwrap(),hash); assert!(s.has(&hash));
        assert!(s.manifest("object:C",2,serde_json::json!({"descriptors":{}})));
        assert!(!s.manifest("object:C",1,serde_json::json!({"descriptors":{}})));
        s.begin("b",&hash,6).unwrap();s.chunk("b",0,b"xxxxxx").unwrap();assert!(s.end("b").is_err());
        s.begin("c",&hash,6).unwrap();s.chunk("c",0,b"ab").unwrap();assert!(s.end("c").is_err());
        assert!(s.begin("too-big",&hash,s.limits.image_decoded_bytes as usize+1).is_err());
        s.begin("range",&hash,6).unwrap();
        assert!(s.chunk("range",usize::MAX,b"x").is_err());
        assert!(s.chunk("range",5,b"xx").is_err());
        assert!(s.chunk("range",0,&vec![0;CHUNK+1]).is_err());
    }
    #[test] fn bad_slot_does_not_poison_supported_binding() {
        let mut service=Service::default();
        service.manifest("peer1:instance:test",1,serde_json::json!({"descriptors":{
            "peer1:blender:test:0":{"version":1,"alpha_mode":"BLEND"},
            "peer1:blender:test:1":{"version":1,"textures":{"base_color":{"image_id":"sha256:missing"}}}
        },"bindings":["peer1:blender:test:0","peer1:blender:test:1"]}));
        let mut registry=Registry::new(".");
        service.bind("peer1:instance:test",&mut registry).unwrap();
        assert!(matches!(registry.resolve("peer1:blender:test:0").state,crate::materials::State::Unsupported(_)));
        assert!(registry.resolve("peer1:blender:test:1").descriptor.unwrap().textures.base_color.is_some());
    }
}
