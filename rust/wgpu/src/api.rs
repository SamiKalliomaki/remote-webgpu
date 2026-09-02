//! The wgpu-compatible object API, implemented over the `remote_webgpu` C
//! library.  Every C call happens under the global runtime lock; completion
//! callbacks fire from the websocket reader thread.

use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut, Range, RangeBounds};
use std::os::raw::c_void;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Waker};

use remote_wgpu_runtime::{runtime, Client, HasRemoteClient, Runtime};
use remote_wgpu_sys as sys;
use sys::remote as remote_sys;

use crate::convert::*;
use crate::types::*;

fn rt() -> &'static Runtime {
    runtime()
}

/// Run a user-supplied completion callback (map_async, on_submitted_work_done)
/// on a dedicated thread instead of inline in the websocket reader.
///
/// The reader thread holds the runtime's C lock while it dispatches client
/// replies; a user callback that takes an application mutex there deadlocks
/// against any render thread holding that same mutex while it waits for the
/// C lock (seen with bevy_pbr's GPU-clustering readback).  Upstream wgpu
/// fires these callbacks from `poll()` on an application thread, never with
/// internal locks held, so applications are entitled to lock whatever they
/// like inside them.  Callbacks run in submission order.
fn dispatch_user_callback(callback: Box<dyn FnOnce() + Send>) {
    type Tx = std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>>;
    static TX: OnceLock<Tx> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
        std::thread::Builder::new()
            .name("remote-wgpu-callbacks".into())
            .spawn(move || {
                while let Ok(callback) = rx.recv() {
                    callback();
                    // Wake anything blocked in Runtime::wait/wait_until on
                    // the callback's effect (e.g. Device::poll(Wait)).
                    rt().notify();
                }
            })
            .expect("spawn remote-wgpu callback thread");
        tx
    });
    let _ = tx.send(callback);
}

// ---------------------------------------------------------------------
// completion futures
// ---------------------------------------------------------------------

struct CallbackState<T> {
    value: Option<T>,
    waker: Option<Waker>,
}

pub(crate) struct CallbackFuture<T> {
    state: Arc<Mutex<CallbackState<T>>>,
}

pub(crate) struct CallbackHandle<T> {
    state: Arc<Mutex<CallbackState<T>>>,
}

impl<T> CallbackFuture<T> {
    pub(crate) fn new() -> (Self, CallbackHandle<T>) {
        let state = Arc::new(Mutex::new(CallbackState { value: None, waker: None }));
        (Self { state: state.clone() }, CallbackHandle { state })
    }
}

impl<T> CallbackHandle<T> {
    pub(crate) fn complete(&self, value: T) {
        let mut state = self.state.lock().unwrap();
        state.value = Some(value);
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }
}

impl<T> Future for CallbackFuture<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let mut state = self.state.lock().unwrap();
        if let Some(value) = state.value.take() {
            Poll::Ready(value)
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

// ---------------------------------------------------------------------
// handle plumbing
// ---------------------------------------------------------------------

macro_rules! owned_handle {
    ($owner:ident, $raw:ty, $release:path) => {
        pub(crate) struct $owner(pub(crate) $raw);
        unsafe impl Send for $owner {}
        unsafe impl Sync for $owner {}
        impl Drop for $owner {
            fn drop(&mut self) {
                let _guard = rt().lock();
                unsafe { $release(self.0) };
            }
        }
    };
}

owned_handle!(OwnedBuffer, sys::WGPUBuffer, sys::wgpuBufferRelease);
owned_handle!(OwnedTexture, sys::WGPUTexture, sys::wgpuTextureRelease);
owned_handle!(OwnedTextureView, sys::WGPUTextureView, sys::wgpuTextureViewRelease);
owned_handle!(OwnedSampler, sys::WGPUSampler, sys::wgpuSamplerRelease);
owned_handle!(OwnedBindGroup, sys::WGPUBindGroup, sys::wgpuBindGroupRelease);
owned_handle!(OwnedBindGroupLayout, sys::WGPUBindGroupLayout, sys::wgpuBindGroupLayoutRelease);
owned_handle!(OwnedPipelineLayout, sys::WGPUPipelineLayout, sys::wgpuPipelineLayoutRelease);
owned_handle!(OwnedShaderModule, sys::WGPUShaderModule, sys::wgpuShaderModuleRelease);
owned_handle!(OwnedRenderPipeline, sys::WGPURenderPipeline, sys::wgpuRenderPipelineRelease);
owned_handle!(OwnedComputePipeline, sys::WGPUComputePipeline, sys::wgpuComputePipelineRelease);
owned_handle!(OwnedCommandBuffer, sys::WGPUCommandBuffer, sys::wgpuCommandBufferRelease);
owned_handle!(OwnedRenderBundle, sys::WGPURenderBundle, sys::wgpuRenderBundleRelease);
owned_handle!(OwnedQuerySet, sys::WGPUQuerySet, sys::wgpuQuerySetRelease);
owned_handle!(OwnedDevice, sys::WGPUDevice, sys::wgpuDeviceRelease);
owned_handle!(OwnedQueue, sys::WGPUQueue, sys::wgpuQueueRelease);
owned_handle!(OwnedSurface, sys::WGPUSurface, sys::wgpuSurfaceRelease);

// ---------------------------------------------------------------------
// Instance
// ---------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Instance {
    _priv: (),
}

impl Instance {
    /// Connects to (i.e. waits for) the remote client on first use.
    pub fn new(_desc: InstanceDescriptor) -> Self {
        let _ = rt();
        Instance { _priv: () }
    }

    pub fn enabled_backend_features() -> Backends {
        Backends::BROWSER_WEBGPU
    }

    pub fn wgsl_language_features(&self) -> WgslLanguageFeatures {
        WgslLanguageFeatures::empty()
    }

    /// One adapter per connected client; blocks until at least one client
    /// has connected.
    pub fn enumerate_adapters(&self, _backends: Backends) -> impl Future<Output = Vec<Adapter>> {
        rt().default_client();
        let adapters = rt()
            .clients()
            .into_iter()
            .filter(|client| !client.is_disconnected())
            .map(|client| Adapter { client })
            .collect();
        std::future::ready(adapters)
    }

    /// With a `compatible_surface`, resolves to the adapter of the client
    /// behind that surface's window; without one, to the first connected
    /// client (blocking until one connects).
    pub fn request_adapter(
        &self,
        options: &RequestAdapterOptions<'_, '_>,
    ) -> impl Future<Output = Result<Adapter, RequestAdapterError>> {
        let client = match options.compatible_surface {
            Some(surface) => surface.client(),
            None => rt().default_client(),
        };
        std::future::ready(Ok(Adapter { client }))
    }

    /// The "window" is a connected client's canvas: the target (the
    /// winit-compatible `Window`) names which client the surface refers to.
    /// The raw-handle escape hatch of real wgpu.  There are no real window
    /// handles in this backend: a window's client id travels in a
    /// [`raw_window_handle::WebWindowHandle`], and this decodes it again.
    ///
    /// # Safety
    ///
    /// Always safe here; the handle is only a client id, never dereferenced.
    pub unsafe fn create_surface_unsafe<'window>(
        &self,
        target: SurfaceTargetUnsafe,
    ) -> Result<Surface<'window>, CreateSurfaceError> {
        let SurfaceTargetUnsafe::RawHandle { raw_window_handle, .. } = target;
        let id = match raw_window_handle {
            raw_window_handle::RawWindowHandle::Web(handle) => handle.id as u64,
            _ => {
                return Err(CreateSurfaceError {
                    message: "remote surfaces are identified by WebWindowHandle client ids"
                        .to_string(),
                })
            }
        };
        let client = runtime()
            .clients()
            .into_iter()
            .find(|client| client.id() == id)
            .ok_or_else(|| CreateSurfaceError {
                message: format!("no connected client with id {id}"),
            })?;
        self.create_surface(client)
    }

    pub fn create_surface<'window, T: HasRemoteClient + 'window>(
        &self,
        target: T,
    ) -> Result<Surface<'window>, CreateSurfaceError> {
        Ok(Surface {
            inner: surface_shared(target.remote_client()),
            _marker: PhantomData,
        })
    }

    /// wgpu-core reports are not available for the remote backend.
    pub fn generate_report(&self) -> Option<()> {
        None
    }

    pub fn poll_all(&self, _force_wait: bool) -> bool {
        false
    }
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub struct WgslLanguageFeatures: u32 {
        const READONLY_AND_READWRITE_STORAGE_TEXTURES = 1 << 0;
        const PACKED4X8_INTEGER_DOT_PRODUCT = 1 << 1;
        const UNRESTRICTED_POINTER_PARAMETERS = 1 << 2;
        const POINTER_COMPOSITE_ACCESS = 1 << 3;
    }
}

pub(crate) struct SurfaceShared {
    client: Arc<Client>,
    raw: OwnedSurface,
    config: Mutex<Option<SurfaceConfiguration>>,
    device: Mutex<Option<Device>>,
}

/// One canvas per client, so one surface per client, shared by however
/// many `Surface` handles are created for it.
static SURFACES: OnceLock<Mutex<std::collections::HashMap<u64, Arc<SurfaceShared>>>> =
    OnceLock::new();

fn surface_shared(client: Arc<Client>) -> Arc<SurfaceShared> {
    let mut registry = SURFACES
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap();
    registry
        .entry(client.id())
        .or_insert_with(|| {
            let _guard = rt().lock();
            let mut desc: sys::WGPUSurfaceDescriptor = unsafe { std::mem::zeroed() };
            desc.label = sv(Some("remote canvas"));
            let raw = unsafe { sys::wgpuInstanceCreateSurface(client.instance(), &desc) };
            assert!(!raw.is_null(), "failed to create the remote surface");
            Arc::new(SurfaceShared {
                client,
                raw: OwnedSurface(raw),
                config: Mutex::new(None),
                device: Mutex::new(None),
            })
        })
        .clone()
}

// ---------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------

/// One adapter per connected client.
#[derive(Clone)]
pub struct Adapter {
    client: Arc<Client>,
}

impl std::fmt::Debug for Adapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Adapter").field("client", &self.client.id()).finish()
    }
}

impl Adapter {
    fn raw(&self) -> sys::WGPUAdapter {
        self.client.adapter()
    }

