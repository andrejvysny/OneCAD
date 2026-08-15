import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "@/icons/Icon";
import { Button } from "@/ui/Button";
import { SectionLabel } from "@/ui/SectionLabel";
import { SegmentedToggle, type SegmentedOption } from "@/ui/SegmentedToggle";
import { Switch } from "@/ui/Switch";
import { useSettingsStore, type ShowKey, type SnapKey } from "@/stores/settingsStore";
import { THEMES, THEME_ORDER, type ThemePref } from "@/theme/themes";
import { LENGTH_UNITS, LENGTH_UNIT_ORDER, type LengthUnitId } from "@/units/lengthUnits";
import { RENDER_MODES, RENDER_MODE_ORDER, type RenderModeId } from "@/viewport/engine/renderModes";
import type { DevicePref } from "@/viewport/engine/navInput";

type SettingsTab = "general" | "display" | "snap";

const TABS: { id: SettingsTab; label: string }[] = [
  { id: "general", label: "General" },
  { id: "display", label: "Display" },
  { id: "snap", label: "Snap" },
];

const THEME_OPTIONS: SegmentedOption<ThemePref>[] = THEME_ORDER.map((id) => ({
  value: id,
  label: THEMES[id].label,
}));

const UNIT_OPTIONS: SegmentedOption<LengthUnitId>[] = LENGTH_UNIT_ORDER.map((id) => ({
  value: id,
  label: LENGTH_UNITS[id].symbol,
}));

const DISPLAY_OPTIONS: SegmentedOption<RenderModeId>[] = RENDER_MODE_ORDER.map((id) => ({
  value: id,
  label: RENDER_MODES[id].label,
}));

const DEVICE_OPTIONS: SegmentedOption<DevicePref>[] = [
  { value: "auto", label: "Auto" },
  { value: "trackpad", label: "Trackpad" },
  { value: "mouse", label: "Mouse" },
];

const SNAP_ROWS: { key: SnapKey; label: string }[] = [
  { key: "grid", label: "Grid" },
  { key: "sketchGuideLines", label: "Sketch guide lines" },
  { key: "sketchGuidePoints", label: "Sketch guide points" },
  { key: "quadrant", label: "Quadrant points" },
  { key: "intersection", label: "Intersections" },
  { key: "onCurve", label: "On-curve points" },
];

const SHOW_ROWS: { key: ShowKey; label: string }[] = [
  { key: "guidePoints", label: "Guide points" },
  { key: "snappingHints", label: "Snapping hints" },
  { key: "liveDimensions", label: "Live dimensions" },
  { key: "constraintChips", label: "Constraint chips" },
];

function SwitchRow({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div className="flex h-9 items-center gap-3 rounded-sm px-3 hover:bg-hover">
      <span className="flex-1 text-[13px] text-ink-2">{label}</span>
      <Switch checked={checked} onChange={onChange} ariaLabel={label} />
    </div>
  );
}

