//! A client that disconnects must be released once the application lets
//! go of it: nothing in the runtime may keep it alive.

mod support {
    pub mod fake_client;
}

use std::sync::Arc;

use remote_wgpu_runtime::{ClientEvent, HasRemoteClient};
use support::fake_client::{exclusive, runtime, wait_for, FakeClient};

#[test]
fn disconnected_client_is_released_when_the_application_lets_go() {
    let _serial = exclusive();
    let rt = runtime();
    let browser = FakeClient::connect(rt);

    // The application claims the client, as a window would.
    let client = rt.try_next_client().expect("a connected client is claimable");
    assert!(!client.is_disconnected());
    assert!(rt.clients().iter().any(|c| Arc::ptr_eq(c, &client)));
    // `&Client` can be re-wrapped without the registry.
    assert!(Arc::ptr_eq(&client.remote_client(), &client));
    let weak = Arc::downgrade(&client);

    browser.disconnect();
    wait_for(|| client.is_disconnected(), "the disconnect to be noticed");
    wait_for(
        || !rt.clients().iter().any(|c| Arc::ptr_eq(c, &client)),
        "the registry to drop the client",
    );
    assert!(matches!(client.poll_event(), Some(ClientEvent::Disconnected)));
    assert!(rt.try_next_client().is_none(), "a dead client must not be claimable");

    // The application was the last owner.
    drop(client);
    wait_for(|| weak.upgrade().is_none(), "the client to be freed");
}

#[test]
fn unclaimed_client_is_released_on_disconnect() {
    let _serial = exclusive();
    let rt = runtime();
    let browser = FakeClient::connect(rt);
    let weak = {
        let listed = rt.clients();
        Arc::downgrade(listed.last().expect("the new client is listed"))
    };
    // Nobody claims it; the connection is its only owner.
    browser.disconnect();
    wait_for(|| weak.upgrade().is_none(), "the unclaimed client to be freed");
    assert!(rt.try_next_client().is_none());
}

#[test]
fn a_peer_that_hangs_up_mid_handshake_leaves_nothing_behind() {
    let _serial = exclusive();
    let rt = runtime();
    let before = rt.clients().len();
    let stream = std::net::TcpStream::connect(("127.0.0.1", rt.port())).unwrap();
    let (ws, _) =
        tungstenite::client(format!("ws://127.0.0.1:{}/", rt.port()), stream).unwrap();
    // No hello; just go away.
    let _ = ws.get_ref().shutdown(std::net::Shutdown::Both);
    drop(ws);
    // The next honest client still gets through, and the registry only
    // ever saw it.
    let browser = FakeClient::connect(rt);
    assert_eq!(rt.clients().len(), before + 1);
    let client = rt.try_next_client().unwrap();
    let weak = Arc::downgrade(&client);
    browser.disconnect();
    drop(client);
    wait_for(|| weak.upgrade().is_none(), "the client to be freed");
}
