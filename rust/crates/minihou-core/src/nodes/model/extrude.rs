use super::*;

pub(crate) fn polyextrude(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = POLYEXTRUDE.one(inputs, 0)?;
    let direction = p.v3("direction")?;
    if !p.s("edges")?.is_empty() || !p.s("points")?.is_empty() {
        return component_extrude(&g, &p.s("edges")?, &p.s("points")?, p.f("distance")?, direction);
    }
    let divide_into = p.s("divide_into").unwrap_or_else(|_| "connected".to_string());
    let mode = if divide_into == "individual" {
        "individual".to_string()
    } else {
        p.e("mode", &["connected", "individual"])?
    };
    extrude_with_direction(
        &g,
        &p.s("group")?,
        p.f("distance")?,
        p.f("inset")?,
        &mode,
        p.b("output_back")?,
        Some(p),
        direction,
        inputs.get(1).and_then(|port|port.first()).map(|g| g.as_ref()),
    )
}

fn extrude_scalar(g: &Geometry, name: &str, prim: usize, point: usize) -> Option<f64> {
    if name.is_empty() { return None; }
    for (table, index) in [(&g.prim, prim), (&g.point, point)] {
        let Some(attr)=table.get(name) else {continue};
        if attr.width != 1 { continue; }
        if let Some(data)=attr.as_f32() { return data.get(index).map(|v|*v as f64); }
        if let AttrData::I32(data)=&attr.data { return data.get(index).map(|v|*v as f64); }
    }
    None
}

fn extrude_vector(g: &Geometry, name: &str, prim: usize, point: usize) -> Option<[f64;3]> {
    if name.is_empty() { return None; }
    for (table,index) in [(&g.prim,prim),(&g.point,point)] {
        let Some(attr)=table.get(name) else {continue};
        if attr.width != 3 {continue;}
        if let Some(data)=attr.as_f32() {return Some([data[3*index] as f64,data[3*index+1] as f64,data[3*index+2] as f64]);}
    }
    None
}

fn extrude_ramp(value: &serde_json::Value, t: f64) -> Result<f64> {
    let Some(points)=value.as_array() else {return Err(Error::new("extrusion ramp must be an array of [position, value] pairs"))};
    if points.len()<2 {return Err(Error::new("extrusion ramp needs at least two points"));}
    let mut previous=(-1.,0.);
    for point in points {
        let (Some(x),Some(y))=(point[0].as_f64(),point[1].as_f64()) else {return Err(Error::new("invalid extrusion ramp point"))};
        if !(0.0..=1.0).contains(&x) || !y.is_finite() || x<=previous.0 {return Err(Error::new("extrusion ramp positions must increase from 0 to 1"));}
        if t<=x {return Ok(if previous.0<0. {y} else {previous.1+(y-previous.1)*(t-previous.0)/(x-previous.0)});}
        previous=(x,y);
    }
    Ok(previous.1)
}

fn rotate_around(point:[f64;3],center:[f64;3],axis:[f64;3],angle:f64)->[f64;3] {
    let w=sub(point,center);let axis=unit(axis);let (s,c)=angle.sin_cos();
    add(center,add(mul(w,c),add(mul(cross(axis,w),s),mul(axis,dot(axis,w)*(1.-c)))))
}

