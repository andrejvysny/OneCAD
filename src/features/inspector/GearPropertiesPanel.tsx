/**
 * The GEAR properties form (Gear Generator G1-h) — the right-sidebar "properties
 * panel" a FreeCAD user expects, replacing a 16-field floating chip that could
 * not read on screen. Renders whenever `toolChipStore.kind === "gear"`: a fresh
 * placement (`onToolChange("gear")` → `startGear`) or a re-edit
 * (`editGearFeature`) both drive the SAME store, so this is the one host for
 * both — nothing here is placement-specific.
 *
 * Every field reads/writes `toolChipStore`'s gear slice directly — the SAME
 * state and the SAME `onXxx` handlers `ModelToolController.onGearEvent` already
 * wires for the (now field-free) floating chip. This is a second VIEW onto that
 * state, not a second source of truth.
 */
import { useEffect, useState } from "react";
import { SectionLabel } from "@/ui/SectionLabel";
import { cn } from "@/ui/cn";
import { createClient } from "@/ipc/client";
import { useSettingsStore } from "@/stores/settingsStore";
import { useToolChipStore, toolChipStore } from "@/stores/toolChipStore";
import { formatLength, parseLength } from "@/units/format";
import { getModelToolController } from "@/tools/modelTools/modelToolBridge";
import {
  GEAR_MIN_TEETH,
  GEAR_MIN_VALUE,
  gearFsmFromParams,
  gearValueText,
  type GearFsm,
} from "@/tools/modelTools/gearMachine";

/** `dw`/`da`/`df` — pitch / tip / root diameter (mirrors `InvoluteMath.cpp`'s
 *  closed form; `helixAngleDeg` is always 0 in this version). */
function gearDiameters(
  teeth: number,
  module: number,
  shift: number,
  head: number,
  clearance: number,
): { dw: number; da: number; df: number } {
  const dw = module * teeth;
  const da = dw + 2 * module * (1 + shift + head);
  const df = dw - 2 * module * (1 + clearance - shift);
  return { dw, da, df };
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2.5">
      <span className="flex-1 truncate text-[12px] text-ink-5">{label}</span>
      {children}
    </div>
  );
}

/** A plain numeric row — no unit conversion (coefficients, degrees, counts). */
function NumField({
  testid,
  value,
  onValue,
  min,
  integer,
}: {
  testid: string;
  value: number;
  onValue: (v: number) => void;
  min?: number;
  integer?: boolean;
}) {
  const fmt = (v: number) => (integer ? String(Math.round(v)) : Number.isInteger(v) ? String(v) : v.toFixed(2));
  const [text, setText] = useState(() => fmt(value));
  useEffect(() => setText(fmt(value)), [value]); // eslint-disable-line react-hooks/exhaustive-deps

  const commit = (): void => {
    const n = integer ? Math.round(Number(text.trim())) : Number(text.trim());
    if (!Number.isFinite(n) || (min !== undefined && n < min)) {
      setText(fmt(value));
      return;
    }
    if (n !== value) onValue(n);
    setText(fmt(n));
  };

  return (
    <input
      aria-label={testid}
      data-testid={testid}
      className="w-20 rounded-sm border border-border-strong bg-surface px-1 text-right font-mono text-[12px] text-ink-2 outline-none focus:border-accent"
      value={text}
      inputMode={integer ? "numeric" : "decimal"}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
      }}
    />
  );
}

/** A unit-aware length row (module/height/bore dimensions are raw mm on the
 *  wire, displayed per `settingsStore.displayUnit`). */
