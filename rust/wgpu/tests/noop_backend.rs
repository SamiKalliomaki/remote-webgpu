//! The no-op backend: a device with no browser behind it.
//!
//! This test binary never calls `set_port`, which is the point. Asking for the
//! no-op backend must not need a listening runtime, a port, or a client.

use wgpu::*;

fn noop_instance() -> Instance {
    Instance::new(InstanceDescriptor {
        backends: Backends::NOOP,
        flags: InstanceFlags::default(),
        memory_budget_thresholds: Default::default(),
        display: None,
        backend_options: BackendOptions {
            noop: NoopBackendOptions { enable: true },
            ..Default::default()
        },
    })
}

fn noop_device() -> (Device, Queue) {
    let instance = noop_instance();
    let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions::default()))
        .expect("the no-op backend always produces an adapter");
    pollster::block_on(adapter.request_device(&DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("the no-op backend always produces a device")
}

#[test]
fn a_noop_adapter_reports_the_webgpu_baseline_limits() {
    let instance = noop_instance();
    let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions::default()))
        .expect("adapter");
    let limits = adapter.limits();

    // The library clamps every limit the hello leaves unset up to the WebGPU
    // default, so callers that size allocations from these get sane numbers
    // rather than zeroes.
    let baseline = Limits::default();
    assert_eq!(limits.max_buffer_size, baseline.max_buffer_size);
    assert_eq!(limits.max_texture_dimension_2d, baseline.max_texture_dimension_2d);
    assert_eq!(limits.max_bind_groups, baseline.max_bind_groups);
}

#[test]
fn buffers_can_be_created_written_and_dropped() {
    let (device, queue) = noop_device();

    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("noop test buffer"),
        size: 1024,
        usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    assert_eq!(buffer.size(), 1024);
    queue.write_buffer(&buffer, 0, &[7u8; 64]);

    // Dropping releases the C objects; a double free or a dangling send
    // userdata would show up here.
    drop(buffer);
    drop(queue);
    drop(device);
}

#[test]
fn every_noop_instance_gets_its_own_device() {
    let (first, _queue) = noop_device();
    let (second, _queue) = noop_device();
    let a = first.create_buffer(&BufferDescriptor {
        label: None,
        size: 256,
        usage: BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let b = second.create_buffer(&BufferDescriptor {
        label: None,
        size: 256,
        usage: BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    assert_eq!(a.size(), b.size());
}

#[test]
fn a_plain_instance_does_not_get_the_noop_backend() {
    // `enable` is off by default, so the descriptor the real server builds
    // must never silently resolve to a device that renders nothing.
    // Make sure the offline runtime already exists, so this is not just the
    // "no port configured" panic from starting one.
    let _noop = noop_instance();

    let plain = Instance::new(InstanceDescriptor::new_without_display_handle());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pollster::block_on(plain.request_adapter(&RequestAdapterOptions::default()))
    }));

    let panic = result.expect_err("a default instance must not hand out a no-op adapter");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or_default();
    assert!(
        message.contains("needs a listening runtime"),
        "expected the offline-runtime guard, got: {message}"
    );
}
