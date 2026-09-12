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
//! select every node, click the pane's close and its jump to a subgraph's own
//! canvas, walk back to the flow overview, and do it again with `localStorage`
//! throwing on every access, which is what a private window and a browser with
//! site data blocked both look like.
//!
//! It is also the one test that reads the page's **geometry**. The dashed
//! container a fan-out is drawn in is an axis-aligned box over a map and the
//! instances it dispatches, and a node that is neither, drawn inside it, reads as
//! one of them — a misreading no other test here can see, because the document is
//! correct and says nothing about where anything is drawn.
//!
//! CLAUDE.md's *Validation strategy* is why it is a test rather than a look:
//! "CI green" has to mean "actually done", and for an HTML artifact that means
//! something ran it.

#[path = "support/toolchain.rs"]
mod toolchain;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use compose_core::graph::{self, GraphDocument, NodeKind};
use compose_core::resolve;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// The graph document one composition resolves to, by the path of its
/// entrypoint.
fn document_at(entrypoint: &Path) -> GraphDocument {
    let resolution = resolve(entrypoint);
    let ir = resolution.ir.unwrap_or_else(|| {
        panic!(
            "`{}` does not resolve: {:#?}",
            entrypoint.display(),
            resolution.diagnostics
        )
    });
    assert!(
        compose_core::check(&ir).is_empty(),
        "`{}` does not validate",
        entrypoint.display()
    );
    graph::graph(&ir)
}

/// The graph document one project in this repository renders from.
fn document(project: &str) -> GraphDocument {
    document_at(&repository().join(project).join("main.yml"))
}

/// The page one example project renders to.
fn page(project: &str) -> String {
    graph::render(&document(project)).expect("the document renders")
}

/// How many fan-outs a document draws a dashed container around — what the
/// harness's geometry check has to have looked at.
fn containers(document: &GraphDocument) -> usize {
    document
        .flows
        .iter()
        .flat_map(|flow| &flow.nodes)
        .filter(|node| node.kind == NodeKind::Map)
        .count()
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
    listeners: {},
    found: {},
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
    addEventListener(type, listener) {
      (this.listeners[type] = this.listeners[type] || []).push(listener);
    },
    /* What the harness clicks with: the listeners the page itself bound, fired
       in the order it bound them. */
    fire(type, event) {
      const held = this.listeners[type] || [];
      if (!held.length) throw new Error(this.tagName + " has no `" + type + "` listener");
      held.forEach(listener => listener(event || { target: { closest: () => null } }));
    },
    setPointerCapture() {},
    /* The pane's content is a **string** in this stub — the template writes it
       with `innerHTML` and never builds its children — so the two selectors the
       template reaches into that content with are answered from the string. Each
       answer is a stand-in cached against the content it was found in, so the
       template and the harness hold the *same* element: the handler the page
       bound to it is the handler a click fires. Without that the close button
       and the subgraph jump are reached by nothing. */
    querySelector(selector) {
      const key = selector + "\n" + this.innerHTML;
      if (key in this.found) return this.found[key];
      let answer = null;
      if (selector === ".pane-close") {
        if (this.innerHTML.indexOf('class="pane-close"') >= 0) answer = element("button");
      } else if (selector === "[data-jump]:not([disabled])") {
        const match = /data-jump="([^"]*)"([^>]*)>/.exec(this.innerHTML);
        if (match && match[2].indexOf("disabled") < 0) {
          answer = element("button");
          answer.setAttribute("data-jump", match[1]);
        }
      } else {
        throw new Error("the stub has no element selector `" + selector + "`");
      }
      return (this.found[key] = answer);
    },
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
/* The one check here that reads the page's **geometry**.
 *
 * The dashed container is an axis-aligned box over a map and the instances it
 * dispatches, and a node that is neither, drawn inside it, reads as one of them
 * — the exact misreading PRD resolved q56 puts the container there to prevent.
 * The document is no help: it is correct, and it says nothing about where
 * anything is drawn. So the box the page sized is measured against every node
 * the page placed. */
function containersHoldOnlyTheirOwn(flow) {
  mapEls.forEach(entry => {
    const x = +entry.rect.getAttribute("x"), y = +entry.rect.getAttribute("y");
    const box = {
      x0: x, y0: y,
      x1: x + +entry.rect.getAttribute("width"),
      y1: y + +entry.rect.getAttribute("height")
    };
    flow.nodes.forEach(node => {
      if (entry.members.indexOf(node.id) >= 0) return;
      const held = current.byId[node.id];
      if (held.x < box.x1 && held.x + held.w > box.x0 &&
          held.y < box.y1 && held.y + held.h > box.y0) {
        throw new Error("`" + node.id + "` is drawn inside the fan-out container `" +
          entry.text.textContent + "` of `" + flow.address + "`, and is none of " +
          entry.members.join(", "));
      }
    });
    boxes += 1;
  });
}