function LengthField({ testid, value, onValue }: { testid: string; value: number; onValue: (v: number) => void }) {
  const unit = useSettingsStore((s) => s.displayUnit);
  const [text, setText] = useState(() => formatLength(value, unit));
  useEffect(() => setText(formatLength(value, unit)), [value, unit]);

  const commit = (): void => {
    const n = parseLength(text.trim(), unit);
    if (n === undefined || n === null || !Number.isFinite(n) || n < GEAR_MIN_VALUE) {
      setText(formatLength(value, unit));
      return;
    }
    if (n !== value) onValue(n);
    setText(formatLength(n, unit));
  };

  return (
    <input
      aria-label={testid}
      data-testid={testid}
      className="w-20 rounded-sm border border-border-strong bg-surface px-1 text-right font-mono text-[12px] text-ink-2 outline-none focus:border-accent"
      value={text}
      inputMode="decimal"
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
      }}
    />
  );
}

function ToggleField({ testid, pressed, onToggle }: { testid: string; pressed: boolean; onToggle: () => void }) {
  return (
    <button
      type="button"
      data-testid={testid}
      aria-pressed={pressed}
      onClick={onToggle}
      className={cn(
        "rounded-sm px-2 py-1 text-[11.5px] font-medium",
        pressed ? "bg-sel-bg text-sel-text" : "bg-surface text-ink-4 hover:bg-hover-2",
      )}
    >
      {pressed ? "On" : "Off"}
    </button>
  );
}

