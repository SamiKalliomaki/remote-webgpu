//! Shared runtime for the wgpu- and winit-compatible crates.
//!
//! Owns the websocket server that browser clients connect to.  Any number
//! of clients may connect; each one gets its own `remote_webgpu`
//! instance/adapter and its own [`Client`] carrying the state the two API
//! crates rendezvous through: the client's event queue (resizes, key
//! presses, pointer moves), present/vsync pacing and connection liveness.
//! The winit-compatible crate claims one connected client per `Window`;
//! the wgpu-compatible crate exposes each client as an `Adapter`.
//!
//! Threading model: every call into the C library happens while holding
//! [`Runtime::lock`], a reentrant mutex shared by all clients (completion
//! callbacks fire from inside `wgpuRemoteAdapterReceiveData()` and may call
//! back into the API).  A reader thread per client pumps its websocket
//! messages into the library and bumps a global progress counter
//! afterwards; blocking waits loop on that counter.

use std::collections::VecDeque;
use std::net::TcpListener;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use remote_wgpu_sys as sys;
use sys::remote as remote_sys;

/// An event reported by a client, drained by the winit-compatible crate.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// The client's canvas changed physical size (surface must be
    /// reconfigured for the canvas backing store to actually resize).
    Resize { width: u32, height: u32 },
    /// A user-defined event from the client application (name + payload).
    User { name: String, payload: Vec<u8> },
    /// The websocket connection closed.
    Disconnected,
}

/// Implemented by window-like types that are backed by a connected client;
/// this is how `wgpu`'s `create_surface` finds the client behind whatever
/// window handle it is given.
pub trait HasRemoteClient {
    fn remote_client(&self) -> Arc<Client>;
}

impl HasRemoteClient for Client {
    fn remote_client(&self) -> Arc<Client> {
        // Every Client the runtime hands out lives in the registry's Arcs,
        // so a plain reference can be re-wrapped by looking it up again.
        runtime()
            .clients()
            .into_iter()
            .find(|client| client.id() == self.id())
            .expect("Client is not registered with the runtime")
    }
}

impl<T: HasRemoteClient> HasRemoteClient for Arc<T> {
    fn remote_client(&self) -> Arc<Client> {
        (**self).remote_client()
    }
}

impl<T: HasRemoteClient> HasRemoteClient for &T {
    fn remote_client(&self) -> Arc<Client> {
        (**self).remote_client()
    }
}

struct SendCtx {
    id: u64,
    tx: Sender<Vec<u8>>,
}

/// Temporary instrumentation: with REMOTE_WEBGPU_TRAFFIC_STATS=1, tally
/// outgoing bytes per client per envelope type and print every 2 seconds.
fn traffic_stats(client: u64, envelope: &[u8]) {
    use std::collections::HashMap;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var("REMOTE_WEBGPU_TRAFFIC_STATS").is_ok()) {
        return;
    }
    static STATS: OnceLock<Mutex<(HashMap<(u64, u32), (u64, u64)>, Option<std::time::Instant>)>> =
        OnceLock::new();
    // The envelope is a protobuf message whose first field tag names the
    // oneof variant: varint key = (field_number << 3) | wire_type.
    let mut key: u32 = 0;
    let mut shift = 0;
    for &byte in envelope.iter().take(5) {
        key |= ((byte & 0x7f) as u32) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    let field = key >> 3;
    let mut stats = STATS.get_or_init(Default::default).lock().unwrap();
    let (map, last_print) = &mut *stats;
    let entry = map.entry((client, field)).or_insert((0, 0));
    entry.0 += envelope.len() as u64 + 4;
    entry.1 += 1;
    let now = std::time::Instant::now();
    let due = last_print.map_or(true, |t| now.duration_since(t) >= Duration::from_secs(2));
    if due {
        let elapsed = last_print.map_or(2.0, |t| now.duration_since(t).as_secs_f64());
        *last_print = Some(now);
        let mut rows: Vec<_> = map.iter().collect();
        rows.sort_by_key(|(_, (bytes, _))| std::cmp::Reverse(*bytes));
        eprintln!("--- traffic per {elapsed:.1}s ---");
        for ((client, field), (bytes, count)) in rows {
            eprintln!(
                "  client {client} field {field:3}: {:9.1} KiB/s  {:7.0} msg/s",
                *bytes as f64 / elapsed / 1024.0,
                *count as f64 / elapsed,
            );
        }
        map.clear();
    }
}

