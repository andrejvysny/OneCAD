import type {
  ActiveToolBodyReference,
  ActiveToolElementReference,
  ActiveToolPresentation,
  ActiveToolSketchReference,
} from "@/tools/modelTools/activeToolPresentation";
import { useDocumentStore } from "@/stores/documentStore";

type PresentedTargets = NonNullable<ActiveToolPresentation["targets"]>;
type TargetFacts = {
  bodies: Record<string, { name: string }>;
  sketches: Record<string, { name: string }>;
};

function identityLabel(resolution: ActiveToolElementReference["resolution"]): string | null {
  if (resolution === "identity-only") return "Identity only; topology not verified";
  if (resolution === "unresolved") return "Prepared reference; unresolved";
  if (resolution === "missing") return "Missing reference";
  return null;
}

function bodyLabel(reference: ActiveToolBodyReference, facts: TargetFacts): string {
  return facts.bodies[reference.bodyId]?.name ?? reference.label;
}

function sketchLabel(reference: ActiveToolSketchReference, facts: TargetFacts): string {
  return facts.sketches[reference.sketchId]?.name ?? reference.label;
}

function elementLabel(reference: ActiveToolElementReference, facts: TargetFacts): string {
  const kind = reference.kind === "edge" ? "Edge" : "Face";
  const body = facts.bodies[reference.bodyId];
  return body ? `${body.name} · ${kind}` : `Missing body (${reference.bodyId}) · ${kind}`;
}

function BodyReference({ reference, label, facts, testId }: {
  reference: ActiveToolBodyReference;
  label: string;
  facts: TargetFacts;
  testId?: string;
}) {
  return (
    <div className="min-w-0" data-testid={testId ?? "inspector-target-body"}>
      <span className="mr-1 text-ink-5">{label}</span>
      <span className="break-words">{bodyLabel(reference, facts)}</span>
      {reference.resolution === "missing" && <span className="ml-1 text-warn">(missing)</span>}
      <details className="mt-0.5 text-[11px] text-ink-5">
        <summary aria-label={`${label} reference details`} className="cursor-pointer">Reference details</summary>
        <code className="block break-all" title={reference.bodyId}>Body ID: {reference.bodyId}</code>
      </details>
    </div>
  );
}

function SketchReference({ reference, facts }: { reference: ActiveToolSketchReference; facts: TargetFacts }) {
  return (
    <div className="min-w-0" data-testid="inspector-target-sketch">
      <span className="mr-1 text-ink-5">Sketch</span>
      <span className="break-words">{sketchLabel(reference, facts)}</span>
      {reference.resolution === "missing" && <span className="ml-1 text-warn">(missing)</span>}
      <details className="mt-0.5 text-[11px] text-ink-5">
        <summary aria-label="Sketch reference details" className="cursor-pointer">Reference details</summary>
        <code className="block break-all" title={reference.sketchId}>Sketch ID: {reference.sketchId}</code>
      </details>
    </div>
  );
}

function ElementReference({ reference, label, facts }: {
  reference: ActiveToolElementReference;
  label: string;
  facts: TargetFacts;
}) {
  const resolution = identityLabel(reference.resolution);
  return (
    <div className="min-w-0" data-testid="inspector-target-element">
      <span className="mr-1 text-ink-5">{label}</span>
      <span className="break-words">{elementLabel(reference, facts)}</span>
      {reference.resolution === "missing" && <span className="ml-1 text-warn">(missing)</span>}
      <details className="mt-0.5 text-[11px] text-ink-5">
        <summary aria-label={`${label} reference details`} className="cursor-pointer">Reference details</summary>
        {resolution && <span className="block">{resolution}</span>}
        <code className="block break-all" title={reference.bodyId}>Body ID: {reference.bodyId}</code>
        {reference.elementId && <code className="block break-all" title={reference.elementId}>Element ID: {reference.elementId}</code>}
      </details>
    </div>
  );
}

function ElementSlot({ reference, label, facts }: {
  reference: ActiveToolElementReference | null;
  label: string;
  facts: TargetFacts;
}) {
  if (!reference) {
    return <div className="min-w-0"><span className="mr-1 text-ink-5">{label}</span>Not selected</div>;
  }
  return <ElementReference reference={reference} label={label} facts={facts} />;
}

function EmptyList({ label }: { label: string }) {
  return <div className="text-ink-5">{label}: None selected</div>;
}