    pub fn features(&self) -> Features {
        let _guard = rt().lock();
        let mut supported: sys::WGPUSupportedFeatures = unsafe { std::mem::zeroed() };
        unsafe { sys::wgpuAdapterGetFeatures(self.raw(), &mut supported) };
        let names = if supported.features.is_null() || supported.featureCount == 0 {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(supported.features, supported.featureCount) }
        };
        let features = unmap_features(names);
        unsafe { sys::wgpuSupportedFeaturesFreeMembers(supported) };
        features
    }

    pub fn limits(&self) -> Limits {
        let _guard = rt().lock();
        let mut limits: sys::WGPULimits = unsafe { std::mem::zeroed() };
        unsafe { sys::wgpuAdapterGetLimits(self.raw(), &mut limits) };
        unmap_limits(&limits)
    }

    pub fn get_info(&self) -> AdapterInfo {
        let _guard = rt().lock();
        let mut info: sys::WGPUAdapterInfo = unsafe { std::mem::zeroed() };
        unsafe { sys::wgpuAdapterGetInfo(self.raw(), &mut info) };
        let mut name_parts = Vec::new();
        for part in [info.vendor, info.architecture, info.device, info.description] {
            let s = from_sv(part);
            if !s.is_empty() {
                name_parts.push(s);
            }
        }
        let result = AdapterInfo {
            name: if name_parts.is_empty() {
                "Remote WebGPU adapter".to_string()
            } else {
                name_parts.join(" / ")
            },
            vendor: info.vendorID,
            device: info.deviceID,
            device_type: DeviceType::Other,
            device_pci_bus_id: String::new(),
            subgroup_min_size: 4,
            subgroup_max_size: 128,
            transient_saves_memory: false,
            driver: "remote-webgpu".to_string(),
            driver_info: String::new(),
            backend: Backend::BrowserWebGpu,
        };
        unsafe { sys::wgpuAdapterInfoFreeMembers(info) };
        result
    }

    pub fn get_downlevel_capabilities(&self) -> DownlevelCapabilities {
        DownlevelCapabilities::default()
    }

    /// A surface is only usable with the adapter of the client whose
    /// canvas it is.
    pub fn is_surface_supported(&self, surface: &Surface<'_>) -> bool {
        Arc::ptr_eq(&self.client, &surface.inner.client)
    }

    pub fn get_texture_format_features(&self, format: TextureFormat) -> TextureFormatFeatures {
        // The browser supports sample counts 1 and 4 for all renderable
        // formats; report that, plus filtering, and let validation on the
        // client catch anything fancier.
        let _ = format;
        TextureFormatFeatures {
            allowed_usages: TextureUsages::all(),
            flags: TextureFormatFeatureFlags::FILTERABLE
                | TextureFormatFeatureFlags::MULTISAMPLE_X4
                | TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE
                | TextureFormatFeatureFlags::BLENDABLE,
        }
    }

    pub fn request_device(
        &self,
        desc: &DeviceDescriptor<'_>,
    ) -> impl Future<Output = Result<(Device, Queue), RequestDeviceError>> {
        let result = self.request_device_sync(desc);
        std::future::ready(result)
    }

    fn request_device_sync(
        &self,
        desc: &DeviceDescriptor<'_>,
    ) -> Result<(Device, Queue), RequestDeviceError> {
        let _guard = rt().lock();

        let mut features = map_features(desc.required_features);
        // WebGPU features newer than the wgpu-types 29 `Features` set cannot
        // be named by the caller at all, but gate real validation rules in
        // the browser (bevy's SSAO needs texture-formats-tier1 for R16Float
        // storage textures, for example).  Implicitly request every one the
        // adapter supports; they are purely additive.
        const UNNAMEABLE_FEATURES: &[sys::WGPUFeatureName] = &[
            sys::WGPUFeatureName_Subgroups,
            sys::WGPUFeatureName_TextureFormatsTier1,
            sys::WGPUFeatureName_TextureFormatsTier2,
            sys::WGPUFeatureName_PrimitiveIndex,
            sys::WGPUFeatureName_TextureComponentSwizzle,
        ];
        for &feature in UNNAMEABLE_FEATURES {
            if unsafe { sys::wgpuAdapterHasFeature(self.raw(), feature) } != 0 {
                features.push(feature);
            }
        }
        let limits = map_limits(&desc.required_limits);

        let mut raw_desc: sys::WGPUDeviceDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.requiredFeatureCount = features.len();
        raw_desc.requiredFeatures = features.as_ptr();
        raw_desc.requiredLimits = &limits;
        raw_desc.deviceLostCallbackInfo.mode = sys::WGPUCallbackMode_AllowProcessEvents;
        raw_desc.deviceLostCallbackInfo.callback = Some(device_lost_cb);
        raw_desc.uncapturedErrorCallbackInfo.callback = Some(uncaptured_error_cb);

        struct RequestState {
            device: sys::WGPUDevice,
            message: String,
            done: bool,
        }
        let mut state = RequestState { device: std::ptr::null_mut(), message: String::new(), done: false };

        unsafe extern "C" fn on_device(
            status: sys::WGPURequestDeviceStatus,
            device: sys::WGPUDevice,
            message: sys::WGPUStringView,
            userdata1: *mut c_void,
            _userdata2: *mut c_void,
        ) {
            let state = unsafe { &mut *(userdata1 as *mut RequestState) };
            if status == sys::WGPURequestDeviceStatus_Success {
                state.device = device;
            } else {
                state.message = from_sv(message);
            }
            state.done = true;
        }

        let mut cb: sys::WGPURequestDeviceCallbackInfo = unsafe { std::mem::zeroed() };
        cb.mode = sys::WGPUCallbackMode_AllowProcessEvents;
        cb.callback = Some(on_device);
        cb.userdata1 = &mut state as *mut RequestState as *mut c_void;
        unsafe { sys::wgpuAdapterRequestDevice(self.raw(), &raw_desc, cb) };
        // The remote implementation resolves the request synchronously.
        assert!(state.done, "wgpuAdapterRequestDevice did not resolve synchronously");

        if state.device.is_null() {
            return Err(RequestDeviceError { message: state.message });
        }
        let queue_raw = unsafe { sys::wgpuDeviceGetQueue(state.device) };
        // A freed device's address can come back for a new one; make sure
        // the fresh device does not inherit the old one's error handler.
        if let Some(handlers) = uncaptured_error_handlers().as_mut() {
            handlers.remove(&(state.device as usize));
        }
        let device = Device {
            inner: Arc::new(OwnedDevice(state.device)),
            client: self.client.clone(),
        };
        let queue = Queue { inner: Arc::new(OwnedQueue(queue_raw)) };
        Ok((device, queue))
    }
}

unsafe extern "C" fn device_lost_cb(
    _device: *const sys::WGPUDevice,
    _reason: sys::WGPUDeviceLostReason,
    message: sys::WGPUStringView,
    _userdata1: *mut c_void,
    _userdata2: *mut c_void,
) {
    log::error!("remote-wgpu: device lost: {}", from_sv(message));
}

pub trait UncapturedErrorHandler: Fn(Error) + Send + Sync + 'static {}
impl<T> UncapturedErrorHandler for T where T: Fn(Error) + Send + Sync + 'static {}

type ErrorHandler = Arc<dyn UncapturedErrorHandler>;

/// Uncaptured-error handlers, keyed by the raw device they belong to.
///
/// Errors are reported by the client that owns the device, and every client
/// is a separate, untrusted peer: a single global handler would let one
/// client's error (or an `UncapturedError` message it simply made up) be
/// delivered to another client's device, taking down a render world that
/// has nothing to do with it.
static UNCAPTURED_ERROR_HANDLERS: Mutex<Option<HashMap<usize, ErrorHandler>>> = Mutex::new(None);

fn uncaptured_error_handlers() -> std::sync::MutexGuard<'static, Option<HashMap<usize, ErrorHandler>>>
{
    let mut guard = UNCAPTURED_ERROR_HANDLERS.lock().unwrap();
    guard.get_or_insert_with(HashMap::new);
    guard
}

fn make_error(ty: sys::WGPUErrorType, message: String) -> Error {
    let source: ErrorSource = Box::new(std::io::Error::other(message.clone()));
    match ty {
        x if x == sys::WGPUErrorType_OutOfMemory => Error::OutOfMemory { source },
        x if x == sys::WGPUErrorType_Internal => Error::Internal { source, description: message },
        _ => Error::Validation { source, description: message },
    }
}

unsafe extern "C" fn uncaptured_error_cb(
    device: *const sys::WGPUDevice,
    ty: sys::WGPUErrorType,
    message: sys::WGPUStringView,
    _userdata1: *mut c_void,
    _userdata2: *mut c_void,
) {
    let error = make_error(ty, from_sv(message));
    // `device` points at the WGPUDevice the C library reported the error
    // for; a null pointer means it could not attribute it to a device.
    let raw = if device.is_null() { std::ptr::null_mut() } else { unsafe { *device } };
    let handler = uncaptured_error_handlers()
        .as_ref()
        .and_then(|handlers| handlers.get(&(raw as usize)).cloned());
    match handler {
        // The handler is application code; keep it off the reader thread
        // (which holds the C lock here) like the other user callbacks.
        Some(handler) => dispatch_user_callback(Box::new(move || handler(error))),
        None => log::error!("remote-wgpu: uncaptured error: {error}"),
    }
}

// ---------------------------------------------------------------------
// Device
// ---------------------------------------------------------------------

#[derive(Clone)]
pub struct Device {
    inner: Arc<OwnedDevice>,
    client: Arc<Client>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device").finish()
    }
}

impl Device {
    pub(crate) fn raw(&self) -> sys::WGPUDevice {
        self.inner.0
    }

