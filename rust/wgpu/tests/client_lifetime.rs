//! The wgpu-side objects built on a client (surface, adapter, device) must
//! not outlive the application's handles to them, so that a disconnected
//! client's whole session can be freed.

#[path = "../../remote-wgpu-runtime/tests/support/fake_client.rs"]
mod fake_client;

use std::sync::Arc;

use fake_client::{exclusive, runtime, wait_for, FakeClient};

#[test]
fn surface_adapter_and_device_release_the_client() {
    let _serial = exclusive();
    let rt = runtime();
    let browser = FakeClient::connect(rt);
    let client = rt.try_next_client().expect("a connected client is claimable");
    let weak = Arc::downgrade(&client);

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let surface = instance.create_surface(client.clone()).unwrap();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    device.on_uncaptured_error(Arc::new(|error| panic!("uncaptured error: {error}")));
    let config = surface.get_default_config(&adapter, 64, 64).unwrap();
    surface.configure(&device, &config);
    // A second handle to the same canvas shares the surface.
    let surface_again = instance.create_surface(client.clone()).unwrap();
    assert!(adapter.is_surface_supported(&surface_again));

    browser.disconnect();
    wait_for(|| client.is_disconnected(), "the disconnect to be noticed");
    drop(client);
    assert!(weak.upgrade().is_some(), "the wgpu objects still hold the client");

    // Drop the handles in an order that exercises the C-side refcounts:
    // the adapter and device go before the surface configured with them.
    drop(adapter);
    drop(queue);
    drop(device);
    assert!(weak.upgrade().is_some(), "the surface still holds the client");
    drop(surface);
    drop(surface_again);
    wait_for(|| weak.upgrade().is_none(), "the client to be freed");

    // The surface table forgot it too: a fresh client with the same id
    // would get a fresh surface rather than a dead one.  Observable only
    // indirectly, through a new client working end to end.
    let browser = FakeClient::connect(rt);
    let client = rt.try_next_client().unwrap();
    let surface = instance.create_surface(client.clone()).unwrap();
    assert_eq!(surface.get_capabilities(&adapter_for(&surface, &instance)).formats.is_empty(), false);
    browser.disconnect();
}

fn adapter_for(surface: &wgpu::Surface<'_>, instance: &wgpu::Instance) -> wgpu::Adapter {
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(surface),
        ..Default::default()
    }))
    .unwrap()
}
