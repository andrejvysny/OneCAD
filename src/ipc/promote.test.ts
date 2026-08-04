/*
 * The shared promote lane's degradation contract (HISTORY-HARDEN H4).
 *
 * Promotion is snapshot-scoped on both sides of the wire now, so a refusal is an
 * ORDINARY outcome (pick → edit → act on the old pick), not a worker fault. Every
 * call site depends on `promoteOne` turning that into a status hint plus `null` —
 * never a throw, and never a guessed element.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { promoteOne, STALE_PICK_HINT } from "./promote";
import type { CadClient } from "./client";
import type { PromotedElement, PromotePick } from "./types";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

const PICK: PromotePick = { topoKey: "f:22", anchor: { worldPoint: [1, 2, 3] } };

/** A `CadClient` stub whose ONLY live method is `promoteSelection`. */
function clientWith(promoteSelection: CadClient["promoteSelection"]): CadClient {
  return { promoteSelection } as unknown as CadClient;
}

function hint(): string | undefined {
  return viewportStore.getState().statusHint?.message;
}

describe("promoteOne", () => {
  beforeEach(() => {
    resetStores();
  });

  it("returns the promoted element and leaves the status bar alone on success", async () => {
    const el: PromotedElement = {
      topoKey: "f:22",
      elementId: "el_1",
      kind: "face",
      bodyId: "body_1",
    };
    const out = await promoteOne(clientWith(async () => [el]), "body_1", PICK);
    expect(out).toEqual(el);
    expect(hint()).toBeUndefined();
  });

  it("hints and returns null when the backend resolved nothing", async () => {
    // An unresolvable pick is OMITTED from the reply — an empty array IS the
    // signal, and it must not read as "success with no id".
    const out = await promoteOne(clientWith(async () => []), "body_1", PICK);
    expect(out).toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("hints and returns null when the backend REFUSES a stale pick", async () => {
    // The shape the Rust gate produces: a recoverable OpFailed naming both ids.
    const out = await promoteOne(
      clientWith(async () => {
        throw new Error("pick was taken against snapshot 4; head is 7 — re-pick");
      }),
      "body_1",
      PICK,
    );
    expect(out).toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("does not throw out of the tool when promotion rejects", async () => {
    await expect(
      promoteOne(
        clientWith(async () => {
          throw new Error("worker gone");
        }),
        "body_1",
        PICK,
      ),
    ).resolves.toBeNull();
  });

  it("degrades the same way before the first publish (snapshotId 0)", async () => {
    // `tauriClient` starts at currentSnapshotId 0 and only adopts a POSITIVE
    // published id, so a pick made before any `document-changed` addresses
    // snapshot 0 against a positive head and comes back refused.
    let sent = -1;
    const client = clientWith(async (_bodyId, picks) => {
      sent = picks.length;
      throw new Error("pick was taken against snapshot 0; head is 3 — re-pick");
    });
    const out = await promoteOne(client, "body_1", PICK);
    expect(sent).toBe(1);
    expect(out).toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("keeps the hint sticky so it survives the next repaint", async () => {
    await promoteOne(clientWith(async () => []), "body_1", PICK);
    expect(viewportStore.getState().statusHint?.sticky).toBe(true);
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
  });
});
