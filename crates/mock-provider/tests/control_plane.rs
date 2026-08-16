//! The control plane over HTTP, and the determinism guarantees the acceptance
//! suite rests on.
//!
//! Everything here is what a *harness* needs rather than what a provider does:
//! staging a run, reading the transcript back, resetting between cases, and the
//! two properties that make a scripted run an assertion — matched FIFO under
//! concurrency, and scripted delays as the only clock.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mock_provider::{Client, MockProvider, Outcome, Request, Script};
use serde_json::{Value, json};

const MODEL: &str = "claude-haiku-4-5";

fn call(client: &Client, marker: &str) -> mock_provider::Response {
    client
        .send(Request::post("/v1/messages").anthropic_auth().json(&json!({
            "model": MODEL,
            "max_tokens": 256,
            "messages": [{ "role": "user", "content": marker }],
        })))
        .expect("the mock provider answers")
}

fn text(response: &mock_provider::Response) -> String {
    response.json()["content"][0]["text"]
        .as_str()
        .expect("a text block")
        .to_string()
}

/// A whole run staged over HTTP, read back over HTTP, and cleared over HTTP —
/// the surface a TypeScript harness will use, with no Rust API involved.
#[test]
fn a_run_is_staged_read_and_cleared_over_http() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();

    let staged = client
        .post_json(
            "/_mock/enqueue",
            &json!([
                { "model": MODEL, "outcome": { "reply": { "body": { "text": "first" } } } },
                {
                    "model": MODEL,
                    "times": 2,
                    "outcome": { "reply": { "body": { "text": "twice" } } },
                },
            ]),
        )
        .expect("the control plane accepts a batch");
    assert_eq!(staged.status, 200);
    assert_eq!(staged.json()["queued"], 2);
    assert_eq!(staged.json()["state"]["queues"][MODEL], 3);

    assert_eq!(text(&call(&client, "one")), "first");
    assert_eq!(text(&call(&client, "two")), "twice");
    assert_eq!(text(&call(&client, "three")), "twice");

    let transcript = client
        .get("/_mock/requests")
        .expect("the control plane answers")
        .json();
    let requests = transcript["requests"].as_array().expect("a list");
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["sequence"], 1);
    assert_eq!(requests[0]["surface"], "anthropic");
    assert_eq!(requests[0]["model"], MODEL);
    assert_eq!(requests[0]["verdict"], "valid");
    assert_eq!(requests[0]["served"], "reply.text");
    assert_eq!(
        requests[2]["body"]["messages"][0]["content"], "three",
        "the parsed body is what a transcript assertion reads"
    );

    let state = client.get("/_mock/state").expect("state").json();
    assert_eq!(state["requests"], 3);
    assert_eq!(state["invalid"], 0);
    assert_eq!(state["unscripted"], 0);
    assert_eq!(
        state["queues"],
        json!({}),
        "a drained model leaves the queue report"
    );

    let cleared = client
        .post_json("/_mock/reset", &json!({}))
        .expect("reset answers")
        .json();
    assert_eq!(cleared["discarded"]["requests"], 3);
    assert!(
        client.get("/_mock/state").expect("state").json()["requests"] == 0,
        "reset forgets the transcript too"
    );
}

/// The transcript records what failed and why, per field, which is what makes a
/// codegen bug legible from a test that only ran a graph.
#[test]
fn the_transcript_carries_per_field_verdicts() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();
    client
        .send(
            Request::post("/v1/messages")
                .anthropic_auth()
                .json(&json!({ "messages": [{ "role": "user", "content": "go" }] })),
        )
        .expect("the server answers");

    let transcript = client.get("/_mock/requests").expect("answers").json();
    let failures = transcript["requests"][0]["failures"]
        .as_array()
        .expect("a list of failures");
    let pointers: Vec<&str> = failures
        .iter()
        .map(|failure| failure["pointer"].as_str().expect("a pointer"))
        .collect();
    assert_eq!(pointers, ["model", "max_tokens"]);
    assert_eq!(failures[1]["message"], "max_tokens: Field required");
    assert_eq!(transcript["requests"][0]["verdict"], "invalid");
    assert_eq!(provider.invalid().len(), 1);
}

