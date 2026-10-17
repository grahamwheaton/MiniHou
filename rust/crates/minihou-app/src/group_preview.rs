//! Cached overlay geometry from the cooked membership of the active Group Create.
use super::*;
#[derive(Default)]
pub struct State {key:Option<(u64,String)>}
impl State {
    pub fn update(&mut self,gpu:&mut Gpu,device:&wgpu::Device,g:Option<&minihou_core::Geometry>,doc:&Value,selected:&str,revision:u64) {
        let key=(revision,selected.to_owned());if self.key.as_ref()==Some(&key){return;}
        self.key=Some(key);
        let def=&doc["nodes"][selected];
        let buffers=if def["type"]=="group_create" {g.map(|g|geometry(g,def["params"]["name"].as_str().unwrap_or("group1")))}else{None};
        let (faces,lines)=buffers.unwrap_or_default();gpu.set_group_overlay(device,&faces,&lines);
    }
}
pub fn geometry(g:&minihou_core::Geometry,name:&str)->(Vec<f32>,Vec<f32>) {
    let key=format!("group_{name}");let mut faces=Vec::new();let mut lines=Vec::new();
    let member=|a:&minihou_core::geometry::Attr| match &a.data {
        minihou_core::geometry::AttrData::F32(v)=>v.chunks(a.width).map(|r|r[0]!=0.).collect::<Vec<_>>(),
        minihou_core::geometry::AttrData::I32(v)=>v.chunks(a.width).map(|r|r[0]!=0).collect::<Vec<_>>(),
    };
    if let Some(a)=g.prim.get(&key) {
        let mask=member(a);let layout=minihou_core::coplanar::preview_layout(g);
        for (tri,&prim) in layout.tri.chunks_exact(3).zip(&layout.tri_prim) {if mask[prim as usize] {for &id in tri {faces.extend_from_slice(&g.p()[id as usize*3..id as usize*3+3]);}}}
        let mut edges=std::collections::BTreeSet::new();
        for (prim,selected) in mask.into_iter().enumerate() {if selected {
            let r=&g.topology.prim_offsets[prim..prim+2];let ids=&g.topology.vertex_point[r[0] as usize..r[1] as usize];
            for (i,&a) in ids.iter().enumerate() {if i+1<ids.len()||g.topology.is_closed(prim) {let b=ids[(i+1)%ids.len()];edges.insert((a.min(b),a.max(b)));}}
        }}
        for (a,b) in edges {for id in [a,b] {lines.extend_from_slice(&g.p()[id as usize*3..id as usize*3+3]);}}
    }else if let Some(a)=g.point.get(&key) {
        let selection=picking::Selection {mode:ui::SelectionMode::Points,points:member(a).into_iter().enumerate().filter(|(_,v)|*v).map(|(i,_)|i as u32).collect(),..Default::default()};
        (_,lines)=picking::highlight(g.p(),&[],&selection,0.01);
    }
    (faces,lines)
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn point_groups_preview_only_their_members() {
        let doc=json!({"nodes":{"box":{"type":"box"},"group":{"type":"group_create","inputs":{"geo":"box"},"params":{"domain":"points","name":"tip","pattern":"0"}}},"display":"group"});
        let g=Graph::from_value(&doc).unwrap().cook(Some("group")).unwrap();
        let (faces,lines)=geometry(&g,"tip");assert!(faces.is_empty());assert_eq!(lines.len(),18);
        assert_eq!(geometry(&g,"missing"),(vec![],vec![]));
    }
}
