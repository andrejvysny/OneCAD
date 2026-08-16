/*
 * The `Slots.ShellOverlay` occupant that mounts render's dialogs — the same
 * thin bridge `SaveAsComponentHost` is for the library's.
 *
 * A slot contribution renders unconditionally (the host mounts every overlay
 * and each one decides whether it has anything to show, see `Slots.ShellOverlay`'s
 * contract), so all three sit here and each returns `null` until
 * `renderDialogStore` says otherwise.
 */
import { AssignMaterialDialog } from "./AssignMaterialDialog";
import { MaterialEditorSheet } from "./MaterialEditorSheet";
import { OverrideKeepDialog } from "./OverrideKeepDialog";

export function RenderDialogHost() {
  return (
    <>
      <OverrideKeepDialog />
      <AssignMaterialDialog />
      <MaterialEditorSheet />
    </>
  );
}
