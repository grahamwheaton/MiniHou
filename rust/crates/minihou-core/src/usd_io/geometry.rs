use super::*;

pub(super) fn geometry(
    out: &mut String,
    g: &Geometry,
    name: &str,
    k: f64,
    looks: &BTreeMap<String, String>,
) -> Result<Value> {
    g.validate()?;
    let mut mesh = vec![];
    let mut curves = vec![];
    let mut used = vec![false; g.npoints];
    let mut dropped = [0usize; 2];
    for i in 0..g.nprims() {
        let n = (g.topology.prim_offsets[i + 1] - g.topology.prim_offsets[i]) as usize;
        let closed = g.topology.is_closed(i);
        if closed && n >= 3 {
            mesh.push(i)
        } else if !closed && n >= 2 {
            curves.push(i)
        } else {
            dropped[usize::from(!closed)] += 1;
            continue;
        }
        for &v in &g.topology.vertex_point
            [g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i + 1] as usize]
        {
            used[v as usize] = true;
        }
    }
    let _ = writeln!(
        out,
        "def Xform {} (customData = {{ dictionary minihou = {{ string detail = {} dictionary counts = {{ int points = {} int prims = {} }} dictionary dropped = {{ int faces = {} int curves = {} }} }} }}) {{",
        q(name),
        q(&Value::Object(g.detail.clone()).to_string()),
        g.npoints,
        g.nprims(),
        dropped[0],
        dropped[1]
    );
    property(out, "token", "visibility", "\"inherited\"", "");
    // Also an attribute, so animation can sample details without changing metadata.
    property(
        out,
        "custom string",
        "minihou:detail",
        &q(&Value::Object(g.detail.clone()).to_string()),
        "",
    );
    for (prims, kind) in [(&mesh, "mesh"), (&curves, "curves")] {
        if prims.is_empty() {
            continue;
        }
        let corners: Vec<usize> = prims
            .iter()
            .flat_map(|&i| {
                g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i + 1] as usize
            })
            .collect();
        let pts: Vec<usize> = if kind == "mesh" {
            corners
                .iter()
                .map(|&c| g.topology.vertex_point[c] as usize)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        } else {
            corners
                .iter()
                .map(|&c| g.topology.vertex_point[c] as usize)
                .collect()
        };
        let _ = writeln!(
            out,
            "def {} {} (prepend apiSchemas = [\"MaterialBindingAPI\"]) {{",
            if kind == "mesh" {
                "Mesh"
            } else {
                "BasisCurves"
            },
            q(kind)
        );
        property(out, "token", "visibility", "\"inherited\"", "");
        positions(out, g, &pts, k, "points");
        let counts = ints(
            prims
                .iter()
                .map(|&i| (g.topology.prim_offsets[i + 1] - g.topology.prim_offsets[i]) as usize),
        );
        property(
            out,
            "int[]",
            if kind == "mesh" {
                "faceVertexCounts"
            } else {
                "curveVertexCounts"
            },
            &counts,
            "",
        );
        if kind == "mesh" {
            let local: BTreeMap<_, _> = pts.iter().enumerate().map(|(i, &p)| (p, i)).collect();
            property(
                out,
                "int[]",
                "faceVertexIndices",
                &ints(
                    corners
                        .iter()
                        .map(|&c| local[&(g.topology.vertex_point[c] as usize)]),
                ),
                "",
            );
            property(out, "uniform token", "subdivisionScheme", "\"none\"", "");
            property(out, "uniform token", "orientation", "\"rightHanded\"", "");
        } else {
            property(out, "uniform token", "type", "\"linear\"", "");
            property(out, "uniform token", "wrap", "\"nonperiodic\"", "");
            property(
                out,
                "float[]",
                "widths",
                "[0.01]",
                "(interpolation = \"constant\")",
            );
        }
        internal(out, "point_index", &pts, "vertex");
        internal(out, "prim_index", prims, "uniform");
        attributes(out, g, &pts, &corners, prims, kind == "mesh", k);
        if kind == "mesh" {
            let mut used = BTreeSet::new();
            for (table, rows, el, family) in [
                (&g.prim, prims.as_slice(), "face", "minihou_face_groups"),
                (&g.point, pts.as_slice(), "point", "minihou_point_groups"),
            ] {
                for (n, a) in table
                    .iter()
                    .filter(|(n, a)| n.starts_with("group_") && a.width == 1)
                {
                    let selected: Vec<_> = rows
                        .iter()
                        .enumerate()
                        .filter_map(|(i, &r)| (scalar(Some(a), r, 0.) != 0.).then_some(i))
                        .collect();
                    if !selected.is_empty() {
                        subset(
                            out,
                            &unique(&format!("g_{}", &n[6..]), &mut used, el),
                            el,
                            family,
                            &selected,
                            Some(n),
                            None,
                        );
                    }
                }
                property(
                    out,
                    "uniform token",
                    &format!("subsetFamily:{family}:familyType"),
                    "\"unrestricted\"",
                    "",
                );
            }
            if let Some(mats) = g.detail.get("materials").and_then(Value::as_array) {
                for (slot, n) in mats.iter().enumerate() {
                    if let Some(path) = n.as_str().and_then(|n| looks.get(n)) {
                        let faces: Vec<_> = prims
                            .iter()
                            .enumerate()
                            .filter_map(|(i, &p)| {
                                (scalar(g.prim.get("material_index"), p, 0.) as usize == slot)
                                    .then_some(i)
                            })
                            .collect();
                        if faces.len() == prims.len() {
                            let _ = writeln!(out, "rel material:binding = <{path}>");
                        } else if !faces.is_empty() {
                            subset(
                                out,
                                &format!("mat_{slot}"),
                                "face",
                                "materialBind",
                                &faces,
                                None,
                                Some(path),
                            );
                        }
                    }
                }
                property(
                    out,
                    "uniform token",
                    "subsetFamily:materialBind:familyType",
                    "\"nonOverlapping\"",
                    "",
                );
            }
        }
        out.push_str("}\n");
    }
    let names: Vec<_> = g
        .detail
        .get("instance_objects")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|n| n.as_str().unwrap_or("").to_owned())
                .collect()
        })
        .unwrap_or_default();
    let (placed, loose): (Vec<_>, Vec<_>) = (0..g.npoints).filter(|&i| !used[i]).partition(|&i| {
        let n = scalar(g.point.get("instance"), i, 0.);
        n >= 1. && n <= names.len() as f64
    });
    if !placed.is_empty() {
        out.push_str("def PointInstancer \"instances\" {\n");
        property(out, "token", "visibility", "\"inherited\"", "");
        positions(out, g, &placed, k, "positions");
        property(
            out,
            "int[]",
            "protoIndices",
            &ints(
                placed
                    .iter()
                    .map(|&i| scalar(g.point.get("instance"), i, 1.) as usize - 1),
            ),
            "",
        );
        let mut proto_names = BTreeSet::new();
        let proto_names: Vec<_> = names
            .iter()
            .map(|n| unique(n, &mut proto_names, "proto"))
            .collect();
        // Relative relationship targets are resolved against this prim by USD.
        let _ = writeln!(
            out,
            "rel prototypes = {}",
            array(proto_names.iter().map(|n| format!("<Prototypes/{n}>")))
        );
        let scales = g
            .point
            .get("scale")
            .filter(|a| a.width == 3)
            .cloned()
            .unwrap_or_else(|| Arc::new(Attr::f32(3, vec![1.; g.npoints * 3])));
        property(
            out,
            "float3[]",
            "scales",
            &attr_values(&scales, &placed, k),
            "",
        );
        let rotations = array(placed.iter().map(|&i| {
            let r = g
                .point
                .get("rot")
                .filter(|a| a.width == 3)
                .and_then(|a| a.as_f32())
                .map(|a| [a[i * 3] as f64, a[i * 3 + 1] as f64, a[i * 3 + 2] as f64])
                .unwrap_or([0.; 3]);
            let (sx, cx) = (r[0] * 0.5).sin_cos();
            let (sy, cy) = (r[1] * 0.5).sin_cos();
            let (sz, cz) = (r[2] * 0.5).sin_cos();
            format!(
                "({}, {}, {}, {})",
                num(cx * cy * cz + sx * sy * sz),
                num(sx * cy * cz - cx * sy * sz),
                num(cx * sy * cz + sx * cy * sz),
                num(cx * cy * sz - sx * sy * cz)
            )
        }));
        property(out, "quatf[]", "orientationsf", &rotations, "");
        if let Some(a) = g.point.get("id") {
            property(
                out,
                "int64[]",
                "ids",
                &array(placed.iter().map(|&i| scalar(Some(a), i, 0.).to_string())),
                "",
            );
        }
        for (src, dst, factor) in [
            ("v", "velocities", 1.),
            ("w", "angularVelocities", 180. / std::f64::consts::PI),
        ] {
            if let Some(a) = g.point.get(src).filter(|a| a.width == 3) {
                property(out, "vector3f[]", dst, &attr_values(a, &placed, factor), "");
            }
        }
        if let Some(invis) = g
            .point
            .get("group_invisible")
            .or_else(|| g.point.get("invisible"))
        {
            property(
                out,
                "int64[]",
                "invisibleIds",
                &array(
                    placed
                        .iter()
                        .enumerate()
                        .filter(|(_, i)| scalar(Some(invis), **i, 0.) != 0.)
                        .map(|(r, &i)| scalar(g.point.get("id"), i, r as f64).to_string()),
                ),
                "",
            );
        }
        internal(out, "point_index", &placed, "vertex");
        attributes(out, g, &placed, &[], &[], false, k);
        out.push_str("def Scope \"Prototypes\" {\n");
        for (n, p) in names.iter().zip(proto_names) {
            let _ = writeln!(
                out,
                "def Xform {} (customData = {{dictionary minihou = {{string blender_object = {}}}}}) {{}}",
                q(&p),
                q(n)
            );
        }
        out.push_str("}\n}\n");
    }
    if !loose.is_empty() {
        out.push_str("def Points \"points\" {\n");
        property(out, "token", "visibility", "\"inherited\"", "");
        positions(out, g, &loose, k, "points");
        property(
            out,
            "float[]",
            "widths",
            "[0.02]",
            "(interpolation = \"constant\")",
        );
        internal(out, "point_index", &loose, "vertex");
        attributes(out, g, &loose, &[], &[], false, k);
        out.push_str("}\n");
    }
    out.push_str("}\n");
    Ok(
        json!({"faces":mesh.len(),"curves":curves.len(),"points":loose.len(),"instances":placed.len(),"dropped":{"faces":dropped[0],"curves":dropped[1]}}),
    )
}
