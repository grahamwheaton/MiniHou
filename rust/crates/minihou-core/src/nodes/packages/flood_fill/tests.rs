use super::*;
use serde_json::{json, Value};

fn run(g: &Geometry, params: Value) -> Geometry {
    let p=Params::resolve(NODE.params,params.as_object()).unwrap();
    cook(&[vec![Arc::new(g.clone())]],&p).unwrap()
}
fn values(g:&Geometry, domain:&str, name:&str)->Vec<i32> {
    let table=match domain {"points"=>&g.point,"edges"=>&g.edge,_=>&g.prim};
    table[name].as_i32().unwrap().to_vec()
}
fn grid()->Geometry {
    let mut p=Vec::new(); for y in 0..3 {for x in 0..3 {p.extend([x as f32,y as f32,0.]);}}
    let mut vp=Vec::new(); let mut offsets=vec![0];
    for y in 0..2 {for x in 0..2 {let a=y*3+x;vp.extend([a,a+1,a+4,a+3]);offsets.push(vp.len() as i32);}}
    Geometry::from_arrays(p,vp,offsets,None).unwrap()
}
fn cube()->Geometry {
    Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0., 0.,0.,1., 1.,0.,1., 1.,1.,1., 0.,1.,1.],
        vec![0,3,2,1, 4,5,6,7, 0,1,5,4, 1,2,6,5, 2,3,7,6, 3,0,4,7],
        vec![0,4,8,12,16,20,24],None).unwrap()
}
fn curve()->Geometry {
    Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,2.,0.,0.,3.,0.,0.],vec![0,1,2,3],vec![0,4],Some(vec![false])).unwrap()
}

#[test]
fn grid_domains_and_cube_faces() {
    let g=grid();
    let points=run(&g,json!({"seed_group":"0","max_steps":1,"store_step_distance":true}));
    assert_eq!(values(&points,"points","group_flood"),vec![1,1,0,1,0,0,0,0,0]);
    assert_eq!(values(&points,"points","step_distance")[0],0);
    let edges=run(&g,json!({"domain":"edges","seed_group":"0","max_steps":1}));
    assert_eq!(values(&edges,"edges","group_flood").iter().sum::<i32>(),4);
    let faces=run(&g,json!({"domain":"faces","seed_group":"0","max_steps":1}));
    assert_eq!(values(&faces,"faces","group_flood"),vec![1,1,1,0]);
    let cube_faces=run(&cube(),json!({"domain":"faces","seed_group":"0"}));
    assert_eq!(values(&cube_faces,"faces","group_flood"),vec![1;6]);
}

#[test]
fn masks_barriers_and_combine() {
    let mut g=curve();
    g.point.insert("group_old".into(),Arc::new(Attr::i32(1,vec![0,0,1,1])));
    let blocked=run(&g,json!({"seed_group":"0","barrier_group":"2","frontier_group":"front"}));
    assert_eq!(values(&blocked,"points","group_flood"),vec![1,1,0,0]);
    assert_eq!(values(&blocked,"points","group_front"),vec![1,1,0,0]);
    let avoided=run(&g,json!({"seed_group":"0","avoid_group":"1"}));
    assert_eq!(values(&avoided,"points","group_flood"),vec![1,0,0,0]);
    let allowed=run(&g,json!({"seed_group":"0","allowed_group":"0-1"}));
    assert_eq!(values(&allowed,"points","group_flood"),vec![1,1,0,0]);
    let union=run(&g,json!({"seed_group":"0","max_steps":1,"output_group":"old","combine":"union"}));
    assert_eq!(values(&union,"points","group_old"),vec![1,1,1,1]);
    let subtract=run(&g,json!({"seed_group":"2","max_steps":0,"output_group":"old","combine":"subtract"}));
    assert_eq!(values(&subtract,"points","group_old"),vec![0,0,0,1]);
}

