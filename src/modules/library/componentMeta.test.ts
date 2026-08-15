/*
 * The library's copy decisions (LGU-1 WP-A, canvas D1-A/D2-A).
 *
 * These are tested without a renderer because every branch is a sentence a user
 * reads, and the standing rule — the word "generator" never appears as a source
 * label — is exactly the kind of thing that regresses silently the next time
 * someone adds a source kind.
 */
import { describe, it, expect } from "vitest";
import {
  attachmentPhraseOf,
  badgeKindOf,
  badgeLabelOf,
  cardSubtitleOf,
  humanizeAttachmentName,
  provenancePhraseOf,
  sizeRangeOf,
  standardOf,
} from "./componentMeta";
import type { LibraryComponent } from "@/ipc/types";

function component(over: Partial<LibraryComponent> = {}): LibraryComponent {
  return {
    id: "onecad.std.iso4762",
    version: "1.0.0",
    name: "Socket Head Cap Screw",
    category: ["fastener"],
    tags: ["metric", "shcs"],
    sourceKind: "generator",
    revision: "sha256:0",
    attachments: {},
    parameters: {},
    designation: "ISO 4762 {thread}x{length}",
    ...over,
  };
}

const THREAD_DOMAIN = {
  thread: { role: "free" as const, key: "M6", domain: ["M2", "M4", "M6", "M8", "M12"] },
};

describe("badgeKindOf", () => {
  it("calls a generator-sourced component parametric", () => {
    expect(badgeKindOf(component())).toBe("parametric");
  });

  it("calls a configurable non-generator component parametric too", () => {
    expect(badgeKindOf(component({ sourceKind: "document", parameters: THREAD_DOMAIN }))).toBe(
      "parametric",
    );
  });

  it("calls a fixed embedded component static", () => {
    expect(badgeKindOf(component({ sourceKind: "embedded", parameters: {} }))).toBe("static");
  });

  /*
   * MINE WINS over parametric. A user scanning a mixed grid is asking "is this
   * mine or shipped" — they already know their own work is configurable.
   */
  it("calls a user-authored component mine even when it is parametric", () => {
    const mine = component({ id: "mine.bracket-plate", sourceKind: "document", parameters: THREAD_DOMAIN });
    expect(badgeKindOf(mine)).toBe("mine");
  });

  it("labels each kind", () => {
    expect(badgeLabelOf("parametric")).toBe("Parametric");
    expect(badgeLabelOf("static")).toBe("Static");
    expect(badgeLabelOf("mine")).toBe("Mine");
  });
});

describe("standardOf / sizeRangeOf", () => {
  it("reads the standard off the designation template's literal prefix", () => {
    expect(standardOf(component())).toBe("ISO 4762");
  });

  it("has no standard to report when the template opens with a hole", () => {
    expect(standardOf(component({ designation: "{thread} washer" }))).toBeNull();
    expect(standardOf(component({ designation: undefined }))).toBeNull();
  });

  it("spans the first domain-driven free param", () => {
    expect(sizeRangeOf(component({ parameters: THREAD_DOMAIN }))).toBe("M2–M12");
  });

  it("reports no range for a free numeric — a range is a claim about a fixed set", () => {
    expect(sizeRangeOf(component({ parameters: { length: { role: "free", value: 20 } } }))).toBeNull();
  });

  /*
   * The seeded ISO 4762 package declares `thread_detail` as a free domain param
   * alongside `thread`. Answering "which one is the size" by table order would
   * put "cosmetic–modeled" on the card the moment the table is reordered; the
   * designation template's first hole answers it on purpose.
   */
  it("takes the size from the designation's first hole, not from table order", () => {
    const c = component({
      designation: "ISO 4762 {thread}x{length}",
      parameters: {
        thread_detail: {
          role: "free",
          value: "cosmetic",
          domain: ["cosmetic", "simplified", "modeled"],
        },
        ...THREAD_DOMAIN,
      },
    });
    expect(sizeRangeOf(c)).toBe("M2–M12");
  });
});

describe("cardSubtitleOf", () => {
  it("leads with the standard and the sizes it covers", () => {
    expect(cardSubtitleOf(component({ parameters: THREAD_DOMAIN }))).toBe("ISO 4762 · M2–M12");
  });

  it("falls back to param count and a short version for a parametric part with no standard", () => {
    const c = component({
      designation: undefined,
      version: "1.2.3",
      parameters: { length: { role: "free", value: 20 }, width: { role: "free", value: 5 } },
    });
    expect(cardSubtitleOf(c)).toBe("2 params · v1.2");
  });

  it("singularizes one param", () => {
    const c = component({ designation: undefined, parameters: { length: { role: "free", value: 20 } } });
    expect(cardSubtitleOf(c)).toBe("1 param · v1.0");
  });

  it("falls back to the version alone for static geometry", () => {
    expect(cardSubtitleOf(component({ designation: undefined, parameters: {} }))).toBe("v1.0");
  });

  /* The card face drops the patch digit; the full semver lives in the detail
     panel's provenance footer, where a version is actually compared. */
  it("never puts a full semver on the card face", () => {
    expect(cardSubtitleOf(component({ designation: undefined, version: "1.0.0" }))).not.toContain("1.0.0");
  });
});

describe("provenancePhraseOf", () => {
  it("says how each source kind was made, in the user's words", () => {
    expect(provenancePhraseOf(component({ sourceKind: "generator" }))).toBe(
      "Parametric, generated on demand",
    );
    expect(provenancePhraseOf(component({ sourceKind: "document" }))).toBe(
      "Made from a document you authored",
    );
    expect(provenancePhraseOf(component({ sourceKind: "embedded" }))).toBe("Static geometry");
  });

  /*
   * THE STANDING RULE. "generator" is the wire's word for a source kind and
   * must never surface as one — the only user-facing survivor of the word is
   * the "Generators" menu title, which names tools, not provenance.
   */
  it("never leaks the internal source enum", () => {
    for (const kind of ["generator", "document", "embedded", "somethingNew"]) {
      expect(provenancePhraseOf(component({ sourceKind: kind })).toLowerCase()).not.toContain(
        "generator",
      );
    }
  });
});

describe("attachment prose", () => {
  it("humanizes a snake_case key", () => {
    expect(humanizeAttachmentName("head_seat")).toBe("Head seat");
    expect(humanizeAttachmentName("shank-axis")).toBe("Shank axis");
  });

  it("says what an attachment mates with as geometry, not as solver kinds", () => {
    expect(attachmentPhraseOf("head_seat", { on: "face:head_underside", accepts: ["plane"] })).toBe(
      "Head seat → flat faces",
    );
    expect(
      attachmentPhraseOf("shank_axis", { on: "cylinder:shank", accepts: ["cylinder", "hole"] }),
    ).toBe("Shank axis → holes & cylinders");
    expect(
      attachmentPhraseOf("rim", { on: "edge:rim", accepts: ["cylinder", "hole", "circularEdge"] }),
    ).toBe("Rim → holes & cylinders, hole rims");
  });

  it("falls back to the raw accepts list rather than inventing a phrase", () => {
    expect(attachmentPhraseOf("odd", { on: "x", accepts: ["somethingNew"] })).toBe(
      "Odd → somethingNew",
    );
  });
});
