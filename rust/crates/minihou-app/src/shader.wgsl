// Workbench studio lighting and view-normal sphere-capture shading.
struct U {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    base: vec4<f32>,
    wire: vec4<f32>,
    flat_shading: u32,
    // 0..1: how visible the edges are. The CPU lowers it as the mesh gets denser than the screen can show (an edge shorter
    // than about 2 px is not drawn at all), so a dense mesh reads as shading, not as a black or white block of lines.
    wire_fade: f32,
    // 1 when the target is a plain (non-sRGB) texture that is shown as it is, as in the editor: the shader then encodes
    // its linear result to sRGB itself so it matches a window whose surface is an sRGB format.
    encode_srgb: u32,
    matcap: u32, // 0: off, 1: existing captures, 2: SketchUp clean
    object_color: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    floor: vec4<f32>,
    element_size: vec4<f32>,
    element_flags: vec4<u32>,
    element_color: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var matcap: texture_2d<f32>;
@group(0) @binding(2) var matcap_sampler: sampler;
struct Material { factor: vec4<f32> };
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_image: texture_2d<f32>;
@group(1) @binding(2) var base_sampler: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) vpos: vec3<f32>,
    @location(1) vnorm: vec3<f32>,
    @location(2) vcolor: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) uv_valid: f32,
};

// the three-term rig; `n` is a unit view-space normal that faces the viewer
fn light(n: vec3<f32>) -> f32 {
    let key = max(dot(n, normalize(vec3<f32>(-0.35, 0.55, 0.75))), 0.0);
    let fill = max(dot(n, normalize(vec3<f32>(0.6, -0.2, 0.5))), 0.0);
    let hemi = 0.5 + 0.5 * n.y;
    return 0.22 + 0.22 * hemi + 0.62 * key + 0.18 * fill;
}

fn out_color(c: vec3<f32>) -> vec4<f32> {
    if (u.encode_srgb != 0u) {
        let low = c * 12.92;
        let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
        return vec4<f32>(select(high, low, c <= vec3<f32>(0.0031308)), 1.0);
    }
    return vec4<f32>(c, 1.0);
}

@vertex
fn vs_surf(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>, @location(2) color: vec3<f32>) -> VOut {
    var o: VOut;
    o.pos = u.view_proj * vec4<f32>(p, 1.0);
    let vp = u.view * vec4<f32>(p, 1.0);
    o.vpos = vp.xyz;
    o.vnorm = (u.view * vec4<f32>(n, 0.0)).xyz;
    o.vcolor = color;
    o.uv = vec2<f32>(0.0); o.uv_valid = 0.0;
    return o;
}

@vertex
fn vs_tex(@location(0) p:vec3<f32>, @location(1) n:vec3<f32>, @location(2) color:vec3<f32>,
          @location(3) uv:vec2<f32>, @location(4) valid:f32) -> VOut {
    var o:VOut;
    o.pos=u.view_proj*vec4<f32>(p,1.0);
    o.vpos=(u.view*vec4<f32>(p,1.0)).xyz;
    o.vnorm=(u.view*vec4<f32>(n,0.0)).xyz;
    o.vcolor=color; o.uv=uv; o.uv_valid=valid;
    return o;
}

fn textured(i:VOut,front:bool)->vec3<f32> {
    var rgb:vec3<f32>;
    if (i.uv_valid < 0.5) {
        let tile=floor(i.vpos.xy * 8.0);
        let alternate=(i32(tile.x)+i32(tile.y)) & 1;
        rgb=select(vec3<f32>(0.35,0.12,0.42),vec3<f32>(0.9,0.9,0.9),alternate==0);
    // UVs use Blender's bottom-left origin; uploaded PNG/JPEG and normalized Blender pixels start at the top.
    } else { rgb=textureSample(base_image,base_sampler,vec2<f32>(i.uv.x,1.0-i.uv.y)).rgb; }
    return rgb * material.factor.rgb * (shade_surf(i,front) / u.base.rgb);
}
@fragment fn fs_tex(i:VOut,@builtin(front_facing) front:bool)->@location(0) vec4<f32> { return out_color(textured(i,front)); }
@fragment fn fs_tex_xray(i:VOut,@builtin(front_facing) front:bool)->@location(0) vec4<f32> { return vec4<f32>(out_color(textured(i,front)).rgb,0.32); }

