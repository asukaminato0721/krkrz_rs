use anyhow::{Context, Result};
use krkrz_assets::media::Image;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};
use winit::window::Window;

pub struct Presenter {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    rect_buffer: wgpu::Buffer,
    texture: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
}
impl Presenter {
    pub fn new(window: Arc<dyn Window>) -> Result<Self> {
        pollster::block_on(Self::create(window))
    }
    async fn create(window: Arc<dyn Window>) -> Result<Self> {
        let requested = wgpu::util::backend_bits_from_env();
        let result =
            Self::create_backend(window.clone(), requested.unwrap_or(wgpu::Backends::all())).await;
        match result {
            Err(primary) if requested.is_none() => {
                // An adapter may be available even when its window system cannot
                // present (for example Vulkan on Xvfb without DRI3).
                Self::create_backend(window, wgpu::Backends::GL)
                    .await
                    .with_context(|| {
                        format!(
                            "GPU initialization failed ({primary:#}); OpenGL fallback also failed"
                        )
                    })
            }
            result => result,
        }
    }
    async fn create_backend(window: Arc<dyn Window>, backends: wgpu::Backends) -> Result<Self> {
        let size = window.surface_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
            })
            .await
            .context("no GPU adapter can present this window")?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Kirikiri presentation"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults()
                        .using_resolution(adapter.limits()),
                },
                None,
            )
            .await?;
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("GPU surface has no supported configuration")?;
        // CPU pixels already contain the game's encoded color values. Avoid an
        // extra sRGB conversion by sampling and presenting unorm textures.
        if let Some(format) = surface
            .get_capabilities(&adapter)
            .formats
            .into_iter()
            .find(|f| !f.is_srgb())
        {
            config.format = format;
        }
        config.present_mode = wgpu::PresentMode::Fifo;
        configure_surface(&surface, &device, &config)?;
        let (layout, pipeline) = create_pipeline(&device, config.format);
        let rect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Kirikiri destination rectangle"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            layout,
            rect_buffer,
            texture: None,
        })
    }
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        self.config.width = width;
        self.config.height = height;
        configure_surface(&self.surface, &self.device, &self.config)
    }
    pub fn present(&mut self, image: &Image, rect: [i32; 4]) -> Result<()> {
        if self
            .texture
            .as_ref()
            .is_none_or(|t| (t.2, t.3) != (image.width, image.height))
        {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Kirikiri CPU frame"),
                size: wgpu::Extent3d {
                    width: image.width,
                    height: image.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: if self.config.format.is_srgb() {
                    wgpu::TextureFormat::Rgba8UnormSrgb
                } else {
                    wgpu::TextureFormat::Rgba8Unorm
                },
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.rect_buffer.as_entire_binding(),
                    },
                ],
            });
            self.texture = Some((texture, group, image.width, image.height));
        }
        let (texture, group, _, _) = self.texture.as_ref().unwrap();
        self.queue.write_texture(
            texture.as_image_copy(),
            &image.rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(image.width * 4),
                rows_per_image: Some(image.height),
            },
            texture.size(),
        );
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                return configure_surface(&self.surface, &self.device, &self.config);
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        // The game rectangle may extend beyond the acquired surface during a
        // resize, or intentionally through zoom/full-screen placement. Keep the
        // default viewport (the attachment extent) and clip in the shader so the
        // image retains its original scale and source coordinates.
        let rect_bytes = rect.map(|value| (value as f32).to_ne_bytes());
        self.queue
            .write_buffer(&self.rect_buffer, 0, rect_bytes.as_flattened());
        let view = frame.texture.create_view(&Default::default());
        self.queue.submit(Some(render_frame(
            &self.device,
            &self.pipeline,
            group,
            &view,
            rect,
        )));
        frame.present();
        Ok(())
    }
}

/// wgpu 0.20 routes surface configuration errors through its fatal panic
/// handler, not device error scopes. Keep this boundary narrow: initialization
/// can then try another backend, and resize/surface loss report a host error.
fn configure_surface(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> Result<()> {
    catch_unwind(AssertUnwindSafe(|| surface.configure(device, config))).map_err(|payload| {
        let reason = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("unknown surface configuration failure");
        anyhow::anyhow!("GPU cannot configure the native window surface: {reason}")
    })
}

fn create_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            },
        ],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("CPU frame upload"),
        source: wgpu::ShaderSource::Wgsl(include_str!("presenter.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vertex",
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fragment",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
    });
    (layout, pipeline)
}

fn render_frame(
    device: &wgpu::Device,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
    view: &wgpu::TextureView,
    rect: [i32; 4],
) -> wgpu::CommandBuffer {
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        if rect[2] > 0 && rect[3] > 0 {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
    encoder.finish()
}

#[cfg(test)]
mod tests;
