//! Reading a documented list out of the prose that publishes it, for the tests
//! that hold the docs to the mechanism.
//!
//! Several decisions of this compiler are stated in more than one document — a
//! grammar section, a topic of `agent-compose docs`, an explanation the
//! diagnostic's own footer sends an author to — and prose is not compiled, so
//! the second and third copies drift silently. The tests that prevent that read
//! the list out of the sentence that publishes it and compare it against the
//! constant the compiler actually branches on.

/// The fragment of `document` between two markers, or a failure naming the one
/// that has moved.
///
/// The prose is asserted about **where it enumerates**, not anywhere in the
/// file: `docs/grammar.md` names `trace_sink` in §14.5 whether or not §1.5's
/// table row does, so a whole-file search would pass over exactly the drift
/// these tests are written to catch. A reworded sentence fails here saying which
/// marker went missing, which is the same posture
/// `codegen::otlp::the_resource_attributes_are_the_documented_ones` takes to the
/// emitted source it reads.
///
/// A caller that reads a sentence spanning a line break flattens the document's
/// newlines to spaces first, since a list broken across lines is the same list.
pub(crate) fn enumeration<'a>(name: &str, document: &'a str, from: &str, to: &str) -> &'a str {
    let after = document.split_once(from).unwrap_or_else(|| {
        panic!("`{name}` no longer says `{from}`, so nothing here reads its list of sections")
    });
    after
        .1
        .split_once(to)
        .unwrap_or_else(|| panic!("`{name}`'s `{from}` sentence no longer ends at `{to}`"))
        .0
}
