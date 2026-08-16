//! The connection loop and the routing table.
//!
//! Seven routes and nothing else. Two are provider surfaces, three more are the
//! Azure spellings of one of them, and the rest is the control plane under
//! `/_mock/`. Anything else is a 404 that says so in the harness's own voice,
//! because a request to a path no provider serves is a codegen bug too — a base
//! URL joined wrongly reaches a real provider's 404 in production and would
//! reach silence here.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming as IncomingBody;
use hyper::service::service_fn;
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};

use crate::control::{Decision, Delay, Incoming, Script, Store, Surface, canonical};
use crate::wire::{Answer, HARNESS_STATUS, Response as Wire};
use crate::{anthropic, openai};

/// The largest request body this server will read.
///
/// A prompt is text and a tool surface is schemas; nothing a compiled graph
/// sends is close to this. The cap is here so a client that mis-frames a body
/// fails as a request rather than as memory.
const MAX_BODY: usize = 8 * 1024 * 1024;

/// Serve until `shutdown` resolves.
///
/// Every connection is its own task, so a scripted delay on one model call
/// cannot hold up another — which is what a fan-out test needs from it.
pub async fn serve(
    listener: TcpListener,
    store: Arc<Store>,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    tokio::pin!(shutdown);
    loop {
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            _ = &mut shutdown => return,
        };
        let Ok((stream, _)) = accepted else {
            // A failed accept is not this server's business to report: the test
            // that lost a connection will say so far more usefully.
            continue;
        };
        let store = Arc::clone(&store);
        tokio::spawn(async move { connection(stream, store).await });
    }
}

async fn connection(stream: TcpStream, store: Arc<Store>) {
    let service = service_fn(move |request: Request<IncomingBody>| {
        let store = Arc::clone(&store);
        async move { handle(request, store).await }
    });
    let _ = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .await;
}

/// A connection that ends without an answer: how a scripted timeout is
/// delivered, since HTTP has no status for "the provider never replied".
#[derive(Debug)]
struct Closed;

impl std::fmt::Display for Closed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the scripted outcome is a timeout: closing without a response")
    }
}

impl std::error::Error for Closed {}

async fn handle(
    request: Request<IncomingBody>,
    store: Arc<Store>,
) -> Result<hyper::Response<Full<Bytes>>, Closed> {
    let (parts, body) = request.into_parts();
    let method = parts.method.clone();
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().unwrap_or_default().to_string();
    let headers = read_headers(&parts.headers);

    let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => Bytes::new(),
    };
    if bytes.len() > MAX_BODY {
        return Ok(render(
            Wire::new(
                StatusCode::PAYLOAD_TOO_LARGE.as_u16(),
                json!({ "mock_provider": format!("request body exceeds {MAX_BODY} bytes") }),
            )
            .harness("oversized-request")
            .answer(),
        ));
    }

    let answer = route(&store, &method, &path, &query, headers, &bytes);
    let delay: Delay = match &answer {
        Answer::Close(delay) => *delay,
        Answer::Respond(response) => response.delay,
    };
    if !delay.is_none() {
        tokio::time::sleep(delay.duration()).await;
    }
    match answer {
        Answer::Close(_) => Err(Closed),
        Answer::Respond(_) => Ok(render(answer)),
    }
}

/// The routing table.
fn route(
    store: &Store,
    method: &Method,
    path: &str,
    query: &str,
    headers: BTreeMap<String, String>,
    bytes: &[u8],
) -> Answer {
    match (method.as_str(), path) {
        ("POST", "/v1/messages") => {
            provider(store, Surface::Anthropic, None, path, query, headers, bytes)
        }
        ("POST", "/v1/chat/completions") => {
            provider(store, Surface::OpenAi, None, path, query, headers, bytes)
        }
        // Azure, both spellings: the deployment sits in the path on the classic
        // route and is absent from the newer `/openai/v1` one.
        ("POST", "/openai/v1/chat/completions") => provider(
            store,
            Surface::AzureOpenAi,
            None,
            path,
            query,
            headers,
            bytes,
        ),
        ("POST", _) if deployment(path).is_some() => provider(
            store,
            Surface::AzureOpenAi,
            deployment(path),
            path,
            query,
            headers,
            bytes,
        ),
        ("POST", "/_mock/enqueue") => enqueue(store, bytes),
        ("POST", "/_mock/reset") => {
            let discarded = store.reset();
            ok(json!({ "discarded": discarded })).answer()
        }
        ("GET", "/_mock/requests") => ok(json!({ "requests": store.requests() })).answer(),
        ("GET", "/_mock/state") => ok(json!(store.snapshot())).answer(),
        _ => Wire::new(
            StatusCode::NOT_FOUND.as_u16(),
            json!({
                "mock_provider": format!(
                    "no route for {method} {path}; this server serves /v1/messages, \
                     /v1/chat/completions, the Azure chat-completions routes, and /_mock/*"
                )
            }),
        )
        .harness("unknown-route")
        .answer(),
    }
}

