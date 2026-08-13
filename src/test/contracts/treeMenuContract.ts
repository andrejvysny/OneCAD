/*
 * FROZEN tree context-menu contract — the ordered item labels the Document
 * Explorer's right-click menu renders per row, as shipped before contributed
 * menu items existed (WP-B1).
 *
 * ORDER is the invariant that change threatens. Once a foreign module can add
 * items to a row it does not own, the row's OWN items could silently be
 * displaced, reordered, or interleaved — and the first place a user notices is
 * when the item they aim for by muscle memory has moved under their cursor.
 * The rule this pins: capability items first (Rename, Hide/Show), then the
 * row's declared actions, then contributed items, each block behind its own
 * separator.
 *
 * See ./README.md before editing.
 */

/** Ordered menu-item labels per row kind, for a row declaring every capability. */
export const TREE_MENU_CONTRACT: Readonly<Record<string, readonly string[]>> = {
  /** A body: rename + visibility, then whatever actions its provider declares. */
  body: ["Rename", "Hide"],
  /**
   * A world-plane-hosted sketch. "Reattach…" is the one remaining
   * kind-specific branch in the panel and is documented as such there.
   */
  sketch: ["Rename", "Hide", "Reattach…"],
  /** A datum: no rename command and no visibility fact, so neither item exists. */
  datum: [],
};

/**
 * Where a contributed item may appear: only AFTER every item the row's own
 * owner supplies. A contribution that could interleave with them would let one
 * module rearrange another's menu.
 */
export const CONTRIBUTED_ITEMS_COME_LAST = true;
