import type { ChipKind } from "@/stores/toolChipStore";

export interface AuthoredBodyRef {
  bodyId: string;
}

export interface AuthoredElementRef extends AuthoredBodyRef {
  kind: "face" | "edge";
  elementId?: string;
  /** Snapshot evidence only. Never presented as persistent identity. */
  topoKey?: string;
  /**
   * Measured size of the element the publisher already had in hand — a face's
   * area or an edge's arc length, in mm² / mm (`ElementInfo.magnitude`). OPTIONAL
   * and never fetched for presentation: the inspector says what it was handed
   * and stays silent otherwise (UX review 2026-09-14, N11).
   */
  area?: number;
  /** Surface normal the publisher already had (`ElementInfo.normal`), same rule. */
  normal?: readonly [number, number, number];
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
