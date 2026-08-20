# agent-compose — Plan Format

**Plan version:** `1`

This document is **normative** for `agent-compose plan --format json`. It is the
whole of what a consumer of that document may rely on, and §12 is the contract:
what is stable, what may be added without notice, and what moves the version
number at the top of this file.

`crates/compose-core/tests/plan_format_inventory.rs` holds this document to the
compiler. Every record type the format is made of, every field of one, and every
member of the closed vocabularies those fields draw on must be named here, in
the section that specifies it — a field shipped without a row is a test failure,
because a reader pinning `plan_version` on this file has not been told about it.

The command itself, its two entrypoint arguments and its exit codes are
`crates/agent-compose/src/main.rs`; §13 covers the other format, the human one,
which is a report rather than a contract.

## Table of contents

- [1. What a plan is](#1-what-a-plan-is)
- [2. The two documents](#2-the-two-documents)
  - [2.1 The plan](#21-the-plan)
  - [2.2 Each side](#22-each-side)
  - [2.3 A spec that does not resolve](#23-a-spec-that-does-not-resolve)
- [3. What a change record says](#3-what-a-change-record-says)
- [4. Components](#4-components)
- [5. Topology](#5-topology)
- [6. Interfaces](#6-interfaces)
- [7. Validation](#7-validation)
- [8. Addresses](#8-addresses)
- [9. Ordering](#9-ordering)
- [10. Locations](#10-locations)
- [11. What is not compared](#11-what-is-not-compared)
- [12. Stability](#12-stability)
  - [12.1 What a reader may rely on](#121-what-a-reader-may-rely-on)
  - [12.2 What is a compatible change](#122-what-is-a-compatible-change)
  - [12.3 What requires a version bump](#123-what-requires-a-version-bump)
  - [12.4 How the two are held together](#124-how-the-two-are-held-together)
- [13. The human report](#13-the-human-report)

## 1. What a plan is

PRD §2 states the problem: "reviewing what changed in the topology requires
reading router functions". A plan is the answer — the difference between two
compositions, as data, in four sections:

| section | the question it answers |
|---|---|
| components | what does this composition have that the other does not, and what is different about the ones both have |
| topology | what moved inside a flow's graph, in the state channels, and in the policy defaults |
| interfaces | what a **caller** feels: a flow's declared I/O, a trigger's route and response mode |
| validation | what is the compiler now saying that it was not, and what has it stopped saying |

Two properties decide everything else.

**The diff is over the resolved artifact, not over the files.** Both sides are
parsed, their imports followed, and every name bound, producing the flat IR of
PRD 5.1 — and the comparison is over *that*. Multi-file composition is authoring
UX, so a definition moved between files, an `imports:` list reordered, a comment
added, a mapping reflowed, and two definitions swapped in one file are changes to
files and to nothing the composition does. A plan reports none of them. What
follows from that is §10: source regions are taken out of the comparison before
anything is compared, and carried alongside as locations instead.

**A diagnostic is content, not a refusal.** The whole check phase runs over both
sides, and the difference between the two reports is §7. "The after spec
introduces three errors" is exactly the sentence a plan exists to say, and the
command exits `0` having said it. What the command cannot do is compare a spec
that has no artifact — one that does not parse, or does not resolve — and that is
§2.3.

**The target is `local` on both sides.** `plan` takes no `--target`: the question
it answers is what changed in the composition, and an artifact for one target
against an artifact for another answers a different one. The consequence for this
version of the format is in §11.

## 2. The two documents

Every document this command writes opens with `plan_version`, so one dispatch
reads either: a reader checks that key first, and refuses a version it does not
know rather than guessing.

### 2.1 The plan

`Plan`, the document a comparison produces.

| field | meaning |
|---|---|
| `plan_version` | the shape of this document — `1`, and the first key so a reader can dispatch on it |
| `before` | the composition compared **from**, as §2.2 describes it |
| `after` | the composition compared **to** |
| `components` | §4's array. Empty when nothing in it changed |
| `topology` | §5's array |
| `interfaces` | §6's array |
| `validation` | §7's object, which is present whether or not either of its arrays holds anything |

All four sections are always present. A plan over two compositions that agree is
four empty collections rather than an absent key, so a consumer parses one shape
whatever the outcome — and "no changes" is a document rather than the absence of
one.

### 2.2 Each side

`Spec`, under `before` and under `after`.

| field | meaning |
|---|---|
| `entrypoint` | the entrypoint as the command named it, which is also the path §10's locations are read against |
| `target` | the deploy target the composition was resolved for; `local` in this version (§1) |
| `spec_version` | the DSL version that composition declares (`version:` in its entrypoint) |

`entrypoint` is the command's own argument rather than the artifact's, and
deliberately: the IR records its entrypoint relative to the project root, which
makes it `main.yml` on both sides of nearly every comparison. What a reader needs
is the path that tells the two apart.

### 2.3 A spec that does not resolve

`Refusal`, written **instead of** a plan when one of the two specs has no
artifact to compare. The command exits `1`.

| field | meaning |
|---|---|
| `plan_version` | the same key a plan opens with, holding the same value |
| `failed` | one entry per spec that could not be resolved, in `before`, `after` order; never empty |

Each entry is a `Refused`:

| field | meaning |
|---|---|
| `spec` | which side it is: `"before"` or `"after"` |
| `entrypoint` | that spec's entrypoint, as the command named it |
| `diagnostics` | everything the parser and the resolver reported, in source order, in exactly the shape `agent-compose validate --format json` writes — the same `Diagnostic` records, with their codes, labels and help |

Both entries are present when both specs failed. A person comparing two branches
wants to know that neither of them resolves, not to find out one at a time.

`SpecSide` is the closed vocabulary `spec` draws on: `"before"` and `"after"`.

## 3. What a change record says

The three structural sections hold records of three different types, and all
three answer the same four questions: what happened, to what, where, and — when
the subject exists on both sides — which of its fields differ.

`ChangeKind` is what happened, and it is the same closed vocabulary in all three:

| member | meaning |
|---|---|
| `"added"` | the after spec declares the subject and the before spec does not |
| `"removed"` | the before spec declares it and the after spec does not |
| `"changed"` | both declare it, and something about it differs |

An added or removed subject carries an **empty** `fields` array: the subject
itself is the change, and expanding a flow that has just arrived into one record
per node it declares would bury the two edges that moved in the flow beside it.
For the same reason, §5 and §6 report only on subjects present in both specs —
what a new component contains is not a diff, it is the spec.

`FieldChange` is one field of one subject:

| field | meaning |
|---|---|
| `path` | where the field sits inside the resolved subject: keys joined with `.`, array elements as `[i]`, entries of a named array as `[<name>]` (§3's paragraph on arrays), relative to the subject |
| `before` | what the before spec declares there, as the value the IR holds. **Absent** when the field is not declared at all |
| `after` | the same for the after spec |

An absent `before` or `after` key means the field is **not declared** on that
side, which is not the same as one declared `null`. The distinction is load
bearing: an absent `timeout:` inherits the next level of grammar 9.3's resolution
chain, and a declared one does not.

Paths descend as far as they can name what they descended into. Two objects are
compared key by key, so a key on one side only is one change at that key. Two
arrays whose elements name themselves — a field map's `fields`, a binding list's
`entries`, a union's `variants`, a routed map's `routes` — are matched on that
name and compared entry by entry, which is what makes a schema that gained a
property one change at that property; their declaration order is not reported,
because all four are dispatched on by name. Every other array is compared element
by element when the two are the same length, so the lists whose order *is*
semantic — a model's `route:`, an `exec:`'s `args:`, an `enum:`'s variants —
report a move; when the lengths differ, the array is one change, because an
insertion shifts every index after it.

## 4. Components

`ComponentChange`: one declared thing added, removed, or changed.

| field | meaning |
|---|---|
| `change` | §3's vocabulary |
| `component` | which kind of component it is |
| `address` | its canonical address (§8) |
| `fields` | §3's field records; empty unless `change` is `"changed"` |
| `span` | where it is written (§10) |

`ComponentKind` is the closed vocabulary `component` draws on:

| member | what it names |
|---|---|
| `"agent"` | an `agent.*` definition |
| `"tool"` | a `tool.*` definition |
| `"flow"` | a `flow.*` definition |
| `"store"` | a `store.*` definition |
| `"provider"` | a `provider.*` definition |
| `"model"` | a `model.*` definition |
| `"trigger"` | an entry of `triggers:` |
| `"placement"` | an entry of the active target's `placements:` |
| `"event_source"` | an entry of the active target's `event_sources:` |

**What this section owns.** Every component arriving or leaving, and every field
of one that §5 and §6 do not own. The sections partition the artifact so that one
edit is one line: a flow's `inputs:` is part of its definition, part of what a
caller passes, and nothing to do with its graph, and printing it three times
would make a plan longer without making it say more.

Concretely, two components have fields held elsewhere:

* a **flow** reports only its `description:` here. Its `nodes:` and `edges:` are
  §5's, and its `inputs:`/`outputs:` are §6's;
* a **trigger** reports only `flow` and `description` here — which flow it runs,
  and what it is for. Its whole delivery surface is §6's.

Everything else — an agent, a tool, a store, a provider, a model, a placement, an
event source — reports every field of its resolved definition here.

One field of every definition is never reported: the `address` the artifact
repeats beside the key it sits under, so that a definition read on its own still
names itself. Two definitions are compared only when they sit at the same
address, so it is equal by construction.

## 5. Topology

`TopologyChange`: one node, edge, channel, or policy block.

| field | meaning |
|---|---|
| `change` | §3's vocabulary |
| `site` | which kind of site it is |
| `address` | its canonical address (§8) |
| `flow` | the flow the site belongs to. **Absent** on a channel and on the defaults, which are the composition's rather than any one flow's |
| `fields` | §3's field records; empty unless `change` is `"changed"` |
| `span` | where it is written (§10) |

`TopologyKind` is the closed vocabulary `site` draws on:

| member | what it names |
|---|---|
| `"node"` | one node of a flow |
| `"edge"` | one edge of a flow |
| `"channel"` | one `state:` channel |
| `"defaults"` | the composition's `defaults:` block |

A node's record covers everything a node declares: its kind and the block that
kind opens, its `input:` bindings, its `writes:` remap, its own
`retry`/`timeout`/`on_error`, and — on a `flow:` node — the `policy:` it hands
the instance inside it. A `map:` node's dispatch, its routes, its bound, and its
`detach:` are fields of that block, and are reported at the paths they sit at.

**Nodes are matched by id, edges by identity.** A node has a name and that is
what identifies it: a node renamed is one added and one removed, and a node whose
body changed is one `"changed"` record however far it moved in the file — the
order the `nodes:` mapping declares them in is not reported, because the graph is
the edges' and not that mapping's. An edge has no name, so what identifies it is
where it runs from, where it runs to, and what guards it — and an edit changes
exactly one of those. The match runs in three passes over what is still unpaired:
edges that agree in every field, then edges agreeing in `from` and `to` (the
guard, the `else:`, or the budget moved), then edges agreeing in `from` and guard
— which is an edge **retargeted**, the edit a pair of added/removed lines would
hide. What is left really arrived or really left.

One key an edge record can report is not a key of the artifact: `order`, the
edge's position among the outgoing edges of its own source node. Grammar 7.3
evaluates a node's outgoing edges in declaration order and takes the first whose
guard passes, so swapping two of them changes which one fires on a composition
where every edge is otherwise untouched; the IR carries that as list position,
and this is the plan's name for it. It counts per source node, because that is
what the rule is stated over — an edge inserted between two edges of a *different*
node changes nobody's precedence.

## 6. Interfaces

`InterfaceChange`: one caller-visible surface.

| field | meaning |
|---|---|
| `change` | §3's vocabulary. Always `"changed"` — a surface that arrived or left did so with its component, which is §4's to report |
| `surface` | which surface it is |
| `address` | the address of the flow or trigger whose surface it is (§8) |
| `fields` | §3's field records |
| `span` | where it is written (§10) |

`InterfaceKind` is the closed vocabulary `surface` draws on:

| member | what it names |
|---|---|
| `"flow"` | a flow's declared `inputs:` and `outputs:` — its module signature (grammar 7.5), which is also its signature as an agent's tool |
| `"trigger"` | a trigger's delivery surface: its type, its route and method, its response mode and timeout, its callback, its `session_key:`, and its `input:` bindings |

Paths on a flow's record are rooted at `inputs` or at `outputs`, so a reader
never has to ask which surface a change is on.

## 7. Validation

`Validation`, under the plan's `validation` key: what each side is told that the
other is not.

| field | meaning |
|---|---|
| `introduced` | what the **after** spec is told and the before spec is not |
| `resolved` | what the **before** spec is told and the after spec is not |

Each entry is a `Finding`:

| field | meaning |
|---|---|
| `code` | the stable machine-readable identity of the failure class, as `agent-compose validate` reports it |
| `severity` | `"error"` or `"warning"`, the same vocabulary a diagnostic carries |
| `message` | the one-line statement of what is wrong |
| `span` | where it is reported (§10) |

Both sides are given the **whole** report: what resolving the composition said —
which is warnings, since a composition with an error has no artifact to plan over
— and every static check. `code` and `severity` are the diagnostic vocabularies
`docs/grammar.md` Appendix B and `crates/compose-core/src/diag.rs` define, not
this format's; a code added there is not a change to this document.

A finding is the identifying half of a diagnostic and no more. A plan says
*which* diagnostics moved; `agent-compose validate` is where one is read in full,
with its labels, its help, and the snippet under it.

**Findings are matched by code and message, never by location.** A diagnostic's
span moves when anything above it in its file does, so a plan that keyed on one
would report an inserted comment as an error resolved and the same error
introduced two lines down — which is the exact class of noise §1 exists to
remove. What a diagnostic *says* is what identifies it: the message names the
construct it is about, and the code names the failure class. The match is a
multiset, so a composition that really is told the same thing twice reports two,
and one that grew a third reports one introduced.

## 8. Addresses

Every record names its subject by an address, and the spelling is fixed:

| subject | address |
|---|---|
| a definition | its typed address, as grammar 2.2 spells it: `agent.reviewer`, `flow.review_loop` |
| a trigger | `trigger.` and the name it is declared under: `trigger.on_request` |
| a placement | `placement.` and the component address it is keyed by: `placement.agent.fixer` |
| an event source | `event_source.` and its logical name: `event_source.bug_reports` |
| a node | the flow's address, a `.`, and the flow-local node id: `flow.review_loop.draft` |
| an edge | the flow's address, a `.`, the source, `->`, and the target: `flow.review_loop.draft->review` |
| a channel | `state.` and the channel name: `state.verdict` |
| the policy defaults | `defaults` |

The first is the DSL's own; the rest are this format's spelling for things the
DSL names without a global address. A node id, a channel name and a trigger name
are identifiers (grammar 2.1), so none of these spellings is ambiguous with
another.

Two edges of one flow may run between the same pair of nodes under different
guards, so an edge address is not unique on its own. The second and further
records carrying one take a `#2`, `#3` suffix, assigned in the order the records
are built — after edges in declaration order, then before edges in theirs — which
is fixed by the two artifacts rather than by the diff.

## 9. Ordering

Byte-identical output for a byte-identical pair, every run (PRD 5.12). Nothing in
a plan is ordered by the order anything was visited in.

* **Sections** are in the order §4–§7 gives them, which is the order they appear
  in the document.
* **components** and **interfaces** are sorted by `address`, in UTF-8 byte order.
* **topology** is sorted by the site's scope — the `flow` for a node or an edge,
  the `address` itself for a channel and for the defaults — then nodes before
  edges, then by `address`. So one flow's changes read together, its nodes before
  the edges between them.
* **`fields`** within one record are in path order, which is the sorted key order
  of the objects they were found in.
* **`introduced`** and **`resolved`** are each in the source order their own
  side's report is written in: by file, then by position, then by code.

## 10. Locations

Every structural record carries a `span`, written the way the IR and every
diagnostic write one — `<file>:<line>:<col>..<line>:<col>`, the file relative to
the **project root**, which is that composition's entrypoint's own directory
(grammar 1.4).

Which of the two compositions the file belongs to follows from the record, and
this is the whole rule:

| record | its `span` is in |
|---|---|
| `change` is `"added"` or `"changed"` | the **after** spec |
| `change` is `"removed"` | the **before** spec |
| a finding under `introduced` | the **after** spec |
| a finding under `resolved` | the **before** spec |

So a consumer joining a span back onto a checkout takes the directory of
`after.entrypoint` or of `before.entrypoint` accordingly. Both sides commonly
name a file `main.yml`, and the span alone does not say which tree it is in — the
table above is what does.

Spans are **not** part of the comparison. Two compositions that differ only in
where their constructs are written produce an empty plan; the spans are carried
so that a reader can go and look, not so that they can be diffed.

## 11. What is not compared

Four things in the artifact are outside this version of the format, and each for
a reason:

* **`sources`** — the list of files the composition was read from. It is the one
  part of the IR that is *about* file layout, which §1 excludes by construction.
* **`entrypoint`**, **`ir_version`** — the first is `main.yml` on both sides of
  nearly every comparison (§2.2 carries the useful one instead), and the second is
  the same value on both sides by construction: one compiler produced both
  artifacts.
* **`storage_backends:`** — the deploy layer's backend bindings. `plan` resolves
  the built-in `local` target on both sides (§1), and grammar 14 makes
  `storage_backends:` a compile error under `local` (Decision D87), so no
  artifact this command can build carries one. The two reserved sections `local`
  *does* admit — `placements:` and `event_sources:` — are compared, and appear in
  §4.
* **a declared-but-empty section**, as against an absent one. The IR draws that
  distinction — `state:` written with no channels is not `state:` unwritten — and
  a plan reports the channels, the triggers and the placements themselves rather
  than the sections that hold them. The difference is invisible to a plan and
  visible to `validate`, which is the command that has a rule about it.

## 12. Stability

### 12.1 What a reader may rely on

* the document opens with `plan_version`, and so does the refusal;
* the four sections, their names, and the fact that all four are always present;
* the field names and meanings in §2–§7;
* the closed vocabularies — the members `change`, `component`, `site`,
  `surface` and `spec` draw on — hold exactly what §3–§7 list, so a reader may
  exhaust them. `code` and `severity` are the compiler's own vocabularies and
  grow with it, which §7 says;
* the address spellings of §8;
* the ordering of §9, and the byte-identical output it produces;
* the location rule of §10;
* an absent `before`/`after` key on a field change meaning **not declared** (§3);
* the exit codes: `0` when a plan was produced, whatever it says; `1` when a spec
  did not resolve; `2` when the command could not run.

### 12.2 What is a compatible change

Adding a **key** that was previously absent, to any record here: a reader of the
older shape does not look for it and is unaffected. Adding a section of the plan
document is the same thing at the top level.

Comparing something that was not compared before — a field of a definition that
this version delegates nowhere and therefore never reported — is also compatible.
It produces records of a shape a reader already parses.

Improving a `message`, which is a diagnostic's own text and is free to get
better; §7 makes the pair `code`/`message` the identity of a finding *within one
comparison*, not across compiler releases.

### 12.3 What requires a version bump

* removing or renaming any key, or changing what one means;
* adding a member to any of the five closed vocabularies of §12.1, or changing
  a member's spelling — a reader is allowed to exhaust them;
* changing an address spelling in §8, which a reader may be keying its own
  records off;
* changing the ordering of §9, or the location rule of §10;
* changing which section owns a field (§4), because a reader watching one section
  would stop seeing an edit it was watching for.

### 12.4 How the two are held together

`crates/compose-core/tests/plan_format_inventory.rs` reads the record types out
of `crates/compose-core/src/plan/` and holds each of them to this file: every
type must be introduced in exactly one section, every field must have a row in
that section, and every member of a closed vocabulary must have a row spelled
with its JSON quotes. It also pins the version above to the constant the
compiler emits, so a bump moves both or neither.

What that check cannot decide is whether a sentence here is *true*. That is what
`crates/agent-compose/tests/plan_cli.rs` is for: it pins whole documents,
byte for byte, over a corpus of spec pairs — a pair that differs only in
formatting, a rename, a topology edit, a policy edit, a surface edit, a deploy
edit, and a pair where the after spec introduces errors.

## 13. The human report

`--format human` is the default, and it is a **report** rather than a contract:
it is written for a person, it is not covered by §12, and nothing should be
parsed out of it. It goes to stderr, and `--format json` goes to stdout, which is
the split every verb of this CLI makes.

It is a list, one line per change, marked `+` for added, `-` for removed and `~`
for changed, grouped under the name of the section that owns it — with a section
that has nothing to say left out entirely, where the JSON writes an empty array.
Each line names its subject's address and where to look, printed against the root
it belongs to (§10) so that every path in the report opens from the directory the
command ran in. A changed subject is followed by one indented line per field,
`path: before -> after`, where a field the spec does not declare reads
`(absent)`.

Values are written as compact JSON and **cut** at 48 characters, with a `…` and
no closing quote so that a cut is visible rather than plausible: a changed
`prompt:` is a paragraph, and a report that printed both copies of it in full
would be unreadable for the one line it was run to find. `--format json` carries
every value whole.

The closing line is the verdict, and every run prints one: either the two specs
describe the same composition, or they differ — followed by a count per section.
