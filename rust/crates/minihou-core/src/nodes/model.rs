//! Faithful polygon operations from minihou/nodes/model.py. Shared builders follow _build.py.
use super::{GEO, Inputs, NodeSpec, groups};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, AttrTable, Geometry, Topology},
    normals,
    params::Params,
    topo,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|k| a[k] * b[k]).sum()
}
pub(super) fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k] + b[k])
}
pub(super) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k] - b[k])
}
pub(super) fn mul(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|v| v * s)
}
pub(super) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(super) fn len(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
pub(super) fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = len(a);
    mul(a, 1.0 / if n < 1e-20 { 1.0 } else { n })
}
// Python normalize_rows returns float32 even when its input is float64.
pub(super) fn unit32(a: [f64; 3]) -> [f64; 3] {
    unit(a).map(|v| v as f32 as f64)
}
mod build;
pub(super) use build::{positions, range, gather, promote_corner_uv_sets, concat, lerp, mean, make, rebuild, prim_mask, group_name, write_group, outgroup};
pub(crate) use build::{corners, polygons, compact};

pub const DIVIDE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "divide",
    inputs: GEO,
    params: crate::params!("group":""),
    cook: divide,
};
pub const POLYFILL: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "polyfill",
    inputs: GEO,
    params: super::polyfill::PARAMS,
    cook: super::polyfill::polyfill,
};
pub const MIRROR: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "mirror",
    inputs: GEO,
    params: crate::params!("origin":[0.,0.,0.];"normal":[1.,0.,0.];"keep_original":true;"consolidate_seam":true;"distance":0.001;"outgrp_mirrored":false;"outgrp_mirrored_name":"mirrored";"outgrp_mirroredpoints":false;"outgrp_mirroredpoints_name":"mirroredpoints";"outgrp_seam":false;"outgrp_seam_name":"seampoints";"group":"";"operation":"mirror_all";"offset":0.0;"reverse":false;"cut":false;"unshared_only":false),
    cook: super::mirror::mirror,
};
pub const CLIP: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "clip",
    inputs: GEO,
    params: crate::params!("origin":[0.,0.,0.];"normal":[0.,0.,1.];"keep":"above";"outgrp_above":true;"outgrp_above_name":"above";"outgrp_below":false;"outgrp_below_name":"below";"outgrp_cutpolys":false;"outgrp_cutpolys_name":"cutpolys";"outgrp_cutpoints":false;"outgrp_cutpoints_name":"cutpoints";"outgrp_cutedges":false;"outgrp_cutedges_name":"cutedges"),
    cook: clip,
};
pub const POLYEXTRUDE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "polyextrude",
    inputs: &[super::InputSpec{name:"geo",multi:false,optional:false},super::InputSpec{name:"spine",multi:false,optional:true}],
    params: crate::params!("group":"";"edges":"";"points":"";"distance":0.5;"direction":[0.,0.,0.];"inset":0.;"mode":"connected";"output_back":false;"outgrp_front":true;"outgrp_front_name":"extrudeFront";"outgrp_back":false;"outgrp_back_name":"extrudeBack";"outgrp_side":true;"outgrp_side_name":"extrudeSide";"divide_into":"connected";"split_group":"";"extrusion_mode":"primitive_normal";"twist":0.;"divisions":1;"spine_shape":"straight";"transform_front":true;"output_front":true;"output_side":true;"outgrp_front_boundary":false;"outgrp_front_boundary_name":"extrudeFrontBoundary";"outgrp_back_boundary":false;"outgrp_back_boundary_name":"extrudeBackBoundary";"preserve_groups":true;"copy_edge_groups":true;"copy_point_groups":true;"copy_primitive_groups":true;"limit_insetting":false;"use_common_limit":false;"offset_even":true;"add_vertex_normals":false;"cusp_angle":60.;"generate_sides_uvs":false;"uv_style":"proportional";"uv_scaling":"match_incident";
        "front_magnitude":1.;"back_magnitude":1.;"front_stiffness":1.;"back_stiffness":1.;"interpolation":"rotating_frame";"division_spacing":"uniform";
        "reverse_spine_direction":false;"axial_rotation":0.;"front_blend":0.;"back_blend":0.;
        "thickness_scale":1.;"thickness_ramp":[[0.,0.5],[1.,0.5]];"thickness_attribute":"";
        "twist_range_half_width":1.;"local_twist_ramp":[[0.,0.5],[1.,0.5]];"twist_attribute":"";
        "distance_scale_attribute":"zscale";"inset_scale_attribute":"insetscale";"twist_scale_attribute":"twistscale";"divisions_scale_attribute":"divsscale";
        "local_x_attribute":"localx";"local_z_attribute":"localz";"local_center_attribute":"localctr";"front_translate":[0.,0.,0.]),
    cook: polyextrude,
};
mod cuts;
pub(super) use cuts::ear_clip;
use cuts::{divide, clip};

