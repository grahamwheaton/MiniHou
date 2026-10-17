//! MHL1 TCP framing and per-subscriber geometry diffs (Python link protocol v1).
use std::{collections::BTreeMap, io::{self, Read}, sync::Arc};
use minihou_core::{Geometry, geometry::AttrData};
use serde_json::{json, Value};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_HEADER: usize = 64 * 1024 * 1024;
pub const MAX_BLOB: u64 = 8 * 1024 * 1024 * 1024;
const TOPO: [&str; 3] = ["topo/vertex_point", "topo/prim_offsets", "topo/prim_closed"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Array {
    pub dtype: String,
    pub shape: Vec<usize>,
    pub bytes: Arc<Vec<u8>>,
}
pub type Arrays = BTreeMap<String, Array>;
fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }
fn width(dtype: &str) -> io::Result<usize> {
    match dtype { "<f4" | "<i4" => Ok(4), "<f8" | "<i8" => Ok(8), "|u1" | "|b1" => Ok(1), _ => Err(invalid("unsupported dtype")) }
}
fn array_len(dtype: &str, shape: &[usize]) -> io::Result<usize> {
    shape.iter().try_fold(width(dtype)?, |n, &s| n.checked_mul(s).ok_or_else(|| invalid("array size overflow")))
}

pub fn encode_message(header: &Value, arrays: &Arrays) -> io::Result<Vec<u8>> {
    if !header.is_object() || !header["type"].is_string() { return Err(invalid("header needs a type")); }
    let mut header = header.clone();
    header.as_object_mut().unwrap().remove("arrays");
    let mut blob = Vec::new();
    let mut descriptors = Vec::new();
    for (key, a) in arrays {
        if array_len(&a.dtype, &a.shape)? != a.bytes.len() { return Err(invalid("array shape/size mismatch")); }
        descriptors.push(json!({"key":key,"dtype":a.dtype,"shape":a.shape,"offset":blob.len(),"nbytes":a.bytes.len()}));
        blob.extend_from_slice(&a.bytes);
        blob.resize(blob.len() + (8 - blob.len() % 8) % 8, 0);
    }
    if !descriptors.is_empty() { header["arrays"] = json!(descriptors); }
    let mut hb = serde_json::to_vec(&header)?;
    hb.resize(hb.len() + (8 - hb.len() % 8) % 8, b' ');
    if hb.len() > MAX_HEADER || blob.len() as u64 > MAX_BLOB { return Err(invalid("frame too large")); }
    let mut frame = Vec::with_capacity(16 + hb.len() + blob.len());
    frame.extend_from_slice(b"MHL1");
    frame.extend_from_slice(&(hb.len() as u32).to_le_bytes());
    frame.extend_from_slice(&(blob.len() as u64).to_le_bytes());
    frame.extend(hb); frame.extend(blob);
    Ok(frame)
}

/// `limit` bounds untrusted allocations; the server only accepts small control frames.
pub fn read_message(reader: &mut impl Read, limit: usize) -> io::Result<(Value, Arrays)> {
    let mut prefix = [0; 16]; reader.read_exact(&mut prefix)?;
    if &prefix[..4] != b"MHL1" { return Err(invalid("bad magic")); }
    let hlen = u32::from_le_bytes(prefix[4..8].try_into().unwrap()) as usize;
    let blen = u64::from_le_bytes(prefix[8..16].try_into().unwrap());
    if hlen == 0 || hlen > MAX_HEADER || hlen % 8 != 0 || blen > MAX_BLOB { return Err(invalid("unreasonable frame sizes")); }
    let total = u64::try_from(hlen).unwrap().checked_add(blen).ok_or_else(|| invalid("frame overflow"))?;
    if total > limit as u64 { return Err(invalid("frame exceeds receive limit")); }
    let mut payload = vec![0; hlen]; reader.read_exact(&mut payload)?;
    let header:Value=serde_json::from_slice(&payload)?;
    let kind=header["type"].as_str().unwrap_or("");
    if kind=="image_chunk" && blen>1024*1024+7 {return Err(invalid("image chunk exceeds 1 MiB"));}
    if matches!(kind,"image_begin"|"image_end"|"image_ack"|"image_request"|"image_error"|"material_manifest") && blen!=0 {return Err(invalid("image control frame has a blob"));}
    payload.resize(total as usize,0);
    reader.read_exact(&mut payload[hlen..])?;
    decode_payload(&payload, hlen)
}

