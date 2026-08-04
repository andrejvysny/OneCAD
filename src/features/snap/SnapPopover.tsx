import type { RefObject } from "react";
import { Popover } from "@/ui/Popover";
import { SectionLabel } from "@/ui/SectionLabel";
import { Switch } from "@/ui/Switch";
import { SegmentedToggle } from "@/ui/SegmentedToggle";
import {
  useSettingsStore,
  type SnapKey,
  type ShowKey,
} from "@/stores/settingsStore";
import { useViewportStore } from "@/stores/viewportStore";
import type { DevicePref } from "@/viewport/engine/navInput";

const SNAP_ROWS: { key: SnapKey; label: string }[] = [
  { key: "grid", label: "Grid" },
  { key: "sketchGuideLines", label: "Sketch guide lines" },
  { key: "sketchGuidePoints", label: "Sketch guide points" },
  { key: "quadrant", label: "Quadrant points" },
  { key: "intersection", label: "Intersections" },
  { key: "onCurve", label: "On-curve points" },
  { key: "dimensionRound", label: "Dimension rounding" },
];

const SHOW_ROWS: { key: ShowKey; label: string }[] = [
  { key: "guidePoints", label: "Guide points" },
  { key: "snappingHints", label: "Snapping hints" },
  { key: "liveDimensions", label: "Live dimensions" },
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
