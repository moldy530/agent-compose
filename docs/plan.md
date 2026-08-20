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
anything is compared, and carried alongside as locations instead. What it does
*not* cover is a declaration order the composition behaves differently for, which
§3 reports and §11 draws the line for.

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
| `entrypoint` | the entrypoint the command resolved — the file it was given, or the `main.yml` inside the directory it was given — which is also the path §10's locations are read against |
| `target` | the deploy target the composition was resolved for; `local` in this version (§1) |
| `spec_version` | the DSL version that composition declares (`version:` in its entrypoint) |

`entrypoint` is read off the command's own argument rather than off the
artifact, and deliberately: the IR records its entrypoint relative to the project
root, which makes it `main.yml` on both sides of nearly every comparison. What a
reader needs is the path that tells the two apart. Each side may be named by
either spelling and the plan is the same, so a side handed `renamed-model/before`
is written `renamed-model/before/main.yml` here — the file, never the directory.

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
| `entrypoint` | that spec's entrypoint, resolved the way §2.2's is |
| `diagnostics` | everything the parser and the resolver reported, in source order, in exactly the shape `agent-compose validate --format json` writes — the same `Diagnostic` records, with their codes, labels and help |

Both entries are present when both specs failed. A person comparing two branches
wants to know that neither of them resolves, not to find out one at a time.

`SpecSide` is the closed vocabulary `spec` draws on:

| member | what it names |
|---|---|
| `"before"` | the composition compared **from** |
| `"after"` | the composition compared **to** |

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
arrays under one of the six keys the grammar gives **named** entries are matched
on that name and compared entry by entry, which is what makes a schema that
gained a property one change at that property. That set is closed, and this is
all of it:

| key | what it holds | the name an entry carries |
|---|---|---|
| `fields` | a field map's properties (grammar 3.1) | `name` |
| `variants` | a discriminated union's variants (grammar 3.7) | `tag` |
| `entries` | a binding map's bindings (grammar 8.0), or a `writes:` remap's | `name`, `field` |
| `env` | an `exec:` block's environment (grammar 6.1, 8.2) | `name` |
| `headers` | an `http:` block's or a provider's headers (grammar 6.1, 8.3, 12.1) | `name` |
| `routes` | a routed `map:`'s destinations (grammar 8.6) | `tag` |

Every other array is compared element by element when the two are the same
length, so the lists whose order *is* semantic — a model's `route:`, an
`exec:`'s `args:`, an `enum:`'s variants — report a move; when the lengths
differ, the array is one change, because an insertion shifts every index after
it.

**The key decides, never the shape of the elements.** Two surfaces of a
composition hold author-written data under author-chosen keys — a schema's
`default:` and a model's `settings:` — and a rule that read an array's elements
rather than the key above them would take a `default: [{name: one}, {name: two}]`
for a keyed map and report its reversal as no change at all. It is a different
literal, handed to every caller, so it is compared by position like any other
value and reports as `default[0].name` and `default[1].name`. The `[<name>]`
spelling above therefore always means one of the six.

**A declaration order is reported when the composition behaves differently for
it.** Two of the named arrays do: a field map's `fields` is the order of a JSON
Schema's `properties` and of its `required`, and a union's `variants` is the
order of its `oneOf` — both of which are handed to the model as written. An entry
of one of those whose position moved carries one extra field record, whose `path`
is the entry's own path with `.order` on the end
(`output.fields[verdict].order`), holding its position on each side as a number.
Positions are counted over the names **both**
specs declare, so an entry inserted ahead of others is one addition rather than a
move of everything below it. `order` is this format's key rather than the
artifact's; §5 gives an edge one for the same reason.

Four keys are the opposite of that: `optional:`, `expect_exit:`, `expect_status:`
and `route_on:` are parsed as distinct memberships and membership-tested at run
time, so their order is not compared at all and a spec that only reshuffles one
of them has changed nothing. `route_on:` is the one worth naming beside its own
neighbour: a model's `route:` is the order its members are **tried** in and is
reported, while the `route_on:` next to it is the set of conditions that decide
whether to try the next member at all, and is not. The remaining named arrays — a
node's `input:` bindings, a `writes:` remap, an `env:` or `headers:` map, a
routed map's `routes:` — are dispatched on by name, and §11 is where their order
is accounted for.

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

One field of every entry is never reported: the key the artifact repeats
**inside** the value, so that an entry read on its own still names what it is.
Two entries are compared only when they sit under the same key, so that repeat
is equal by construction, and a record naming it would be noise on every change.
Which field it is depends on what the entry is, and this is all of them:

