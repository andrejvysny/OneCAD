/*
 * Id validation (ADR-0008, ADR-0006 namespacing).
 */
import { describe, it, expect } from "vitest";
import {
  moduleId,
  addonId,
  contributionId,
  isOwnedBy,
  isOwnerIdShape,
  IdError,
  type CommandId,
} from "./ids";

describe("owner ids", () => {
  it("accepts reverse-DNS ids", () => {
    expect(moduleId("onecad.modeling")).toBe("onecad.modeling");
    expect(addonId("com.example.foo")).toBe("com.example.foo");
    expect(addonId("dev.andrej.my-addon")).toBe("dev.andrej.my-addon");
  });

  it("rejects single-segment, uppercase and empty-segment ids", () => {
    for (const bad of ["modeling", "OneCAD.Modeling", "com..foo", "com.foo.", ".com.foo", ""]) {
      expect(() => moduleId(bad), bad).toThrowError(IdError);
    }
  });

  it("isOwnerIdShape agrees with the constructor", () => {
    expect(isOwnerIdShape("onecad.modeling")).toBe(true);
    expect(isOwnerIdShape("modeling")).toBe(false);
  });
});

describe("contribution ids", () => {
  const owner = moduleId("onecad.modeling");

  it("accepts an id under its owner, including camelCase segments", () => {
    expect(contributionId<CommandId>(owner, "onecad.modeling.tool.offsetFace")).toBe(
      "onecad.modeling.tool.offsetFace",
    );
  });

  it("rejects an id outside its owner's namespace", () => {
    const foo = addonId("com.example.foo");
    try {
      contributionId<CommandId>(foo, "onecad.modeling.extrude");
      expect.unreachable("an addon must not be able to claim a built-in namespace");
    } catch (e) {
      expect((e as IdError).code).toBe("foreign-namespace");
    }
  });

  it("rejects a prefix collision that is not a namespace boundary", () => {
    // "onecad.modelingx" starts with the owner string but is a different owner.
    expect(() => contributionId<CommandId>(owner, "onecad.modelingx.thing")).toThrowError(IdError);
  });

  it("rejects the bare owner id and malformed segments", () => {
    for (const bad of ["onecad.modeling", "onecad.modeling.", "onecad.modeling..a", "onecad.modeling.1a"]) {
      expect(() => contributionId<CommandId>(owner, bad), bad).toThrowError(IdError);
    }
  });

  it("isOwnedBy requires a dot boundary", () => {
    expect(isOwnedBy(owner, "onecad.modeling.a")).toBe(true);
    expect(isOwnedBy(owner, "onecad.modelingx.a")).toBe(false);
    expect(isOwnedBy(owner, "onecad.modeling")).toBe(false);
  });
});