/// The deployment name in an Azure classic route, if this is one.
fn deployment(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/openai/deployments/")?;
    let (deployment, tail) = rest.split_once('/')?;
    (tail == "chat/completions" && !deployment.is_empty()).then_some(deployment)
}

/// Check, record, and answer one model call.
fn provider(
    store: &Store,
    surface: Surface,
    deployment: Option<&str>,
    path: &str,
    query: &str,
    headers: BTreeMap<String, String>,
    bytes: &[u8],
) -> Answer {
    let body_text = String::from_utf8_lossy(bytes).into_owned();
    let body: Option<Value> = serde_json::from_slice(bytes).ok();

    let (model, failures, tools, structured): (String, _, _, _) = match surface {
        Surface::Anthropic => {
            let parsed = anthropic::parse(&headers, body.as_ref());
            (
                parsed.model,
                parsed.failures,
                parsed.tools,
                parsed.structured_output,
            )
        }
        Surface::OpenAi | Surface::AzureOpenAi => {
            let route = if surface == Surface::AzureOpenAi {
                openai::Route::Azure
            } else {
                openai::Route::Direct
            };
            let parsed = openai::parse(route, &headers, query, deployment, body.as_ref());
            (
                parsed.model,
                parsed.failures,
                parsed.tools,
                parsed.structured_output,
            )
        }
    };

    let (decision, sequence) = store.serve(Incoming {
        surface,
        method: "POST".to_string(),
        path: path.to_string(),
        query: query.to_string(),
        headers,
        model: model.clone(),
        body: body.clone(),
        body_text,
        failures,
        tools,
        structured_output: structured.clone(),
    });

    let body = body.unwrap_or(Value::Null);
    match surface {
        Surface::Anthropic => match decision {
            Decision::Serve(outcome) => {
                anthropic::render(sequence, &body, structured.as_ref(), &outcome)
            }
            Decision::Rejected(failures) => anthropic::rejected(&failures),
            Decision::Unscripted { model, reason } => anthropic::unscripted(&model, &reason),
        },
        Surface::OpenAi | Surface::AzureOpenAi => match decision {
            Decision::Serve(outcome) => {
                openai::render(sequence, &body, &model, structured.as_ref(), &outcome)
            }
            Decision::Rejected(failures) => openai::rejected(&failures),
            Decision::Unscripted { model, reason } => openai::unscripted(&model, &reason),
        },
    }
}

/// `POST /_mock/enqueue`: one scripted outcome, or a list of them.
fn enqueue(store: &Store, bytes: &[u8]) -> Answer {
    let document: Value = match serde_json::from_slice(bytes) {
        Ok(document) => document,
        Err(error) => return control_error(format!("the request body is not JSON: {error}")),
    };
    let scripts: Vec<Script> = if document.is_array() {
        match serde_json::from_value(document) {
            Ok(scripts) => scripts,
            Err(error) => return control_error(format!("not a list of scripts: {error}")),
        }
    } else {
        match serde_json::from_value::<Script>(document) {
            Ok(script) => vec![script],
            Err(error) => return control_error(format!("not a script: {error}")),
        }
    };
    let queued = scripts.len();
    for script in scripts {
        store.enqueue(script);
    }
    ok(json!({ "queued": queued, "state": store.snapshot() })).answer()
}

/// A control-plane request the harness could not understand. Loud, and never a
/// provider shape: nothing on `/_mock/` is a provider talking.
fn control_error(reason: String) -> Answer {
    Wire::new(HARNESS_STATUS, json!({ "mock_provider": reason }))
        .harness("bad-control-request")
        .answer()
}

fn ok(body: Value) -> Wire {
    Wire::new(StatusCode::OK.as_u16(), body)
}

