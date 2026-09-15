/*
 * Where the user names their local model endpoint.
 *
 * This exists because the assistant is useless without it: `provider.fetch` is
 * refused with "unknown provider" until the Rust-side registry holds something,
 * and the registry is populated only from these two settings (ADR-0017 —
 * configuration changes through a trusted command, never from the model or the
 * sidecar). Before this form there was no way to supply them short of
 * hand-editing `localStorage`.
 *
 * It opens by itself while either field is empty or the endpoint was refused,
 * and collapses once a configuration is accepted, because the only moment this
 * form is interesting is the moment it is wrong.
 *
 * Validation lives in Rust, not here. `validate_provider_base` decides what a
 * legal endpoint is — literal loopback, canonical spelling, no userinfo, no
 * encoded host — and the refusal it returns is rendered by
 * `LocalModelIndicator`. Duplicating any of that check here would create a
 * second answer to "is this endpoint allowed?", and the two would drift.
 */
import { useState } from "react";
import { useSettingsStore, settingsStore } from "@/stores/settingsStore";
import { cn } from "@/ui/cn";
import type { AssistantProviderConfigState } from "./useAssistantProviderConfig";

export interface LocalModelSettingsProps {
  /** The live result of pushing these settings into the Rust-side registry. */
  readonly config: AssistantProviderConfigState;
}

/** Placeholders, not defaults: an unconfigured assistant must stay unconfigured
 *  rather than silently point at a port the user never mentioned. */
const BASE_URL_HINT = "http://127.0.0.1:11434/v1";
const MODEL_HINT = "qwen2.5-coder:7b";

export function LocalModelSettings({ config }: LocalModelSettingsProps) {
  const baseUrl = useSettingsStore((s) => s.assistantProviderBaseUrl);
  const model = useSettingsStore((s) => s.assistantProviderModel);

  const unconfigured = baseUrl.trim() === "" || model.trim() === "";
  const refused = config.status === "error";
  // Open while it matters; the user can reopen it from the summary line.
  const [open, setOpen] = useState(unconfigured || refused);

  if (!open) {
    return (
      <button
        type="button"
        className="flex-none border-b border-border px-3 py-1.5 text-left text-[11px] text-ink-4 hover:text-ink-2"
        data-testid="assistant-model-settings-open"
        onClick={() => setOpen(true)}
      >
        Change local model…
      </button>
    );
  }

  return (
    <section
      className="flex flex-none flex-col gap-1.5 border-b border-border px-3 py-2"
      data-testid="assistant-model-settings"
      aria-label="Local model"
    >
      <p className="text-[11px] text-ink-4">
        The assistant talks only to a model on this machine. Point it at a local
        OpenAI-compatible server.
      </p>

      <label className="flex flex-col gap-0.5 text-[11px] text-ink-4">
        Endpoint
        <input
          type="text"
          className={cn(
            "rounded-sm border border-border bg-surface px-1.5 py-1 font-mono text-[11px] text-ink-2",
            refused && "border-danger-strong",
          )}
          data-testid="assistant-provider-base-url"
          placeholder={BASE_URL_HINT}
          aria-invalid={refused}
          value={baseUrl}
          onChange={(event) =>
            settingsStore.getState().setAssistantProviderBaseUrl(event.target.value)
          }
        />
      </label>

      <label className="flex flex-col gap-0.5 text-[11px] text-ink-4">
        Model
        <input
          type="text"
          className="rounded-sm border border-border bg-surface px-1.5 py-1 font-mono text-[11px] text-ink-2"
          data-testid="assistant-provider-model"
          placeholder={MODEL_HINT}
          value={model}
          onChange={(event) =>
            settingsStore.getState().setAssistantProviderModel(event.target.value)
          }
        />
      </label>

      {!unconfigured && !refused && (
        <button
          type="button"
          className="self-start text-[11px] text-ink-4 hover:text-ink-2"
          data-testid="assistant-model-settings-close"
          onClick={() => setOpen(false)}
        >
          Done
        </button>
      )}
    </section>
  );
}