fn shade_surf(i: VOut, front: bool) -> vec3<f32> {
    // The unit vector from this point to the eye (view space: the eye is at the origin). In a perspective view it is NOT
    // (0, 0, 1): near the edges of the screen the ray is angled, and testing against the z axis misjudges grazing faces.
    let v = normalize(-i.vpos);
    // The geometric normal of the face under this pixel (screen-space derivatives), turned toward the eye.
    let ng_raw = cross(dpdx(i.vpos), dpdy(i.vpos));
    var ng = v;
    if (dot(ng_raw, ng_raw) > 1e-30) {
        ng = normalize(ng_raw);
    }
    if (dot(ng, v) < 0.0) { ng = -ng; }
    var n = ng;
    if (u.flat_shading == 0u && dot(i.vnorm, i.vnorm) > 1e-20) {
        // smooth (Shaded): the interpolated vertex normal. The vertex normals come from the triangles' winding, so a triangle
        // seen from its back (front == false) gets the same smooth normal turned to face the eye: it stays smooth, where
        // using the face normal here would show every triangle. Where it still points a little away from the eye (a silhouette
        // or a sharp ridge) it is bent back toward the eye plane, which is continuous, instead of flipped.
        n = normalize(i.vnorm);
        if (!front) { n = -n; }
        let d = dot(n, v);
        if (d < 0.0) {
            n = normalize(n - v * d * 1.25);
        }
    }
    if (u.matcap != 0u) {
        return textureSample(matcap, matcap_sampler, vec2<f32>(n.x * 0.5 + 0.5, 0.5 - n.y * 0.5)).rgb;
    }
    return select(u.base.rgb, i.vcolor, u.object_color != 0u) * light(n);
}

@fragment
fn fs_surf(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return out_color(shade_surf(i, front));
}

struct WOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) vpos: vec3<f32>,
    @location(1) vnorm: vec3<f32>,
    @location(2) vcolor: vec3<f32>,
};

@vertex
fn vs_wire(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>, @location(2) color: vec3<f32>) -> WOut {
    var o: WOut;
    o.pos = u.view_proj * vec4<f32>(p, 1.0);
    o.vpos = (u.view * vec4<f32>(p, 1.0)).xyz;
    o.vnorm = (u.view * vec4<f32>(n, 0.0)).xyz;
    o.vcolor = color;
    return o;
}

@fragment
fn fs_wire(i: WOut) -> @location(0) vec4<f32> {
    if (u.matcap == 2u && u.wire.a > 0.5) {
        return vec4<f32>(out_color(vec3<f32>(0.34)).rgb, u.wire_fade);
    }
    var n = vec3<f32>(0.0,0.0,1.0);
    if (dot(i.vnorm,i.vnorm)>1e-20) { n = normalize(i.vnorm); }
    if (dot(n, normalize(-i.vpos)) < 0.0) { n = -n; }
    // lit like the surface so the shape reads even when the edges fill the screen; over a surface (Shaded+Wire) the line is a
    // darker version of the surface colour, fading to the surface itself as the mesh gets denser
    let lit = select(u.base.rgb, i.vcolor, u.object_color != 0u && u.matcap == 0u) * light(n);
    if (u.wire.a > 0.5) {
        return vec4<f32>(out_color(lit * 0.45).rgb, u.wire_fade);
    }
    return out_color(lit);
}

// Selected faces: a flat orange wash
@fragment
fn fs_select(i: VOut) -> @location(0) vec4<f32> { return vec4<f32>(out_color(vec3<f32>(1.0, 0.55, 0.12)).rgb, 0.65); }