/// Request headers, lowercased and sorted, with repeats joined the way HTTP
/// defines them.
fn read_headers(headers: &hyper::HeaderMap) -> BTreeMap<String, String> {
    let mut read: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let value = value.to_str().unwrap_or_default().to_string();
        read.entry(name.as_str().to_ascii_lowercase())
            .and_modify(|existing| {
                existing.push_str(", ");
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    read
}

/// Turn an answer into bytes.
fn render(answer: Answer) -> hyper::Response<Full<Bytes>> {
    let Answer::Respond(response) = answer else {
        unreachable!("a closed connection never renders");
    };
    let body = canonical(&response.body);
    let mut built = hyper::Response::builder()
        .status(StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header("content-type", "application/json");
    for (name, value) in &response.headers {
        built = built.header(name, value);
    }
    built
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| unreachable!("every header this server sets is well formed"))
}

/// The address a bound listener is on, for the handle and the binary.
pub(crate) fn address(listener: &TcpListener) -> std::io::Result<SocketAddr> {
    listener.local_addr()
}

/// A scripted delay, for the connection loop's own tests.
#[cfg(test)]
pub(crate) fn delay_of(answer: &Answer) -> Delay {
    match answer {
        Answer::Close(delay) => *delay,
        Answer::Respond(response) => response.delay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{HARNESS_HEADER, Outcome, Store};

    fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect()
    }

    fn anthropic_headers() -> BTreeMap<String, String> {
        headers(&[
            ("x-api-key", "test"),
            ("anthropic-version", "2023-06-01"),
            ("content-type", "application/json"),
        ])
    }

    /// The Azure classic route yields its deployment; near misses do not.
    #[test]
    fn the_azure_route_is_recognized_by_its_shape() {
        assert_eq!(
            deployment("/openai/deployments/smart/chat/completions"),
            Some("smart")
        );
        assert_eq!(
            deployment("/openai/deployments/smart-4o/chat/completions"),
            Some("smart-4o")
        );
        assert_eq!(deployment("/openai/deployments//chat/completions"), None);
        assert_eq!(deployment("/openai/deployments/smart/embeddings"), None);
        assert_eq!(deployment("/v1/chat/completions"), None);
    }

    /// An unrouted path is a 404 in the harness's voice, not silence.
    #[test]
    fn an_unknown_route_says_what_it_serves() {
        let store = Store::new();
        let answer = route(
            &store,
            &Method::POST,
            "/v1/complete",
            "",
            BTreeMap::new(),
            b"{}",
        );
        let Answer::Respond(response) = answer else {
            panic!("a 404 is a response");
        };
        assert_eq!(response.status, 404);
        assert_eq!(response.headers[HARNESS_HEADER], "unknown-route");
        assert!(
            response.body["mock_provider"]
                .as_str()
                .unwrap()
                .contains("/v1/messages")
        );
        assert!(
            store.requests().is_empty(),
            "a request to no provider surface is not a model call"
        );
    }

    /// A model call is recorded once, with its parsed body and its verdict, and
    /// the answer it draws comes from the model's own queue.
    #[test]
    fn a_model_call_is_checked_recorded_and_answered() {
        let store = Store::new();
        store.enqueue(Script::new("claude-haiku-4-5", Outcome::text("the answer")));
        let body = json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 32,
            "messages": [{ "role": "user", "content": "hi" }],
        });
        let answer = route(
            &store,
            &Method::POST,
            "/v1/messages",
            "",
            anthropic_headers(),
            canonical(&body).as_bytes(),
        );
        let Answer::Respond(response) = answer else {
            panic!("a reply is a response");
        };
        assert_eq!(response.status, 200);
        assert_eq!(response.body["content"][0]["text"], "the answer");

        let recorded = store.requests();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].sequence, 1);
        assert_eq!(recorded[0].surface, Surface::Anthropic);
        assert_eq!(recorded[0].model, "claude-haiku-4-5");
        assert!(recorded[0].is_valid());
        assert_eq!(recorded[0].served, "reply.text");
        assert_eq!(recorded[0].body()["max_tokens"], 32);
        assert_eq!(recorded[0].headers["x-api-key"], "test");
    }

    /// A malformed request is recorded with its verdict and **consumes no
    /// outcome**: one codegen bug must not cascade into the next call's answer.
    #[test]
    fn a_malformed_request_consumes_no_outcome() {
        let store = Store::new();
        store.enqueue(Script::new("claude-haiku-4-5", Outcome::text("the answer")));
        let body = json!({
            "model": "claude-haiku-4-5",
            "messages": [{ "role": "user", "content": "hi" }],
        });
        let answer = route(
            &store,
            &Method::POST,
            "/v1/messages",
            "",
            anthropic_headers(),
            canonical(&body).as_bytes(),
        );
        let Answer::Respond(response) = answer else {
            panic!("a rejection is a response");
        };
        assert_eq!(response.status, 400);

        let recorded = store.requests();
        assert_eq!(recorded[0].served, "rejected");
        assert_eq!(
            recorded[0]
                .failures()
                .iter()
                .map(|failure| failure.pointer.as_str())
                .collect::<Vec<_>>(),
            ["max_tokens"]
        );
        assert_eq!(
            store.snapshot().queues["claude-haiku-4-5"],
            1,
            "the scripted answer is still there for the call that was supposed to get it"
        );
    }

    /// A body that is not JSON at all still lands in the transcript, with the
    /// bytes that arrived.
    #[test]
    fn an_unparseable_body_is_still_recorded() {
        let store = Store::new();
        let answer = route(
            &store,
            &Method::POST,
            "/v1/messages",
            "",
            anthropic_headers(),
            b"{not json",
        );
        let Answer::Respond(response) = answer else {
            panic!("a rejection is a response");
        };
        assert_eq!(response.status, 400);
        let recorded = store.requests();
        assert!(recorded[0].body.is_none());
        assert_eq!(recorded[0].body_text, "{not json");
        assert!(!recorded[0].is_valid());
    }

    /// The control plane: enqueue, read the transcript, reset.
    #[test]
    fn the_control_plane_scripts_reads_and_resets() {
        let store = Store::new();
        let script = json!({
            "model": "gpt-4o-mini",
            "outcome": { "reply": { "body": { "text": "scripted over http" } } },
            "times": 2,
        });
        let Answer::Respond(response) = enqueue(&store, canonical(&script).as_bytes()) else {
            panic!("enqueue answers");
        };
        assert_eq!(response.status, 200);
        assert_eq!(response.body["queued"], 1);
        assert_eq!(response.body["state"]["queues"]["gpt-4o-mini"], 2);

        // A list is accepted too, which is how a whole run is staged at once.
        let batch = json!([
            { "model": "a", "outcome": { "failure": "overloaded" } },
            { "model": "b", "outcome": { "raw": { "status": 402, "body": {} } } },
        ]);
        let Answer::Respond(response) = enqueue(&store, canonical(&batch).as_bytes()) else {
            panic!("enqueue answers");
        };
        assert_eq!(response.body["queued"], 2);

        let discarded = store.reset();
        assert_eq!(discarded.queues["gpt-4o-mini"], 2);
        assert!(store.snapshot().is_drained());
    }

    /// A typo in a scripted outcome is refused with the serde error naming it,
    /// rather than accepted as a queue entry that answers nothing.
    #[test]
    fn a_malformed_script_is_refused_by_name() {
        let store = Store::new();
        let script = json!({ "model": "a", "outcome": { "reply": { "body": { "txet": "hi" } } } });
        let Answer::Respond(response) = enqueue(&store, canonical(&script).as_bytes()) else {
            panic!("enqueue answers");
        };
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[HARNESS_HEADER], "bad-control-request");
        assert!(
            response.body["mock_provider"]
                .as_str()
                .unwrap()
                .contains("txet"),
            "{:?}",
            response.body
        );
        assert!(store.snapshot().is_drained(), "nothing was queued");
    }

    /// A scripted timeout is delivered as no answer at all, after its wait.
    #[test]
    fn a_scripted_timeout_closes_instead_of_answering() {
        let store = Store::new();
        store.enqueue(Script::new(
            "claude-haiku-4-5",
            Outcome::timeout(std::time::Duration::from_millis(20)),
        ));
        let body = json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 32,
            "messages": [{ "role": "user", "content": "hi" }],
        });
        let answer = route(
            &store,
            &Method::POST,
            "/v1/messages",
            "",
            anthropic_headers(),
            canonical(&body).as_bytes(),
        );
        assert!(matches!(answer, Answer::Close(_)));
        assert_eq!(delay_of(&answer).duration().as_millis(), 20);
        assert_eq!(store.requests()[0].served, "failure.timeout");
    }

    /// Repeated headers arrive joined, and every name is lowercase, so a
    /// transcript assertion does not depend on how a client cased them.
    #[test]
    fn headers_are_normalized() {
        let mut map = hyper::HeaderMap::new();
        map.append("X-Api-Key", "one".parse().unwrap());
        map.append("accept", "a".parse().unwrap());
        map.append("accept", "b".parse().unwrap());
        let read = read_headers(&map);
        assert_eq!(read["x-api-key"], "one");
        assert_eq!(read["accept"], "a, b");
    }
}