/// Reentrant lock guarding all calls into the C library.
pub struct CLock {
    owner: Mutex<(Option<std::thread::ThreadId>, u32)>,
    cond: Condvar,
}

pub struct CLockGuard<'a>(&'a CLock);

impl CLock {
    fn new() -> Self {
        CLock {
            owner: Mutex::new((None, 0)),
            cond: Condvar::new(),
        }
    }

    pub fn lock(&self) -> CLockGuard<'_> {
        let me = std::thread::current().id();
        let mut state = self.owner.lock().unwrap();
        loop {
            match state.0 {
                None => {
                    *state = (Some(me), 1);
                    return CLockGuard(self);
                }
                Some(owner) if owner == me => {
                    state.1 += 1;
                    return CLockGuard(self);
                }
                _ => state = self.cond.wait(state).unwrap(),
            }
        }
    }
}

impl Drop for CLockGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.0.owner.lock().unwrap();
        state.1 -= 1;
        if state.1 == 0 {
            state.0 = None;
            self.0.cond.notify_one();
        }
    }
}

/// One connected browser client: its own remote instance/adapter plus the
/// per-connection state (events, vsync pacing, liveness).
pub struct Client {
    id: u64,
    instance: sys::WGPUInstance,
    adapter: sys::WGPUAdapter,

    events: Mutex<VecDeque<ClientEvent>>,
    disconnected: AtomicBool,
    /// Number of presented frames whose vsync ack is still outstanding.
    vsync_pending: AtomicU64,
}

// The raw handles are only ever used under the runtime lock.
unsafe impl Send for Client {}
unsafe impl Sync for Client {}

impl Client {
    /// Small integer identifying this client (1 for the first connection).
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn instance(&self) -> sys::WGPUInstance {
        self.instance
    }

    pub fn adapter(&self) -> sys::WGPUAdapter {
        self.adapter
    }

    pub fn is_disconnected(&self) -> bool {
        self.disconnected.load(Ordering::SeqCst)
    }

    pub fn canvas_size(&self) -> (u32, u32) {
        let _guard = runtime().lock();
        let (mut w, mut h) = (0u32, 0u32);
        unsafe { remote_sys::wgpuRemoteAdapterGetCanvasSize(self.adapter, &mut w, &mut h) };
        (w, h)
    }

    pub fn poll_event(&self) -> Option<ClientEvent> {
        self.events.lock().unwrap().pop_front()
    }

    pub fn has_events(&self) -> bool {
        !self.events.lock().unwrap().is_empty()
    }

    /// Called by the wgpu crate's `SurfaceTexture::present`: the frame was
    /// queued and a vsync ack requested; the winit crate paces this
    /// client's redraws on it.
    pub fn vsync_requested(&self) {
        self.vsync_pending.fetch_add(1, Ordering::SeqCst);
    }

    /// Called from the vsync completion callback.
    pub fn vsync_arrived(&self) {
        self.vsync_pending.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn vsync_is_pending(&self) -> bool {
        self.vsync_pending.load(Ordering::SeqCst) > 0
    }

    pub fn vsync_frames_pending(&self) -> u64 {
        self.vsync_pending.load(Ordering::SeqCst)
    }
}

pub struct Runtime {
    lock: CLock,
    port: u16,

    /// Bumped after every processed websocket message (and on wake-ups);
    /// blocking waits sleep on this.
    progress: Mutex<u64>,
    progress_cond: Condvar,

