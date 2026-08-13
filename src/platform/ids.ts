/*
 * Platform identity (ADR-0008).
 *
 * Every registered thing carries a namespaced id and a declared owner. Ids are
 * branded strings: structurally they are strings on the wire and in storage, but
 * a `CommandId` cannot be passed where a `ToolId` is wanted, and a bare string
 * cannot be passed at all without going through a constructor that validates it.
 *
 * Two id shapes:
 *   OWNER id       reverse-DNS, lowercase segments — `onecad.modeling`,
 *                  `com.example.foo`. Stable forever; never a display name.
 *   CONTRIBUTION   the owner's id, then one or more segments which may be
 *                  camelCase — `onecad.modeling.tool.offsetFace`.
 *
 * The namespace rule is enforced, not documented: an addon may not register
 * `onecad.*`, even though addons are trusted in v1 (ADR-0006).
 */

declare const brand: unique symbol;

/** A string carrying a compile-time-only tag. */
export type Branded<Tag extends string> = string & { readonly [brand]: Tag };

export type ModuleId = Branded<"ModuleId">;
export type AddonId = Branded<"AddonId">;
/** Whoever a registration belongs to. Modules and addons register alike. */
export type OwnerId = ModuleId | AddonId;

export type WorkspaceId = Branded<"WorkspaceId">;
export type CommandId = Branded<"CommandId">;
export type ToolId = Branded<"ToolId">;
export type PanelId = Branded<"PanelId">;
export type ServiceId = Branded<"ServiceId">;
export type EventId = Branded<"EventId">;
export type TypeId = Branded<"TypeId">;
export type EntityId = Branded<"EntityId">;
export type InspectorContributionId = Branded<"InspectorContributionId">;
export type ViewportContributionId = Branded<"ViewportContributionId">;
export type TreeProviderId = Branded<"TreeProviderId">;
export type MenuContributionId = Branded<"MenuContributionId">;

/** Thrown for every malformed or unauthorized id. Carries a stable `code`. */
export class IdError extends Error {
  constructor(
    readonly code: "invalid-owner-id" | "invalid-id" | "foreign-namespace",
    message: string,
  ) {
    super(message);
    this.name = "IdError";
  }
}

const OWNER_SEGMENT = /^[a-z][a-z0-9-]*$/;
const CONTRIBUTION_SEGMENT = /^[a-zA-Z][a-zA-Z0-9-]*$/;

/** Reverse-DNS, at least two lowercase segments. */
export function isOwnerIdShape(value: string): boolean {
  const parts = value.split(".");
  return parts.length >= 2 && parts.every((p) => OWNER_SEGMENT.test(p));
}

function assertOwnerShape(value: string): void {
  if (!isOwnerIdShape(value)) {
    throw new IdError(
      "invalid-owner-id",
      `"${value}" is not a reverse-DNS owner id (expected e.g. "com.example.foo")`,
    );
  }
}

export function moduleId(value: string): ModuleId {
  assertOwnerShape(value);
  return value as ModuleId;
}

export function addonId(value: string): AddonId {
  assertOwnerShape(value);
  return value as AddonId;
}

/**
 * Validates a contribution id and brands it.
 *
 * `owner` is required: an id that does not sit under its owner's namespace is
 * rejected here rather than at registration time, so the failure names the
 * offending id at the point it was constructed.
 */
export function contributionId<T extends Branded<string>>(owner: OwnerId, value: string): T {
  if (value !== owner && !value.startsWith(`${owner}.`)) {
    throw new IdError(
      "foreign-namespace",
      `"${value}" is outside its owner's namespace ("${owner}.*")`,
    );
  }
  const suffix = value.slice(owner.length + 1);
  const parts = suffix.length === 0 ? [] : suffix.split(".");
  if (parts.length === 0 || !parts.every((p) => CONTRIBUTION_SEGMENT.test(p))) {
    throw new IdError("invalid-id", `"${value}" is not a valid contribution id`);
  }
  return value as T;
}

/**
 * Whether `id` belongs to `owner`'s namespace. Used by the registries, which
 * must reject a foreign id even when the caller built it by hand.
 */
export function isOwnedBy(owner: OwnerId, id: string): boolean {
  return id.startsWith(`${owner}.`);
}

/** Unsafe brand for ids that arrive from outside (manifests, wire, storage). */
export function unsafeId<T extends Branded<string>>(value: string): T {
  return value as T;
}
