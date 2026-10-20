use super::*;

fn document(outputs: &Outputs, o: &WriteOptions, frame: f64) -> Result<(sdf::Data, Value)> {
    let mpu = UNITS
        .iter()
        .find(|(n, _)| *n == o.units)
        .map(|(_, v)| *v)
        .ok_or_else(|| {
            err("units must be one of meters, centimeters, millimeters, inches, feet")
        })?;
    if !["z", "y"].contains(&o.up_axis.as_str()) {
        return Err(err("up_axis must be 'z' or 'y'"));
    }
    let top = valid_name(if o.root.is_empty() {
        "MiniHou"
    } else {
        &o.root
    });
    let graph = o
        .graph
        .as_ref()
        .map(|g| format!("string graph = {}", q(&g.to_string())))
        .unwrap_or_default();
    let mut text = format!(
        "#usda 1.0\n(defaultPrim = {} upAxis = {} metersPerUnit = {} documentation = \"Exported from MiniHou (Rust)\" customLayerData = {{dictionary minihou = {{ int format = 1 string outputs = {} double frame = {} {graph} }} }})\ndef Xform {} {{\n",
        q(&top),
        q(&o.up_axis.to_uppercase()),
        num(mpu),
        q(&json!(outputs.keys().collect::<Vec<_>>()).to_string()),
        num(frame),
        q(&top)
    );
    if o.up_axis == "y" {
        text.push_str("float3 xformOp:rotateXYZ = (-90, 0, 0)\nuniform token[] xformOpOrder = [\"xformOp:rotateXYZ\"]\n");
    }
    let looks = if o.materials {
        looks(&mut text, outputs, &top)
    } else {
        BTreeMap::new()
    };
    let mut used = BTreeSet::from(["Looks".into()]);
    let mut reports = serde_json::Map::new();
    for (name, g) in outputs {
        let nm = unique(name, &mut used, "out");
        let mut report = geometry(&mut text, g, &nm, 1. / mpu, &looks)?;
        report["prim"] = json!(format!("/{top}/{nm}"));
        reports.insert(name.clone(), report);
    }
    text.push_str("}\n");
    let data = openusd::usda::parse(&text).map_err(err)?;
    Ok((data, Value::Object(reports)))
}

/// Write a single stage through the same encoder used by animation, nodes and the editor.
fn merge_prototypes(data: &mut sdf::Data, merged: &sdf::Data) -> Result<()> {
    for path in data
        .spec_paths()
        .into_iter()
        .filter(|p| p.to_string().ends_with(".prototypes"))
    {
        let Some(new) = data.try_field(&path, "targetPaths").map_err(err)? else {
            continue;
        };
        let U::PathListOp(new) = new.into_owned() else {
            continue;
        };
        let old = merged
            .try_field(&path, "targetPaths")
            .map_err(err)?
            .map(|v| v.into_owned());
        let mut targets = match old {
            Some(U::PathListOp(v)) => v.explicit_items().to_vec(),
            _ => vec![],
        };
        let mut mapping = vec![];
        for p in new.explicit_items() {
            let index = targets.iter().position(|t| t == p).unwrap_or_else(|| {
                targets.push(p.clone());
                targets.len() - 1
            });
            mapping.push(index as i32);
        }
        let index_path = sdf::Path::new(
            &(path.to_string().trim_end_matches("prototypes").to_owned() + "protoIndices"),
        )
        .map_err(err)?;
        let values = data
            .try_field(&index_path, "default")
            .map_err(err)?
            .map(|v| v.into_owned());
        if let Some(U::IntVec(v)) = values {
            let remapped = v
                .into_iter()
                .map(|i| {
                    mapping
                        .get(i as usize)
                        .copied()
                        .ok_or_else(|| err("invalid protoIndices"))
                })
                .collect::<Result<Vec<_>>>()?;
            data.set_field(&index_path, "default", U::IntVec(remapped));
        }
        data.set_field(
            &path,
            "targetPaths",
            U::PathListOp(sdf::PathListOp::explicit(targets)),
        );
    }
    Ok(())
}

