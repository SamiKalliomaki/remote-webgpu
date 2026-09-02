//! Hand-written bindings for `webgpu/remote.h`.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

use std::os::raw::{c_char, c_void};

use crate::ffi::*;

pub type WGPURemoteSendCallback =
    Option<unsafe extern "C" fn(data: *const c_void, size: usize, userdata: *mut c_void)>;

pub type WGPURemoteEventType = u32;
pub const WGPURemoteEventType_CanvasResize: WGPURemoteEventType = 1;
pub const WGPURemoteEventType_User: WGPURemoteEventType = 2;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct WGPURemoteEvent {
    pub r#type: WGPURemoteEventType,
    pub width: u32,
    pub height: u32,
    pub name: *const c_char,
    pub payload: *const c_void,
    pub payload_size: usize,
}

pub type WGPURemoteEventCallback = Option<
    unsafe extern "C" fn(event: *const WGPURemoteEvent, userdata1: *mut c_void, userdata2: *mut c_void),
>;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct WGPURemoteEventCallbackInfo {
    pub callback: WGPURemoteEventCallback,
    pub userdata1: *mut c_void,
    pub userdata2: *mut c_void,
}

pub type WGPURemoteTextureLoadCallback = Option<
    unsafe extern "C" fn(
        status: WGPUStatus,
        texture: WGPUTexture,
        message: WGPUStringView,
        userdata1: *mut c_void,
        userdata2: *mut c_void,
    ),
>;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct WGPURemoteTextureLoadCallbackInfo {
    pub callback: WGPURemoteTextureLoadCallback,
    pub userdata1: *mut c_void,
    pub userdata2: *mut c_void,
}

pub type WGPURemoteVsyncCallback =
    Option<unsafe extern "C" fn(userdata1: *mut c_void, userdata2: *mut c_void)>;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct WGPURemoteVsyncCallbackInfo {
    pub callback: WGPURemoteVsyncCallback,
    pub userdata1: *mut c_void,
    pub userdata2: *mut c_void,
}

unsafe extern "C" {
    pub fn wgpuRemoteInstanceCreateAdapter(
        instance: WGPUInstance,
        send: WGPURemoteSendCallback,
        userdata: *mut c_void,
    ) -> WGPUAdapter;
    pub fn wgpuRemoteAdapterReceiveData(adapter: WGPUAdapter, data: *const c_void, size: usize);
    pub fn wgpuRemoteAdapterAbandonRequests(adapter: WGPUAdapter);
    pub fn wgpuRemoteAdapterDisconnect(adapter: WGPUAdapter);
    pub fn wgpuRemoteAdapterIsReady(adapter: WGPUAdapter) -> WGPUBool;
    pub fn wgpuRemoteAdapterGetCanvasSize(adapter: WGPUAdapter, width: *mut u32, height: *mut u32);
    pub fn wgpuRemoteAdapterSetEventCallback(
        adapter: WGPUAdapter,
        callbackInfo: WGPURemoteEventCallbackInfo,
    );
    pub fn wgpuRemoteDeviceLoadTextureFromURL(
        device: WGPUDevice,
        url: WGPUStringView,
        usage: WGPUTextureUsage,
        callbackInfo: WGPURemoteTextureLoadCallbackInfo,
    ) -> WGPUFuture;
    pub fn wgpuRemoteSurfaceOnNextVsync(
        surface: WGPUSurface,
        callbackInfo: WGPURemoteVsyncCallbackInfo,
    ) -> WGPUFuture;
}
