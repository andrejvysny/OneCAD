/*
 * The Experimental → WebGPU control (VP-HARDENING VP01, spec §5).
 *
 * WebGL2 is the supported production backend, so the preference must be
 * PRESENTED AS UNAVAILABLE rather than removed: the stored value still exists
 * (`settingsStore` version 11 is unchanged), and a saved `true` falls back to
 * WebGL with a once-per-session diagnostic instead of an empty viewport. A
 * toggle a user can still flip would promise a backend that is not there.
 */
import { describe, it, expect, afterEach } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { SettingsModal } from "./SettingsModal";
import { settingsStore } from "@/stores/settingsStore";

afterEach(() => {
  cleanup();
  settingsStore.getState().setExperimentalWebGpu(false);
});

describe("SettingsModal — experimental WebGPU", () => {
  it("renders the toggle DISABLED with the reason underneath", () => {
    render(<SettingsModal open onClose={() => {}} />);

    const toggle = screen.getByRole("switch", { name: "Enable WebGPU renderer" });
    expect(toggle).toBeDisabled();
    expect(
      screen.getByText(
        "Unavailable — WebGPU has not passed the viewport capability suite; WebGL is used.",
      ),
    ).toBeInTheDocument();
  });

  it("still reflects a SAVED preference — the value is kept, not erased", () => {
    settingsStore.getState().setExperimentalWebGpu(true);
    render(<SettingsModal open onClose={() => {}} />);

    const toggle = screen.getByRole("switch", { name: "Enable WebGPU renderer" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(toggle).toBeDisabled(); // shown, honest, and not flippable
    expect(settingsStore.getState().experimentalWebGpu).toBe(true);
  });
});
