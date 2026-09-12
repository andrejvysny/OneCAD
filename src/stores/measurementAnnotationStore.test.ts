import { beforeEach, describe, expect, it } from "vitest";
import { measurementAnnotationStore } from "./measurementAnnotationStore";

describe("measurementAnnotationStore", () => {
  beforeEach(() => measurementAnnotationStore.getState().reset());

  it("preserves an unchanged slot while resetting only replaced identities", () => {
    const store = measurementAnnotationStore.getState();
    store.reconcile({ a: "face:body1:a", b: "edge:body1:b", pair: "face:body1:a|edge:body1:b" });
    store.setLivePosition("a", { x: 10, y: 20 });
    store.pin("a");
    store.toggleHidden("b");
    store.reconcile({ a: "face:body1:a", b: "edge:body1:c", pair: "face:body1:a|edge:body1:c" });

    expect(measurementAnnotationStore.getState().slots.a.pinnedScreenPosition).toEqual({ x: 10, y: 20 });
    expect(measurementAnnotationStore.getState().slots.b).toMatchObject({ identity: "edge:body1:c", manuallyHidden: false });
    expect(measurementAnnotationStore.getState().slots.pair.pinnedScreenPosition).toBeNull();
  });

  it("clears all transient state with an empty measurement session", () => {
    const store = measurementAnnotationStore.getState();
    store.reconcile({ a: "face:body1:a" });
    store.setLivePosition("a", { x: 10, y: 20 });
    store.pin("a");
    store.reconcile({});

    expect(measurementAnnotationStore.getState().slots).toMatchObject({
      a: { identity: null, pinnedScreenPosition: null },
      b: { identity: null, pinnedScreenPosition: null },
      pair: { identity: null, pinnedScreenPosition: null },
    });
  });

  it("only enables pinning after a current visible screen position arrives", () => {
    const store = measurementAnnotationStore.getState();
    store.reconcile({ pair: "face:body1:a|face:body1:b" });
    store.pin("pair");
    expect(measurementAnnotationStore.getState().slots.pair.pinnedScreenPosition).toBeNull();

    store.setLivePosition("pair", { x: 42, y: 36 });
    store.pin("pair");
    expect(measurementAnnotationStore.getState().slots.pair.pinnedScreenPosition).toEqual({ x: 42, y: 36 });
  });
});
