/*
 * OneCAD Platform — public surface.
 *
 * Modules and the shell import from here. Nothing in `src/platform/**` may import
 * `@/features`, `@/tools`, `@/modules` or a modeling store; that rule is enforced
 * by test (see architecture.test.ts) rather than left to review.
 */
export * from "./ids";
export * from "./registry";
export * from "./slots";
export * from "./contributions";
export * from "./events";
export * from "./services";
export * from "./platform";
export * from "./toolHost";
export * from "./shortcuts";
export * from "./documentState";
export {
  PlatformProvider,
  usePlatform,
  useOptionalPlatform,
  useRegistryEntries,
  useActiveToolId,
} from "./react/PlatformProvider";
export { SlotHost, useSlotContributions } from "./react/SlotHost";
export { ContributionBoundary } from "./react/ContributionBoundary";