fn spine_position(spine:&Geometry,t:f64,reverse:bool)->Result<[f64;3]> {
    if spine.npoints<2 {return Err(Error::new("external spine needs at least two points"));}
    let u=(if reverse {1.-t} else {t}).clamp(0.,1.)*(spine.npoints-1) as f64;
    let i=(u.floor() as usize).min(spine.npoints-2);let f=u-i as f64;
    let p=spine.p();
    Ok(std::array::from_fn(|k|p[3*i+k] as f64*(1.-f)+p[3*(i+1)+k] as f64*f))
}
fn spine_scalar(spine:Option<&Geometry>,name:&str,t:f64,reverse:bool)->Option<f64> {
    let spine=spine?;if name.is_empty() || spine.npoints<2 {return None;}
    let attr=spine.point.get(name)?;if attr.width!=1 {return None;}
    let u=(if reverse {1.-t} else {t}).clamp(0.,1.)*(spine.npoints-1) as f64;
    let i=(u.floor() as usize).min(spine.npoints-2);let f=u-i as f64;
    let get=|i:usize|match &attr.data {AttrData::F32(xs)=>xs.get(i).map(|v|*v as f64),AttrData::I32(xs)=>xs.get(i).map(|v|*v as f64),_=>None};
    Some(get(i)?*(1.-f)+get(i+1)?*f)
}
fn curvature_fraction(samples:&[[f64;3]],fraction:f64)->f64 {
    if samples.len()<3 {return fraction;}
    let mut cumulative=vec![0.;samples.len()];
    for i in 1..samples.len() {
        let curvature=if i+1<samples.len() {
            let before=unit(sub(samples[i],samples[i-1]));let after=unit(sub(samples[i+1],samples[i]));
            len(sub(after,before))
        } else {0.};
        cumulative[i]=cumulative[i-1]+1.+8.*curvature;
    }
    let wanted=fraction*cumulative.last().copied().unwrap_or(0.);
    let i=cumulative.partition_point(|&v|v<wanted).clamp(1,cumulative.len()-1);
    let segment=(wanted-cumulative[i-1])/(cumulative[i]-cumulative[i-1]).max(1e-12);
    (i as f64-1.+segment)/(samples.len()-1) as f64
}
fn component_extrude(g: &Geometry, edges: &str, points: &str, distance: f64, direction: [f64; 3]) -> Result<Geometry> {
    if distance == 0. { return Ok(g.clone()); }
    let edge_pairs: Vec<(usize,usize)> = edges.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty()).map(|s| {
            let (a,b)=s.split_once('-').ok_or_else(||Error::new("invalid extrusion edge"))?;
            let a=a.parse::<usize>().map_err(|_|Error::new("invalid extrusion edge"))?;
            let b=b.parse::<usize>().map_err(|_|Error::new("invalid extrusion edge"))?;
            Ok((a,b))
        }).collect::<Result<_>>()?;
    let point_ids: Vec<usize> = points.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty()).map(|s|s.parse::<usize>().map_err(|_|Error::new("invalid extrusion point")))
        .collect::<Result<_>>()?;
    if edge_pairs.is_empty() && point_ids.is_empty() { return Ok(g.clone()); }
    let mut owner=vec![None;g.npoints];
    let mut normals=vec![[0.;3];g.npoints];
    let mut curve_directions=vec![[0.;3];g.npoints];
    let pos=positions(g);
    let face_normals=normals::prim_normals(g.p(),&g.topology);
    let mut valid_edges=std::collections::BTreeMap::new();
    for prim in 0..g.nprims() {
        let verts: Vec<_>=range(g,prim).collect();
        for (i,&v) in verts.iter().enumerate() {
            let a=g.topology.vertex_point[v] as usize;
            owner[a]=Some((v,prim));
            normals[a]=add(normals[a],face_normals[prim]);
            let closed=g.topology.closed.as_ref().is_none_or(|closed|closed[prim]);
            if i+1<verts.len() || closed {
                let b=g.topology.vertex_point[verts[(i+1)%verts.len()]] as usize;
                valid_edges.entry((a.min(b),a.max(b))).or_insert((v,prim));
                if !closed {
                    curve_directions[a]=add(curve_directions[a],sub(pos[a],pos[b]));
                    curve_directions[b]=add(curve_directions[b],sub(pos[b],pos[a]));
                }
            }
        }
    }
    let mut selected=std::collections::BTreeSet::new();
    for &(a,b) in &edge_pairs {
        if !valid_edges.contains_key(&(a.min(b),a.max(b))) { return Err(Error::new("extrusion edge is not in the source mesh")); }
        selected.extend([a,b]);
    }
    for &p in &point_ids {
        if owner.get(p).copied().flatten().is_none() { return Err(Error::new("extrusion point is not in the source mesh")); }
        selected.insert(p);
    }
    let selected: Vec<_>=selected.into_iter().collect();
    let mut map=vec![0usize;g.npoints];
    for (i,&p) in selected.iter().enumerate() { map[p]=g.npoints+i; }
    let mut outp=g.p().to_vec();
    for &p in &selected {
        let normal=if len(direction)>0. { direction }
            else if len(normals[p])>1e-12 {unit32(normals[p])}
            else {unit32(curve_directions[p])};
        outp.extend(add(pos[p],mul(normal,distance)).map(|x|x as f32));
    }
    let mut point=concat(&g.point,&gather(&g.point,&selected));
    point.insert("P".into(),Arc::new(Attr::f32(3,outp)));
    let mut vp=g.topology.vertex_point.clone();
    let mut sizes: Vec<_>=(0..g.nprims()).map(|i|range(g,i).len()).collect();
    let mut vs: Vec<_>=(0..g.nvertices()).collect();
    let mut ps: Vec<_>=(0..g.nprims()).collect();
    let mut closed=g.topology.closed.clone().unwrap_or_else(||vec![true;g.nprims()]);
    for &(a,b) in &edge_pairs {
        let (v,prim)=valid_edges[&(a.min(b),a.max(b))];
        vp.extend([a as i32,b as i32,map[b] as i32,map[a] as i32]);
        vs.extend([v,v,v,v]); ps.push(prim); sizes.push(4); closed.push(true);
    }
    for &p in &point_ids {
        let (v,prim)=owner[p].unwrap();
        vp.extend([p as i32,map[p] as i32]);
        vs.extend([v,v]); ps.push(prim); sizes.push(2); closed.push(false);
    }
    let mut out=make(point,vp,&sizes,gather(&g.vertex,&vs),gather(&g.prim,&ps),g.detail.clone(),Some(closed))?;
    out.point.remove("N");
    Ok(out)
}
pub(crate) fn extrude(
    g: &Geometry,
    group: &str,
    distance: f64,
    inset: f64,
    mode: &str,
    back: bool,
    params: Option<&Params>,
) -> Result<Geometry> {
    extrude_with_direction(g, group, distance, inset, mode, back, params, [0.; 3], None)
}
fn compute_vertex_normals(geo: &Geometry, cusp_angle: f64) -> Vec<f32> {
    let p_slice = geo.p();
    let vp = &geo.topology.vertex_point;
    let offs = &geo.topology.prim_offsets;
    let nprims = geo.nprims();
    let nverts = geo.nvertices();
    let npoints = geo.npoints;
    let fnorm = normals::prim_normals(p_slice, &geo.topology);
    let funit_f32 = normals::normalize_to_f32(&fnorm);
    let funit: Vec<[f64; 3]> = (0..nprims)
        .map(|f| [
            funit_f32[f * 3] as f64,
            funit_f32[f * 3 + 1] as f64,
            funit_f32[f * 3 + 2] as f64,
        ])
        .collect();
    let cos_cusp = (cusp_angle.to_radians()).cos() - 1e-9;
    let (start, prims) = normals::point_prims(vp, offs, npoints);
    let mut n_data = vec![0.0f32; nverts * 3];
    for f in 0..nprims {
        let u_f = funit[f];
        let s = offs[f] as usize;
        let e = offs[f + 1] as usize;
        for v in s..e {
            let q = vp[v] as usize;
            let mut acc = [0.0f64; 3];
            for &nbr_u32 in &prims[start[q] as usize..start[q + 1] as usize] {
                let nbr = nbr_u32 as usize;
                let u_nbr = funit[nbr];
                let dot = u_f[0] * u_nbr[0] + u_f[1] * u_nbr[1] + u_f[2] * u_nbr[2];
                if dot >= cos_cusp {
                    acc[0] += fnorm[nbr][0];
                    acc[1] += fnorm[nbr][1];
                    acc[2] += fnorm[nbr][2];
                }
            }
            let len = (acc[0] * acc[0] + acc[1] * acc[1] + acc[2] * acc[2]).sqrt();
            if len > 1e-12 {
                n_data[v * 3] = (acc[0] / len) as f32;
                n_data[v * 3 + 1] = (acc[1] / len) as f32;
                n_data[v * 3 + 2] = (acc[2] / len) as f32;
            } else {
                n_data[v * 3] = u_f[0] as f32;
                n_data[v * 3 + 1] = u_f[1] as f32;
                n_data[v * 3 + 2] = u_f[2] as f32;
            }
        }
    }
    n_data
}

