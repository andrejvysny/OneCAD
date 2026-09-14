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

export const TargetSchema = z.union([
  z.object({ ref: z.string().regex(/^@s\d+e\d+$/, "a ref looks like @s3e12, from the latest ui_snapshot") }),
  z.object({ testId: z.string().min(1) }),
  z.object({ role: z.string().min(1), name: z.string().optional() }),
  z.object({ text: z.string().min(1) }),
  z.object({ css: z.string().min(1) }),
  z.object({ point: SpacePointSchema }),
]);
export type TargetInput = z.infer<typeof TargetSchema>;

export const RectSchema = z.object({
  x: z.number(),
  y: z.number(),
  width: z.number(),
  height: z.number(),
});

/** Every tool accepts `inline` so a caller can suppress the inline image content block. */
export const InlineSchema = z.boolean().optional();
