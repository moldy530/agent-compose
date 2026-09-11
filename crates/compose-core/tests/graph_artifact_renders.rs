//! The emitted page's own code, **run**.
//!
//! Every other test here reads the artifact as text: the goldens pin its bytes,
//! the self-containment suite pins what it does not reach for, and the inventory
//! pins the document it carries. None of them runs a line of it — and the page
//! *is* the product of `agent-compose visualize`, so a renderer that threw on
//! the first node a reader clicked would ship green.
//!
//! The failure class this exists for is specific and has one shape. The graph
//! document **omits** an array rather than writing an empty one (`docs/graph.md`
//! §9.1: an absent key and an empty value are two different facts), so a
//! renderer that reaches `route.covers.length` on a homogeneous map, or
//! `model.settings.forEach` on a model that declares no settings, throws a
//! `TypeError` on exactly the compositions that leave those keys out — which is
//! most compositions, and none of the ones the goldens happen to cover.
//!
//! So this file runs the page under Bun, against a DOM stub sized to what the
//! template touches, and exercises the paths a reader takes: render every flow,
//! select every node, walk back to the flow overview, and do it again with
//! `localStorage` throwing on every access, which is what a private window and a
//! browser with site data blocked both look like.
//!
//! CLAUDE.md's *Validation strategy* is why it is a test rather than a look:
//! "CI green" has to mean "actually done", and for an HTML artifact that means
//! something ran it.

#[path = "support/toolchain.rs"]
mod toolchain;

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::graph;
use compose_core::resolve;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// The page one example project renders to.
fn page(project: &str) -> String {
    let resolution = resolve(repository().join(project).join("main.yml"));
    let ir = resolution.ir.expect("the example resolves");
    assert!(
        compose_core::check(&ir).is_empty(),
        "`{project}` does not validate"
    );
    graph::render(&graph::graph(&ir)).expect("the document renders")
}

/// The script the page carries, lifted out of its one `<script>` element.
fn script(page: &str) -> &str {
    let opened = page.find("<script>").expect("the page has a script") + "<script>".len();
    let closed = page.find("</script>").expect("the script is closed");
    &page[opened..closed]
}

/// A DOM sized to what the template touches, and nothing else.
///
/// Deliberately small: it is a harness for the page's own logic, not a browser.
/// Every method here is one the template calls, and a template that started
/// calling something else fails with a name rather than silently doing nothing —
/// which is why the stub has no catch-all.
const DOM: &str = r#"
const created = [];
function element(name) {
  const held = {
    tagName: name,
    children: [],
    attributes: {},
    style: {},
    dataset: {},
    textContent: "",
    innerHTML: "",
    scrollTop: 0,
    clientWidth: 1200,
    clientHeight: 800,
    firstChild: null,
    classes: new Set(),
    setAttribute(key, value) {
      if (value === undefined || value === null) throw new Error(name + ": " + key + " is " + value);
      this.attributes[key] = String(value);
    },
    getAttribute(key) { return key in this.attributes ? this.attributes[key] : null; },
    appendChild(child) { this.children.push(child); this.firstChild = this.children[0]; return child; },
    insertBefore(child, before) { this.children.unshift(child); this.firstChild = this.children[0]; return child; },
    removeChild(child) {
      this.children = this.children.filter(held => held !== child);
      this.firstChild = this.children.length ? this.children[0] : null;
      return child;
    },
    addEventListener() {},
    setPointerCapture() {},
    querySelector() { return null; },
    querySelectorAll() { return []; },
    getBBox() { return { x: 0, y: 0, width: 240, height: 18 }; },
    getBoundingClientRect() { return { left: 0, top: 0, width: 1200, height: 800 }; },
    classList: {
      add() {}, remove() {}, toggle() {}, contains() { return false; }
    }
  };
  created.push(held);
  return held;
}

const byId = {};
globalThis.document = {
  title: "",
  createElement: element,
  createElementNS: (namespace, name) => element(name),
  createTextNode: text => ({ textContent: String(text) }),
  getElementById(id) {
    if (!(id in byId)) byId[id] = element(id);
    return byId[id];
  },
  querySelectorAll(selector) {
    if (selector === ".node") {
      return created.filter(held => (held.attributes.class || "") === "node");
    }
    if (selector === ".tab[data-flow]") {
      return created.filter(held => "data-flow" in held.attributes);
    }
    throw new Error("the stub has no selector `" + selector + "`");
  }
};
globalThis.window = { addEventListener() {} };
globalThis.addEventListener = () => {};
"#;

/// A `localStorage` that works, and one that throws on every access.
const STORAGE_WORKS: &str = r#"
const held = {};
globalThis.localStorage = {
  getItem: key => (key in held ? held[key] : null),
  setItem: (key, value) => { held[key] = String(value); },
  removeItem: key => { delete held[key]; }
};
"#;
const STORAGE_REFUSES: &str = r#"
globalThis.localStorage = {
  getItem() { throw new Error("site data is blocked"); },
  setItem() { throw new Error("site data is blocked"); },
  removeItem() { throw new Error("site data is blocked"); }
};
"#;

