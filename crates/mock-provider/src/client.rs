//! A blocking HTTP client for the harness.
//!
//! It exists for one reason: the acceptance suite has to be able to send *the
//! request a compiled graph will send*, byte for byte, including the malformed
//! ones. Driving the control plane in process would be simpler and would prove
//! less — the wire is the thing under test, and a test that reached past it
//! could not tell a server that refuses a bad request from one that never saw
//! it.
//!
//! Blocking, because the acceptance suite is blocking: it shells out to
//! `agent-compose build` and to `node`, and an async test macro over that work
//! would buy nothing. Every call runs on a `tokio` runtime — the server's own
//! when the client came from a [`MockProvider`](crate::MockProvider), its own
//! otherwise — so it must not be called from inside a runtime thread.

use std::collections::BTreeMap;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::runtime::{Handle, Runtime};

/// How long a call waits for an answer before giving up.
///
/// A scripted timeout is delivered as silence followed by a close, so a client
/// with no budget of its own would wait for the close; this is the budget that
/// makes "the provider timed out" observable as a timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// One request, built by hand.
#[derive(Clone, Debug)]
pub struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    /// A request with no headers and no body.
    #[must_use]
    pub fn new(method: &str, path: impl Into<String>) -> Self {
        Self {
            method: method.to_string(),
            path: path.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// `POST <path>`.
    #[must_use]
    pub fn post(path: impl Into<String>) -> Self {
        Self::new("POST", path)
    }

    /// `GET <path>`.
    #[must_use]
    pub fn get(path: impl Into<String>) -> Self {
        Self::new("GET", path)
    }

    /// Add a header. Repeats are sent as repeats.
    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    /// A JSON body, with the content type that goes with it.
    #[must_use]
    pub fn json(self, body: &Value) -> Self {
        let text = serde_json::to_vec(body).unwrap_or_default();
        self.header("content-type", "application/json").bytes(text)
    }

    /// A body of exactly these bytes, whatever they are. The way a test sends
    /// something no serializer would produce.
    #[must_use]
    pub fn bytes(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// The headers an Anthropic client sends, with a placeholder key.
    ///
    /// The harness needs no API keys (PRD §7 M1), but the *shape* of a
    /// request still includes its auth headers, and this server checks them.
    #[must_use]
    pub fn anthropic_auth(self) -> Self {
        self.header("x-api-key", "mock-provider-key")
            .header("anthropic-version", "2023-06-01")
    }

    /// The header an OpenAI client sends, with a placeholder key.
    #[must_use]
    pub fn openai_auth(self) -> Self {
        self.header("authorization", "Bearer mock-provider-key")
    }

    /// The header an Azure OpenAI client sends, with a placeholder key.
    #[must_use]
    pub fn azure_auth(self) -> Self {
        self.header("api-key", "mock-provider-key")
    }
}

/// One answer.
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Response {
    /// The body, parsed as JSON.
    ///
    /// # Panics
    ///
    /// If the body is not JSON. Every answer this server sends is, so a panic
    /// here is the server having gone wrong rather than the test.
    #[must_use]
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            panic!(
                "the mock provider answered with something that is not JSON ({error}): {}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    /// The body as text.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// One response header, lowercased.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// Where a client's runtime comes from.
#[derive(Clone)]
enum Reactor {
    /// The client made it and owns it.
    Owned(Arc<Runtime>),
    /// The server it talks to owns it.
    Borrowed(Handle),
}

impl Reactor {
    /// Run one exchange to completion.
    ///
    /// The two arms are not interchangeable: on a **current-thread** runtime
    /// only `Runtime::block_on` drives the I/O driver — `Handle::block_on` runs
    /// the future and leaves its sockets unpolled, which is a hang rather than
    /// an error. A borrowed handle is always the server's multi-threaded
    /// runtime, whose workers drive I/O for it.
    fn block_on<Future: std::future::Future>(&self, future: Future) -> Future::Output {
        match self {
            Self::Owned(runtime) => runtime.block_on(future),
            Self::Borrowed(handle) => handle.block_on(future),
        }
    }
}

/// A blocking HTTP client bound to one address.
#[derive(Clone)]
pub struct Client {
    address: SocketAddr,
    base_url: String,
    timeout: Duration,
    reactor: Reactor,
}

impl Client {
    /// A client for `base_url`, with its own runtime.
    ///
    /// The form the future TypeScript harness's Rust-side equivalent takes: a
    /// standalone `mock-provider` binary is reached this way.
    ///
    /// # Errors
    ///
    /// If the URL has no resolvable `host:port`, or a runtime cannot be built.
    pub fn new(base_url: impl Into<String>) -> io::Result<Self> {
        let base_url = base_url.into();
        let address = resolve(&base_url)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()?;
        Ok(Self {
            address,
            base_url,
            timeout: DEFAULT_TIMEOUT,
            reactor: Reactor::Owned(Arc::new(runtime)),
        })
    }

    /// A client sharing a running server's runtime.
    pub(crate) fn attached(address: SocketAddr, handle: Handle) -> Self {
        Self {
            address,
            base_url: format!("http://{address}"),
            timeout: DEFAULT_TIMEOUT,
            reactor: Reactor::Borrowed(handle),
        }
    }

    /// Wait this long for an answer instead of [`DEFAULT_TIMEOUT`].
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The base URL this client is bound to.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Send one request.
    ///
    /// # Errors
    ///
    /// If the connection fails, the answer is malformed, or the client's own
    /// budget runs out — which is what a scripted provider timeout looks like
    /// from here.
    pub fn send(&self, request: Request) -> io::Result<Response> {
        let address = self.address;
        let timeout = self.timeout;
        self.reactor
            .block_on(async move { exchange(address, request, timeout).await })
    }

    /// `POST <path>` with a JSON body.
    ///
    /// # Errors
    ///
    /// As [`Client::send`].
    pub fn post_json(&self, path: &str, body: &Value) -> io::Result<Response> {
        self.send(Request::post(path).json(body))
    }

    /// `GET <path>`.
    ///
    /// # Errors
    ///
    /// As [`Client::send`].
    pub fn get(&self, path: &str) -> io::Result<Response> {
        self.send(Request::get(path))
    }
}

async fn exchange(address: SocketAddr, request: Request, budget: Duration) -> io::Result<Response> {
    let exchange = async {
        let stream = TcpStream::connect(address).await?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(io::Error::other)?;
        // The connection task drives the socket while the response is read; it
        // ends when the response is done, or when the server closes on a
        // scripted timeout.
        let driver = tokio::spawn(connection);

        let mut built = hyper::Request::builder()
            .method(request.method.as_str())
            .uri(request.path.as_str())
            .header("host", address.to_string());
        for (name, value) in &request.headers {
            built = built.header(name, value);
        }
        let built = built
            .body(Full::new(Bytes::from(request.body)))
            .map_err(io::Error::other)?;

        let response = sender.send_request(built).await.map_err(io::Error::other)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        let body = response
            .into_body()
            .collect()
            .await
            .map_err(io::Error::other)?
            .to_bytes()
            .to_vec();
        driver.abort();
        Ok(Response {
            status,
            headers,
            body,
        })
    };

    match tokio::time::timeout(budget, exchange).await {
        Ok(result) => result,
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("no answer from the mock provider within {budget:?}"),
        )),
    }
}

/// The socket address behind an `http://host:port` URL.
fn resolve(base_url: &str) -> io::Result<SocketAddr> {
    let authority = base_url
        .strip_prefix("http://")
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("`{base_url}` is not an http:// URL; the mock provider serves plain HTTP"),
            )
        })?
        .split('/')
        .next()
        .unwrap_or_default();
    authority.to_socket_addrs()?.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("`{authority}` resolves to no address"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A URL is a socket address, or it is a mistake with a message.
    #[test]
    fn a_base_url_resolves_to_an_address() {
        assert_eq!(
            resolve("http://127.0.0.1:8123").expect("a loopback address"),
            SocketAddr::from(([127, 0, 0, 1], 8123))
        );
        assert_eq!(
            resolve("http://127.0.0.1:8123/_mock/state").expect("the path is ignored"),
            SocketAddr::from(([127, 0, 0, 1], 8123))
        );
        let error = resolve("https://127.0.0.1:8123").expect_err("TLS is not served");
        assert!(error.to_string().contains("plain HTTP"), "{error}");
    }

    /// A JSON body brings its content type with it, because a request without
    /// one is refused by both surfaces.
    #[test]
    fn a_json_body_carries_its_content_type() {
        let request = Request::post("/v1/messages").json(&serde_json::json!({ "a": 1 }));
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == "content-type" && value == "application/json")
        );
        assert_eq!(request.body, b"{\"a\":1}");
    }

    /// The auth helpers send what each client library sends, with a placeholder
    /// where the key would be — no API keys anywhere (PRD §7 M1).
    #[test]
    fn the_auth_helpers_carry_placeholder_credentials() {
        let anthropic = Request::post("/v1/messages").anthropic_auth();
        assert_eq!(
            anthropic
                .headers
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["x-api-key", "anthropic-version"]
        );
        let openai = Request::post("/v1/chat/completions").openai_auth();
        assert_eq!(openai.headers[0].1, "Bearer mock-provider-key");
        let azure = Request::post("/openai/v1/chat/completions").azure_auth();
        assert_eq!(azure.headers[0].0, "api-key");
    }
}