#[test]
fn angles_direction_and_open_curve() {
    let g=curve();
    assert_eq!(g.nedges(),3);
    let out=run(&g,json!({"seed_group":"0","direction":[1,0,0],"max_deviation":0}));
    assert_eq!(values(&out,"points","group_flood"),vec![1;4]);
    let reverse=run(&g,json!({"seed_group":"0","direction":[-1,0,0],"max_deviation":0}));
    assert_eq!(values(&reverse,"points","group_flood"),vec![1,0,0,0]);
    let mut cube=cube();
    // A sharply folded face pair must not cross a zero degree normal limit.
    cube.prim.insert("group_seed".into(),Arc::new(Attr::i32(1,vec![1,0,0,0,0,0])));
    let cut=run(&cube,json!({"domain":"faces","seed_group":"seed","max_angle":0}));
    assert_eq!(values(&cut,"faces","group_flood"),vec![1,0,0,0,0,0]);
}

#[test]
fn disconnected_regions_and_target() {
    let g=Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0., 5.,0.,0.,6.,0.,0.],
        vec![0,1,2,3],vec![0,2,4],Some(vec![false,false])).unwrap();
    let island=run(&g,json!({"seed_group":"0"}));
    assert_eq!(values(&island,"points","group_flood"),vec![1,1,0,0]);
    let regions=run(&g,json!({"mode":"regions"}));
    assert_eq!(values(&regions,"points","region_id"),vec![0,0,1,1]);
    let target=run(&curve(),json!({"seed_group":"0","target_group":"2","mode":"flood_to_target"}));
    assert_eq!(values(&target,"points","group_flood"),vec![1,1,1,0]);
}

#[test]
fn steps_distance_and_attribute_tolerance() {
    let mut g=curve();
    let steps=run(&g,json!({"seed_group":"0","max_steps":1,"store_step_distance":true}));
    assert_eq!(values(&steps,"points","group_flood"),vec![1,1,0,0]);
    let distance=run(&g,json!({"seed_group":"0","max_distance":2.0,"store_surface_distance":true}));
    assert_eq!(values(&distance,"points","group_flood"),vec![1,1,1,0]);
    assert_eq!(distance.point["surface_distance"].as_f32().unwrap(),&[0.,1.,2.,-1.]);
    g.point.insert("heat".into(),Arc::new(Attr::f32(1,vec![0.,0.4,0.8,2.])));
    let neighbor=run(&g,json!({"seed_group":"0","attr_name":"heat","attr_mode":"neighbor_diff","attr_tolerance":0.5}));
    assert_eq!(values(&neighbor,"points","group_flood"),vec![1,1,1,0]);
    let seed=run(&g,json!({"seed_group":"0","attr_name":"heat","attr_mode":"seed_diff","attr_tolerance":0.5}));
    assert_eq!(values(&seed,"points","group_flood"),vec![1,1,0,0]);
    let threshold=run(&g,json!({"seed_group":"0","attr_name":"heat","attr_mode":"threshold","attr_tolerance":0.5}));
    assert_eq!(values(&threshold,"points","group_flood"),vec![1,1,0,0]);
}

#[test]
fn dijkstra_prefers_shorter_surface_path_to_fewer_steps() {
    let g=Geometry::from_arrays(vec![0.,0.,0., 100.,0.,0., 4.,0.,0., 1.,0.,0., 2.,0.,0.],
        vec![0,1,2, 0,3,4,2],vec![0,3,7],Some(vec![false,false])).unwrap();
    let bfs=run(&g,json!({"seed_group":"0","max_steps":3,"store_step_distance":true}));
    assert_eq!(values(&bfs,"points","step_distance")[2],2);
    let dijkstra=run(&g,json!({"seed_group":"0","max_distance":10.0,"store_step_distance":true,"store_surface_distance":true}));
    assert_eq!(values(&dijkstra,"points","step_distance")[2],3);
    assert_eq!(dijkstra.point["surface_distance"].as_f32().unwrap()[2],4.0);
}

#[test]
fn boundary_and_element_limit() {
    let g=curve();
    let boundary=run(&g,json!({"seed_group":"0","stop_at_boundary":true}));
    assert_eq!(values(&boundary,"points","group_flood"),vec![1,0,0,0]);
    let limited=run(&g,json!({"seed_group":"0","max_elements":2}));
    assert_eq!(values(&limited,"points","group_flood"),vec![1,1,0,0]);
}

#[test]
fn missing_input_and_invalid_options() {
    assert!(cook(&[vec![]],&Params::resolve(NODE.params,None).unwrap()).is_err());
    let p=Params::resolve(NODE.params,json!({"domain":"vertices"}).as_object()).unwrap();
    assert!(cook(&[vec![Arc::new(grid())]],&p).is_err());
}
