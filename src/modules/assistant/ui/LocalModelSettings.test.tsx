/*
 * The form is the ONLY way to configure a model without hand-editing
 * localStorage, so these pin the three states that decide whether a user can
 * get unstuck: unconfigured (must open), refused (must stay open), and accepted
 * (may collapse, and must be reopenable).
 */
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { settingsStore } from "@/stores/settingsStore";
import { LocalModelSettings } from "./LocalModelSettings";
import type { AssistantProviderConfigState } from "./useAssistantProviderConfig";

const ACCEPTED: AssistantProviderConfigState = {
  status: "eligible",
  error: null,
  readiness: {
    generation: 1,
    configured: true,
    synchronized: true,
    reachable: true,
    eligible: true,
    detail: null,
  },
};
const REFUSED: AssistantProviderConfigState = {
  status: "error",
  error: "not a loopback address",
  readiness: null,
};

function seed(baseUrl: string, model: string): void {
  settingsStore.getState().setAssistantProviderBaseUrl(baseUrl);
  settingsStore.getState().setAssistantProviderModel(model);
}

describe("LocalModelSettings", () => {
  beforeEach(() => seed("", ""));
  afterEach(() => seed("", ""));

  it("opens by itself while either field is empty", () => {
    render(<LocalModelSettings config={ACCEPTED} />);
    expect(screen.getByTestId("assistant-model-settings")).toBeTruthy();
  });

  it("stays open when the endpoint was refused, even though both fields are filled", () => {
    // The collapse rule is "configured AND accepted". A refused endpoint that
    // hid its own form would leave the user with an error and nothing to edit.
    seed("http://10.0.0.5:11434/v1", "some-model");
    render(<LocalModelSettings config={REFUSED} />);
    expect(screen.getByTestId("assistant-model-settings")).toBeTruthy();
    expect(
      screen.getByTestId("assistant-provider-base-url").getAttribute("aria-invalid"),
    ).toBe("true");
  });

  it("starts collapsed once a configuration is accepted, and reopens on demand", () => {
    // An already-working assistant should not greet the user with a settings
    // form every time they open the tab.
    seed("http://127.0.0.1:11434/v1", "qwen2.5");
    render(<LocalModelSettings config={ACCEPTED} />);
    expect(screen.queryByTestId("assistant-model-settings")).toBeNull();

    fireEvent.click(screen.getByTestId("assistant-model-settings-open"));
    expect(screen.getByTestId("assistant-model-settings")).toBeTruthy();

    fireEvent.click(screen.getByTestId("assistant-model-settings-close"));
    expect(screen.queryByTestId("assistant-model-settings")).toBeNull();
  });

  it("writes both fields straight through to settings", () => {
    render(<LocalModelSettings config={ACCEPTED} />);
    fireEvent.change(screen.getByTestId("assistant-provider-base-url"), {
      target: { value: "http://127.0.0.1:1234/v1" },
    });
    fireEvent.change(screen.getByTestId("assistant-provider-model"), {
      target: { value: "llama3.1" },
    });
    expect(settingsStore.getState().assistantProviderBaseUrl).toBe("http://127.0.0.1:1234/v1");
    expect(settingsStore.getState().assistantProviderModel).toBe("llama3.1");
  });

  it("suggests an endpoint without ever defaulting to one", () => {
    // A placeholder, not a value: an unconfigured assistant must not silently
    // point at a port the user never named.
    render(<LocalModelSettings config={ACCEPTED} />);
    const input = screen.getByTestId("assistant-provider-base-url") as HTMLInputElement;
    expect(input.placeholder).toContain("127.0.0.1");
    expect(input.value).toBe("");
    expect(settingsStore.getState().assistantProviderBaseUrl).toBe("");
  });
});