export function SettingsModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [tab, setTab] = useState<SettingsTab>("general");

  const displayMode = useSettingsStore((s) => s.displayMode);
  const setDisplayMode = useSettingsStore((s) => s.setDisplayMode);
  const theme = useSettingsStore((s) => s.theme);
  const setTheme = useSettingsStore((s) => s.setTheme);
  const displayUnit = useSettingsStore((s) => s.displayUnit);
  const setDisplayUnit = useSettingsStore((s) => s.setDisplayUnit);
  const inputDevice = useSettingsStore((s) => s.navigation.inputDevice);
  const setInputDevice = useSettingsStore((s) => s.setInputDevice);
  const experimentalWebGpu = useSettingsStore((s) => s.experimentalWebGpu);
  const setExperimentalWebGpu = useSettingsStore((s) => s.setExperimentalWebGpu);
  const snapTo = useSettingsStore((s) => s.snapTo);
  const show = useSettingsStore((s) => s.show);
  const setSnap = useSettingsStore((s) => s.setSnap);
  const setShow = useSettingsStore((s) => s.setShow);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return createPortal(
    <div className="fixed inset-0 z-[220] flex items-center justify-center bg-scrim p-5">
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
        className="flex h-[min(760px,calc(100vh-40px))] w-[min(1080px,calc(100vw-40px))] overflow-hidden rounded-md border border-border bg-surface shadow-popover"
      >
        <aside className="flex w-[220px] shrink-0 flex-col border-r border-border bg-panel p-2">
          <div className="px-2 pb-2 pt-1 text-[13px] font-semibold text-ink-3">Settings</div>
          <nav className="flex flex-col gap-1">
            {TABS.map((entry) => (
              <button
                key={entry.id}
                type="button"
                aria-current={tab === entry.id ? "page" : undefined}
                onClick={() => setTab(entry.id)}
                className={
                  tab === entry.id
                    ? "h-8 cursor-pointer rounded-sm px-2 text-left text-[13px] font-medium text-sel-text bg-sel-bg"
                    : "h-8 cursor-pointer rounded-sm px-2 text-left text-[13px] text-ink-3 hover:bg-hover"
                }
              >
                {entry.label}
              </button>
            ))}
          </nav>
          <span className="flex-1" />
          <Button variant="ghost" size="sm" className="justify-start px-2" onClick={onClose}>
            <Icon name="close" size={14} strokeWidth={2} />
            Close
          </Button>
        </aside>

        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-[44px] items-center border-b border-border px-4">
            <h2 className="text-[14px] font-semibold text-ink-2">Settings</h2>
            <span className="flex-1" />
            <button
              type="button"
              aria-label="Close settings"
              onClick={onClose}
              className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-sm text-ink-4 hover:bg-hover hover:text-ink-2"
            >
              <Icon name="close" size={14} strokeWidth={2} />
            </button>
          </header>

          <div className="min-h-0 flex-1 overflow-auto p-4">
            {tab === "general" && (
              <div className="max-w-[560px] space-y-5">
                <section>
                  <SectionLabel className="pb-1">Navigation</SectionLabel>
                  <SegmentedToggle
                    options={DEVICE_OPTIONS}
                    value={inputDevice}
                    onChange={setInputDevice}
                    ariaLabel="Input device"
                    size="md"
                  />
                </section>
                <section>
                  <SectionLabel className="pb-1">Experimental</SectionLabel>
                  <SwitchRow
                    label="Enable WebGPU renderer"
                    checked={experimentalWebGpu}
                    onChange={setExperimentalWebGpu}
                  />
                </section>
              </div>
            )}

            {tab === "display" && (
              <div className="max-w-[560px] space-y-5">
                <section>
                  <SectionLabel className="pb-1">Appearance</SectionLabel>
                  <SegmentedToggle
                    options={THEME_OPTIONS}
                    value={theme}
                    onChange={setTheme}
                    ariaLabel="Appearance"
                    size="md"
                  />
                </section>
                <section>
                  <SectionLabel className="pb-1">Display mode</SectionLabel>
                  <SegmentedToggle
                    options={DISPLAY_OPTIONS}
                    value={displayMode}
                    onChange={setDisplayMode}
                    ariaLabel="Display mode"
                    size="md"
                    className="w-full"
                  />
                </section>
                <section>
                  <SectionLabel className="pb-1">Units</SectionLabel>
                  <SegmentedToggle
                    options={UNIT_OPTIONS}
                    value={displayUnit}
                    onChange={setDisplayUnit}
                    ariaLabel="Units"
                    size="md"
                  />
                </section>
              </div>
            )}

            {tab === "snap" && (
              <div className="max-w-[560px] space-y-5">
                <section>
                  <SectionLabel className="pb-1">Snap to</SectionLabel>
                  <div className="space-y-0.5">
                    {SNAP_ROWS.map((entry) => (
                      <SwitchRow
                        key={entry.key}
                        label={entry.label}
                        checked={snapTo[entry.key]}
                        onChange={(value) => setSnap(entry.key, value)}
                      />
                    ))}
                  </div>
                </section>
                <section>
                  <SectionLabel className="pb-1">Show</SectionLabel>
                  <div className="space-y-0.5">
                    {SHOW_ROWS.map((entry) => (
                      <SwitchRow
                        key={entry.key}
                        label={entry.label}
                        checked={show[entry.key]}
                        onChange={(value) => setShow(entry.key, value)}
                      />
                    ))}
                  </div>
                </section>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}
