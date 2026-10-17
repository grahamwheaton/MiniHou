use super::*;
use crate::session::Session;

fn sample() -> Geometry {
    let mut g = Geometry::from_arrays(vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., f32::NAN], vec![0, 1, 2, 3, 0, 1], vec![0, 4, 6], Some(vec![true, false])).unwrap();
    g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0., 0., 1., 0., 1., 1., 0., 1., f32::INFINITY, -0., 0.5, 0.25])));
    g.prim.insert("group_top".into(), Arc::new(Attr::i32(1, vec![1, 0])));
    g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![0, 1])));
    g.point.insert("m".into(), Arc::new(Attr::f32(4, (0..16).map(|x| x as f32 * 0.1).collect())));
    g.detail.insert("materials".into(), json!(["steel", "glass"]));
    g.detail.insert("material_bindings".into(), json!({"version":1,"slots":["lib:steel@3", null]}));
    g.detail.insert("note".into(), json!({"nested":[1, 2.5, "x", null, true]}));
    g
}
fn same(a: &Geometry, b: &Geometry) {
    assert_eq!(a.npoints, b.npoints);
    assert_eq!(a.topology.vertex_point, b.topology.vertex_point);
    assert_eq!(a.topology.prim_offsets, b.topology.prim_offsets);
    assert_eq!(a.topology.closed_mask(), b.topology.closed_mask());
    assert_eq!(a.detail, b.detail);
    for (ta, tb) in [(&a.point, &b.point), (&a.vertex, &b.vertex), (&a.prim, &b.prim)] {
        assert_eq!(ta.keys().collect::<Vec<_>>(), tb.keys().collect::<Vec<_>>());
        for (k, x) in ta {
            let y = &tb[k];
            assert_eq!(x.width, y.width, "{k}");
            match (&x.data, &y.data) {
                (AttrData::F32(u), AttrData::F32(v)) => assert_eq!(u.iter().map(|f| f.to_bits()).collect::<Vec<_>>(), v.iter().map(|f| f.to_bits()).collect::<Vec<_>>(), "{k}"),
                (u, v) => assert_eq!(u, v, "{k}"),
            }
        }
    }
}
fn params(code: &str) -> Params {
    let mut m = Map::new();
    m.insert("code".into(), json!(code));
    Params::resolve(crate::nodes::get("python").unwrap().params, Some(&m)).unwrap()
}

#[test]
fn decide_truth_table() {
    assert_eq!(decide(false, true, false), Decision::Disabled);
    assert_eq!(decide(false, false, true), Decision::Disabled);
    assert_eq!(decide(true, false, false), Decision::NeedsConfirmation);
    assert_eq!(decide(true, false, true), Decision::Skipped);
    assert_eq!(decide(true, true, false), Decision::Run);
}

#[test]
fn gate_is_denied_by_default_and_confirms_once_per_snippet() {
    let g = PythonGate::default();
    assert!(!g.allowed());
    assert_eq!(g.check("x=1"), Decision::Disabled);
    assert!(g.pending().is_empty(), "nothing is queued while disabled");
    g.set_allowed(true);
    assert_eq!(g.check("x=1"), Decision::NeedsConfirmation);
    assert_eq!(g.check("x=1"), Decision::NeedsConfirmation);
    assert_eq!(g.pending(), vec!["x=1".to_owned()], "asked once");
    g.approve("x=1");
    assert_eq!(g.check("x=1"), Decision::Run);
    assert_eq!(g.check("x=1 "), Decision::NeedsConfirmation, "a different snippet asks again");
    g.skip("x=1 ");
    assert_eq!(g.check("x=1 "), Decision::Skipped);
    assert!(g.pending().is_empty());
    g.set_allowed(false);
    assert_eq!(g.check("x=1"), Decision::Disabled, "switching off wins over earlier approvals");
    assert_ne!(code_hash("a"), code_hash("b"));
}

