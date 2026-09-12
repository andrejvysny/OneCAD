import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { viewportWorkAreaStore } from "@/stores/viewportWorkAreaStore";
import { resetStores } from "@/test/resetStores";
import { CornerCluster } from "./CornerCluster";

describe("CornerCluster work-area placement", () => {
  beforeEach(() => resetStores());

  it("tracks the measured right edge and toolbar height", () => {
    render(<CornerCluster />);
    const cluster = screen.getByTestId("corner-cluster");
    act(() => {
      const store = viewportWorkAreaStore.getState();
      store.setViewport({ x: 0, y: 0, width: 1024, height: 700 });
      store.publishRegion("right", { x: 704, y: 0, width: 320, height: 666 });
      store.publishRegion("toolbar", { x: 250, y: 12, width: 440, height: 76 });
    });
    expect(cluster).toHaveStyle({ right: "332px", top: "96px" });
  });
});
