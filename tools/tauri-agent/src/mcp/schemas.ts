import { z } from "zod";

export const SpaceSchema = z.enum(["webview", "window", "global"]);
export type SpaceInput = z.infer<typeof SpaceSchema>;

export const ModeSchema = z.enum(["real_user", "webview", "diagnostic"]);

export const ButtonSchema = z.enum(["left", "right", "middle"]);

/** "Primary" is the platform accelerator key (Command on macOS); the adapter never sees it. */
export const ModSchema = z.enum(["Primary", "Command", "Control", "Option", "Shift", "Fn"]);
export type ModInput = z.infer<typeof ModSchema>;

export const ModsSchema = z.array(ModSchema);

export const PtSchema = z.object({ x: z.number(), y: z.number() });

/** Offset from the resolved element's centre, in CSS px. */
export const OffsetSchema = PtSchema;

export const SpacePointSchema = z.object({ x: z.number(), y: z.number(), space: SpaceSchema });

/**
 * An accessibility ref, minted by `native_snapshot` / `native_find`. Generation-scoped exactly
 * as a webview ref is: `@a1e7` is dead the moment a newer AX walk runs, and is then refused
 * (ELEMENT_STALE) rather than aliased onto whatever now occupies that slot.
 */
export const AxRefSchema = z
  .string()
  .regex(/^@a\d+e\d+$/, "an accessibility ref looks like @a1e7, from the latest native_snapshot or native_find");

export const TargetSchema = z
  .union([
    z.object({ ref: z.string().regex(/^@s\d+e\d+$/, "a ref looks like @s3e12, from the latest ui_snapshot") }),
    z.object({ axRef: AxRefSchema }),
    z.object({ testId: z.string().min(1) }),
    z.object({ role: z.string().min(1), name: z.string().optional() }),
    z.object({ text: z.string().min(1) }),
    z.object({ css: z.string().min(1) }),
    z.object({ point: SpacePointSchema }),
  ])
  .describe(
    "How to find what to act on. {ref} @s<gen>e<n> from ui_snapshot, {testId}, {role,name}, {text}, {css} and {point} all address the WebView. " +
      "{axRef} @a<gen>e<n> from native_snapshot / native_find addresses a NATIVE element the WebView does not own — a Save/Open panel, a sheet, " +
      "a native menu, a permission dialog, the title-bar buttons. Both are clicked the same way, with real OS events.",
  );
export type TargetInput = z.infer<typeof TargetSchema>;

/** An accessibility target: a ref the caller is holding from an AX walk. */
export function isAxTargetInput(target: TargetInput): target is { axRef: string } {
  return "axRef" in target;
}

export const RectSchema = z.object({
  x: z.number(),
  y: z.number(),
  width: z.number(),
  height: z.number(),
});

/** Every tool accepts `inline` so a caller can suppress the inline image content block. */
export const InlineSchema = z.boolean().optional();
