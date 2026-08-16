/*
 * Inspector section-order GOLDEN probe (Platform refactor W0).
 *
 * The inspector is the surface most exposed to the refactor: once its blocks
 * become registered contributions, their order could quietly start following
 * registration time instead of an explicit priority. This pins the shipped
 * section order per selection state against a frozen contract.
 *
 * It asserts ORDER and PRESENCE only — the content of each section stays covered
 * by InspectorPanel.test.tsx.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { act } from "@testing-library/react";
import { InspectorPanel } from "./InspectorPanel";
import { selectionStore } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { resetStores } from "@/test/resetStores";
import { setMockLatency } from "@/ipc/mockClient";
import { renderWithPlatform, bootTestPlatform } from "@/test/renderWithPlatform";
import { contributeInspectorSections } from "@/modules/modeling/inspectorSections";
import { registerRenderUi } from "@/modules/render/ui/register";
import { RENDER_MODULE_ID } from "@/modules/render/manifest";
import type { Platform } from "@/platform";
import type { SketchSession } from "@/ipc/types";
import {
  INSPECTOR_SECTIONS_CONTRACT,
  INSPECTOR_FEATURE_PREFIX_CONTRACT,
} from "@/test/contracts/inspectorContract";

/** `SectionLabel` renders a div carrying this tracking class — its only marker. */
const SECTION_CLASS = "tracking-[0.07em]";

/**
 * Modeling's sections AND render's, each under its OWN owner.
 *
 * The contract freezes the order the user sees, which is now produced by two
 * modules: a section id is namespaced to its owner, so registering render's
 * "Material" through a modeling scope would be rejected outright — and a probe
 * that registered only modeling's half would assert an inspector nobody runs.
 * The mechanism changed, so the probe did (see `src/test/contracts/README.md`);
 * the contract's amendment is its own, separately recorded decision.
 */
function bootSections(): Platform {
  const platform = bootTestPlatform(contributeInspectorSections);
  registerRenderUi(platform.createScope(RENDER_MODULE_ID));
  return platform;
}

/**
 * Reads the rendered section labels AFTER letting the async sections settle.
 *
 * The settle is load-bearing since WP-VE.2: `Variables` loads the document's
 * variable table through the client and renders nothing until that promise
 * resolves, so a synchronous read would report the pre-load section list and
 * pass this probe against a panel the user never sees. The mock lane's simulated
 * latency is zeroed for the suite (below) so one timer tick is enough.
 *
 * The CONTRACT itself is unchanged by any of this — only what this probe is
 * entitled to call "the shipped order".
 */
async function sectionsOf(container: HTMLElement): Promise<string[]> {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
  return [...container.querySelectorAll("div")]
    .filter((el) => el.className.includes(SECTION_CLASS))
    .map((el) => el.textContent?.trim() ?? "");
}

function emptySession(): SketchSession {
  return {
    sketchId: "sketch2",
    plane: { kind: "XY", origin: [0, 0, 0], xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] },
    entities: [],
    constraints: [],
    dof: 3,
    status: "UnderConstrained",
  };
}

describe("inspector section order", () => {
  beforeEach(() => {
    resetStores();
    setMockLatency(0);
  });
  afterEach(() => setMockLatency(120));

  it("EMPTY state renders no sections", async () => {
    const { container } = renderWithPlatform(<InspectorPanel />, { platform: bootSections() });
    act(() => selectionStore.getState().clear());
    expect(await sectionsOf(container)).toEqual([...INSPECTOR_SECTIONS_CONTRACT.empty]);
  });

  it("a body renders Appearance, then Material, then History", async () => {
    const { container } = renderWithPlatform(<InspectorPanel />, { platform: bootSections() });
    act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));
    expect(await sectionsOf(container)).toEqual([...INSPECTOR_SECTIONS_CONTRACT.body]);
  });

  it("a promoted face renders Appearance, then Material, then History", async () => {
    const { container } = renderWithPlatform(<InspectorPanel />, { platform: bootSections() });
    act(() =>
      selectionStore.getState().set([
        { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0", elementId: "el_top" },
      ]),
    );
    expect(await sectionsOf(container)).toEqual([...INSPECTOR_SECTIONS_CONTRACT.face]);
  });

  it("an edge renders History only — Appearance and Material are faces and bodies", async () => {
    const { container } = renderWithPlatform(<InspectorPanel />, { platform: bootSections() });
    act(() =>
      selectionStore
        .getState()
        .set([{ kind: "edge", id: "body1#e:0", bodyId: "body1", topoKey: "e:0" }]),
    );
    expect(await sectionsOf(container)).toEqual([...INSPECTOR_SECTIONS_CONTRACT.edge]);
  });

  it("a sketch renders History before Constraints", async () => {
    const { container } = renderWithPlatform(<InspectorPanel />, { platform: bootSections() });
    act(() => selectionStore.getState().set([{ kind: "sketch", id: "sketch2" }]));
    expect(await sectionsOf(container)).toEqual([...INSPECTOR_SECTIONS_CONTRACT.sketch]);
  });

  it("a sketch region renders the same sections as its owning sketch", async () => {
    const { container } = renderWithPlatform(<InspectorPanel />, { platform: bootSections() });
    act(() =>
      selectionStore.getState().set([
        {
          kind: "sketchRegion",
          id: '["sketch2","r_profile"]',
          sketchId: "sketch2",
          regionId: "r_profile",
        },
      ]),
    );
    expect(await sectionsOf(container)).toEqual([...INSPECTOR_SECTIONS_CONTRACT.sketchRegion]);
  });

  it("sketch mode renders Constraints whether or not the session has any", async () => {
    const { container } = renderWithPlatform(<InspectorPanel />, { platform: bootSections() });
    act(() => {
      toolStore.getState().setMode("sketch", "sketch2");
      sketchStore.getState().setSession(emptySession());
    });
    // Unconditional: an empty sketch shows the label over "No constraints yet.",
    // so the panel does not reflow when the first constraint lands.
    expect(await sectionsOf(container)).toEqual([...INSPECTOR_SECTIONS_CONTRACT.sketchMode]);

    act(() =>
      sketchStore.getState().setSession({
        ...emptySession(),
        constraints: [{ id: "c1", type: "Horizontal", entities: ["l1"] }],
      }),
    );
    expect(await sectionsOf(container)).toEqual([...INSPECTOR_SECTIONS_CONTRACT.sketchMode]);
  });

  it("a selected feature leads with History", async () => {
    const { container } = renderWithPlatform(<InspectorPanel />, { platform: bootSections() });
    act(() => selectionStore.getState().set([{ kind: "feature", id: "op1" }]));
    // Dependency sections are data-driven and arrive async, so only the frozen
    // prefix is asserted — what matters is that History stays first.
    const sections = await sectionsOf(container);
    expect(sections.slice(0, INSPECTOR_FEATURE_PREFIX_CONTRACT.length)).toEqual([
      ...INSPECTOR_FEATURE_PREFIX_CONTRACT,
    ]);
  });
});
