//! The embeddings surface a `vector` store's `embed:` reaches (grammar 11.2).
//!
//! It is the one route this server answers **without** a script, and the reason
//! is in `WIRE-NOTES.md`: an embedding is not a decision a compiled graph makes,
//! so a queue for it would make every store test enqueue answers it never reads.
//! What a store test does assert about is the *search* — that the document it
//! indexed is the one the search finds — and that needs the vector to be
//! deterministic and to carry something about the text, which is what these
//! tests pin.
//!
//! The **request** is held to the surface's shape like every other route here: a
//! compiled graph that sent something else would be a codegen bug this harness
//! exists to catch.

use mock_provider::{Client, MockProvider, Request};
use serde_json::{Value, json};

const MODEL: &str = "text-embedding-3-small";

/// One embeddings call, at whichever of the two spellings the caller names.
fn embed(client: &Client, route: &str, body: &Value) -> mock_provider::Response {
    client
        .send(Request::post(route).openai_auth().json(body))
        .expect("the mock provider answers")
}

/// The vector of one text, as the surface answers it.
fn vector(response: &mock_provider::Response, index: usize) -> Vec<f64> {
    response.json()["data"][index]["embedding"]
        .as_array()
        .expect("an embedding is a list of numbers")
        .iter()
        .map(|value| value.as_f64().expect("a number"))
        .collect()
}

/// Cosine similarity, which is what a `vector` store's `search` ranks by.
fn cosine(left: &[f64], right: &[f64]) -> f64 {
    let dot: f64 = left.iter().zip(right).map(|(a, b)| a * b).sum();
    let magnitude = |values: &[f64]| values.iter().map(|value| value * value).sum::<f64>().sqrt();
    let scale = magnitude(left) * magnitude(right);
    if scale == 0.0 { 0.0 } else { dot / scale }
}

/// The shape of an answer: one row per input, in request order, at a fixed
/// width.
#[test]
fn one_row_per_input_in_request_order() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();

    let answer = embed(
        &client,
        "/v1/embeddings",
        &json!({ "model": MODEL, "input": ["alpha", "beta", "gamma"] }),
    );
    assert_eq!(answer.status, 200);
    let body = answer.json();
    assert_eq!(body["object"], "list");
    assert_eq!(body["model"], MODEL);
    let rows = body["data"].as_array().expect("a list of rows");
    assert_eq!(rows.len(), 3);
    for (position, row) in rows.iter().enumerate() {
        assert_eq!(row["object"], "embedding");
        assert_eq!(row["index"], position, "the rows are in request order");
    }
    let widths: Vec<usize> = (0..3).map(|index| vector(&answer, index).len()).collect();
    assert_eq!(
        widths,
        [8, 8, 8],
        "one declared width, which is what `embed.dimensions:` is asserted against"
    );

    // An embeddings call is not a *model* call: it joins no queue and appears in
    // no transcript, so a store test's assertions about which model was asked
    // what are unaffected by how many searches the run made.
    assert!(provider.requests().is_empty());
    assert!(provider.snapshot().is_drained());
}

/// The Azure spelling reaches the same surface.
#[test]
fn the_azure_route_answers_the_same_surface() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();

    let direct = embed(
        &client,
        "/v1/embeddings",
        &json!({ "model": MODEL, "input": ["the quick brown fox"] }),
    );
    let azure = embed(
        &client,
        "/openai/v1/embeddings",
        &json!({ "model": MODEL, "input": ["the quick brown fox"] }),
    );
    assert_eq!(azure.status, 200);
    assert_eq!(vector(&direct, 0), vector(&azure, 0));
}

/// The property a `search` assertion rests on: identical texts embed
/// identically, and a text is nearer its own document than an unrelated one.
#[test]
fn the_answer_is_deterministic_and_carries_something_about_the_text() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();

    let asked = |text: &str| {
        let answer = embed(
            &client,
            "/v1/embeddings",
            &json!({ "model": MODEL, "input": [text] }),
        );
        vector(&answer, 0)
    };

    let document = asked("the quick brown fox jumps");
    assert_eq!(
        document,
        asked("the quick brown fox jumps"),
        "the same text embeds the same way, or no store test could assert twice"
    );
    // Case and punctuation are not the text, which is what makes a query find a
    // document it does not quote exactly.
    assert_eq!(document, asked("The quick, brown Fox jumps!"));

    let near = asked("the quick brown fox");
    let far = asked("unrelated words entirely");
    assert!(
        cosine(&document, &near) > cosine(&document, &far),
        "a document is nearer a query that shares its words: {} vs {}",
        cosine(&document, &near),
        cosine(&document, &far)
    );
}

/// A request that is not the surface's shape is refused rather than answered
/// approximately — the rule every route here follows.
#[test]
fn a_request_that_is_not_the_surface_is_refused() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();

    for body in [
        json!({ "input": ["a"] }),
        json!({ "model": MODEL }),
        json!({ "model": MODEL, "input": "a bare string" }),
        json!({ "model": MODEL, "input": [42] }),
    ] {
        let refused = embed(&client, "/v1/embeddings", &body);
        assert_eq!(
            refused.status, 400,
            "{body} was answered: {:?}",
            refused.body
        );
        assert!(
            refused.json()["mock_provider"].is_string(),
            "the refusal is the harness's own voice: {:?}",
            refused.body
        );
    }
}