    pub fn features(&self) -> Features {
        let _guard = rt().lock();
        let mut supported: sys::WGPUSupportedFeatures = unsafe { std::mem::zeroed() };
        unsafe { sys::wgpuDeviceGetFeatures(self.raw(), &mut supported) };
        let names = if supported.features.is_null() || supported.featureCount == 0 {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(supported.features, supported.featureCount) }
        };
        let features = unmap_features(names);
        unsafe { sys::wgpuSupportedFeaturesFreeMembers(supported) };
        features
    }

    pub fn limits(&self) -> Limits {
        let _guard = rt().lock();
        let mut limits: sys::WGPULimits = unsafe { std::mem::zeroed() };
        unsafe { sys::wgpuDeviceGetLimits(self.raw(), &mut limits) };
        unmap_limits(&limits)
    }

    pub fn adapter_info(&self) -> AdapterInfo {
        Adapter { client: self.client.clone() }.get_info()
    }

    pub fn poll(&self, poll_type: PollType) -> Result<PollStatus, PollError> {
        match poll_type {
            PollType::Poll => Ok(PollStatus::Poll),
            PollType::Wait { timeout, .. } => {
                let (future, handle) = CallbackFuture::<()>::new();
                let queue_raw = {
                    let _guard = rt().lock();
                    unsafe { sys::wgpuDeviceGetQueue(self.raw()) }
                };
                let queue = Queue { inner: Arc::new(OwnedQueue(queue_raw)) };
                queue.on_submitted_work_done_impl(Box::new(move || handle.complete(())));
                drop(queue);
                let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
                {
                    let done = done.clone();
                    let mut future = Box::pin(future);
                    let start = std::time::Instant::now();
                    rt().wait_until(move || {
                        if let Some(limit) = timeout {
                            if start.elapsed() > limit {
                                return true;
                            }
                        }
                        let waker = Waker::noop();
                        let mut cx = Context::from_waker(waker);
                        if future.as_mut().poll(&mut cx).is_ready() {
                            done.store(true, std::sync::atomic::Ordering::SeqCst);
                            true
                        } else {
                            false
                        }
                    });
                }
                if done.load(std::sync::atomic::Ordering::SeqCst) {
                    Ok(PollStatus::WaitSucceeded)
                } else {
                    Err(PollError::Timeout)
                }
            }
        }
    }

    pub fn create_shader_module(&self, desc: ShaderModuleDescriptor<'_>) -> ShaderModule {
        let _guard = rt().lock();
        let code: std::borrow::Cow<'_, str> = match &desc.source {
            ShaderSource::Wgsl(code) => std::borrow::Cow::Borrowed(code.as_ref()),
            ShaderSource::Naga(module) => std::borrow::Cow::Owned(naga_to_wgsl(module)),
            _ => panic!("only WGSL and Naga IR shader sources reach the remote backend"),
        };
        let code = code.as_ref();
        let mut wgsl: sys::WGPUShaderSourceWGSL = unsafe { std::mem::zeroed() };
        wgsl.chain.sType = sys::WGPUSType_ShaderSourceWGSL;
        wgsl.code = sv_str(code);
        let mut raw_desc: sys::WGPUShaderModuleDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.nextInChain = &mut wgsl.chain;
        raw_desc.label = sv(desc.label);
        let raw = unsafe { sys::wgpuDeviceCreateShaderModule(self.raw(), &raw_desc) };
        ShaderModule { inner: Arc::new(OwnedShaderModule(raw)) }
    }

    /// The remote backend always ships shaders to a validating browser, so
    /// runtime checks cannot actually be skipped; this is create_shader_module.
    ///
    /// # Safety
    ///
    /// Always safe here; the browser validates every shader regardless.
    pub unsafe fn create_shader_module_trusted(
        &self,
        desc: ShaderModuleDescriptor<'_>,
        _runtime_checks: ShaderRuntimeChecks,
    ) -> ShaderModule {
        self.create_shader_module(desc)
    }

    pub fn create_command_encoder(&self, desc: &CommandEncoderDescriptor<'_>) -> CommandEncoder {
        let _guard = rt().lock();
        let mut raw_desc: sys::WGPUCommandEncoderDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        let raw = unsafe { sys::wgpuDeviceCreateCommandEncoder(self.raw(), &raw_desc) };
        CommandEncoder { raw, finished: false, mappings: Vec::new() }
    }

    pub fn create_buffer(&self, desc: &BufferDescriptor<'_>) -> Buffer {
        let _guard = rt().lock();
        let mut raw_desc: sys::WGPUBufferDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.usage = map_buffer_usages(desc.usage);
        raw_desc.size = desc.size;
        raw_desc.mappedAtCreation = bool32(desc.mapped_at_creation);
        let raw = unsafe { sys::wgpuDeviceCreateBuffer(self.raw(), &raw_desc) };
        Buffer {
            inner: Arc::new(OwnedBuffer(raw)),
            size: desc.size,
            usage: desc.usage,
        }
    }

    pub fn create_texture(&self, desc: &TextureDescriptor<'_>) -> Texture {
        let _guard = rt().lock();
        let view_formats: Vec<_> =
            desc.view_formats.iter().map(|&f| map_texture_format(f)).collect();
        let mut raw_desc: sys::WGPUTextureDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.usage = map_texture_usages(desc.usage);
        raw_desc.dimension = map_texture_dimension(desc.dimension);
        raw_desc.size = map_extent(desc.size);
        raw_desc.format = map_texture_format(desc.format);
        raw_desc.mipLevelCount = desc.mip_level_count;
        raw_desc.sampleCount = desc.sample_count;
        raw_desc.viewFormatCount = view_formats.len();
        raw_desc.viewFormats = view_formats.as_ptr();
        let raw = unsafe { sys::wgpuDeviceCreateTexture(self.raw(), &raw_desc) };
        Texture {
            inner: Arc::new(OwnedTexture(raw)),
            size: desc.size,
            format: desc.format,
            dimension: desc.dimension,
            usage: desc.usage,
            mip_level_count: desc.mip_level_count,
            sample_count: desc.sample_count,
        }
    }

    pub fn create_sampler(&self, desc: &SamplerDescriptor<'_>) -> Sampler {
        let _guard = rt().lock();
        let mut raw_desc: sys::WGPUSamplerDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.addressModeU = map_address_mode(desc.address_mode_u);
        raw_desc.addressModeV = map_address_mode(desc.address_mode_v);
        raw_desc.addressModeW = map_address_mode(desc.address_mode_w);
        raw_desc.magFilter = map_filter_mode(desc.mag_filter);
        raw_desc.minFilter = map_filter_mode(desc.min_filter);
        raw_desc.mipmapFilter = map_mipmap_filter_mode(desc.mipmap_filter);
        raw_desc.lodMinClamp = desc.lod_min_clamp;
        raw_desc.lodMaxClamp = desc.lod_max_clamp;
        raw_desc.compare = match desc.compare {
            Some(func) => map_compare_function(func),
            None => sys::WGPUCompareFunction_Undefined,
        };
        raw_desc.maxAnisotropy = desc.anisotropy_clamp;
        let raw = unsafe { sys::wgpuDeviceCreateSampler(self.raw(), &raw_desc) };
        Sampler { inner: Arc::new(OwnedSampler(raw)) }
    }

    pub fn create_bind_group_layout(
        &self,
        desc: &BindGroupLayoutDescriptor<'_>,
    ) -> BindGroupLayout {
        let _guard = rt().lock();
        let entries: Vec<sys::WGPUBindGroupLayoutEntry> = desc
            .entries
            .iter()
            .map(|entry| {
                let mut raw: sys::WGPUBindGroupLayoutEntry = unsafe { std::mem::zeroed() };
                raw.binding = entry.binding;
                raw.visibility = map_shader_stages(entry.visibility);
                if let Some(count) = entry.count {
                    raw.bindingArraySize = count.get();
                }
                raw.buffer.r#type = sys::WGPUBufferBindingType_BindingNotUsed;
                raw.sampler.r#type = sys::WGPUSamplerBindingType_BindingNotUsed;
                raw.texture.sampleType = sys::WGPUTextureSampleType_BindingNotUsed;
                raw.storageTexture.access = sys::WGPUStorageTextureAccess_BindingNotUsed;
                match entry.ty {
                    BindingType::Buffer { ty, has_dynamic_offset, min_binding_size } => {
                        raw.buffer.r#type = match ty {
                            BufferBindingType::Uniform => sys::WGPUBufferBindingType_Uniform,
                            BufferBindingType::Storage { read_only: true } => {
                                sys::WGPUBufferBindingType_ReadOnlyStorage
                            }
                            BufferBindingType::Storage { read_only: false } => {
                                sys::WGPUBufferBindingType_Storage
                            }
                        };
                        raw.buffer.hasDynamicOffset = bool32(has_dynamic_offset);
                        raw.buffer.minBindingSize =
                            min_binding_size.map(|s| s.get()).unwrap_or(0);
                    }
                    BindingType::Sampler(ty) => {
                        raw.sampler.r#type = match ty {
                            SamplerBindingType::Filtering => {
                                sys::WGPUSamplerBindingType_Filtering
                            }
                            SamplerBindingType::NonFiltering => {
                                sys::WGPUSamplerBindingType_NonFiltering
                            }
                            SamplerBindingType::Comparison => {
                                sys::WGPUSamplerBindingType_Comparison
                            }
                        };
                    }
                    BindingType::Texture { sample_type, view_dimension, multisampled } => {
                        raw.texture.sampleType = match sample_type {
                            TextureSampleType::Float { filterable: true } => {
                                sys::WGPUTextureSampleType_Float
                            }
                            TextureSampleType::Float { filterable: false } => {
                                sys::WGPUTextureSampleType_UnfilterableFloat
                            }
                            TextureSampleType::Depth => sys::WGPUTextureSampleType_Depth,
                            TextureSampleType::Sint => sys::WGPUTextureSampleType_Sint,
                            TextureSampleType::Uint => sys::WGPUTextureSampleType_Uint,
                        };
                        raw.texture.viewDimension = map_texture_view_dimension(view_dimension);
                        raw.texture.multisampled = bool32(multisampled);
                    }
                    BindingType::StorageTexture { access, format, view_dimension } => {
                        raw.storageTexture.access = match access {
                            StorageTextureAccess::WriteOnly => {
                                sys::WGPUStorageTextureAccess_WriteOnly
                            }
                            StorageTextureAccess::ReadOnly => {
                                sys::WGPUStorageTextureAccess_ReadOnly
                            }
                            StorageTextureAccess::ReadWrite => {
                                sys::WGPUStorageTextureAccess_ReadWrite
                            }
                            StorageTextureAccess::Atomic => {
                                panic!("atomic storage textures are not available in WebGPU")
                            }
                        };
                        raw.storageTexture.format = map_texture_format(format);
                        raw.storageTexture.viewDimension =
                            map_texture_view_dimension(view_dimension);
                    }
                    BindingType::AccelerationStructure { .. } | BindingType::ExternalTexture => {
                        panic!("this binding type is not available over remote WebGPU")
                    }
                }
                raw
            })
            .collect();
        let mut raw_desc: sys::WGPUBindGroupLayoutDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.entryCount = entries.len();
        raw_desc.entries = entries.as_ptr();
        let raw = unsafe { sys::wgpuDeviceCreateBindGroupLayout(self.raw(), &raw_desc) };
        BindGroupLayout { inner: Arc::new(OwnedBindGroupLayout(raw)) }
    }

    pub fn create_bind_group(&self, desc: &BindGroupDescriptor<'_>) -> BindGroup {
        let _guard = rt().lock();
        let entries: Vec<sys::WGPUBindGroupEntry> = desc
            .entries
            .iter()
            .map(|entry| {
                let mut raw: sys::WGPUBindGroupEntry = unsafe { std::mem::zeroed() };
                raw.binding = entry.binding;
                match &entry.resource {
                    BindingResource::Buffer(binding) => {
                        raw.buffer = binding.buffer.raw();
                        raw.offset = binding.offset;
                        raw.size = binding
                            .size
                            .map(|s| s.get())
                            .unwrap_or(sys::WGPU_WHOLE_SIZE);
                    }
                    BindingResource::Sampler(sampler) => {
                        raw.sampler = sampler.inner.0;
                    }
                    BindingResource::TextureView(view) => {
                        raw.textureView = view.inner.0;
                    }
                    BindingResource::BufferArray(_)
                    | BindingResource::SamplerArray(_)
                    | BindingResource::TextureViewArray(_) => {
                        panic!("binding arrays are not available over remote WebGPU")
                    }
                    _ => panic!("unsupported binding resource over remote WebGPU"),
                }
                raw
            })
            .collect();
        let mut raw_desc: sys::WGPUBindGroupDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.layout = desc.layout.inner.0;
        raw_desc.entryCount = entries.len();
        raw_desc.entries = entries.as_ptr();
        let raw = unsafe { sys::wgpuDeviceCreateBindGroup(self.raw(), &raw_desc) };
        let keepalive = desc
            .entries
            .iter()
            .filter_map(|entry| match &entry.resource {
                BindingResource::Buffer(binding) => Some(binding.buffer.clone()),
                _ => None,
            })
            .collect();
        BindGroup { inner: Arc::new(OwnedBindGroup(raw)), _keepalive: Arc::new(keepalive) }
    }

    pub fn create_pipeline_layout(&self, desc: &PipelineLayoutDescriptor<'_>) -> PipelineLayout {
        let _guard = rt().lock();
        let layouts: Vec<sys::WGPUBindGroupLayout> = desc
            .bind_group_layouts
            .iter()
            .map(|l| l.map(|l| l.inner.0).unwrap_or(std::ptr::null_mut()))
            .collect();
        let mut raw_desc: sys::WGPUPipelineLayoutDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.bindGroupLayoutCount = layouts.len();
        raw_desc.bindGroupLayouts = layouts.as_ptr();
        let raw = unsafe { sys::wgpuDeviceCreatePipelineLayout(self.raw(), &raw_desc) };
        PipelineLayout { inner: Arc::new(OwnedPipelineLayout(raw)) }
    }

    pub fn create_render_pipeline(&self, desc: &RenderPipelineDescriptor<'_>) -> RenderPipeline {
        let _guard = rt().lock();

        // Keep every temporary alive until the call returns.
        let vertex_constants: Vec<sys::WGPUConstantEntry> =
            map_constants(desc.vertex.compilation_options.constants);
        let vertex_attributes: Vec<Vec<sys::WGPUVertexAttribute>> = desc
            .vertex
            .buffers
            .iter()
            .map(|buffer| {
                buffer
                    .attributes
                    .iter()
                    .map(|attr| {
                        let mut raw: sys::WGPUVertexAttribute = unsafe { std::mem::zeroed() };
                        raw.format = map_vertex_format(attr.format);
                        raw.offset = attr.offset;
                        raw.shaderLocation = attr.shader_location;
                        raw
                    })
                    .collect()
            })
            .collect();
        let vertex_buffers: Vec<sys::WGPUVertexBufferLayout> = desc
            .vertex
            .buffers
            .iter()
            .zip(&vertex_attributes)
            .map(|(buffer, attrs)| {
                let mut raw: sys::WGPUVertexBufferLayout = unsafe { std::mem::zeroed() };
                raw.stepMode = map_vertex_step_mode(buffer.step_mode);
                raw.arrayStride = buffer.array_stride;
                raw.attributeCount = attrs.len();
                raw.attributes = attrs.as_ptr();
                raw
            })
            .collect();

        let mut raw_desc: sys::WGPURenderPipelineDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.layout = desc.layout.map(|l| l.inner.0).unwrap_or(std::ptr::null_mut());

        raw_desc.vertex.module = desc.vertex.module.inner.0;
        raw_desc.vertex.entryPoint = sv(desc.vertex.entry_point);
        raw_desc.vertex.constantCount = vertex_constants.len();
        raw_desc.vertex.constants = vertex_constants.as_ptr();
        raw_desc.vertex.bufferCount = vertex_buffers.len();
        raw_desc.vertex.buffers = vertex_buffers.as_ptr();

        raw_desc.primitive.topology = map_primitive_topology(desc.primitive.topology);
        raw_desc.primitive.stripIndexFormat = match desc.primitive.strip_index_format {
            Some(format) => map_index_format(format),
            None => sys::WGPUIndexFormat_Undefined,
        };
        raw_desc.primitive.frontFace = map_front_face(desc.primitive.front_face);
        raw_desc.primitive.cullMode = map_cull_mode(desc.primitive.cull_mode);
        raw_desc.primitive.unclippedDepth = bool32(desc.primitive.unclipped_depth);

        let raw_depth_stencil = desc.depth_stencil.as_ref().map(|ds| {
            let mut raw: sys::WGPUDepthStencilState = unsafe { std::mem::zeroed() };
            raw.format = map_texture_format(ds.format);
            raw.depthWriteEnabled = match ds.depth_write_enabled {
                Some(true) => sys::WGPUOptionalBool_True,
                Some(false) => sys::WGPUOptionalBool_False,
                None => sys::WGPUOptionalBool_Undefined,
            };
            raw.depthCompare = ds
                .depth_compare
                .map(map_compare_function)
                .unwrap_or(sys::WGPUCompareFunction_Undefined);
            raw.stencilFront = map_stencil_face_state(ds.stencil.front);
            raw.stencilBack = map_stencil_face_state(ds.stencil.back);
            raw.stencilReadMask = ds.stencil.read_mask;
            raw.stencilWriteMask = ds.stencil.write_mask;
            raw.depthBias = ds.bias.constant;
            raw.depthBiasSlopeScale = ds.bias.slope_scale;
            raw.depthBiasClamp = ds.bias.clamp;
            raw
        });
        raw_desc.depthStencil = raw_depth_stencil
            .as_ref()
            .map(|ds| ds as *const _)
            .unwrap_or(std::ptr::null());

        raw_desc.multisample.count = desc.multisample.count;
        raw_desc.multisample.mask = desc.multisample.mask as u32;
        raw_desc.multisample.alphaToCoverageEnabled =
            bool32(desc.multisample.alpha_to_coverage_enabled);

        let fragment_constants: Vec<sys::WGPUConstantEntry> = desc
            .fragment
            .as_ref()
            .map(|f| map_constants(f.compilation_options.constants))
            .unwrap_or_default();
        let blends: Vec<Option<sys::WGPUBlendState>> = desc
            .fragment
            .as_ref()
            .map(|f| {
                f.targets
                    .iter()
                    .map(|t| t.as_ref().and_then(|t| t.blend).map(map_blend_state))
                    .collect()
            })
            .unwrap_or_default();
        let targets: Vec<sys::WGPUColorTargetState> = desc
            .fragment
            .as_ref()
            .map(|f| {
                f.targets
                    .iter()
                    .zip(&blends)
                    .map(|(target, blend)| {
                        let mut raw: sys::WGPUColorTargetState = unsafe { std::mem::zeroed() };
                        match target {
                            Some(target) => {
                                raw.format = map_texture_format(target.format);
                                raw.blend = blend
                                    .as_ref()
                                    .map(|b| b as *const _)
                                    .unwrap_or(std::ptr::null());
                                raw.writeMask = map_color_writes(target.write_mask);
                            }
                            None => {
                                raw.format = sys::WGPUTextureFormat_Undefined;
                                raw.writeMask = 0;
                            }
                        }
                        raw
                    })
                    .collect()
            })
            .unwrap_or_default();
        let raw_fragment = desc.fragment.as_ref().map(|f| {
            let mut raw: sys::WGPUFragmentState = unsafe { std::mem::zeroed() };
            raw.module = f.module.inner.0;
            raw.entryPoint = sv(f.entry_point);
            raw.constantCount = fragment_constants.len();
            raw.constants = fragment_constants.as_ptr();
            raw.targetCount = targets.len();
            raw.targets = targets.as_ptr();
            raw
        });
        raw_desc.fragment = raw_fragment
            .as_ref()
            .map(|f| f as *const _)
            .unwrap_or(std::ptr::null());

        let raw = unsafe { sys::wgpuDeviceCreateRenderPipeline(self.raw(), &raw_desc) };
        RenderPipeline { inner: Arc::new(OwnedRenderPipeline(raw)) }
    }

    pub fn create_compute_pipeline(&self, desc: &ComputePipelineDescriptor<'_>) -> ComputePipeline {
        let _guard = rt().lock();
        let constants = map_constants(desc.compilation_options.constants);
        let mut raw_desc: sys::WGPUComputePipelineDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.layout = desc.layout.map(|l| l.inner.0).unwrap_or(std::ptr::null_mut());
        raw_desc.compute.module = desc.module.inner.0;
        raw_desc.compute.entryPoint = sv(desc.entry_point);
        raw_desc.compute.constantCount = constants.len();
        raw_desc.compute.constants = constants.as_ptr();
        let raw = unsafe { sys::wgpuDeviceCreateComputePipeline(self.raw(), &raw_desc) };
        ComputePipeline { inner: Arc::new(OwnedComputePipeline(raw)) }
    }

    pub fn create_query_set(&self, desc: &QuerySetDescriptor<'_>) -> QuerySet {
        let _guard = rt().lock();
        let mut raw_desc: sys::WGPUQuerySetDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.r#type = match desc.ty {
            QueryType::Occlusion => sys::WGPUQueryType_Occlusion,
            QueryType::Timestamp => sys::WGPUQueryType_Timestamp,
            QueryType::PipelineStatistics(_) => {
                panic!("pipeline statistics queries are not available in WebGPU")
            }
        };
        raw_desc.count = desc.count;
        let raw = unsafe { sys::wgpuDeviceCreateQuerySet(self.raw(), &raw_desc) };
        QuerySet { inner: Arc::new(OwnedQuerySet(raw)) }
    }

    pub fn create_render_bundle_encoder<'a>(
        &self,
        desc: &RenderBundleEncoderDescriptor<'_>,
    ) -> RenderBundleEncoder<'a> {
        let _guard = rt().lock();
        let formats: Vec<sys::WGPUTextureFormat> = desc
            .color_formats
            .iter()
            .map(|f| {
                f.map(map_texture_format)
                    .unwrap_or(sys::WGPUTextureFormat_Undefined)
            })
            .collect();
        let mut raw_desc: sys::WGPURenderBundleEncoderDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.colorFormatCount = formats.len();
        raw_desc.colorFormats = formats.as_ptr();
        raw_desc.depthStencilFormat = desc
            .depth_stencil
            .map(|ds| map_texture_format(ds.format))
            .unwrap_or(sys::WGPUTextureFormat_Undefined);
        raw_desc.sampleCount = desc.sample_count.max(1);
        if let Some(ds) = desc.depth_stencil {
            raw_desc.depthReadOnly = bool32(ds.depth_read_only);
            raw_desc.stencilReadOnly = bool32(ds.stencil_read_only);
        }
        let raw = unsafe { sys::wgpuDeviceCreateRenderBundleEncoder(self.raw(), &raw_desc) };
        RenderBundleEncoder { raw, _marker: PhantomData }
    }

    pub fn push_error_scope(&self, filter: ErrorFilter) -> ErrorScopeGuard {
        let _guard = rt().lock();
        unsafe { sys::wgpuDevicePushErrorScope(self.raw(), map_error_filter(filter)) };
        ErrorScopeGuard { device: self.clone() }
    }

    pub fn on_uncaptured_error(&self, handler: Arc<dyn UncapturedErrorHandler>) {
        if let Some(handlers) = uncaptured_error_handlers().as_mut() {
            handlers.insert(self.raw() as usize, handler);
        }
    }

    pub fn set_device_lost_callback(&self, _callback: impl Fn(DeviceLostReason, String) + Send + 'static) {
        // The runtime logs device loss; per-device callbacks are not wired up.
    }

    pub fn destroy(&self) {
        let _guard = rt().lock();
        unsafe { sys::wgpuDeviceDestroy(self.raw()) };
    }

    pub fn start_capture(&self) {}
    pub fn stop_capture(&self) {}
}

