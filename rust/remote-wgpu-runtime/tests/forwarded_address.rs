//! A client's reported address behind a reverse proxy.
//!
//! The runtime is process-wide and reads its proxy policy from the
//! environment when it starts, so each policy needs its own test binary;
//! this one runs with 127.0.0.1 trusted, and `unconfigured_address.rs` runs
//! with nothing trusted.

mod support {
    pub mod fake_client;
}

use support::fake_client::{exclusive, runtime, FakeClient};

fn setup() -> &'static remote_wgpu_runtime::Runtime {
    // Set before the first `runtime()` call in this process.
    std::env::set_var("REMOTE_WEBGPU_TRUSTED_PROXIES", "127.0.0.0/8, ::1");
    runtime()
}

#[test]
fn a_trusted_proxys_forwarded_address_is_the_clients_address() {
    let _serial = exclusive();
    let rt = setup();
    // The chain a single nginx produces for a client that forged an entry
    // of its own: ours is the rightmost, and the only one believed.
    let _browser = FakeClient::connect_with_headers(
        rt,
        &[("x-forwarded-for", "10.0.0.9, 198.51.100.7")],
    );

    let client = rt.try_next_client().expect("a connected client is claimable");
    assert_eq!(client.ip(), Some("198.51.100.7".parse().unwrap()));
    assert_eq!(
        client.addr().forwarded_ip(),
        Some("198.51.100.7".parse().unwrap())
    );
    // The socket is still the proxy's, and logging says so.
    let peer = client.addr().peer_addr().expect("a connected client has a peer");
    assert!(peer.ip().is_loopback());
    assert_eq!(
        client.addr().to_string(),
        format!("198.51.100.7 (via proxy {peer})")
    );
}

#[test]
fn without_the_header_the_peer_is_the_client() {
    let _serial = exclusive();
    let rt = setup();
    let _browser = FakeClient::connect(rt);

    let client = rt.try_next_client().expect("a connected client is claimable");
    assert!(client.ip().expect("a connected client has an address").is_loopback());
    assert_eq!(client.addr().forwarded_ip(), None);
}