export function GearPropertiesPanel() {
  const teeth = useToolChipStore((s) => s.count);
  const module = useToolChipStore((s) => s.value);
  const height = useToolChipStore((s) => s.gearHeight);
  const pressureAngleDeg = useToolChipStore((s) => s.gearPressureAngleDeg);
  const shift = useToolChipStore((s) => s.gearShift);
  const clearance = useToolChipStore((s) => s.gearClearance);
  const head = useToolChipStore((s) => s.gearHead);
  const backlash = useToolChipStore((s) => s.gearBacklash);
  const undercut = useToolChipStore((s) => s.gearUndercut);
  const propertiesFromTool = useToolChipStore((s) => s.gearPropertiesFromTool);
  const sampleCount = useToolChipStore((s) => s.gearSampleCount);
  const axleHole = useToolChipStore((s) => s.gearAxleHole);
  const axleHoleDiameter = useToolChipStore((s) => s.gearAxleHoleDiameter);
  const offsetHole = useToolChipStore((s) => s.gearOffsetHole);
  const offsetHoleDiameter = useToolChipStore((s) => s.gearOffsetHoleDiameter);
  const offsetHoleOffset = useToolChipStore((s) => s.gearOffsetHoleOffset);
  const unit = useSettingsStore((s) => s.displayUnit);

  const { dw, da, df } = gearDiameters(teeth, module, shift, head, clearance);

  return (
    <div data-testid="gear-properties-panel">
      <div className="text-[15px] font-semibold text-ink">Gear</div>
      <div className="mt-0.5 text-[12px] text-ink-5">{gearValueText(teeth, module)}</div>

      <div className="mt-3 flex gap-2">
        <button
          type="button"
          data-testid="gear-properties-apply"
          onClick={() => toolChipStore.getState().onConfirm?.()}
          className="flex-1 rounded-sm bg-accent px-2.5 py-1.5 text-[12px] font-medium text-on-accent hover:bg-accent-hover"
        >
          Apply
        </button>
        <button
          type="button"
          data-testid="gear-properties-cancel"
          onClick={() => toolChipStore.getState().onCancel?.()}
          className="rounded-sm bg-chip px-2.5 py-1.5 text-[12px] font-medium text-ink-3 hover:bg-hover-2"
        >
          Cancel
        </button>
      </div>

      <SectionLabel className="pb-1.5 pt-4">Base</SectionLabel>
      <Row label="Teeth">
        <NumField
          testid="gear-prop-teeth"
          value={teeth}
          min={GEAR_MIN_TEETH}
          integer
          onValue={(v) => toolChipStore.getState().onTeeth?.(v)}
        />
      </Row>
      <Row label="Module">
        <LengthField testid="gear-prop-module" value={module} onValue={(v) => toolChipStore.getState().onValue?.(v)} />
      </Row>
      <Row label="Face width">
        <LengthField
          testid="gear-prop-height"
          value={height}
          onValue={(v) => toolChipStore.getState().onGearHeight?.(v)}
        />
      </Row>

      <SectionLabel className="pb-1.5 pt-4">Tooth</SectionLabel>
      <Row label="Pressure angle °">
        <NumField
          testid="gear-prop-pressure-angle"
          value={pressureAngleDeg}
          min={0.1}
          onValue={(v) => toolChipStore.getState().onGearPressureAngle?.(v)}
        />
      </Row>
      <Row label="Profile shift x">
        <NumField testid="gear-prop-shift" value={shift} onValue={(v) => toolChipStore.getState().onGearShift?.(v)} />
      </Row>
      <Row label="Clearance c">
        <NumField
          testid="gear-prop-clearance"
          value={clearance}
          min={0}
          onValue={(v) => toolChipStore.getState().onGearClearance?.(v)}
        />
      </Row>
      <Row label="Head ha">
        <NumField testid="gear-prop-head" value={head} onValue={(v) => toolChipStore.getState().onGearHead?.(v)} />
      </Row>
      <Row label="Backlash">
        <NumField
          testid="gear-prop-backlash"
          value={backlash}
          min={0}
          onValue={(v) => toolChipStore.getState().onGearBacklash?.(v)}
        />
      </Row>
      <Row label="Undercut">
        <ToggleField
          testid="gear-prop-undercut"
          pressed={undercut}
          onToggle={() => toolChipStore.getState().onGearUndercut?.(!undercut)}
        />
      </Row>
      <Row label="Properties from tool">
        <ToggleField
          testid="gear-prop-from-tool"
          pressed={propertiesFromTool}
          onToggle={() => toolChipStore.getState().onGearPropertiesFromTool?.(!propertiesFromTool)}
        />
      </Row>

      <SectionLabel className="pb-1.5 pt-4">Bores</SectionLabel>
      <Row label="Axle hole">
        <ToggleField
          testid="gear-prop-axle-hole"
          pressed={axleHole}
          onToggle={() => toolChipStore.getState().onGearAxleHole?.(!axleHole)}
        />
      </Row>
      {axleHole && (
        <Row label="Axle ⌀">
          <LengthField
            testid="gear-prop-axle-diameter"
            value={axleHoleDiameter}
            onValue={(v) => toolChipStore.getState().onGearAxleHoleDiameter?.(v)}
          />
        </Row>
      )}
      <Row label="Offset hole">
        <ToggleField
          testid="gear-prop-offset-hole"
          pressed={offsetHole}
          onToggle={() => toolChipStore.getState().onGearOffsetHole?.(!offsetHole)}
        />
      </Row>
      {offsetHole && (
        <>
          <Row label="Offset ⌀">
            <LengthField
              testid="gear-prop-offset-diameter"
              value={offsetHoleDiameter}
              onValue={(v) => toolChipStore.getState().onGearOffsetHoleDiameter?.(v)}
            />
          </Row>
          <Row label="Offset radius">
            <LengthField
              testid="gear-prop-offset-offset"
              value={offsetHoleOffset}
              onValue={(v) => toolChipStore.getState().onGearOffsetHoleOffset?.(v)}
            />
          </Row>
        </>
      )}

      <SectionLabel className="pb-1.5 pt-4">Accuracy</SectionLabel>
      <Row label="Spline samples">
        <NumField
          testid="gear-prop-sample-count"
          value={sampleCount}
          min={2}
          integer
          onValue={(v) => toolChipStore.getState().onGearSampleCount?.(v)}
        />
      </Row>

      <SectionLabel className="pb-1.5 pt-4">Computed</SectionLabel>
      <div data-testid="gear-prop-diameters" className="mb-1 rounded-sm bg-chip px-2.5 py-2 font-mono text-[11.5px] text-ink-4">
        <div>da (tip) {formatLength(da, unit)}</div>
        <div>dw (pitch) {formatLength(dw, unit)}</div>
        <div>df (root) {formatLength(df, unit)}</div>
      </div>
    </div>
  );
}

function ReadRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="mb-1 flex h-[26px] items-center gap-2 rounded-sm bg-chip px-2.5">
      <span className="flex-1 truncate text-[12px] text-ink-5">{label}</span>
      <span className="font-mono text-[12px] text-ink-2">{value}</span>
    </div>
  );
}

/**
 * A read-only properties card for an ALREADY-COMMITTED Gear feature, shown
 * when its body or history row is SELECTED but the tool is not armed — the
 * FreeCAD "select an object, see its properties" moment. "Edit parameters"
 * hands off to `editGearFeature`, which arms live editing and swaps this card
 * for {@link GearPropertiesPanel} (same store, same fields).
 */
export function GearSelectedSummary({ featureId }: { featureId: string }) {
  const [seeded, setSeeded] = useState<GearFsm | null | undefined>(undefined);
  const unit = useSettingsStore((s) => s.displayUnit);

  useEffect(() => {
    let alive = true;
    setSeeded(undefined);
    void createClient()
      .getOperationParams(featureId)
      .then((stored) => {
        if (alive) setSeeded(gearFsmFromParams(stored));
      })
      .catch(() => {
        if (alive) setSeeded(null);
      });
    return () => {
      alive = false;
    };
  }, [featureId]);

  if (seeded === undefined) return <div className="mt-3 text-[12px] text-ink-6">Loading…</div>;
  if (seeded === null) {
    return <div className="mt-3 text-[12px] text-ink-6">This gear's stored parameters are unavailable.</div>;
  }

  const { dw, da, df } = gearDiameters(seeded.teeth, seeded.module, seeded.shift, seeded.head, seeded.clearance);

  return (
    <div data-testid="gear-selected-summary">
      <div className="mt-0.5 text-[12px] text-ink-5">{gearValueText(seeded.teeth, seeded.module)}</div>
      <button
        type="button"
        data-testid="gear-edit-parameters"
        onClick={() => void getModelToolController()?.editGearFeature(featureId)}
        className="mt-3 w-full rounded-sm bg-accent px-2.5 py-1.5 text-[12px] font-medium text-on-accent hover:bg-accent-hover"
      >
        Edit parameters
      </button>

      <SectionLabel className="pb-1.5 pt-4">Base</SectionLabel>
      <ReadRow label="Teeth" value={String(seeded.teeth)} />
      <ReadRow label="Module" value={formatLength(seeded.module, unit)} />
      <ReadRow label="Face width" value={formatLength(seeded.height, unit)} />

      <SectionLabel className="pb-1.5 pt-4">Tooth</SectionLabel>
      <ReadRow label="Pressure angle °" value={seeded.pressureAngleDeg.toFixed(1)} />
      <ReadRow label="Profile shift x" value={seeded.shift.toFixed(2)} />
      <ReadRow label="Clearance c" value={seeded.clearance.toFixed(2)} />
      <ReadRow label="Head ha" value={seeded.head.toFixed(2)} />
      <ReadRow label="Backlash" value={seeded.backlash.toFixed(2)} />
      <ReadRow label="Undercut" value={seeded.undercut ? "On" : "Off"} />

      {seeded.axleHole && <ReadRow label="Axle ⌀" value={formatLength(seeded.axleHoleDiameter, unit)} />}
      {seeded.offsetHole && (
        <>
          <ReadRow label="Offset ⌀" value={formatLength(seeded.offsetHoleDiameter, unit)} />
          <ReadRow label="Offset radius" value={formatLength(seeded.offsetHoleOffset, unit)} />
        </>
      )}

      <SectionLabel className="pb-1.5 pt-4">Computed</SectionLabel>
      <div className="mb-1 rounded-sm bg-chip px-2.5 py-2 font-mono text-[11.5px] text-ink-4">
        <div>da (tip) {formatLength(da, unit)}</div>
        <div>dw (pitch) {formatLength(dw, unit)}</div>
        <div>df (root) {formatLength(df, unit)}</div>
      </div>
    </div>
  );
}
