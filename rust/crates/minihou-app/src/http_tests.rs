use super::*;
use crate::http_api::{HttpServer, Response};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

fn base() -> Value {
    json!({"version":1,"nodes":{"box1":{"type":"box","pos":[0,0]},"output1":{"type":"output","inputs":{"geo":"box1"},"pos":[0,150]}},"display":"output1"})
}
#[test]
fn http_ops_attrib_paint_assign_roundtrip() {
    let graph = json!({"version":1,"nodes":{"g":{"type":"grid","params":{"columns":2,"rows":2}},"p":{"type":"attrib_paint","inputs":{"geo":"g"},"params":{"attribute":"w"}}},"display":"p"});
    let (graph, results) = http_ops::apply_ops(&graph, &json!([
        {"op":"paint_assign","id":"p","indices":[0,2],"values":[0.25,0.75]},
        {"op":"paint_assign","id":"p","indices":[2],"value":1.0}
    ])).unwrap();
    let data = graph["nodes"]["p"]["params"]["data"].as_str().unwrap();
    assert_eq!(results[1]["bytes"], data.len());
    assert_eq!(minihou_core::nodes::attrib_paint::decode(data, 1).unwrap(), vec![(0,vec![0.25]),(2,vec![1.0])]);
    let doc = Document::parse(graph.to_string()).unwrap();
    let out = doc.session().unwrap().cook(None).unwrap();
    assert_eq!(out.point["w"].as_f32().unwrap(), &[0.25, 0., 1., 0.]);
    let (graph, _) = http_ops::apply_ops(&graph, &json!([{"op":"paint_assign","id":"p","indices":[0],"erase":true}])).unwrap();
    assert_eq!(minihou_core::nodes::attrib_paint::decode(graph["nodes"]["p"]["params"]["data"].as_str().unwrap(), 1).unwrap(), vec![(2,vec![1.0])]);
}
fn http(address: std::net::SocketAddr, method: &str, path: &str, body: Value) -> (u16, Value) {
    let mut socket = TcpStream::connect(address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let data = body.to_string();
    write!(socket,"{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{data}",address.port(),data.len()).unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        serde_json::from_str(body).unwrap(),
    )
}

