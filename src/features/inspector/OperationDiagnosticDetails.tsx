import type { OperationDiagnostic } from "@/ipc/types";

/** Inspector-visible, bounded diagnostic detail. It deliberately has no tooltip-only content. */
export function OperationDiagnosticDetails({ diagnostics }: { diagnostics?: OperationDiagnostic[] }) {
  if (!diagnostics || diagnostics.length === 0) return null;
  return (
    <div data-testid="operation-diagnostics" className="mt-3 space-y-2">
      {diagnostics.map((diagnostic, index) => (
        <div key={`${diagnostic.code}-${index}`} className="rounded-sm border border-border bg-well px-2.5 py-2">
          <div className="text-[11.5px] font-semibold text-traffic-close">{diagnostic.code}</div>
          {diagnostic.stage && <div className="mt-0.5 text-[11px] text-ink-5">Stage: {diagnostic.stage}</div>}
          <div className="mt-1 text-[12px] leading-normal text-ink-2">{diagnostic.message}</div>
          {diagnostic.evidence && (
            <pre data-testid={`operation-diagnostic-evidence-${index}`} className="mt-1.5 overflow-auto whitespace-pre-wrap break-words text-[10.5px] leading-normal text-ink-5">
              {JSON.stringify(diagnostic.evidence, null, 2)}
            </pre>
          )}
        </div>
      ))}
    </div>
  );
}
