//! Geometry-wire connectivity for network and object selection (never cooks nodes).
use std::collections::{BTreeSet,HashMap};
pub fn linked(graph:&minihou_core::Graph, seeds:&BTreeSet<String>)->BTreeSet<String> {
    let mut neighbours:HashMap<&str,Vec<&str>>=graph.nodes.iter().map(|(id,_)|(id.as_str(),Vec::new())).collect();
    for (id,node) in &graph.nodes { for (_,inputs) in &node.inputs { for input in inputs {
        if neighbours.contains_key(input.as_str()) {
            neighbours.get_mut(id.as_str()).unwrap().push(input.as_str());
            neighbours.get_mut(input.as_str()).unwrap().push(id.as_str());
        }
    } } }
    let mut selected=BTreeSet::new(); let mut pending:Vec<_>=seeds.iter().map(String::as_str).collect();
    while let Some(id)=pending.pop() {
        if let Some(next)=neighbours.get(id) { if selected.insert(id.to_owned()) {pending.extend(next.iter().copied());} }
    }
    selected
}
pub fn inverse(universe:impl IntoIterator<Item=String>, selected:&BTreeSet<String>)->BTreeSet<String> {
    universe.into_iter().filter(|id|!selected.contains(id)).collect()
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn linked_walks_branches_both_ways_and_keeps_other_streams_separate() {
        let graph=minihou_core::Graph::from_value(&serde_json::json!({"nodes":{
            "a":{"type":"box"},"b":{"type":"normal","inputs":{"geo":"a"}},
            "c":{"type":"normal","inputs":{"geo":"a"}},"other":{"type":"box"}
        }})).unwrap();
        let seeds=BTreeSet::from(["b".into()]);
        assert_eq!(linked(&graph,&seeds),BTreeSet::from(["a".into(),"b".into(),"c".into()]));
        assert!(linked(&graph,&BTreeSet::new()).is_empty());
        assert_eq!(inverse(graph.nodes.iter().map(|(id,_)|id.clone()),&seeds),BTreeSet::from(["a".into(),"c".into(),"other".into()]));
        let cycle=minihou_core::Graph::from_value(&serde_json::json!({"nodes":{
            "a":{"type":"normal","inputs":{"geo":"b"}},"b":{"type":"normal","inputs":{"geo":"a"}}
        }})).unwrap();
        assert_eq!(linked(&cycle,&seeds),BTreeSet::from(["a".into(),"b".into()]));
    }
}
