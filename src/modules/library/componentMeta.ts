/*
 * Library vocabulary: how a component DESCRIBES itself on screen (LGU-1 WP-A,
 * canvas D1-A/D2-A).
 *
 * The library's surfaces have to answer three questions at a glance — what is
 * it, can I configure it, whose is it — and they were answering them with
 * internal identifiers: a `SOURCE: generator` row that leaked a Rust enum
 * variant into the UI, and snake_case attachment keys printed as prose.
 *
 * THE RULE THIS MODULE ENFORCES: the string "generator" is an implementation
 * word and never reaches the user as a source label. It survives in exactly one
 * user-facing place in the app — the "Generators" menu title, which names a
 * category of tools, not a component's provenance.
 *
 * Pure and dependency-free on purpose: every branch here is a copy decision,
 * and copy decisions are worth testing without a renderer.
 */
import type { LibraryAttachment, LibraryComponent } from "@/ipc/types";
import { freeParams } from "./designation";

export type SourceBadgeKind = "parametric" | "static" | "mine";

/** The `mine.` namespace `suggestedId` mints for a user-authored component. */
const MINE_PREFIX = "mine.";

/**
 * Which badge a card wears.
 *
 * MINE WINS. A component the user authored is theirs first and parametric
 * second — "can I configure this" is a question they already know the answer
 * to for their own work, while "is this mine or shipped" is the one they scan
 * a mixed grid for.
 */
export function badgeKindOf(component: LibraryComponent): SourceBadgeKind {
  if (component.id.startsWith(MINE_PREFIX)) return "mine";
  if (component.sourceKind === "generator") return "parametric";
  return freeParams(component.parameters).length > 0 ? "parametric" : "static";
}

/** The badge's own text. */
export function badgeLabelOf(kind: SourceBadgeKind): string {
  return kind === "parametric" ? "Parametric" : kind === "static" ? "Static" : "Mine";
}

/**
 * The standard a component belongs to, e.g. "ISO 4762".
 *
 * Read off the DESIGNATION TEMPLATE's literal prefix rather than a dedicated
 * field: `metadata.standard` exists in the Rust manifest but is not projected
 * into `LibraryComponent`, and a template like `"ISO 4762 M{thread}x{length}"`
 * already states the standard in the only place an author has to keep correct.
 * `null` when the template opens with a hole (nothing standard-shaped to show).
 */
export function standardOf(component: LibraryComponent): string | null {
  const template = component.designation;
  if (!template) return null;
  const literal = template.slice(0, template.indexOf("{") === -1 ? undefined : template.indexOf("{"));
  const trimmed = literal.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/**
 * The size range a component covers, e.g. "M2–M12".
 *
 * WHICH param is the size is read off the DESIGNATION TEMPLATE's first hole,
 * not off table order. The author already answered the question by writing
 * `"ISO 4762 {thread}x{length}"`: the param that opens the BOM string is the
 * one the part is chosen by. Table order would answer it by accident — the
 * seeded ISO 4762 package declares `thread_detail` (cosmetic/simplified/
 * modeled) as a free domain param too, and a reordering would put
 * "cosmetic–modeled" on the card as if it were a size.
 *
 * `null` when nothing domain-driven is in play: a range is a claim about a
 * fixed set, and a free numeric has none.
 */
export function sizeRangeOf(component: LibraryComponent): string | null {
  const spec = sizeDrivingParam(component);
  const domain = spec?.domain;
  if (!domain || domain.length < 2) return null;
  return `${String(domain[0])}–${String(domain[domain.length - 1])}`;
}

function sizeDrivingParam(component: LibraryComponent) {
  const free = new Map(freeParams(component.parameters));
  for (const match of component.designation?.matchAll(/\{(\w+)\}/g) ?? []) {
    const spec = free.get(match[1]);
    if (spec?.domain && spec.domain.length > 1) return spec;
  }
  // No template, or none of its holes is domain-driven: fall back to the first
  // free param that IS, which is all table order can honestly tell us.
  for (const [, spec] of free) {
    if (spec.domain && spec.domain.length > 1) return spec;
  }
  return undefined;
}

/** `"1.2"` from `"1.2.3"` — a card shows the meaningful half of a semver. */
function shortVersion(version: string): string {
  const parts = version.split(".");
  return parts.length >= 2 ? `${parts[0]}.${parts[1]}` : version;
}

/**
 * The card's second line, resolved in three steps (canvas D1-A).
 *
 * A standards part leads with its standard and the sizes it covers, because
 * that is how it is chosen; the full semver leaves the card face and lives in
 * the detail panel's provenance footer, where a version actually gets compared.
 * Everything else falls back to what it can honestly say.
 */
export function cardSubtitleOf(component: LibraryComponent): string {
  const standard = standardOf(component);
  if (standard) {
    const range = sizeRangeOf(component);
    return range ? `${standard} · ${range}` : standard;
  }
  const free = freeParams(component.parameters).length;
  if (free > 0) {
    return `${free} param${free === 1 ? "" : "s"} · v${shortVersion(component.version)}`;
  }
  return `v${shortVersion(component.version)}`;
}

/**
 * One line naming where the geometry comes from, in the user's vocabulary
 * (canvas D2-A's provenance footer).
 *
 * Never "generator", never "embedded", never "document" — those are the wire's
 * words for the same three things.
 */
export function provenancePhraseOf(component: LibraryComponent): string {
  switch (component.sourceKind) {
    case "generator":
      return "Parametric, generated on demand";
    case "document":
      return "Made from a document you authored";
    default:
      return "Static geometry";
  }
}

/** `"head_seat"` → `"Head seat"`. */
export function humanizeAttachmentName(key: string): string {
  const words = key.replace(/[_-]+/g, " ").trim();
  return words.length === 0 ? key : words.charAt(0).toUpperCase() + words.slice(1);
}

/**
 * What a single `accepts` token mates with, said as geometry a user can point
 * at rather than as a solver kind.
 */
function acceptsPhrase(accepts: readonly string[]): string {
  const set = new Set(accepts);
  const parts: string[] = [];
  if (set.has("plane")) parts.push("flat faces");
  if (set.has("cylinder") || set.has("hole")) parts.push("holes & cylinders");
  if (set.has("circularEdge")) parts.push("hole rims");
  return parts.length > 0 ? parts.join(", ") : accepts.join(", ");
}

/**
 * One attachment as a sentence: `"Shank axis → holes & cylinders"`.
 *
 * The panel used to print `shank_axis — cylinder:shank`, which is the manifest
 * key and the manifest's own `on` string — true, and meaningless to anyone who
 * did not write the package.
 */
export function attachmentPhraseOf(key: string, attachment: LibraryAttachment): string {
  return `${humanizeAttachmentName(key)} → ${acceptsPhrase(attachment.accepts)}`;
}
