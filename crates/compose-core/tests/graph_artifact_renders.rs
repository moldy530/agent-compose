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
console.log("panes=" + panes);
"#;

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
        format!("{DOM}\n{storage}\n{}\n{EXERCISE}\n", script(&page(project))),
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

/// Every node of every flow opens a detail pane without throwing.
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
        let panes: usize = printed
            .trim()
            .strip_prefix("panes=")
            .expect("the harness counts the panes it opened")
            .parse()
            .expect("a count");
        assert!(
            panes >= 3,
            "`{project}` has more nodes than {panes} on its canvases"
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
        printed.starts_with("panes="),
        "the page still opened its panes: {printed}"
    );
}
