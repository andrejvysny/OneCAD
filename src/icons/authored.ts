/*
 * Icons with NO counterpart in the OneCAD-CPP master set.
 *
 * Everything the CPP family covers lives in `cppIcons.generated.ts` and is
 * regenerated from those masters. These are the remainder: glyphs the Qt app
 * never needed because it has no start screen, no theme switcher, and no
 * center-rectangle tool. They are maintained BY HAND here.
 *
 * Two provenance tiers, both preserved from the original `paths.ts`:
 *   - extracted VERBATIM from the design prototype
 *     (`design/project/OneCAD UI Explorations.dc.html`), cited per entry
 *   - authored for a specific work package, on the same 24x24 grid
 *
 * TONE POLICY. The chrome glyphs below are deliberately single-tone. That is
 * not an omission: DESIGN.md §3 reserves the accent for "the operation verb",
 * and a chevron or a checkmark has no verb to accent — the CPP set draws its
 * own chrome (`close`, `menu`, `overflow`) flat for exactly this reason. The
 * CAD-tool glyphs in the second half ARE two-tone, authored against DESIGN.md
 * so they sit correctly beside their generated neighbours in the toolbar.
 */

import type { IconDef } from "./types";

export const AUTHORED_ICONS = {
  // ---- Chrome (single-tone by policy, see above) ----
  plus: [{ d: "M12 5v14M5 12h14" }], // New project (prototype line 52)
  check: [{ d: "M5 12.5l4.5 4.5L19 7" }], // Finish sketch (line 177)
  chevronDown: [{ d: "M6 9l6 6 6-6" }], // sort dropdown (line 64)
  clock: [{ d: "M12 8v4l2.5 2.5M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0z" }], // Recent (line 101)
  star: [
    {
      d: "M12 4l2.4 4.9 5.4.8-3.9 3.8.9 5.4-4.8-2.5-4.8 2.5.9-5.4L4.2 9.7l5.4-.8L12 4z",
    },
  ], // Starred (line 102)
  template: [{ d: "M4.5 7.5h15v12h-15zM8 7.5V5h8v2.5" }], // Templates (line 103)

  // ---- Appearance (DARK-MODE WP; not from the prototype) ----
  // One glyph per ThemePref, so the title-bar button always shows the CURRENT
  // preference rather than a generic "theme" symbol.
  // Sun: disc + eight rays.
  themeLight: [
    {
      d: "M12 8.2a3.8 3.8 0 1 1 0 7.6 3.8 3.8 0 0 1 0-7.6M12 3v2.2M12 18.8V21M4.6 4.6l1.6 1.6M17.8 17.8l1.6 1.6M3 12h2.2M18.8 12H21M4.6 19.4l1.6-1.6M17.8 6.2l1.6-1.6",
    },
  ],
  // Crescent: one arc cut by another.
  themeDark: [{ d: "M20 14.2A8.2 8.2 0 0 1 9.8 4 8.4 8.4 0 1 0 20 14.2z" }],
  // Monitor = "whatever the desktop says" (System).
  themeSystem: [{ d: "M3.5 5.5h17v10h-17zM9 19.5h6M12 15.5v4" }],

  /*
   * ---- CAD tools with no CPP master (two-tone, authored against DESIGN.md) ----
   *
   * Same rules as the generated family: 24x24 grid, meaningful marks inside
   * 2..22, round caps/joins, and EXACTLY ONE accent idea — the thing the tool
   * does. Sketch tools stay flat/orthographic per §4; only the solid-body tool
   * (`mirrorBody`) uses the shared dimetric depth vector d = (+4,-3).
   */

  /*
   * The three "drawn by clicking N points" sketch tools all follow the grammar
   * the generated `rectangle` / `arc` / `line` establish: the shape in primary,
   * an accent DOT on each point the user actually clicks. That is what makes
   * `centerRect` legible against `rect` at 15px — same outline, but the dots
   * say centre-then-corner instead of corner-then-corner.
   */

  // Rectangle drawn FROM its centre. Same outline as the generated `rectangle`,
  // so only the click points differ.
  centerRect: [
    { d: "M5 6 H19 V18 H5 Z" },
    { d: "M12 12 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
    { d: "M19 18 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],

  // Regular polygon: centre + circumradius, the two things that define it.
  // An inscribed construction circle was tried first and read as a dotted halo
  // at toolbar size — the hexagon and the circle are nearly the same diameter.
  polygon: [
    { d: "M12 4 L18.9 8 V16 L12 20 L5.1 16 V8 Z" },
    { d: "M12 12 L18.9 8", tone: "accent", sw: 0.8 },
    { d: "M12 12 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
    { d: "M18.9 8 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],

  // Slot: the stadium outline in primary, the centre-to-centre axis and its two
  // end points accented (a slot IS its axis plus a width).
  slot: [
    { d: "M8.5 7.5 H15.5 a4.5 4.5 0 0 1 0 9 H8.5 a4.5 4.5 0 0 1 0 -9 Z" },
    { d: "M8.5 12 H15.5", tone: "accent", sw: 0.8 },
    { d: "M8.5 12 m-1.4 0 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0", tone: "accent", filled: true },
    { d: "M15.5 12 m-1.4 0 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0", tone: "accent", filled: true },
  ],

  // ---- SP-4 3-point arc ----
  // LITERALLY the generated `arc` plus one dot: same sweep, same two endpoint
  // dots, plus the through point that bends it. The pair then reads as two ways
  // of drawing one shape, and the only thing a user has to spot is the third
  // click. The through point sits at the arc's apex — for the shared sweep
  // (centre 15,17.8 · r 11) that is (9.2, 8.5), not the chord midpoint; a dot
  // floating beside the curve instead of on it is the failure mode here.
  arc3p: [
    { d: "M4 18 A11 11 0 0 1 20 8" },
    { d: "M4 18 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
    { d: "M20 8 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
    { d: "M9.2 8.5 m-1.5 0 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0", tone: "accent", filled: true },
  ],

  // Sketch point: crosshair ticks in primary, the point itself accented — the
  // point is the whole verb, so it takes the accent even though it is central.
  point: [
    { d: "M12 3.5 V8M12 16 v4.5M3.5 12 H8M16 12 h4.5" },
    { d: "M12 12 m-2 0 a2 2 0 1 0 4 0 a2 2 0 1 0 -4 0", tone: "accent", filled: true },
  ],

  // Hole: a bore in three-quarter view. The host face and the bore walls are
  // primary; the accent is the down-arrow on the axis — the drilling verb.
  hole: [
    { d: "M17 7.5 a5 2.5 0 1 1 -10 0 a5 2.5 0 1 1 10 0 Z" },
    { d: "M7 7.5 V15.5 a5 2.5 0 0 0 10 0 V7.5" },
    { d: "M2.5 7.5 H7M17 7.5 H21.5" },
    { d: "M12 8 V17", tone: "accent", sw: 0.8 },
    { d: "M9.9 14.9 L12 17 L14.1 14.9", tone: "accent", sw: 0.8 },
  ],

  // Measure: a ruler laid diagonally. The instrument is primary; the accent is
  // the graduation ticks — reading the scale is the verb, and it is the only
  // part that survives shrinking. An earlier version accented the ruler's two
  // short ends, which drew the accent directly on top of the primary outline
  // and so showed no second tone at all.
  measure: [
    { d: "M14.5 3.5 L20.5 9.5 L10.5 19.5 L4.5 13.5 Z" },
    { d: "M10.5 7.5 L12.5 9.5", tone: "accent", sw: 0.8 },
    { d: "M8 10 L10 12", tone: "accent", sw: 0.8 },
    { d: "M5.5 12.5 L7.5 14.5", tone: "accent", sw: 0.8 },
  ],

  // Mirror a BODY across a plane — the solid counterpart to the generated flat
  // `mirror` (which is sketch geometry). Same grammar as that icon: accent
  // dashed axis, then the two shapes it reflects, BOTH solid. Drawing the copy
  // as a dashed ghost was tried and turned to noise at toolbar size — `mirror`
  // draws both of its triangles solid for the same reason. Dimetric per §4.
  // Clearance around the axis is load-bearing: the dimetric top face leans
  // +2.5 toward it, so a body sized like `mirror`'s triangles closes the gap
  // and the three shapes merge into one blob. Each body is kept inside 6 units
  // so ~3.5 units of air survive on either side of the plane.
  mirrorBody: [
    { d: "M12 3 V21", tone: "accent", dash: "2 2.4" },
    { d: "M2.5 12 H6.5 V18 H2.5 Z" },
    { d: "M2.5 12 L4.5 10 H8.5 L6.5 12" },
    { d: "M6.5 12 L8.5 10 V16 L6.5 18" },
    { d: "M21.5 12 H17.5 V18 H21.5 Z" },
    { d: "M21.5 12 L19.5 10 H15.5 L17.5 12" },
    { d: "M17.5 12 L15.5 10 V16 L17.5 18" },
  ],
} as const satisfies Record<string, IconDef>;

export type AuthoredIconName = keyof typeof AUTHORED_ICONS;