fn map_constants(constants: &[(&str, f64)]) -> Vec<sys::WGPUConstantEntry> {
    constants
        .iter()
        .map(|(key, value)| {
            let mut raw: sys::WGPUConstantEntry = unsafe { std::mem::zeroed() };
            raw.key = sv_str(key);
            raw.value = *value;
            raw
        })
        .collect()
}

pub struct ErrorScopeGuard {
    device: Device,
}

impl ErrorScopeGuard {
    pub fn pop(self) -> impl Future<Output = Option<Error>> {
        let (future, handle) = CallbackFuture::<Option<Error>>::new();

        unsafe extern "C" fn on_pop(
            _status: sys::WGPUPopErrorScopeStatus,
            ty: sys::WGPUErrorType,
            message: sys::WGPUStringView,
            userdata1: *mut c_void,
            _userdata2: *mut c_void,
        ) {
            let handle =
                unsafe { Box::from_raw(userdata1 as *mut CallbackHandle<Option<Error>>) };
            if ty == sys::WGPUErrorType_NoError {
                handle.complete(None);
            } else {
                handle.complete(Some(make_error(ty, from_sv(message))));
            }
        }

        {
            let _guard = rt().lock();
            let mut cb: sys::WGPUPopErrorScopeCallbackInfo = unsafe { std::mem::zeroed() };
            cb.mode = sys::WGPUCallbackMode_AllowProcessEvents;
            cb.callback = Some(on_pop);
            cb.userdata1 = Box::into_raw(Box::new(handle)) as *mut c_void;
            unsafe { sys::wgpuDevicePopErrorScope(self.device.raw(), cb) };
        }
        std::mem::forget(self);
        future
    }
}

impl Drop for ErrorScopeGuard {
    fn drop(&mut self) {
        // Pop the scope and discard the result so scopes stay balanced.
        unsafe extern "C" fn on_pop(
            _status: sys::WGPUPopErrorScopeStatus,
            _ty: sys::WGPUErrorType,
            _message: sys::WGPUStringView,
            _userdata1: *mut c_void,
            _userdata2: *mut c_void,
        ) {
        }
        let _guard = rt().lock();
        let mut cb: sys::WGPUPopErrorScopeCallbackInfo = unsafe { std::mem::zeroed() };
        cb.mode = sys::WGPUCallbackMode_AllowProcessEvents;
        cb.callback = Some(on_pop);
        unsafe { sys::wgpuDevicePopErrorScope(self.device.raw(), cb) };
    }
}

// ---------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------

#[derive(Clone)]
pub struct Queue {
    inner: Arc<OwnedQueue>,
}

impl std::fmt::Debug for Queue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Queue").finish()
    }
}

impl Queue {
    fn raw(&self) -> sys::WGPUQueue {
        self.inner.0
    }

    pub fn write_buffer(&self, buffer: &Buffer, offset: BufferAddress, data: &[u8]) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuQueueWriteBuffer(
                self.raw(),
                buffer.raw(),
                offset,
                data.as_ptr() as *const c_void,
                data.len(),
            )
        };
    }

    pub fn write_buffer_with(
        &self,
        buffer: &Buffer,
        offset: BufferAddress,
        size: BufferSize,
    ) -> Option<QueueWriteBufferView> {
        Some(QueueWriteBufferView {
            queue: self.clone(),
            buffer: buffer.clone(),
            offset,
            data: vec![0; size.get() as usize],
        })
    }

    pub fn write_texture(
        &self,
        texture: TexelCopyTextureInfo<'_>,
        data: &[u8],
        data_layout: TexelCopyBufferLayout,
        size: Extent3d,
    ) {
        let _guard = rt().lock();
        let dst = map_texel_copy_texture(&texture);
        let layout = map_texel_copy_layout(&data_layout);
        unsafe {
            sys::wgpuQueueWriteTexture(
                self.raw(),
                &dst,
                data.as_ptr() as *const c_void,
                data.len(),
                &layout,
                &map_extent(size),
            )
        };
    }

    pub fn submit<I: IntoIterator<Item = CommandBuffer>>(
        &self,
        command_buffers: I,
    ) -> SubmissionIndex {
        let buffers: Vec<CommandBuffer> = command_buffers.into_iter().collect();
        let raws: Vec<sys::WGPUCommandBuffer> = buffers.iter().map(|b| b.inner.0).collect();
        let _guard = rt().lock();
        unsafe { sys::wgpuQueueSubmit(self.raw(), raws.len(), raws.as_ptr()) };
        for buffer in &buffers {
            for mapping in buffer.mappings.lock().unwrap().drain(..) {
                mapping
                    .buffer
                    .slice(mapping.offset..mapping.offset + mapping.size)
                    .map_async(mapping.mode, mapping.callback);
            }
        }
        SubmissionIndex(0)
    }

    pub fn get_timestamp_period(&self) -> f32 {
        1.0
    }

    pub fn on_submitted_work_done(&self, callback: impl FnOnce() + Send + 'static) {
        self.on_submitted_work_done_impl(Box::new(callback));
    }

    pub(crate) fn on_submitted_work_done_impl(&self, callback: Box<dyn FnOnce() + Send>) {
        unsafe extern "C" fn on_done(
            _status: sys::WGPUQueueWorkDoneStatus,
            _message: sys::WGPUStringView,
            userdata1: *mut c_void,
            _userdata2: *mut c_void,
        ) {
            let callback =
                unsafe { Box::from_raw(userdata1 as *mut Box<dyn FnOnce() + Send>) };
            dispatch_user_callback(*callback);
        }
        let _guard = rt().lock();
        let mut cb: sys::WGPUQueueWorkDoneCallbackInfo = unsafe { std::mem::zeroed() };
        cb.mode = sys::WGPUCallbackMode_AllowProcessEvents;
        cb.callback = Some(on_done);
        cb.userdata1 = Box::into_raw(Box::new(callback)) as *mut c_void;
        unsafe { sys::wgpuQueueOnSubmittedWorkDone(self.raw(), cb) };
    }

    /// Presents the surface texture and registers a vsync ack so the event
    /// loop can pace the next frame to the client's refresh rate.
    pub fn present(&self, surface_texture: SurfaceTexture) {
        surface_texture.present();
    }
}

