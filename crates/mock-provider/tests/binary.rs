//! The standalone `mock-provider` binary.
//!
//! The library form serves Rust tests; this is what everything else uses — the
//! TypeScript e2e harness M1 will grow, and a person poking at a compiled graph
//! by hand. The contract it has to keep is small and worth pinning: bind, print
//! **one** JSON line naming the address, and serve on it. A harness that reads
//! that line knows the server is ready, which is what makes `--port 0` — and so
//! parallel CI runs — safe.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use mock_provider::{Client, Outcome, Request, Script};
use serde_json::{Value, json};

/// A running binary, killed when the test ends.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start the binary on an ephemeral port and read the readiness line.
fn start(arguments: &[&str]) -> (Server, Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mock-provider"))
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary under test is built");
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("the binary announces itself before serving");
    let ready: Value = serde_json::from_str(&line)
        .unwrap_or_else(|error| panic!("the readiness line is not JSON ({error}): {line:?}"));
    (Server(child), ready)
}

/// The readiness line names the address, and the server on it is the same
/// server the library exposes: scripted over the control plane, answering on the
/// provider surface.
#[test]
fn the_binary_announces_an_address_and_serves_on_it() {
    let (_server, ready) = start(&[]);
    let base_url = ready["base_url"].as_str().expect("a base url").to_string();
    assert_eq!(
        ready["address"]
            .as_str()
            .map(|address| format!("http://{address}")),
        Some(base_url.clone())
    );
    assert!(base_url.starts_with("http://127.0.0.1:"), "{base_url}");

    let client = Client::new(&base_url).expect("a client for the announced address");
    let state = client
        .get("/_mock/state")
        .expect("the control plane answers");
    assert_eq!(state.status, 200);
    assert_eq!(state.json()["requests"], 0);

    client
        .post_json(
            "/_mock/enqueue",
            &json!({
                "model": "claude-haiku-4-5",
                "outcome": { "reply": { "body": { "text": "from the binary" } } },
            }),
        )
        .expect("the control plane accepts a script");

    let answer = client
        .send(Request::post("/v1/messages").anthropic_auth().json(&json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 64,
            "messages": [{ "role": "user", "content": "go" }],
        })))
        .expect("the provider surface answers");
    assert_eq!(answer.status, 200);
    assert_eq!(answer.json()["content"][0]["text"], "from the binary");

    let transcript = client.get("/_mock/requests").expect("answers").json();
    assert_eq!(transcript["requests"].as_array().expect("a list").len(), 1);
}

/// `--port` binds where it is told, and the announced address says so.
#[test]
fn the_binary_binds_the_port_it_is_given() {
    // Ask the OS for a free port by taking one and letting it go: a fixed number
    // would collide with whatever else CI is running.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        listener.local_addr().expect("an address").port()
    };
    let port = port.to_string();
    let (_server, ready) = start(&["--port", &port]);
    assert_eq!(ready["address"], format!("127.0.0.1:{port}"));
}

/// The library handle and the binary are the same server, so a test may script
/// through either. (Also the one place the in-process API is exercised against a
/// client that resolved a URL rather than being handed an address.)
#[test]
fn a_url_client_and_the_in_process_handle_agree() {
    let provider = mock_provider::MockProvider::start().expect("a port");
    provider.enqueue(Script::new("claude-haiku-4-5", Outcome::text("in process")));

    let client = Client::new(provider.base_url()).expect("a client");
    let answer = client
        .send(Request::post("/v1/messages").anthropic_auth().json(&json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 64,
            "messages": [{ "role": "user", "content": "go" }],
        })))
        .expect("the provider surface answers");
    assert_eq!(answer.json()["content"][0]["text"], "in process");
    assert_eq!(provider.requests().len(), 1);
}
