import type { RefObject } from "react";
import { Popover } from "@/ui/Popover";
import { SectionLabel } from "@/ui/SectionLabel";
import { Switch } from "@/ui/Switch";
import { SegmentedToggle } from "@/ui/SegmentedToggle";
import {
  AUTO_CONSTRAIN_LABEL,
  AUTO_CONSTRAIN_MODES,
  useSettingsStore,
  type AutoConstrainMode,
  type SnapKey,
  type ShowKey,
} from "@/stores/settingsStore";
import { useViewportStore } from "@/stores/viewportStore";
import {
  SNAP_RADIUS_LABEL,
  SNAP_RADIUS_ORDER,
  type SnapRadiusId,
} from "@/tools/sketch/snapRadius";
import type { DevicePref } from "@/viewport/engine/navInput";

const SNAP_ROWS: { key: SnapKey; label: string }[] = [
  { key: "grid", label: "Grid" },
  { key: "sketchGuideLines", label: "Sketch guide lines" },
  { key: "sketchGuidePoints", label: "Sketch guide points" },
  { key: "quadrant", label: "Quadrant points" },
  { key: "intersection", label: "Intersections" },
  { key: "onCurve", label: "On-curve points" },
  { key: "dimensionRound", label: "Dimension rounding" },
  { key: "polarTracking", label: "Polar tracking" },
];

const RADIUS_OPTIONS: { value: SnapRadiusId; label: string }[] = SNAP_RADIUS_ORDER.map((id) => ({
  value: id,
  label: SNAP_RADIUS_LABEL[id],
}));

const AUTO_CONSTRAIN_OPTIONS: { value: AutoConstrainMode; label: string }[] =
  AUTO_CONSTRAIN_MODES.map((m) => ({ value: m, label: AUTO_CONSTRAIN_LABEL[m] }));

/** One line per mode, so the control says what it DOES rather than making the
 *  user infer three states from three words. */
const AUTO_CONSTRAIN_HELP: Record<AutoConstrainMode, string> = {
  off: "Shapes still hold together; nothing is related to existing geometry.",
  minimal: "Keeps the points you snapped to: coincident, midpoint, on-curve, origin.",
  standard: "Also keeps directions: horizontal, vertical, parallel, perpendicular, tangent.",
};

const SHOW_ROWS: { key: ShowKey; label: string }[] = [
  { key: "guidePoints", label: "Guide points" },
  { key: "snappingHints", label: "Snapping hints" },
  { key: "liveDimensions", label: "Live dimensions" },
  { key: "constraintChips", label: "Constraint chips" },
  { key: "coincidentBadges", label: "Coincident badges" },
];

const DEVICE_OPTIONS: { value: DevicePref; label: string }[] = [
  { value: "auto", label: "Auto" },
  { value: "trackpad", label: "Trackpad" },
  { value: "mouse", label: "Mouse" },
];

function SnapRow({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex h-8 items-center gap-2 px-3.5">
      <span className="flex-1 text-[13px] text-ink-2">{label}</span>
      <Switch checked={checked} onChange={onChange} ariaLabel={label} />
    </div>
  );
}

type SnapPopoverProps = {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLButtonElement | null>;
};

/**
 * Snap settings popover (prototype 1c). Anchored to the corner-cluster snap
 * button, opening to its left with a right-pointing caret. Bound to the
 * persisted settings store.
 */
export function SnapPopover({ open, onClose, anchorRef }: SnapPopoverProps) {
  const snapTo = useSettingsStore((s) => s.snapTo);
  const show = useSettingsStore((s) => s.show);
  const setSnap = useSettingsStore((s) => s.setSnap);
  const setShow = useSettingsStore((s) => s.setShow);
  const snapRadius = useSettingsStore((s) => s.snapRadius);
  const setSnapRadius = useSettingsStore((s) => s.setSnapRadius);
  const autoConstrainMode = useSettingsStore((s) => s.autoConstrainMode);
  const setAutoConstrainMode = useSettingsStore((s) => s.setAutoConstrainMode);
  const inputDevice = useSettingsStore((s) => s.navigation.inputDevice);
  const setInputDevice = useSettingsStore((s) => s.setInputDevice);
  const detected = useViewportStore((s) => s.detectedInputDevice);

  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      width={238}
      caret
      placement="left-start"
      className="pb-2 pt-1.5"
    >
      <SectionLabel className="px-3.5 pb-0.5 pt-2">Snap to</SectionLabel>
      {SNAP_ROWS.map((r) => (
        <SnapRow
          key={r.key}
          label={r.label}
          checked={snapTo[r.key]}
          onChange={(v) => setSnap(r.key, v)}
        />
      ))}
      {/* Numeric, so a segmented control rather than a switch — and last in the
          section because it scales every switch above it rather than adding a
          tier of its own. */}
      <div className="flex h-8 items-center gap-2 px-3.5">
        <span className="flex-1 text-[13px] text-ink-2">Snap radius</span>
        <SegmentedToggle
          options={RADIUS_OPTIONS}
          value={snapRadius}
          onChange={setSnapRadius}
          ariaLabel="Snap radius"
          size="sm"
        />
      </div>

      <div className="mx-3.5 my-1.5 h-px bg-border-subtle" />

      {/* SHOWING a relationship, SNAPPING to it and KEEPING it are three
          separate decisions, so this is its own section rather than another
          "Snap to" switch. Three states, so a segmented control — two
          independent switches could express "off but standard", which is
          meaningless. */}
      <SectionLabel className="px-3.5 pb-0.5 pt-1.5">Keep relationships</SectionLabel>
      <div className="px-3.5 pb-1 pt-0.5">
        <SegmentedToggle
          options={AUTO_CONSTRAIN_OPTIONS}
          value={autoConstrainMode}
          onChange={setAutoConstrainMode}
          ariaLabel="Auto-constrain"
          size="sm"
          className="w-full"
        />
        <p className="pt-1.5 text-[11px] leading-snug text-ink-4">
          {AUTO_CONSTRAIN_HELP[autoConstrainMode]} Hold Alt while clicking to place one point
          freely.
        </p>
      </div>

      <div className="mx-3.5 my-1.5 h-px bg-border-subtle" />

      <SectionLabel className="px-3.5 pb-0.5 pt-1.5">Show</SectionLabel>
      {SHOW_ROWS.map((r) => (
        <SnapRow
          key={r.key}
          label={r.label}
          checked={show[r.key]}
          onChange={(v) => setShow(r.key, v)}
        />
      ))}

      <div className="mx-3.5 my-1.5 h-px bg-border-subtle" />

      <SectionLabel className="px-3.5 pb-0.5 pt-1.5">Navigation</SectionLabel>
      <div className="px-3.5 pb-1 pt-0.5">
        <SegmentedToggle
          options={DEVICE_OPTIONS}
          value={inputDevice}
          onChange={setInputDevice}
          ariaLabel="Input device"
          size="sm"
          className="w-full"
        />
        <p className="pt-1.5 text-[11px] leading-snug text-ink-4">
          {inputDevice === "auto"
            ? `Detected: ${detected}. Detection is best-effort — pin it if scrolling feels wrong.`
            : "Trackpad: two fingers pan, shift + two fingers orbit, pinch zooms."}
        </p>
      </div>
    </Popover>
  );
}
