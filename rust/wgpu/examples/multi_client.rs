//! Two clients, two windows, two GPUs.
//!
//! Waits for two browser tabs to connect (each becomes its own `Window`
//! with its own `Adapter`), then animates a differently-colored screen
//! clear on each: the first client pulses red, the second pulses blue.
//!
//! Run with `cargo run -p wgpu --example multi_client`, then open the web
//! client (see ../../README.md) in two tabs.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

const CLIENTS: usize = 2;

struct View {
    frames: u64,
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    /// Base color this client's clear pulses around.
    color: wgpu::Color,
}

#[derive(Default)]
struct App {
    views: HashMap<WindowId, View>,
    start: Option<Instant>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        for index in 0..CLIENTS {
            eprintln!("multi_client: waiting for client {}/{CLIENTS} ...", index + 1);
            let window = Arc::new(
                event_loop
                    .create_window(Window::default_attributes())
                    .expect("create window"),
            );

            // Each window gets its own adapter (its client's GPU) and its
            // own device driving its own surface.
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
            let surface = instance.create_surface(window.clone()).expect("create surface");
            let adapter = pollster::block_on(instance.request_adapter(
                &wgpu::RequestAdapterOptions {
                    compatible_surface: Some(&surface),
                    ..Default::default()
                },
            ))
            .expect("request adapter");
            eprintln!(
                "multi_client: window {:?} uses adapter \"{}\"",
                window.id(),
                adapter.get_info().name
            );
            let (device, queue) = pollster::block_on(
                adapter.request_device(&wgpu::DeviceDescriptor::default()),
            )
            .expect("request device");

            let size = window.inner_size();
            let config = surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .expect("surface configuration");
            surface.configure(&device, &config);

            let color = match index {
                0 => wgpu::Color { r: 1.0, g: 0.1, b: 0.1, a: 1.0 },
                _ => wgpu::Color { r: 0.1, g: 0.1, b: 1.0, a: 1.0 },
            };
            window.request_redraw();
            self.views
                .insert(window.id(), View { frames: 0, window, device, queue, surface, color });
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(view) = self.views.get_mut(&window_id) else { return };
        match event {
            WindowEvent::Resized(size) => {
                let adapter_config = view
                    .surface
                    .get_configuration()
                    .map(|mut config| {
                        config.width = size.width.max(1);
                        config.height = size.height.max(1);
                        config
                    });
                if let Some(config) = adapter_config {
                    view.surface.configure(&view.device, &config);
                }
                view.window.request_redraw();
            }
            WindowEvent::CloseRequested => {
                self.views.remove(&window_id);
                if self.views.is_empty() {
                    event_loop.exit();
                }
            }
            WindowEvent::RedrawRequested => {
                let time = self
                    .start
                    .get_or_insert_with(Instant::now)
                    .elapsed()
                    .as_secs_f64();
                let pulse = 0.5 + 0.5 * (time * 2.0).sin();
                let color = wgpu::Color {
                    r: view.color.r * pulse,
                    g: view.color.g * pulse,
                    b: view.color.b * pulse,
                    a: 1.0,
                };

                let frame = match view.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    _ => return,
                };
                let target = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = view
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(color),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                view.queue.submit([encoder.finish()]);
                view.queue.present(frame);
                view.frames += 1;
                if view.frames % 120 == 0 {
                    eprintln!("multi_client: {window_id:?} presented {} frames", view.frames);
                }
                view.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    remote_wgpu_runtime::set_port(8080);
    let event_loop = EventLoop::new().unwrap();
    event_loop.run_app(&mut App::default()).unwrap();
    eprintln!("multi_client: all clients disconnected, exiting");
}