| entry | the field that repeats its key |
|---|---|
| a definition | `address` |
| a placement | `address` |
| an event source | `name` |
| a trigger | `name` |
| a `state:` channel (§5) | `name` |

A definition's `namespace` — the tag its body is written under (grammar 2.2) — is
equal by construction for the same reason: it follows from the address. It is
compared like any other field and can never differ.

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
The one field of a node never reported is its `id`, which is the key it is
matched by; a channel's is its `name`, for the reason §4 gives about every entry
that repeats its own key.

**Nodes are matched by id, edges by identity.** A node has a name and that is
what identifies it: a node renamed is one added and one removed, and a node whose
body changed is one `"changed"` record however far it moved in the file — the
order the `nodes:` mapping declares them in is not reported, because the graph is
the edges' and not that mapping's. An edge has no name, so what identifies it is
where it runs from, where it runs to, and what guards it — and an edit changes
exactly one of those. The match runs in three passes over what is still unpaired:
edges that agree in every field **of the artifact**, then edges agreeing in
`from` and `to` (the guard, the `else:`, or the budget moved), then edges
agreeing in `from` and guard — which is an edge **retargeted**, the edit a pair of
added/removed lines would hide. What is left really arrived or really left.

One key an edge record can report is not a key of the artifact: `order`, the
edge's position among the outgoing edges of its own source node. Grammar 7.3
rule 1 evaluates a node's outgoing edges **in declaration order**; the IR carries
that as list position, and this is the plan's name for it.

**It is not which edge fires, and must not be read that way.** Grammar 7.3 rule 6
fires **all** taken edges — routing is multicast with an `else:`, not
first-match-wins (grammar Decision D17) — and rule 4 decides an `else:` edge
against whether *any* guarded sibling was taken rather than against one of them
in particular. So no reordering of a node's out-edges changes which of them are
taken, and none changes what a run writes either: concurrent writers are ordered
by node id (Decision D72). What a node's declaration order does decide is two
things, and each is worth the line a plan spends on it:

* **the order the decision is recorded in.** A trace entry's `edges` array holds
  "what every outgoing edge answered, in **declaration order**", and its
  `targets` are the taken ones "in the declaration order of the edges that
  reached them" (`docs/trace.md` §4, §4.1). That is a machine surface with a
  version of its own, and a swap rewrites it;
* **which of two unevaluable guards fails the run.** The guards are evaluated in
  that order and evaluation stops at the first one that throws, so a swap can
  change the expression the run dies naming.

Two rules say what `order` counts:

* **per source node**, because that is what grammar 7.3 is stated over, and what
  a trace's routing decision is one of — an edge moved past an edge of a
  *different* node leaves both nodes' decisions reading exactly as they did. The
  position of an edge in the flow's `edges:` list *as a whole* is a different
  number, and §11 is where the one thing it decides is accounted for;
* **over the edges both specs declare**, which is §3's rule for a named sequence
  applied here — an edge inserted ahead of others is one addition rather than a
  move of everything below it, and a genuine swap still reports, because a swap
  moves an edge past another edge that is also on both sides.

A position is therefore **not** part of what identifies an edge: it is compared
after the pairing above is settled, not during it. Folding it in would defeat the
first pass — an untouched edge that sits one place lower would fail to match
itself, the genuinely new edge beside it would be paired with it by `from`/`to`
instead, and the plan would report an arrival as a guard edit on an edge nobody
touched.

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
never has to ask which surface a change is on. A trigger's `name` is not on its
record here either, for §4's reason: it repeats the key the trigger is declared
under.

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
remove. What a diagnostic *says* is what identifies it: the code names the
failure class, and the message states what is wrong. The match is a multiset, so
a composition that really is told the same thing twice reports two, and one that
grew a third reports one introduced.

That is a trade with a residual, and this is it. Most messages name the construct
they are about — `` node `merge` of `flow.diamond` … `` — but the compiler does
not guarantee it, and some are stated about the offending text alone:
`unknown-root` reads `` `bogus` is not a root in scope here `` and names no
construct. So one failure that *moved* from one construct to another — fixed in
`flow.first`, introduced identically in `flow.second` — is one finding on each
side that reads the same, matches itself, and leaves both `introduced` and
`resolved` empty. `agent-compose validate` reports an error on each side; a plan
says the verdict did not move. The structural sections still carry both edits, so
what a reader loses is the validator's verdict having moved with them — and the
alternative is keying on a location, which costs the whole class of noise this
paragraph opens with. The trade is taken deliberately, and stated here rather
than left to be discovered.

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
* **`fields`** within one record are in the order the comparison walks them: an
  object's keys sorted, a positional array's elements by index, and a named
  array's entries by name, sorted. That is not the same as sorting the `path`
  strings — index `[9]` is walked before `[10]`, which sorts after it — so a
  consumer reproducing the order walks the subject rather than re-sorting the
  paths. Either way it is decided by the two artifacts and by nothing else.
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

