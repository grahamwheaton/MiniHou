//! Produce interoperability fixtures without launching the editor.
use minihou_core::{usd_io::*, Attr, Geometry};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
fn main() -> minihou_core::Result<()> {
    let dir = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    let mut g = Geometry::from_arrays(
        vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
        vec![0, 1, 2],
        vec![0, 3],
        None,
    )?;
    g.vertex.insert(
        "uv".into(),
        Arc::new(Attr::f32(2, vec![0., 0., 1., 0., 0., 1.])),
    );
    g.prim
        .insert("material_index".into(), Arc::new(Attr::i32(1, vec![0])));
    g.prim
        .insert("group_front".into(), Arc::new(Attr::i32(1, vec![1])));
    g.prim
        .insert("Cd".into(), Arc::new(Attr::f32(3, vec![0.8, 0.2, 0.1])));
    g.detail.insert("materials".into(), json!(["RustRed"]));
    let opts = WriteOptions {
        overwrite: true,
        ..Default::default()
    };
    for ext in ["usda", "usdc", "usd", "usdz"] {
        let p = dir.join(format!("triangle.{ext}"));
        write_stage(
            &p,
            &Outputs::from([("Triangle".into(), Arc::new(g.clone()))]),
            &opts,
            1.,
        )?;
        println!("{}", p.display());
    }
    let samples = (1..=3).map(|f| {
        let mut geo = g.clone();
        geo.set_p(
            geo.p()
                .iter()
                .enumerate()
                .map(|(i, &x)| if i % 3 == 0 { x + f as f32 } else { x })
                .collect(),
        );
        Ok((
            f as f64,
            Outputs::from([("Triangle".into(), Arc::new(geo))]),
        ))
    });
    write_animation(&dir.join("animated.usdc"), samples, &opts, true)?;
    Ok(())
}
