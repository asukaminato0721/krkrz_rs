@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
}
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var<uniform> rect: vec4<f32>;
@fragment fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = (position.xy - rect.xy) / rect.zw;
    if any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) {
        discard;
    }
    let size = textureDimensions(image);
    let pixel = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2<i32>(0), vec2<i32>(size) - 1);
    return vec4<f32>(textureLoad(image, pixel, 0).rgb, 1.0);
}