/// Matched FIFO is what makes a concurrent fan-out scriptable: the instances
/// share one model id and arrive in whatever order the scheduler chose, and each
/// still draws the outcome written for *its* item.
#[test]
fn concurrent_calls_draw_the_outcome_written_for_their_item() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue_all((0..8).map(|item| {
        Script::new(MODEL, Outcome::text(format!("answer-{item}"))).matching(format!("task-{item}"))
    }));

    let client = Arc::new(provider.client());
    let answers: Vec<(usize, String)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|item| {
                let client = Arc::clone(&client);
                scope.spawn(move || (item, text(&call(&client, &format!("task-{item}")))))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("the call finished"))
            .collect()
    });

    for (item, answer) in answers {
        assert_eq!(answer, format!("answer-{item}"));
    }
    assert!(provider.snapshot().is_drained());
    assert_eq!(provider.requests().len(), 8);
}

/// A scripted delay is the only clock: it reorders completion without changing
/// which answer belongs to which call, which is exactly what an index-tagged
/// reducer has to survive (PRD 5.6).
#[test]
fn a_scripted_delay_reorders_completion_without_crossing_answers() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue_all([
        Script::new(
            MODEL,
            Outcome::text("slow").after(Duration::from_millis(250)),
        )
        .matching("task-slow"),
        Script::new(MODEL, Outcome::text("fast")).matching("task-fast"),
    ]);

    let client = Arc::new(provider.client());
    let started = Instant::now();
    let finishes: Vec<(String, Duration)> = std::thread::scope(|scope| {
        let handles: Vec<_> = ["task-slow", "task-fast"]
            .into_iter()
            .map(|marker| {
                let client = Arc::clone(&client);
                scope.spawn(move || (text(&call(&client, marker)), started.elapsed()))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("the call finished"))
            .collect()
    });

    let slow = &finishes[0];
    let fast = &finishes[1];
    assert_eq!(slow.0, "slow", "the slow call still got its own answer");
    assert_eq!(fast.0, "fast");
    assert!(
        fast.1 < slow.1,
        "the delayed call finished last: {:?} then {:?}",
        fast.1,
        slow.1
    );
    assert!(
        slow.1 >= Duration::from_millis(250),
        "the scripted wait was honoured: {:?}",
        slow.1
    );
}

/// Ids are derived from the arrival sequence, so a transcript points at the call
/// that produced each answer and two runs of one script agree.
#[test]
fn ids_are_derived_from_the_arrival_sequence() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue_all([
        Script::new(MODEL, Outcome::text("one")),
        Script::new(MODEL, Outcome::text("two")),
    ]);
    let client = provider.client();

    let first = call(&client, "one").json();
    let second = call(&client, "two").json();
    assert_eq!(first["id"], "msg_mock_00000001");
    assert_eq!(second["id"], "msg_mock_00000002");

    let recorded = provider.requests();
    assert_eq!(recorded[0].sequence, 1);
    assert_eq!(recorded[1].sequence, 2);
}

/// The control plane refuses a script it cannot read, rather than queueing
/// something that answers nothing.
#[test]
fn a_malformed_script_is_refused_with_its_own_error() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();

    let refused = client
        .post_json(
            "/_mock/enqueue",
            &json!({ "model": MODEL, "outcome": { "reply": { "body": { "txet": "hi" } } } }),
        )
        .expect("the control plane answers");
    assert_eq!(refused.status, mock_provider::HARNESS_STATUS);
    assert!(
        refused.json()["mock_provider"]
            .as_str()
            .expect("a reason")
            .contains("txet"),
        "{}",
        refused.text()
    );

    let refused = client
        .send(Request::post("/_mock/enqueue").bytes("{not json"))
        .expect("the control plane answers");
    assert_eq!(refused.status, mock_provider::HARNESS_STATUS);
    assert!(provider.snapshot().is_drained(), "nothing was queued");
}

