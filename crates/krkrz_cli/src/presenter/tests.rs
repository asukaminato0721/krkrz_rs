use super::{create_pipeline, render_frame};
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires a GPU adapter (a software Vulkan adapter also works)"]
fn presentation_clips_without_rescaling() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("GPU adapter required for presentation test");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let (layout, pipeline) = create_pipeline(&device, format);
    let source = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test source"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let pixels: Vec<u8> = (0..16)
        .flat_map(|i| [i % 4 * 60, i / 4 * 60, 128, 255])
        .collect();
    queue.write_texture(
        source.as_image_copy(),
        &pixels,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(4),
        },
        source.size(),
    );
    let source_view = source.create_view(&Default::default());
    for (width, height, rect) in [
        (3072, 1772, [0, 0, 3072, 1774]), // Reported crash.
        (4, 4, [0, 0, 4, 4]),             // Exact fit.
        (4, 4, [0, 0, 4, 6]),             // Crop bottom, preserve sampling scale.
        (4, 4, [-2, -1, 8, 8]),           // Negative origin and zoom.
        (6, 6, [1, 1, 4, 4]),             // Black borders.
        (4, 4, [2, 1, 4, 4]),             // Clip right and bottom.
        (4, 4, [5, 0, 4, 4]),             // Entirely offscreen.
        (4, 4, [-5, 0, 4, 4]),
        (4, 4, [0, 0, 0, 4]), // Empty rectangles only clear.
        (4, 4, [0, 0, 4, -1]),
    ] {
        let bytes = rect.map(|v| (v as f32).to_ne_bytes()).concat();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        queue.submit(Some(render_frame(&device, &pipeline, &group, &view, rect)));
        let stride = (width * 4).div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(stride) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            target.size(),
        );
        queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
        device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();
        for y in 0..height {
            for x in 0..width {
                let dx = i64::from(x) - i64::from(rect[0]);
                let dy = i64::from(y) - i64::from(rect[1]);
                let expected =
                    if dx >= 0 && dy >= 0 && dx < i64::from(rect[2]) && dy < i64::from(rect[3]) {
                        let sx = ((2 * dx + 1) * 4 / (2 * i64::from(rect[2]))) as usize;
                        let sy = ((2 * dy + 1) * 4 / (2 * i64::from(rect[3]))) as usize;
                        &pixels[(sy * 4 + sx) * 4..][..4]
                    } else {
                        &[0, 0, 0, 255]
                    };
                let offset = (y * stride + x * 4) as usize;
                assert_eq!(
                    &data[offset..offset + 4],
                    expected,
                    "rect {rect:?}, target {width}x{height}, pixel ({x}, {y})"
                );
            }
        }
    }
}
