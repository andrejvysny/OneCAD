/*
 * The MINIMAL P1 material editor.
 *
 * SCOPE, stated so the gap is not mistaken for an oversight: this edits the
 * handful of parameters that decide what a material READS as — its colour, how
 * metallic it is, how rough, how transparent — plus two whole-lobe toggles
 * (coat, emission). It is NOT the gray-out editor: OpenPBR's remaining tiers
 * (fuzz, subsurface, thin film, anisotropy, the transmission stack) are P3's,
 * together with the "what does this parameter do" affordances that make them
 * usable. Offering thirty sliders with no such affordance would be worse than
 * offering six.
 *
 * COMMIT ON SAVE, not per keystroke. Every write goes through the store's
 * optimistic-then-persist path, which is a document TRANSACTION — so a
 * per-keystroke editor would push one undo entry per character and make Ctrl-Z
 * useless on a material. This is the same reason `sections.tsx` commits a
 * dimension on blur/Enter rather than on change.
 *
 * A TOGGLE ADDS OR REMOVES A LAYER, and that is exactly the model's own
 * convention: an ABSENT layer means "spec default, lobe off", so turning coat
 * off deletes the sub-object rather than writing `coat_weight: 0`. The record
 * stays sparse, and a material that never had a coat never grows the key.
 */
import { useState } from "react";
import { createPortal } from "react-dom";

import { Button } from "@/ui/Button";
import { TextInput } from "@/ui/TextInput";
import { Switch } from "@/ui/Switch";
import { SectionLabel } from "@/ui/SectionLabel";

import { OPENPBR_DEFAULTS, type MaterialDef } from "../model/material";
import { renderStore } from "../store/renderStore";
import { useRenderDialogStore, renderDialogStore } from "./dialogStore";
import { colorToInputValue, inputValueToColor } from "./materialColor";
import { useRenderStore } from "./state";

/** One 0..1 slider with a numeric readout, the shape `ColorControls` uses. */
function UnitSlider({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (next: number) => void;
}) {
  return (
    <label className="mt-2 flex items-center gap-2">
      <span className="w-[92px] flex-none text-[12px] text-ink-4">{label}</span>
      <input
        type="range"
        min={0}
        max={1}
        step={0.01}
        value={value}
        aria-label={label}
        onChange={(e) => onChange(Number(e.target.value))}
        className="flex-1"
      />
      <span className="w-8 text-right font-mono text-[11px] text-ink-5">{value.toFixed(2)}</span>
    </label>
  );
}

function ColorRow({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (next: string) => void;
}) {
  return (
    <label className="mt-2 flex items-center gap-2">
      <span className="w-[92px] flex-none text-[12px] text-ink-4">{label}</span>
      <input
        type="color"
        value={value}
        aria-label={label}
        onChange={(e) => onChange(e.target.value)}
        className="h-8 w-8 cursor-pointer rounded border-0 bg-transparent p-0"
      />
    </label>
  );
}

/** The draft the sheet edits. Flat on purpose — layers are re-assembled on save. */
interface Draft {
  name: string;
  baseColor: string;
  metalness: number;
  roughness: number;
  opacity: number;
  coat: boolean;
  coatWeight: number;
  coatRoughness: number;
  emission: boolean;
  emissionColor: string;
  emissionLuminance: number;
}

const D = OPENPBR_DEFAULTS;

export function draftFrom(def: MaterialDef): Draft {
  return {
    name: def.name,
    baseColor: colorToInputValue(def.base.base_color),
    metalness: def.base.base_metalness ?? D.base.base_metalness,
    roughness: def.specular?.specular_roughness ?? D.specular.specular_roughness,
    opacity: def.geometry?.geometry_opacity ?? D.geometry.geometry_opacity,
    coat: def.coat !== undefined,
    coatWeight: def.coat?.coat_weight ?? 1,
    coatRoughness: def.coat?.coat_roughness ?? D.coat.coat_roughness,
    emission: def.emission !== undefined,
    emissionColor: colorToInputValue(def.emission?.emission_color),
    emissionLuminance: def.emission?.emission_luminance ?? 0,
  };
}

/**
 * Draft → def, PRESERVING every layer the sheet does not edit.
 *
 * Spreading `def` first is load-bearing: a material carrying a transmission
 * lobe (every "Clear Glass" in the starter set does) must not lose it because
 * this editor has no field for it. Only the keys below are authored.
 */
