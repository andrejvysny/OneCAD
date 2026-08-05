import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import { Icon } from "@/icons/Icon";
import { ICONS, type IconName } from "@/icons/paths";

function pathsIn(container: HTMLElement) {
  return [...container.querySelectorAll("path")];
}

describe("Icon", () => {
  const names = Object.keys(ICONS) as IconName[];

  it.each(names)("renders every element of '%s' in order", (name) => {
    const { container } = render(<Icon name={name} />);
    const rendered = pathsIn(container).map((p) => p.getAttribute("d"));
    expect(rendered).toEqual(ICONS[name].map((el) => el.d));
    expect(rendered.length).toBeGreaterThan(0);
  });

  it("applies prototype defaults (size 15, strokeWidth 1.7, viewBox)", () => {
    const { container } = render(<Icon name="select" />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("width")).toBe("15");
    expect(svg?.getAttribute("height")).toBe("15");
    expect(svg?.getAttribute("stroke-width")).toBe("1.7");
    expect(svg?.getAttribute("viewBox")).toBe("0 0 24 24");
    expect(svg?.getAttribute("fill")).toBe("none");
    expect(svg?.getAttribute("stroke")).toBe("currentColor");
  });

  it("honours size and strokeWidth overrides", () => {
    const { container } = render(<Icon name="plus" size={18} strokeWidth={2} />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("width")).toBe("18");
    expect(svg?.getAttribute("stroke-width")).toBe("2");
  });

  /*
   * The two-tone contract. `extrude` is 3 primary structure paths followed by
   * 2 accent paths (the pull arrow) — the canonical shape of the family.
   */
  it("paints the primary tone with currentColor and the accent with the token", () => {
    const { container } = render(<Icon name="extrude" />);
    const strokes = pathsIn(container).map((p) => p.getAttribute("stroke"));
    expect(strokes).toEqual([
      "currentColor",
      "currentColor",
      "currentColor",
      "var(--color-icon-accent)",
      "var(--color-icon-accent)",
    ]);
  });

  it("emits the accent as a var() so a container can collapse it to mono", () => {
    // Not cosmetic: ToolButton's active state redefines --color-icon-accent to
    // currentColor. A resolved color here would make that impossible.
    const { container } = render(<Icon name="extrude" />);
    const rendered = pathsIn(container);
    const accent = rendered[rendered.length - 1];
    expect(accent?.getAttribute("stroke")).toContain("var(");
  });

  it("fills a filled element and does not stroke it", () => {
    // `line` = one stroked segment + two accent-filled endpoint dots.
    const { container } = render(<Icon name="line" />);
    const [segment, dot] = pathsIn(container);
    expect(segment.getAttribute("fill")).toBe("none");
    expect(segment.getAttribute("stroke")).toBe("currentColor");
    expect(dot.getAttribute("fill")).toBe("var(--color-icon-accent)");
    expect(dot.getAttribute("stroke")).toBe("none");
    expect(dot.getAttribute("stroke-width")).toBeNull();
  });

  it("scales per-element stroke widths off the caller's strokeWidth", () => {
    // `constraintHorizontal`'s accent bar is authored at 1.6 against a base of
    // 2, i.e. sw 0.8 — so it must track the caller, not sit at a fixed 1.6.
    const def = ICONS.constraintHorizontal;
    const accentIndex = def.findIndex((el) => el.tone === "accent");
    expect(def[accentIndex]?.sw).toBe(0.8);

    const { container } = render(
      <Icon name="constraintHorizontal" strokeWidth={2.5} />,
    );
    expect(
      pathsIn(container)[accentIndex]?.getAttribute("stroke-width"),
    ).toBe("2");
  });

  it("passes stroke-dasharray and transform through", () => {
    const { container } = render(<Icon name="mirror" />);
    const dashed = pathsIn(container).find((p) =>
      p.hasAttribute("stroke-dasharray"),
    );
    expect(dashed?.getAttribute("stroke-dasharray")).toBe("2 2.4");

    // ic_orbit is the only master carrying an element-local transform.
    const { container: orbit } = render(<Icon name="orbit" />);
    const rotated = pathsIn(orbit).find((p) => p.hasAttribute("transform"));
    expect(rotated?.getAttribute("transform")).toBe("rotate(-28 12 12)");
  });

  it("keeps aliases pointing at the same definition as their master", () => {
    expect(ICONS.rect).toBe(ICONS.rectangle);
    expect(ICONS.x).toBe(ICONS.close);
    expect(ICONS.eye).toBe(ICONS.eyeOn);
    expect(ICONS.linearPattern).toBe(ICONS.patternLinear);
  });
});
