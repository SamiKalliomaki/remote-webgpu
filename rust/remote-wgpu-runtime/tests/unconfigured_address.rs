//! Without `REMOTE_WEBGPU_TRUSTED_PROXIES`, a forwarding header is just
//! text a client made up: it must not change the address the runtime
//! reports.  Its counterpart with a configured proxy is
//! `forwarded_address.rs`.

mod support {
    pub mod fake_client;
}

use support::fake_client::{exclusive, runtime, FakeClient};

#[test]
fn an_unconfigured_runtime_ignores_a_forged_forwarding_header() {
    let _serial = exclusive();
    std::env::remove_var("REMOTE_WEBGPU_TRUSTED_PROXIES");
    let rt = runtime();
    let _browser = FakeClient::connect_with_headers(
        rt,
        &[("x-forwarded-for", "198.51.100.7"), ("x-real-ip", "198.51.100.7")],
    );

    let client = rt.try_next_client().expect("a connected client is claimable");
    assert_eq!(client.addr().forwarded_ip(), None);
    assert!(client.ip().expect("a connected client has an address").is_loopback());
}
