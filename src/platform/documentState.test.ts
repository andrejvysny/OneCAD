/*
 * Module-owned document state through the mock lane (ADR-0004 / ADR-0005).
 *
 * The mock is the whole point here: an addon's persistence path must be
 * exercisable with no backend, and the payload must survive the round trip
 * without anything on the way deciding it knows better.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { mockClient } from "@/ipc/mockClient";
import { moduleId } from "./ids";
import { createDocumentStateService, missingModules } from "./documentState";

const FOO = moduleId("com.example.foo");
const BAR = moduleId("com.example.bar");

describe("document state service", () => {
  beforeEach(async () => {
    await mockClient.closeDocument();
  });

  it("reads back exactly what was written", async () => {
    const state = createDocumentStateService(mockClient, FOO);
    const payload = { nested: [1, 2, { deep: true }], unicode: "påyload" };

    await state.write({ schemaVersion: 3, payload });
    const read = await state.read<typeof payload>();

    expect(read).toEqual({ schemaVersion: 3, payload });
  });

  it("returns null for a module with no slice", async () => {
    expect(await createDocumentStateService(mockClient, BAR).read()).toBeNull();
  });

  it("isolates one module's slice from another's", async () => {
    await createDocumentStateService(mockClient, FOO).write({ schemaVersion: 1, payload: "foo" });
    await createDocumentStateService(mockClient, BAR).write({ schemaVersion: 9, payload: "bar" });

    expect((await createDocumentStateService(mockClient, FOO).read())?.payload).toBe("foo");
    expect((await createDocumentStateService(mockClient, BAR).read())?.payload).toBe("bar");
  });

  it("clear removes only its own slice", async () => {
    await createDocumentStateService(mockClient, FOO).write({ schemaVersion: 1, payload: "foo" });
    await createDocumentStateService(mockClient, BAR).write({ schemaVersion: 1, payload: "bar" });

    await createDocumentStateService(mockClient, FOO).clear();

    expect(await createDocumentStateService(mockClient, FOO).read()).toBeNull();
    expect((await createDocumentStateService(mockClient, BAR).read())?.payload).toBe("bar");
  });

  it("does not hand back a live reference to stored state", async () => {
    const state = createDocumentStateService(mockClient, FOO);
    const payload = { list: [1] };
    await state.write({ schemaVersion: 1, payload });

    payload.list.push(2);
    const read = await state.read<{ list: number[] }>();

    expect(read?.payload.list, "the store must not alias the caller's object").toEqual([1]);
  });

  it("lists the document's modules in a stable order", async () => {
    await createDocumentStateService(mockClient, FOO).write({ schemaVersion: 2, payload: {} });
    await createDocumentStateService(mockClient, BAR).write({ schemaVersion: 5, payload: {} });

    expect(await mockClient.listDocumentModules()).toEqual([
      { moduleId: "com.example.bar", schemaVersion: 5 },
      { moduleId: "com.example.foo", schemaVersion: 2 },
    ]);
  });
});

describe("missing-module reporting", () => {
  it("names the modules this build has no code for", () => {
    const documentModules = [
      { moduleId: "onecad.modeling", schemaVersion: 1 },
      { moduleId: "com.example.foo", schemaVersion: 2 },
    ];

    expect(missingModules(documentModules, ["onecad.modeling"])).toEqual([
      { moduleId: "com.example.foo", schemaVersion: 2, missing: true },
    ]);
  });

  it("reports nothing when every module is installed", () => {
    const documentModules = [{ moduleId: "onecad.modeling", schemaVersion: 1 }];
    expect(missingModules(documentModules, ["onecad.modeling", "onecad.shell"])).toEqual([]);
  });
});
