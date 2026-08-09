/*
 * The reference addon's React surfaces.
 *
 * `react` is HOST-PROVIDED (ADR-0009's reasoning, applied to React): the host
 * mounts these components into its own tree, so a second React runtime inside an
 * addon bundle would break hooks outright.
 *
 * Styling uses the host's Tailwind tokens by class name only. An addon gets no
 * stylesheet of its own and no raw DOM outside its component (ADR-0003).
 */
import { useSyncExternalStore } from "react";
import type { WidgetStore } from "./domain";

function useWidgets(store: WidgetStore) {
  return useSyncExternalStore(store.subscribe, store.get, store.get);
}

/** A docked panel: the addon's own list, in a host-owned region. */
export function widgetPanel(store: WidgetStore) {
  return function WidgetPanel() {
    const { widgets, selectedId } = useWidgets(store);
    return (
      <div className="p-2 text-[12px] text-ink-3" data-testid="reference-widget-panel">
        <div className="pb-1 font-semibold">Widgets</div>
        <ul>
          {widgets.map((w) => (
            <li key={w.id}>
              <button
                type="button"
                aria-pressed={w.id === selectedId}
                onClick={() => store.select(w.id)}
                className="cursor-pointer border-none bg-transparent text-ink-3"
              >
                {w.name} {w.enabled ? "on" : "off"}
              </button>
            </li>
          ))}
        </ul>
      </div>
    );
  };
}

/** An inspector section, shown only when one of the addon's widgets is selected. */
export function widgetInspectorSection(store: WidgetStore) {
  return function WidgetInspectorSection() {
    const { widgets, selectedId } = useWidgets(store);
    const widget = widgets.find((w) => w.id === selectedId);
    if (!widget) return null;
    return (
      <div className="p-2 text-[12px]" data-testid="reference-widget-inspector">
        <div className="font-semibold">{widget.name}</div>
        <button
          type="button"
          onClick={() => store.toggle(widget.id)}
          className="cursor-pointer border-none bg-transparent text-accent"
        >
          {widget.enabled ? "Disable" : "Enable"}
        </button>
      </div>
    );
  };
}