#[test]
fn wire_format_round_trips_everything() {
    let g = sample();
    let p = params("pass");
    let job = encode_job([Some(&g), None, Some(&Geometry::points_only(vec![]).unwrap()), None], &p).unwrap();
    same(&decode_job_input(&job, 0).unwrap().unwrap(), &g);
    assert!(decode_job_input(&job, 1).unwrap().is_none());
    assert_eq!(decode_job_input(&job, 2).unwrap().unwrap().npoints, 0);
    let mut arrays = geometry_arrays("", &g);
    arrays.push(("point/flagged".into(), "|b1", vec![4], vec![1, 0, 0, 1]));
    let mut out = decode_result(&encode_frame(&json!({"type":"python_result","version":1,"npoints":4,"detail":g.detail}), &arrays)).unwrap();
    assert_eq!(out.point["flagged"].as_i32().unwrap(), &[1, 0, 0, 1]);
    out.point.remove("flagged");
    same(&out, &g);
    assert!(decode_result(&encode_frame(&json!({"type":"python_result","version":2,"npoints":4}), &arrays)).unwrap_err().contains("version"));
    assert!(decode_result(b"MHL1garbage").is_err());
    let bad = [("point/P".to_owned(), "<f4", vec![1, 3], vec![0; 12]),
        ("topo/vertex_point".to_owned(), "<i4", vec![1], 5i32.to_le_bytes().to_vec()),
        ("topo/prim_offsets".to_owned(), "<i4", vec![2], [0i32, 1].iter().flat_map(|x| x.to_le_bytes()).collect())];
    assert!(decode_result(&encode_frame(&json!({"type":"python_result","version":1,"npoints":1}), &bad)).is_err(), "indices out of range");
}

// ---- fake runners: tiny Python scripts standing in for tools/python_node_runner.py
fn python() -> Option<Runtime> {
    match discover() { Ok(r) => Some(r), Err(e) => { eprintln!("skipping: {e}"); None } }
}
fn gate_with(runner: String) -> Option<Arc<PythonGate>> {
    let mut rt = python()?;
    rt.runner = Some(runner);
    let g = Arc::new(PythonGate::denied());
    g.set_runtime(rt);
    g.set_allowed(true);
    Some(g)
}
fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
fn ok_frame() -> Vec<u8> { encode_frame(&json!({"type":"python_result","version":1,"npoints":4,"detail":sample().detail}), &geometry_arrays("", &sample())) }
fn graph(code: &str) -> Session {
    Session::from_json(&json!({"version":1,"nodes":{"py":{"type":"python","params":{"code":code}}},"display":"py"}).to_string()).unwrap()
}

#[test]
fn permission_denied_without_gate_and_until_confirmed() {
    let mut s = graph("out = None");
    let e = s.cook(None).unwrap_err().to_string();
    assert!(e.contains(DISABLED_MESSAGE), "{e}");
    let g = Arc::new(PythonGate::denied());
    s.set_python_gate(Some(g.clone()));
    assert!(s.cook(None).unwrap_err().to_string().contains(DISABLED_MESSAGE));
    g.set_allowed(true);
    assert!(s.cook(None).unwrap_err().to_string().contains(CONFIRM_MESSAGE));
    assert_eq!(g.pending(), vec!["out = None".to_owned()]);
    g.skip("out = None");
    assert!(s.cook(None).unwrap_err().to_string().contains(SKIPPED_MESSAGE));
    let plain = crate::Graph::from_json(&json!({"version":1,"nodes":{"py":{"type":"python"}},"display":"py"}).to_string()).unwrap();
    assert!(plain.cook(None).unwrap_err().to_string().contains(DISABLED_MESSAGE), "Graph::cook has no gate at all");
}