    /// Every client that completed the handshake, in connection order.
    clients: Mutex<Vec<Arc<Client>>>,
    /// Connected clients not yet claimed by a `Window`.
    unclaimed: Mutex<VecDeque<Arc<Client>>>,
}

static RUNTIME: OnceLock<&'static Runtime> = OnceLock::new();

/// The global runtime, started on first use: binds the websocket port and
/// begins accepting clients in the background.  Nothing blocks until
/// someone asks for a client ([`Runtime::next_client`] /
/// [`Runtime::default_client`]).
pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(Runtime::start)
}

/// Splits one binary websocket message into the size-prefixed envelopes it
/// carries (see the transport notes in the .proto).  A malformed prefix
/// ends the iteration; the C library's parser reports the garbage envelope.
fn envelopes(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = data;
    std::iter::from_fn(move || {
        let (prefix, tail) = rest.split_at_checked(4)?;
        let size = u32::from_le_bytes(prefix.try_into().unwrap()) as usize;
        let (envelope, tail) = tail.split_at_checked(size)?;
        rest = tail;
        Some(envelope)
    })
}

unsafe extern "C" fn send_cb(data: *const c_void, size: usize, userdata: *mut c_void) {
    let ctx = unsafe { &*(userdata as *const SendCtx) };
    let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, size) }.to_vec();
    traffic_stats(ctx.id, &bytes);
    // A send failure means the writer thread is gone; the reader thread
    // notices the close and reports the disconnect.
    let _ = ctx.tx.send(bytes);
}

unsafe extern "C" fn event_cb(
    event: *const remote_sys::WGPURemoteEvent,
    userdata1: *mut c_void,
    _userdata2: *mut c_void,
) {
    let client = unsafe { &*(userdata1 as *const Client) };
    let event = unsafe { &*event };
    let parsed = match event.r#type {
        remote_sys::WGPURemoteEventType_CanvasResize => ClientEvent::Resize {
            width: event.width,
            height: event.height,
        },
        remote_sys::WGPURemoteEventType_User => {
            let name = unsafe { std::ffi::CStr::from_ptr(event.name) }
                .to_string_lossy()
                .into_owned();
            let payload = if event.payload.is_null() || event.payload_size == 0 {
                Vec::new()
            } else {
                unsafe {
                    std::slice::from_raw_parts(event.payload as *const u8, event.payload_size)
                }
                .to_vec()
            };
            ClientEvent::User { name, payload }
        }
        _ => return,
    };
    client.events.lock().unwrap().push_back(parsed);
}

