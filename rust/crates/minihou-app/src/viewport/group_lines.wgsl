// A line segment as two triangles, kept at a constant width in screen pixels.
@vertex
fn vs_group_thick(@builtin(vertex_index) vertex:u32,
                  @location(0) a:vec3<f32>, @location(1) b:vec3<f32>) -> @builtin(position) vec4<f32> {
    let corners=array<vec2<f32>,6>(vec2(-1.,-1.),vec2(1.,-1.),vec2(1.,1.),vec2(-1.,-1.),vec2(1.,1.),vec2(-1.,1.));
    let corner=corners[vertex];
    let start=u.view_proj*vec4(a,1.);
    let end=u.view_proj*vec4(b,1.);
    if (start.w<=0. || end.w<=0. || start.z<0. || end.z<0.) { return vec4(2.,2.,2.,1.); }
    let delta=(end.xy/end.w-start.xy/start.w)*u.element_size.xy;
    let direction=delta/max(length(delta),0.00001);
    let perpendicular=vec2(-direction.y,direction.x);
    let clip=select(start,end,corner.x>0.);
    return vec4(clip.xy+perpendicular*corner.y*2.5/u.element_size.xy*clip.w,clip.zw);
}