// X-ray: the same shading, drawn see-through
@fragment
fn fs_xray(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = out_color(shade_surf(i, front));
    return vec4<f32>(c.rgb, 0.32);
}

// Overlay lines in one flat colour: the point normals and the bounding box
@fragment
fn fs_normals(i: WOut) -> @location(0) vec4<f32> { return out_color(vec3<f32>(0.25, 0.78, 0.92)); }
@fragment
fn fs_bounds(i: WOut) -> @location(0) vec4<f32> { return out_color(vec3<f32>(0.95, 0.72, 0.25)); }

// Object mask for silhouette outline: single-channel write
@fragment
fn fs_mask(i: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}


// Faint surfaces/edges for Wireframe Ghost, Hidden Line Ghost and other objects.
@fragment
fn fs_ghost(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return vec4<f32>(out_color(shade_surf(i,front)).rgb,0.09);
}
@fragment
fn fs_ghost_wire(i: WOut) -> @location(0) vec4<f32> {
    return vec4<f32>(0.65,0.69,0.74,0.17 * select(1.0,u.wire_fade,u.wire.a>0.5));
}

// Constant-pixel squares and segment quads. Clip segments before perspective division.
struct ElementOut {
    @builtin(position) pos:vec4<f32>,
    @location(0) @interpolate(flat) selected:u32,
    @location(1) @interpolate(flat) kind:u32,
};
@vertex
fn vs_element(@builtin(vertex_index) vertex:u32, @location(0) a:vec3<f32>, @location(1) b:vec3<f32>,
              @location(2) kind:u32, @location(3) selected:u32) -> ElementOut {
    let corners=array<vec2<f32>,6>(vec2(-1.,-1.),vec2(1.,-1.),vec2(1.,1.),vec2(-1.,-1.),vec2(1.,1.),vec2(-1.,1.));
    let corner=corners[vertex];
    let scale=select(1.,u.floor.x,kind>=3u);
    var start=u.view_proj*vec4(a*scale,1.); var end=u.view_proj*vec4(b*scale,1.);
    var o:ElementOut; o.selected=selected*u.element_flags.w; o.kind=kind;
    o.pos=vec4(2.,2.,2.,1.);
    if (kind==0u || kind>=3u) {
        if ((kind==0u && u.element_flags.y==0u) || (start.z<0. && end.z<0.)) { return o; }
        if ((kind==3u && u.floor.y==0.) || (kind>=4u && u.floor.z==0.)) { return o; }
        if (start.z<0.) { start=mix(start,end,-start.z/(end.z-start.z)); }
        if (end.z<0.) { end=mix(end,start,-end.z/(start.z-end.z)); }
        if (start.w<=0. || end.w<=0.) { return o; }
        let delta=(end.xy/end.w-start.xy/start.w)*u.element_size.xy;
        let direction=delta/max(length(delta),0.00001);
        let perpendicular=vec2(-direction.y,direction.x);
        var clip=select(start,end,corner.x>0.);
        clip=vec4(clip.xy+perpendicular*corner.y*select(u.element_size.w,1.,kind>=3u)/u.element_size.xy*clip.w,clip.zw);
        o.pos=clip;
    } else {
        if (kind==2u) {
            // At a recessed corner the neighbouring faces are closer at the square's
            // edge pixels. Move the marker toward the eye by its screen footprint.
            let view_point=u.view*vec4(a,1.);
            let perspective=length(vec3(u.view_proj[0].w,u.view_proj[1].w,u.view_proj[2].w))>0.5;
            let depth=select(u.floor.w,max(-view_point.z,0.001),perspective);
            let lift=depth*0.845/u.element_size.y*max(4.,u.element_size.z+1.);
            let toward_eye=vec3(u.view[0].z,u.view[1].z,u.view[2].z);
            start=u.view_proj*vec4(a+toward_eye*lift,1.);
        }
        if (u.element_flags.x==0u || (kind==2u && u.element_flags.z==0u) || start.z<0. || start.w<=0.) { return o; }
        o.pos=vec4(start.xy+corner*u.element_size.z/u.element_size.xy*start.w,start.zw);
    }
    return o;
}
@fragment
fn fs_element(i:ElementOut) -> @location(0) vec4<f32> {
    if (i.kind==3u) {return vec4(u.element_color.rgb,0.16);}
    if (i.kind==4u) {return vec4(0.7,0.16,0.12,0.8);}
    if (i.kind==5u) {return vec4(0.22,0.65,0.18,0.8);}
    if (i.kind==6u) {return vec4(0.18,0.38,0.8,0.8);}
    return select(u.element_color,vec4(1.,0.627,0.157,1.),i.selected!=0u);
}
@fragment
fn fs_element_ghost(i:ElementOut) -> @location(0) vec4<f32> { return vec4(u.element_color.rgb,0.20); }