pub struct QueueWriteBufferView {
    queue: Queue,
    buffer: Buffer,
    offset: BufferAddress,
    data: Vec<u8>,
}

impl QueueWriteBufferView {
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn slice<'a, S: RangeBounds<usize>>(&'a mut self, bounds: S) -> WriteOnly<'a, [u8]> {
        WriteOnly::from_mut(&mut self.data[(bounds.start_bound().cloned(), bounds.end_bound().cloned())])
    }

    pub fn copy_from_slice(&mut self, src: &[u8]) {
        self.data.copy_from_slice(src);
    }
}

impl Deref for QueueWriteBufferView {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.data
    }
}

impl DerefMut for QueueWriteBufferView {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

impl Drop for QueueWriteBufferView {
    fn drop(&mut self) {
        self.queue.write_buffer(&self.buffer, self.offset, &self.data);
    }
}

fn map_texel_copy_texture(info: &TexelCopyTextureInfo<'_>) -> sys::WGPUTexelCopyTextureInfo {
    sys::WGPUTexelCopyTextureInfo {
        texture: info.texture.raw(),
        mipLevel: info.mip_level,
        origin: map_origin(info.origin),
        aspect: map_texture_aspect(info.aspect),
    }
}

fn map_texel_copy_layout(layout: &TexelCopyBufferLayout) -> sys::WGPUTexelCopyBufferLayout {
    sys::WGPUTexelCopyBufferLayout {
        offset: layout.offset,
        bytesPerRow: layout.bytes_per_row.unwrap_or(sys::WGPU_COPY_STRIDE_UNDEFINED),
        rowsPerImage: layout.rows_per_image.unwrap_or(sys::WGPU_COPY_STRIDE_UNDEFINED),
    }
}

// ---------------------------------------------------------------------
// Buffer
// ---------------------------------------------------------------------

#[derive(Clone)]
pub struct Buffer {
    inner: Arc<OwnedBuffer>,
    size: BufferAddress,
    usage: BufferUsages,
}

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buffer").field("size", &self.size).finish()
    }
}

fn resolve_bounds<S: RangeBounds<BufferAddress>>(bounds: S, size: BufferAddress) -> (u64, u64) {
    let start = match bounds.start_bound() {
        std::ops::Bound::Included(&b) => b,
        std::ops::Bound::Excluded(&b) => b + 1,
        std::ops::Bound::Unbounded => 0,
    };
    let end = match bounds.end_bound() {
        std::ops::Bound::Included(&b) => b + 1,
        std::ops::Bound::Excluded(&b) => b,
        std::ops::Bound::Unbounded => size,
    };
    assert!(start <= end && end <= size, "buffer slice out of range");
    (start, end - start)
}

impl Buffer {
    pub(crate) fn raw(&self) -> sys::WGPUBuffer {
        self.inner.0
    }

    pub fn size(&self) -> BufferAddress {
        self.size
    }

    pub fn usage(&self) -> BufferUsages {
        self.usage
    }

    pub fn slice<S: RangeBounds<BufferAddress>>(&self, bounds: S) -> BufferSlice<'_> {
        let (offset, size) = resolve_bounds(bounds, self.size);
        BufferSlice { buffer: self, offset, size }
    }

    pub fn as_entire_binding(&self) -> BindingResource<'_> {
        BindingResource::Buffer(self.as_entire_buffer_binding())
    }

    pub fn as_entire_buffer_binding(&self) -> BufferBinding<'_> {
        BufferBinding { buffer: self, offset: 0, size: None }
    }

    pub fn map_async<S: RangeBounds<BufferAddress>>(
        &self,
        mode: MapMode,
        bounds: S,
        callback: impl FnOnce(Result<(), BufferAsyncError>) + Send + 'static,
    ) {
        self.slice(bounds).map_async(mode, callback)
    }

    pub fn get_mapped_range<S: RangeBounds<BufferAddress>>(&self, bounds: S) -> BufferView<'_> {
        let (offset, size) = resolve_bounds(bounds, self.size);
        let _guard = rt().lock();
        let ptr = unsafe {
            sys::wgpuBufferGetConstMappedRange(self.raw(), offset as usize, size as usize)
        };
        assert!(!ptr.is_null(), "buffer range is not mapped");
        BufferView { _buffer: self.clone(), ptr: ptr as *const u8, len: size as usize, _marker: PhantomData }
    }

    pub fn get_mapped_range_mut<S: RangeBounds<BufferAddress>>(
        &self,
        bounds: S,
    ) -> BufferViewMut<'_> {
        let (offset, size) = resolve_bounds(bounds, self.size);
        let _guard = rt().lock();
        let ptr =
            unsafe { sys::wgpuBufferGetMappedRange(self.raw(), offset as usize, size as usize) };
        assert!(!ptr.is_null(), "buffer range is not mapped for writing");
        BufferViewMut { _buffer: self.clone(), ptr: ptr as *mut u8, len: size as usize, _marker: PhantomData }
    }

    pub fn unmap(&self) {
        let _guard = rt().lock();
        unsafe { sys::wgpuBufferUnmap(self.raw()) };
    }

    pub fn destroy(&self) {
        let _guard = rt().lock();
        unsafe { sys::wgpuBufferDestroy(self.raw()) };
    }
}

#[derive(Copy, Clone)]
pub struct BufferSlice<'a> {
    buffer: &'a Buffer,
    offset: BufferAddress,
    size: BufferAddress,
}

impl std::fmt::Debug for BufferSlice<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferSlice")
            .field("offset", &self.offset)
            .field("size", &self.size)
            .finish()
    }
}

impl<'a> BufferSlice<'a> {
    pub fn buffer(&self) -> &'a Buffer {
        self.buffer
    }

    pub fn offset(&self) -> BufferAddress {
        self.offset
    }

    pub fn size(&self) -> BufferSize {
        BufferSize::new(self.size).expect("zero-sized buffer slice")
    }

    pub fn slice<S: RangeBounds<BufferAddress>>(&self, bounds: S) -> BufferSlice<'a> {
        let (offset, size) = resolve_bounds(bounds, self.size);
        BufferSlice { buffer: self.buffer, offset: self.offset + offset, size }
    }

    pub fn map_async(
        &self,
        mode: MapMode,
        callback: impl FnOnce(Result<(), BufferAsyncError>) + Send + 'static,
    ) {
        type MapCallback = Box<dyn FnOnce(Result<(), BufferAsyncError>) + Send>;
        unsafe extern "C" fn on_map(
            status: sys::WGPUMapAsyncStatus,
            _message: sys::WGPUStringView,
            userdata1: *mut c_void,
            _userdata2: *mut c_void,
        ) {
            let callback = unsafe { Box::from_raw(userdata1 as *mut MapCallback) };
            let result = if status == sys::WGPUMapAsyncStatus_Success {
                Ok(())
            } else {
                Err(BufferAsyncError)
            };
            dispatch_user_callback(Box::new(move || callback(result)));
        }
        let boxed: MapCallback = Box::new(callback);
        let _guard = rt().lock();
        let mut cb: sys::WGPUBufferMapCallbackInfo = unsafe { std::mem::zeroed() };
        cb.mode = sys::WGPUCallbackMode_AllowProcessEvents;
        cb.callback = Some(on_map);
        cb.userdata1 = Box::into_raw(Box::new(boxed)) as *mut c_void;
        unsafe {
            sys::wgpuBufferMapAsync(
                self.buffer.raw(),
                map_map_mode(mode),
                self.offset as usize,
                self.size as usize,
                cb,
            )
        };
    }

    pub fn get_mapped_range(&self) -> BufferView<'a> {
        let _guard = rt().lock();
        let ptr = unsafe {
            sys::wgpuBufferGetConstMappedRange(
                self.buffer.raw(),
                self.offset as usize,
                self.size as usize,
            )
        };
        assert!(!ptr.is_null(), "buffer range is not mapped");
        BufferView { _buffer: self.buffer.clone(), ptr: ptr as *const u8, len: self.size as usize, _marker: PhantomData }
    }

    pub fn get_mapped_range_mut(&self) -> BufferViewMut<'a> {
        let _guard = rt().lock();
        let ptr = unsafe {
            sys::wgpuBufferGetMappedRange(
                self.buffer.raw(),
                self.offset as usize,
                self.size as usize,
            )
        };
        assert!(!ptr.is_null(), "buffer range is not mapped for writing");
        BufferViewMut { _buffer: self.buffer.clone(), ptr: ptr as *mut u8, len: self.size as usize, _marker: PhantomData }
    }
}

pub struct BufferView<'a> {
    _buffer: Buffer,
    ptr: *const u8,
    len: usize,
    _marker: PhantomData<&'a ()>,
}

unsafe impl Send for BufferView<'_> {}
unsafe impl Sync for BufferView<'_> {}

impl Deref for BufferView<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl AsRef<[u8]> for BufferView<'_> {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

pub struct BufferViewMut<'a> {
    _buffer: Buffer,
    ptr: *mut u8,
    len: usize,
    _marker: PhantomData<&'a ()>,
}

unsafe impl Send for BufferViewMut<'_> {}
unsafe impl Sync for BufferViewMut<'_> {}

impl BufferViewMut<'_> {
    pub fn slice<'b, S: RangeBounds<usize>>(&'b mut self, bounds: S) -> WriteOnly<'b, [u8]> {
        let all: &mut [u8] = unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) };
        WriteOnly::from_mut(&mut all[(bounds.start_bound().cloned(), bounds.end_bound().cloned())])
    }
}

impl Deref for BufferViewMut<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl DerefMut for BufferViewMut<'_> {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl AsMut<[u8]> for BufferViewMut<'_> {
    fn as_mut(&mut self) -> &mut [u8] {
        self
    }
}

impl BufferViewMut<'_> {
    /// The view keeps its buffer alive by value, so the borrow lifetime is
    /// only advisory; the staging belt hands out detached views.
    pub(crate) fn detach(self) -> BufferViewMut<'static> {
        BufferViewMut { _buffer: self._buffer, ptr: self.ptr, len: self.len, _marker: PhantomData }
    }
}

// ---------------------------------------------------------------------
// Texture family
// ---------------------------------------------------------------------

#[derive(Clone)]
pub struct Texture {
    pub(crate) inner: Arc<OwnedTexture>,
    size: Extent3d,
    format: TextureFormat,
    dimension: TextureDimension,
    usage: TextureUsages,
    mip_level_count: u32,
    sample_count: u32,
}

impl std::fmt::Debug for Texture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Texture")
            .field("size", &self.size)
            .field("format", &self.format)
            .finish()
    }
}

impl Texture {
    pub(crate) fn raw(&self) -> sys::WGPUTexture {
        self.inner.0
    }

    pub(crate) fn from_raw_parts(
        raw: sys::WGPUTexture,
        size: Extent3d,
        format: TextureFormat,
        usage: TextureUsages,
    ) -> Self {
        Texture {
            inner: Arc::new(OwnedTexture(raw)),
            size,
            format,
            dimension: TextureDimension::D2,
            usage,
            mip_level_count: 1,
            sample_count: 1,
        }
    }

    pub fn create_view(&self, desc: &TextureViewDescriptor<'_>) -> TextureView {
        let _guard = rt().lock();
        let mut raw_desc: sys::WGPUTextureViewDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.format = desc
            .format
            .map(map_texture_format)
            .unwrap_or(sys::WGPUTextureFormat_Undefined);
        raw_desc.dimension = desc
            .dimension
            .map(map_texture_view_dimension)
            .unwrap_or(sys::WGPUTextureViewDimension_Undefined);
        raw_desc.baseMipLevel = desc.base_mip_level;
        raw_desc.mipLevelCount = desc
            .mip_level_count
            .unwrap_or(sys::WGPU_MIP_LEVEL_COUNT_UNDEFINED);
        raw_desc.baseArrayLayer = desc.base_array_layer;
        raw_desc.arrayLayerCount = desc
            .array_layer_count
            .unwrap_or(sys::WGPU_ARRAY_LAYER_COUNT_UNDEFINED);
        raw_desc.aspect = map_texture_aspect(desc.aspect);
        raw_desc.usage = desc.usage.map(map_texture_usages).unwrap_or(0);
        let raw = unsafe { sys::wgpuTextureCreateView(self.raw(), &raw_desc) };
        TextureView { inner: Arc::new(OwnedTextureView(raw)), texture: self.clone() }
    }

