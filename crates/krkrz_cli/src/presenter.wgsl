struct Output { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vertex(@builtin(vertex_index) index: u32) -> Output {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return Output(vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}
@group(0) @binding(0) var image: texture_2d<f32>;
@fragment fn fragment(input: Output) -> @location(0) vec4<f32> {
    let size = textureDimensions(image);
    let pixel = clamp(vec2<i32>(input.uv * vec2<f32>(size)), vec2<i32>(0), vec2<i32>(size) - 1);
    return vec4<f32>(textureLoad(image, pixel, 0).rgb, 1.0);
}