pub fn decode_payload(payload: &[u8], hlen: usize) -> io::Result<(Value, Arrays)> {
    if hlen > payload.len() || hlen % 8 != 0 { return Err(invalid("bad header length")); }
    let header: Value = serde_json::from_slice(&payload[..hlen])?;
    if !header.is_object() || !header["type"].is_string() { return Err(invalid("header needs a type")); }
    let blob = &payload[hlen..]; let mut arrays = Arrays::new();
    if let Some(descs) = header.get("arrays") {
        for d in descs.as_array().ok_or_else(|| invalid("arrays must be a list"))? {
            let key = d["key"].as_str().ok_or_else(|| invalid("missing array key"))?;
            let dtype = d["dtype"].as_str().ok_or_else(|| invalid("missing dtype"))?;
            let number = |v: &Value| -> io::Result<usize> { v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| invalid("invalid array dimension/offset")) };
            let shape: Vec<usize> = d["shape"].as_array().ok_or_else(|| invalid("missing shape"))?.iter().map(number).collect::<io::Result<_>>()?;
            let offset = number(&d["offset"])?; let nbytes = number(&d["nbytes"])?;
            let end = offset.checked_add(nbytes).ok_or_else(|| invalid("array offset overflow"))?;
            if offset % 8 != 0 || end > blob.len() || array_len(dtype, &shape)? != nbytes { return Err(invalid("array lies outside blob or size mismatch")); }
            if arrays.insert(key.into(), Array { dtype:dtype.into(), shape, bytes:Arc::new(blob[offset..end].to_vec()) }).is_some() { return Err(invalid("duplicate array key")); }
        }
    }
    Ok((header, arrays))
}

fn i32_array(v: &[i32]) -> Array {
    Array { dtype:"<i4".into(), shape:vec![v.len()], bytes:Arc::new(v.iter().flat_map(|x| x.to_le_bytes()).collect()) }
}
pub fn geometry_arrays(geo: &Geometry) -> Arrays {
    let mut arrays = Arrays::new();
    for (domain, table) in [("point", &geo.point), ("vertex", &geo.vertex), ("prim", &geo.prim)] {
        for (name, attr) in table {
            let (dtype, bytes) = match &attr.data {
                AttrData::F32(v) if domain == "vertex" && matches!(attr.width, 2 | 3)
                    && !matches!(name.as_str(), "N" | "Cd" | "P") =>
                    ("<f4", v.iter().flat_map(|x| (if x.is_finite() { *x } else { 0.0 }).to_le_bytes()).collect()),
                AttrData::F32(v) => ("<f4", v.iter().flat_map(|x| x.to_le_bytes()).collect()),
                AttrData::I32(v) => ("<i4", v.iter().flat_map(|x| x.to_le_bytes()).collect()),
            };
            let shape = if attr.width == 1 { vec![attr.rows()] } else { vec![attr.rows(), attr.width] };
            arrays.insert(format!("{domain}/{name}"), Array { dtype:dtype.into(), shape, bytes:Arc::new(bytes) });
        }
    }
    // Empty geometry must still provide P for the addon's Mirror.
    arrays.entry("point/P".into()).or_insert_with(|| Array { dtype:"<f4".into(), shape:vec![0,3], bytes:Arc::new(Vec::new()) });
    arrays.insert(TOPO[0].into(), i32_array(&geo.topology.vertex_point));
    arrays.insert(TOPO[1].into(), i32_array(&geo.topology.prim_offsets));
    if geo.topology.has_open() {
        arrays.insert(TOPO[2].into(), Array { dtype:"|u1".into(), shape:vec![geo.nprims()], bytes:Arc::new(geo.topology.closed_mask().iter().map(|&b| u8::from(b)).collect()) });
    }
    arrays
}

