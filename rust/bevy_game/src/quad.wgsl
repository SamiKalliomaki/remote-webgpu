// Everything in the game is a flat, outlined rectangle, so one instanced
// quad pipeline draws the whole frame: scenery, coins, characters and the
// heads-up display.

struct Camera {
    center: vec2<f32>,
    half_extent: vec2<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) fill: vec4<f32>,
    @location(1) border: vec4<f32>,
    @location(2) local: vec2<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) index: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) border: vec4<f32>,
) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );

    let local = corners[index];
    let world = center + local * half_size;
    // The HUD passes an identity camera, so its instances are already in
    // clip space.
    let ndc = (world - camera.center) / camera.half_extent;

    var out: VertexOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.fill = fill;
    out.border = border;
    out.local = local;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // A border alpha of zero means "no outline".
    let edge = max(abs(in.local.x), abs(in.local.y));
    if (in.border.a > 0.0 && edge > 0.78) {
        return in.border;
    }
    return in.fill;
}
