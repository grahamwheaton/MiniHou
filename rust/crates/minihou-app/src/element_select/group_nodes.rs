use super::*;

fn minihou_app_meta(node_type: &str, name: &str) -> Option<&'static serde_json::Value> { minihou_app::param_meta(node_type, name) }

/// The input group field a new node takes the viewport selection in, plus its domain parameter (`domain` / `group_type`)
/// when it has one. `None` for generators (no inputs), output group fields and nodes without a group field.
pub fn find_input_group_field(node_type: &str) -> Option<(&'static str, Option<&'static str>)> {
    use crate::ui::params::is_input_group_field;
    let spec = minihou_core::nodes::get(node_type)?;
    if spec.inputs.is_empty() { return None; }
    let field = spec.params.iter().map(|p| p.name)
        .filter(|n| is_input_group_field(n, minihou_app_meta(node_type, n)))
        .min_by_key(|n| (*n != "group", *n != "pattern"))?;
    let domain = ["domain", "group_type"].into_iter().find(|d| spec.params.iter().any(|p| p.name == *d));
    Some((field, domain))
}

/// Selection nodes that take their seeds from the viewport selection when added from the Tab menu (docs/RUST_SELECT_NODES.md).
pub const SELECTION_NODES: [&str; 4] = ["grow", "select_path", "select_loop", "select_ring"];

impl Topology {
    /// The ids of `sel` in its mode's id space (points, edge ids, polygons), in selection order.
    pub(super) fn ids(&self, sel: &Selection) -> Vec<u32> {
        match sel.mode {
            SelectionMode::Points => sel.points.clone(),
            SelectionMode::Edges => sel.edges.iter().filter_map(|&(a, b)| self.edge_of(a, b)).collect(),
            SelectionMode::Faces => self.polys_of(&sel.tris),
            SelectionMode::Object => Vec::new(),
        }
    }
    /// Element tokens of the selection-node patterns: point / primitive numbers, `p<a>-<b>` edges.
    pub(super) fn tokens(&self, mode: SelectionMode, ids: &[u32]) -> Vec<String> {
        ids.iter().map(|&i| match mode {
            SelectionMode::Edges => { let (a, b) = self.edges[i as usize]; format!("p{a}-{b}") }
            _ => i.to_string(),
        }).collect()
    }

    /// Expand connected components or invert, within the currently pickable domain.
    pub fn linked_or_inverse(&self, sel:&mut Selection, mode:SelectionMode, inverse:bool, visible:impl Fn(u32)->bool) -> bool {
        let Some(domain)=domain(mode) else {return false;};
        let eligible=|id:u32| match mode {
            SelectionMode::Points=>visible(id),
            SelectionMode::Edges=>{let (a,b)=self.edges[id as usize];visible(a)&&visible(b)},
            SelectionMode::Faces=>self.polygon_vertices(id).iter().all(|&v|visible(v)),
            _=>false,
        };
        let ids=if sel.mode==mode {self.ids(sel)}else{Vec::new()};
        let mut mask=vec![false;self.count(domain)];
        for &id in &ids {if let Some(value)=mask.get_mut(id as usize) {if eligible(id) {*value=true;}}}
        if inverse {for (id,value) in mask.iter_mut().enumerate() {*value=!*value&&eligible(id as u32);}}
        else {
            let mut pending:Vec<_>=ids.iter().copied().filter(|&id|mask.get(id as usize)==Some(&true)).collect();
            let mut neighbours=Vec::new();
            while let Some(id)=pending.pop() {
                self.neighbours(domain,id,&mut neighbours);
                for &next in &neighbours {if !mask[next as usize]&&eligible(next) {mask[next as usize]=true;pending.push(next);}}
            }
        }
        let mut next=Selection {mode,..Default::default()};
        let selected:Vec<_>=mask.iter().enumerate().filter(|(_,value)|**value).map(|(id,_)|id as u32).collect();
        match mode {
            SelectionMode::Points=>next.points=selected,
            SelectionMode::Edges=>next.edges=selected.into_iter().map(|id|self.edges[id as usize]).collect(),
            SelectionMode::Faces=>next.tris=self.tris_of(&selected),
            _=>{},
        }
        let changed=*sel!=next;*sel=next;changed
    }

    /// Select More / Less (Blender Ctrl+Numpad +/-, Face Step on) on the viewport selection. Selection state only: the
    /// `grow` node is never created by this. Kept elements keep their order; added ones follow in id order. Returns
    /// whether the selection changed.
    pub fn grow_selection(&self, sel: &mut Selection, shrink: bool) -> bool {
        let Some(d) = domain(sel.mode) else { return false; };
        let ids = self.ids(sel);
        if ids.is_empty() { return false; }
        let mut mask = vec![false; self.count(d)];
        for &i in &ids { if let Some(m) = mask.get_mut(i as usize) { *m = true; } }
        self.core.grow(d, &mut mask, 1, true, shrink);
        let mut next: Vec<u32> = ids.iter().copied().filter(|&i| mask.get(i as usize) == Some(&true)).collect();
        let kept: HashSet<u32> = next.iter().copied().collect();
        next.extend(mask.iter().enumerate().filter(|(i, m)| **m && !kept.contains(&(*i as u32))).map(|(i, _)| i as u32));
        let before = sel.count();
        match sel.mode {
            SelectionMode::Points => sel.points = next,
            SelectionMode::Edges => sel.edges = next.into_iter().map(|e| self.edges[e as usize]).collect(),
            SelectionMode::Faces => sel.tris = self.tris_of(&next),
            SelectionMode::Object => {}
        }
        sel.count() != before
    }

