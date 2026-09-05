//! A minimal stand-in for the browser, for lifetime tests: it speaks just
//! enough of the protocol to complete the handshake (a `ClientHello` with
//! nothing but the protocol version) and then holds the socket open until
//! dropped.  Shared between the runtime and wgpu crates' tests via
//! `#[path]`.

#![allow(dead_code)]

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest;
use tungstenite::http::header::{HeaderName, HeaderValue};

const PROTOCOL_VERSION: u64 = remote_wgpu_sys::PROTOCOL_VERSION as u64;

/// Tests share one runtime and claim clients from one queue, so they must
/// not interleave; every test holds this for its whole body.
pub fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Start the runtime on a free port, once per test process.
pub fn runtime() -> &'static remote_wgpu_runtime::Runtime {
    static PORT: OnceLock<u16> = OnceLock::new();
    let port = *PORT.get_or_init(|| {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        remote_wgpu_runtime::set_port(port);
        port
    });
    let runtime = remote_wgpu_runtime::runtime();
    assert_eq!(runtime.port(), port);
    runtime
}

fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

fn varint_field(field: u64, value: u64) -> Vec<u8> {
    let mut out = varint(field << 3);
    out.extend(varint(value));
    out
}

fn bytes_field(field: u64, data: &[u8]) -> Vec<u8> {
    let mut out = varint(field << 3 | 2);
    out.extend(varint(data.len() as u64));
    out.extend_from_slice(data);
    out
}

/// `Envelope { client_hello: ClientHello { protocol_version } }`.
fn client_hello() -> Vec<u8> {
    bytes_field(2, &varint_field(1, PROTOCOL_VERSION))
}

pub struct FakeClient {
    ws: tungstenite::WebSocket<TcpStream>,
}

impl FakeClient {
    /// Connect and complete the handshake; returns once the runtime lists
    /// the new client.
    pub fn connect(runtime: &remote_wgpu_runtime::Runtime) -> FakeClient {
        FakeClient::connect_with_headers(runtime, &[])
    }

    /// As [`FakeClient::connect`], with extra headers on the websocket
    /// upgrade request -- how a reverse proxy (or a client pretending to be
    /// one) names the address behind it.
    pub fn connect_with_headers(
        runtime: &remote_wgpu_runtime::Runtime,
        headers: &[(&str, &str)],
    ) -> FakeClient {
        let before = runtime.clients().len();
        let stream = TcpStream::connect(("127.0.0.1", runtime.port())).unwrap();
        let mut request = format!("ws://127.0.0.1:{}/", runtime.port())
            .into_client_request()
            .unwrap();
        for (name, value) in headers {
            request.headers_mut().insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        let (ws, _response) = tungstenite::client(request, stream).unwrap();
        let mut client = FakeClient { ws };
        client.send_envelope(&client_hello());
        wait_for(|| runtime.clients().len() > before, "the handshake to complete");
        client
    }

    /// Send one size-prefixed envelope as a binary websocket message.
    pub fn send_envelope(&mut self, envelope: &[u8]) {
        let mut message = Vec::with_capacity(4 + envelope.len());
        message.extend_from_slice(&(envelope.len() as u32).to_le_bytes());
        message.extend_from_slice(envelope);
        self.ws.send(tungstenite::Message::Binary(message)).unwrap();
        self.ws.get_mut().flush().unwrap();
    }

    /// Hang up without a close frame, the way a killed tab does.
    pub fn disconnect(self) {
        let _ = self.ws.get_ref().shutdown(std::net::Shutdown::Both);
    }
}

/// Poll `pred` until it holds or a generous deadline passes.
pub fn wait_for(mut pred: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pred() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