    pub fn as_image_copy(&self) -> TexelCopyTextureInfo<'_> {
        TexelCopyTextureInfo {
            texture: self,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        }
    }

    pub fn size(&self) -> Extent3d {
        self.size
    }
    pub fn width(&self) -> u32 {
        self.size.width
    }
    pub fn height(&self) -> u32 {
        self.size.height
    }
    pub fn depth_or_array_layers(&self) -> u32 {
        self.size.depth_or_array_layers
    }
    pub fn format(&self) -> TextureFormat {
        self.format
    }
    pub fn dimension(&self) -> TextureDimension {
        self.dimension
    }
    pub fn usage(&self) -> TextureUsages {
        self.usage
    }
    pub fn mip_level_count(&self) -> u32 {
        self.mip_level_count
    }
    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub fn destroy(&self) {
        let _guard = rt().lock();
        unsafe { sys::wgpuTextureDestroy(self.raw()) };
    }
}

#[derive(Clone)]
pub struct TextureView {
    pub(crate) inner: Arc<OwnedTextureView>,
    texture: Texture,
}

impl TextureView {
    /// The texture this view refers to.
    pub fn texture(&self) -> &Texture {
        &self.texture
    }
}

impl std::fmt::Debug for TextureView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextureView").finish()
    }
}

#[derive(Clone)]
pub struct Sampler {
    pub(crate) inner: Arc<OwnedSampler>,
}

impl std::fmt::Debug for Sampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sampler").finish()
    }
}

// ---------------------------------------------------------------------
// Bindings, layouts, shaders, pipelines
// ---------------------------------------------------------------------

/// The remote protocol destroys a buffer when its last server-side handle
/// is released, so a bind group keeps the buffers it references alive.
#[derive(Clone)]
pub struct BindGroup {
    pub(crate) inner: Arc<OwnedBindGroup>,
    _keepalive: Arc<Vec<Buffer>>,
}

impl std::fmt::Debug for BindGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BindGroup").finish()
    }
}

macro_rules! simple_handle {
    ($name:ident, $owner:ident) => {
        #[derive(Clone)]
        pub struct $name {
            pub(crate) inner: Arc<$owner>,
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name)).finish()
            }
        }
    };
}

simple_handle!(BindGroupLayout, OwnedBindGroupLayout);
simple_handle!(PipelineLayout, OwnedPipelineLayout);
simple_handle!(ShaderModule, OwnedShaderModule);
#[derive(Clone)]
pub struct CommandBuffer {
    pub(crate) inner: Arc<OwnedCommandBuffer>,
    /// Buffer mappings requested with `map_buffer_on_submit`, started by
    /// `Queue::submit` right after the buffer is submitted.
    pub(crate) mappings: Arc<Mutex<Vec<DeferredBufferMapping>>>,
}
impl std::fmt::Debug for CommandBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandBuffer").finish()
    }
}

pub(crate) struct DeferredBufferMapping {
    pub buffer: Buffer,
    pub mode: MapMode,
    pub offset: BufferAddress,
    pub size: BufferAddress,
    pub callback: Box<dyn FnOnce(Result<(), BufferAsyncError>) + Send + 'static>,
}
simple_handle!(RenderBundle, OwnedRenderBundle);
simple_handle!(QuerySet, OwnedQuerySet);

/// Pipeline caching is a native concept; this type exists only so that
/// `cache: None` compiles.
#[derive(Clone, Debug)]
pub struct PipelineCache {
    _priv: (),
}

#[derive(Clone)]
pub struct RenderPipeline {
    pub(crate) inner: Arc<OwnedRenderPipeline>,
}

impl std::fmt::Debug for RenderPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderPipeline").finish()
    }
}

impl RenderPipeline {
    pub fn get_bind_group_layout(&self, index: u32) -> BindGroupLayout {
        let _guard = rt().lock();
        let raw = unsafe { sys::wgpuRenderPipelineGetBindGroupLayout(self.inner.0, index) };
        BindGroupLayout { inner: Arc::new(OwnedBindGroupLayout(raw)) }
    }
}

#[derive(Clone)]
pub struct ComputePipeline {
    pub(crate) inner: Arc<OwnedComputePipeline>,
}

impl std::fmt::Debug for ComputePipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComputePipeline").finish()
    }
}

impl ComputePipeline {
    pub fn get_bind_group_layout(&self, index: u32) -> BindGroupLayout {
        let _guard = rt().lock();
        let raw = unsafe { sys::wgpuComputePipelineGetBindGroupLayout(self.inner.0, index) };
        BindGroupLayout { inner: Arc::new(OwnedBindGroupLayout(raw)) }
    }
}

// ---------------------------------------------------------------------
// CommandEncoder and passes
// ---------------------------------------------------------------------

pub struct CommandEncoder {
    raw: sys::WGPUCommandEncoder,
    finished: bool,
    mappings: Vec<DeferredBufferMapping>,
}

unsafe impl Send for CommandEncoder {}
unsafe impl Sync for CommandEncoder {}

impl Drop for CommandEncoder {
    fn drop(&mut self) {
        let _guard = rt().lock();
        unsafe { sys::wgpuCommandEncoderRelease(self.raw) };
    }
}

impl std::fmt::Debug for CommandEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandEncoder").finish()
    }
}

fn map_timestamp_writes(
    query_set: &QuerySet,
    begin: Option<u32>,
    end: Option<u32>,
) -> sys::WGPUPassTimestampWrites {
    let mut raw: sys::WGPUPassTimestampWrites = unsafe { std::mem::zeroed() };
    raw.querySet = query_set.inner.0;
    raw.beginningOfPassWriteIndex = begin.unwrap_or(sys::WGPU_QUERY_SET_INDEX_UNDEFINED);
    raw.endOfPassWriteIndex = end.unwrap_or(sys::WGPU_QUERY_SET_INDEX_UNDEFINED);
    raw
}

impl CommandEncoder {
    pub fn finish(mut self) -> CommandBuffer {
        let _guard = rt().lock();
        let mut desc: sys::WGPUCommandBufferDescriptor = unsafe { std::mem::zeroed() };
        desc.label = sv(None);
        let raw = unsafe { sys::wgpuCommandEncoderFinish(self.raw, &desc) };
        self.finished = true;
        CommandBuffer {
            inner: Arc::new(OwnedCommandBuffer(raw)),
            mappings: Arc::new(Mutex::new(std::mem::take(&mut self.mappings))),
        }
    }

    /// Registers a buffer to be mapped once this encoder's commands are
    /// submitted; the callback fires from the websocket reader thread when
    /// the map completes.
    pub fn map_buffer_on_submit<S: RangeBounds<BufferAddress>>(
        &mut self,
        buffer: &Buffer,
        mode: MapMode,
        bounds: S,
        callback: impl FnOnce(Result<(), BufferAsyncError>) + Send + 'static,
    ) {
        let (offset, size) = resolve_bounds(bounds, buffer.size());
        self.mappings.push(DeferredBufferMapping {
            buffer: buffer.clone(),
            mode,
            offset,
            size,
            callback: Box::new(callback),
        });
    }

    pub fn begin_render_pass<'encoder>(
        &'encoder mut self,
        desc: &RenderPassDescriptor<'_>,
    ) -> RenderPass<'encoder> {
        let _guard = rt().lock();
        let color_attachments: Vec<sys::WGPURenderPassColorAttachment> = desc
            .color_attachments
            .iter()
            .map(|attachment| {
                let mut raw: sys::WGPURenderPassColorAttachment = unsafe { std::mem::zeroed() };
                raw.depthSlice = sys::WGPU_DEPTH_SLICE_UNDEFINED;
                match attachment {
                    Some(attachment) => {
                        raw.view = attachment.view.inner.0;
                        if let Some(slice) = attachment.depth_slice {
                            raw.depthSlice = slice;
                        }
                        raw.resolveTarget = attachment
                            .resolve_target
                            .map(|v| v.inner.0)
                            .unwrap_or(std::ptr::null_mut());
                        raw.loadOp = map_load_op(&attachment.ops.load);
                        raw.storeOp = map_store_op(attachment.ops.store);
                        if let LoadOp::Clear(color) = attachment.ops.load {
                            raw.clearValue = map_color(color);
                        }
                    }
                    None => {
                        raw.loadOp = sys::WGPULoadOp_Undefined;
                        raw.storeOp = sys::WGPUStoreOp_Undefined;
                    }
                }
                raw
            })
            .collect();

        let depth_stencil = desc.depth_stencil_attachment.as_ref().map(|attachment| {
            let mut raw: sys::WGPURenderPassDepthStencilAttachment =
                unsafe { std::mem::zeroed() };
            raw.view = attachment.view.inner.0;
            match &attachment.depth_ops {
                Some(ops) => {
                    raw.depthLoadOp = map_load_op(&ops.load);
                    raw.depthStoreOp = map_store_op(ops.store);
                    if let LoadOp::Clear(value) = ops.load {
                        raw.depthClearValue = value;
                    }
                }
                None => {
                    raw.depthReadOnly = 1;
                }
            }
            match &attachment.stencil_ops {
                Some(ops) => {
                    raw.stencilLoadOp = map_load_op(&ops.load);
                    raw.stencilStoreOp = map_store_op(ops.store);
                    if let LoadOp::Clear(value) = ops.load {
                        raw.stencilClearValue = value;
                    }
                }
                None => {
                    raw.stencilReadOnly = 1;
                }
            }
            raw
        });

        let timestamp_writes = desc.timestamp_writes.as_ref().map(|writes| {
            map_timestamp_writes(
                writes.query_set,
                writes.beginning_of_pass_write_index,
                writes.end_of_pass_write_index,
            )
        });

        let mut raw_desc: sys::WGPURenderPassDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.colorAttachmentCount = color_attachments.len();
        raw_desc.colorAttachments = color_attachments.as_ptr();
        raw_desc.depthStencilAttachment = depth_stencil
            .as_ref()
            .map(|ds| ds as *const _)
            .unwrap_or(std::ptr::null());
        raw_desc.occlusionQuerySet = desc
            .occlusion_query_set
            .map(|qs| qs.inner.0)
            .unwrap_or(std::ptr::null_mut());
        raw_desc.timestampWrites = timestamp_writes
            .as_ref()
            .map(|tw| tw as *const _)
            .unwrap_or(std::ptr::null());

        let raw = unsafe { sys::wgpuCommandEncoderBeginRenderPass(self.raw, &raw_desc) };
        RenderPass { raw, ended: false, _marker: PhantomData }
    }

    pub fn begin_compute_pass<'encoder>(
        &'encoder mut self,
        desc: &ComputePassDescriptor<'_>,
    ) -> ComputePass<'encoder> {
        let _guard = rt().lock();
        let timestamp_writes = desc.timestamp_writes.as_ref().map(|writes| {
            map_timestamp_writes(
                writes.query_set,
                writes.beginning_of_pass_write_index,
                writes.end_of_pass_write_index,
            )
        });
        let mut raw_desc: sys::WGPUComputePassDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        raw_desc.timestampWrites = timestamp_writes
            .as_ref()
            .map(|tw| tw as *const _)
            .unwrap_or(std::ptr::null());
        let raw = unsafe { sys::wgpuCommandEncoderBeginComputePass(self.raw, &raw_desc) };
        ComputePass { raw, ended: false, _marker: PhantomData }
    }

    pub fn copy_buffer_to_buffer(
        &mut self,
        source: &Buffer,
        source_offset: BufferAddress,
        destination: &Buffer,
        destination_offset: BufferAddress,
        copy_size: impl Into<Option<BufferAddress>>,
    ) {
        let copy_size = copy_size.into().unwrap_or(source.size() - source_offset);
        let _guard = rt().lock();
        unsafe {
            sys::wgpuCommandEncoderCopyBufferToBuffer(
                self.raw,
                source.raw(),
                source_offset,
                destination.raw(),
                destination_offset,
                copy_size,
            )
        };
    }

    pub fn copy_buffer_to_texture(
        &mut self,
        source: TexelCopyBufferInfo<'_>,
        destination: TexelCopyTextureInfo<'_>,
        copy_size: Extent3d,
    ) {
        let _guard = rt().lock();
        let src = sys::WGPUTexelCopyBufferInfo {
            layout: map_texel_copy_layout(&source.layout),
            buffer: source.buffer.raw(),
        };
        let dst = map_texel_copy_texture(&destination);
        unsafe {
            sys::wgpuCommandEncoderCopyBufferToTexture(
                self.raw,
                &src,
                &dst,
                &map_extent(copy_size),
            )
        };
    }

    pub fn copy_texture_to_buffer(
        &mut self,
        source: TexelCopyTextureInfo<'_>,
        destination: TexelCopyBufferInfo<'_>,
        copy_size: Extent3d,
    ) {
        let _guard = rt().lock();
        let src = map_texel_copy_texture(&source);
        let dst = sys::WGPUTexelCopyBufferInfo {
            layout: map_texel_copy_layout(&destination.layout),
            buffer: destination.buffer.raw(),
        };
        unsafe {
            sys::wgpuCommandEncoderCopyTextureToBuffer(
                self.raw,
                &src,
                &dst,
                &map_extent(copy_size),
            )
        };
    }

    pub fn copy_texture_to_texture(
        &mut self,
        source: TexelCopyTextureInfo<'_>,
        destination: TexelCopyTextureInfo<'_>,
        copy_size: Extent3d,
    ) {
        let _guard = rt().lock();
        let src = map_texel_copy_texture(&source);
        let dst = map_texel_copy_texture(&destination);
        unsafe {
            sys::wgpuCommandEncoderCopyTextureToTexture(
                self.raw,
                &src,
                &dst,
                &map_extent(copy_size),
            )
        };
    }

    pub fn clear_buffer(
        &mut self,
        buffer: &Buffer,
        offset: BufferAddress,
        size: Option<BufferAddress>,
    ) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuCommandEncoderClearBuffer(
                self.raw,
                buffer.raw(),
                offset,
                size.unwrap_or(sys::WGPU_WHOLE_SIZE),
            )
        };
    }

    pub fn resolve_query_set(
        &mut self,
        query_set: &QuerySet,
        query_range: Range<u32>,
        destination: &Buffer,
        destination_offset: BufferAddress,
    ) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuCommandEncoderResolveQuerySet(
                self.raw,
                query_set.inner.0,
                query_range.start,
                query_range.end - query_range.start,
                destination.raw(),
                destination_offset,
            )
        };
    }

    pub fn write_timestamp(&mut self, query_set: &QuerySet, query_index: u32) {
        let _guard = rt().lock();
        unsafe { sys::wgpuCommandEncoderWriteTimestamp(self.raw, query_set.inner.0, query_index) };
    }

    pub fn insert_debug_marker(&mut self, label: &str) {
        let _guard = rt().lock();
        unsafe { sys::wgpuCommandEncoderInsertDebugMarker(self.raw, sv_str(label)) };
    }

    pub fn push_debug_group(&mut self, label: &str) {
        let _guard = rt().lock();
        unsafe { sys::wgpuCommandEncoderPushDebugGroup(self.raw, sv_str(label)) };
    }

    pub fn pop_debug_group(&mut self) {
        let _guard = rt().lock();
        unsafe { sys::wgpuCommandEncoderPopDebugGroup(self.raw) };
    }
}

