use super::*;
pub fn read_graph(path: &Path) -> Result<Option<Value>> {
    let layer = sdf::Layer::open(path.to_string_lossy().as_ref()).map_err(err)?;
    let v = layer
        .data()
        .try_field(&sdf::Path::abs_root(), "customLayerData")
        .map_err(err)?
        .map(|v| json_value(v.into_owned()))
        .transpose()?
        .unwrap_or(Value::Null);
    v["minihou"]["graph"]
        .as_str()
        .map(|s| serde_json::from_str(s).map_err(err))
        .transpose()
}
pub fn read_geometry(path: &Path, o: &ReadOptions) -> Result<Geometry> {
    if !["z", "keep"].contains(&o.up_axis.as_str()) {
        return Err(err("up_axis must be 'z' or 'keep'"));
    }
    let purpose_text = o.purposes.to_ascii_lowercase();
    let purposes: Vec<_> = purpose_text
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .collect();
    for purpose in &purposes {
        if !purposes.contains(&"all")
            && !["default", "render", "proxy", "guide", "all"].contains(purpose)
        {
            return Err(err(format!(
                "unknown purpose '{purpose}': use default, render, proxy, guide or all"
            )));
        }
    }
    let purposes = if purposes.is_empty() {
        vec!["default", "render"]
    } else {
        purposes
    };
    let mut inc = vec![];
    let mut exc = vec![];
    for pat in o.prim_path.split_whitespace() {
        let (exclude, pat) = if pat.starts_with(['^', '-']) {
            (true, pat.trim_start_matches(['^', '-']))
        } else {
            (false, pat)
        };
        if !pat.starts_with('/') {
            return Err(err(format!("prim pattern '{pat}' must start with /")));
        }
        if exclude {
            exc.push(pat)
        } else {
            inc.push(pat)
        }
    }
    let stage = usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(path.to_string_lossy().as_ref())
        .map_err(|e| {
            err(format!(
                "cannot open {} as USD: {e}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ))
        })?;
    let root = stage.root_layer();
    let layer = root.data();
    let get = |key: &str| -> Result<Value> {
        layer
            .try_field(&sdf::Path::abs_root(), key)
            .map_err(err)?
            .map(|v| json_value(v.into_owned()))
            .transpose()
            .map(|v| v.unwrap_or(Value::Null))
    };
    let y_up = get("upAxis")?.as_str() == Some("Y");
    let unit = if o.apply_units {
        get("metersPerUnit")?
            .as_f64()
            .filter(|x| *x != 0.)
            .unwrap_or(1.)
    } else {
        1.
    };
    drop(root);
    let t = o
        .time
        .map(usd::TimeCode::new)
        .unwrap_or(usd::TimeCode::EARLIEST);
    let mut xcache = XformCache::new(t);
    let axis = if y_up && o.up_axis == "z" {
        Matrix4d::rotation_x(std::f64::consts::FRAC_PI_2)
    } else {
        Matrix4d::IDENTITY
    };
    let mut paths = vec![];
    stage
        .traverse(
            usd::PrimPredicate::DEFAULT.with_instance_proxies(true),
            |p| paths.push(p.clone()),
        )
        .map_err(err)?;
    if inc.len() == 1
        && exc.is_empty()
        && !inc[0].contains(['*', '?'])
        && inc[0] != "/"
        && !paths.iter().any(|p| p.to_string() == inc[0])
    {
        return Err(err(format!(
            "no prim {:?} in {}",
            inc[0],
            path.file_name().unwrap_or_default().to_string_lossy()
        )));
    }
    let mut groups: BTreeMap<String, Vec<Part>> = BTreeMap::new();
    let mut group_details = BTreeMap::new();
    let mut notes = vec![];
    let mut skipped = vec![];
    let mut prune = vec![];
    let mut count = 0;
    let mut source_paths = vec![];
    let mut instance_names = vec![];
    let mut instance_bounds = vec![];
    for path_ in paths {
        let path_s = path_.to_string();
        if prune
            .iter()
            .any(|s: &String| path_s.starts_with(&format!("{s}/")))
        {
            continue;
        }
        let p = stage.prim(&path_).map_err(err)?;
        let ty = p
            .type_name()
            .map_err(err)?
            .map(|s| s.to_string())
            .unwrap_or_default();
        if ty == "PointInstancer" {
            prune.push(path_s.clone());
        }
        if ty == "Xform" && metadata(&p)?["minihou"]["counts"].is_object() {
            group_details.insert(path_s.clone(), detail(&p, t)?);
        }
        if !["Mesh", "BasisCurves", "Points", "PointInstancer"].contains(&ty.as_str()) {
            if [
                "Cube",
                "Sphere",
                "Capsule",
                "Cylinder",
                "Cone",
                "NurbsCurves",
                "NurbsPatch",
            ]
            .contains(&ty.as_str())
            {
                skipped.push(format!("{path_s} ({ty})"));
            }
            continue;
        }
        if (!inc.is_empty() && !inc.iter().any(|pat| hit(&path_s, pat)))
            || exc.iter().any(|pat| hit(&path_s, pat))
        {
            continue;
        }
        let mut purpose = String::new();
        let mut a = Some(p.clone());
        let mut hidden = false;
        while let Some(p) = a {
            if purpose.is_empty() {
                purpose = string(&p, "purpose", t)?;
                if purpose == "default" {
                    purpose.clear();
                }
            }
            let vis = p.attribute("visibility");
            if vis
                .time_samples()
                .map_err(err)?
                .is_some_and(|s| !s.is_empty())
                && string(&p, "visibility", t)? == "invisible"
            {
                hidden = true;
            }
            a = p.parent();
        }
        if hidden {
            continue;
        }
        if purpose.is_empty() {
            purpose = "default".into();
        }
        if !purposes.contains(&"all") && !purposes.contains(&purpose.as_str()) {
            skipped.push(format!("{path_s} ({purpose} purpose)"));
            continue;
        }
        let pts = numbers(
            &p,
            if ty == "PointInstancer" {
                "positions"
            } else {
                "points"
            },
            t,
        )?;
        if pts.is_empty() {
            notes.push(format!("{path_s}: no points at this time; skipped"));
            continue;
        }
        if !pts.len().is_multiple_of(3) {
            return Err(err("USD points must have three components"));
        }
        let (counts, idx) = if ty == "Mesh" {
            (
                indices(&p, "faceVertexCounts", t)?,
                indices(&p, "faceVertexIndices", t)?,
            )
        } else if ty == "BasisCurves" {
            let counts = indices(&p, "curveVertexCounts", t)?;
            let n: usize = counts.iter().map(|&x| x.max(0) as usize).sum();
            if n > pts.len() / 3 {
                return Err(err("curve counts exceed its points"));
            }
            if string(&p, "type", t)? != "linear" {
                notes.push(format!("{path_s}: cubic curve read as its control polygon"));
            }
            (counts, (0..n as i32).collect())
        } else {
            (vec![], vec![])
        };
        let mut offs = vec![0i32];
        for n in counts {
            if n < 0 {
                return Err(err("negative USD face/curve count"));
            }
            offs.push(
                offs.last()
                    .unwrap()
                    .checked_add(n)
                    .ok_or_else(|| err("USD topology exceeds index limit"))?,
            );
        }
        let nc = offs.len() - 1;
        let mut g = Geometry::from_arrays(
            pts.into_iter().map(|x| x as f32).collect(),
            idx,
            offs,
            Some(vec![ty == "Mesh"; nc]),
        )?;
        let (pi, qi) = primvars(&p, &mut g, t, &mut notes)?;
        let group_prim = if pi.is_some() { group(&p)? } else { None };
        if group_prim.is_some() && unit != 1. {
            // The writer scales this MiniHou attribute with positions. Undo that
            // storage-unit conversion before exact primvars win in the instancer.
            if let Some(scale) = g
                .point
                .get("scale")
                .and_then(|a| a.as_f32().map(|v| (a.width, v.to_vec())))
            {
                g.point.insert(
                    "scale".into(),
                    Arc::new(Attr::f32(
                        scale.0,
                        scale
                            .1
                            .into_iter()
                            .map(|v| (v as f64 * unit) as f32)
                            .collect(),
                    )),
                );
            }
        }
        let key = group_prim
            .as_ref()
            .map(|p| p.path().to_string())
            .unwrap_or_default();
        let d = if let Some(ref p) = group_prim {
            detail(p, t)?
        } else {
            Default::default()
        };
        let stored_materials = d.get("materials").is_some_and(Value::is_array)
            && g.prim.contains_key("material_index");
        if ty == "Mesh" {
            groups_materials(&stage, &p, &mut g, t, path, stored_materials)?;
        }
        let m = xcache.local_to_world_transform(&p).map_err(err)? * axis;
        if ty == "PointInstancer" {
            instancer(
                &stage,
                &p,
                &mut g,
                t,
                m,
                unit,
                &mut instance_names,
                &mut instance_bounds,
                &mut notes,
            )?;
        }
        transform(
            &mut g,
            m,
            unit,
            ty == "Mesh",
            string(&p, "orientation", t)? == "leftHanded",
        )?;
        if !o.path_attrib.is_empty() && g.nprims() > 0 {
            g.prim
                .entry(o.path_attrib.clone())
                .or_insert_with(|| Arc::new(Attr::i32(1, vec![source_paths.len() as i32; nc])));
            source_paths.push(path_s);
        }
        group_details.insert(key.clone(), d);
        groups.entry(key).or_default().push(Part {
            geo: g,
            point_index: pi,
            prim_index: qi,
        });
        count += 1;
    }
    let mut geos = vec![];
    for (key, parts) in groups {
        let mut geo = assemble(parts)?;
        geo.detail
            .extend(group_details.remove(&key).unwrap_or_default());
        geos.push(Arc::new(geo));
    }
    let mut geo = if geos.len() == 1 {
        (*geos[0]).clone()
    } else if geos.is_empty() {
        let mut g = Geometry::points_only(vec![])?;
        for d in group_details.into_values() {
            g.detail.extend(d);
        }
        g
    } else {
        assemble(
            geos.into_iter()
                .map(|g| Part {
                    geo: (*g).clone(),
                    point_index: None,
                    prim_index: None,
                })
                .collect(),
        )?
    };
    if !instance_names.is_empty() {
        geo.detail
            .entry("instance_objects")
            .or_insert(json!(instance_names));
    }
    if instance_bounds.iter().any(|b| !b.is_null()) {
        geo.detail
            .entry("instance_bounds")
            .or_insert(json!(instance_bounds));
    }
    if !source_paths.is_empty() {
        geo.detail.insert("usd_paths".into(), json!(source_paths));
    }
    geo.detail.insert("usd_prims".into(), json!(count));
    if !skipped.is_empty() {
        geo.detail.insert(
            "usd_skipped".into(),
            json!(skipped.into_iter().take(20).collect::<Vec<_>>().join("; ")),
        );
    }
    if !notes.is_empty() {
        notes.sort();
        notes.dedup();
        geo.detail.insert(
            "usd_notes".into(),
            json!(notes.join("; ").chars().take(500).collect::<String>()),
        );
    }
    geo.validate()?;
    Ok(geo)
}