pub fn write_stage(path: &Path, outputs: &Outputs, o: &WriteOptions, frame: f64) -> Result<Value> {
    write_animation(
        path,
        std::iter::once(Ok((frame, outputs.clone()))),
        o,
        false,
    )
}
/// Consumes samples incrementally. Cooked geometry is released after each frame;
/// only the authored USD layer and its samples remain in memory.
pub fn write_animation(
    path: &Path,
    frames: impl IntoIterator<Item = Result<(f64, Outputs)>>,
    o: &WriteOptions,
    animated: bool,
) -> Result<Value> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["usd", "usda", "usdc", "usdz"].contains(&ext.as_str()) {
        return Err(err("a USD file must end in .usd, .usda or .usdc"));
    }
    if path.exists() && !o.overwrite {
        return Err(err(format!(
            "{} already exists",
            path.file_name().unwrap_or_default().to_string_lossy()
        )));
    }
    if !o.fps.is_finite() || o.fps <= 0. {
        return Err(err("fps must be positive and finite"));
    }
    let mut merged = sdf::Data::new();
    let mut times = vec![];
    let mut report = json!({});
    for sample in frames {
        let (frame, outputs) = sample?;
        if !frame.is_finite() || times.last().is_some_and(|&t| frame <= t) {
            return Err(err(
                "animation frames must be finite and strictly increasing",
            ));
        }
        let (mut data, r) = document(&outputs, o, frame)?;
        if times.is_empty() {
            report = r;
        }
        merge_prototypes(&mut data, &merged)?;
        for p in data.spec_paths() {
            let new = !merged.has_spec(&p);
            if new {
                merged.create_spec(p.clone(), data.spec_type(&p).unwrap());
            }
            for key in data.list_fields(&p).unwrap_or_default() {
                let v = data.field(&p, &key).map_err(err)?.into_owned();
                if animated
                    && key == "default"
                    && data
                        .try_field(&p, "variability")
                        .map_err(err)?
                        .is_none_or(|v| v.as_ref() != &U::Variability(sdf::Variability::Uniform))
                {
                    // Shading consumers commonly query defaults even for an animated stage.
                    if new && p.to_string().contains("/Looks/") {
                        merged.set_field(&p, "default", v.clone());
                    }
                    let mut samples = match merged.try_field(&p, "timeSamples").map_err(err)? {
                        Some(v) => match v.into_owned() {
                            U::TimeSamples(v) => v,
                            _ => vec![],
                        },
                        None => vec![],
                    };
                    if new && !times.is_empty() {
                        samples.push((
                            times[0],
                            if p.to_string().ends_with(".visibility") {
                                U::token("invisible")
                            } else {
                                U::ValueBlock
                            },
                        ));
                    }
                    samples.push((frame, v));
                    merged.set_field(&p, "timeSamples", U::TimeSamples(samples));
                } else if new || !merged.has_field(&p, &key) {
                    merged.set_field(&p, &key, v);
                } else if key == "targetPaths" && p.to_string().ends_with(".prototypes") {
                    merged.set_field(&p, &key, v);
                } else if key == "typeName" && merged.field(&p, &key).map_err(err)?.as_ref() != &v {
                    return Err(err(format!("{p}: attribute type changes between frames")));
                } else if key == "primChildren" || key == "propertyChildren" {
                    // union children/properties when topology introduces a new prim
                    if let U::TokenVec(mut v) = v {
                        if let U::TokenVec(old) = merged.field(&p, &key).map_err(err)?.as_ref() {
                            for n in old {
                                if !v.contains(n) {
                                    v.push(n.clone());
                                }
                            }
                        }
                        merged.set_field(&p, &key, U::TokenVec(v));
                    }
                }
            }
        }
        if animated {
            for p in merged.spec_paths() {
                if merged.spec_type(&p) == Some(sdf::SpecType::Attribute) && !data.has_spec(&p) {
                    let mut samples = match merged.try_field(&p, "timeSamples").map_err(err)? {
                        Some(v) => match v.into_owned() {
                            U::TimeSamples(v) => v,
                            _ => continue,
                        },
                        None => continue,
                    };
                    samples.push((
                        frame,
                        if p.to_string().ends_with(".visibility") {
                            U::token("invisible")
                        } else {
                            U::ValueBlock
                        },
                    ));
                    merged.set_field(&p, "timeSamples", U::TimeSamples(samples));
                }
            }
        }
        times.push(frame);
    }
    if times.is_empty() {
        return Err(err("no frames to export"));
    }
    if animated {
        let root = sdf::Path::abs_root();
        for (k, v) in [
            ("startTimeCode", times[0]),
            ("endTimeCode", *times.last().unwrap()),
            ("timeCodesPerSecond", o.fps),
            ("framesPerSecond", o.fps),
        ] {
            merged.set_field(&root, k, U::Double(v));
        }
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(err)?;
    // Layer::export uses the library's atomic replacement writer (including Windows).
    let mut bytes = vec![];
    openusd::usda::TextWriter::write(&merged, &mut bytes).map_err(err)?;
    let layer = sdf::Layer::from_bytes("minihou.usda", bytes).map_err(err)?;
    layer.export(path.to_string_lossy().as_ref()).map_err(err)?;
    Ok(
        json!({"ok":true,"path":path,"outputs":report,"frames":if animated{Some(json!([times[0],times.last(),times.len()]))}else{None},"frame":times[0],"message":format!("Wrote {}",path.display())}),
    )
}

/// Relative file node paths start at the workspace set by the application, or cwd.
pub fn resolve_path(file: &str) -> PathBuf {
    let p = PathBuf::from(file);
    if p.is_absolute() {
        p
    } else {
        let root = std::env::var_os("MINIHOU_WORKSPACE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let cwd = std::env::current_dir().unwrap_or_default();
                cwd.ancestors()
                    .find(|p| p.join("minihou").is_dir() && p.join("rust/Cargo.toml").is_file())
                    .map(Path::to_owned)
                    .unwrap_or(cwd)
            });
        root.join(p)
    }
}

/// Full-resolution worker/session export, preserving live inputs and restoring time on failure.
pub fn export_session(
    session: &mut crate::session::Session,
    path: &Path,
    targets: &[(String, String)],
    o: &WriteOptions,
    frames: Option<Vec<f64>>,
) -> Result<Value> {
    let (frame, fps) = session.time();
    let animated = frames.is_some();
    let frames = frames.unwrap_or_else(|| vec![frame]);
    let result = crate::nodes::usd_nodes::without_exports(|| {
        write_animation(
            path,
            frames.into_iter().map(|f| {
                session.set_time(f, fps)?;
                let mut outputs = Outputs::new();
                for (name, id) in targets {
                    outputs.insert(name.clone(), session.cook(Some(id))?);
                }
                Ok((f, outputs))
            }),
            o,
            animated,
        )
    });
    session.set_time(frame, fps)?;
    result
}