fn polyextrude(inputs: &Inputs, p: &Params) -> Result<Geometry> {
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
pub(super) fn extrude(
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
        Some(super::planar_offset::offset_selected(
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    pub(super) fn quad() -> Geometry {
        Geometry::from_arrays(
            vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 2., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap()
    }
    fn cook(s: &NodeSpec, g: Geometry, v: serde_json::Value) -> Geometry {
        let p = Params::resolve(s.params, v.as_object()).unwrap();
        (s.cook)(&[vec![Arc::new(g)]], &p).unwrap()
    }
    #[test]
    fn reverse_and_divide_keep_corner_rows() {
        let mut g = quad();
        g.vertex
            .insert("id".into(), Arc::new(Attr::i32(1, vec![10, 11, 12, 13])));
        let r = cook(crate::nodes::registry().iter().find(|spec| spec.name == "reverse").unwrap(), g.clone(), json!({}));
        assert_eq!(r.topology.vertex_point, vec![3, 2, 1, 0]);
        assert_eq!(r.vertex["id"].data, AttrData::I32(vec![13, 12, 11, 10]));
        let d = cook(&DIVIDE, g, json!({}));
        assert_eq!(d.topology.vertex_point, vec![0, 1, 2, 0, 2, 3]);
        d.validate().unwrap();
    }
    #[test]
    fn extrusion_closed_shell_and_zero_noop() {
        let g = quad();
        let out = cook(
            &POLYEXTRUDE,
            g.clone(),
            json!({"distance":2.,"output_back":true}),
        );
        assert_eq!((out.npoints, out.nprims()), (8, 6));
        assert!(
            topo::edges(&out.topology, out.npoints)
                .count
                .iter()
                .all(|&c| c == 2)
        );
        assert_eq!(
            out.prim["group_extrudeSide"].data,
            AttrData::I32(vec![0, 1, 1, 1, 1, 0])
        );
        let z = cook(&POLYEXTRUDE, g.clone(), json!({"distance":0.}));
        assert_eq!(z.p(), g.p());
        assert_eq!(z.topology, g.topology);
    }
    #[test]
    fn polyextrude_axis_direction_changes_preview_geometry() {
        let g=quad();
        let normal=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.}));
        let axis=cook(&POLYEXTRUDE,g,json!({"distance":1.,"direction":[1.,0.,0.]}));
        assert_eq!(normal.nprims(),axis.nprims());
        assert!(axis.p().chunks_exact(3).any(|p|p[0]>2.9));
        assert_ne!(normal.p(),axis.p());
    }
    #[test]
    fn polyextrude_point_can_continue_along_its_new_open_segment() {
        let first=cook(&POLYEXTRUDE,quad(),json!({"points":"0","distance":0.05}));
        let second=cook(&POLYEXTRUDE,first.clone(),json!({"points":"4","distance":0.05}));
        assert_eq!(second.npoints,6);assert_eq!(second.nprims(),3);
        let a=&first.p()[12..15];let b=&second.p()[15..18];
        assert!((b[2]-a[2]-0.05).abs()<1e-6);
        assert_eq!(&second.topology.closed.as_ref().unwrap()[..],&[true,false,false]);
        second.validate().unwrap();
    }
    #[test]
    fn polyextrude_edge_and_point_previews_keep_original_geometry() {
        let g=quad();
        let edge=cook(&POLYEXTRUDE,g.clone(),json!({"edges":"0-1","distance":1.}));
        assert_eq!(edge.nprims(),g.nprims()+1);
        assert_eq!(edge.npoints,g.npoints+2);
        assert_eq!(edge.topology.vertex_point[0..4],g.topology.vertex_point[0..4]);
        assert!(edge.p().chunks_exact(3).any(|p|p[2]>0.9));
        let point=cook(&POLYEXTRUDE,g.clone(),json!({"points":"0","distance":1.}));
        assert_eq!(point.nprims(),g.nprims()+1);
        assert_eq!(point.npoints,g.npoints+1);
        assert_eq!(point.topology.closed.as_ref().unwrap(),&vec![true,false]);
        point.validate().unwrap();
    }
    #[test]
    fn polyextrude_houdini_parity_extrusion_options() {
        let g = quad();
        // 1. Divisions
        let div = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "divisions": 3}));
        // Quad has 4 boundary edges * 3 divisions = 12 side quads + 1 front = 13 prims
        assert_eq!(div.nprims(), 13);
        div.validate().unwrap();

        // 2. Output toggles
        let no_front = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "output_front": false}));
        assert_eq!(no_front.nprims(), 4);
        assert!(!no_front.prim.contains_key("group_extrudeFront") || no_front.prim["group_extrudeFront"].data == AttrData::I32(vec![0, 0, 0, 0]));

        let no_side = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "output_side": false}));
        assert_eq!(no_side.nprims(), 1);

        // 3. Twist
        let straight = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0}));
        let twisted = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "twist": 45.0}));
        assert_ne!(straight.p(), twisted.p());

        // 4. Vertex normals
        let with_n = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "add_vertex_normals": true, "cusp_angle": 60.0}));
        assert!(with_n.vertex.contains_key("N"));
        assert_eq!(with_n.vertex["N"].width, 3);

        // 5. Generate sides UVs
        let with_uv = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "generate_sides_uvs": true}));
        assert!(with_uv.vertex.contains_key("uv"));
        assert_eq!(with_uv.vertex["uv"].width, 2);

        // 6. Boundary edge groups
        let with_bounds = cook(&POLYEXTRUDE, g.clone(), json!({
            "distance": 1.0,
            "outgrp_front_boundary": true,
            "outgrp_front_boundary_name": "front_edges"
        }));
        assert!(with_bounds.vertex.contains_key("edge_group_front_edges") || with_bounds.edge.contains_key("group_front_edges"));
    }
    #[test]
    fn polyextrude_spine_controls_change_bridge_and_follow_second_input() {
        let g=quad();
        let baseline=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"divisions":4}));
        let curved=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"divisions":4,"spine_shape":"curved","front_magnitude":2.0,"back_stiffness":0.5}));
        assert_ne!(curved.p(),baseline.p());
        let curved_translated=json!({"distance":1.0,"divisions":4,"spine_shape":"curved","front_magnitude":2.0,"front_translate":[0.3,0.0,0.0]});
        let uniform=cook(&POLYEXTRUDE,g.clone(),curved_translated.clone());
        let mut spaced_params=curved_translated;spaced_params["division_spacing"]=json!("curvature_sensitive");
        let spaced=cook(&POLYEXTRUDE,g.clone(),spaced_params);
        assert_ne!(spaced.p(),uniform.p());
        let mut spine=Geometry::from_arrays(vec![0.,0.,0.,0.5,0.,0.5,0.,0.,1.],vec![0,1,2],vec![0,3],Some(vec![false])).unwrap();
        spine.point.insert("thick".into(),Arc::new(Attr::f32(1,vec![0.,0.1,0.])));
        spine.point.insert("bend".into(),Arc::new(Attr::f32(1,vec![0.,0.2,0.])));
        let p=Params::resolve(POLYEXTRUDE.params,json!({"distance":1.0,"divisions":4,"spine_shape":"curve_from_second_input"}).as_object()).unwrap();
        let along=(POLYEXTRUDE.cook)(&[vec![Arc::new(g.clone())],vec![Arc::new(spine.clone())]],&p).unwrap();
        assert_ne!(along.p(),baseline.p());
        let attributes=Params::resolve(POLYEXTRUDE.params,json!({"distance":1.0,"divisions":4,"spine_shape":"curve_from_second_input","thickness_attribute":"thick","twist_attribute":"bend","twist_range_half_width":90.0}).as_object()).unwrap();
        let attributed=(POLYEXTRUDE.cook)(&[vec![Arc::new(g.clone())],vec![Arc::new(spine.clone())]],&attributes).unwrap();
        assert_ne!(attributed.p(),along.p());
        let reversed=Params::resolve(POLYEXTRUDE.params,json!({"distance":1.0,"divisions":4,"spine_shape":"curve_from_second_input","reverse_spine_direction":true,"axial_rotation":30.0,"front_blend":0.5,"back_blend":0.5}).as_object()).unwrap();
        let other=(POLYEXTRUDE.cook)(&[vec![Arc::new(g)],vec![Arc::new(spine)]],&reversed).unwrap();
        assert_ne!(along.p(),other.p());
    }
    #[test]
    fn polyextrude_thickness_and_twist_ramps_affect_bridge() {
        let g=quad();
        let base=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"divisions":3}));
        let thick=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"divisions":3,"thickness_scale":2.0,"thickness_ramp":[[0.0,0.5],[1.0,0.75]]}));
        assert_ne!(base.p(),thick.p());
        let twisted=cook(&POLYEXTRUDE,g,json!({"distance":1.0,"divisions":3,"twist_range_half_width":90.0,"local_twist_ramp":[[0.0,0.5],[1.0,1.0]]}));
        assert_ne!(base.p(),twisted.p());
    }
    #[test]
    fn polyextrude_local_control_attributes_and_frames() {
        let mut g=quad();
        g.prim.insert("zscale".into(),Arc::new(Attr::f32(1,vec![2.])));
        g.prim.insert("insetscale".into(),Arc::new(Attr::f32(1,vec![0.5])));
        g.prim.insert("twistscale".into(),Arc::new(Attr::f32(1,vec![0.])));
        g.prim.insert("divsscale".into(),Arc::new(Attr::f32(1,vec![2.])));
        g.prim.insert("localz".into(),Arc::new(Attr::f32(3,vec![1.,0.,0.])));
        g.prim.insert("localx".into(),Arc::new(Attr::f32(3,vec![0.,1.,0.])));
        g.prim.insert("localctr".into(),Arc::new(Attr::f32(3,vec![5.,0.,0.])));
        let out=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"inset":0.1,"twist":45.0,"divisions":2,"front_translate":[1.,0.,0.]}));
        assert_eq!(out.nprims(),17); // 4 edges * (2 divisions * divsscale 2) + front
        let unscaled=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"inset":0.1,"twist":45.0,"divisions":2,"front_translate":[1.,0.,0.],"distance_scale_attribute":"missing","divisions_scale_attribute":"missing"}));
        assert_ne!(out.p(),unscaled.p());
        let no_frame=cook(&POLYEXTRUDE,g,json!({"distance":1.0,"inset":0.1,"twist":45.0,"divisions":2,"front_translate":[1.,0.,0.],"local_x_attribute":"missing","local_z_attribute":"missing","local_center_attribute":"missing"}));
        assert_ne!(out.p(),no_frame.p());
    }
    #[test]
    fn polyextrude_offset_even_and_houdini_output_group_options() {
        let mut g=quad();
        g.point.insert("group_seed".into(),Arc::new(Attr::i32(1,vec![1,0,0,0])));
        g.prim.insert("group_source".into(),Arc::new(Attr::i32(1,vec![1])));
        g.vertex.insert("edge_group_source".into(),Arc::new(Attr::i32(1,vec![1,0,0,0])));
        let even=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"inset":0.15,"offset_even":true}));
        let uneven=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"inset":0.15,"offset_even":false}));
        assert_ne!(even.p(),uneven.p());
        let out=cook(&POLYEXTRUDE,g,json!({"distance":1.0,"copy_point_groups":false,"copy_primitive_groups":false,"copy_edge_groups":false,
            "outgrp_front_name":"HoudiniFront","outgrp_side_name":"HoudiniSide","outgrp_back":true,"output_back":true}));
        assert!(out.prim.contains_key("group_HoudiniFront") && out.prim.contains_key("group_HoudiniSide") && out.prim.contains_key("group_extrudeBack"));
        assert!(out.point["group_seed"].as_i32().unwrap().iter().skip(4).all(|v|*v==0));
        assert!(out.prim["group_source"].as_i32().unwrap().iter().take(5).all(|v|*v==0));
        assert!(out.vertex["edge_group_source"].as_i32().unwrap().iter().take(20).all(|v|*v==0));
    }
    #[test]
    fn clip_shared_intersections_and_int_sources() {
        let mut g = quad();
        g.point
            .insert("id".into(), Arc::new(Attr::i32(1, vec![7, 8, 9, 10])));
        let out = cook(
            &CLIP,
            g,
            json!({"origin":[1.,0.,0.],"normal":[1.,0.,0.],"keep":"both"}),
        );
        assert_eq!((out.npoints, out.nprims()), (6, 2));
        assert_eq!(out.point["id"].data, AttrData::I32(vec![7, 8, 9, 10, 7, 9]));
        out.validate().unwrap();
    }
    #[test]
    fn concave_divide_area() {
        let g = Geometry::from_arrays(
            vec![
                0., 0., 0., 2., 0., 0., 2., 1., 0., 1., 1., 0., 1., 2., 0., 0., 2., 0.,
            ],
            vec![0, 1, 2, 3, 4, 5],
            vec![0, 6],
            None,
        )
        .unwrap();
        let out = cook(&DIVIDE, g, json!({}));
        assert_eq!(out.nprims(), 4);
        let n = normals::prim_normals(out.p(), &out.topology);
        assert!((n.iter().map(|v| len(*v)).sum::<f64>() / 2. - 3.).abs() < 1e-6);
        assert!(n.iter().all(|v| v[2] > 0.));
    }
}
/// Eigenvectors (columns) of a symmetric 3x3 matrix. Used for SVD-equivalent least squares on local Jacobians.
pub(super) fn eigen(mut a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut v = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    for _ in 0..64 {
        let mut pair = (0, 1);
        for (i, j) in [(0, 2), (1, 2)] {
            if a[i][j].abs() > a[pair.0][pair.1].abs() {
                pair = (i, j);
            }
        }
        let (p, q) = pair;
        if a[p][q].abs() <= 1e-16 * (a[0][0].abs() + a[1][1].abs() + a[2][2].abs()).max(1e-300) {
            break;
        }
        let tau = (a[q][q] - a[p][p]) / (2. * a[p][q]);
        let t = if tau >= 0. {
            1. / (tau + (1. + tau * tau).sqrt())
        } else {
            -1. / (-tau + (1. + tau * tau).sqrt())
        };
        let c = 1. / (1. + t * t).sqrt();
        let s = t * c;
        let ap = a[p][p];
        let aq = a[q][q];
        let off = a[p][q];
        a[p][p] = ap - t * off;
        a[q][q] = aq + t * off;
        a[p][q] = 0.;
        a[q][p] = 0.;
        for k in 0..3 {
            if k != p && k != q {
                let x = a[k][p];
                let y = a[k][q];
                a[k][p] = c * x - s * y;
                a[p][k] = a[k][p];
                a[k][q] = s * x + c * y;
                a[q][k] = a[k][q];
            }
            let x = v[k][p];
            let y = v[k][q];
            v[k][p] = c * x - s * y;
            v[k][q] = s * x + c * y;
        }
    }
    ([a[0][0], a[1][1], a[2][2]], v)
}
#[cfg(test)]
mod python_oracles {
    use super::*;
    use serde_json::{Map, Value};