// ---------------------------------------------------------------------
// RenderPass
// ---------------------------------------------------------------------

pub struct RenderPass<'encoder> {
    raw: sys::WGPURenderPassEncoder,
    ended: bool,
    _marker: PhantomData<&'encoder ()>,
}

unsafe impl Send for RenderPass<'_> {}
unsafe impl Sync for RenderPass<'_> {}

impl Drop for RenderPass<'_> {
    fn drop(&mut self) {
        let _guard = rt().lock();
        if !self.ended {
            unsafe { sys::wgpuRenderPassEncoderEnd(self.raw) };
        }
        unsafe { sys::wgpuRenderPassEncoderRelease(self.raw) };
    }
}

impl std::fmt::Debug for RenderPass<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderPass").finish()
    }
}

impl<'encoder> RenderPass<'encoder> {
    pub fn forget_lifetime(mut self) -> RenderPass<'static> {
        let raw = self.raw;
        let ended = self.ended;
        // Keep Drop from double-releasing.
        self.raw = std::ptr::null_mut();
        self.ended = true;
        let this = RenderPass { raw, ended, _marker: PhantomData };
        std::mem::forget(self);
        this
    }

    pub fn set_pipeline(&mut self, pipeline: &RenderPipeline) {
        let _guard = rt().lock();
        unsafe { sys::wgpuRenderPassEncoderSetPipeline(self.raw, pipeline.inner.0) };
    }

    pub fn set_bind_group<'a, BG>(
        &mut self,
        index: u32,
        bind_group: BG,
        offsets: &[DynamicOffset],
    ) where
        Option<&'a BindGroup>: From<BG>,
    {
        let bind_group: Option<&BindGroup> = bind_group.into();
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderPassEncoderSetBindGroup(
                self.raw,
                index,
                bind_group.map(|bg| bg.inner.0).unwrap_or(std::ptr::null_mut()),
                offsets.len(),
                offsets.as_ptr(),
            )
        };
    }

    pub fn set_blend_constant(&mut self, color: Color) {
        let _guard = rt().lock();
        let raw = map_color(color);
        unsafe { sys::wgpuRenderPassEncoderSetBlendConstant(self.raw, &raw) };
    }

    pub fn set_index_buffer(&mut self, buffer_slice: BufferSlice<'_>, index_format: IndexFormat) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderPassEncoderSetIndexBuffer(
                self.raw,
                buffer_slice.buffer.raw(),
                map_index_format(index_format),
                buffer_slice.offset,
                buffer_slice.size,
            )
        };
    }

    pub fn set_vertex_buffer(&mut self, slot: u32, buffer_slice: BufferSlice<'_>) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderPassEncoderSetVertexBuffer(
                self.raw,
                slot,
                buffer_slice.buffer.raw(),
                buffer_slice.offset,
                buffer_slice.size,
            )
        };
    }

    pub fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let _guard = rt().lock();
        unsafe { sys::wgpuRenderPassEncoderSetScissorRect(self.raw, x, y, width, height) };
    }

    pub fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32, min_depth: f32, max_depth: f32) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderPassEncoderSetViewport(self.raw, x, y, w, h, min_depth, max_depth)
        };
    }

    pub fn set_stencil_reference(&mut self, reference: u32) {
        let _guard = rt().lock();
        unsafe { sys::wgpuRenderPassEncoderSetStencilReference(self.raw, reference) };
    }

    pub fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderPassEncoderDraw(
                self.raw,
                vertices.end - vertices.start,
                instances.end - instances.start,
                vertices.start,
                instances.start,
            )
        };
    }

    pub fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderPassEncoderDrawIndexed(
                self.raw,
                indices.end - indices.start,
                instances.end - instances.start,
                indices.start,
                base_vertex,
                instances.start,
            )
        };
    }

    pub fn draw_indirect(&mut self, indirect_buffer: &Buffer, indirect_offset: BufferAddress) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderPassEncoderDrawIndirect(
                self.raw,
                indirect_buffer.raw(),
                indirect_offset,
            )
        };
    }

    pub fn draw_indexed_indirect(
        &mut self,
        indirect_buffer: &Buffer,
        indirect_offset: BufferAddress,
    ) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderPassEncoderDrawIndexedIndirect(
                self.raw,
                indirect_buffer.raw(),
                indirect_offset,
            )
        };
    }

    /// WebGPU has no multi-draw; the remote backend expands it into `count`
    /// single indirect draws.
    pub fn multi_draw_indirect(
        &mut self,
        indirect_buffer: &Buffer,
        indirect_offset: BufferAddress,
        count: u32,
    ) {
        for i in 0..count as u64 {
            self.draw_indirect(indirect_buffer, indirect_offset + i * 16);
        }
    }

    /// See [`Self::multi_draw_indirect`].
    pub fn multi_draw_indexed_indirect(
        &mut self,
        indirect_buffer: &Buffer,
        indirect_offset: BufferAddress,
        count: u32,
    ) {
        for i in 0..count as u64 {
            self.draw_indexed_indirect(indirect_buffer, indirect_offset + i * 20);
        }
    }

    pub fn multi_draw_indirect_count(
        &mut self,
        _indirect_buffer: &Buffer,
        _indirect_offset: BufferAddress,
        _count_buffer: &Buffer,
        _count_offset: BufferAddress,
        _max_count: u32,
    ) {
        panic!("multi_draw_indirect_count is not available over remote WebGPU")
    }

    pub fn multi_draw_indexed_indirect_count(
        &mut self,
        _indirect_buffer: &Buffer,
        _indirect_offset: BufferAddress,
        _count_buffer: &Buffer,
        _count_offset: BufferAddress,
        _max_count: u32,
    ) {
        panic!("multi_draw_indexed_indirect_count is not available over remote WebGPU")
    }

    /// Forwarded to the C library, which warns and drops it: WebGPU has no
    /// immediates.  Only reachable from paths this backend disables.
    pub fn set_immediates(&mut self, offset: u32, data: &[u8]) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderPassEncoderSetImmediates(
                self.raw,
                offset,
                data.as_ptr() as *const c_void,
                data.len(),
            )
        };
    }

    pub fn execute_bundles<'a, I: IntoIterator<Item = &'a RenderBundle>>(&mut self, bundles: I) {
        let raws: Vec<sys::WGPURenderBundle> =
            bundles.into_iter().map(|b| b.inner.0).collect();
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderPassEncoderExecuteBundles(self.raw, raws.len(), raws.as_ptr())
        };
    }

    pub fn begin_occlusion_query(&mut self, query_index: u32) {
        let _guard = rt().lock();
        unsafe { sys::wgpuRenderPassEncoderBeginOcclusionQuery(self.raw, query_index) };
    }

    pub fn end_occlusion_query(&mut self) {
        let _guard = rt().lock();
        unsafe { sys::wgpuRenderPassEncoderEndOcclusionQuery(self.raw) };
    }

    pub fn begin_pipeline_statistics_query(&mut self, _query_set: &QuerySet, _query_index: u32) {
        panic!("pipeline statistics queries are not available in WebGPU")
    }

    pub fn end_pipeline_statistics_query(&mut self) {
        panic!("pipeline statistics queries are not available in WebGPU")
    }

    pub fn insert_debug_marker(&mut self, label: &str) {
        let _guard = rt().lock();
        unsafe { sys::wgpuRenderPassEncoderInsertDebugMarker(self.raw, sv_str(label)) };
    }

    pub fn push_debug_group(&mut self, label: &str) {
        let _guard = rt().lock();
        unsafe { sys::wgpuRenderPassEncoderPushDebugGroup(self.raw, sv_str(label)) };
    }

    pub fn pop_debug_group(&mut self) {
        let _guard = rt().lock();
        unsafe { sys::wgpuRenderPassEncoderPopDebugGroup(self.raw) };
    }

    pub fn write_timestamp(&mut self, _query_set: &QuerySet, _query_index: u32) {
        panic!("write_timestamp inside passes is not available in WebGPU")
    }
}

// ---------------------------------------------------------------------
// ComputePass
// ---------------------------------------------------------------------

pub struct ComputePass<'encoder> {
    raw: sys::WGPUComputePassEncoder,
    ended: bool,
    _marker: PhantomData<&'encoder ()>,
}

unsafe impl Send for ComputePass<'_> {}
unsafe impl Sync for ComputePass<'_> {}

impl Drop for ComputePass<'_> {
    fn drop(&mut self) {
        let _guard = rt().lock();
        if !self.ended {
            unsafe { sys::wgpuComputePassEncoderEnd(self.raw) };
        }
        unsafe { sys::wgpuComputePassEncoderRelease(self.raw) };
    }
}

impl std::fmt::Debug for ComputePass<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComputePass").finish()
    }
}

