# state

`state:` declares the graph's channels. Data that has to travel between nodes
goes through one — node configuration never reads another node's output, which
is what keeps node configs order-independent.

Wiring is **name-based in both directions**: a node's output field lands on the
channel of the same name, and a node's input field resolves from the channel of
the same name. `writes:` remaps the write side; `input:` bindings remap the read
side.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5

agent.reviewer:
  model: model.m
  prompt: Review the draft.
  input:
    draft: { type: string }
  output:
    verdict:  { enum: [approve, revise] }
    feedback: { type: string }

state:
  draft:    { type: string, default: "" }
  feedback: { type: string, default: "" }
  patches:
    type: array
    max_items: 50
    items: { type: string }
    reduce: append
  totals:
    type: object
    properties:
      fixed:   { type: integer }
      skipped: { type: integer }
    reduce: merge
    default: { fixed: 0, skipped: 0 }

flow.review:
  outputs:
    feedback: { type: string }
  nodes:
    review:
      agent: agent.reviewer
      input: { draft: "state.draft" }       # explicit read
      writes: { feedback: feedback }        # explicit write remap
  edges:
    - { from: start, to: review }
    - { from: review, to: end }
```

## Channels

A channel is a type node plus two channel-only keys:

| Key | Required | Notes |
|---|---|---|
| `reduce` | no | `append` \| `merge` \| `last_wins`; absent means *unreduced* |
| `default` | no | the initial value; illegal on a union channel |

Channel names are identifiers and must not be reserved — `input`, `state`,
`execution`, `item`, `messages`, `output`, `payload` are all illegal here.

The channel *set* is composition-global in **shape**; each flow instance holds
its own **values**, so two flows may both use `draft` without interfering.

**Initial values.** A channel starts each instance at its `default:`. With none
declared, an `append` channel starts `[]` and a `merge` channel `{}` — the
identity element, which is what makes a fan-out of zero items read as "nothing
yet" rather than as an error. **Every other channel is unset**, and reading an
unset channel — from CEL, from a name-based input binding, or from a flow's
`outputs:` materialization — **fails the execution** naming the channel and the
reader. Declaring a `default:` is how a channel becomes readable before its
first write, which is usually what a convergence reached by only one of two
branches wants.

**`merge` channels are per-property.** Each write may supply a subset, so a
`merge` channel holds `{}` before the first write. Reading a property it does
not currently hold fails the execution, naming the channel, the property, and
the reader. Two fixes: declare a `default:` supplying every property (one
`default:` makes the whole channel total from step 0), or ask with `has()` —
`when: "has(state.totals.skipped) && state.totals.skipped > 0"`. Type-checking
is unaffected either way: `state.totals.skipped` is an `integer` wherever it is
legal to read.

## Reduce policies

| Policy | Requires | Semantics |
|---|---|---|
| `append` | `type: array` | each write contributes **one element**, appended in canonical write order |
| `merge` | `type: object` | shallow key-wise merge; the last writer in canonical order wins a conflicting key |
| `last_wins` | any | the last write in canonical order wins, declared as concurrency-safe |

All three are defined against the **canonical write order** — writers of a step
ordered by node id, a map's instances by source-item index — never against
completion order. Two runs over the same inputs leave every channel holding the
same value.

A channel **without** `reduce:` is single-writer and sequential. Writing one
from inside a `map`, or from two concurrent nodes, is `unreduced-write`.
`reduce: last_wins` is how you opt into concurrent overwrite explicitly.

**What a write supplies** follows from the policy, and is type-checked from
declared types alone:

| Channel | A write supplies | Accepted type |
|---|---|---|
| unreduced | the whole value | the channel's type |
| `last_wins` | the whole value | the channel's type |
| `append` | one element | the channel's `items` type |
| `merge` | a partial object | properties a subset of the channel's, types matching |

So an `append` channel of `items: {type: string}` accepts a write of a
**string**, never of an `array<string>` — which is exactly what makes fan-in
work, one element per instance. Appending several values in one write is
deliberately not expressible: fan out with a `map`, or declare the channel
`last_wins` and set it whole.

## Reading: the resolution chains

For **in-flow targets** (`agent:`, `exec:`, `http:`, `function:`, `human:`):

1. an explicit `input:` binding for that field;
2. otherwise the state channel of the same name;
3. otherwise the enclosing flow input of the same name;
4. otherwise the field's own `default:`;
5. otherwise `missing-binding`.

For **module-boundary targets** (`flow:` nodes and `map` dispatch), steps 2 and
3 **do not apply**: an explicit binding, then the field's `default:`, then a
compile error. Name-based wiring is a convenience *within* one flow's scope, and
nothing crosses a module boundary implicitly.

A `store:` node has no `input:` key at all — its parameters are the op's own
row, and nothing falls through by name.

Steps 2 and 3 put no expression between the two declarations, so the source must
**satisfy** the field: every value the channel can hold must be a legal value of
the field's type, or it is a compile error naming both.

## Writing, and `writes:`

After a node completes, each output field **the result carries** is written to
the channel of the same name **if one is declared**; fields with no matching
channel stay node-scoped and remain readable as `<node>.output.<field>` by that
node's edge guards and by `map.over`.

```yaml
review:
  agent: agent.reviewer
  writes: { feedback: reviewer_feedback }   # output field -> channel
```

Keys must be output field names; values must be declared channels. A node's
**effective write map** — every output field paired with the channel it actually
writes, name-based destinations included — must be **injective**, or two writes
from one writer would land on one channel with no order between them
(`conflicting-writes`). Both spellings are caught: two remaps onto one channel,
and a remap landing on a sibling's name-based destination.

A remapped field is not also written to its same-named channel. A field the
result does not carry performs **no** write — the channel keeps what it held.

Writing one channel from concurrent contexts requires a `reduce:` policy.

## A flow's outputs

Each field of `outputs:` is read from the channel of the same name at
quiescence, and that channel must be declared or it is `undefined-channel`.
There is no `returns:` binding: use a `writes:` remap to feed a
differently-named channel.

## Conversation history

An implicit append-only channel named `messages` carries conversation history
for agent nodes. It must **not** be declared in `state:` and must not be named
in `writes:`. It is isolated across flow boundaries by default; a `flow:` node
opts into sharing the caller's history with `context: inherit`.

A `map` dispatch is a module boundary for history **with no opt-in**: every
instance runs on a fresh history that is discarded when it completes.

Normative source: `docs/grammar.md` §8.0, §10, §10.1–10.4
