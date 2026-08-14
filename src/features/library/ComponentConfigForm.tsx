/*
 * A component's `role: "free"` params as editable controls (LGU-1 canvas D2-A's
 * CONFIGURE block).
 *
 * ONE form, three homes: the library detail panel (configure before placing),
 * the "Component Parameters" inspector section (re-configure a placed
 * instance), and — next — WP-C's armed placement panel. They differ only in
 * WHEN a change is committed, which is why that is the one thing the caller
 * decides:
 *
 * - `commitOn: "change"` for a local, free configuration (no round trip).
 * - `commitOn: "blur"` where each commit is an IPC call and a re-bake, so
 *   committing per keystroke would queue a regen per digit typed.
 *
 * A domain-driven param always commits on change: picking from a select IS the
 * decision, there is nothing to finish typing.
 */
import type { ComponentParameterSpec, ComponentParamValue } from "@/ipc/types";

export interface ComponentConfigFormProps {
  parameters: Readonly<Record<string, ComponentParameterSpec>>;
  values: Readonly<Record<string, ComponentParamValue>>;
  onChange: (key: string, value: ComponentParamValue) => void;
  /** Defaults to "change"; the inspector section passes "blur" (see above). */
  commitOn?: "change" | "blur";
  /** Key currently mid-commit — its control is disabled until it settles. */
  pendingKey?: string | null;
  /** Reported instead of committing when a numeric falls below `spec.min`. */
  onInvalid?: (message: string) => void;
}

export function ComponentConfigForm({
  parameters,
  values,
  onChange,
  commitOn = "change",
  pendingKey = null,
  onInvalid,
}: ComponentConfigFormProps) {
  const free = Object.entries(parameters).filter(([, spec]) => spec.role === "free");
  if (free.length === 0) return null;

  return (
    <>
      {free.map(([key, spec]) => {
        const value = values[key];
        const disabled = pendingKey === key;
        return (
          <div
            key={key}
            data-testid="component-config-row"
            className="mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2.5"
          >
            <span className="flex-1 truncate text-[12.5px] text-ink-2">{key}</span>
            {spec.domain && spec.domain.length > 0 ? (
              <select
                aria-label={`${key} value`}
                className="rounded-sm border border-border bg-surface px-1 text-[12px] text-ink-2 outline-none"
                value={String(value ?? "")}
                disabled={disabled}
                onChange={(e) => onChange(key, e.target.value)}
              >
                {spec.domain.map((d) => (
                  <option key={String(d)} value={String(d)}>
                    {String(d)}
                  </option>
                ))}
              </select>
            ) : (
              <NumericField
                paramKey={key}
                spec={spec}
                value={value}
                disabled={disabled}
                commitOn={commitOn}
                onChange={onChange}
                onInvalid={onInvalid}
              />
            )}
          </div>
        );
      })}
    </>
  );
}

function NumericField({
  paramKey,
  spec,
  value,
  disabled,
  commitOn,
  onChange,
  onInvalid,
}: {
  paramKey: string;
  spec: ComponentParameterSpec;
  value: ComponentParamValue | undefined;
  disabled: boolean;
  commitOn: "change" | "blur";
  onChange: (key: string, value: ComponentParamValue) => void;
  onInvalid?: (message: string) => void;
}) {
  const commit = (raw: number): void => {
    if (!Number.isFinite(raw)) return;
    if (spec.min !== undefined && raw < spec.min) {
      onInvalid?.(`${paramKey} must be at least ${spec.min}`);
      return;
    }
    if (raw !== value) onChange(paramKey, raw);
  };

  const common = {
    "aria-label": `${paramKey} value`,
    type: "number" as const,
    className:
      "w-16 rounded-sm border border-border bg-surface px-1 text-right text-[12px] text-ink-2 outline-none",
    min: spec.min,
    step: spec.snap,
    disabled,
  };

  /*
   * CONTROLLED in "change" mode, UNCONTROLLED in "blur" mode. A blur-committing
   * field has to keep the half-typed string the user is still editing — making
   * it controlled off `values` would rewrite "1" back to the committed 20 on
   * the first keystroke.
   */
  return commitOn === "change" ? (
    <input
      {...common}
      value={typeof value === "number" ? String(value) : ""}
      onChange={(e) => commit(e.target.valueAsNumber)}
    />
  ) : (
    <input
      {...common}
      defaultValue={typeof value === "number" ? value : ""}
      onBlur={(e) => commit(e.target.valueAsNumber)}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
      }}
    />
  );
}