let panes = 0, boxes = 0, closes = 0, jumps = 0;
for (const flow of DOC.flows) {
  renderFlow(flow.address);
  containersHoldOnlyTheirOwn(flow);
  for (const node of flow.nodes) {
    select(node.id);
    if (pane.innerHTML.length < 40) {
      throw new Error("the pane is empty for `" + flow.address + "`.`" + node.id + "`");
    }
    panes += 1;
    // The pane's own two controls, clicked rather than read: the close that
    // walks back to the flow overview, and the jump a `flow:` node carries to
    // its own canvas — the one navigation PRD resolved q56 names.
    const jump = pane.querySelector("[data-jump]:not([disabled])");
    if (jump) {
      const target = jump.getAttribute("data-jump");
      jump.fire("click");
      if (!current.flow || current.flow.address !== target) {
        throw new Error("the jump on `" + node.id + "` did not open `" + target + "`");
      }
      jumps += 1;
      renderFlow(flow.address);
      select(node.id);
    }
    const close = pane.querySelector(".pane-close");
    if (close) {
      close.fire("click");
      if (current.selected !== null) {
        throw new Error("closing `" + node.id + "`'s pane left it selected");
      }
      closes += 1;
    }
    showFlow();
  }
  // The controls a reader reaches for, and the one that discards a layout.
  document.getElementById("zoom-in").attributes;
  fit();
  resetLayout();
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
console.log("boxes=" + boxes);
console.log("closes=" + closes);
console.log("jumps=" + jumps);
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
            // A built-in bound as a `tool.*`: the pane lists it under the name
            // the **model** calls it by, which the definition key does not spell
            // (PRD resolved q54 ruling d).
            Pane {
                flow: "flow.triage",
                node: "dispatch/auto_fixable",
                facts: &[
                    "agent.fixer",
                    "str_replace_based_edit_tool",
                    "builtin.files",
                ],
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
            //
            // `each item` is what a homogeneous map's one route is called in
            // both places a reader meets it — the satellite's badge and this
            // row — so the two are one name rather than two.
            Pane {
                flow: "flow.bare",
                node: "work",
                facts: &[
                    "homogeneous",
                    "each item",
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
        "crates/compose-core/tests/projects/every-schema-form" => &[
            // The two halves of `ToolView.source` no other project here
            // attaches. `docs/graph.md` §9.1 makes that field a closed
            // vocabulary a reader may switch on, and the pane is where each
            // value becomes a word somebody reads: a `flow.*` used as a tool
            // (PRD resolved q19) and a `builtin.*` attached by the `tools:`
            // shorthand (PRD resolved q54) — the second distinguishable from the
            // same built-in bound as a `tool.*` only by what bounded it.
            Pane {
                flow: "flow.shape",
                node: "shape",
                facts: &[
                    "condense",
                    "class=\"tag\">flow</span>",
                    "a flow instantiated once per model tool call",
                    "str_replace_based_edit_tool",
                    "class=\"tag\">builtin · builtin</span>",
                    "attached by the `tools:` shorthand, under its default bounds",
                    // …beside the same shell bound as a `tool.*`, which is the
                    // row that says what the shorthand's bounds are not.
                    "builtin.bash, under the name its provider dictates",
                ],
            },
            Pane {
                flow: "flow.shape",
                node: "spread/(item)",
                facts: &[
                    "bash",
                    "class=\"tag\">builtin · builtin</span>",
                    "attached by the `tools:` shorthand, under its default bounds",
                ],
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

/// A scratch directory of this file's own, named for the *call* rather than for
/// what it runs.
///
/// The tests here run on parallel threads of one process and two of them
/// exercise `examples/triage-fanout`, so a name built from the pid and the
/// project alone would have them writing and deleting one `harness.js` — a race
/// whose symptom is bun failing to resolve a module and an assertion blaming the
/// template.
fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let directory = std::env::temp_dir().join(format!(
        "agent-compose-graph-render-{}-{}-{}",
        std::process::id(),
        label.replace('/', "-"),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

/// Run one page's script under Bun with `storage` and `epilogue`, and answer
/// what it printed.
fn run(label: &str, page: &str, storage: &str, epilogue: &str) -> String {
    let Some(mut bun) = toolchain::bun_command() else {
        return String::new();
    };
    let directory = scratch(label);
    let harness = directory.join("harness.js");
    fs::write(
        &harness,
        format!("{DOM}\n{storage}\n{}\n{epilogue}\n", script(page)),
    )
    .expect("the harness is writable");

    let output = bun.arg(&harness).output().expect("bun runs");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "`{label}`'s page threw:\n{}\n{stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(&directory);
    stdout
}

/// Walk one project's page the way a reader does.
fn exercise(project: &str, storage: &str) -> String {
    run(
        project,
        &page(project),
        storage,
        &format!("{}\n{EXERCISE}", facts_of(project)),
    )
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

/// Every node of every flow opens a detail pane without throwing, every pane's
/// own controls work, no fan-out container holds a node it does not dispatch,
/// and the panes a reader reads carry the facts the compiler resolved.
///
/// The two fixtures matter more than the examples. `omitted-graph-keys` is
/// written to leave every optional array **out**, which is the shape both worked
/// examples happen never to produce and the shape this whole file exists for;
/// `every-schema-form` is the only project here that attaches a `flow.*` as a
/// tool and a `builtin.*` by the `tools:` shorthand, which are two of the four
/// values `docs/graph.md` §9.1 makes `ToolView.source` out of.
#[test]
fn every_node_of_every_flow_opens_its_pane() {
    for project in [
        "examples/triage-fanout",
        "examples/review-loop",
        "crates/compose-core/tests/projects/omitted-graph-keys",
        "crates/compose-core/tests/projects/every-schema-form",
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
        // Every fan-out the document declares is a container the geometry check
        // measured — the count is what catches the check quietly looking at
        // nothing, which is how a geometric invariant rots.
        assert_eq!(
            counted(&printed, "boxes="),
            containers(&document(project)),
            "`{project}`'s containers were not all measured"
        );
        // Every pane a node opens carries the close button, so the walk cannot
        // have clicked none of them.
        assert!(
            counted(&printed, "closes=") > 0,
            "`{project}`'s panes were never closed"
        );
    }
}

/// The subgraph jump: a `flow:` node's pane opens that flow's own canvas.
///
/// PRD resolved q56 names this one by name — a `flow:` node links to its flow's
/// tab, never expanding inline — and `examples/triage-fanout` is the project
/// here with a subgraph to click. The walk above fires it wherever it finds one;
/// this is what insists it was found.
#[test]
fn a_subgraph_pane_opens_the_flow_it_instantiates() {
    let printed = exercise("examples/triage-fanout", STORAGE_WORKS);
    if printed.is_empty() {
        return;
    }
    assert!(
        counted(&printed, "jumps=") > 0,
        "`enrich`'s pane offered no jump to `flow.enrich`: {printed}"
    );
}

/// A page emitted for a composition with **no flows** answers its controls.
///
/// A composition of `provider.*` and `model.*` definitions and no `flow.*`
/// validates clean, so `visualize` emits a page for it — one with no flow to
/// land on, an empty canvas, and the same background and the same "Reset layout"
/// every other page has. Both of those read the flow on screen, and there is
/// none.
#[test]
fn a_page_with_no_flows_answers_its_background_and_its_reset() {
    let directory = scratch("flowless");
    let entrypoint = directory.join("main.yml");
    fs::write(
        &entrypoint,
        r#"version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.smart:
  provider: provider.anthropic
  id: claude-sonnet-4-6
"#,
    )
    .expect("the entrypoint is writable");
    let document = document_at(&entrypoint);
    assert!(
        document.flows.is_empty(),
        "the fixture is the composition with nothing to draw"
    );
    let page = graph::render(&document).expect("the document renders");
    let _ = fs::remove_dir_all(&directory);

    let printed = run(
        "flowless",
        &page,
        STORAGE_WORKS,
        r#"
if (current.flow !== null) throw new Error("there is no flow to have landed on");
// A click on the canvas background, which every page answers by walking the
// pane back to the flow overview, and the control that discards a layout.
canvas.fire("click");
resetLayout();
if (pane.innerHTML.indexOf("declares no flows") < 0) {
  throw new Error("the pane does not say what is missing:\n" + pane.innerHTML);
}
console.log("answered=2");
"#,
    );
    if printed.is_empty() {
        return;
    }
    assert_eq!(
        counted(&printed, "answered="),
        2,
        "both controls answered: {printed}"
    );
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