// Face centre discs share one instanced draw, with camera projection on the GPU.
struct FaceMarkerOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) state: u32,
};
@vertex
fn vs_face_marker(@builtin(vertex_index) vertex:u32, @location(0) center:vec3<f32>, @location(3) state:u32) -> FaceMarkerOut {
    let corners=array<vec2<f32>,6>(vec2(-1.,-1.),vec2(1.,-1.),vec2(1.,1.),vec2(-1.,-1.),vec2(1.,1.),vec2(-1.,1.));
    let corner=corners[vertex];
    let clip=u.view_proj*vec4(center,1.);
    let radius=select(select(2.5,3.5,(state&1u)!=0u),4.,(state&2u)!=0u);
    var o:FaceMarkerOut;
    o.pos=vec4(2.,2.,2.,1.);
    if (clip.z>=0. && clip.w>0.) { o.pos=vec4(clip.xy+corner*(radius+0.5)*2./u.element_size.xy*clip.w,clip.zw); }
    o.uv=corner*(radius+0.5); o.state=state;
    return o;
}
@fragment
fn fs_face_marker(i:FaceMarkerOut) -> @location(0) vec4<f32> {
    let hovered=(i.state&2u)!=0u; let selected=(i.state&1u)!=0u;
    let radius=select(select(2.5,3.5,selected),4.,hovered);
    let d=length(i.uv);
    let orange=vec3(1.,0.627,0.157);
    var color=select(vec3(0.188,0.220,0.282),vec3(0.502,0.565,0.659),d>1.5);
    if (selected) { color=orange; }
    if (hovered) { color=select(vec3(1.),orange,d>3.); }
    return vec4(color,1.-smoothstep(radius-0.5,radius+0.5,d));
}

