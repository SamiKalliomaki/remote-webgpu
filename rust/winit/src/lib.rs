//! A winit-compatible shim whose "window" is the remote client's canvas.
//!
//! `EventLoop::run_app` drives the shared remote-webgpu runtime: it waits
//! for a browser client, translates its events (canvas resizes, key
//! presses, pointer moves) into winit `WindowEvent`s, and paces
//! `RedrawRequested` to the client's vsync acknowledgements.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use remote_wgpu_runtime::{runtime, Client, ClientEvent, HasRemoteClient};

pub mod dpi {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub struct PhysicalSize<P> {
        pub width: P,
        pub height: P,
    }

    impl<P> PhysicalSize<P> {
        pub fn new(width: P, height: P) -> Self {
            Self { width, height }
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub struct LogicalSize<P> {
        pub width: P,
        pub height: P,
    }

    impl<P> LogicalSize<P> {
        pub fn new(width: P, height: P) -> Self {
            Self { width, height }
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq, Default)]
    pub struct PhysicalPosition<P> {
        pub x: P,
        pub y: P,
    }

    impl<P> PhysicalPosition<P> {
        pub fn new(x: P, y: P) -> Self {
            Self { x, y }
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq, Default)]
    pub struct LogicalPosition<P> {
        pub x: P,
        pub y: P,
    }

    impl<P> LogicalPosition<P> {
        pub fn new(x: P, y: P) -> Self {
            Self { x, y }
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum Size {
        Physical(PhysicalSize<u32>),
        Logical(LogicalSize<f64>),
    }

    impl From<PhysicalSize<u32>> for Size {
        fn from(size: PhysicalSize<u32>) -> Self {
            Size::Physical(size)
        }
    }

    impl<T: Into<f64>> From<LogicalSize<T>> for Size {
        fn from(size: LogicalSize<T>) -> Self {
            Size::Logical(LogicalSize::new(size.width.into(), size.height.into()))
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum Position {
        Physical(PhysicalPosition<i32>),
        Logical(LogicalPosition<f64>),
    }

    impl From<PhysicalPosition<i32>> for Position {
        fn from(position: PhysicalPosition<i32>) -> Self {
            Position::Physical(position)
        }
    }

    impl<T: Into<f64>> From<LogicalPosition<T>> for Position {
        fn from(position: LogicalPosition<T>) -> Self {
            Position::Logical(LogicalPosition::new(position.x.into(), position.y.into()))
        }
    }
}

pub mod keyboard {
    /// Logical key, mirroring `winit::keyboard::Key`.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub enum Key<Str = String> {
        Named(NamedKey),
        Character(Str),
        Unidentified,
        Dead(Option<char>),
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub enum NamedKey {
        Alt,
        AltGraph,
        CapsLock,
        Control,
        Fn,
        NumLock,
        ScrollLock,
        Shift,
        Meta,
        Super,
        Enter,
        Tab,
        Space,
        ArrowDown,
        ArrowLeft,
        ArrowRight,
        ArrowUp,
        End,
        Home,
        PageDown,
        PageUp,
        Backspace,
        Delete,
        Insert,
        Escape,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F11,
        F12,
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub enum PhysicalKey {
        Code(KeyCode),
        Unidentified,
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    #[allow(missing_docs)]
    pub enum KeyCode {
        Backquote, Backslash, BracketLeft, BracketRight, Comma,
        Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
        Equal, Minus, Period, Quote, Semicolon, Slash,
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM,
        KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
        AltLeft, AltRight, ControlLeft, ControlRight, ShiftLeft, ShiftRight,
        SuperLeft, SuperRight,
        Enter, Space, Tab, Backspace, Delete, Insert, Escape, CapsLock,
        ArrowDown, ArrowLeft, ArrowRight, ArrowUp,
        End, Home, PageDown, PageUp,
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub enum KeyLocation {
        #[default]
        Standard,
        Left,
        Right,
        Numpad,
    }

    pub(crate) fn parse_logical(key: &str) -> Key {
        use NamedKey::*;
        let named = match key {
            " " => Space,
            "Alt" => Alt,
            "AltGraph" => AltGraph,
            "CapsLock" => CapsLock,
            "Control" => Control,
            "Fn" => Fn,
            "NumLock" => NumLock,
            "ScrollLock" => ScrollLock,
            "Shift" => Shift,
            "Meta" => Meta,
            "Super" => Super,
            "Enter" => Enter,
            "Tab" => Tab,
            "ArrowDown" => ArrowDown,
            "ArrowLeft" => ArrowLeft,
            "ArrowRight" => ArrowRight,
            "ArrowUp" => ArrowUp,
            "End" => End,
            "Home" => Home,
            "PageDown" => PageDown,
            "PageUp" => PageUp,
            "Backspace" => Backspace,
            "Delete" => Delete,
            "Insert" => Insert,
            "Escape" => Escape,
            "F1" => F1,
            "F2" => F2,
            "F3" => F3,
            "F4" => F4,
            "F5" => F5,
            "F6" => F6,
            "F7" => F7,
            "F8" => F8,
            "F9" => F9,
            "F10" => F10,
            "F11" => F11,
            "F12" => F12,
            "Dead" => return Key::Dead(None),
            "Unidentified" => return Key::Unidentified,
            other => return Key::Character(other.to_string()),
        };
        Key::Named(named)
    }

    pub(crate) fn parse_physical(code: &str) -> PhysicalKey {
        use KeyCode::*;
        let code = match code {
            "Backquote" => Backquote,
            "Backslash" => Backslash,
            "BracketLeft" => BracketLeft,
            "BracketRight" => BracketRight,
            "Comma" => Comma,
            "Digit0" => Digit0, "Digit1" => Digit1, "Digit2" => Digit2, "Digit3" => Digit3,
            "Digit4" => Digit4, "Digit5" => Digit5, "Digit6" => Digit6, "Digit7" => Digit7,
            "Digit8" => Digit8, "Digit9" => Digit9,
            "Equal" => Equal,
            "Minus" => Minus,
            "Period" => Period,
            "Quote" => Quote,
            "Semicolon" => Semicolon,
            "Slash" => Slash,
            "KeyA" => KeyA, "KeyB" => KeyB, "KeyC" => KeyC, "KeyD" => KeyD, "KeyE" => KeyE,
            "KeyF" => KeyF, "KeyG" => KeyG, "KeyH" => KeyH, "KeyI" => KeyI, "KeyJ" => KeyJ,
            "KeyK" => KeyK, "KeyL" => KeyL, "KeyM" => KeyM, "KeyN" => KeyN, "KeyO" => KeyO,
            "KeyP" => KeyP, "KeyQ" => KeyQ, "KeyR" => KeyR, "KeyS" => KeyS, "KeyT" => KeyT,
            "KeyU" => KeyU, "KeyV" => KeyV, "KeyW" => KeyW, "KeyX" => KeyX, "KeyY" => KeyY,
            "KeyZ" => KeyZ,
            "AltLeft" => AltLeft, "AltRight" => AltRight,
            "ControlLeft" => ControlLeft, "ControlRight" => ControlRight,
            "ShiftLeft" => ShiftLeft, "ShiftRight" => ShiftRight,
            "MetaLeft" | "OSLeft" => SuperLeft,
            "MetaRight" | "OSRight" => SuperRight,
            "Enter" => Enter,
            "Space" => Space,
            "Tab" => Tab,
            "Backspace" => Backspace,
            "Delete" => Delete,
            "Insert" => Insert,
            "Escape" => Escape,
            "CapsLock" => CapsLock,
            "ArrowDown" => ArrowDown, "ArrowLeft" => ArrowLeft,
            "ArrowRight" => ArrowRight, "ArrowUp" => ArrowUp,
            "End" => End, "Home" => Home, "PageDown" => PageDown, "PageUp" => PageUp,
            "F1" => F1, "F2" => F2, "F3" => F3, "F4" => F4, "F5" => F5, "F6" => F6,
            "F7" => F7, "F8" => F8, "F9" => F9, "F10" => F10, "F11" => F11, "F12" => F12,
            _ => return PhysicalKey::Unidentified,
        };
        PhysicalKey::Code(code)
    }
}

pub mod event {
    use crate::dpi::{PhysicalPosition, PhysicalSize};
    use crate::keyboard::{Key, KeyLocation, PhysicalKey};

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct DeviceId(pub(crate) u32);

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub enum ElementState {
        Pressed,
        Released,
    }

    impl ElementState {
        pub fn is_pressed(self) -> bool {
            self == ElementState::Pressed
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub enum MouseButton {
        Left,
        Right,
        Middle,
        Back,
        Forward,
        Other(u16),
    }

    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum MouseScrollDelta {
        LineDelta(f32, f32),
        PixelDelta(PhysicalPosition<f64>),
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub enum TouchPhase {
        Started,
        Moved,
        Ended,
        Cancelled,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct KeyEvent {
        pub physical_key: PhysicalKey,
        pub logical_key: Key,
        pub text: Option<String>,
        pub location: KeyLocation,
        pub state: ElementState,
        pub repeat: bool,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum WindowEvent {
        Resized(PhysicalSize<u32>),
        Moved(PhysicalPosition<i32>),
        CloseRequested,
        Destroyed,
        Focused(bool),
        KeyboardInput {
            device_id: DeviceId,
            event: KeyEvent,
            is_synthetic: bool,
        },
        CursorMoved {
            device_id: DeviceId,
            position: PhysicalPosition<f64>,
        },
        CursorEntered {
            device_id: DeviceId,
        },
        CursorLeft {
            device_id: DeviceId,
        },
        MouseWheel {
            device_id: DeviceId,
            delta: MouseScrollDelta,
            phase: TouchPhase,
        },
        MouseInput {
            device_id: DeviceId,
            state: ElementState,
            button: MouseButton,
        },
        Occluded(bool),
        RedrawRequested,
    }
}

pub mod error {
    #[derive(Debug)]
    pub enum EventLoopError {
        Os(OsError),
        AlreadyRunning,
    }

    impl std::fmt::Display for EventLoopError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                EventLoopError::Os(e) => write!(f, "os error: {e}"),
                EventLoopError::AlreadyRunning => write!(f, "event loop already running"),
            }
        }
    }
    impl std::error::Error for EventLoopError {}

    #[derive(Debug)]
    pub struct OsError {
        pub(crate) message: String,
    }

    impl std::fmt::Display for OsError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.message)
        }
    }
    impl std::error::Error for OsError {}
}

pub mod window {
    use super::*;
    use crate::dpi::{PhysicalSize, Position, Size};

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct WindowId(pub(crate) u64);

    #[derive(Debug, Clone, Default)]
    pub struct WindowAttributes {
        pub title: String,
        pub inner_size: Option<Size>,
        pub position: Option<Position>,
    }

    impl WindowAttributes {
        pub fn with_title<T: Into<String>>(mut self, title: T) -> Self {
            self.title = title.into();
            self
        }

        pub fn with_inner_size<S: Into<Size>>(mut self, size: S) -> Self {
            self.inner_size = Some(size.into());
            self
        }

        pub fn with_position<P: Into<Position>>(mut self, position: P) -> Self {
            self.position = Some(position.into());
            self
        }
    }

    pub(crate) struct WindowShared {
        pub(crate) id: u64,
        pub(crate) client: Arc<Client>,
        pub(crate) redraw_requested: AtomicBool,
    }

    /// A handle to one connected client's canvas.  Each window created via
    /// `ActiveEventLoop::create_window` claims the next connected client
    /// (blocking until one connects), so every window is its own browser
    /// tab with its own remote GPU.
    pub struct Window {
        pub(crate) shared: Arc<WindowShared>,
    }

    impl HasRemoteClient for Window {
        fn remote_client(&self) -> Arc<Client> {
            self.shared.client.clone()
        }
    }

    impl std::fmt::Debug for Window {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Window").finish()
        }
    }

    impl Window {
        pub fn default_attributes() -> WindowAttributes {
            WindowAttributes::default()
        }

        pub fn id(&self) -> WindowId {
            WindowId(self.shared.id)
        }

        pub fn inner_size(&self) -> PhysicalSize<u32> {
            let (width, height) = self.shared.client.canvas_size();
            if width == 0 || height == 0 {
                PhysicalSize::new(640, 480)
            } else {
                PhysicalSize::new(width, height)
            }
        }

        pub fn scale_factor(&self) -> f64 {
            1.0
        }

        pub fn request_redraw(&self) {
            self.shared.redraw_requested.store(true, Ordering::SeqCst);
            runtime().notify();
        }

        /// Presentation is paced by the client's vsync acks; nothing to do.
        pub fn pre_present_notify(&self) {}

        pub fn set_title(&self, _title: &str) {}

        /// There is no window system; positioning is meaningless.
        pub fn set_outer_position<P>(&self, _position: P) {}
    }
}

pub mod application {
    use super::event_loop::ActiveEventLoop;
    use super::event::WindowEvent;
    use super::window::WindowId;

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum StartCause {
        Poll,
        Init,
    }

    pub trait ApplicationHandler<T: 'static = ()> {
        fn resumed(&mut self, event_loop: &ActiveEventLoop);

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            window_id: WindowId,
            event: WindowEvent,
        );

        fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: StartCause) {}
        fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: T) {}
        fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {}
        fn suspended(&mut self, _event_loop: &ActiveEventLoop) {}
        fn exiting(&mut self, _event_loop: &ActiveEventLoop) {}
        fn memory_warning(&mut self, _event_loop: &ActiveEventLoop) {}
    }
}

pub mod event_loop {
    use super::*;
    use crate::application::ApplicationHandler;
    use crate::error::{EventLoopError, OsError};
    use crate::event::{DeviceId, ElementState, KeyEvent, WindowEvent};
    use crate::window::{Window, WindowAttributes, WindowId, WindowShared};

    /// The display handle for a display-less backend.
    #[derive(Debug, Clone)]
    pub struct OwnedDisplayHandle {
        pub(crate) _priv: (),
    }

    impl raw_window_handle::HasDisplayHandle for OwnedDisplayHandle {
        fn display_handle(
            &self,
        ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
            let raw = raw_window_handle::RawDisplayHandle::Web(
                raw_window_handle::WebDisplayHandle::new(),
            );
            // SAFETY: there is no underlying display object to outlive.
            Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(raw) })
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
    pub enum ControlFlow {
        Poll,
        #[default]
        Wait,
    }

    pub struct EventLoop<T: 'static = ()> {
        proxy_tx: Sender<T>,
        proxy_rx: Receiver<T>,
    }

    pub struct EventLoopBuilder<T: 'static> {
        _marker: std::marker::PhantomData<T>,
    }

