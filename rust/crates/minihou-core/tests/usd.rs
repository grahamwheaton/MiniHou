use minihou_core::{session::Session, usd_io::*, Attr, Geometry, Graph};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn file(ext: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "minihou_native_usd_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("scene.{ext}"))
}
fn sample() -> Geometry {
    let mut g = Geometry::from_arrays(
        vec![
            0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0., 0., 0., 1., 4., 5., 6.,
        ],
        vec![0, 1, 2, 3, 0, 4, 2],
        vec![0, 4, 7],
        Some(vec![true, false]),
    )
    .unwrap();
    g.point
        .insert("Cd".into(), Arc::new(Attr::f32(3, vec![0.25; 18])));
    g.point
        .insert("id".into(), Arc::new(Attr::i32(1, (0..6).collect())));
    g.point.insert(
        "group_corners".into(),
        Arc::new(Attr::i32(1, vec![1, 0, 1, 0, 1, 0])),
    );
    g.vertex.insert(
        "uv".into(),
        Arc::new(Attr::f32(2, (0..14).map(|x| x as f32 * 0.1).collect())),
    );
    g.prim
        .insert("group_front".into(), Arc::new(Attr::i32(1, vec![1, 0])));
    g.prim
        .insert("material_index".into(), Arc::new(Attr::i32(1, vec![0, 1])));
    g.prim.insert(
        "Cd".into(),
        Arc::new(Attr::f32(3, vec![1., 0., 0., 0., 1., 0.])),
    );
    g.point.insert(
        "wide attr".into(),
        Arc::new(Attr::i32(6, (0..36).collect())),
    );
    g.detail
        .insert("materials".into(), json!(["Red paint", "Green"]));
    g.detail.insert(
        "custom".into(),
        json!({"answer":42,"unicode":"café","quote":"a\"b\\c\n"}),
    );
    g
}
fn assert_geo(a: &Geometry, b: &Geometry) {
    assert_eq!(a.npoints, b.npoints);
    assert_eq!(a.topology.vertex_point, b.topology.vertex_point);
    assert_eq!(a.topology.prim_offsets, b.topology.prim_offsets);
    assert_eq!(a.topology.closed_mask(), b.topology.closed_mask());
    for (ta, tb) in [
        (&a.point, &b.point),
        (&a.vertex, &b.vertex),
        (&a.prim, &b.prim),
    ] {
        for (n, v) in ta {
            let other = tb.get(n).unwrap_or_else(|| panic!("missing {n}"));
            assert_eq!(v.width, other.width, "{n}");
            if let (Some(x), Some(y)) = (v.as_f32(), other.as_f32()) {
                assert_eq!(x.len(), y.len());
                for (x, y) in x.iter().zip(y) {
                    assert!((x - y).abs() < 1e-5, "{n}: {x} != {y}");
                }
            } else {
                assert_eq!(v, other, "{n}");
            }
        }
    }
    for (n, v) in &a.detail {
        assert_eq!(Some(v), b.detail.get(n), "detail {n}");
    }
}
#[test]
fn usd_roundtrip_all_formats_and_units() {
    for ext in ["usda", "usdc", "usd", "usdz"] {
        for axis in ["z", "y"] {
            let path = file(ext);
            let g = sample();
            let opts = WriteOptions {
                up_axis: axis.into(),
                units: "centimeters".into(),
                graph: Some(json!({"nodes":{}})),
                ..Default::default()
            };
            write_stage(
                &path,
                &Outputs::from([("Result".into(), Arc::new(g.clone()))]),
                &opts,
                12.,
            )
            .unwrap();
            if ext == "usdc" || ext == "usd" {
                assert!(std::fs::read(&path).unwrap().starts_with(b"PXR-USDC"));
            }
            let out = read_geometry(&path, &Default::default()).unwrap();
            assert_geo(&g, &out);
            assert_eq!(read_graph(&path).unwrap(), opts.graph);
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }
}
#[test]
fn usd_instancer_roundtrip() {
    let mut g = Geometry::points_only(vec![1., 2., 3., 4., 5., 6., 7., 8., 9.]).unwrap();
    g.point
        .insert("instance".into(), Arc::new(Attr::i32(1, vec![1, 2, 0])));
    g.point
        .insert("rot".into(), Arc::new(Attr::f32(3, vec![0.1; 9])));
    g.point
        .insert("scale".into(), Arc::new(Attr::f32(3, vec![1.5; 9])));
    g.point.insert(
        "group_invisible".into(),
        Arc::new(Attr::i32(1, vec![0, 1, 0])),
    );
    g.detail
        .insert("instance_objects".into(), json!(["Oak", "Pine"]));
    for units in ["meters", "centimeters"] {
        let path = file("usdc");
        write_stage(
            &path,
            &Outputs::from([("Trees".into(), Arc::new(g.clone()))]),
            &WriteOptions {
                units: units.into(),
                ..Default::default()
            },
            1.,
        )
        .unwrap();
        assert_geo(&g, &read_geometry(&path, &Default::default()).unwrap());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
#[test]
fn usd_hand_authored_stage_transforms_primvars_subsets() {
    let path = file("usda");
    std::fs::write(&path, include_str!("fixtures/usd_foreign.usda")).unwrap();
    let g = read_geometry(
        &path,
        &ReadOptions {
            path_attrib: "source".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(g.nprims(), 1);
    for (a, b) in g.p().iter().zip([1., 0., 0., 2., 0., 0., 1., 0., 1.]) {
        assert!((a - b).abs() < 1e-5);
    }
    assert_eq!(g.vertex["uv"].width, 2);
    assert_eq!(
        g.prim["group_front"].data,
        minihou_core::AttrData::I32(vec![1])
    );
    assert_eq!(
        g.prim["material_index"].data,
        minihou_core::AttrData::I32(vec![1])
    );
    assert_eq!(g.detail["materials"], json!(["", "Paint"]));
    assert_eq!(g.detail["usd_paths"], json!(["/World/chair"]));
    let proxy = read_geometry(
        &path,
        &ReadOptions {
            purposes: "proxy".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(proxy.npoints, 1);
    let empty = read_geometry(
        &path,
        &ReadOptions {
            prim_path: "/World ^/World/chair* ^/World/proxy".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(empty.npoints, 0);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
#[test]
fn usd_animated_node_cooks_upstream_and_restores_time() {
    let path = file("usdc");
    let doc = json!({"version":1,"editor":{"zoom":1.25},"nodes":{"p":{"type":"point","pos":[10,20],"params":{}},"w":{"type":"wrangle","inputs":{"geo":"p"},"params":{"snippet":"@P.x = @Frame;"}},"save":{"type":"usd_export","inputs":{"geo":"w"},"params":{"file":path,"frames":"range","start":1,"end":3,"name":"Animated"}}},"display":"save"});
    let mut session = Session::new(Graph::from_value(&doc).unwrap());
    session.set_time(12., 24.).unwrap();
    let g = session.cook(None).unwrap();
    assert_eq!(g.p()[0], 12.);
    assert_eq!(session.time(), (12., 24.));
    let out = read_geometry(
        &path,
        &ReadOptions {
            time: Some(2.5),
            ..Default::default()
        },
    )
    .unwrap();
    assert!((out.p()[0] - 2.5).abs() < 1e-6);
    assert_eq!(read_graph(&path).unwrap().unwrap(), doc);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
#[test]
fn usd_errors_overwrite_import_reload() {
    let path = file("usda");
    let g = sample();
    let outputs = Outputs::from([("g".into(), Arc::new(g))]);
    write_stage(&path, &outputs, &Default::default(), 1.).unwrap();
    assert!(write_stage(&path, &outputs, &Default::default(), 1.)
        .unwrap_err()
        .to_string()
        .contains("already exists"));
    let mut session=Session::new(Graph::from_value(&json!({"nodes":{"read":{"type":"usd_import","params":{"file":path}}},"display":"read"})).unwrap());
    assert_eq!(session.cook(None).unwrap().npoints, 6);
    let g = Geometry::points_only(vec![9., 0., 0.]).unwrap();
    write_stage(
        &path,
        &Outputs::from([("g".into(), Arc::new(g))]),
        &WriteOptions {
            overwrite: true,
            ..Default::default()
        },
        1.,
    )
    .unwrap();
    assert_eq!(session.cook(None).unwrap().npoints, 1);
    let bad = Graph::from_value(&json!({"nodes":{"s":{"type":"usd_import"}},"display":"s"}))
        .unwrap()
        .cook(None)
        .unwrap_err();
    assert!(bad.to_string().contains("set 'file' to a USD file"));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn usd_blender_binary_fixture_bakes_parent_transform() {
    let path = file("usdc");
    std::fs::write(&path, include_bytes!("fixtures/usd_blender_4_5.usdc")).unwrap();
    let g = read_geometry(&path, &Default::default()).unwrap();
    assert_eq!(g.nprims(), 1);
    assert_eq!(g.npoints, 3);
    assert_eq!(g.p(), &[3., 4., 5., 4., 4., 5., 3., 5., 5.]);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn usd_multiple_outputs_material_tables_and_references() {
    let path = file("usda");
    let a = sample();
    let mut b = sample();
    b.detail
        .insert("materials".into(), json!(["Blue", "Other"]));
    write_stage(
        &path,
        &Outputs::from([("A".into(), Arc::new(a)), ("B".into(), Arc::new(b))]),
        &WriteOptions::default(),
        1.,
    )
    .unwrap();
    let g = read_geometry(&path, &Default::default()).unwrap();
    assert_eq!(g.npoints, 12);
    assert_eq!(g.nprims(), 4);
    let mats = g.detail["materials"].as_array().unwrap();
    let slots = match &g.prim["material_index"].data {
        minihou_core::AttrData::I32(v) => v,
        _ => panic!(),
    };
    assert_eq!(
        slots
            .iter()
            .map(|&s| mats[s as usize].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("Red paint"),
            json!("Green"),
            json!("Blue"),
            json!("Other")
        ]
    );
    let source = file("usda");
    std::fs::write(&source, include_str!("fixtures/usd_foreign.usda")).unwrap();
    let imported = read_geometry(&source, &Default::default()).unwrap();
    assert!(imported.prim.contains_key("Cd"));
    let dest = file("usdc");
    write_stage(
        &dest,
        &Outputs::from([("Imported".into(), Arc::new(imported))]),
        &WriteOptions::default(),
        1.,
    )
    .unwrap();
    let layer = openusd::sdf::Layer::open(dest.to_string_lossy().as_ref()).unwrap();
    assert!(layer.export_to_string().unwrap().contains("references = @"));
    assert_eq!(
        read_geometry(&dest, &Default::default()).unwrap().nprims(),
        1
    );
    for p in [path, source, dest] {
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
}

#[test]
fn usd_changing_topology_late_attributes_and_empty_frames() {
    let path = file("usdc");
    let a = sample();
    let mut b = Geometry::points_only(vec![1., 2., 3.]).unwrap();
    b.point
        .insert("late".into(), Arc::new(Attr::f32(1, vec![7.])));
    b.detail.insert("frame_detail".into(), json!(2));
    let empty = Geometry::points_only(vec![]).unwrap();
    let samples = [a.clone(), b.clone(), empty]
        .into_iter()
        .enumerate()
        .map(|(i, g)| Ok((i as f64 + 1., Outputs::from([("G".into(), Arc::new(g))]))));
    write_animation(&path, samples, &Default::default(), true).unwrap();
    let first = read_geometry(&path, &Default::default()).unwrap();
    assert_eq!(first.npoints, 6);
    assert!(!first.point.contains_key("late"));
    let second = read_geometry(
        &path,
        &ReadOptions {
            time: Some(2.),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(second.npoints, 1);
    assert_eq!(second.nprims(), 0);
    assert_eq!(second.point["late"].as_f32(), Some(&[7.][..]));
    assert_eq!(second.detail["frame_detail"], 2);
    let third = read_geometry(
        &path,
        &ReadOptions {
            time: Some(3.),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(third.npoints, 0);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn usd_animation_merges_instancer_prototypes() {
    let path = file("usda");
    let mut a = Geometry::points_only(vec![0., 0., 0.]).unwrap();
    a.point
        .insert("instance".into(), Arc::new(Attr::i32(1, vec![1])));
    a.detail.insert("instance_objects".into(), json!(["Oak"]));
    let mut b = a.clone();
    b.detail
        .insert("instance_objects".into(), json!(["Pine", "Oak"]));
    write_animation(
        &path,
        [(1., a), (2., b)]
            .into_iter()
            .map(|(t, g)| Ok((t, Outputs::from([("G".into(), Arc::new(g))])))),
        &Default::default(),
        true,
    )
    .unwrap();
    let stage = openusd::usd::Stage::open(path.to_string_lossy().as_ref()).unwrap();
    let p = stage.prim("/MiniHou/G/instances").unwrap();
    assert_eq!(p.relationship("prototypes").targets().unwrap().len(), 2);
    let idx = p
        .attribute("protoIndices")
        .get_at::<Vec<i32>>(openusd::usd::TimeCode::new(2.))
        .unwrap()
        .unwrap();
    assert_eq!(idx, vec![1]);
    let g = read_geometry(
        &path,
        &ReadOptions {
            time: Some(2.),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(g.detail["instance_objects"], json!(["Pine", "Oak"]));
    assert_eq!(
        g.point["instance"].data,
        minihou_core::AttrData::I32(vec![1])
    );
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn usd_current_export_cook_rewrites_deleted_file_and_bad_range_preserves_target() {
    let path = file("usda");
    let doc = json!({"nodes":{"p":{"type":"point"},"s":{"type":"usd_export","inputs":{"geo":"p"},"params":{"file":path}}},"display":"s"});
    let mut session = Session::new(Graph::from_value(&doc).unwrap());
    session.cook(None).unwrap();
    std::fs::remove_file(&path).unwrap();
    session.cook(None).unwrap();
    assert!(path.exists());
    let old = std::fs::read(&path).unwrap();
    session.set_param("s", "frames", json!("range")).unwrap();
    session.set_param("s", "end", json!(0)).unwrap();
    assert!(session
        .cook(None)
        .unwrap_err()
        .to_string()
        .contains("the frame range 1..0 is empty"));
    assert_eq!(old, std::fs::read(&path).unwrap());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn usd_foreign_instancer_and_scene_instances() {
    let path = file("usda");
    std::fs::write(
        &path,
        r#"#usda 1.0
(upAxis = "Y" metersPerUnit = 1)
class Xform "Prototype" {
    def Mesh "Shape" {
        point3f[] points = [(0,0,0),(1,0,0),(0,1,0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0,1,2]
    }
}
def Xform "World" {
    float3 xformOp:scale = (2,2,2)
    uniform token[] xformOpOrder = ["xformOp:scale"]
    def PointInstancer "Scatter" {
        point3f[] positions = [(1,0,0),(0,0,3)]
        int[] protoIndices = [0,0]
        float3[] scales = [(1,1,1),(2,2,2)]
        int64[] ids = [100,200]
        int64[] invisibleIds = [200]
        rel prototypes = </World/Scatter/Proto>
        def Xform "Proto" (references = </Prototype>) {}
    }
    def Xform "Copy" (instanceable = true references = </Prototype>) {
        double3 xformOp:translate = (10,0,0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
}
"#,
    )
    .unwrap();
    let scatter = read_geometry(
        &path,
        &ReadOptions {
            prim_path: "/World/Scatter".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(scatter.npoints, 2);
    assert_eq!(scatter.nprims(), 0);
    assert_eq!(scatter.detail["instance_objects"], json!(["Proto"]));
    assert_eq!(
        scatter.point["group_invisible"].data,
        minihou_core::AttrData::I32(vec![0, 1])
    );
    assert_eq!(
        scatter.point["scale"].as_f32(),
        Some(&[2., 2., 2., 4., 4., 4.][..])
    );
    assert!(scatter.detail["instance_bounds"].is_array());
    let copy = read_geometry(
        &path,
        &ReadOptions {
            prim_path: "/World/Copy".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(copy.nprims(), 1);
    assert_eq!(copy.p()[0], 20.);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