function BodyList({ label, references, facts }: {
  label: string;
  references: ActiveToolBodyReference[];
  facts: TargetFacts;
}) {
  if (references.length === 0) return <EmptyList label={label} />;
  return (
    <div>
      <div className="text-ink-5">{label}</div>
      {references.map((reference, index) => (
        <BodyReference key={`${reference.bodyId}-${index}`} reference={reference} label={`${index + 1}.`} facts={facts} />
      ))}
    </div>
  );
}

function ElementList({ label, references, facts }: {
  label: string;
  references: ActiveToolElementReference[];
  facts: TargetFacts;
}) {
  if (references.length === 0) return <EmptyList label={label} />;
  return (
    <div>
      <div className="text-ink-5">{label}</div>
      {references.map((reference, index) => (
        <ElementReference key={`${reference.bodyId}-${reference.elementId ?? "unresolved"}-${index}`} reference={reference} label={`${index + 1}.`} facts={facts} />
      ))}
    </div>
  );
}

type ProfileTargets = Extract<PresentedTargets, { kind: "profile" }>;

function ProfileTargets({ targets, facts }: { targets: ProfileTargets; facts: TargetFacts }) {
  return (
    <>
      <SketchReference reference={targets.sketch} facts={facts} />
      <div>Regions: {targets.regionIds.length}</div>
      <BodyList label="Affected bodies" references={targets.hostBodies} facts={facts} />
      <div>
        Direction: {targets.direction
          ? targets.direction.kind === "normal"
            ? `Normal [${targets.direction.vector.join(", ")}]`
            : `Sketch line ${targets.direction.lineId}`
          : "Not selected"}
      </div>
    </>
  );
}

function TargetDetails({ targets, facts }: { targets: PresentedTargets; facts: TargetFacts }) {
  switch (targets.kind) {
    case "profile": return <ProfileTargets targets={targets} facts={facts} />;
    case "regions":
      return <><SketchReference reference={targets.sketch} facts={facts} /><div>Regions: {targets.selectedRegionIds.length}</div></>;
    case "edgeOperation":
      return <>
        <BodyList label="Affected bodies" references={targets.affectedBodies} facts={facts} />
        <ElementList label="Edges" references={targets.edges} facts={facts} />
        {targets.referenceFaces.map((pair, index) => <div key={index}>
          <ElementSlot reference={pair.a} label={`Reference A${targets.referenceFaces.length > 1 ? ` ${index + 1}` : ""}`} facts={facts} />
          <ElementSlot reference={pair.b} label={`Reference B${targets.referenceFaces.length > 1 ? ` ${index + 1}` : ""}`} facts={facts} />
        </div>)}
      </>;
    case "faces":
      return <>
        <BodyList label="Affected bodies" references={targets.affectedBodies} facts={facts} />
        <ElementList label="Face targets" references={targets.faces} facts={facts} />
        {targets.oppositeFace && <ElementReference reference={targets.oppositeFace} label="Opposite face" facts={facts} />}
      </>;
    case "bodies": return <BodyList label="Bodies" references={targets.bodies} facts={facts} />;
    case "boolean": return <>
      <BodyReference reference={targets.target} label="Boolean Target" facts={facts} testId="chip-bool-target" />
      <BodyReference reference={targets.toolBody} label="Boolean Tool" facts={facts} testId="chip-bool-tool" />
    </>;
    case "datum": return <div>Datum base: {targets.baseLabel || "Unavailable"} <code className="break-all">({targets.baseId ?? "none"})</code></div>;
    case "gear": return targets.support
      ? <ElementReference reference={targets.support} label="Gear support" facts={facts} />
      : <div>Gear support: World/free support</div>;
  }
}

export function ActiveToolTargets({ targets }: { targets: ActiveToolPresentation["targets"] }) {
  const bodyFacts = useDocumentStore((current) => current.bodies);
  const sketchFacts = useDocumentStore((current) => current.sketches);
  const facts: TargetFacts = { bodies: bodyFacts, sketches: sketchFacts };
  return (
    <section
      aria-label="Tool targets"
      data-testid="active-tool-targets"
      className="mb-3 min-w-0 max-w-full rounded-md border border-border bg-surface px-2.5 py-2 text-[12px] text-ink-3"
    >
      <div className="mb-1 font-medium text-ink">Targets</div>
      {targets ? <div className="flex min-w-0 flex-col gap-1"><TargetDetails targets={targets} facts={facts} /></div> : (
        <div data-testid="inspector-targets-missing" className="text-ink-5">
          Tool targets unavailable — cancel and reopen the tool.
        </div>
      )}
    </section>
  );
}
