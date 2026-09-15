/*
 * The settings → Rust registry path, from the user's side (ADR-0017).
 *
 * Three properties, and the third is the one that was missing in production:
 *
 *   1. What the frontend sends is a PROPOSAL of `{id, baseUrl, model}` and
 *      nothing else — no credential, no pre-approval of the URL.
 *   2. An assistant that is off, or a half-filled endpoint, sends `null` and
 *      CLEARS the registry. A cleared registry answers `provider.fetch`
 *      `unimplemented`, which is what a build with no local model should say.
 *   3. A refusal is on screen. Rust rejects anything that is not a canonical
 *      literal loopback base; if that refusal were swallowed, the user would see
 *      "unknown provider" on every later answer and never learn about the typo.
 *
 * Tauri is driven through `mockIPC` rather than an injected invoke, matching
 * `createDesktopAgentKitFetch.test.ts`: the command really is reached through
 * `@tauri-apps/api/core`.
 */
import { describe, it, expect, afterEach, beforeEach, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { settingsStore } from "@/stores/settingsStore";
import { sidebarTabStore } from "@/stores/sidebarTabStore";
import { ASSISTANT_BRIDGE_COMMAND } from "../client/createDesktopAgentKitFetch";
import {
  ASSISTANT_CONFIGURE_PROVIDER_COMMAND,
  providerFromSettings,
  type AssistantProviderReadiness,
} from "../client/providerConfig";
import { statusOf } from "./useAssistantProviderConfig";
import { AssistantPanel } from "./AssistantPanel";

interface ConfigureArgs {
  readonly provider: { id: string; baseUrl: string; model: string } | null;
}

let configured: ConfigureArgs[];

/**
 * Answers the configure command, recording each call. `refusal` makes it reject
 * with the backend's `{kind, message}` envelope — the shape Tauri really rejects
 * an `invoke` with (see `src/ipc/apiError.ts`).
 */
function installIpc(refusal?: string, readiness?: Partial<AssistantProviderReadiness>): void {
  mockIPC((command, args) => {
    if (command === ASSISTANT_CONFIGURE_PROVIDER_COMMAND) {
      const call = args as unknown as ConfigureArgs;
      configured.push(call);
      if (refusal !== undefined) {
        throw { kind: "invalidCommand", message: refusal };
      }
      // The command answers with the four-state ladder, not with nothing: Rust
      // accepting a URL is only the first rung of it.
      return call.provider === null
        ? {
            generation: 0,
            configured: false,
            synchronized: false,
            reachable: false,
            eligible: false,
            detail: "no provider is configured",
          }
        : {
            generation: configured.length,
            configured: true,
            synchronized: true,
            reachable: true,
            eligible: true,
            detail: null,
            ...readiness,
          };
    }
    // The indicator's own provider probe. Failing it keeps this file about the
    // configuration path: the probe result is never what these cases assert.
    if (command === ASSISTANT_BRIDGE_COMMAND) throw new Error("no host in this test");
    throw new Error(`unexpected command ${command}`);
  });
}

function showPanel(baseUrl: string, model: string): void {
  act(() => {
    settingsStore.getState().setAssistantProviderBaseUrl(baseUrl);
    settingsStore.getState().setAssistantProviderModel(model);
    settingsStore.getState().setAssistantEnabled(true);
    sidebarTabStore.getState().setActiveTab("assistant");
  });
}

beforeEach(() => {
  configured = [];
});

afterEach(() => {
  clearMocks();
  vi.restoreAllMocks();
  act(() => {
    settingsStore.getState().setAssistantEnabled(false);
    settingsStore.getState().setAssistantProviderBaseUrl("");
    settingsStore.getState().setAssistantProviderModel("");
    sidebarTabStore.getState().setActiveTab("model");
  });
  localStorage.clear();
});

describe("providerFromSettings", () => {
  it("proposes the configured provider, trimmed, with no credential field", () => {
    expect(
      providerFromSettings({
        enabled: true,
        baseUrl: "  http://127.0.0.1:11434/v1 ",
        model: " qwen3:8b ",
      }),
    ).toEqual({ id: "local", baseUrl: "http://127.0.0.1:11434/v1", model: "qwen3:8b" });
  });

  it("is null while the assistant is off or either field is empty", () => {
    const filled = { baseUrl: "http://127.0.0.1:11434/v1", model: "qwen3:8b" };
    expect(providerFromSettings({ enabled: false, ...filled })).toBeNull();
    expect(providerFromSettings({ enabled: true, baseUrl: "", model: "qwen3:8b" })).toBeNull();
    expect(providerFromSettings({ enabled: true, baseUrl: "   ", model: "qwen3:8b" })).toBeNull();
    expect(providerFromSettings({ enabled: true, ...filled, model: "" })).toBeNull();
  });
});

describe("the assistant panel's provider configuration", () => {
  it("installs the configured provider on mount", async () => {
    installIpc();
    showPanel("http://127.0.0.1:11434/v1", "qwen3:8b");
    render(<AssistantPanel />);

    await waitFor(() => expect(configured).toHaveLength(1));
    expect(configured[0]).toEqual({
      provider: { id: "local", baseUrl: "http://127.0.0.1:11434/v1", model: "qwen3:8b" },
    });
    // No credential rides along, at any nesting.
    expect(JSON.stringify(configured[0])).not.toContain("apiKey");
  });

  it("clears the registry when the assistant is turned off", async () => {
    installIpc();
    showPanel("http://127.0.0.1:11434/v1", "qwen3:8b");
    render(<AssistantPanel />);
    await waitFor(() => expect(configured).toHaveLength(1));

    // The panel renders nothing once the gate is off, but it stays MOUNTED —
    // which is what lets the clear happen at all.
    act(() => settingsStore.getState().setAssistantEnabled(false));
    await waitFor(() => expect(configured).toHaveLength(2));
    expect(configured[1]).toEqual({ provider: null });
  });

  it("clears the registry when the endpoint is emptied", async () => {
    installIpc();
    showPanel("http://127.0.0.1:11434/v1", "qwen3:8b");
    render(<AssistantPanel />);
    await waitFor(() => expect(configured).toHaveLength(1));

    act(() => settingsStore.getState().setAssistantProviderBaseUrl(""));
    await waitFor(() => expect(configured).toHaveLength(2));
    expect(configured[1]).toEqual({ provider: null });
  });

  it("reinstalls the provider when the endpoint changes", async () => {
    installIpc();
    showPanel("http://127.0.0.1:11434/v1", "qwen3:8b");
    render(<AssistantPanel />);
    await waitFor(() => expect(configured).toHaveLength(1));

    act(() => settingsStore.getState().setAssistantProviderBaseUrl("http://127.0.0.1:8080/v1"));
    await waitFor(() => expect(configured).toHaveLength(2));
    expect(configured[1]?.provider?.baseUrl).toBe("http://127.0.0.1:8080/v1");
  });

  it("shows a refused configuration instead of throwing", async () => {
    installIpc('assistant provider refused (internal): provider base host "evil.com" is refused');
    // The URL is sent verbatim: validation is Rust's, and the refusal is the
    // proof the frontend never pre-approved it.
    showPanel("http://evil.com/v1", "qwen3:8b");
    render(<AssistantPanel />);

    const message = await screen.findByTestId("assistant-provider-error");
    expect(message).toHaveTextContent("Model endpoint refused");
    expect(message).toHaveTextContent("evil.com");
    expect(screen.getByTestId("assistant-model-indicator")).toHaveAttribute(
      "data-status",
      "error",
    );
    expect(configured).toHaveLength(1);
  });
});

describe("the four states a configured provider can be in", () => {
  const rung = (over: Partial<AssistantProviderReadiness>): AssistantProviderReadiness => ({
    generation: 1,
    configured: true,
    synchronized: true,
    reachable: true,
    eligible: true,
    detail: null,
    ...over,
  });

  it("reports the highest rung actually reached, never more", () => {
    // The whole point: Rust accepting an endpoint is `configured`, and a panel
    // that painted that green would be claiming three things it never checked.
    expect(statusOf(rung({ configured: false }))).toBe("none");
    expect(statusOf(rung({ synchronized: false, reachable: false, eligible: false }))).toBe(
      "configured",
    );
    expect(statusOf(rung({ reachable: false, eligible: false }))).toBe("synchronized");
    expect(statusOf(rung({ eligible: false }))).toBe("reachable");
    expect(statusOf(rung({}))).toBe("eligible");
  });

  it("a sidecar that has not acknowledged the edit is not a working model", () => {
    const state = rung({ synchronized: false, reachable: true, eligible: true });
    expect(statusOf(state)).toBe("configured");
  });
});
