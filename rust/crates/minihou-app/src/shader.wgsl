// Workbench-style shading: no PBR, no textures. Textures later: add a `uv` vertex buffer (location 2) and a
// `@group(1)` texture_2d + sampler pair, and multiply `base` by the sample in fs_surf.
struct U {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    base: vec4<f32>,
    wire: vec4<f32>,
    flat_shading: u32,
    // 0..1: how visible the edges are. The CPU lowers it as the mesh gets denser than the screen can show (an edge shorter
    // than about 2 px is not drawn at all), so a dense mesh reads as shading, not as a black or white block of lines.
    wire_fade: f32,
    _pad0: u32,
    _pad1: u32,
};
@group(0) @binding(0) var<uniform> u: U;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) vpos: vec3<f32>,
    @location(1) vnorm: vec3<f32>,
};

// the three-term rig; `n` is a unit view-space normal that faces the viewer
fn light(n: vec3<f32>) -> f32 {
    let key = max(dot(n, normalize(vec3<f32>(-0.35, 0.55, 0.75))), 0.0);
    let fill = max(dot(n, normalize(vec3<f32>(0.6, -0.2, 0.5))), 0.0);
    let hemi = 0.5 + 0.5 * n.y;
    return 0.22 + 0.22 * hemi + 0.62 * key + 0.18 * fill;
}

@vertex
fn vs_surf(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>) -> VOut {
    var o: VOut;
    o.pos = u.view_proj * vec4<f32>(p, 1.0);
    let vp = u.view * vec4<f32>(p, 1.0);
    o.vpos = vp.xyz;
    o.vnorm = (u.view * vec4<f32>(n, 0.0)).xyz;
    return o;
}

@fragment
fn fs_surf(i: VOut) -> @location(0) vec4<f32> {
    // The geometric normal of the face under this pixel (screen-space derivatives), turned to face the viewer: it says which
    // side of the surface is visible. Two-sided lighting must be decided by this, not by the sign of the interpolated vertex
    // normal: at grazing angles that sign flips on surfaces that are still front-facing and paints hard-edged patches.
    let ng_raw = cross(dpdx(i.vpos), dpdy(i.vpos));
    var ng = normalize(ng_raw);
    if (dot(ng_raw, ng_raw) < 1e-30) {
        ng = vec3<f32>(0.0, 0.0, 1.0);
    }
    if (ng.z < 0.0) { ng = -ng; }
    var n = ng;
    if (u.flat_shading == 0u) {
        // smooth (Shaded): the interpolated vertex normal, on the visible side of the face
        n = normalize(i.vnorm);
        if (dot(n, ng) < 0.0) { n = -n; }
    }
    return vec4<f32>(u.base.rgb * light(n), 1.0);
}

struct WOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) vnorm: vec3<f32>,
};

@vertex
fn vs_wire(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>) -> WOut {
    var o: WOut;
    o.pos = u.view_proj * vec4<f32>(p, 1.0);
    o.vnorm = (u.view * vec4<f32>(n, 0.0)).xyz;
    return o;
}

@fragment
fn fs_wire(i: WOut) -> @location(0) vec4<f32> {
    var n = normalize(i.vnorm);
    if (n.z < 0.0) { n = -n; }
    // lit like the surface so the shape reads even when the edges fill the screen; over a surface (Shaded+Wire) the line is a
    // darker version of the surface colour, fading to the surface itself as the mesh gets denser
    let lit = u.base.rgb * light(n);
    if (u.wire.a > 0.5) {
        return vec4<f32>(lit * mix(1.0, 0.45, u.wire_fade), 1.0);
    }
    return vec4<f32>(lit, 1.0);
}
