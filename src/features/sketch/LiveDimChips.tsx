/*
 * LiveDimChips — the overlay hosts for the live dimension chips (SP-1 Wave 3).
 *
 * One plain DOM host per field, handed to `ViewportEngine.mountChip` so the HTML
 * overlay driver owns its per-frame world→screen transform; React only portals
 * the chip's CONTENT into it. That split is what keeps a pointer move from
 * re-rendering a focused text input: the anchor moves imperatively (the
 * controller calls `moveChip`), and this component re-renders only when a chip's
 * VALUE, focus or placement changes.
 *
 * Anchors are read through `getState()` inside the mount effect, never
 * subscribed — see `liveDimStore` for why.
 */
import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useToolStore } from "@/stores/toolStore";
import { useViewportEngine } from "@/viewport/engineBridge";
import {
  liveDimChipId,
  liveDimStore,
  useLiveDimStore,
  type LiveDimPlacement,
} from "@/stores/liveDimStore";
import type { DimFieldId } from "@/tools/sketch/liveDimension";
import { LiveDimField } from "./LiveDimField";

/**
 * Where the chip sits relative to its anchor. The overlay driver owns
 * `transform` on the HOST, so the nudge lives on the inner span and composes
 * with it untouched (the ConstraintBadgeLayer trick). A flipped chip is pulled
 * fully to the other side by its own width, not merely nudged.
 */
const PLACEMENT_CLASS: Record<LiveDimPlacement, string> = {
  tr: "translate-x-3 -translate-y-3",
  tl: "-translate-x-full -translate-y-3",
  br: "translate-x-3 translate-y-3",
  bl: "-translate-x-full translate-y-3",
};

function hostFor(hosts: Map<DimFieldId, HTMLDivElement>, field: DimFieldId): HTMLDivElement {
  const existing = hosts.get(field);
  if (existing) return existing;
  const el = document.createElement("div");
  el.dataset.liveDimHost = field;
  hosts.set(field, el);
  return el;
}

export function LiveDimChips() {
  const engine = useViewportEngine();
  const mode = useToolStore((s) => s.mode);
  const open = useLiveDimStore((s) => s.open);
  const fields = useLiveDimStore((s) => s.fields);
  const focus = useLiveDimStore((s) => s.focus);
  const text = useLiveDimStore((s) => s.text);
  const placement = useLiveDimStore((s) => s.placement);
  const hosts = useRef(new Map<DimFieldId, HTMLDivElement>());

  const active = open && mode === "sketch";
  // Keyed on the field SET, not on values: re-mounting on every pointer move
  // would tear the focused input out of the DOM several times a second.
  const fieldKey = active ? fields.map((f) => f.field).join(",") : "";
  // Hosts are created during render so the portals below have somewhere to go on
  // the very first pass; `hostFor` is idempotent, so StrictMode's double render
  // reuses the same element.
  if (active) for (const f of fields) hostFor(hosts.current, f.field);

  useEffect(() => {
    if (!engine || fieldKey === "") return;
    const anchors = liveDimStore.getState().anchors;
    // `clusterId` is a per-frame, per-field constant (set by `dimFrame`) — it
    // never changes without the field SET also changing, so reading it here
    // via getState() (rather than subscribing) is safe, same as `anchors`.
    const clusterIds = new Map(liveDimStore.getState().fields.map((f) => [f.field, f.clusterId]));
    const mounted: Array<[string, HTMLDivElement]> = [];
    for (const field of fieldKey.split(",") as DimFieldId[]) {
      const at = anchors[field];
      if (!at) continue;
      const host = hostFor(hosts.current, field);
      // Fields that share a `clusterId` (e.g. a circle's Ø + R) are kept a
      // constant screen gap apart; a field with none is its own cluster of
      // one, so it keeps its real per-field anchor instead of being forced
      // into one shared stack with fields that were never meant to sit near
      // it (a box's W and H, say).
      const clusterId = clusterIds.get(field) ?? liveDimChipId(field);
      engine.mountChip(liveDimChipId(field), host, at, { clusterId });
      mounted.push([liveDimChipId(field), host]);
    }
    return () => {
      for (const [id, host] of mounted) engine.unmountChip(id, host);
    };
  }, [engine, fieldKey]);

  if (!active) return null;

  return (
    <>
      {fields.map((chip) =>
        createPortal(
          <span className={`inline-block ${PLACEMENT_CLASS[placement]}`}>
            <LiveDimField
              chip={chip}
              focused={focus === chip.field}
              text={text}
              onText={(t) => liveDimStore.getState().handlers?.onText(t)}
              onFocus={() => liveDimStore.getState().handlers?.onFocus(chip.field)}
              onTab={(back, v) => liveDimStore.getState().handlers?.onTab(back, v)}
              onEnter={(v) => liveDimStore.getState().handlers?.onEnter(v)}
              onEscape={() => liveDimStore.getState().handlers?.onEscape()}
              onBlur={(v) => liveDimStore.getState().handlers?.onBlur(chip.field, v)}
            />
          </span>,
          hostFor(hosts.current, chip.field),
          chip.field,
        ),
      )}
    </>
  );
}
