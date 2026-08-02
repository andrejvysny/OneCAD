/*
 * Title-bar appearance toggle: a one-click shortcut that cycles the PREFERENCE
 * in registry order. The full three-way picker lives in the display popover;
 * this only has to stay in step with it and never become ambiguous about what
 * the current value is.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TitleBar } from "./TitleBar";
import { settingsStore } from "@/stores/settingsStore";
import { THEME_ORDER } from "@/theme/themes";
import { resetStores } from "@/test/resetStores";

describe("TitleBar appearance toggle", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStores();
  });

  it("names the CURRENT preference, so the changing icon is never ambiguous", () => {
    settingsStore.setState({ theme: "dark" });
    render(<TitleBar />);
    expect(screen.getByRole("button", { name: "Appearance: Dark" })).toBeInTheDocument();
  });

  it("cycles Light → Dark → System → Light, in registry order", async () => {
    const user = userEvent.setup();
    settingsStore.setState({ theme: "light" });
    const { rerender } = render(<TitleBar />);

    const seen: string[] = [];
    for (let i = 0; i < THEME_ORDER.length; i++) {
      await user.click(screen.getByRole("button", { name: /^Appearance:/ }));
      rerender(<TitleBar />);
      seen.push(settingsStore.getState().theme);
    }

    expect(seen).toEqual(["dark", "system", "light"]);
  });

  it("writes the preference through to localStorage", async () => {
    const user = userEvent.setup();
    settingsStore.setState({ theme: "light" });
    render(<TitleBar />);

    await user.click(screen.getByRole("button", { name: "Appearance: Light" }));

    const raw = localStorage.getItem("onecad.settings");
    expect(JSON.parse(raw!).state.theme).toBe("dark");
  });

  it("covers every registered theme — a new one cannot be silently skipped", async () => {
    const user = userEvent.setup();
    settingsStore.setState({ theme: THEME_ORDER[0] });
    const { rerender } = render(<TitleBar />);

    const visited = new Set<string>([THEME_ORDER[0]]);
    for (let i = 0; i < THEME_ORDER.length; i++) {
      await user.click(screen.getByRole("button", { name: /^Appearance:/ }));
      rerender(<TitleBar />);
      visited.add(settingsStore.getState().theme);
    }

    expect([...visited].sort()).toEqual([...THEME_ORDER].sort());
  });

  it("does not carry the Tauri drag region — that would swallow the click", () => {
    render(<TitleBar />);
    const btn = screen.getByRole("button", { name: /^Appearance:/ });
    expect(btn).not.toHaveAttribute("data-tauri-drag-region");
  });
});