pub fn attr_array(a:&minihou_core::Attr)->Array {
    let (dtype,bytes)=match &a.data {
        AttrData::F32(v)=>("<f4",v.iter().flat_map(|x|x.to_le_bytes()).collect()),
        AttrData::I32(v)=>("<i4",v.iter().flat_map(|x|x.to_le_bytes()).collect()),
    };
    Array{dtype:dtype.into(),shape:if a.width==1{vec![a.rows()]}else{vec![a.rows(),a.width]},bytes:Arc::new(bytes)}
}
fn decode_attr(a:&Array)->io::Result<minihou_core::Attr>{
    use minihou_core::Attr;
    if !(1..=2).contains(&a.shape.len()) || a.shape.get(1)==Some(&0) || array_len(&a.dtype,&a.shape)?!=a.bytes.len(){return Err(invalid("attribute must be a scalar or vector column"));}
    let w=a.shape.get(1).copied().unwrap_or(1);
    Ok(match a.dtype.as_str(){
        "<f4"=>Attr::f32(w,a.bytes.chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())).collect()),
        "<i4"=>Attr::i32(w,a.bytes.chunks_exact(4).map(|b|i32::from_le_bytes(b.try_into().unwrap())).collect()),
        // Boolean/byte attributes cannot retain their dtype in this core. Refuse them
        // rather than silently change attribute semantics; topology flags are handled separately.
        _=>return Err(invalid("attribute dtype is not supported by the Rust core (requires float32 or int32)")),
    })
}
pub fn decode_object(h:&Value,arrays:&Arrays)->io::Result<Geometry>{
    let pa=arrays.get("point/P").ok_or_else(||invalid("missing point/P"))?;
    let p=decode_attr(pa)?;if p.width!=3{return Err(invalid("P must have shape (n,3)"));}
    let AttrData::F32(p)=p.data else{return Err(invalid("P must be float32"));};
    let indices=|key:&str,default:Vec<i32>|->io::Result<Vec<i32>>{match arrays.get(key){None=>Ok(default),Some(a)=>{
        if a.shape.len()!=1{return Err(invalid("topology must be one dimensional"));}
        match decode_attr(a)?.data{AttrData::I32(v)=>Ok(v),_=>Err(invalid("topology indices must be int32"))}
    }}};
    let closed=match arrays.get(TOPO[2]).filter(|a|!a.bytes.is_empty()){
        None=>None,Some(a)=>{if a.shape.len()!=1||!["|u1","|b1"].contains(&a.dtype.as_str()){return Err(invalid("closed must be a byte column"));}Some(a.bytes.iter().map(|&x|x!=0).collect())}
    };
    let mut geo=Geometry::from_arrays(p,indices(TOPO[0],vec![])?,indices(TOPO[1],vec![0])?,closed).map_err(io::Error::other)?;
    for (key,a) in arrays {
        if key=="point/P"||TOPO.contains(&key.as_str()){continue;}
        let (dom,name)=key.split_once('/').ok_or_else(||invalid("bad attribute key"))?;
        if name.is_empty(){return Err(invalid("empty attribute name"));}
        let table=match dom{"point"=>&mut geo.point,"vertex"=>&mut geo.vertex,"prim"=>&mut geo.prim,_=>return Err(invalid("bad attribute domain"))};
        table.insert(name.into(),Arc::new(decode_attr(a)?));
    }
    if let Some(mats)=h["materials"].as_array().filter(|a|!a.is_empty()){
        geo.detail.insert("materials".into(),json!(mats.iter().take(4096).map(|m|m.as_str().unwrap_or("").chars().take(63).collect::<String>()).collect::<Vec<_>>()));
    }
    // Older and manifest-free peers may send null or empty bindings. Treat those as absent.
    if let Some(bindings)=h.get("material_bindings").filter(|v| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty)) {
        geo.detail.insert("material_bindings".into(),bindings.clone());
    }
    geo.validate().map_err(io::Error::other)?;Ok(geo)
}
/// Decode on the UI thread into a transient worker edit, without touching document history.
pub fn external_edit(h:&Value,arrays:&Arrays)->io::Result<crate::Edit>{
    use minihou_core::nodes::blender_input::ExternalInput;
    let typ=h["type"].as_str().unwrap_or("");let image=typ.starts_with("image_");
    let name=h[if image{"image"}else{"object"}].as_str().ok_or_else(||invalid("missing input name"))?;
    let input=match typ{
        "object_missing"|"image_missing"=>ExternalInput::Missing(h["reason"].as_str().unwrap_or(if image{"no such image"}else{"not found"}).into()),
        "object_geo"=>{
            let matrix=if h["matrix"].is_null(){None}else{let values=h["matrix"].as_array().filter(|v|v.len()==16).ok_or_else(||invalid("matrix must have 16 values"))?;let mut m=[0.;16];for(i,v)in values.iter().enumerate(){m[i]=v.as_f64().filter(|x|x.is_finite()).ok_or_else(||invalid("invalid matrix value"))?;}Some(m)};
            ExternalInput::Object{geo:Arc::new(decode_object(h,arrays)?),matrix,info:h["info"].clone()}
        },
        "image_data"=>{
            let w=h["width"].as_u64().and_then(|x|usize::try_from(x).ok()).ok_or_else(||invalid("bad image width"))?;
            let he=h["height"].as_u64().and_then(|x|usize::try_from(x).ok()).ok_or_else(||invalid("bad image height"))?;
            let pixel_bytes=w.checked_mul(he).and_then(|n|n.checked_mul(16)).ok_or_else(||invalid("image size overflow"))?;
            if w==0||he==0{return Err(invalid("image dimensions must be positive"));}
            let a=arrays.get("pixels").ok_or_else(||invalid("missing pixels"))?;
            if a.dtype!="<f4"||a.bytes.len()!=pixel_bytes{return Err(invalid("pixels must be width * height * 4 float32 values"));}
            ExternalInput::Image{pixels:Arc::new(a.bytes.chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())).collect()),width:w,height:he,meta:h.clone()}
        },_=>return Err(invalid("not an external input message")),
    };
    Ok(crate::Edit::External{name:format!("{}:{name}",if image{"image"}else{"object"}),input})
}
pub fn watch_objects(doc:&Value)->Value{
    let mut objects=std::collections::BTreeSet::new();let mut images=std::collections::BTreeSet::new();
    if let Some(nodes)=doc["nodes"].as_object(){for n in nodes.values(){
        match n["type"].as_str(){Some("blender_object")=>{if let Some(s)=n["params"]["object"].as_str().filter(|s|!s.is_empty()&&!["MiniHou_Output","MiniHou_Proxy"].contains(s)){objects.insert(s.to_string());}},Some("blender_image")=>{if let Some(s)=n["params"]["image"].as_str().filter(|s|!s.is_empty()){images.insert(s.to_string());}},_=>{}}
    }}json!({"type":"watch_objects","objects":objects,"images":images})
}
fn stable_manifest(mut header:Value,arrays:Arrays)->(Value,Arrays){
    use std::hash::{Hash,Hasher};
    header.as_object_mut().unwrap().remove("key");
    let mut hash=std::collections::hash_map::DefaultHasher::new();header.to_string().hash(&mut hash);
    for (name,a) in &arrays{name.hash(&mut hash);a.dtype.hash(&mut hash);a.shape.hash(&mut hash);a.bytes.hash(&mut hash);}
    header["key"]=json!(format!("{:016x}",hash.finish()));(header,arrays)
}
pub fn sink_manifest(typ:&str,id:&str,p:&minihou_core::params::Params,g:&Geometry,key:&str)->crate::Result<(Value,Arrays)>{
    if typ=="image_out"{
        let(w,h,mut pixels)=minihou_core::nodes::image_nodes::pixels(g,p)?;let encoding=p.e("encoding",&["linear","srgb","data"])?;
        if encoding=="srgb"{for v in pixels.chunks_exact_mut(4){for x in &mut v[..3]{*x=minihou_core::nodes::image_nodes::linear_to_srgb(*x);}}}
        let image=p.s("image")?;let image=if image.trim().is_empty(){"MiniHou"}else{image.trim()};
        let mut arrays=Arrays::new();arrays.insert("pixels".into(),Array{dtype:"<f4".into(),shape:vec![h,w,4],bytes:Arc::new(pixels.iter().flat_map(|x|x.to_le_bytes()).collect())});
        return Ok(stable_manifest(json!({"type":"scene_objects","node":id,"key":key,"mode":"image","prefix":"","collection":"","sources":[],"id":[],"image":image.chars().take(63).collect::<String>(),"width":w,"height":h,"encoding":encoding}),arrays));
    }
    let m=minihou_core::nodes::scene_objects::build_manifest(id,p,g,key)?;
    let arrays=m.arrays.iter().map(|(k,a)|{
        let mut wire=attr_array(a);
        if k=="cyclic"{if let AttrData::I32(v)=&a.data{wire.dtype="|u1".into();wire.bytes=Arc::new(v.iter().map(|&x|x as u8).collect());}}
        (k.clone(),wire)
    }).collect();Ok(stable_manifest(m.header,arrays))
}

