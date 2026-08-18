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
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::Incoming as IncomingBody;
use hyper::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use hyper::service::service_fn;
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};

use crate::control::{
    Decision, Delay, HARNESS_HEADER, Incoming, Outcome, Script, Store, Surface, canonical,
};
use crate::wire::{Answer, HARNESS_STATUS, MISMATCH, Response as Wire, UNSENDABLE};
use crate::{anthropic, openai};

/// The largest request body this server will read.
///
/// A prompt is text and a tool surface is schemas; nothing a compiled graph
/// sends is close to this. The cap is here so a client that mis-frames a body
/// fails as a request rather than as memory — which is why it is applied by
/// [`Limited`] *while* the body streams in rather than to the length of an
/// already-buffered one: a cap checked after the fact bounds nothing, and a
/// client advertising two gigabytes would have had them allocated before the
/// check could refuse it.
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

    let bytes = match Limited::new(body, MAX_BODY).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(error) if error.downcast_ref::<LengthLimitError>().is_some() => {
            return Ok(render(
                Wire::new(
                    StatusCode::PAYLOAD_TOO_LARGE.as_u16(),
                    json!({ "mock_provider": format!("request body exceeds {MAX_BODY} bytes") }),
                )
                .harness("oversized-request")
                .answer(),
            ));
        }
        // Any other read error is a connection that ended mid-body. There is
        // nothing to record but the fact that nothing arrived, and the surfaces
        // already answer an empty body with "could not parse".
        Err(_) => Bytes::new(),
    };

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
    let arriving = Arriving {
        method: method.as_str(),
        path,
        query,
        headers,
        bytes,
    };
    match (method.as_str(), path) {
        ("POST", "/v1/messages") => provider(store, Reached::Anthropic, arriving),
        ("POST", "/v1/chat/completions") => provider(
            store,
            Reached::ChatCompletions {
                route: openai::Route::Direct,
                deployment: None,
            },
            arriving,
        ),
        // Azure, both spellings: the deployment sits in the path on the classic
        // route and is absent from the newer `/openai/v1` one, which also drops
        // the `api-version` requirement (WIRE-NOTES §7) — which is why the two
        // are told apart here rather than merged into one Azure route.
        ("POST", "/openai/v1/chat/completions") => provider(
            store,
            Reached::ChatCompletions {
                route: openai::Route::AzureV1,
                deployment: None,
            },
            arriving,
        ),
        ("POST", _) if deployment(path).is_some() => provider(
            store,
            Reached::ChatCompletions {
                route: openai::Route::Azure,
                deployment: deployment(path),
            },
            arriving,
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

/// One arriving request, before a surface has looked at it.
///
/// The four things a transcript records about a call's *identity*, kept together
/// so the routing table can hand them on without either spelling them out at
/// every call site or hard-coding what it already knows (a method it matched on
/// is a method it should be passing, not asserting).
struct Arriving<'a> {
    method: &'a str,
    path: &'a str,
    query: &'a str,
    headers: BTreeMap<String, String>,
    bytes: &'a [u8],
}

/// Which surface a route matched, and what that surface needs to know about how
/// it was reached.
///
/// The route is carried rather than re-derived: Chat Completions is reached at
/// three routes that differ in auth header, in whether the path names the model,
/// and in whether `api-version` is required, and a `provider` that re-read the
/// path to work out which would be asserting what the routing table already
/// matched on.
enum Reached<'a> {
    /// The Messages API.
    Anthropic,
    /// Chat Completions, at one of its three routes.
    ChatCompletions {
        route: openai::Route,
        /// The Azure deployment in the path, on the one route that has one.
        deployment: Option<&'a str>,
    },
}

impl Reached<'_> {
    /// Which surface a transcript records this call on.
    fn surface(&self) -> Surface {
        match self {
            Self::Anthropic => Surface::Anthropic,
            Self::ChatCompletions {
                route: openai::Route::Direct,
                ..
            } => Surface::OpenAi,
            Self::ChatCompletions { .. } => Surface::AzureOpenAi,
        }
    }
}

/// Check, record, and answer one model call.
fn provider(store: &Store, reached: Reached<'_>, arriving: Arriving<'_>) -> Answer {
    let Arriving {
        method,
        path,
        query,
        headers,
        bytes,
    } = arriving;
    let body_text = String::from_utf8_lossy(bytes).into_owned();
    let body: Option<Value> = serde_json::from_slice(bytes).ok();
    let surface = reached.surface();

    let (model, failures, tools, structured): (String, _, _, _) = match reached {
        Reached::Anthropic => {
            let parsed = anthropic::parse(&headers, body.as_ref());
            (
                parsed.model,
                parsed.failures,
                parsed.tools,
                parsed.structured_output,
            )
        }
        Reached::ChatCompletions { route, deployment } => {
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
        method: method.to_string(),
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
                let answer = anthropic::render(sequence, &body, structured.as_ref(), &outcome);
                recorded(store, sequence, &outcome, answer)
            }
            Decision::Rejected(failures) => anthropic::rejected(sequence, &failures),
            Decision::Unscripted { model, reason } => {
                anthropic::unscripted(sequence, &model, &reason)
            }
        },
        Surface::OpenAi | Surface::AzureOpenAi => match decision {
            Decision::Serve(outcome) => {
                let answer = openai::render(sequence, &body, &model, structured.as_ref(), &outcome);
                recorded(store, sequence, &outcome, answer)
            }
            Decision::Rejected(failures) => openai::rejected(sequence, &failures),
            Decision::Unscripted { model, reason } => openai::unscripted(sequence, &model, &reason),
        },
    }
}

