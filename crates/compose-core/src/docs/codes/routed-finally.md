# routed-finally

## What it protects

A flow's `finally:` node is **outside routing**. What runs it is the instance
ending — once, whether the instance completed, failed or was cancelled — not a
branch arriving at it, so:

- **no edge may name it**, at either end: an edge *into* it would run it a
  second time, mid-instance, with no outcome to read, and an edge *out of* it
  would continue a branch the instance has already finished with;
- **no control transfer may name it** — an `on_error: { fallback: … }`, a
  `human.on_timeout:` — for the same reason as an edge into it;
- **it declares no `on_error:`**. Its failure has exactly one meaning: once its
  own `retry:` and `timeout:` are spent, the error is recorded as
  `finally_error` on the execution's record and the instance's outcome stays
  what it was. `skip` and `fallback` would route out of a node that routes
  nowhere.

It needs no outgoing edge — it is the one node "every node exits" does not apply
to — and it is reachable because it is `finally:`.

## A spec that triggers it

```yaml triggers
version: "0.1"
flow.f:
  outputs: {}
  nodes:
    work:
      exec: { command: "true" }
    cleanup:
      exec: { command: "true" }
  edges:
    - { from: start, to: work }
    - { from: work, to: cleanup }
    - { from: cleanup, to: end }
  finally: cleanup
```

## The fix

**Delete the edges that name it**, and route the branch to `end` where it is
done. The finally node runs when the instance ends, which is after the branch
reaches `end`:

```yaml spec
version: "0.1"
flow.f:
  outputs: {}
  nodes:
    work:
      exec: { command: "true" }
    cleanup:
      exec: { command: "true" }
      retry: { max: 2, backoff: 1s }
  edges:
    - { from: start, to: work }
    - { from: work, to: end }
  finally: cleanup
```

Its own `retry:` and `timeout:` still apply, as the repair shows.

Grammar: `docs/grammar.md` §7.8, §7.9, §9.3, Decision D159. Topic:
`agent-compose docs flows`.