fn extrude_with_direction(
    g: &Geometry, group: &str, distance: f64, inset: f64, mode: &str,
    back: bool, params: Option<&Params>, direction: [f64; 3], spine: Option<&Geometry>,
) -> Result<Geometry> {
    polygons(g, "polyextrude")?;
    let sel = prim_mask(g, group)?;
    let output_front = params.map_or(Ok(true), |p| p.b("output_front"))?;
    let output_side = params.map_or(Ok(true), |p| p.b("output_side"))?;
    let output_back = params.map_or(Ok(back), |p| p.b("output_back"))?;
    let mut divisions = params.map_or(Ok(1), |p| p.i("divisions"))?.max(1) as usize;
    let twist = params.map_or(Ok(0.0), |p| p.f("twist"))?;
    let offset_even = params.map_or(Ok(true), |p| p.b("offset_even"))?;
    let shape = params.map_or(Ok("straight".to_owned()), |p| p.e("spine_shape", &["straight","spline","curved","curve_from_second_input"]))?;
    let interpolation = params.map_or(Ok("rotating_frame".to_owned()), |p| p.e("interpolation", &["rotating_frame","linear","smooth"]))?;
    let spacing = params.map_or(Ok("uniform".to_owned()), |p| p.e("division_spacing", &["uniform","curvature_sensitive"]))?;
    let spine = if shape=="curve_from_second_input" {Some(spine.ok_or_else(||Error::new("Curve from second input requires a spine curve"))?)} else {None};
    let reverse_spine=params.map_or(Ok(false),|p|p.b("reverse_spine_direction"))?;
    let axial_rotation=params.map_or(Ok(0.),|p|p.f("axial_rotation"))?.to_radians();
    let front_blend=params.map_or(Ok(0.),|p|p.f("front_blend"))?.clamp(0.,1.);
    let back_blend=params.map_or(Ok(0.),|p|p.f("back_blend"))?.clamp(0.,1.);
    let front_magnitude=params.map_or(Ok(1.),|p|p.f("front_magnitude"))?;
    let back_magnitude=params.map_or(Ok(1.),|p|p.f("back_magnitude"))?;
    let front_stiffness=params.map_or(Ok(1.),|p|p.f("front_stiffness"))?;
    let back_stiffness=params.map_or(Ok(1.),|p|p.f("back_stiffness"))?;
    let thickness_scale=params.map_or(Ok(1.),|p|p.f("thickness_scale"))?;
    let twist_half_width=params.map_or(Ok(1.),|p|p.f("twist_range_half_width"))?;
    let thickness_attr=params.map_or(Ok(String::new()),|p|p.s("thickness_attribute"))?;
    let spine_twist_attr=params.map_or(Ok(String::new()),|p|p.s("twist_attribute"))?;
    let add_vertex_normals = params.map_or(Ok(false), |p| p.b("add_vertex_normals"))?;
    let cusp_angle = params.map_or(Ok(60.0), |p| p.f("cusp_angle"))?;
    let generate_sides_uvs = params.map_or(Ok(false), |p| p.b("generate_sides_uvs"))?;

    let apply_groups =
        |out: &mut Geometry, front: &[bool], back: &[bool], side: &[bool]| -> Result<()> {
            if let Some(p) = params {
                for (key,mask) in [("front",front),("back",back),("side",side)] {
                    let mut values=mask.to_vec();
                    if p.b("preserve_groups")? && p.b(&format!("outgrp_{key}"))? {
                        let group=p.s(&format!("outgrp_{key}_name"))?;
                        if let Some(existing)=out.prim.get(&format!("group_{group}")).and_then(|a|match &a.data {AttrData::I32(v)=>Some(v.as_slice()),_=>None}) {
                            for (value,&old) in values.iter_mut().zip(existing) {*value|=old!=0;}
                        }
                    }
                    outgroup(out,p,key,"prim",&values)?;
                }
            } else {
                write_group(out, "prim", "extrudeFront", front)?;
                write_group(out, "prim", "extrudeSide", side)?;
            }
            Ok(())
        };

    if !sel.iter().any(|&b| b) || (distance == 0. && inset == 0. && shape=="straight" && !output_back && twist == 0. && output_front && divisions == 1) {
        let mut out = g.clone();
        apply_groups(
            &mut out,
            &sel,
            &vec![false; g.nprims()],
            &vec![false; g.nprims()],
        )?;
        return Ok(out);
    }
    let (pv, nx, _) = corners(g);
    let vp = &g.topology.vertex_point;
    let pos = positions(g);
    let fn_ = normals::prim_normals(g.p(), &g.topology);
    let fu: Vec<_> = fn_.iter().map(|&v| unit32(v)).collect();
    let selected: Vec<_> = (0..g.nvertices()).filter(|&v| sel[pv[v]]).collect();
    if let Some(p)=params {
        let name=p.s("divisions_scale_attribute")?;
        let scale=selected.iter().filter_map(|&v|extrude_scalar(g,&name,pv[v],vp[v] as usize)).fold(1.,f64::max);
        divisions=((divisions as f64*scale).round() as usize).clamp(1,256);
        extrude_ramp(p.value("thickness_ramp"),0.)?;
        extrude_ramp(p.value("local_twist_ramp"),0.)?;
    }
    let individual = mode == "individual";
    let distance_attr=params.map_or(Ok(String::new()),|p|p.s("distance_scale_attribute"))?;
    let inset_attr=params.map_or(Ok(String::new()),|p|p.s("inset_scale_attribute"))?;
    let twist_attr=params.map_or(Ok(String::new()),|p|p.s("twist_scale_attribute"))?;
    let local_z_attr=params.map_or(Ok(String::new()),|p|p.s("local_z_attribute"))?;
    let local_x_attr=params.map_or(Ok(String::new()),|p|p.s("local_x_attribute"))?;
    let local_center_attr=params.map_or(Ok(String::new()),|p|p.s("local_center_attribute"))?;
    let extrusion_mode=params.map_or(Ok("primitive_normal".to_owned()),|p|p.e("extrusion_mode", &["primitive_normal","point_normal","edge_normal","direction"]))?;
    let point_normals=if extrusion_mode=="point_normal" {Some(normals::point_normals(g.p(),&g.topology,g.npoints))} else {None};
    let spine_translation=if let Some(spine)=spine {sub(spine_position(spine,1.,reverse_spine)?,spine_position(spine,0.,reverse_spine)?)} else {[0.;3]};
    let shifted = if inset != 0. {
        Some(super::super::planar_offset::offset_selected(
            g, &sel, inset, individual, offset_even,
        )?)
    } else {
        None
    };
    let src: Vec<usize> = if individual {
        selected.iter().map(|&v| vp[v] as usize).collect()
    } else {
        selected
            .iter()
            .map(|&v| vp[v] as usize)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    let mut owner=vec![0usize;g.npoints];
    for &v in &selected {owner[vp[v] as usize]=v;}
    let source_corner:Vec<usize>=if individual {selected.clone()} else {src.iter().map(|&q|owner[q]).collect()};
    let mut nv = vec![0; g.nvertices()];
    let mut newp = Vec::new();
    if individual {
        for (i, &v) in selected.iter().enumerate() {
            nv[v] = g.npoints + i;
            let q=vp[v] as usize;
            let scale=extrude_scalar(g,&inset_attr,pv[v],q).unwrap_or(1.);
            let base=shifted.as_ref().map_or(pos[q],|s|add(pos[q],mul(sub(s[v],pos[q]),scale)));
            let normal=extrude_vector(g,&local_z_attr,pv[v],q).filter(|n|len(*n)>1e-12).map(unit)
                .or_else(||point_normals.as_ref().map(|pn|[pn[3*q] as f64,pn[3*q+1] as f64,pn[3*q+2] as f64]))
                .unwrap_or_else(||if len(direction)>0. {unit(direction)} else {fu[pv[v]]});
            let amount=distance*extrude_scalar(g,&distance_attr,pv[v],q).unwrap_or(1.);
            newp.push(add(add(base,mul(normal,amount)),spine_translation));
        }
    } else {
        let mut map = vec![0; g.npoints];
        let mut acc = vec![[0.; 3]; g.npoints];
        let mut repr = vec![0; g.npoints];
        for (i, &q) in src.iter().enumerate() {
            map[q] = g.npoints + i;
        }
        for &v in &selected {
            let q = vp[v] as usize;
            nv[v] = map[q];
            acc[q] = add(acc[q], fn_[pv[v]]);
            repr[q] = v;
        }
        let pn: Vec<_> = acc.iter().map(|&v| unit32(v)).collect();
        let mut mincos = vec![1.0f64; g.npoints];
        for &v in &selected {
            let q = vp[v] as usize;
            mincos[q] = mincos[q].min(dot(pn[q], fu[pv[v]]).max(0.25));
        }
        for &q in &src {
            let v=repr[q];
            let normal=extrude_vector(g,&local_z_attr,pv[v],q).filter(|n|len(*n)>1e-12).map(unit)
                .or_else(||point_normals.as_ref().map(|normals|[normals[3*q] as f64,normals[3*q+1] as f64,normals[3*q+2] as f64]))
                .unwrap_or_else(||if len(direction)>0. {unit(direction)} else {pn[q]});
            let amount=distance*extrude_scalar(g,&distance_attr,pv[v],q).unwrap_or(1.);
            let mut point = add(add(pos[q],mul(normal,if offset_even && len(direction)==0. && extrusion_mode=="primitive_normal" {amount/mincos[q]} else {amount})),spine_translation);
            if let Some(sh) = &shifted {
                point = add(point, mul(sub(sh[v], pos[q]),extrude_scalar(g,&inset_attr,pv[v],q).unwrap_or(1.)));
            }
            newp.push(point);
        }
    }

    if let Some(p)=params {
        if p.b("transform_front")? {
            let translation=p.v3("front_translate")?;
            if len(translation)>0. {
                for (i,point) in newp.iter_mut().enumerate() {
                    let q=src[i];let v=source_corner[i];
                    let z=extrude_vector(g,&local_z_attr,pv[v],q).filter(|n|len(*n)>1e-12).map(unit).unwrap_or(fu[pv[v]]);
                    let fallback=sub(pos[vp[nx[v]] as usize],pos[q]);
                    let x=extrude_vector(g,&local_x_attr,pv[v],q).filter(|n|len(*n)>1e-12).unwrap_or(fallback);
                    let x=unit(sub(x,mul(z,dot(x,z))));let y=unit(cross(z,x));
                    *point=add(*point,add(mul(x,translation[0]),add(mul(y,translation[1]),mul(z,translation[2]))));
                }
            }
        }
    }

    if twist.abs() > 1e-6 {
        if individual {
            for i in 0..g.nprims() {
                if !sel[i] { continue; }
                let verts: Vec<usize> = range(g, i).collect();
                let axis = if len(direction) > 0. { unit(direction) } else { fu[i] };
                let pts: Vec<[f64; 3]> = verts.iter().map(|&v| newp[nv[v] - g.npoints]).collect();
                let count = pts.len() as f64;
                let center = [
                    pts.iter().map(|p| p[0]).sum::<f64>() / count,
                    pts.iter().map(|p| p[1]).sum::<f64>() / count,
                    pts.iter().map(|p| p[2]).sum::<f64>() / count,
                ];
                for &v in &verts {
                    let idx = nv[v] - g.npoints;
                    let q=vp[v] as usize;
                    let center=extrude_vector(g,&local_center_attr,i,q).unwrap_or(center);
                    let axis=extrude_vector(g,&local_z_attr,i,q).filter(|n|len(*n)>1e-12).map(unit).unwrap_or(axis);
                    let angle=twist.to_radians()*extrude_scalar(g,&twist_attr,i,q).unwrap_or(1.);
                    newp[idx] = rotate_around(newp[idx], center, axis, angle);
                }
            }
        } else if !newp.is_empty() {
            let count = newp.len() as f64;
            let center = [
                newp.iter().map(|p| p[0]).sum::<f64>() / count,
                newp.iter().map(|p| p[1]).sum::<f64>() / count,
                newp.iter().map(|p| p[2]).sum::<f64>() / count,
            ];
            let avg_norm = if len(direction) > 0. {
                unit(direction)
            } else {
                let mut acc = [0.; 3];
                for &v in &selected {
                    acc = add(acc, fn_[pv[v]]);
                }
                unit(acc)
            };
            for (i,p) in newp.iter_mut().enumerate() {
                let q=src[i];let v=source_corner[i];
                let pivot=extrude_vector(g,&local_center_attr,pv[v],q).unwrap_or(center);
                let angle=twist.to_radians()*extrude_scalar(g,&twist_attr,pv[v],q).unwrap_or(1.);
                let axis=extrude_vector(g,&local_z_attr,pv[v],q).filter(|n|len(*n)>1e-12).map(unit).unwrap_or(avg_norm);
                *p = rotate_around(*p, pivot, axis, angle);
            }
        }
    }

    let back_center=if src.is_empty(){[0.;3]}else{mul(src.iter().fold([0.;3],|acc,&q|add(acc,pos[q])),1./src.len() as f64)};
    let front_center=if newp.is_empty(){[0.;3]}else{mul(newp.iter().fold([0.;3],|acc,&q|add(acc,q)),1./newp.len() as f64)};
    if let Some(p)=params {
        let thick=1.+2.*(extrude_ramp(p.value("thickness_ramp"),1.)? - 0.5 + spine_scalar(spine,&thickness_attr,1.,reverse_spine).unwrap_or(0.))*thickness_scale;
        let local_twist=2.*(extrude_ramp(p.value("local_twist_ramp"),1.)? - 0.5 + spine_scalar(spine,&spine_twist_attr,1.,reverse_spine).unwrap_or(0.))*twist_half_width.to_radians();
        for (i,q) in newp.iter_mut().enumerate() {
            let center=extrude_vector(g,&local_center_attr,pv[source_corner[i]],src[i]).unwrap_or(front_center);
            *q=add(center,mul(sub(*q,center),thick));
            if local_twist.abs()>1e-12 {*q=rotate_around(*q,center,if len(direction)>0. {direction} else {fu[pv[source_corner[i]]]},local_twist);}
        }
    }

    let boundary: Vec<_> = if individual {
        selected.clone()
    } else {
        let e = topo::edges(&g.topology, g.npoints);
        let mut cnt = vec![0; e.len()];
        for &v in &selected {
            cnt[e.of_vertex[v] as usize] += 1;
        }
        selected
            .iter()
            .copied()
            .filter(|&v| cnt[e.of_vertex[v] as usize] == 1)
            .collect()
    };
    let layer_t:Vec<f64>=if spacing=="curvature_sensitive" && divisions>1 && (spine.is_some() || shape=="curved" || shape=="spline") {
        let samples=(0..=64).map(|i| {
            let t=i as f64/64.;
            if let Some(spine)=spine {spine_position(spine,t,reverse_spine)} else {
                let base=pos[src[0]];let front=newp[0];let normal=if len(direction)>0. {unit(direction)} else {fu[pv[source_corner[0]]]};
                let h10=t*(1.-t)*(1.-t);let h11=t*t*(t-1.);
                Ok(add(add(base,mul(sub(front,base),t)),mul(normal,distance*(h10*(back_magnitude*back_stiffness-1.)+h11*(front_magnitude*front_stiffness-1.)))))
            }
        }).collect::<Result<Vec<_>>>()?;
        (0..=divisions).map(|d|curvature_fraction(&samples,d as f64/divisions as f64)).collect()
    } else {(0..=divisions).map(|d|d as f64/divisions as f64).collect()};

    let mut point = concat(&g.point, &gather(&g.point, &src));
    let mut p = g.p().to_vec();
    p.extend(newp.iter().flat_map(|q| q.map(|x| x as f32)));

    let mut pt_map = BTreeMap::new();
    if divisions > 1 {
        let mut extra_pts = Vec::new();
        let mut extra_src = Vec::new();
        for &v in &boundary {
            for &corner in &[v, nx[v]] {
                let (a, b) = (vp[corner] as usize, nv[corner]);
                if pt_map.contains_key(&(1, a, b)) {
                    continue;
                }
                let pos_a = pos[a];
                let pos_b = [
                    newp[b - g.npoints][0],
                    newp[b - g.npoints][1],
                    newp[b - g.npoints][2],
                ];
                for d in 1..divisions {
                    let t=layer_t[d];
                    let mut interp=add(pos_a,mul(sub(pos_b,pos_a),t));
                    if (shape=="spline" || shape=="curved") && interpolation!="linear" {
                        let n=if len(direction)>0. {unit(direction)} else {fu[pv[v]]};
                        let h10=t*(1.-t)*(1.-t);let h11=t*t*(t-1.);
                        interp=add(interp,mul(n,distance*(h10*(back_magnitude*back_stiffness-1.)+h11*(front_magnitude*front_stiffness-1.))));
                        if interpolation=="smooth" {interp=add(pos_a,mul(sub(interp,pos_a),t*t*(3.-2.*t)/t.max(1e-12)));}
                    }
                    if let Some(spine)=spine {
                        let start=spine_position(spine,0.,reverse_spine)?;
                        let path=spine_position(spine,t,reverse_spine)?;
                        let straight=add(start,mul(spine_translation,t));
                        let deviation=rotate_around(sub(path,straight),[0.;3],spine_translation,axial_rotation);
                        let blend=1.-back_blend*(1.-t)-front_blend*t;
                        interp=add(interp,mul(deviation,blend));
                    }
                    if let Some(p)=params {
                        let thick=1.+2.*(extrude_ramp(p.value("thickness_ramp"),t)? - 0.5 + spine_scalar(spine,&thickness_attr,t,reverse_spine).unwrap_or(0.))*thickness_scale;
                        let center=add(back_center,mul(sub(front_center,back_center),t));
                        interp=add(center,mul(sub(interp,center),thick));
                        let angle=2.*(extrude_ramp(p.value("local_twist_ramp"),t)? - 0.5 + spine_scalar(spine,&spine_twist_attr,t,reverse_spine).unwrap_or(0.))*twist_half_width.to_radians();
                        if angle.abs()>1e-12 {interp=rotate_around(interp,center,if len(direction)>0. {direction} else {fu[pv[v]]},angle);}
                    }
                    let pt_idx = g.npoints + newp.len() + extra_pts.len();
                    pt_map.insert((d, a, b), pt_idx);
                    extra_pts.push(interp);
                    extra_src.push(a);
                }
            }
        }
        let extra_gather = gather(&g.point, &extra_src);
        point = concat(&point, &extra_gather);
        for pt in extra_pts {
            p.extend([pt[0] as f32, pt[1] as f32, pt[2] as f32]);
        }
    }
    point.insert("P".into(), Arc::new(Attr::f32(3, p)));

    let step_pt = |d: usize, corner: usize| -> i32 {
        if d == 0 {
            vp[corner]
        } else if d == divisions {
            nv[corner] as i32
        } else {
            *pt_map.get(&(d, vp[corner] as usize, nv[corner])).unwrap() as i32
        }
    };

    let mut outvp: Vec<i32> = Vec::new();
    let mut vs: Vec<usize> = Vec::new();
    let mut ps: Vec<usize> = Vec::new();
    let mut sizes: Vec<usize> = Vec::new();

    let mut is_front = Vec::new();
    let mut is_side = Vec::new();
    let mut is_back = Vec::new();

    for i in 0..g.nprims() {
        let r = range(g, i);
        if sel[i] {
            if output_front {
                sizes.push(r.len());
                ps.push(i);
                is_front.push(true);
                is_side.push(false);
                is_back.push(false);
                for v in r {
                    outvp.push(nv[v] as i32);
                    vs.push(v);
                }
            }
        } else {
            sizes.push(r.len());
            ps.push(i);
            is_front.push(false);
            is_side.push(false);
            is_back.push(false);
            for v in r {
                outvp.push(vp[v]);
                vs.push(v);
            }
        }
    }

    if output_side {
        for &v in &boundary {
            for d in 0..divisions {
                outvp.extend([
                    step_pt(d, v),
                    step_pt(d, nx[v]),
                    step_pt(d + 1, nx[v]),
                    step_pt(d + 1, v),
                ]);
                vs.extend([v, nx[v], nx[v], v]);
                ps.push(pv[v]);
                sizes.push(4);
                is_front.push(false);
                is_side.push(true);
                is_back.push(false);
            }
        }
    }

    if output_back {
        for i in 0..g.nprims() {
            if !sel[i] {
                continue;
            }
            let r = range(g, i);
            sizes.push(r.len());
            ps.push(i);
            is_front.push(false);
            is_side.push(false);
            is_back.push(true);
            for v in r.rev() {
                outvp.push(vp[v]);
                vs.push(v);
            }
        }
    }

    let mut out = make(
        point,
        outvp,
        &sizes,
        gather(&g.vertex, &vs),
        gather(&g.prim, &ps),
        g.detail.clone(),
        None,
    )?;

    if let Some(p)=params {
        let copy_points=p.b("copy_point_groups")?;
        for (name,attr) in out.point.clone() {
            if !name.starts_with("group_") {continue;}
            if let AttrData::I32(data)=&attr.data {
                let mut values=data.clone();
                if !copy_points {values[g.npoints..].fill(0);}
                out.point.insert(name,Arc::new(Attr::i32(1,values)));
            }
        }
        let copy_prims=p.b("copy_primitive_groups")?;
        for (name,attr) in out.prim.clone() {
            if !name.starts_with("group_") {continue;}
            if let AttrData::I32(data)=&attr.data {
                let mut values=data.clone();
                for (i,value) in values.iter_mut().enumerate() {if (is_side[i] || is_front[i]) && !copy_prims {*value=0;}}
                out.prim.insert(name,Arc::new(Attr::i32(1,values)));
            }
        }
        let copy_edges=p.b("copy_edge_groups")?;
        for (name,attr) in out.vertex.clone() {
            if !name.starts_with("edge_group_") {continue;}
            if let AttrData::I32(data)=&attr.data {
                let mut values=data.clone();let mut offset=0;
                for (i,&size) in sizes.iter().enumerate() {if is_side[i] || is_front[i] && !copy_edges {values[offset..offset+size].fill(0);}offset+=size;}
                out.vertex.insert(name,Arc::new(Attr::i32(1,values)));
            }
        }
    }

    apply_groups(&mut out, &is_front, &is_back, &is_side)?;

    if let Some(p) = params {
        let front_boundary_set: BTreeSet<(i64, i64)> = boundary.iter().map(|&v| {
            let a = nv[v] as i64;
            let b = nv[nx[v]] as i64;
            (a.min(b), a.max(b))
        }).collect();
        let back_boundary_set: BTreeSet<(i64, i64)> = boundary.iter().map(|&v| {
            let a = vp[v] as i64;
            let b = vp[nx[v]] as i64;
            (a.min(b), a.max(b))
        }).collect();
        let ed = topo::edges(&out.topology, out.npoints);
        let front_edge_mask: Vec<bool> = ed.points.chunks_exact(2)
            .map(|q| front_boundary_set.contains(&(q[0].min(q[1]), q[0].max(q[1]))))
            .collect();
        let back_edge_mask: Vec<bool> = ed.points.chunks_exact(2)
            .map(|q| back_boundary_set.contains(&(q[0].min(q[1]), q[0].max(q[1]))))
            .collect();
        outgroup(&mut out, p, "front_boundary", "edge", &front_edge_mask)?;
        outgroup(&mut out, p, "back_boundary", "edge", &back_edge_mask)?;
    }

    if generate_sides_uvs && output_side {
        let mut uv_data = vec![0.0f32; out.nvertices() * 2];
        if let Some(existing) = out.vertex.get("uv").and_then(|a| a.as_f32()) {
            let len = existing.len().min(uv_data.len());
            uv_data[..len].copy_from_slice(&existing[..len]);
        }
        let mut side_quad_idx = 0;
        let mut prim_vert_offset = 0;
        for (i, &size) in sizes.iter().enumerate() {
            if is_side[i] {
                let d = side_quad_idx % divisions;
                let v0 = d as f32 / divisions as f32;
                let v1 = (d + 1) as f32 / divisions as f32;
                let wall_idx = side_quad_idx / divisions;
                let u0 = wall_idx as f32 / boundary.len().max(1) as f32;
                let u1 = (wall_idx + 1) as f32 / boundary.len().max(1) as f32;
                let idx = prim_vert_offset * 2;
                uv_data[idx] = u0;
                uv_data[idx + 1] = v0;
                uv_data[idx + 2] = u1;
                uv_data[idx + 3] = v0;
                uv_data[idx + 4] = u1;
                uv_data[idx + 5] = v1;
                uv_data[idx + 6] = u0;
                uv_data[idx + 7] = v1;
                side_quad_idx += 1;
            }
            prim_vert_offset += size;
        }
        out.vertex.insert("uv".into(), Arc::new(Attr::f32(2, uv_data)));
    }

    if add_vertex_normals {
        let n_data = compute_vertex_normals(&out, cusp_angle);
        out.vertex.insert("N".into(), Arc::new(Attr::f32(3, n_data)));
        out.point.remove("N");
    } else {
        out.point.remove("N");
    }

    compact(out)
}

