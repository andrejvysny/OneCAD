/*
 * "Unavailable data" — the Document Explorer's section for state whose owner is
 * not installed (prototype 2a).
 *
 * This is the visible half of ADR-0005. A module's slice round-trips verbatim
 * whether or not that module is present, and the promise is only worth anything
 * if the user can SEE that their data is still in the file. A banner they
 * dismissed once does not do that; a row in the tree does, for as long as the
 * situation lasts.
 *
 * It is a `TreeProvider` rather than a special case inside the panel for the
 * ordinary reason: the panel renders providers, and the moment it learned about
 * "missing modules" it would be back to knowing what its rows mean.
 *
 * The rows are not selectable in the platform sense — there is nothing to
 * inspect — so `select()` opens the details dialog instead. Clicking a row that
 * says something is wrong should explain it.
 */
import type { Disposable, TreeSection } from "@/platform";
import { extensionsStore } from "@/stores/extensionsStore";
import { ShellTreeProvider } from "./ids";

export function missingModulesSections(): readonly TreeSection[] {
  const { missing, detailsFor, showDetails } = extensionsStore.getState();
  // No section at all when nothing is missing: an always-present empty
  // "Unavailable data" header would be a permanent accusation.
  if (missing.length === 0) return [];

  return [
    {
      id: "onecad.shell.tree.unavailable",
      title: "Unavailable data",
      nodes: missing.map((m) => ({
        id: m.moduleId,
        label: m.moduleId,
        icon: "alert",
        kind: "missing-module",
        problem: true,
        meta: "not installed",
        selected: detailsFor === m.moduleId,
        select: () => showDetails(m.moduleId),
      })),
    },
  ];
}

function subscribeToProjection(onChange: () => void): Disposable {
  const unsubscribe = extensionsStore.subscribe(onChange);
  return { dispose: unsubscribe };
}

export const missingModulesTreeProvider = {
  id: ShellTreeProvider,
  // AFTER modeling's sections (default priority 100): a notice belongs at the
  // bottom of the explorer, under the document's own content.
  priority: 900,
  sections: missingModulesSections,
  subscribe: subscribeToProjection,
};
