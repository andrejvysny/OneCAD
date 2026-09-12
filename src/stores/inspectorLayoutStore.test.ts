import { beforeEach, describe, expect, it } from "vitest";
import {
  INSPECTOR_CHROME_GUTTER,
  INSPECTOR_COLLAPSED_WIDTH,
  INSPECTOR_DEFAULT_WIDTH,
  INSPECTOR_MAX_WIDTH,
  INSPECTOR_MIN_WIDTH,
  clampInspectorWidth,
  inspectorChromeInset,
  inspectorInset,
  inspectorLayoutStore,
} from "./inspectorLayoutStore";

describe("inspectorLayoutStore", () => {
  beforeEach(() => inspectorLayoutStore.getState().reset());

  it("defaults to the open 320px inset", () => {
    const state = inspectorLayoutStore.getState();
    expect(state.width).toBe(INSPECTOR_DEFAULT_WIDTH);
    expect(state.open).toBe(true);
    expect(inspectorInset(state)).toBe(INSPECTOR_DEFAULT_WIDTH);
    expect(inspectorChromeInset(state)).toBe(INSPECTOR_DEFAULT_WIDTH + INSPECTOR_CHROME_GUTTER);
  });

  it("clamps width and derives the collapsed rail inset", () => {
    inspectorLayoutStore.getState().setWidth(0);
    expect(inspectorLayoutStore.getState().width).toBe(INSPECTOR_MIN_WIDTH);
    inspectorLayoutStore.getState().setWidth(999);
    expect(inspectorLayoutStore.getState().width).toBe(INSPECTOR_MAX_WIDTH);
    inspectorLayoutStore.getState().setWidth(Number.NaN);
    expect(inspectorLayoutStore.getState().width).toBe(INSPECTOR_MAX_WIDTH);
    inspectorLayoutStore.getState().setWidth(Number.POSITIVE_INFINITY);
    expect(inspectorLayoutStore.getState().width).toBe(INSPECTOR_MAX_WIDTH);
    expect(clampInspectorWidth(Number.NaN)).toBe(INSPECTOR_DEFAULT_WIDTH);
    expect(clampInspectorWidth(Number.NaN, 400)).toBe(400);

    inspectorLayoutStore.getState().setOpen(false);
    expect(inspectorInset(inspectorLayoutStore.getState())).toBe(INSPECTOR_COLLAPSED_WIDTH);
    expect(inspectorChromeInset(inspectorLayoutStore.getState())).toBe(
      INSPECTOR_COLLAPSED_WIDTH + INSPECTOR_CHROME_GUTTER,
    );
  });

  it("resets layout state without persisting it", () => {
    inspectorLayoutStore.getState().setWidth(400);
    inspectorLayoutStore.getState().setOpen(false);
    inspectorLayoutStore.getState().reset();

    expect(inspectorLayoutStore.getState()).toMatchObject({
      width: INSPECTOR_DEFAULT_WIDTH,
      open: true,
    });
  });
});