/// What the harness does once the page's own code has run.
///
/// The paths a reader takes, in the order they take them — which is what makes
/// a `TypeError` in one node's pane a failure here rather than a blank side
/// panel in somebody's browser.
///
/// Opening every pane catches the renderer that **throws**. It does not catch
/// the renderer that quietly renders nothing, because every value in a pane
/// goes through `esc()`, which answers `""` for `undefined` — so a key read
/// under the wrong name comes out as empty markup and a pane full of facts and
/// a pane full of blanks are the same length. That is the second half, `FACTS`:
/// named nodes, and resolved facts a reader must be able to find in their
/// panes. A key that stops arriving fails here instead of shipping.
const EXERCISE: &str = r#"
let panes = 0;
for (const flow of DOC.flows) {
  renderFlow(flow.address);
  for (const node of flow.nodes) {
    select(node.id);
    if (pane.innerHTML.length < 40) {
      throw new Error("the pane is empty for `" + flow.address + "`.`" + node.id + "`");
    }
    panes += 1;
    showFlow();
  }
  // The controls a reader reaches for, and the one that discards a layout.
  document.getElementById("zoom-in").attributes;
  fit();
}
if (panes < 3) throw new Error("the harness selected nothing");

let facts = 0;
for (const held of FACTS) {
  renderFlow(held[0]);
  const where = "`" + held[0] + "`.`" + held[1] + "`";
  if (!current.byId[held[1]]) throw new Error(where + " is not a node of that flow");
  select(held[1]);
  for (const fact of held[2]) {
    if (pane.innerHTML.indexOf(fact) < 0) {
      throw new Error(where + "'s pane does not carry `" + fact + "`:\n" + pane.innerHTML);
    }
    facts += 1;
  }
}
console.log("panes=" + panes);
console.log("facts=" + facts);
"#;

/// One pane, and resolved facts a reader must find in it.
///
/// Every string is asserted against the pane's markup as the page writes it, so
/// what is pinned is the whole path — the compiler resolved it, the document
/// carried it under the name the template reads, and the template printed it.
struct Pane {
    /// The flow to render first.
    flow: &'static str,
    /// The node to select.
    node: &'static str,
    /// Text the pane must contain, HTML-escaped as `esc` leaves it.
    facts: &'static [&'static str],
}

/// The panes checked for each project, chosen for the facts that are hardest to
/// notice going missing: a resolution that happens in the compiler and appears
/// nowhere else.
fn panes(project: &str) -> &'static [Pane] {
    match project {
        "examples/triage-fanout" => &[
            // The model resolved through to its provider, the store tool
            // grammar 11.5 synthesizes, the policy level, and a summarized
            // union — the four the pane is the only place a reader meets.
            Pane {
                flow: "flow.triage",
                node: "classify",
                facts: &[
                    "model.smart",
                    "claude-sonnet-4-6",
                    "provider.anthropic (anthropic)",
                    "docs_search",
                    "synthesized from store.docs",
                    "union on kind [auto_fixable, needs_human, duplicate]",
                    "(max_items 50)",
                    "90s  (defaults)",
                ],
            },
            // The wait, its transfer, and the channel its answer is remapped
            // onto.
            Pane {
                flow: "flow.triage",
                node: "approve",
                facts: &[
                    "24h",
                    "on_timeout",
                    "→ escalate",
                    "enum [approve, reject]",
                    "→ state.human_decision",
                ],
            },
            // The fan-out: both bounds, the per-item policy, and the narrowing
            // of the catch-all.
            Pane {
                flow: "flow.triage",
                node: "dispatch",
                facts: &[
                    "classify.output.findings",
                    "routed",
                    "retry (max 2, backoff 2s)",
                    "≤ 4 concurrent",
                    "→ tool.dead_letter · narrowed to duplicate",
                    "skip  (node)",
                ],
            },
            // A satellite, which is the only node whose configuration comes
            // from a route rather than from a `nodes:` entry.
            Pane {
                flow: "flow.triage",
                node: "dispatch/(default)",
                facts: &["tool.dead_letter", "the catch-all route", "duplicate"],
            },
            // The fallback target, and an `${ENV}` reference left as written.
            Pane {
                flow: "flow.triage",
                node: "announce",
                facts: &[
                    "https://${QUEUE_HOST}/v1/triage-started",
                    "fallback → announce_failed",
                ],
            },
            // The boundary of the subgraph, and the policy inside the instance.
            Pane {
                flow: "flow.triage",
                node: "enrich",
                facts: &["flow.enrich", "isolated", "30s"],
            },
        ],
        "examples/review-loop" => &[
            // A failover route rather than a direct model: the conditions and
            // every member, in failover order.
            Pane {
                flow: "flow.review_loop",
                node: "write",
                facts: &[
                    "model.default",
                    "on rate_limit, overloaded, timeout",
                    "↳ model.smart",
                    "↳ model.fast",
                    "claude-haiku-4-5",
                    "web_search",
                    "max 2, backoff 5s  (node)",
                ],
            },
            // The bounded cycle and the two edges that leave the loop.
            Pane {
                flow: "flow.review_loop",
                node: "review",
                facts: &[
                    "when review.output.verdict == 'revise'",
                    "max_iterations 3",
                    "else",
                    "on_error: fallback",
                ],
            },
        ],
        "crates/compose-core/tests/projects/omitted-graph-keys" => &[
            // The shapes nothing is declared on: a homogeneous fan-out, a
            // policy resolved at the built-in, and a wait with no budget.
            Pane {
                flow: "flow.bare",
                node: "work",
                facts: &[
                    "homogeneous",
                    "every item",
                    "→ agent.worker",
                    "fail  (built_in)",
                ],
            },
            Pane {
                flow: "flow.bare",
                node: "work/(item)",
                facts: &["agent.worker", "one unnamed string"],
            },
            Pane {
                flow: "flow.bare",
                node: "sign_off",
                facts: &["unbounded"],
            },
        ],
        _ => &[],
    }
}

