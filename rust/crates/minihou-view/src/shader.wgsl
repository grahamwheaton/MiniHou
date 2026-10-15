// Workbench-style shading: no PBR, no textures. Textures later: add a `uv` vertex buffer (location 2) and a
// `@group(1)` texture_2d + sampler pair, and multiply `base` by the sample in fs_surf.
struct U {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    base: vec4<f32>,
    wire: vec4<f32>,
    flat_shading: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};
@group(0) @binding(0) var<uniform> u: U;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) vpos: vec3<f32>,
    @location(1) vnorm: vec3<f32>,
};

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
    var n: vec3<f32>;
    if (u.flat_shading != 0u) {
        // flat (Solid): the face normal from screen-space derivatives, so no normal buffer is read
        n = normalize(cross(dpdx(i.vpos), dpdy(i.vpos)));
    } else {
        n = normalize(i.vnorm);
    }
    // two-sided: face the viewer (view space looks down -z)
    if (n.z < 0.0) { n = -n; }
    let key = max(dot(n, normalize(vec3<f32>(-0.35, 0.55, 0.75))), 0.0);
    let fill = max(dot(n, normalize(vec3<f32>(0.6, -0.2, 0.5))), 0.0);
    let hemi = 0.5 + 0.5 * n.y;
    let l = 0.22 + 0.22 * hemi + 0.62 * key + 0.18 * fill;
    return vec4<f32>(u.base.rgb * l, 1.0);
}

@vertex
fn vs_wire(@location(0) p: vec3<f32>) -> @builtin(position) vec4<f32> {
    return u.view_proj * vec4<f32>(p, 1.0);
}

@fragment
fn fs_wire() -> @location(0) vec4<f32> {
    return u.wire;
}