/// Hand the answer on, correcting the transcript first if the outcome the store
/// handed over never made it onto the wire.
fn recorded(store: &Store, sequence: u64, outcome: &Outcome, answer: Answer) -> Answer {
    if let Some(refusal) = refusal(outcome, &answer) {
        store.refused(sequence, refusal);
    }
    answer
}

/// The refusal a rendered answer carries, for an outcome that had already been
/// taken from its queue.
///
/// Both refusals here are decided *after* [`Store::serve`] has written the
/// transcript entry — the surface refuses an outcome it cannot render into what
/// the request asked for, and [`render`] refuses one that cannot be put on the
/// wire — and both are read back off the answer rather than plumbed out of the
/// surfaces, because in each case the refusal **is** the answer: it is the 422
/// the client receives, header and all.
///
/// A `raw` outcome is exempt from the header half. Its headers are the script's
/// own, verbatim, so a script that writes `x-mock-provider-error` is staging a
/// response for generated code to reject — not a refusal by this server.
fn refusal(outcome: &Outcome, answer: &Answer) -> Option<&'static str> {
    let Answer::Respond(response) = answer else {
        return None;
    };
    if sendable(response.status, &response.headers).is_err() {
        return Some(UNSENDABLE);
    }
    let refused = response.headers.get(HARNESS_HEADER).map(String::as_str) == Some(MISMATCH);
    (refused && !matches!(outcome, Outcome::Raw(_))).then_some(MISMATCH)
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
///
/// Total, deliberately. The one part of a response this server does not write
/// itself is a scripted `raw` outcome's **status and header map**, and a test
/// can write either one so that HTTP cannot carry it (see
/// [`crate::wire::header`]). Both would otherwise fail quietly and in the worst
/// possible direction:
///
/// * a header hyper cannot build ends the connection with no answer, which PRD
///   5.9 classifies as a provider **timeout**;
/// * a status outside 100..=999 has no `StatusCode`, and the obvious fallback is
///   **500** — a failover condition *and* a status both SDKs retry twice.
///
/// So each is checked here and answered as the harness bug it is, in the same
/// voice as an unscripted call. The queue entry is spent either way — the take
/// happened before the answer was built — but the transcript records the
/// refusal rather than the outcome ([`refusal`] and [`Store::refused`]), so a
/// test cannot read `reply.text` off a call that received a 422 and no reply.
/// The control plane refuses both when the script is enqueued; this is the
/// second gate, the one a struct literal filling the public fields still has to
/// pass.
fn render(answer: Answer) -> hyper::Response<Full<Bytes>> {
    let Answer::Respond(response) = answer else {
        unreachable!("a closed connection never renders");
    };
    match sendable(response.status, &response.headers) {
        Ok((status, headers)) => assemble(status, headers, &response.body),
        Err(reason) => assemble(
            // `from_u16` cannot fail on this crate's own refusal status, and
            // `from_static` cannot fail on the two constants below: both are
            // lowercase, printable ASCII.
            StatusCode::from_u16(HARNESS_STATUS).expect("422 is a status"),
            vec![(
                HeaderName::from_static(HARNESS_HEADER),
                HeaderValue::from_static(UNSENDABLE),
            )],
            &json!({
                "mock_provider": format!("the scripted response cannot be sent: {reason}"),
            }),
        ),
    }
}

/// A response's status and headers, parsed — or the first part of it that
/// cannot be sent.
fn sendable(
    status: u16,
    headers: &BTreeMap<String, String>,
) -> Result<(StatusCode, Vec<(HeaderName, HeaderValue)>), String> {
    let status = StatusCode::from_u16(status)
        .map_err(|_| format!("`{status}` is not a status HTTP can carry (100..=999)"))?;
    let headers = headers
        .iter()
        .map(|(name, value)| crate::wire::header(name, value))
        .collect::<Result<_, _>>()?;
    Ok((status, headers))
}