impl Runtime {
    fn start() -> &'static Runtime {
        let port: u16 = std::env::var("REMOTE_WEBGPU_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);
        let listener = TcpListener::bind(("0.0.0.0", port))
            .unwrap_or_else(|e| panic!("remote-wgpu: failed to bind port {port}: {e}"));
        eprintln!("remote-wgpu: accepting clients on ws://localhost:{port}");

        let runtime: &'static Runtime = Box::leak(Box::new(Runtime {
            lock: CLock::new(),
            port,
            progress: Mutex::new(0),
            progress_cond: Condvar::new(),
            clients: Mutex::new(Vec::new()),
            unclaimed: Mutex::new(VecDeque::new()),
        }));

        std::thread::Builder::new()
            .name("remote-wgpu-accept".into())
            .spawn(move || {
                let mut next_id: u64 = 1;
                for stream in listener.incoming() {
                    let stream = match stream {
                        Ok(stream) => stream,
                        Err(error) => {
                            eprintln!("remote-wgpu: accept failed: {error}");
                            continue;
                        }
                    };
                    let id = next_id;
                    next_id += 1;
                    std::thread::Builder::new()
                        .name(format!("remote-wgpu-client-{id}"))
                        .spawn(move || {
                            if let Err(error) = runtime.connect_client(id, stream) {
                                eprintln!("remote-wgpu: client #{id} setup failed: {error}");
                            }
                        })
                        .expect("spawn client setup thread");
                }
            })
            .expect("spawn accept thread");

        runtime
    }

    /// Perform the websocket + remote-adapter handshake for one connection
    /// and run its reader loop (this call is the connection's reader
    /// thread; it returns when the client disconnects).
    fn connect_client(
        &'static self,
        id: u64,
        stream: std::net::TcpStream,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let peer = stream.peer_addr()?;
        stream.set_nodelay(true).ok();
        eprintln!("remote-wgpu: client #{id} connected from {peer}");

        let mut read_ws = tungstenite::accept(stream.try_clone()?)?;
        // A second websocket over a clone of the stream, used only for
        // writing.  Reads happen only on `read_ws`, writes only here, so the
        // two never interleave frames (the browser does not send pings).
        let mut write_ws = tungstenite::WebSocket::from_raw_socket(
            read_ws.get_ref().try_clone()?,
            tungstenite::protocol::Role::Server,
            None,
        );

        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = std::sync::mpsc::channel();
        let send_ctx = Box::leak(Box::new(SendCtx { id, tx }));

        std::thread::Builder::new()
            .name(format!("remote-wgpu-writer-{id}"))
            .spawn(move || {
                // A websocket message carries a *batch* of size-prefixed
                // envelopes (see the transport notes in the .proto): every
                // envelope already waiting in the channel joins the batch,
                // so a burst of commands costs one websocket message and
                // one syscall instead of one each.
                // Send everything queued as one message, sleep 5 ms, and
                // repeat: envelopes batch up while the writer sleeps.  The
                // blocking recv() just keeps an idle connection from
                // spinning.
                while let Ok(mut message) = rx.recv() {
                    let mut batch = Vec::with_capacity(4 + message.len());
                    loop {
                        batch.extend_from_slice(&(message.len() as u32).to_le_bytes());
                        batch.extend_from_slice(&message);
                        match rx.try_recv() {
                            Ok(next) => message = next,
                            Err(_) => break,
                        }
                    }
                    if write_ws.send(tungstenite::Message::Binary(batch)).is_err() {
                        break;
                    }
                }
                let _ = write_ws.close(None);
            })?;

        let (instance, adapter) = {
            let _guard = self.lock();
            unsafe {
                let instance = sys::wgpuCreateInstance(std::ptr::null());
                assert!(!instance.is_null(), "wgpuCreateInstance failed");
                let adapter = remote_sys::wgpuRemoteInstanceCreateAdapter(
                    instance,
                    Some(send_cb),
                    send_ctx as *mut SendCtx as *mut c_void,
                );
                assert!(!adapter.is_null(), "wgpuRemoteInstanceCreateAdapter failed");
                (instance, adapter)
            }
        };

        let client = Arc::new(Client {
            id,
            instance,
            adapter,
            events: Mutex::new(VecDeque::new()),
            disconnected: AtomicBool::new(false),
            vsync_pending: AtomicU64::new(0),
        });

        {
            let _guard = self.lock();
            // The callback borrows the client for the connection's lifetime;
            // hand it a leaked strong reference.
            let callback_ref = Arc::into_raw(client.clone());
            unsafe {
                remote_sys::wgpuRemoteAdapterSetEventCallback(
                    adapter,
                    remote_sys::WGPURemoteEventCallbackInfo {
                        callback: Some(event_cb),
                        userdata1: callback_ref as *mut c_void,
                        userdata2: std::ptr::null_mut(),
                    },
                );
            }
        }

        // Pump until the client's hello arrives; this thread is the
        // connection's only reader, so pump inline.
        while {
            let _guard = self.lock();
            unsafe { remote_sys::wgpuRemoteAdapterIsReady(client.adapter) == 0 }
        } {
            match read_ws.read() {
                Ok(tungstenite::Message::Binary(data)) => {
                    let _guard = self.lock();
                    for envelope in envelopes(&data) {
                        unsafe {
                            remote_sys::wgpuRemoteAdapterReceiveData(
                                client.adapter,
                                envelope.as_ptr() as *const c_void,
                                envelope.len(),
                            );
                        }
                    }
                }
                Ok(tungstenite::Message::Close(_)) | Err(_) => {
                    return Err("client disconnected during handshake".into());
                }
                Ok(_) => {}
            }
        }
        eprintln!("remote-wgpu: client #{id} handshake complete, remote adapter ready");

        self.clients.lock().unwrap().push(client.clone());
        self.unclaimed.lock().unwrap().push_back(client.clone());
        self.notify();

        // Reader loop: pump every websocket message into the library.
        loop {
            match read_ws.read() {
                Ok(tungstenite::Message::Binary(data)) => {
                    {
                        let _guard = self.lock();
                        for envelope in envelopes(&data) {
                            unsafe {
                                remote_sys::wgpuRemoteAdapterReceiveData(
                                    client.adapter,
                                    envelope.as_ptr() as *const c_void,
                                    envelope.len(),
                                );
                            }
                        }
                    }
                    self.notify();
                }
                Ok(tungstenite::Message::Close(_)) | Err(_) => {
                    client.disconnected.store(true, Ordering::SeqCst);
                    client
                        .events
                        .lock()
                        .unwrap()
                        .push_back(ClientEvent::Disconnected);
                    self.notify();
                    eprintln!("remote-wgpu: client #{id} disconnected");
                    return Ok(());
                }
                Ok(_) => {}
            }
        }
    }

    /// Acquire the (reentrant) lock guarding all C library calls.
    pub fn lock(&self) -> CLockGuard<'_> {
        self.lock.lock()
    }

    /// Wake anything blocked in [`Runtime::wait_until`] / [`Runtime::wait`].
    pub fn notify(&self) {
        let mut progress = self.progress.lock().unwrap();
        *progress += 1;
        self.progress_cond.notify_all();
    }

    /// Block until `pred()` returns true, waking on incoming messages.
    /// `pred` is called without the C lock held; take it inside if needed.
    pub fn wait_until(&self, mut pred: impl FnMut() -> bool) {
        loop {
            if pred() {
                return;
            }
            let progress = self.progress.lock().unwrap();
            let last = *progress;
            let _unused = self
                .progress_cond
                .wait_timeout_while(progress, Duration::from_millis(100), |p| *p == last)
                .unwrap();
        }
    }

    /// Block until woken (message processed, event queued, proxy wake-up) or
    /// the timeout elapses.
    pub fn wait(&self, timeout: Duration) {
        let progress = self.progress.lock().unwrap();
        let last = *progress;
        let _unused = self
            .progress_cond
            .wait_timeout_while(progress, timeout, |p| *p == last)
            .unwrap();
    }

    /// Claim the next connected client no window has claimed yet, or
    /// `None` if every connected client is already claimed.  The
    /// non-blocking counterpart of [`Runtime::next_client`], for
    /// applications that let clients join while they are running.
    pub fn try_next_client(&self) -> Option<Arc<Client>> {
        self.unclaimed.lock().unwrap().pop_front()
    }

    /// Claim the next connected client no window has claimed yet, blocking
    /// until one connects.  Each `Window` owns the client it claims.
    pub fn next_client(&self) -> Arc<Client> {
        let mut announced = false;
        let mut result = None;
        self.wait_until(|| {
            if let Some(client) = self.unclaimed.lock().unwrap().pop_front() {
                result = Some(client);
                return true;
            }
            if !announced {
                eprintln!(
                    "remote-wgpu: waiting for a client on ws://localhost:{} ...",
                    self.port
                );
                announced = true;
            }
            false
        });
        result.unwrap()
    }

    /// The first connected client, blocking until one connects.  Used when
    /// an adapter is requested without a surface (compute-only apps);
    /// unlike [`Runtime::next_client`] this does not claim it.
    pub fn default_client(&self) -> Arc<Client> {
        let mut announced = false;
        let mut result = None;
        self.wait_until(|| {
            if let Some(client) = self.clients.lock().unwrap().first() {
                result = Some(client.clone());
                return true;
            }
            if !announced {
                eprintln!(
                    "remote-wgpu: waiting for a client on ws://localhost:{} ...",
                    self.port
                );
                announced = true;
            }
            false
        });
        result.unwrap()
    }

    /// Every client that has completed the handshake, in connection order
    /// (including disconnected ones).
    pub fn clients(&self) -> Vec<Arc<Client>> {
        self.clients.lock().unwrap().clone()
    }
}