    fn table(v: &Value) -> AttrTable {
        v.as_object()
            .unwrap()
            .iter()
            .map(|(name, a)| {
                let width = a["width"].as_u64().unwrap() as usize;
                let rows = a["data"].as_array().unwrap();
                let attr = if a["dtype"] == "float32" {
                    Attr::f32(
                        width,
                        rows.iter().map(|x| x.as_f64().unwrap() as f32).collect(),
                    )
                } else {
                    Attr::i32(
                        width,
                        rows.iter().map(|x| x.as_i64().unwrap() as i32).collect(),
                    )
                };
                (name.clone(), Arc::new(attr))
            })
            .collect()
    }
    fn geometry(v: &Value) -> Geometry {
        let point = table(&v["point"]);
        let closed = v["closed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_bool().unwrap())
            .collect();
        let topo = Topology {
            vertex_point: v["vp"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap() as i32)
                .collect(),
            prim_offsets: v["offsets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap() as i32)
                .collect(),
            closed: Some(closed),
        };
        let g = Geometry { packed: Default::default(), edge: Default::default(),
            npoints: point["P"].rows(),
            topology: Arc::new(topo),
            point,
            vertex: table(&v["vertex"]),
            prim: table(&v["prim"]),
            detail: v["detail"].as_object().unwrap().clone(),
        };
        g.validate().unwrap();
        g
    }
    fn compare(name: &str, a: &Geometry, b: &Geometry) {
        assert_eq!(a.npoints, b.npoints, "{name}: point count");
        assert_eq!(
            a.topology.vertex_point, b.topology.vertex_point,
            "{name}: corner points"
        );
        assert_eq!(
            a.topology.prim_offsets, b.topology.prim_offsets,
            "{name}: offsets"
        );
        assert_eq!(
            a.topology.closed_mask(),
            b.topology.closed_mask(),
            "{name}: closed flags"
        );
        // Python lacks the new neutral slot and binding metadata; its detail keys still match exactly.
        super::super::oracle_material_detail::compare(name, a, b);
        let (original_a, original_b) = (a, b);
        for domain in ["point", "vertex", "prim"] {
            let (a, b) = (a.table(domain), b.table(domain));
            let neutral_index = domain == "prim" && super::super::oracle_material_detail::neutral_index_added(original_a, original_b);
            // `point_group_knife` is a Rust-only group written by the knife node; Python has no such attribute.
            let rust_only = |key: &str| key == "point_group_knife";
            assert_eq!(
                a.keys().filter(|key| (!neutral_index || key.as_str() != "material_index") && !rust_only(key)).collect::<Vec<_>>(),
                b.keys().collect::<Vec<_>>(),
                "{name}: {domain} attribute names"
            );
            for (key, a) in a {
                if neutral_index && key == "material_index" { continue; }
                if rust_only(key) { continue; }
                let b = &b[key];
                let invalid_uv = if domain == "vertex" {
                    super::super::oracle_material_detail::invalid_uv_rows(
                        original_a, original_b, key, &(0..a.rows()).collect::<Vec<_>>())
                } else { vec![false; a.rows()] };
                assert_eq!(a.width, b.width, "{name}: {domain} {key} width");
                match (&a.data, &b.data) {
                    (AttrData::F32(a), AttrData::F32(b)) => {
                        assert_eq!(a.len(), b.len(), "{name}: {domain} {key} length");
                        for (i, (&a, &b)) in a.iter().zip(b).enumerate() {
                            if invalid_uv[i / original_a.table(domain)[key].width] && a.is_nan() && b == 0. { continue; }
                            assert!(
                                (a - b).abs() <= 1e-5 + 1e-5 * b.abs(),
                                "{name}: {domain} {key}[{i}]: Rust {a}, Python {b}"
                            );
                        }
                    }
                    (AttrData::I32(a), AttrData::I32(b)) => {
                        assert_eq!(a, b, "{name}: {domain} {key}")
                    }
                    _ => panic!("{name}: {domain} {key} dtype"),
                }
            }
        }
    }
    #[test]
    fn modeling_matches_python_topology_attributes_and_defaults() {
        // Regenerate only with tools/dump_rust_modeling_fixtures.py; Python is the oracle.
        // New source-less faces may add a neutral slot and NaN UV where Python stored zero.
        let cases: Value =
            serde_json::from_str(include_str!("test_data/modeling_parity.json")).unwrap();
        for c in cases.as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let spec = super::super::get(c["type"].as_str().unwrap()).unwrap();
            let defaults: Map<String, Value> = spec
                .params
                .iter()
                .map(|p| (p.name.into(), (p.default)()))
                .collect();
            let expected_defaults = c["defaults"].as_object().unwrap();
            // Rust may add Rust-only params; every Python param must exist with the same default.
            for (key, expected) in expected_defaults {
                assert!(
                    defaults
                        .get(key)
                        .is_some_and(|value| crate::params::same_value(value, expected)),
                    "{name}: parameter {key} default"
                );
            }
            let p = Params::resolve(spec.params, c["params"].as_object()).unwrap();
            let mut inputs: Vec<Vec<Arc<Geometry>>> = c["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| vec![Arc::new(geometry(v))])
                .collect();
            inputs.resize_with(spec.inputs.len(), Vec::new);
            let result = (spec.cook)(&inputs, &p);
            if c.get("error").is_some() {
                assert!(result.is_err(), "{name}: expected rejection");
            } else {
                let actual = result.unwrap_or_else(|e| panic!("{name}: {e}"));
                actual.validate().unwrap();
                compare(name, &actual, &geometry(&c["expected"]));
            }
        }
    }
    #[test]
    fn loop_cut_is_polysplit() {
        let alias = super::super::get("loop_cut").unwrap();
        let canonical = super::super::get("polysplit").unwrap();
        assert_eq!(alias.params.len(), canonical.params.len());
        let input = super::tests::quad();
        let given = serde_json::json!({"split_edges":"p0-1"});
        let p = Params::resolve(alias.params, given.as_object()).unwrap();
        let inputs = [vec![Arc::new(input)]];
        compare(
            "loop_cut alias",
            &(alias.cook)(&inputs, &p).unwrap(),
            &(canonical.cook)(&inputs, &p).unwrap(),
        );
    }
}
/// Small least-squares solve with a one-sided Jacobi SVD. Avoid forming A^T A:
/// squaring the condition number loses the null-space behavior of numpy.pinv/lstsq.
pub(super) fn least_squares<const N: usize>(
    rows: &[[f64; N]],
    rhs: &[f64],
    rcond: f64,
) -> [f64; N] {
    let mut a = rows.to_vec();
    let mut v = [[0.; N]; N];
    for i in 0..N {
        v[i][i] = 1.;
    }
    for _ in 0..64 {
        let mut changed = false;
        for p in 0..N {
            for q in p + 1..N {
                let aa = a.iter().map(|r| r[p] * r[p]).sum::<f64>();
                let bb = a.iter().map(|r| r[q] * r[q]).sum::<f64>();
                let ab = a.iter().map(|r| r[p] * r[q]).sum::<f64>();
                if ab.abs() <= f64::EPSILON * (aa * bb).sqrt() || ab == 0. {
                    continue;
                }
                let tau = (bb - aa) / (2. * ab);
                let t = if tau >= 0. {
                    1. / (tau + tau.hypot(1.))
                } else {
                    -1. / (-tau + tau.hypot(1.))
                };
                let c = 1. / (1. + t * t).sqrt();
                let s = t * c;
                for r in &mut a {
                    let x = r[p];
                    let y = r[q];
                    r[p] = c * x - s * y;
                    r[q] = s * x + c * y;
                }
                for r in &mut v {
                    let x = r[p];
                    let y = r[q];
                    r[p] = c * x - s * y;
                    r[q] = s * x + c * y;
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let norm: [f64; N] = std::array::from_fn(|k| a.iter().map(|r| r[k] * r[k]).sum::<f64>().sqrt());
    let limit = norm.iter().copied().fold(0., f64::max) * rcond;
    let mut result = [0.; N];
    for k in 0..N {
        if norm[k] > limit && norm[k] > 0. {
            let coeff =
                a.iter().zip(rhs).map(|(r, &b)| r[k] * b).sum::<f64>() / (norm[k] * norm[k]);
            for i in 0..N {
                result[i] += v[i][k] * coeff;
            }
        }
    }
    result
}
#[cfg(test)]
mod least_squares_tests {
    use super::*;
    #[test]
    fn singular_solve_returns_minimum_norm_solution() {
        let rows = [[1., 2., 0.], [2., 4., 0.], [0., 0., 0.]];
        let result = least_squares(&rows, &[3., 6., 1.], 1e-15);
        assert!(len(sub(result, [0.6, 1.2, 0.])) < 1e-12);
    }
    #[test]
    fn full_rank_solve_recovers_an_affine_field() {
        let rows = [
            [1., 0., 0., 1.],
            [0., 1., 0., 1.],
            [0., 0., 1., 1.],
            [1., 1., 1., 1.],
            [2., -1., 3., 1.],
        ];
        let exact = [2., -3., 4., 0.5];
        let rhs: Vec<_> = rows
            .iter()
            .map(|r| (0..4).map(|k| r[k] * exact[k]).sum())
            .collect();
        let result = least_squares(&rows, &rhs, 5. * f64::EPSILON);
        for i in 0..4 {
            assert!((result[i] - exact[i]).abs() < 1e-11);
        }
    }
}
