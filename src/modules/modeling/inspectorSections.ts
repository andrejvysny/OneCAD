/*
 * Modeling's inspector sections, as contributions.
 *
 * `canRender` is a pure predicate over the platform context — it may not reach
 * into a store, because nothing re-runs it when a store changes; the HOST owns
 * the subscriptions (see `InspectorSectionHost`). Anything a predicate cannot
 * decide from selection + scope is decided by the section itself returning
 * `null`, which is honest: a data-driven section (Dimensions, Depends on) is
 * genuinely absent until its data arrives.
 *
 * Priorities come from `inspectorSectionIds.ts` and reproduce the frozen order in
 * `src/test/contracts/inspectorContract.ts`. Pairs that can never render together
 * share one.
 */
import type { InspectorContext, ModuleScope } from "@/platform";
import { ModelingScopes } from "./manifest";
import { ModelingInspectorSections as Ids, ModelingInspectorPriorities as P } from "./inspectorSectionIds";
import {
  BodyAppearanceSection,
  FaceAppearanceSection,
  ConstraintsHintSection,
  ConstraintsListSection,
  FeatureDependenciesSection,
  HistoryFeatureSection,
  HistorySelectionSection,
  SketchDimensionsSection,
} from "@/features/inspector/sections";
import { VariablesSection } from "@/features/inspector/VariablesSection";

const primary = (ctx: InspectorContext) => ctx.selection[0];
const inScope = (ctx: InspectorContext, token: string) => ctx.scopes.includes(token);
const modelling = (ctx: InspectorContext) => inScope(ctx, ModelingScopes.Model);
const sketching = (ctx: InspectorContext) => inScope(ctx, ModelingScopes.Sketch);
const isKind = (ctx: InspectorContext, ...kinds: string[]) => {
  const sel = primary(ctx);
  return sel !== undefined && kinds.includes(sel.typeId);
};

export function contributeInspectorSections(scope: ModuleScope): void {
  scope.registerInspectorSection({
    id: Ids.AppearanceBody,
    title: "Appearance",
    priority: P.Appearance,
    canRender: (ctx) => modelling(ctx) && isKind(ctx, "body"),
    component: BodyAppearanceSection,
  });
  scope.registerInspectorSection({
    id: Ids.AppearanceFace,
    title: "Appearance",
    priority: P.Appearance,
    // A face with no promoted ElementId has nothing to key a color on — the
    // section is absent rather than inert.
    canRender: (ctx) => modelling(ctx) && isKind(ctx, "face") && !!primary(ctx)?.subElement,
    component: FaceAppearanceSection,
  });

  scope.registerInspectorSection({
    id: Ids.Dimensions,
    title: "Dimensions",
    priority: P.Dimensions,
    canRender: (ctx) => modelling(ctx) && isKind(ctx, "feature"),
    component: SketchDimensionsSection,
  });

  scope.registerInspectorSection({
    id: Ids.HistorySelection,
    title: "History",
    priority: P.History,
    canRender: (ctx) => modelling(ctx) && primary(ctx) !== undefined && !isKind(ctx, "feature"),
    component: HistorySelectionSection,
  });
  scope.registerInspectorSection({
    id: Ids.HistoryFeature,
    title: "History",
    priority: P.History,
    canRender: (ctx) => modelling(ctx) && isKind(ctx, "feature"),
    component: HistoryFeatureSection,
  });

  scope.registerInspectorSection({
    id: Ids.ConstraintsHint,
    title: "Constraints",
    priority: P.Constraints,
    canRender: (ctx) => modelling(ctx) && isKind(ctx, "sketch", "sketchRegion"),
    component: ConstraintsHintSection,
  });
  scope.registerInspectorSection({
    id: Ids.ConstraintsList,
    title: "Constraints",
    priority: P.Constraints,
    canRender: (ctx) => sketching(ctx),
    component: ConstraintsListSection,
  });

  scope.registerInspectorSection({
    id: Ids.Dependencies,
    title: "Depends on",
    priority: P.Dependencies,
    canRender: (ctx) => modelling(ctx) && isKind(ctx, "feature"),
    component: FeatureDependenciesSection,
  });

  // WP-VE.2. The ONLY document-level section: the variable table is not about
  // the selection, so it renders throughout Model mode and sits last, below
  // everything that IS about what the user picked. Sketch mode is deliberately
  // excluded — a sketch dimension is a solver value with no `Scalar` behind it,
  // so nothing in that mode can be bound to a variable yet.
  scope.registerInspectorSection({
    id: Ids.Variables,
    title: "Variables",
    priority: P.Variables,
    canRender: modelling,
    component: VariablesSection,
  });
}