#[test]
fn fake_success_is_decoded_and_cached_while_failures_are_not() {
    let dir = std::env::temp_dir().join(format!("minihou-py-count-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let count = dir.join("runs.txt");
    let _ = std::fs::remove_file(&count);
    let flag = dir.join("fail.txt");
    std::fs::write(&flag, "1").unwrap();
    let src = format!("import sys, os\nopen({c:?}, 'a').write('x')\nif os.path.exists({f:?}):\n    sys.stderr.write('boom from fake runner\\n'); sys.exit(3)\nopen(os.path.join(sys.argv[1], 'out.mhl'), 'wb').write(bytes.fromhex('{h}'))\n",
        c = count.to_str().unwrap(), f = flag.to_str().unwrap(), h = hex(&ok_frame()));
    let Some(g) = gate_with(src) else { return };
    let mut s = graph("out = geo");
    s.set_python_gate(Some(g.clone()));
    g.approve("out = geo");
    let runs = || std::fs::read_to_string(&count).map_or(0, |s| s.len());
    let e = s.cook(None).unwrap_err().to_string();
    assert!(e.contains("exit 3") && e.contains("boom from fake runner"), "{e}");
    assert!(s.cook(None).is_err());
    assert_eq!(runs(), 2, "a failure is not cached and is not retried automatically");
    std::fs::remove_file(&flag).unwrap();
    same(&s.cook(None).unwrap(), &sample());
    assert_eq!(runs(), 3);
    s.cook(None).unwrap();
    assert_eq!(runs(), 3, "success is cached");
    assert_eq!(s.last_stats().cached, vec!["py".to_owned()]);
    g.set_timeout(Duration::from_secs(29));
    s.cook(None).unwrap();
    assert_eq!(runs(), 4, "the runtime limits are part of the key");
    g.set_allowed(false);
    assert!(s.cook(None).unwrap_err().to_string().contains(DISABLED_MESSAGE), "switching off is never served from the cache");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn fake_timeout_kills_the_process_tree() {
    let marker = std::env::temp_dir().join(format!("minihou-py-grandchild-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let child = format!("import time; time.sleep(3); open({m:?}, 'w').write('alive')", m = marker.to_str().unwrap());
    let src = format!("import subprocess, sys, time\nsubprocess.Popen([sys.executable, '-c', {child:?}])\ntime.sleep(60)\n");
    let Some(g) = gate_with(src) else { return };
    g.set_timeout(Duration::from_millis(1500));
    let t = Instant::now();
    let e = run(&g, [None; 4], &params("x")).unwrap_err().to_string();
    assert!(e.contains("timed out after 1.5 s"), "{e}");
    assert!(t.elapsed() < Duration::from_secs(10));
    std::thread::sleep(Duration::from_secs(4));
    assert!(!marker.exists(), "the grandchild survived the kill");
}

#[test]
fn fake_malformed_oversized_and_missing_output() {
    let write = |body: &str| format!("import sys, os\nopen(os.path.join(sys.argv[1], 'out.mhl'), 'wb').write({body})\n");
    let Some(g) = gate_with(write("b'MHL1 not a frame'")) else { return };
    assert!(run(&g, [None; 4], &params("x")).unwrap_err().to_string().contains("malformed result"));
    let g = gate_with(write("b'0' * 5000")).unwrap();
    g.set_max_output_bytes(1000);
    assert!(run(&g, [None; 4], &params("x")).unwrap_err().to_string().contains("over the 1000 byte limit"));
    let g = gate_with("pass\n".into()).unwrap();
    assert!(run(&g, [None; 4], &params("x")).unwrap_err().to_string().contains("without writing a result"));
}

#[test]
fn runner_sees_a_fresh_cwd_and_a_scrubbed_environment() {
    // SAFETY: test-only; no other test reads this variable.
    unsafe { std::env::set_var("MINIHOU_TEST_SECRET", "s3cret"); }
    let src = "import os, sys\nassert 'MINIHOU_TEST_SECRET' not in os.environ, 'leaked'\nassert os.path.samefile(os.getcwd(), sys.argv[1]), os.getcwd()\nsys.exit(5)\n".to_owned();
    let Some(g) = gate_with(src) else { return };
    let e = run(&g, [None; 4], &params("x")).unwrap_err().to_string();
    assert!(e.contains("exit 5"), "{e}");
}

// ---- the real runner against this machine's Python, golden-compared with the Python engine
fn python_engine(graph: &Value, rt: &Runtime) -> Option<Value> {
    let script = "import json, sys\nfrom minihou.graph import Graph\nfrom minihou.evaluator import Evaluator\nout = Evaluator().cook(Graph.from_json(sys.argv[1]), 'py')\nd = {k: {n: out.table(k)[n].tolist() for n in out.table(k).names()} for k in ('point', 'vertex', 'prim')}\nd['detail'] = out.detail\nprint(json.dumps(d))\n";
    let mut cmd = base_command(rt, Path::new(env!("CARGO_MANIFEST_DIR")));
    cmd.env("MINIHOU_PYTHON_NODES", "1").args(["-c", script, &graph.to_string()]);
    let o = cmd.output().ok()?;
    if !o.status.success() { eprintln!("skipping golden: {}", String::from_utf8_lossy(&o.stderr)); return None; }
    serde_json::from_slice(&o.stdout).ok()
}
fn as_json(g: &Geometry) -> Value {
    fn rows<T: Copy + Into<Value>>(v: &[T], w: usize) -> Value {
        Value::Array(v.chunks(w).map(|r| if w == 1 { r[0].into() } else { Value::Array(r.iter().map(|x| (*x).into()).collect()) }).collect())
    }
    let t = |t: &crate::geometry::AttrTable| Value::Object(t.iter().map(|(k, a)| (k.clone(), match &a.data {
        AttrData::F32(v) => rows(&v.iter().map(|x| *x as f64).collect::<Vec<_>>(), a.width),
        AttrData::I32(v) => rows(v, a.width),
    })).collect());
    json!({"point": t(&g.point), "vertex": t(&g.vertex), "prim": t(&g.prim), "detail": g.detail})
}

#[test]
fn real_runner_matches_the_python_engine() {
    let Some(rt) = python() else { return };
    let g = Arc::new(PythonGate::denied());
    g.set_runtime(rt.clone());
    g.set_allowed(true);
    let default = crate::nodes::get("python").unwrap().params.iter().find(|p| p.name == "code").map(|p| (p.default)()).unwrap();
    let snippet = "out = geo.copy()\nP = geo.P.copy()\nP[:, 1] += v[1] + a\nout.P = P\nout.point['w'] = np.arange(geo.npoints, dtype=np.float32) * scale\nout.prim['id'] = np.arange(geo.nprims, dtype=np.int32)\nout.detail['materials'] = ['red', s]\n";
    for (code, extra) in [(default, json!({"a":0.5,"b":2.0})), (json!(snippet), json!({"a":0.25,"v":[0,1,0],"s":"blue","variables":"scale = 2.5"}))] {
        let mut params = extra.as_object().unwrap().clone();
        params.insert("code".into(), code.clone());
        let graph = json!({"version":1,"nodes":{"g":{"type":"grid","params":{"columns":3,"rows":2}},"py":{"type":"python","inputs":{"geo":"g"},"params":params}},"display":"py"});
        let mut s = Session::from_json(&graph.to_string()).unwrap();
        s.set_python_gate(Some(g.clone()));
        g.approve(code.as_str().unwrap());
        let out = s.cook(None).unwrap_or_else(|e| panic!("{e}"));
        assert_ne!(out.p(), s.cook(Some("g")).unwrap().p(), "the snippet moved points");
        let Some(golden) = python_engine(&graph, &rt) else { return };
        assert_eq!(as_json(&out), golden, "Rust bridge vs Python engine for {code}");
    }
    let mut s = graph("x = 1\nraise ValueError('nope')");
    s.set_python_gate(Some(g.clone()));
    g.approve("x = 1\nraise ValueError('nope')");
    let e = s.cook(None).unwrap_err().to_string();
    assert!(e.contains("line 2") && e.contains("nope"), "{e}");
}
