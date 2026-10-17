//! Selection-derived gizmo data changes with selection/geometry, not camera redraws.
use super::*;
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) struct Metrics { pub center: Option<[f32;3]>, pub normal: [f64;3] }
struct Entry {
    revision: u64, mode: ui::SelectionMode, selection: picking::Selection,
    visibility: viewport::ObjectVisibility, ranges: Vec<(String,std::ops::Range<u32>)>,
    owners: std::collections::BTreeSet<String>, metrics: Metrics,
}
#[derive(Default)]
pub(crate) struct Cache(std::cell::RefCell<Option<Entry>>);
impl Cache {
    #[allow(clippy::too_many_arguments)]
    pub fn get(&self, revision:u64, selection:&picking::Selection, mode:ui::SelectionMode,
        scene:&Scene, visibility:viewport::ObjectVisibility, ranges:&[(String,std::ops::Range<u32>)],
        owners:&std::collections::BTreeSet<String>) -> Metrics {
        if let Some(entry) = self.0.borrow().as_ref() {
            if entry.revision==revision && entry.mode==mode && entry.selection==*selection && entry.visibility==visibility
                && entry.ranges==ranges && entry.owners==*owners { return entry.metrics; }
        }
        let ids=selected_point_ids(selection,mode,scene.triangle_indices());
        let mut metrics=Metrics::default(); let mut center=[0.;3]; let mut count=0;
        for id in ids {
            if !point_pickable(visibility,ranges,owners,id) { continue; }
            let k=id as usize*3;
            if let Some(p)=scene.positions().get(k..k+3) {
                for a in 0..3 { center[a]+=p[a]; } count+=1;
            }
            if let Some(n)=scene.normals().get(k..k+3) { for a in 0..3 { metrics.normal[a]+=n[a] as f64; } }
        }
        if count>0 { metrics.center=Some(center.map(|x| x/count as f32)); }
        *self.0.borrow_mut()=Some(Entry {revision,mode,selection:selection.clone(),visibility,ranges:ranges.to_vec(),owners:owners.clone(),metrics});
        metrics
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn metrics_refresh_on_selection_geometry_mode_and_visibility() {
        let graph=Graph::from_json(r#"{"nodes":{"box":{"type":"box"}},"display":"box"}"#).unwrap();
        let geo=graph.cook(Some("box")).unwrap();
        let scene=Scene::new(&geo); let cache=Cache::default();
        let mut sel=picking::Selection {mode:ui::SelectionMode::Faces,tris:vec![0],..Default::default()};
        let owners=std::collections::BTreeSet::from(["box".to_owned()]);
        let ranges=vec![("box".into(),0..geo.npoints as u32)];
        let get=|revision,sel:&picking::Selection,scene:&Scene,visibility,ranges:&[(String,std::ops::Range<u32>)],owners:&std::collections::BTreeSet<String>| cache.get(revision,sel,sel.mode,scene,visibility,ranges,owners);
        let expected=get(0,&sel,&scene,viewport::ObjectVisibility::ShowAll,&ranges,&owners);
        assert!(expected.center.is_some());
        for _ in 0..10 { assert_eq!(get(0,&sel,&scene,viewport::ObjectVisibility::ShowAll,&ranges,&owners),expected); }
        sel.tris.clear(); assert_eq!(get(0,&sel,&scene,viewport::ObjectVisibility::ShowAll,&ranges,&owners),Metrics::default());
        sel.mode=ui::SelectionMode::Points; sel.points=vec![0];
        let point=get(0,&sel,&scene,viewport::ObjectVisibility::ShowAll,&ranges,&owners);
        assert_eq!(point.center,Some(picking::at(scene.positions(),0)));
        assert_eq!(get(0,&sel,&scene,viewport::ObjectVisibility::HideOthers,&ranges,&Default::default()),Metrics::default());
        assert_eq!(get(0,&sel,&scene,viewport::ObjectVisibility::HideOthers,&[],&owners),Metrics::default());
        assert_eq!(get(0,&sel,&scene,viewport::ObjectVisibility::HideOthers,&ranges,&owners),point);
        let moved=minihou_core::Geometry::points_only(vec![2.,3.,4.]).unwrap();
        let moved=Scene::new(&moved);
        assert_eq!(get(1,&sel,&moved,viewport::ObjectVisibility::ShowAll,&ranges,&owners).center,Some([2.,3.,4.]));
    }
}
