/*
 * EditorScreen — the editor route.
 *
 * The screen's structure moved to `@/app/shell/EditorShell`, which knows only
 * layout slots; the feature panels that used to be imported here are now
 * contributions registered by `onecad.modeling` and `onecad.shell`
 * (docs/ARCHITECTURE.md §6).
 *
 * This file stays as the route's entry point because `App.tsx` code-splits on
 * this exact specifier and `StartScreen` idle-prefetches the same one.
 */
export { EditorShell as EditorScreen } from "@/app/shell/EditorShell";