/// A matcher is scriptable over HTTP, in the spelling a TypeScript harness will
/// write: `"match": {"body_contains": …}`.
///
/// `Match` is the field the Rust tests lean on hardest — every fan-out and
/// every shared-model queue narrows with it — so the control-plane half has to
/// be exercised too, or a harness written against `/_mock/enqueue` would be the
/// first to discover the spelling.
#[test]
fn a_matcher_is_scriptable_over_http() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();

    let staged = client
        .post_json(
            "/_mock/enqueue",
            &json!([
                {
                    "model": MODEL,
                    "outcome": { "reply": { "body": { "text": "for-beta" } } },
                    "match": { "body_contains": "beta" },
                },
                {
                    "model": MODEL,
                    "outcome": { "reply": { "body": { "text": "for-alpha" } } },
                    "match": { "body_contains": "alpha" },
                },
            ]),
        )
        .expect("the control plane accepts narrowed scripts");
    assert_eq!(staged.status, 200);

    // The second entry answers first, because narrowing beats arrival order —
    // which is the property a concurrent fan-out depends on.
    assert_eq!(text(&call(&client, "alpha")), "for-alpha");
    assert_eq!(text(&call(&client, "beta")), "for-beta");
    assert!(provider.snapshot().is_drained());

    // A request no entry accepts is refused rather than answered by the nearest
    // one, and a matcher spelled wrong is refused on the way in.
    let refused = client
        .post_json(
            "/_mock/enqueue",
            &json!({
                "model": MODEL,
                "outcome": { "reply": { "body": { "text": "hi" } } },
                "match": { "body_contian": "alpha" },
            }),
        )
        .expect("the control plane answers");
    assert_eq!(refused.status, mock_provider::HARNESS_STATUS);
    assert!(
        refused.json()["mock_provider"]
            .as_str()
            .expect("a reason")
            .contains("body_contian"),
        "{}",
        refused.text()
    );
}

/// A body past the server's cap is refused as a request, and the refusal is the
/// harness's own — a mis-framed body must not become the harness's memory
/// problem, which is the only failure mode a test suite cannot diagnose from a
/// transcript.
#[test]
fn an_oversized_body_is_refused_rather_than_buffered() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("never served")));

    // Just past the 8 MiB cap: enough to trip it, small enough that the tail the
    // server stops reading fits in the socket's own buffers.
    let oversized = vec![b'x'; 8 * 1024 * 1024 + 1024];
    let refused = provider
        .client()
        .send(
            Request::post("/v1/messages")
                .anthropic_auth()
                .bytes(oversized),
        )
        .expect("the server answers rather than reading on");

    assert_eq!(refused.status, 413);
    assert_eq!(
        refused.header(mock_provider::HARNESS_HEADER),
        Some("oversized-request")
    );
    assert!(
        provider.requests().is_empty(),
        "a body the server refused to read is not a model call"
    );
    assert_eq!(
        provider.snapshot().queues[MODEL],
        1,
        "and it consumed nothing"
    );
}