// One triangle covers the viewport. Reconstruct the floor from each pixel's view ray so
// perspective and orthographic views have no finite mesh boundary.
@vertex
fn vs_infinite_floor(@builtin(vertex_index) vertex:u32) -> @builtin(position) vec4<f32> {
    let xy=array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));
    return vec4(xy[vertex],0.,1.);
}
struct FloorPixel { @location(0) color:vec4<f32>, @builtin(frag_depth) depth:f32 };
fn grid_line(value:f32, spacing:f32) -> f32 {
    let units=value/spacing;
    let edge=abs(fract(units+0.5)-0.5);
    let width=max(fwidth(units),0.0001);
    return 1.-smoothstep(0.,width*1.2,edge);
}
@fragment
fn fs_infinite_floor(@builtin(position) pixel:vec4<f32>) -> FloorPixel {
    let ndc=vec2(pixel.x/u.element_size.x*2.-1.,1.-pixel.y/u.element_size.y*2.);
    let near_clip=u.inv_view_proj*vec4(ndc,0.,1.);
    let far_clip=u.inv_view_proj*vec4(ndc,1.,1.);
    let near=near_clip.xyz/near_clip.w;
    let far=far_clip.xyz/far_clip.w;
    let delta=far-near;
    if (abs(delta.z)<1e-7) { discard; }
    let t=-near.z/delta.z;
    if (t<=0.) { discard; }
    let world=near+t*delta;
    let clip=u.view_proj*vec4(world,1.);
    let spacing=u.floor.x;
    let major=spacing*10.;
    let minor_line=max(grid_line(world.x,spacing),grid_line(world.y,spacing));
    let major_line=max(grid_line(world.x,major),grid_line(world.y,major));
    let x_axis=1.-smoothstep(0.,max(fwidth(world.y),spacing*0.025),abs(world.y));
    let y_axis=1.-smoothstep(0.,max(fwidth(world.x),spacing*0.025),abs(world.x));
    let dist=length((u.view*vec4(world,1.)).xyz);
    let fade=exp(-pow(dist/max(u.floor.w*5.,major*5.),2.));
    let grid_alpha=select(0.,(minor_line*0.10+major_line*0.18)*fade,u.floor.y>0.);
    let axes_alpha=select(0.,max(x_axis,y_axis)*0.75*fade,u.floor.z>0.);
    if (grid_alpha+axes_alpha<0.002) { discard; }
    let axis_color=select(vec3(0.7,0.16,0.12),vec3(0.22,0.65,0.18),y_axis>x_axis);
    var out:FloorPixel;
    out.color=vec4(mix(u.element_color.rgb,axis_color,axes_alpha/max(grid_alpha+axes_alpha,1e-5)),min(grid_alpha+axes_alpha,0.85));
    out.depth=clip.z/clip.w;
    return out;
}

@vertex
fn vs_group(@location(0) p:vec3<f32>)->@builtin(position) vec4<f32> {return u.view_proj*vec4(p,1.);}
@fragment
fn fs_group_fill()->@location(0) vec4<f32> {return vec4(out_color(vec3(0.64,0.22,0.95)).rgb,0.24);}
@fragment
fn fs_group_line()->@location(0) vec4<f32> {return vec4(out_color(vec3(0.72,0.32,1.)).rgb,1.);}
struct GroupPointOut {
    @builtin(position) pos:vec4<f32>,
    @location(0) uv:vec2<f32>,
};
@vertex
fn vs_group_point(@builtin(vertex_index) vertex:u32,@location(0) center:vec3<f32>)->GroupPointOut {
    let corners=array<vec2<f32>,6>(vec2(-1.,-1.),vec2(1.,-1.),vec2(1.,1.),vec2(-1.,-1.),vec2(1.,1.),vec2(-1.,1.));
    let corner=corners[vertex];
    let clip=u.view_proj*vec4(center,1.);
    var o:GroupPointOut;
    o.pos=vec4(2.,2.,2.,1.);
    if (clip.z>=0. && clip.w>0.) { o.pos=vec4(clip.xy+corner*9./u.element_size.xy*clip.w,clip.zw); }
    o.uv=corner;
    return o;
}
@fragment
fn fs_group_point(i:GroupPointOut)->@location(0) vec4<f32> {
    return vec4(out_color(vec3(0.72,0.32,1.)).rgb,1.-smoothstep(0.78,1.,length(i.uv)));
}
@fragment
fn fs_barrier_fill()->@location(0) vec4<f32> {return vec4(out_color(vec3(0.95,0.45,0.08)).rgb,0.24);}
@fragment
fn fs_barrier_line()->@location(0) vec4<f32> {return vec4(out_color(vec3(1.,0.55,0.12)).rgb,1.);}
@fragment
fn fs_target_fill()->@location(0) vec4<f32> {return vec4(out_color(vec3(0.10,0.80,0.80)).rgb,0.26);}
@fragment
fn fs_target_line()->@location(0) vec4<f32> {return vec4(out_color(vec3(0.20,0.92,0.90)).rgb,1.);}