export function applyDraft(def: MaterialDef, draft: Draft): MaterialDef {
  const next: MaterialDef = {
    ...def,
    name: draft.name.trim() || def.name,
    base: {
      ...def.base,
      base_color: inputValueToColor(draft.baseColor),
      base_metalness: draft.metalness,
    },
    specular: { ...def.specular, specular_roughness: draft.roughness },
    geometry: { ...def.geometry, geometry_opacity: draft.opacity },
  };
  if (draft.coat) {
    next.coat = { ...def.coat, coat_weight: draft.coatWeight, coat_roughness: draft.coatRoughness };
  } else {
    delete next.coat;
  }
  if (draft.emission) {
    next.emission = {
      ...def.emission,
      emission_color: inputValueToColor(draft.emissionColor),
      emission_luminance: draft.emissionLuminance,
    };
  } else {
    delete next.emission;
  }
  return next;
}

function EditorBody({ def, onClose }: { def: MaterialDef; onClose: () => void }) {
  const [draft, setDraft] = useState<Draft>(() => draftFrom(def));
  const readOnly = useRenderStore((s) => s.readOnly);
  const patch = (p: Partial<Draft>) => setDraft((d) => ({ ...d, ...p }));

  const save = () => {
    void renderStore.getState().upsertMaterial(applyDraft(def, draft));
    onClose();
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Edit material"
      data-testid="material-editor"
      onClick={(e) => e.stopPropagation()}
      className="w-[380px] rounded-lg border border-border bg-surface p-[20px_22px] font-ui shadow-popover"
    >
      <div className="text-[14px] font-semibold text-ink">Edit material</div>

      <label className="mt-3 block">
        <span className="mb-1 block text-[12px] text-ink-4">Name</span>
        <TextInput
          value={draft.name}
          aria-label="Material name"
          wrapperClassName="w-full"
          onChange={(e) => patch({ name: e.target.value })}
        />
      </label>

      <SectionLabel className="pb-1 pt-4">Surface</SectionLabel>
      <ColorRow label="Base color" value={draft.baseColor} onChange={(v) => patch({ baseColor: v })} />
      <UnitSlider label="Metalness" value={draft.metalness} onChange={(v) => patch({ metalness: v })} />
      <UnitSlider label="Roughness" value={draft.roughness} onChange={(v) => patch({ roughness: v })} />
      <UnitSlider label="Opacity" value={draft.opacity} onChange={(v) => patch({ opacity: v })} />

      <div className="mt-4 flex items-center justify-between">
        <SectionLabel>Coat</SectionLabel>
        <Switch
          checked={draft.coat}
          ariaLabel="Coat"
          onChange={(on) => patch({ coat: on })}
        />
      </div>
      {draft.coat && (
        <>
          <UnitSlider
            label="Coat weight"
            value={draft.coatWeight}
            onChange={(v) => patch({ coatWeight: v })}
          />
          <UnitSlider
            label="Coat roughness"
            value={draft.coatRoughness}
            onChange={(v) => patch({ coatRoughness: v })}
          />
        </>
      )}

      <div className="mt-4 flex items-center justify-between">
        <SectionLabel>Emission</SectionLabel>
        <Switch
          checked={draft.emission}
          ariaLabel="Emission"
          onChange={(on) => patch({ emission: on })}
        />
      </div>
      {draft.emission && (
        <>
          <ColorRow
            label="Emission color"
            value={draft.emissionColor}
            onChange={(v) => patch({ emissionColor: v })}
          />
          <label className="mt-2 flex items-center gap-2">
            <span className="w-[92px] flex-none text-[12px] text-ink-4">Luminance</span>
            <input
              type="number"
              min={0}
              step={10}
              aria-label="Luminance"
              value={draft.emissionLuminance}
              onChange={(e) => patch({ emissionLuminance: Math.max(0, e.target.valueAsNumber || 0) })}
              className="w-24 rounded-sm border border-border-strong bg-surface px-1 text-right font-mono text-[12px] text-ink-2 outline-none focus:border-accent"
            />
            <span className="text-[11px] text-ink-6">nits</span>
          </label>
        </>
      )}

      <div className="mt-5 flex justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" size="sm" disabled={readOnly} onClick={save}>
          Save
        </Button>
      </div>
    </div>
  );
}

export function MaterialEditorSheet() {
  const editing = useRenderDialogStore((s) => s.editor);
  const close = renderDialogStore.getState().closeEditor;
  const def = useRenderStore((s) => (editing === null ? undefined : s.state.library[editing]));

  // A material deleted while its editor was open (or a stale id after a
  // re-hydrate) has nothing to edit — close rather than render an empty sheet.
  if (editing === null || def === undefined) return null;

  return createPortal(
    <div
      className="fixed inset-0 z-[95] flex items-start justify-center bg-scrim pt-[110px]"
      onClick={close}
    >
      {/* Keyed on the material: opening a DIFFERENT material must reset the
          draft, and a plain re-render would keep the previous one's. */}
      <EditorBody key={def.id} def={def} onClose={close} />
    </div>,
    document.body,
  );
}