Eight things in the artifact are outside this version of the format, and each for
a reason:

* **an ordering nothing dispatches on.** This is the one item here that is a
  judgement rather than a gap, and it is worth reading before relying on the
  verdict line. §3 reports the declaration order of a field map, of a union, and
  of a model's `route:`, because each of those decides something. It does not
  report the order of the arrays the generated code looks entries up in **by
  name** — a node's `input:` bindings, a `writes:` remap, an `env:` or `headers:`
  map, a routed map's `routes:`, and the `nodes:` mapping of a flow, whose graph
  is its edges' and not that mapping's (§5) — nor of the four sets `optional:`,
  `expect_exit:`, `expect_status:` and `route_on:`. Reshuffling one of those is a
  change to the *layout* of the generated project (a route descriptor moves in
  `src/graph.ts`, an `expectExit: [0, 1]` literal is written `[1, 0]`, and the
  failure message that quotes it back reads in the new order) and to nothing the
  composition decides: every one of them is selected by name or tested for
  membership. A plan is a diff of compositions, so it stays silent, and
  `agent-compose build --check` is the command that notices a generated file
  whose bytes moved.

  What this rule is stated over is the **key** an array sits under, never the
  shape of its elements — §3's table is the closed list — so an author's own
  array of objects in a `default:` or a `settings:` is not one of these, whatever
  its elements are called. Reversing one is a different literal value, and it
  reports.

  A flow's `edges:` is the one array this bullet and §5 split between them. §5
  reports an edge's position among the outgoing edges of **its own source node**,
  which is the order grammar 7.3 rule 1 is stated over. Its position in the
  flow's `edges:` list *as a whole* is not reported, and one generated identifier
  reads off it: a budgeted edge's `max_iterations` counter is keyed
  `<flow>#<index>` by that position
  (`crates/compose-core/src/codegen/graph.rs`). Moving an edge past an edge of a
  different node respells that key — in `src/graph.ts`, and in the `budget.key`
  and `counters` a trace writes — while leaving one counter per budgeted edge and
  every spend against the same edge. It is a generated name rather than something
  the composition decides, so it lands in this bullet with the rest.

  The paragraph before that one is about an author's own **array**. Its
  *mapping* is the bullet below, and the two come out opposite ways;

* **the key order of an author's own mapping.** The same open surfaces hold
  literal mappings — a `default: { alpha: 1, beta: 2 }`, a model's `settings:`,
  a deploy backend's plugin config — and the composition does record the order
  their keys were written in: `agent-compose build` emits
  `.default({ "alpha": 1, "beta": 2 })` in that order, so rewriting the same two
  keys the other way round moves bytes in `src/schemas.ts` and in `src/state.ts`.
  A plan is silent about it. The comparison is over the artifact **as JSON**
  (§1), where an object is a set of keys with values rather than a sequence of
  pairs, so the two mappings are already one value before anything is compared.

  It belongs in this section rather than among the three below for the first
  bullet's reason: what a caller looks up under a key is the same on both sides,
  and what reads differently is generated text — the emitted literal, and
  anything that walks it in insertion order, exactly as an `expectExit: [1, 0]`
  failure message does. `agent-compose build --check` is again the command that
  notices.

  Read it directly against the rule for an author's **array**, which is the
  opposite verdict on the neighbouring construct, and note that the reason does
  not carry over: reversing `[{name: alpha}, {name: beta}]` hands every caller a
  different *value*, because an array's positions are part of what it is;

* **`sources`** — the list of files the composition was read from. It is the one
  part of the IR that is *about* file layout, which §1 excludes by construction.
