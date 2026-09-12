import type { ChipKind } from "@/stores/toolChipStore";

export interface AuthoredBodyRef {
  bodyId: string;
}

export interface AuthoredElementRef extends AuthoredBodyRef {
  kind: "face" | "edge";
  elementId?: string;
  /** Snapshot evidence only. Never presented as persistent identity. */
  topoKey?: string;
}

export interface AuthoredSketchRef {
  sketchId: string;
}

type PerTool<K extends ChipKind, C> = { [T in K]: C & { tool: T } }[K];

type ProfileContext = PerTool<"extrudeDepth" | "revolveAngle" | "revolveAxisPick",
  {
      kind: "profile";
      sketch: AuthoredSketchRef;
      regionIds: string[];
      hostBodies: AuthoredBodyRef[];
      direction: { kind: "normal"; vector: readonly [number, number, number] } |
        { kind: "sketchLine"; lineId: string } | null;
  }>;

type FaceContext = PerTool<"shellThickness" | "offsetFace" | "hole", {
  kind: "faces";
  affectedBodies: AuthoredBodyRef[];
  faces: AuthoredElementRef[];
  oppositeFace?: AuthoredElementRef;
}>;

type BodiesContext = PerTool<"linearPattern" | "circularPattern" | "mirror" | "transform", {
  kind: "bodies";
  bodies: AuthoredBodyRef[];
}>;

export type ActiveToolContext =
  | ProfileContext
  | { tool: "regionSelect"; kind: "regions"; sketch: AuthoredSketchRef; selectedRegionIds: string[] }
  | { tool: "filletRadius"; kind: "edgeOperation"; affectedBodies: AuthoredBodyRef[]; edges: AuthoredElementRef[]; referenceFaces: { a: AuthoredElementRef | null; b: AuthoredElementRef | null }[] }
  | FaceContext
  | BodiesContext
  | { tool: "booleanOp"; kind: "boolean"; target: AuthoredBodyRef; toolBody: AuthoredBodyRef }
  | { tool: "datumOffset"; kind: "datum"; baseId: string | null; baseLabel: string }
  | { tool: "gear"; kind: "gear"; support: AuthoredElementRef | null };

export type ContextToolKind = ActiveToolContext["tool"];
export type ActiveToolContextFor<K extends ContextToolKind> = Extract<ActiveToolContext, { tool: K }>;
export type ContextlessToolKind = Exclude<ChipKind, ContextToolKind>;
