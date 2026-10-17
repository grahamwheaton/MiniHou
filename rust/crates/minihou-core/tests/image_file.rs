//! Tests for image_file node (both unit and graph evaluation).
use minihou_core::{session::Session, Graph};
use serde_json::json;

fn fixture_path(name: &str) -> std::path::PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir).join("tests/fixtures").join(name)
}

#[test]
fn test_image_file_graph_cook_8bit() {
    let p = fixture_path("image_file_test.png");
    let doc = json!({
        "nodes": {
            "img": {
                "type": "image_file",
                "params": {
                    "file": p.to_str().unwrap(),
                    "colorspace": "auto",
                    "size": 10.0,
                    "center": [1.0, 2.0, 3.0],
                    "height": "red",
                    "height_scale": 2.0
                }
            }
        },
        "display": "img"
    });
    let graph = Graph::from_value(&doc).unwrap();
    let g = graph.cook(None).unwrap();
    assert_eq!(g.npoints, 12);
    assert_eq!(g.nprims(), 6);
    assert_eq!(g.detail["grid_columns"], json!(4));
    assert_eq!(g.detail["grid_rows"], json!(3));
    assert_eq!(g.detail["image"], json!("image_file_test.png"));
    assert!(g.point.contains_key("Cd"));
    assert!(g.point.contains_key("Alpha"));
    assert!(g.point.contains_key("uv"));
    assert!(g.point.contains_key("height"));
}

#[test]
fn test_image_file_graph_cook_16bit() {
    let p = fixture_path("image_file_16bit.png");
    let doc = json!({
        "nodes": {
            "img": {
                "type": "image_file",
                "params": {
                    "file": p.to_str().unwrap(),
                    "colorspace": "stored",
                    "size": 5.0,
                    "center": [0.0, 0.0, 0.0],
                    "height": "luminance",
                    "height_scale": 1.5
                }
            }
        },
        "display": "img"
    });
    let graph = Graph::from_value(&doc).unwrap();
    let g = graph.cook(None).unwrap();
    assert_eq!(g.npoints, 20);
    assert_eq!(g.nprims(), 12);
    assert_eq!(g.detail["grid_columns"], json!(5));
    assert_eq!(g.detail["grid_rows"], json!(4));
    assert_eq!(g.detail["image"], json!("image_file_16bit.png"));
    assert!(!g.point.contains_key("Alpha")); // fully opaque
}

#[test]
fn test_image_file_session_caching_and_recook() {
    let dir = std::env::temp_dir().join(format!("minihou_test_image_file_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("recook_test.png");
    
    // Copy 8-bit image to temp dir
    let src = fixture_path("image_file_test.png");
    std::fs::copy(&src, &path).unwrap();

    let doc = json!({
        "nodes": {
            "img": {
                "type": "image_file",
                "params": {
                    "file": path.to_str().unwrap()
                }
            }
        },
        "display": "img"
    });

    let mut session = Session::new(Graph::from_value(&doc).unwrap());
    let g1 = session.cook(None).unwrap();
    assert_eq!(g1.npoints, 12);

    // Cook again without changes: should be served from cache
    let g2 = session.cook(None).unwrap();
    assert_eq!(g2.npoints, 12);

    // Overwrite with 16-bit image (different size and mtime)
    let src16 = fixture_path("image_file_16bit.png");
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::copy(&src16, &path).unwrap();

    // Session cook should notice timestamp/size change and recook!
    let g3 = session.cook(None).unwrap();
    assert_eq!(g3.npoints, 20);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_image_file_downsample_and_errors() {
    let p = fixture_path("image_file_16bit.png"); // 5x4 image
    
    // max_size=2: factor = ceil(5/2) = 3 -> w = 5/3 = 1, h = 4/3 = 1 -> w < 2 || h < 2 error
    let doc_small = json!({
        "nodes": {
            "img": {
                "type": "image_file",
                "params": {
                    "file": p.to_str().unwrap(),
                    "max_size": 2
                }
            }
        },
        "display": "img"
    });
    let err = Graph::from_value(&doc_small).unwrap().cook(None).unwrap_err();
    assert!(err.0.contains("an image needs at least 2 x 2 pixels to make a grid"));

    // empty file error
    let doc_empty = json!({
        "nodes": {
            "img": {
                "type": "image_file",
                "params": {
                    "file": ""
                }
            }
        },
        "display": "img"
    });
    let err = Graph::from_value(&doc_empty).unwrap().cook(None).unwrap_err();
    assert!(err.0.contains("set 'file' to a PNG file"));

    // nonexistent file error
    let doc_missing = json!({
        "nodes": {
            "img": {
                "type": "image_file",
                "params": {
                    "file": "nonexistent_file_xyz.png"
                }
            }
        },
        "display": "img"
    });
    let err = Graph::from_value(&doc_missing).unwrap().cook(None).unwrap_err();
    assert!(err.0.contains("no such file:"));
}

#[test]
fn test_image_file_edge_cases_and_determinism() {
    let p = fixture_path("image_file_test.png");
    // Invalid colorspace enum
    let doc_bad_cs = json!({
        "nodes": {
            "img": {
                "type": "image_file",
                "params": {
                    "file": p.to_str().unwrap(),
                    "colorspace": "invalid_cs"
                }
            }
        },
        "display": "img"
    });
    assert!(Graph::from_value(&doc_bad_cs).unwrap().cook(None).is_err());

    // Invalid height enum
    let doc_bad_h = json!({
        "nodes": {
            "img": {
                "type": "image_file",
                "params": {
                    "file": p.to_str().unwrap(),
                    "height": "infrared"
                }
            }
        },
        "display": "img"
    });
    assert!(Graph::from_value(&doc_bad_h).unwrap().cook(None).is_err());

    // Huge max_size doesn't break, clamps factor to 1
    let doc_huge = json!({
        "nodes": {
            "img": {
                "type": "image_file",
                "params": {
                    "file": p.to_str().unwrap(),
                    "max_size": 1_000_000
                }
            }
        },
        "display": "img"
    });
    let g_huge = Graph::from_value(&doc_huge).unwrap().cook(None).unwrap();
    assert_eq!(g_huge.npoints, 12);

    // Determinism: cooking twice produces identical geometry and attributes
    let doc_det = json!({
        "nodes": {
            "img": {
                "type": "image_file",
                "params": {
                    "file": p.to_str().unwrap(),
                    "height": "luminance"
                }
            }
        },
        "display": "img"
    });
    let g1 = Graph::from_value(&doc_det).unwrap().cook(None).unwrap();
    let g2 = Graph::from_value(&doc_det).unwrap().cook(None).unwrap();
    assert_eq!(g1.p(), g2.p());
    assert_eq!(g1.point["Cd"].data, g2.point["Cd"].data);
    assert_eq!(g1.point["uv"].data, g2.point["uv"].data);
    assert_eq!(g1.point["height"].data, g2.point["height"].data);
}