* **`deploy.source`** — the deploy file the active target's layer was read from,
  for the same reason: it is a file path, and which file a layer is written in is
  not something the composition does. What the layer *declares* is compared, and
  appears in §4. (`deploy.target` is likewise uncompared and needs no entry here:
  §2.2's `target` carries it, on both sides.)
* **`entrypoint`**, **`ir_version`** — the first is `main.yml` on both sides of
  nearly every comparison (§2.2 carries the useful one instead), and the second is
  the same value on both sides by construction: one compiler produced both
  artifacts.
* **`spec_version`** — the DSL version the composition declares. It is
  **carried** rather than compared: §2.2 writes it on each side, and a reader
  that cares compares `before.spec_version` against `after.spec_version` itself.
  No section holds it, so it does not reach `components` and does not move the
  verdict line of §13. This compiler supports one version, so no pair of
  resolvable specs can differ in it; the day a second ships, a bump on its own
  will read as "the same composition" until this rule is revisited, and revisiting
  it is a version bump under §12.3.
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
  `crates/compose-core/tests/plan_completeness.rs` restates this clause as
  `SECTIONS` and holds it to an actual edit, the way it does the rest.

Three things a plan **does** report read as more than they are, and they belong
here for the same reason the list above does: this section is what a plan will
and will not tell you.

**Three leaves are compared as the text they were written as.** A `timeout:`, a
`when:` guard, and a numeric schema bound reach the artifact as the author's own
spelling — a duration and a CEL expression are written out verbatim, and a number
keeps whether it was written as an integer or as a float. So each of these is a
reported change, on a pair the *decided* value is equal across:

| written | against | reported as |
|---|---|---|
| `timeout: 120s` | `timeout: 2m` | `timeout: "120s" -> "2m"` |
| `when: "a == b"` | `when: "a==b"` | `when: "a == b" -> "a==b"` |
| `minimum: 1` | `minimum: 1.0` | `minimum: 1 -> 1.0` |

That is the consequence of §1's first property rather than an oversight. The
comparison is over the resolved artifact, the artifact records what the
composition *says* rather than what a later pass makes of it, and one of the
three reaches the emitted project as written — a guard's CEL text is the string
in `src/graph.ts` and the text a trace quotes back, so respelling it is a change
to what is shipped. The other two are not: `agent-compose build` emits a
byte-identical project for `120s` and `2m`, and for `1` and `1.0`, because
codegen reads the milliseconds and the number rather than the spelling. Deciding
a duration from a string would mean reading
the key above it to know when to try, and a key-gated rule over `settings:` and
`default:` is the mistake §3's closing paragraph refuses — a
`settings: { timeout: "2m" }` is data, and normalizing it would hide an edit
rather than suppress a non-edit.

**A default written out is a change.** The artifact records what the composition
declares, and a default is not materialized into it
(`crates/compose-core/src/ir`): a key an author leaves out is a key the artifact
does not hold, and the same key written with the value the grammar already
supplies is a key it does. So each of these is a reported change as well, on a
pair `agent-compose build` emits a byte-identical project for:

| left out | written out | reported as |
|---|---|---|
| `expect_exit:` absent | `expect_exit: [0]` (grammar 6.1) | `exec.expect_exit: (absent) -> [0]` |
| `max_tool_iterations:` absent | `max_tool_iterations: 8` (Decision D51) | `max_tool_iterations: (absent) -> 8` |
| `method:` absent | `method: POST` (grammar 13.3) | `method: (absent) -> "POST"` |

The class is not these three: it is every key `docs/grammar.md` gives a default —
`context: isolated`, `as: item`, `on_item_error: fail`, `detach: false`,
`respond: async`, `unique_items: false`, `agent_access: read_write`,
`timezone: UTC` and the rest all read the same way.

§3's rule that an absent `before` or `after` key means **not declared** is that
representation seen from the format's side, and §3's example is worth reading
against this table rather than as a case of it: an absent `timeout:` is not the
same as one written with the value it would have inherited. Grammar 9.3 resolves
a policy key through four levels, and declaring it at a node takes that node out
of the chain — the emitted project records the value as resolved from the node
rather than from `defaults:`, and a later edit to `defaults:` no longer reaches
it. A key of the table above is not like that. Telling the two apart inside the
comparison would need a table of keys and their defaults consulted **by key**,
and it would have to run over `settings:` and `default:` as well — where an
author's own `method: POST` is data — which is the rule §3's closing paragraph
refuses.

**One edit to an interpolated string is two field records.** A string that may
embed an environment reference reaches the artifact as the text the author wrote
**and** as the list of names it embeds (`crates/compose-core/src/ir/leaf.rs`),
because the list is the authoritative one and the text is not: an escaped
`$${NAME}` stands in the text and names nothing. Renaming a reference edits both,
so a `cwd: "${REPO_ROOT}"` rewritten to `cwd: "${REPO_ROOT2}"` reports at
`exec.cwd.env_refs[0]` and at `exec.cwd.text` — two records, two lines of the
human report, one edit. Suppressing either would hide the one case where the two
disagree, so a reader reads the pair as the rename it is.

None of the three is a plan being wrong about the artifact, and that is the
distinction to hold on to: a plan reports the artifact. A reviewer branching on
`plan` should read the first two the way they read a reformatted comment — real
in the artifact, and nothing the composition does differently, the one exception
being the respelled guard above, whose text is what is shipped — and the third as
one edit written twice.

**How much of this section is held to behaviour.**
`crates/compose-core/tests/plan_completeness.rs` restates every clause above
independently of the compiler and asserts the account as a biconditional: a plan
names a subject, and a place inside it, exactly when the two artifacts differ
there once this section is taken out. That is what keeps the section from
drifting away from what the command does — but the property is stated over the
artifacts two corpora of spec pairs produce, and those are finite. A key an
author leaves out is a key the artifact does not hold (see *a default written
out*, above), so a construct **no spec of either corpus declares** is a construct
nothing there would notice a plan going quiet about. Of the 111 optional keys
`schemas/agent-compose.schema.json` names, 39 are in that position today: about
half are a deploy backend's plugin config, which this section excuses outright,
and the rest are real — a model's `top_p:`, `seed:` and `thinking:`, a schema's
`min_items:`, `multiple_of:` and its two exclusive bounds, a store op's
`filter:`, `metadata:` and `top_k:`. It holds per construct rather than per key
name: a `description:` is declared by every kind of subject in the corpora except
a trigger, so a trigger's is the one the property does not currently reach.

Everything §3 through §10 promise applies to those keys the same way — the
comparison is over the artifact as JSON and has no table of keys in it — and this
paragraph is about the *test* corpus rather than about the format. Growing the
corpora is what closes it, and the test file's module docs say so beside the
property.

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
**that type's own table** — the first table after the sentence naming it, not
merely somewhere under the same heading, because most sections here specify two
types and their two tables sit side by side — and every member of a closed
vocabulary must have a row spelled with its JSON quotes. It also pins the version
above to the constant the compiler emits, so a bump moves both or neither.

What that check cannot decide is whether a sentence here is *true*. Two files
answer that, from opposite ends.

`crates/agent-compose/tests/plan_cli.rs` pins whole documents, byte for byte,
over a corpus of spec pairs — a pair that differs only in formatting, a rename, a
topology edit, an edge-order edit, a policy edit, a surface edit, a deploy edit, a
pair that only reorders declarations, a pair that only respells three leaves, a
pair that only writes four of the grammar's own defaults out — and builds both
sides to prove it decides nothing differently — a pair whose one edit falls past
the end of a report line, and a pair where the after spec introduces errors.

`crates/compose-core/tests/plan_completeness.rs` states §4–§6 and §11 as a
property, which is what holds the **silences** to them: a plan names a subject,
and one of that subject's keys, if and only if the two artifacts differ there
once everything §11 excuses is taken out of them. A golden can only assert lines
that are there; a rule that swallowed a whole class of edit would produce no line
for any golden to miss, so the property is stated over one composition edited one
construct at a time — an edit this document does not excuse and no record names
fails it, and so does a record naming an edit this document says is not compared.

Because that property is stated key for key, the corpora are its coverage: it
catches a field that stopped being compared on the pairs whose two sides differ
in *that field*. So the same file also asserts that every key of the base
composition's artifact is moved by some pair, which is what makes a key added to
the IR arrive with a case that exercises it rather than with a silence nothing
would notice.

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

Values are written as compact JSON and **cut** to 48 characters, with a `…` and
no closing quote so that a cut is visible rather than plausible: a changed
`prompt:` is a paragraph, and a report that printed both copies of it in full
would be unreadable for the one line it was run to find.

What the cut may not do is hide the change. Two values agreeing for more than a
line — a prompt reworded at its end — would both cut to the same text, and the
one line the command was run to produce would read as a non-change. So the window
**moves**: when the first difference falls past the end of the cut, both sides
are printed from a few characters ahead of it instead, with a leading `…` saying
so. It is the same window on both sides, so the two still read against each
other. A report that cut anything says so in one note above the verdict, and
points at `--format json`, which carries every value whole.

The closing line is the verdict, and every run prints one: either the two specs
describe the same composition, or they differ — followed by a count per section.
