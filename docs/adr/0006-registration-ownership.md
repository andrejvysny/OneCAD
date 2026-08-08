# 0006 — Every registration has an owner and an owner-scoped teardown

- Status: Accepted
- Date: 2026-08-08

## Context

A module or addon registers commands, tools, panels, inspector sections, viewport
layers, services and event subscriptions. If each registration hands back its own
disposer, unloading correctly means remembering twenty cleanup callbacks — and the
one that gets forgotten leaks a dead command into the palette or a stale layer
into the scene.

Duplicate ids are the other half of the problem. Last-one-wins means a second
addon can silently take over a built-in command, and the failure appears far from
its cause.

## Decision

Every registration carries `owner: ModuleId | AddonId`, and the platform supports
owner-scoped disposal:

```ts
const scope = moduleContext.createScope();
scope.commands.register(...);
scope.panels.register(...);
scope.events.subscribe(...);
scope.dispose();          // everything above is gone
```

Registering an id that already exists is a **deterministic failure** — reject,
report which owner holds it. Never last-one-wins.

A third-party contribution id must begin with its addon id: `com.example.foo` may
register `com.example.foo.command.inspect` and may not register
`onecad.modeling.extrude`. This holds even though addons are trusted in v1 —
impersonating a built-in is a correctness problem, not only a security one.

Hot unloading is not a v1 requirement. Scopes are still built now: retrofitting
ownership onto registrations already in use is far harder than starting with it.

## Cost

Slightly more ceremony per registration, and a hard failure where a lenient system
would have started up. That is the intent — a startup that fails while naming the
conflicting owner is cheaper than a mystery at runtime.