#[test]
fn http_batch_real_socket_history_worker_inspection_and_python_mcp() {
    // An isolated, windowless editor pump uses the production transport, operation
    // validator, History and CookWorker. It never binds the app's default ports.
    let server = HttpServer::start("127.0.0.1:0".parse().unwrap(), || {}).unwrap();
    let address = server.address;
    assert!(HttpServer::start(address, || {}).is_err());
    let (stop, stopped) = std::sync::mpsc::channel::<()>();
    let pump = std::thread::spawn(move || {
        let doc = Document::parse(base().to_string()).unwrap();
        let mut history = History::default();
        let mut doc = doc;
        let worker = CookWorker::start(&doc, |_| ()).unwrap();
        let mut cooking = CookState::default();
        let mut edit = |edit| {
            cooking.requested += 1;
            worker
                .send(CookRequest {
                    revision: cooking.requested,
                    edit,
                })
                .unwrap();
            loop {
                if let Some(response) = worker.poll() {
                    cooking.accept(response);
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            json!({"ok":cooking.error.is_none(),"error":cooking.error.as_ref().map(|s|http_api::error_detail(s)),"points":cooking.geometry.as_ref().map_or(0,|g|g.npoints),"prims":cooking.geometry.as_ref().map_or(0,|g|g.nprims())})
        };
        edit(Edit::Replace(base()));
        while stopped.try_recv().is_err() {
            for r in server.poll() {
                let response = match r.path.as_str() {
                    "/api/edit" => match http_ops::apply_ops(doc.value(), &r.body["ops"]) {
                        Ok((g, results)) => {
                            let changed = g != *doc.value();
                            if changed {
                                history.replace(&mut doc, Document::parse(g.to_string()).unwrap());
                            }
                            let status = edit(Edit::Replace(g));
                            Response::ok(
                                json!({"changed":changed,"rev":1,"results":results,"status":status,"graph":{"graph":doc.value()}}),
                            )
                        }
                        Err(e) => Response::error(400, e),
                    },
                    "/api/graph" => {
                        Response::ok(json!({"graph":doc.value(),"rev":1,"path":"fixture.json"}))
                    }
                    "/api/nodes" => Response::ok(http_api::node_types()),
                    "/api/undo" | "/api/redo" => {
                        let e = if r.path.ends_with("undo") {
                            history.undo(&mut doc)
                        } else {
                            history.redo(&mut doc)
                        };
                        let ok = e.is_some();
                        if let Some(e) = e {
                            edit(e);
                        }
                        Response::ok(json!({"ok":ok}))
                    }
                    "/api/inspect" => {
                        let (tx, rx) = std::sync::mpsc::channel();
                        worker
                            .query(http_geometry::Query {
                                kind: http_geometry::QueryKind::Inspect {
                                    node: r.body["node"].as_str().unwrap().into(),
                                    detail: true,
                                },
                                revision: 1,
                                reply: tx,
                            })
                            .unwrap();
                        rx.recv_timeout(Duration::from_secs(10)).unwrap()
                    }
                    _ => Response::error(404, "no such endpoint"),
                };
                let _ = r.reply.send(response);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    let ops = json!([
        {"op":"add","id":"shape","type":"box","params":{"size":[2,4,6]},"comment":"MCP geometry"},
        {"op":"connect","from":"shape","to":"output1"},
        {"op":"expose","id":"shape","param":"size","name":"Dimensions"},
        {"op":"display","id":"output1"},
        {"op":"box_add","around":["shape","output1"],"title":"Created via HTTP"}
    ]);
    let (status, r) = http(
        address,
        "POST",
        "/api/edit",
        json!({"ops":ops,"wait":true,"origin":"mcp"}),
    );
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["status"]["ok"], true, "{r}");
    assert_eq!(r["results"][0]["id"], "shape");
    assert_eq!(r["results"][2]["name"], "Dimensions");
    assert_eq!(r["status"]["points"], 8);
    let (_, inspect) = http(
        address,
        "POST",
        "/api/inspect",
        json!({"node":"output1","detail":true}),
    );
    assert_eq!(inspect["bounds"]["size"], json!([2., 4., 6.]));
    let attrs = inspect["attributes"].as_array().unwrap();
    assert!(attrs
        .iter()
        .any(|a| a["name"] == "P" && a["components"] == 3 && a["mean"] == json!([0., 0., 0.])));
    let (_, before) = http(address, "GET", "/api/graph", json!({}));
    for bad in [
        json!([{"op":"set","id":"shape","params":{"size":[9,9,9]}},{"op":"add","id":"invalid","type":"python"}]),
        json!([{"op":"set","id":"shape","params":{"size":[9,9,9]}},{"op":"connect","from":"output1","to":"shape","port":"bogus"}]),
    ] {
        let (code, error) = http(address, "POST", "/api/edit", json!({"ops":bad,"wait":true}));
        assert_eq!(code, 400, "{error}");
        let (_, after) = http(address, "GET", "/api/graph", json!({}));
        assert_eq!(before, after);
    }
    assert_eq!(http(address, "POST", "/api/undo", json!({})).1["ok"], true);
    assert_eq!(
        http(address, "GET", "/api/graph", json!({})).1["graph"],
        base()
    );
    assert_eq!(http(address, "POST", "/api/redo", json!({})).1["ok"], true);
    let (_, failed) = http(
        address,
        "POST",
        "/api/edit",
        json!({"ops":[{"op":"disconnect","to":"output1"}],"wait":true}),
    );
    assert_eq!(failed["changed"], true);
    assert_eq!(failed["status"]["ok"], false);
    assert_eq!(failed["status"]["error"]["node"], "output1");
    assert_eq!(http(address, "POST", "/api/undo", json!({})).1["ok"], true);
    // Exercise the unmodified Python EngineClient and MCP result formatting over the socket.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap();
    let output=std::process::Command::new(std::env::var_os("MINIHOU_PYTHON").unwrap_or_else(||"python".into())).current_dir(root).arg("-c").arg(r#"
import sys
from minihou.mcp import EngineClient, McpServer
m = McpServer(EngineClient(sys.argv[1]))
for name,args in [('list_node_types',{'names':['box']}),('edit_graph',{'ops':[{'op':'set','id':'shape','params':{'size':[3,5,7]}},{'op':'meta','id':'shape','set':{'comment':'Python MCP -> Rust'}}]}),('inspect_node',{'node':'output1'})]:
    result=m.call_tool(name,args)
    assert not result['isError'], result
    print(name, result['content'][0]['text'][:180])
"#).arg(format!("http://{address}")).output().unwrap();
    let _ = stop.send(());
    pump.join().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn http_ops_insert_rename_reset_disconnect_exposure_meta_layout_loop() {
    let(g,r)=http_ops::apply_ops(&base(),&json!([
        {"op":"insert","after":"box1","type":"transform","id":"x","params":{"translate":[1,2,3]}},
        {"op":"rename","id":"x","to":"moved"},
        {"op":"set","id":"moved","params":{"translate":null}},
        {"op":"move","positions":{"moved":[1.26,2.54]}},
        {"op":"expose","id":"moved","param":"translate","name":"Move"},
        {"op":"meta","id":"moved","set":{"comment":"Hello"}},
        {"op":"unexpose","name":"Move"},
        {"op":"box_add","id":"box","around":["moved"],"title":"A"},
        {"op":"box_set","id":"box","title":"B"},
        {"op":"box_remove","id":"box"},
        {"op":"loop","from":"moved","to":"output1","iterations":2,"pos":[300,400]}
    ])).unwrap();
    assert_eq!(r[0]["rewired"], 1);
    assert_eq!(r[1]["id"], "moved");
    assert_eq!(g["nodes"]["moved"]["pos"], json!([1.3, 2.5]));
    assert!(g["nodes"]["moved"]["params"].get("translate").is_none());
    assert_eq!(g["nodes"]["output1"]["inputs"]["geo"], r[10]["end"]);
    assert_eq!(g["display"], "output1");
    assert!(g["exposed"].as_array().unwrap().is_empty());
    let doc = Document::parse(g.to_string()).unwrap();
    assert!(doc.session().unwrap().cook(None).is_ok());
    assert!(
        http_ops::apply_ops(&g, &json!([{"op":"connect","from":"output1","to":"moved"}]))
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    assert!(http_ops::apply_ops(
        &g,
        &json!([{"op":"meta","id":"moved","set":{"type":"python"}}])
    )
    .is_err());
    let (g, _) =
        http_ops::apply_ops(&g, &json!([{"op":"remove","id":["moved"]},{"op":"layout"}])).unwrap();
    assert!(g["nodes"].get("moved").is_none());
}

#[test]
fn http_inspect_uses_live_worker_inputs_and_time() {
    let value = json!({"nodes":{"source":{"type":"blender_object","params":{"object":"Live"}},"animated":{"type":"wrangle","inputs":{"geo":"source"},"params":{"snippet":"@P.x += @Frame;"}}},"display":"animated"});
    let doc = Document::parse(value.to_string()).unwrap();
    let worker = CookWorker::start(&doc, |_| ()).unwrap();
    worker
        .send(CookRequest {
            revision: 1,
            edit: Edit::External {
                name: "object:Live".into(),
                input: minihou_core::nodes::blender_input::ExternalInput::Object {
                    geo: std::sync::Arc::new(
                        minihou_core::Geometry::points_only(vec![2., 3., 4.]).unwrap(),
                    ),
                    matrix: None,
                    info: json!({}),
                },
            },
        })
        .unwrap();
    worker
        .send(CookRequest {
            revision: 2,
            edit: Edit::Time {
                frame: 10.,
                fps: 24.,
            },
        })
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(std::time::Instant::now() < deadline);
        if worker.poll().is_some_and(|r| r.revision == 2) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    worker
        .query(http_geometry::Query {
            kind: http_geometry::QueryKind::Inspect {
                node: "animated".into(),
                detail: true,
            },
            revision: 2,
            reply: tx,
        })
        .unwrap();
    let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(r.body["ok"], true, "{}", r.body);
    assert_eq!(r.body["bounds"]["min"], json!([12., 3., 4.]));

    // Native USD serialization must work without Python/pxr, and restore time.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap();
    let path = std::env::temp_dir().join(format!("minihou_mcp_export_{}.usda", std::process::id()));
    let (tx, rx) = std::sync::mpsc::channel();
    worker
        .query(http_geometry::Query {
            kind: http_geometry::QueryKind::Export {
                path: path.clone(),
                node: Some("animated".into()),
                graph: Some(value),
                overwrite: true,
                options: json!({"frames":"range","start":1,"end":2,"step":1,"name":"Animated"}),
                root: root.to_path_buf(),
            },
            revision: 2,
            reply: tx,
        })
        .unwrap();
    let response = rx.recv_timeout(Duration::from_secs(20)).unwrap();
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.body["frames"], json!([1., 2., 2]));
    let imported = minihou_core::usd_io::read_geometry(&path, &minihou_core::usd_io::ReadOptions { time: Some(2.), ..Default::default() }).unwrap();
    assert_eq!(imported.p()[0], 4.);
    std::fs::remove_file(&path).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    worker
        .query(http_geometry::Query {
            kind: http_geometry::QueryKind::Inspect {
                node: "animated".into(),
                detail: true,
            },
            revision: 2,
            reply: tx,
        })
        .unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)).unwrap().body["bounds"]["min"],
        json!([12., 3., 4.])
    );
}

#[test]
fn http_rejects_cross_origin_and_oversized_requests() {
    let server = HttpServer::start("127.0.0.1:0".parse().unwrap(), || {}).unwrap();
    for headers in [
        "Host: evil.example\r\n",
        "Host: 127.0.0.1\r\nOrigin: https://evil.example\r\n",
        "Host: 127.0.0.1\r\nContent-Length: 99999999\r\n",
    ] {
        let mut socket = TcpStream::connect(server.address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(socket, "POST /api/edit HTTP/1.1\r\n{headers}\r\n").unwrap();
        let mut out = String::new();
        socket.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
    }
    assert!(server.poll().is_empty());
}
