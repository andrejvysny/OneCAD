/*
 * The `Slots.ShellOverlay` occupant that mounts the authoring dialog (WP-B2).
 *
 * A slot contribution renders unconditionally (the host mounts every overlay
 * and each one decides whether it has anything to show — see `Slots.ShellOverlay`'s
 * own contract), so this is the thin bridge between the module-level open-state
 * and the dialog's props.
 */
import { SaveAsComponentDialog } from "./SaveAsComponentDialog";
import {
  closeSaveAsComponent,
  useSaveAsComponentTarget,
} from "@/modules/library/authoringController";

export function SaveAsComponentHost() {
  const target = useSaveAsComponentTarget();
  return (
    <SaveAsComponentDialog
      bodyId={target?.bodyId ?? null}
      bodyName={target?.bodyName}
      onClose={closeSaveAsComponent}
    />
  );
}
