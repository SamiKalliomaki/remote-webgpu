//! Helpers for this crate's unit tests.

use alloc::sync::Arc;
use bevy_platform::future::block_on;
use wgpu::{
    BackendOptions, Backends, DeviceDescriptor, Instance, InstanceDescriptor, InstanceFlags,
    NoopBackendOptions, RequestAdapterOptions,
};

use crate::renderer::{RenderDevice, RenderQueue, WgpuWrapper};

/// Creates a dummy [`RenderDevice`] and [`RenderQueue`] on `wgpu`'s noop backend.
///
/// This lets tests exercise real `wgpu` resource creation without requiring a
/// GPU adapter, so they can run in headless environments.
///
/// remote-webgpu: `wgpu` here is the drop-in shim over `remote_webgpu`, which
/// implements the no-op backend as a client with no transport: real objects in
/// the C library, real ids and refcounts, but the envelopes go nowhere and no
/// browser ever replies. That is enough for tests that only create and destroy
/// GPU resources, which is what the callers below do. Anything that reads
/// results back, `map_async` above all, will never complete on this device.
pub fn create_dummy_device() -> (RenderDevice, RenderQueue) {
    let instance = Instance::new(InstanceDescriptor {
        backends: Backends::NOOP,
        flags: InstanceFlags::default(),
        memory_budget_thresholds: Default::default(),
        display: None,
        backend_options: BackendOptions {
            noop: NoopBackendOptions { enable: true },
            ..Default::default()
        },
    });

    let adapter = block_on(instance.request_adapter(&RequestAdapterOptions::default()))
        .expect("the noop backend should always produce an adapter");
    let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("the noop backend should always produce a device");

    (
        RenderDevice::from(device),
        RenderQueue(Arc::new(WgpuWrapper::new(queue))),
    )
}
