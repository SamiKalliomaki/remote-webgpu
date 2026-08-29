//! One player's view into the game.
//!
//! Each connected browser tab is its own `winit` window, its own wgpu
//! adapter (that tab's GPU) and its own device, so each view owns a
//! complete little renderer.  They all draw the same world from a
//! different camera.

use std::path::Path;
use std::sync::Arc;

use bevy::prelude::Vec2;
use wgpu::util::DeviceExt;
use winit::window::Window;

/// One rectangle to draw.  Positions are world units for the scene and
/// clip-space coordinates for the HUD; the camera uniform tells the shader
/// which of the two it is looking at.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Quad {
    pub center: [f32; 2],
    pub half_size: [f32; 2],
    pub fill: [f32; 4],
    /// Outline color; an alpha of zero means no outline.
    pub border: [f32; 4],
}

impl Quad {
    pub fn new(center: Vec2, half_size: Vec2, fill: [f32; 4]) -> Self {
        Self {
            center: center.into(),
            half_size: half_size.into(),
            fill,
            border: [0.0; 4],
        }
    }

    pub fn with_border(mut self, border: [f32; 4]) -> Self {
        self.border = border;
        self
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    center: [f32; 2],
    half_extent: [f32; 2],
}

/// How many quads a view can draw in one frame.
const CAPACITY: usize = 4096;
const QUAD_SIZE: u64 = std::mem::size_of::<Quad>() as u64;

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    quads: wgpu::Buffer,
    camera: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    /// A second camera whose transform is the identity, for the HUD.
    hud_bind_group: wgpu::BindGroup,
    adapter_name: String,
    /// Set by `--screenshot`: the surface is configured so frames can be
    /// copied back over the websocket.
    capture: bool,
}

impl Renderer {
    /// Claims the window's client GPU and builds the pipeline on it.
    /// With `capture`, frames can be read back with [`Renderer::capture`].
    pub fn new(window: Arc<Window>, capture: bool) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window.clone()).expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            // Pairs this window's canvas with this window's client GPU.
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("request adapter");
        let adapter_name = adapter.get_info().name;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device");

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface configuration");
        config.width = config.width.max(1);
        config.height = config.height.max(1);
        if capture {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::include_wgsl!("quad.wgsl"));

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quads"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let attributes = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quads"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: QUAD_SIZE,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let quads = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quads"),
            size: CAPACITY as u64 * QUAD_SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The HUD's camera never changes: its quads are already in clip
        // space, so the transform is the identity.
        let hud = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hud camera"),
            contents: bytemuck::bytes_of(&CameraUniform {
                center: [0.0, 0.0],
                half_extent: [1.0, 1.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let make_bind_group = |label, buffer: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            })
        };
        let camera_bind_group = make_bind_group("camera", &camera);
        let hud_bind_group = make_bind_group("hud camera", &hud);

        Self {
            device,
            queue,
            surface,
            config,
            pipeline,
            quads,
            camera,
            camera_bind_group,
            hud_bind_group,
            adapter_name,
            capture,
        }
    }

    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height as f32
    }

    /// Draws one frame: the world seen from `center` with a vertical extent
    /// of `2 * half_extent.y` units, then the HUD on top.
    pub fn render(&mut self, center: Vec2, half_extent: Vec2, scene: &[Quad], hud: &[Quad]) {
        self.render_frame(center, half_extent, scene, hud, None);
    }

    /// Renders a frame and writes it out as a binary PPM, by copying the
    /// canvas texture back over the websocket.  Browsers do not expose a
    /// WebGPU canvas to page screenshots, so this is how the example is
    /// checked; it needs a renderer built with `capture`.
    pub fn capture(
        &mut self,
        center: Vec2,
        half_extent: Vec2,
        scene: &[Quad],
        hud: &[Quad],
        path: &Path,
    ) {
        assert!(self.capture, "renderer was not built for capture");
        self.render_frame(center, half_extent, scene, hud, Some(path));
    }

    fn render_frame(
        &mut self,
        center: Vec2,
        half_extent: Vec2,
        scene: &[Quad],
        hud: &[Quad],
        capture: Option<&Path>,
    ) {
        let scene_len = scene.len().min(CAPACITY);
        let hud_len = hud.len().min(CAPACITY - scene_len);

        self.queue.write_buffer(
            &self.camera,
            0,
            bytemuck::bytes_of(&CameraUniform {
                center: center.into(),
                half_extent: half_extent.into(),
            }),
        );
        self.queue
            .write_buffer(&self.quads, 0, bytemuck::cast_slice(&scene[..scene_len]));
        self.queue.write_buffer(
            &self.quads,
            scene_len as u64 * QUAD_SIZE,
            bytemuck::cast_slice(&hud[..hud_len]),
        );

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            // The canvas is being resized; skip this frame.
            _ => return,
        };
        let target = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.055,
                            g: 0.063,
                            b: 0.090,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);

            if scene_len > 0 {
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, self.quads.slice(..scene_len as u64 * QUAD_SIZE));
                pass.draw(0..6, 0..scene_len as u32);
            }
            if hud_len > 0 {
                let start = scene_len as u64 * QUAD_SIZE;
                pass.set_bind_group(0, &self.hud_bind_group, &[]);
                pass.set_vertex_buffer(
                    0,
                    self.quads.slice(start..start + hud_len as u64 * QUAD_SIZE),
                );
                pass.draw(0..6, 0..hud_len as u32);
            }
        }

        self.queue.submit([encoder.finish()]);
        if let Some(path) = capture {
            self.read_back(&frame, path);
        }
        self.queue.present(frame);
    }

    /// Copies the just-drawn frame into a mappable buffer and writes it to
    /// `path` as a PPM.
    fn read_back(&self, frame: &wgpu::SurfaceTexture, path: &Path) {
        let (width, height) = (self.config.width, self.config.height);
        // Buffer rows are padded to the copy alignment.
        let row = width * 4;
        let padded_row = row.div_ceil(256) * 256;

        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded_row as u64 * height as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("readback") });
        encoder.copy_texture_to_buffer(
            frame.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);

        buffer.slice(..).map_async(wgpu::MapMode::Read, |result| {
            result.expect("map readback buffer");
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll for readback");

        let data = buffer.get_mapped_range(..).expect("mapped readback");
        let swap_channels = self.config.format == wgpu::TextureFormat::Bgra8Unorm
            || self.config.format == wgpu::TextureFormat::Bgra8UnormSrgb;
        let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
        for y in 0..height as usize {
            let start = y * padded_row as usize;
            for x in 0..width as usize {
                let pixel = &data[start + x * 4..start + x * 4 + 4];
                if swap_channels {
                    ppm.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
                } else {
                    ppm.extend_from_slice(&[pixel[0], pixel[1], pixel[2]]);
                }
            }
        }
        drop(data);
        buffer.unmap();

        std::fs::write(path, ppm).expect("write screenshot");
        eprintln!("bevy_game: wrote {}", path.display());
    }
}
