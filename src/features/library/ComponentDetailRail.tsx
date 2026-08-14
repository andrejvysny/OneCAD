/*
 * The library detail panel (LGU-1 canvas D2-A) — configurator-first.
 *
 * Reordered from the identity-dump it replaced. The old rail led with rows of
 * internal truth (`Version`, `Source: generator`, attachment keys printed as
 * `head_seat — cylinder:shank`) and offered no way to configure anything before
 * placing it. The order now answers the questions a user actually arrives with:
 * what is this (name + designation + badge), what does it look like (preview),
 * what can I choose (CONFIGURE), where does it attach (ATTACHES TO), can I
 * place it (the action) — and only then, in a single footer line, the
 * provenance a version comparison needs.
 *
 * The configuration made here is CARRIED into the placement that follows
 * (`componentConfigStore`), because picking M8 and then watching the ghost arm
 * at M6 would make this panel a lie.
 */
import { useEffect, useState } from "react";
import { Button } from "@/ui/Button";
import { SourceBadge } from "@/ui/SourceBadge";
import { ComponentPreview3D } from "./ComponentPreview3D";
import { ComponentConfigForm } from "./ComponentConfigForm";
import { componentConfigStore } from "@/modules/library/componentConfigStore";
import { defaultParamValues, formatDesignation } from "@/modules/library/designation";
import { attachmentPhraseOf, badgeKindOf, provenancePhraseOf } from "@/modules/library/componentMeta";
import type { ComponentParamValue, LibraryComponent } from "@/ipc/types";

function SectionHeading({ children }: { children: string }) {
  return (
    <div className="mb-1.5 mt-3 text-[10px] font-bold uppercase tracking-[0.1em] text-ink-3">
      {children}
    </div>
  );
}

export interface ComponentDetailRailProps {
  component: LibraryComponent;
  /** Absent on the Start screen — there is no open document to place into. */
  canPlace?: boolean;
  onPlace: (component: LibraryComponent, params: Record<string, ComponentParamValue>) => void;
}

export function ComponentDetailRail({ component, canPlace, onPlace }: ComponentDetailRailProps) {
  const [values, setValues] = useState<Record<string, ComponentParamValue>>({});

  // Seeded per component from whatever the user last configured for it, else
  // the package's own defaults. Re-seeds when the selection changes rather than
  // carrying the previous card's numbers onto this one.
  useEffect(() => {
    const carried = componentConfigStore.getState().get(component.id, component.version);
    setValues(carried ?? defaultParamValues(component.parameters));
  }, [component.id, component.version, component.parameters]);

  const change = (key: string, value: ComponentParamValue): void => {
    const next = { ...values, [key]: value };
    setValues(next);
    componentConfigStore.getState().set(component.id, component.version, next);
  };

  const designation = component.designation
    ? formatDesignation(component.designation, values)
    : null;
  const attachments = Object.entries(component.attachments);

  return (
    <>
      <div className="mb-3 flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="truncate text-[14px] font-[650] text-ink">{component.name}</div>
          {designation !== null && (
            <div
              data-testid="detail-designation"
              className="truncate font-mono text-[11.5px] text-[var(--designation-fg)]"
            >
              {designation}
            </div>
          )}
        </div>
        <SourceBadge kind={badgeKindOf(component)} className="mt-[3px] shrink-0" />
      </div>

      {/* The live, orbitable preview (WP-B6) — one WebGL context, mounted only
          while a component is selected in this modal. The orbit affordance sits
          INSIDE the canvas bounds: as app chrome it read as a property of the
          panel and leaked outside it. */}
      <div className="relative mb-1 h-[260px]">
        <ComponentPreview3D
          componentId={component.id}
          componentVersion={component.version}
          params={values}
          fill
        />
        <div className="pointer-events-none absolute bottom-1.5 left-2 select-none text-[10px] text-ink-7">
          Drag to orbit
        </div>
      </div>

      <SectionHeading>Configure</SectionHeading>
      <ComponentConfigForm
        parameters={component.parameters}
        values={values}
        onChange={change}
        commitOn="change"
      />
      {Object.values(component.parameters).every((p) => p.role !== "free") && (
        <div className="text-[11.5px] text-ink-7">Fixed geometry — nothing to configure.</div>
      )}

      {attachments.length > 0 && (
        <>
          <SectionHeading>Attaches to</SectionHeading>
          <ul>
            {attachments.map(([key, attachment]) => (
              <li
                key={key}
                data-testid="detail-attachment"
                className="mb-0.5 flex items-center gap-1.5 truncate text-[11.5px] text-ink-2"
              >
                <span
                  aria-hidden="true"
                  className="h-1.5 w-1.5 shrink-0 rounded-full bg-[var(--designation-fg)]"
                />
                {attachmentPhraseOf(key, attachment)}
              </li>
            ))}
          </ul>
        </>
      )}

      <div className="mt-3">
        {canPlace ? (
          <Button variant="primary" onClick={() => onPlace(component, values)}>
            Place in scene
          </Button>
        ) : (
          <Button variant="primary" disabled data-testid="detail-place-disabled">
            Place — open a project first
          </Button>
        )}
      </div>

      <div className="mt-3 border-t border-border pt-2 text-[11px] leading-normal text-ink-3">
        <span className="break-all">{component.id}</span>
        {" · "}v{component.version}
        {" · "}
        {provenancePhraseOf(component)}
        {component.tags.length > 0 ? ` · ${component.tags.join(", ")}` : ""}
      </div>
    </>
  );
}