#[derive(Default)]
pub struct DiffState { sent: Option<Arrays> }
impl DiffState {
    pub fn reset(&mut self) { self.sent = None; }
    /// Compare with what this subscriber was last sent, not the previous cook.
    pub fn build(&mut self, current: &Arrays, geo: &Geometry, seq: u64, cook: &Value) -> (Value, Arrays) {
        let full = self.sent.is_none();
        let changed = |key: &str| self.sent.as_ref().and_then(|s| s.get(key)) != current.get(key);
        let point_count_changed = self.sent.as_ref().and_then(|s|s.get("point/P")).map(|a|a.shape.first()) != current.get("point/P").map(|a|a.shape.first());
        let topology_changed = full || point_count_changed || TOPO.iter().any(|key| changed(key));
        let mut send = Arrays::new(); let mut manifest = serde_json::Map::new();
        for (key, a) in current {
            if key.starts_with("topo/") {
                if topology_changed { send.insert(key.clone(), a.clone()); }
            } else {
                manifest.insert(key.clone(), json!({"dtype":a.dtype,"shape":a.shape}));
                if full || changed(key) { send.insert(key.clone(), a.clone()); }
            }
        }
        if topology_changed && !current.contains_key(TOPO[2]) && self.sent.as_ref().is_some_and(|s| s.contains_key(TOPO[2])) {
            send.insert(TOPO[2].into(), Array { dtype:"|u1".into(), shape:vec![0], bytes:Arc::new(Vec::new()) });
        }
        self.sent = Some(current.clone());
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64();
        (json!({"type":"geo","seq":seq,"full":full,"counts":{"point":geo.npoints,"vertex":geo.nvertices(),"prim":geo.nprims()},"topology_changed":topology_changed,"manifest":manifest,"detail":geo.detail,"cook":cook,"t_sent":t}), send)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn object_input_roundtrip_and_validation() {
        let mut g=Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.],vec![0,1],vec![0,2],Some(vec![false])).unwrap();
        g.point.insert("radius".into(),Arc::new(minihou_core::Attr::f32(1,vec![1.,2.])));
        let arrays=geometry_arrays(&g);let h=json!({"type":"object_geo","object":"Curve","materials":["Paint"],"matrix":null,"info":{"components":["curves"]}});
        let decoded=decode_object(&h,&arrays).unwrap();assert_eq!(decoded.p(),g.p());assert_eq!(decoded.topology,g.topology);assert_eq!(decoded.detail["materials"],json!(["Paint"]));
        assert!(matches!(external_edit(&h,&arrays).unwrap(),crate::Edit::External{name,..} if name=="object:Curve"));
        let mut bad=arrays.clone();bad.insert("point/radius".into(),attr_array(&minihou_core::Attr::f32(1,vec![1.])));assert!(decode_object(&h,&bad).is_err());
        let mut bad=h.clone();bad["matrix"]=json!([1,2,3]);
        assert!(external_edit(&bad,&arrays).is_err());
    }
    #[test]
    fn absent_or_empty_material_bindings_do_not_poison_modifier_geometry() {
        let g=Geometry::points_only(vec![0.,0.,0.]).unwrap();
        let arrays=geometry_arrays(&g);
        for bindings in [Value::Null, json!([])] {
            let h=json!({"materials":[],"material_bindings":bindings});
            let decoded=decode_object(&h,&arrays).unwrap();
            assert!(!decoded.detail.contains_key("material_bindings"));
        }
        let h=json!({"materials":["Paint"],"material_bindings":{"version":1,"slots":["peer:Paint"]}});
        assert_eq!(decode_object(&h,&arrays).unwrap().detail["material_bindings"],h["material_bindings"]);
        let invalid=json!({"materials":["Paint"],"material_bindings":["peer:Paint"]});
        assert!(decode_object(&invalid,&arrays).is_err());
    }
    #[test]
    fn images_keep_bottom_row_order_and_encode_rgb_only() {
        let g=Geometry::points_only(vec![0.,0.,0.,1.,1.,0.]).unwrap();
        let mut g=g;g.detail.insert("grid_columns".into(),json!(2));g.detail.insert("grid_rows".into(),json!(1));g.point.insert("Cd".into(),Arc::new(minihou_core::Attr::f32(4,vec![0.,0.5,1.,0.25,1.,0.,0.,0.75])));
        let p=minihou_core::params::Params::resolve(minihou_core::nodes::image_nodes::IMAGE_OUT.params,json!({"encoding":"srgb"}).as_object()).unwrap();
        let (h,a)=sink_manifest("image_out","image",&p,&g,"k").unwrap();assert_eq!(h["mode"],"image");assert_eq!(a["pixels"].shape,vec![1,2,4]);
        let edit=external_edit(&json!({"type":"image_data","image":"MiniHou","width":2,"height":1}),&a).unwrap();
        let crate::Edit::External{input:minihou_core::nodes::blender_input::ExternalInput::Image{pixels,..},..}=edit else{panic!("image input expected")};assert_eq!(pixels[3],0.25);assert_eq!(pixels[7],0.75);assert!((pixels[1]-0.735357).abs()<1e-6);
    }
    #[test]
    fn watches_deduplicate_and_ignore_own_outputs() {
        let doc=json!({"nodes":{"a":{"type":"blender_object","params":{"object":"Cube"}},"b":{"type":"blender_object","params":{"object":"Cube"}},"own":{"type":"blender_object","params":{"object":"MiniHou_Proxy"}},"i":{"type":"blender_image","params":{"image":"Paint"}}}});
        assert_eq!(watch_objects(&doc),json!({"type":"watch_objects","objects":["Cube"],"images":["Paint"]}));
    }
    #[test]
    fn matches_python_control_frame_and_little_endian_geometry() {
        // bytes.join(encode_message({"type":"ping"})) from protocol.py.
        let expected = b"MHL1\x10\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00{\"type\":\"ping\"} ";
        assert_eq!(encode_message(&json!({"type":"ping"}), &Arrays::new()).unwrap().as_slice(), expected.as_slice());
        let mut geo = Geometry::from_arrays(vec![1.,-2.,0.5],vec![0],vec![0,1],None).unwrap();
        geo.point.insert("id".into(), Arc::new(minihou_core::geometry::Attr::i32(1,vec![42])));
        geo.vertex.insert("uv".into(), Arc::new(minihou_core::geometry::Attr::f32(2,vec![0.25,0.75])));
        geo.prim.insert("group".into(), Arc::new(minihou_core::geometry::Attr::i32(1,vec![1])));
        let arrays = geometry_arrays(&geo);
        assert_eq!(arrays["point/P"].shape,vec![1,3]);
        assert_eq!(arrays["point/P"].bytes.as_slice(), &[0,0,128,63,0,0,0,192,0,0,0,63]);
        assert_eq!(arrays["point/id"].shape,vec![1]);
        assert_eq!(arrays["point/id"].bytes.as_slice(), &[42,0,0,0]);
        assert_eq!(arrays["vertex/uv"].shape,vec![1,2]);
        assert_eq!(arrays["prim/group"].dtype,"<i4");
        assert_eq!(arrays[TOPO[1]].bytes.as_slice(), &[0,0,0,0,1,0,0,0]);
    }
    #[test]
    fn blender_mesh_arrays_replace_invalid_corner_uvs() {
        let mut geo = Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 0.,1.,0.], vec![0,1,2], vec![0,3], None).unwrap();
        geo.vertex.insert("uv".into(), Arc::new(minihou_core::Attr::f32(2, vec![f32::NAN, f32::INFINITY, 0.25, 0.75, 1., 0.])));
        let arrays = geometry_arrays(&geo);
        let uv = &arrays["vertex/uv"].bytes;
        let values: Vec<_> = uv.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        assert_eq!(values, vec![0., 0., 0.25, 0.75, 1., 0.]);
        assert!(geo.vertex["uv"].as_f32().unwrap()[0].is_nan());
    }
    #[test]
    fn round_trip_all_dtypes_alignment_and_fragmented_reads() {
        let mut arrays = Arrays::new();
        for (i, dtype) in ["<f4","<f8","<i4","<i8","|u1","|b1"].iter().enumerate() {
            arrays.insert(i.to_string(), Array { dtype:(*dtype).into(), shape:vec![1], bytes:Arc::new(vec![0; width(dtype).unwrap()]) });
        }
        arrays.insert("empty".into(), Array { dtype:"<f4".into(), shape:vec![0,3], bytes:Arc::new(vec![]) });
        let frame = encode_message(&json!({"type":"geo","label":"é"}), &arrays).unwrap();
        assert_eq!(&frame[..4], b"MHL1");
        struct Fragmented<'a>(&'a [u8]);
        impl Read for Fragmented<'_> { fn read(&mut self, b: &mut [u8]) -> io::Result<usize> { let n = b.len().min(3).min(self.0.len()); b[..n].copy_from_slice(&self.0[..n]); self.0 = &self.0[n..]; Ok(n) } }
        let (header, decoded) = read_message(&mut Fragmented(&frame), 10000).unwrap();
        assert_eq!(decoded, arrays); assert_eq!(header["label"], "é");
        assert!(header["arrays"].as_array().unwrap().iter().all(|d| d["offset"].as_u64().unwrap() % 8 == 0));
    }
    #[test]
    fn rejects_invalid_descriptors_truncation_and_oversize() {
        let mut frame = encode_message(&json!({"type":"ping"}), &Arrays::new()).unwrap();
        assert!(read_message(&mut &frame[..frame.len()-1], 1000).is_err());
        assert!(read_message(&mut &frame[..], 1).is_err());
        frame[0] = 0; assert!(read_message(&mut &frame[..], 1000).is_err());
        let mut header = serde_json::to_vec(&json!({"type":"geo","arrays":[{"key":"x","dtype":"<f4","shape":[3],"offset":0,"nbytes":4}]})).unwrap();
        header.resize(header.len() + (8-header.len()%8)%8, b' ');
        let hlen = header.len(); header.extend([0; 4]);
        assert!(decode_payload(&header, hlen).is_err());
    }
    #[test]
    fn loose_point_count_changes_require_a_topology_rebuild() {
        let mut diff=DiffState::default();
        let a=Geometry::points_only(vec![0.;3]).unwrap();
        diff.build(&geometry_arrays(&a),&a,1,&json!({}));
        let b=Geometry::points_only(vec![0.;6]).unwrap();
        let(h,arrays)=diff.build(&geometry_arrays(&b),&b,2,&json!({}));
        assert_eq!(h["topology_changed"],true);
        assert!(arrays.contains_key(TOPO[0]));
    }

    #[test]
    fn diffs_are_per_client_and_clear_removed_curve_flags_and_attributes() {
        let mut geo = Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.],vec![0,1],vec![0,2],Some(vec![false])).unwrap();
        let mut a = DiffState::default(); let mut b = DiffState::default();
        let arrays = geometry_arrays(&geo);
        assert_eq!(a.build(&arrays,&geo,1,&json!({})).0["full"],true);
        assert!(a.build(&arrays,&geo,2,&json!({})).1.is_empty());
        assert_eq!(b.build(&arrays,&geo,2,&json!({})).0["full"],true);
        geo.set_p(vec![0.,0.,1.,1.,0.,0.]);
        let (h, changed) = a.build(&geometry_arrays(&geo),&geo,3,&json!({}));
        assert_eq!(h["topology_changed"],false); assert_eq!(changed.len(),1);
        Arc::make_mut(&mut geo.topology).closed = None;
        let (_, changed) = a.build(&geometry_arrays(&geo),&geo,4,&json!({}));
        assert!(changed[TOPO[2]].bytes.is_empty()); assert!(changed.contains_key(TOPO[0]));
        geo.point.remove("P"); // P is synthesized even for an empty table.
        let mut arrays = geometry_arrays(&geo); arrays.remove("point/P");
        assert!(a.build(&arrays,&geo,5,&json!({})).0["manifest"].get("point/P").is_none());
        a.reset(); assert_eq!(a.build(&arrays,&geo,6,&json!({})).0["full"],true);
    }
}
