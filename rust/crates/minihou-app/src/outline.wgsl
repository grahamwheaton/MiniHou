@group(0) @binding(0) var mask_tex: texture_2d<f32>;

@vertex
fn vs_outline(@builtin(vertex_index) in_vertex_index: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    return vec4<f32>(pos[in_vertex_index], 0.0, 1.0);
}

@fragment
fn fs_outline(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pos.xy);
    let dims = vec2<i32>(textureDimensions(mask_tex));

    // Outside the object only: if inside, discard
    let center = textureLoad(mask_tex, p, 0).r;
    if (center > 0.5) {
        discard;
    }

    // 2 px radius around silhouette: check 12 neighbors within radius <= 2
    let offsets = array<vec2<i32>, 12>(
        vec2<i32>(-1, 0), vec2<i32>(1, 0), vec2<i32>(0, -1), vec2<i32>(0, 1),
        vec2<i32>(-1, -1), vec2<i32>(1, -1), vec2<i32>(-1, 1), vec2<i32>(1, 1),
        vec2<i32>(-2, 0), vec2<i32>(2, 0), vec2<i32>(0, -2), vec2<i32>(0, 2)
    );
    var hit = false;
    for (var i = 0u; i < 12u; i++) {
        let np = p + offsets[i];
        if (np.x >= 0 && np.x < dims.x && np.y >= 0 && np.y < dims.y) {
            if (textureLoad(mask_tex, np, 0).r > 0.5) {
                hit = true;
                break;
            }
        }
    }

    if (!hit) {
        discard;
    }

    // #ffa028 in sRGB: (1.0, 160.0/255.0, 40.0/255.0, 1.0)
    return vec4<f32>(1.0, 160.0 / 255.0, 40.0 / 255.0, 1.0);
}