    /// Parameters for a node added from the Tab menu while elements are selected: `grow` gets the selection
    /// as its group, `select_loop` / `select_ring` the selected edges as seeds (other modes converted to edges),
    /// `select_path` the selected elements as ordered waypoints. `None` for other nodes or an empty selection.
    pub fn node_params(&self, node_type: &str, sel: &Selection) -> Option<serde_json::Value> {
        let special = SELECTION_NODES.contains(&node_type) || matches!(node_type, "edge_slide" | "group_create" | "boundary_select" | "select_sharp_edges" | "split");
        if !(special || find_input_group_field(node_type).is_some()) || sel.is_empty() { return None; }
        let name = match sel.mode { SelectionMode::Points => "points", SelectionMode::Edges => "edges", SelectionMode::Faces => "primitives", SelectionMode::Object => return None };
        let ids = self.ids(sel);
        if ids.is_empty() { return None; }
        let join = |mode: SelectionMode, ids: &[u32]| self.tokens(mode, ids).join(" ");
        let group_pattern = || if sel.mode == SelectionMode::Edges {
            ids.iter().map(|&i| { let (a, b) = self.edges[i as usize]; format!("{a}-{b}") }).collect::<Vec<_>>().join(" ")
        } else { join(sel.mode, &ids) };
        Some(match node_type {
            "group_create" => {
                serde_json::json!({"domain": name, "method": "pattern", "pattern": group_pattern()})
            }
            "blast" => serde_json::json!({"domain": name, "group": group_pattern()}),
            "split" => serde_json::json!({"group_type": name, "group": join(sel.mode, &ids)}),
            "boundary_select" => {
                let dom = match sel.mode { SelectionMode::Faces => "faces", SelectionMode::Points => "points", _ => "edges" };
                serde_json::json!({"group": join(sel.mode, &ids), "domain": dom})
            }
            "select_sharp_edges" => {
                let edges = if sel.mode == SelectionMode::Edges { ids } else { self.ids(&self.convert(sel, SelectionMode::Edges)) };
                if edges.is_empty() { return None; }
                serde_json::json!({"group": join(SelectionMode::Edges, &edges)})
            }
            "edge_slide" => {
                let edges = if sel.mode == SelectionMode::Edges { ids } else { self.ids(&self.convert(sel, SelectionMode::Edges)) };
                if edges.is_empty() { return None; }
                serde_json::json!({"group": join(SelectionMode::Edges, &edges)})
            }
            "grow" => serde_json::json!({"group": join(sel.mode, &ids), "domain": name}),
            "select_path" => serde_json::json!({"waypoints": join(sel.mode, &ids), "domain": name}),
            "attribute_create" => {
                // Attribute Create has point and primitive classes, but no edge class. Use the
                // selected edges' endpoint points so an edge selection still constrains it.
                let (target, domain) = if sel.mode == SelectionMode::Faces {
                    (SelectionMode::Faces, "primitives")
                } else { (SelectionMode::Points, "points") };
                let ids = if target == sel.mode { ids } else { self.ids(&self.convert(sel, target)) };
                if ids.is_empty() { return None; }
                serde_json::json!({"group": join(target, &ids), "domain": domain, "group_type": domain})
            }
            other if !SELECTION_NODES.contains(&other) => {
                let (field, domain_param) = find_input_group_field(other)?;
                let meta = minihou_app_meta(other, field);
                let fixed = meta.filter(|m| m.get("pick_domain_param").is_none()).and_then(|m| crate::ui::params::pick_domain(Some(m), &serde_json::Value::Null));
                let options: Vec<&str> = domain_param.and_then(|d| minihou_app_meta(other, d)).and_then(|m| m["options"].as_array())
                    .map(|o| o.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
                let (target, domain_value) = match (fixed, domain_param) {
                    (Some(mode), _) => (mode, None),
                    (None, Some(d)) => {
                        let value = match sel.mode { SelectionMode::Points => "points", SelectionMode::Edges => "edges", _ => if options.contains(&"faces") { "faces" } else { "primitives" } };
                        if !options.contains(&value) { return None; }
                        (sel.mode, Some((d, value)))
                    }
                    (None, None) => (SelectionMode::Points, None),
                };
                let ids = if target == sel.mode { ids } else { self.ids(&self.convert(sel, target)) };
                if ids.is_empty() { return None; }
                let mut params = serde_json::json!({ field: join(target, &ids) });
                if let Some((d, v)) = domain_value { params[d] = serde_json::json!(v); }
                params
            }
            _ => {
                let edges = if sel.mode == SelectionMode::Edges { ids } else { self.ids(&self.convert(sel, SelectionMode::Edges)) };
                if edges.is_empty() { return None; }
                serde_json::json!({"seeds": join(SelectionMode::Edges, &edges), "domain": name})
            }
        })
    }
}