/// Status, headers, and body into the response that goes on the wire.
fn assemble(
    status: StatusCode,
    headers: Vec<(HeaderName, HeaderValue)>,
    body: &Value,
) -> hyper::Response<Full<Bytes>> {
    let mut built = hyper::Response::new(Full::new(Bytes::from(canonical(body))));
    *built.status_mut() = status;
    let map = built.headers_mut();
    map.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    // After `content-type`, so a script that means to override it can.
    for (name, value) in headers {
        map.insert(name, value);
    }
    built
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
    use crate::control::{Outcome, Store};

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

    /// A scripted `raw` outcome whose headers cannot be sent is answered as a
    /// harness bug rather than closing the connection — which is what building
    /// the response would have done, and what PRD 5.9 reads as a timeout.
    ///
    /// The control plane refuses such a script by name, but `RawOutcome.headers`
    /// is a public field, so this is the second gate: the one a struct literal
    /// still has to pass.
    #[test]
    fn a_header_that_cannot_be_sent_is_refused_rather_than_dropping_the_connection() {
        for (name, value) in [("bad header", "value"), ("x-ok", "a\nb")] {
            let rendered = render(
                Wire::new(200, json!({ "ok": true }))
                    .header(name, value)
                    .answer(),
            );
            assert_eq!(
                rendered.status().as_u16(),
                HARNESS_STATUS,
                "`{name}: {value}` is not sendable"
            );
            assert_eq!(rendered.headers()[HARNESS_HEADER], UNSENDABLE);
        }

        // …and a header that *can* be sent still is, over the content type this
        // server sets first.
        let rendered = render(
            Wire::new(402, json!({ "nope": true }))
                .header("retry-after", "1")
                .answer(),
        );
        assert_eq!(rendered.status().as_u16(), 402);
        assert_eq!(rendered.headers()["retry-after"], "1");
        assert_eq!(rendered.headers()[CONTENT_TYPE], "application/json");
    }

    /// The same gate on the other half of a `raw` outcome: a status HTTP cannot
    /// carry is answered as the harness bug it is, rather than falling back to
    /// **500** — which PRD 5.9 fails over on and both SDKs retry twice, so a
    /// typo'd status would stage a provider failure the script never asked for.
    #[test]
    fn a_status_that_cannot_be_sent_is_refused_rather_than_answered_as_a_server_error() {
        for status in [0, 42, 99, 1000] {
            let rendered = render(Wire::new(status, json!({ "nope": true })).answer());
            assert_eq!(
                rendered.status().as_u16(),
                HARNESS_STATUS,
                "`{status}` is not a status HTTP can carry"
            );
            assert_eq!(rendered.headers()[HARNESS_HEADER], UNSENDABLE);
        }

        // …and every status it *can* carry is still served untouched, including
        // the ones a `raw` outcome exists to stage.
        for status in [100, 200, 402, 418, 500, 599, 999] {
            let rendered = render(Wire::new(status, json!({ "nope": true })).answer());
            assert_eq!(rendered.status().as_u16(), status);
            assert!(rendered.headers().get(HARNESS_HEADER).is_none());
        }
    }

    /// The same check, at the control plane: a script carrying a header that
    /// cannot be sent is refused by name instead of queued.
    #[test]
    fn a_script_whose_headers_cannot_be_sent_is_refused_by_name() {
        let store = Store::new();
        let script = json!({
            "model": "a",
            "outcome": { "raw": { "status": 200, "body": {}, "headers": { "bad header": "v" } } },
        });
        let Answer::Respond(response) = enqueue(&store, canonical(&script).as_bytes()) else {
            panic!("enqueue answers");
        };
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[HARNESS_HEADER], "bad-control-request");
        assert!(
            response.body["mock_provider"]
                .as_str()
                .unwrap()
                .contains("bad header"),
            "{:?}",
            response.body
        );
        assert!(store.snapshot().is_drained(), "nothing was queued");

        // A sendable one is queued, so the check narrows nothing legitimate.
        let script = json!({
            "model": "a",
            "outcome": { "raw": { "status": 402, "body": {}, "headers": { "retry-after": "1" } } },
        });
        let Answer::Respond(response) = enqueue(&store, canonical(&script).as_bytes()) else {
            panic!("enqueue answers");
        };
        assert_eq!(response.status, 200);
        assert_eq!(store.snapshot().queues["a"], 1);
    }

    /// And a script whose **status** cannot be sent is refused by name too,
    /// rather than queued to be answered as a 500 later.
    #[test]
    fn a_script_whose_status_cannot_be_sent_is_refused_by_name() {
        let store = Store::new();
        let script = json!({ "model": "a", "outcome": { "raw": { "status": 42, "body": {} } } });
        let Answer::Respond(response) = enqueue(&store, canonical(&script).as_bytes()) else {
            panic!("enqueue answers");
        };
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[HARNESS_HEADER], "bad-control-request");
        assert!(
            response.body["mock_provider"]
                .as_str()
                .unwrap()
                .contains("42"),
            "{:?}",
            response.body
        );
        assert!(store.snapshot().is_drained(), "nothing was queued");
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