    impl<T: 'static> EventLoopBuilder<T> {
        pub fn build(&mut self) -> Result<EventLoop<T>, EventLoopError> {
            let (proxy_tx, proxy_rx) = std::sync::mpsc::channel();
            Ok(EventLoop { proxy_tx, proxy_rx })
        }
    }

    impl EventLoop<()> {
        #[allow(clippy::new_ret_no_self)]
        pub fn new() -> Result<EventLoop<()>, EventLoopError> {
            EventLoopBuilder { _marker: std::marker::PhantomData }.build()
        }
    }

    impl<T: 'static> EventLoop<T> {
        pub fn with_user_event() -> EventLoopBuilder<T> {
            EventLoopBuilder { _marker: std::marker::PhantomData }
        }

        pub fn create_proxy(&self) -> EventLoopProxy<T> {
            EventLoopProxy { tx: self.proxy_tx.clone() }
        }

        pub fn owned_display_handle(&self) -> OwnedDisplayHandle {
            OwnedDisplayHandle { _priv: () }
        }

        pub fn run_app<A: ApplicationHandler<T>>(self, app: &mut A) -> Result<(), EventLoopError> {
            let active = ActiveEventLoop {
                exit: std::cell::Cell::new(false),
                windows: std::cell::RefCell::new(Vec::new()),
                next_window_id: std::cell::Cell::new(1),
            };

            let rt = runtime();

            app.resumed(&active);

            loop {
                if active.exit.get() {
                    app.exiting(&active);
                    return Ok(());
                }

                // Async init results / user events from proxies.
                while let Ok(user_event) = self.proxy_rx.try_recv() {
                    app.user_event(&active, user_event);
                    if active.exit.get() {
                        app.exiting(&active);
                        return Ok(());
                    }
                }

                // Client events, per window.
                let windows: Vec<Arc<WindowShared>> = active.windows.borrow().clone();
                let mut dispatched = false;
                'windows: for window in &windows {
                    let window_id = WindowId(window.id);
                    while let Some(client_event) = window.client.poll_event() {
                        dispatched = true;
                        match client_event {
                            ClientEvent::Resize { width, height } => {
                                app.window_event(
                                    &active,
                                    window_id,
                                    WindowEvent::Resized(dpi::PhysicalSize::new(width, height)),
                                );
                            }
                            ClientEvent::User { name, payload } => {
                                for event in translate_user_event(&name, &payload) {
                                    app.window_event(&active, window_id, event);
                                }
                            }
                            ClientEvent::Disconnected => {
                                app.window_event(&active, window_id, WindowEvent::CloseRequested);
                                // The client is gone for good; stop polling it.
                                active
                                    .windows
                                    .borrow_mut()
                                    .retain(|w| w.id != window.id);
                            }
                        }
                        if active.exit.get() {
                            break 'windows;
                        }
                    }
                }
                if active.exit.get() {
                    continue;
                }

                // Redraws, paced per window by its client's vsync acks.
                let mut redrew = false;
                for window in &windows {
                    if window.redraw_requested.load(Ordering::SeqCst)
                        && !window.client.vsync_is_pending()
                        && !window.client.is_disconnected()
                    {
                        window.redraw_requested.store(false, Ordering::SeqCst);
                        app.window_event(&active, WindowId(window.id), WindowEvent::RedrawRequested);
                        redrew = true;
                        if active.exit.get() {
                            break;
                        }
                    }
                }
                if redrew || dispatched {
                    continue;
                }

                app.about_to_wait(&active);
                rt.wait(Duration::from_millis(50));
            }
        }
    }

    fn translate_user_event(name: &str, payload: &[u8]) -> Vec<WindowEvent> {
        let device_id = DeviceId(0);
        match name {
            "mousemove" if payload.len() >= 8 => {
                let x = f32::from_le_bytes(payload[0..4].try_into().unwrap());
                let y = f32::from_le_bytes(payload[4..8].try_into().unwrap());
                vec![WindowEvent::CursorMoved {
                    device_id,
                    position: dpi::PhysicalPosition::new(x as f64, y as f64),
                }]
            }
            "keydown" | "keyup" => {
                // Payload: "<code>\n<key>\n<repeat 0|1>", all UTF-8.
                let text = String::from_utf8_lossy(payload);
                let mut parts = text.split('\n');
                let code = parts.next().unwrap_or("");
                let key = parts.next().unwrap_or("");
                let repeat = parts.next() == Some("1");
                let logical_key = crate::keyboard::parse_logical(key);
                let event = KeyEvent {
                    physical_key: crate::keyboard::parse_physical(code),
                    text: match &logical_key {
                        crate::keyboard::Key::Character(c) => Some(c.clone()),
                        _ => None,
                    },
                    logical_key,
                    location: Default::default(),
                    state: if name == "keydown" {
                        ElementState::Pressed
                    } else {
                        ElementState::Released
                    },
                    repeat,
                };
                vec![WindowEvent::KeyboardInput { device_id, event, is_synthetic: false }]
            }
            _ => vec![],
        }
    }

    pub struct ActiveEventLoop {
        pub(crate) exit: std::cell::Cell<bool>,
        pub(crate) windows: std::cell::RefCell<Vec<Arc<WindowShared>>>,
        pub(crate) next_window_id: std::cell::Cell<u64>,
    }

    impl ActiveEventLoop {
        /// Claims the next connected client as this window's canvas,
        /// blocking until one connects (a message says so).
        pub fn create_window(
            &self,
            _attributes: WindowAttributes,
        ) -> Result<Window, OsError> {
            Ok(self.window_for(runtime().next_client()))
        }

        /// A remote-webgpu extension: claim a client that has connected but
        /// has no window yet, without blocking.  Returns `None` while every
        /// connected client already has a window.  Applications that accept
        /// clients joining at any time poll this instead of calling the
        /// blocking [`ActiveEventLoop::create_window`].
        pub fn create_window_for_new_client(
            &self,
            _attributes: WindowAttributes,
        ) -> Option<Window> {
            runtime().try_next_client().map(|client| self.window_for(client))
        }

        fn window_for(&self, client: Arc<Client>) -> Window {
            let id = self.next_window_id.get();
            self.next_window_id.set(id + 1);
            let shared = Arc::new(WindowShared {
                id,
                client,
                redraw_requested: AtomicBool::new(false),
            });
            self.windows.borrow_mut().push(shared.clone());
            Window { shared }
        }

        pub fn exit(&self) {
            self.exit.set(true);
        }

        pub fn exiting(&self) -> bool {
            self.exit.get()
        }

        pub fn owned_display_handle(&self) -> OwnedDisplayHandle {
            OwnedDisplayHandle { _priv: () }
        }

        pub fn set_control_flow(&self, _control_flow: ControlFlow) {}
    }

    pub struct EventLoopProxy<T: 'static> {
        tx: Sender<T>,
    }

    impl<T: 'static> Clone for EventLoopProxy<T> {
        fn clone(&self) -> Self {
            EventLoopProxy { tx: self.tx.clone() }
        }
    }

    #[derive(Debug)]
    pub struct EventLoopClosed<T>(pub T);

    impl<T: 'static> EventLoopProxy<T> {
        pub fn send_event(&self, event: T) -> Result<(), EventLoopClosed<T>> {
            self.tx.send(event).map_err(|e| EventLoopClosed(e.0))?;
            runtime().notify();
            Ok(())
        }
    }
}