impl ComputePass<'_> {
    pub fn forget_lifetime(mut self) -> ComputePass<'static> {
        let raw = self.raw;
        let ended = self.ended;
        self.raw = std::ptr::null_mut();
        self.ended = true;
        let this = ComputePass { raw, ended, _marker: PhantomData };
        std::mem::forget(self);
        this
    }

    pub fn set_pipeline(&mut self, pipeline: &ComputePipeline) {
        let _guard = rt().lock();
        unsafe { sys::wgpuComputePassEncoderSetPipeline(self.raw, pipeline.inner.0) };
    }

    pub fn set_bind_group<'a, BG>(
        &mut self,
        index: u32,
        bind_group: BG,
        offsets: &[DynamicOffset],
    ) where
        Option<&'a BindGroup>: From<BG>,
    {
        let bind_group: Option<&BindGroup> = bind_group.into();
        let _guard = rt().lock();
        unsafe {
            sys::wgpuComputePassEncoderSetBindGroup(
                self.raw,
                index,
                bind_group.map(|bg| bg.inner.0).unwrap_or(std::ptr::null_mut()),
                offsets.len(),
                offsets.as_ptr(),
            )
        };
    }

    pub fn dispatch_workgroups(&mut self, x: u32, y: u32, z: u32) {
        let _guard = rt().lock();
        unsafe { sys::wgpuComputePassEncoderDispatchWorkgroups(self.raw, x, y, z) };
    }

    pub fn dispatch_workgroups_indirect(
        &mut self,
        indirect_buffer: &Buffer,
        indirect_offset: BufferAddress,
    ) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuComputePassEncoderDispatchWorkgroupsIndirect(
                self.raw,
                indirect_buffer.raw(),
                indirect_offset,
            )
        };
    }

    /// See [`RenderPass::set_immediates`].
    pub fn set_immediates(&mut self, offset: u32, data: &[u8]) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuComputePassEncoderSetImmediates(
                self.raw,
                offset,
                data.as_ptr() as *const c_void,
                data.len(),
            )
        };
    }

    pub fn begin_pipeline_statistics_query(&mut self, _query_set: &QuerySet, _query_index: u32) {
        panic!("pipeline statistics queries are not available in WebGPU")
    }

    pub fn end_pipeline_statistics_query(&mut self) {
        panic!("pipeline statistics queries are not available in WebGPU")
    }

    pub fn insert_debug_marker(&mut self, label: &str) {
        let _guard = rt().lock();
        unsafe { sys::wgpuComputePassEncoderInsertDebugMarker(self.raw, sv_str(label)) };
    }

    pub fn push_debug_group(&mut self, label: &str) {
        let _guard = rt().lock();
        unsafe { sys::wgpuComputePassEncoderPushDebugGroup(self.raw, sv_str(label)) };
    }

    pub fn pop_debug_group(&mut self) {
        let _guard = rt().lock();
        unsafe { sys::wgpuComputePassEncoderPopDebugGroup(self.raw) };
    }

    pub fn write_timestamp(&mut self, _query_set: &QuerySet, _query_index: u32) {
        panic!("write_timestamp inside passes is not available in WebGPU")
    }
}

// ---------------------------------------------------------------------
// RenderBundleEncoder
// ---------------------------------------------------------------------

pub struct RenderBundleEncoder<'a> {
    raw: sys::WGPURenderBundleEncoder,
    _marker: PhantomData<&'a ()>,
}

unsafe impl Send for RenderBundleEncoder<'_> {}
unsafe impl Sync for RenderBundleEncoder<'_> {}

impl std::fmt::Debug for RenderBundleEncoder<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderBundleEncoder").finish()
    }
}

impl RenderBundleEncoder<'_> {
    pub fn finish(self, desc: &RenderBundleDescriptor<'_>) -> RenderBundle {
        let _guard = rt().lock();
        let mut raw_desc: sys::WGPURenderBundleDescriptor = unsafe { std::mem::zeroed() };
        raw_desc.label = sv(desc.label);
        let raw = unsafe { sys::wgpuRenderBundleEncoderFinish(self.raw, &raw_desc) };
        unsafe { sys::wgpuRenderBundleEncoderRelease(self.raw) };
        std::mem::forget(self);
        RenderBundle { inner: Arc::new(OwnedRenderBundle(raw)) }
    }

    pub fn set_pipeline(&mut self, pipeline: &RenderPipeline) {
        let _guard = rt().lock();
        unsafe { sys::wgpuRenderBundleEncoderSetPipeline(self.raw, pipeline.inner.0) };
    }

    pub fn set_bind_group<'a, BG>(
        &mut self,
        index: u32,
        bind_group: BG,
        offsets: &[DynamicOffset],
    ) where
        Option<&'a BindGroup>: From<BG>,
    {
        let bind_group: Option<&BindGroup> = bind_group.into();
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderBundleEncoderSetBindGroup(
                self.raw,
                index,
                bind_group.map(|bg| bg.inner.0).unwrap_or(std::ptr::null_mut()),
                offsets.len(),
                offsets.as_ptr(),
            )
        };
    }

    pub fn set_index_buffer(&mut self, buffer_slice: BufferSlice<'_>, index_format: IndexFormat) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderBundleEncoderSetIndexBuffer(
                self.raw,
                buffer_slice.buffer.raw(),
                map_index_format(index_format),
                buffer_slice.offset,
                buffer_slice.size,
            )
        };
    }

    pub fn set_vertex_buffer(&mut self, slot: u32, buffer_slice: BufferSlice<'_>) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderBundleEncoderSetVertexBuffer(
                self.raw,
                slot,
                buffer_slice.buffer.raw(),
                buffer_slice.offset,
                buffer_slice.size,
            )
        };
    }

    pub fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderBundleEncoderDraw(
                self.raw,
                vertices.end - vertices.start,
                instances.end - instances.start,
                vertices.start,
                instances.start,
            )
        };
    }

    pub fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        let _guard = rt().lock();
        unsafe {
            sys::wgpuRenderBundleEncoderDrawIndexed(
                self.raw,
                indices.end - indices.start,
                instances.end - instances.start,
                indices.start,
                base_vertex,
                instances.start,
            )
        };
    }
}

impl Drop for RenderBundleEncoder<'_> {
    fn drop(&mut self) {
        let _guard = rt().lock();
        unsafe { sys::wgpuRenderBundleEncoderRelease(self.raw) };
    }
}

// ---------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------

#[derive(Clone)]
pub struct Surface<'window> {
    inner: Arc<SurfaceShared>,
    _marker: PhantomData<&'window ()>,
}

impl std::fmt::Debug for Surface<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Surface").finish()
    }
}

#[derive(Debug)]
pub enum CurrentSurfaceTexture {
    Success(SurfaceTexture),
    Suboptimal(SurfaceTexture),
    Timeout,
    Occluded,
    Outdated,
    Lost,
    Validation,
}

pub struct SurfaceTexture {
    pub texture: Texture,
    surface: Arc<SurfaceShared>,
}

impl std::fmt::Debug for SurfaceTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceTexture").finish()
    }
}

unsafe extern "C" fn vsync_cb(userdata1: *mut c_void, _userdata2: *mut c_void) {
    // Reclaims the strong reference leaked when the callback was registered.
    let client = unsafe { Arc::from_raw(userdata1 as *const Client) };
    client.vsync_arrived();
    rt().notify();
}

impl SurfaceTexture {
    pub fn present(self) {
        let client = &self.surface.client;
        let _guard = rt().lock();
        unsafe { sys::wgpuSurfacePresent(self.surface.raw.0) };
        let mut cb: remote_sys::WGPURemoteVsyncCallbackInfo = unsafe { std::mem::zeroed() };
        cb.callback = Some(vsync_cb);
        cb.userdata1 = Arc::into_raw(client.clone()) as *mut c_void;
        unsafe { remote_sys::wgpuRemoteSurfaceOnNextVsync(self.surface.raw.0, cb) };
        client.vsync_requested();
    }
}

impl<'window> Surface<'window> {
    pub(crate) fn client(&self) -> Arc<Client> {
        self.inner.client.clone()
    }

    pub fn get_capabilities(&self, _adapter: &Adapter) -> SurfaceCapabilities {
        let _guard = rt().lock();
        let mut caps: sys::WGPUSurfaceCapabilities = unsafe { std::mem::zeroed() };
        let status = unsafe {
            sys::wgpuSurfaceGetCapabilities(self.inner.raw.0, self.inner.client.adapter(), &mut caps)
        };
        if status != sys::WGPUStatus_Success {
            return SurfaceCapabilities::default();
        }
        let formats = unsafe { std::slice::from_raw_parts(caps.formats, caps.formatCount) }
            .iter()
            .filter_map(|&f| unmap_texture_format(f))
            .collect();
        let present_modes = if caps.presentModeCount == 0 {
            vec![PresentMode::Fifo]
        } else {
            unsafe { std::slice::from_raw_parts(caps.presentModes, caps.presentModeCount) }
                .iter()
                .map(|&m| unmap_present_mode(m))
                .collect()
        };
        let alpha_modes = if caps.alphaModeCount == 0 {
            vec![CompositeAlphaMode::Auto]
        } else {
            unsafe { std::slice::from_raw_parts(caps.alphaModes, caps.alphaModeCount) }
                .iter()
                .map(|&m| unmap_alpha_mode(m))
                .collect()
        };
        let result = SurfaceCapabilities {
            formats,
            present_modes,
            alpha_modes,
            usages: unmap_texture_usages(caps.usages),
        };
        unsafe { sys::wgpuSurfaceCapabilitiesFreeMembers(caps) };
        result
    }

    pub fn get_default_config(
        &self,
        adapter: &Adapter,
        width: u32,
        height: u32,
    ) -> Option<SurfaceConfiguration> {
        let caps = self.get_capabilities(adapter);
        Some(SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: *caps.formats.first()?,
            width,
            height,
            present_mode: *caps.present_modes.first()?,
            desired_maximum_frame_latency: 2,
            alpha_mode: CompositeAlphaMode::Auto,
            view_formats: vec![],
        })
    }

    pub fn configure(&self, device: &Device, config: &SurfaceConfiguration) {
        let _guard = rt().lock();
        let view_formats: Vec<sys::WGPUTextureFormat> =
            config.view_formats.iter().map(|&f| map_texture_format(f)).collect();
        let mut raw_config: sys::WGPUSurfaceConfiguration = unsafe { std::mem::zeroed() };
        raw_config.device = device.raw();
        raw_config.format = map_texture_format(config.format);
        raw_config.usage = map_texture_usages(config.usage);
        raw_config.width = config.width;
        raw_config.height = config.height;
        raw_config.viewFormatCount = view_formats.len();
        raw_config.viewFormats = view_formats.as_ptr();
        raw_config.alphaMode = map_alpha_mode(config.alpha_mode);
        raw_config.presentMode = map_present_mode(config.present_mode);
        unsafe { sys::wgpuSurfaceConfigure(self.inner.raw.0, &raw_config) };
        *self.inner.config.lock().unwrap() = Some(config.clone());
        *self.inner.device.lock().unwrap() = Some(device.clone());
    }

    pub fn get_configuration(&self) -> Option<SurfaceConfiguration> {
        self.inner.config.lock().unwrap().clone()
    }

    pub fn get_current_texture(&self) -> CurrentSurfaceTexture {
        let _guard = rt().lock();
        let mut surface_texture: sys::WGPUSurfaceTexture = unsafe { std::mem::zeroed() };
        unsafe { sys::wgpuSurfaceGetCurrentTexture(self.inner.raw.0, &mut surface_texture) };
        let config = self.inner.config.lock().unwrap().clone();
        let make_texture = |raw: sys::WGPUTexture| {
            let (size, format, usage) = match &config {
                Some(config) => (
                    Extent3d {
                        width: config.width,
                        height: config.height,
                        depth_or_array_layers: 1,
                    },
                    config.format,
                    config.usage,
                ),
                None => (Extent3d::default(), TextureFormat::Bgra8Unorm, TextureUsages::RENDER_ATTACHMENT),
            };
            SurfaceTexture {
                texture: Texture::from_raw_parts(raw, size, format, usage),
                surface: self.inner.clone(),
            }
        };
        match surface_texture.status {
            x if x == sys::WGPUSurfaceGetCurrentTextureStatus_SuccessOptimal => {
                CurrentSurfaceTexture::Success(make_texture(surface_texture.texture))
            }
            x if x == sys::WGPUSurfaceGetCurrentTextureStatus_SuccessSuboptimal => {
                CurrentSurfaceTexture::Suboptimal(make_texture(surface_texture.texture))
            }
            x if x == sys::WGPUSurfaceGetCurrentTextureStatus_Timeout => {
                CurrentSurfaceTexture::Timeout
            }
            x if x == sys::WGPUSurfaceGetCurrentTextureStatus_Outdated => {
                CurrentSurfaceTexture::Outdated
            }
            x if x == sys::WGPUSurfaceGetCurrentTextureStatus_Lost => CurrentSurfaceTexture::Lost,
            _ => CurrentSurfaceTexture::Lost,
        }
    }
}


/// Serializes a Naga IR module back to WGSL for the browser, since the wire
/// protocol only carries WGSL source.
fn naga_to_wgsl(module: &naga::Module) -> String {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    let info = Validator::new(ValidationFlags::empty(), Capabilities::all())
        .validate(module)
        .expect("Naga IR shader failed validation before WGSL serialization");
    naga::back::wgsl::write_string(module, &info, naga::back::wgsl::WriterFlags::empty())
        .expect("failed to write Naga IR module as WGSL")
}