/// The `FACTS` table the harness reads, as a JavaScript literal.
fn facts_of(project: &str) -> String {
    let mut held = String::from("const FACTS = [\n");
    for pane in panes(project) {
        held.push_str(&format!("  [{:?}, {:?}, [", pane.flow, pane.node));
        for fact in pane.facts {
            held.push_str(&format!("{fact:?}, "));
        }
        held.push_str("]],\n");
    }
    held.push_str("];\n");
    held
}

/// Run one page's script under Bun with `storage`, and answer what it printed.
fn exercise(project: &str, storage: &str) -> String {
    let Some(mut bun) = toolchain::bun_command() else {
        return String::new();
    };
    let directory = std::env::temp_dir().join(format!(
        "agent-compose-graph-render-{}-{}",
        std::process::id(),
        project.replace('/', "-")
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("a scratch directory");
    let harness = directory.join("harness.js");
    fs::write(
        &harness,
        format!(
            "{DOM}\n{storage}\n{}\n{}\n{EXERCISE}\n",
            script(&page(project)),
            facts_of(project)
        ),
    )
    .expect("the harness is writable");

    let output = bun.arg(&harness).output().expect("bun runs");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "`{project}`'s page threw:\n{}\n{stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(&directory);
    stdout
}

/// One count the harness printed.
fn counted(printed: &str, what: &str) -> usize {
    printed
        .lines()
        .find_map(|line| line.trim().strip_prefix(what))
        .unwrap_or_else(|| panic!("the harness prints `{what}`: {printed}"))
        .parse()
        .expect("a count")
}

/// Every node of every flow opens a detail pane without throwing, and the panes
/// a reader reads carry the facts the compiler resolved.
///
/// The third project is the one that matters most and is a fixture rather than
/// an example: `omitted-graph-keys` is written to leave every optional array
/// **out**, which is the shape both worked examples happen never to produce and
/// the shape this whole file exists for.
#[test]
fn every_node_of_every_flow_opens_its_pane() {
    for project in [
        "examples/triage-fanout",
        "examples/review-loop",
        "crates/compose-core/tests/projects/omitted-graph-keys",
    ] {
        let printed = exercise(project, STORAGE_WORKS);
        if printed.is_empty() {
            return; // Bun is absent and this is not CI; `toolchain` said so.
        }
        let panes = counted(&printed, "panes=");
        assert!(
            panes >= 3,
            "`{project}` has more nodes than {panes} on its canvases"
        );
        let facts = counted(&printed, "facts=");
        let expected: usize = self::panes(project)
            .iter()
            .map(|pane| pane.facts.len())
            .sum();
        assert!(expected > 0, "`{project}` names no pane to read");
        assert_eq!(
            facts, expected,
            "`{project}`'s harness checked {facts} facts of {expected}"
        );
    }
}

/// …and again with `localStorage` refusing every call.
///
/// Per-viewer layout persistence is a **convenience**: the page renders exactly
/// the same without it, and a private window, a browser with site data blocked,
/// and a `file://` page under a strict policy are all ways a reader arrives with
/// it gone. A guard that was written and not exercised is a guard that is one
/// refactor from being dropped.
#[test]
fn the_page_renders_where_storage_refuses_every_call() {
    let printed = exercise("examples/triage-fanout", STORAGE_REFUSES);
    if printed.is_empty() {
        return;
    }
    assert!(
        counted(&printed, "panes=") >= 3,
        "the page still opened its panes: {printed}"
    );
}