/// Every scripted shape survives the JSON round trip the HTTP control plane puts
/// it through, so a TypeScript harness can stage what a Rust test can.
#[test]
fn every_outcome_shape_can_be_scripted_over_http() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();
    let shapes: Vec<Value> = vec![
        json!({ "reply": { "body": { "text": "hello" } } }),
        json!({ "reply": { "body": { "structured": { "verdict": "approve" } } } }),
        json!({ "reply": {
            "body": { "tools": { "calls": [{ "name": "web_search", "input": { "q": "x" } }] } },
            "delay": 5,
        }}),
        json!({ "reply": {
            "body": { "text": "counted" },
            "usage": { "input_tokens": 11, "output_tokens": 22 },
            "stop_reason": "max_tokens",
        }}),
        json!({ "failure": { "rate_limit": { "retry_after_seconds": 3 } } }),
        json!({ "failure": "overloaded" }),
        json!({ "failure": "server_error" }),
        json!({ "failure": { "timeout": { "delay": 10 } } }),
        json!({ "raw": { "status": 402, "body": { "nope": true } } }),
    ];

    for (index, outcome) in shapes.iter().enumerate() {
        let accepted = client
            .post_json(
                "/_mock/enqueue",
                &json!({ "model": format!("model-{index}"), "outcome": outcome }),
            )
            .expect("the control plane answers");
        assert_eq!(
            accepted.status,
            200,
            "outcome {index} was refused: {}",
            accepted.text()
        );
    }
    assert_eq!(provider.snapshot().queues.len(), shapes.len());

    // The one that carries an override is served the way it was written.
    provider.enqueue(Script::new(MODEL, Outcome::text("plain")));
    let counted = client
        .post_json(
            "/_mock/enqueue",
            &json!({
                "model": MODEL,
                "outcome": { "reply": {
                    "body": { "text": "counted" },
                    "usage": { "input_tokens": 11, "output_tokens": 22 },
                }},
            }),
        )
        .expect("the control plane answers");
    assert_eq!(counted.status, 200);
    let _ = call(&client, "first");
    let served = call(&client, "second").json();
    assert_eq!(served["usage"]["input_tokens"], 11);
    assert_eq!(served["usage"]["output_tokens"], 22);
}

/// A `raw` outcome's headers reach the client, and a header that could not be
/// put on the wire is refused when it is scripted — never served as a dropped
/// connection, which PRD 5.9 would read as a provider timeout.
#[test]
fn a_raw_outcomes_headers_are_served_or_refused_by_name() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();

    let staged = client
        .post_json(
            "/_mock/enqueue",
            &json!({
                "model": MODEL,
                "outcome": { "raw": {
                    "status": 402,
                    "body": { "nope": true },
                    "headers": { "retry-after": "7", "x-note": "served verbatim" },
                }},
            }),
        )
        .expect("the control plane answers");
    assert_eq!(staged.status, 200);

    let served = call(&client, "one");
    assert_eq!(served.status, 402);
    assert_eq!(served.header("retry-after"), Some("7"));
    assert_eq!(served.header("x-note"), Some("served verbatim"));

    for headers in [
        json!({ "bad header": "value" }),
        json!({ "x-ok": "a\nb" }),
        json!({ "x-ok": "a\u{0}b" }),
    ] {
        let refused = client
            .post_json(
                "/_mock/enqueue",
                &json!({
                    "model": MODEL,
                    "outcome": { "raw": { "status": 200, "body": {}, "headers": headers } },
                }),
            )
            .expect("the control plane answers");
        assert_eq!(
            refused.status,
            mock_provider::HARNESS_STATUS,
            "{headers} was accepted: {}",
            refused.text()
        );
        assert_eq!(
            refused.header(mock_provider::HARNESS_HEADER),
            Some("bad-control-request")
        );
    }
    assert!(
        provider.snapshot().queues.is_empty(),
        "a script that cannot be sent is never queued: {:?}",
        provider.snapshot().queues
    );
}

/// The second gate, end to end. A `raw` outcome assembled in Rust never passes
/// through the control plane's check, so the *client* is what must not be hurt
/// by one: it gets the harness's refusal rather than a connection that ends with
/// no answer — which PRD 5.9 classifies as a provider timeout, and which is how
/// a harness bug would arrive wearing a failover condition.
#[test]
fn a_raw_outcome_built_in_rust_cannot_drop_the_connection() {
    let provider = MockProvider::start().expect("a port");
    let mut outcome = Outcome::raw(200, json!({ "ok": true }));
    let Outcome::Raw(raw) = &mut outcome else {
        panic!("`Outcome::raw` is a raw outcome");
    };
    raw.headers
        .insert("bad header".to_string(), "v".to_string());
    provider.enqueue(Script::new(MODEL, outcome));

    let answered = call(&provider.client(), "one");
    assert_eq!(answered.status, mock_provider::HARNESS_STATUS);
    assert_eq!(
        answered.header(mock_provider::HARNESS_HEADER),
        Some(mock_provider::REFUSED_UNSENDABLE)
    );
    assert!(
        answered.text().contains("bad header"),
        "the refusal names the header: {}",
        answered.text()
    );
}
