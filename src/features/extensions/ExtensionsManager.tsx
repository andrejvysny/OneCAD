import { useState } from "react";
import { Icon } from "@/icons/Icon";
import { MonoValue } from "@/ui/MonoValue";
import { TextInput } from "@/ui/TextInput";
import { cn } from "@/ui/cn";
import { usePlatform, type ModuleId, type ModuleState } from "@/platform";
import { useExtensionsStore, type ExtensionsTab } from "@/stores/extensionsStore";

/**
 * Extensions manager (prototype 2b).
 *
 * WHAT IS BOUND AND WHAT IS NOT — the whole design of this screen.
 *
 * "Installed" is REAL: it lists `platform.moduleIds()` with their live
 * lifecycle state and the contributions each one actually registered, counted
 * off the registries. That is a genuinely useful screen today — it is the only
 * place you can see what a build is made of.
 *
 * "Browse" and "Updates" have NO backend: there is no addon loader, no
 * manifest format on disk and no registry to talk to (see the closing note in
 * `platform/extension.ts`). They render an explicit empty state saying so.
 * A mock catalog was the obvious alternative and is the wrong one: fake install
 * buttons that do nothing teach users the feature is broken, and fake listings
 * outlive the mock.
 *
 * Enable/disable and uninstall are likewise absent rather than inert. A
 * built-in module has no meaningful "disabled" state — the platform can unload
 * an owner, but a half-unloaded modeling module is not a state the app is
 * designed to run in, and offering the switch would imply it is.
 */
export function ExtensionsManager() {
  const open = useExtensionsStore((s) => s.managerOpen);
  const tab = useExtensionsStore((s) => s.tab);
  const setTab = useExtensionsStore((s) => s.setTab);
  const close = useExtensionsStore((s) => s.closeManager);
  const missing = useExtensionsStore((s) => s.missing);
  const [query, setQuery] = useState("");

  if (!open) return null;

  return (
    <div
      className="absolute inset-0 z-[85] flex items-start justify-center bg-scrim pt-[64px]"
      onClick={close}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Extensions"
        data-testid="extensions-manager"
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[calc(100%-128px)] w-[860px] flex-col overflow-hidden rounded-lg border border-border bg-surface shadow-popover"
      >
        <div className="flex flex-none items-center gap-3 px-6 pb-4 pt-5">
          <span className="text-[16px] font-bold text-ink">Extensions</span>
          <div className="flex rounded-md bg-canvas p-0.5">
            <TabButton id="installed" tab={tab} onPick={setTab}>
              Installed
            </TabButton>
            <TabButton id="browse" tab={tab} onPick={setTab}>
              Browse
            </TabButton>
            <TabButton id="updates" tab={tab} onPick={setTab}>
              Updates
            </TabButton>
          </div>
          <span className="flex-1" />
          <TextInput
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search extensions…"
            leadingIcon="search"
            aria-label="Search extensions"
            wrapperClassName="w-[220px]"
          />
          <button
            type="button"
            aria-label="Close extensions"
            onClick={close}
            className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-sm border-none bg-transparent text-ink-4 hover:bg-hover"
          >
            <Icon name="close" size={15} strokeWidth={1.8} />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-auto px-6 pb-6">
          {tab === "installed" && <InstalledTab query={query} />}
          {tab === "browse" && <NoRegistry what="browse" />}
          {tab === "updates" && <NoRegistry what="updates" />}
          {tab === "installed" && missing.length > 0 && (
            <>
              <div className="pb-2 pt-5 text-[11px] font-medium uppercase tracking-[0.07em] text-ink-5">
                Referenced by this document, not installed
              </div>
              <div className="grid grid-cols-2 gap-3.5">
                {missing.map((m) => (
                  <div
                    key={m.moduleId}
                    data-testid={`extension-missing-${m.moduleId}`}
                    className="rounded-lg border border-warn-border bg-warn-surface p-4"
                  >
                    <div className="flex items-center gap-2">
                      <Icon name="alert" size={15} strokeWidth={1.8} className="text-warn" />
                      <MonoValue className="text-[12px] text-warn-strong">{m.moduleId}</MonoValue>
                    </div>
                    <div className="mt-1.5 text-[12px] leading-[1.5] text-warn-strong">
                      Its document state (schema v{m.schemaVersion}) is preserved and saved back
                      unchanged.
                    </div>
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function TabButton({
  id,
  tab,
  onPick,
  children,
}: {
  id: ExtensionsTab;
  tab: ExtensionsTab;
  onPick: (t: ExtensionsTab) => void;
  children: string;
}) {
  const active = id === tab;
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      data-testid={`extensions-tab-${id}`}
      onClick={() => onPick(id)}
      className={cn(
        "h-[26px] cursor-pointer rounded-[5px] border-none px-3.5 font-ui text-[12.5px] font-semibold",
        active ? "bg-surface text-ink shadow-ctrl" : "bg-transparent text-ink-4",
      )}
    >
      {children}
    </button>
  );
}

/** One installed module, described by what it actually contributed. */
function InstalledTab({ query }: { query: string }) {
  const platform = usePlatform();
  const needle = query.trim().toLowerCase();
  const ids = platform.moduleIds().filter((id) => id.toLowerCase().includes(needle));

  if (ids.length === 0) {
    return <EmptyState title="No matching extensions" body="Nothing installed matches that." />;
  }

  return (
    <div className="grid grid-cols-2 gap-3.5">
      {ids.map((id) => (
        <ModuleCard key={id} id={id} state={platform.moduleState(id)} />
      ))}
    </div>
  );
}

function ModuleCard({ id, state }: { id: ModuleId; state: ModuleState | undefined }) {
  const platform = usePlatform();
  const owned = (n: number, label: string) => (n === 0 ? null : `${n} ${label}${n === 1 ? "" : "s"}`);
  const adds = [
    owned(platform.workspaces.registrations().filter((r) => r.owner === id).length, "workspace"),
    owned(platform.tools.registrations().filter((r) => r.owner === id).length, "tool"),
    owned(platform.commands.registrations().filter((r) => r.owner === id).length, "command"),
    owned(platform.panels.registrations().filter((r) => r.owner === id).length, "panel"),
    owned(
      platform.inspector.registrations().filter((r) => r.owner === id).length,
      "inspector section",
    ),
    owned(
      platform.viewport.registrations().filter((r) => r.owner === id).length,
      "viewport layer",
    ),
  ].filter((s): s is string => s !== null);

  const failed = state === "failed";
  const ready = state === "ready";

  return (
    <div
      data-testid={`extension-card-${id}`}
      className="rounded-lg border border-border p-[16px_18px]"
    >
      <div className="flex gap-3">
        <div className="flex h-10 w-10 flex-none items-center justify-center rounded-md bg-sel-bg">
          <Icon name="puzzle" size={20} strokeWidth={1.6} className="text-accent" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13.5px] font-semibold text-ink">{id}</div>
          <div className="text-[11.5px] text-ink-5">Built in</div>
        </div>
        <span
          className={cn(
            "h-fit rounded-[5px] px-[7px] py-[3px] text-[11px] font-medium",
            failed
              ? "bg-warn-surface text-warn"
              : ready
                ? "bg-well text-ink-4"
                : "bg-chip text-ink-5",
          )}
        >
          {state ?? "unknown"}
        </span>
      </div>
      <div className="mt-2.5 text-[11.5px] text-ink-5">
        {adds.length > 0 ? `Adds: ${adds.join(" · ")}` : "Registers no UI contributions."}
      </div>
    </div>
  );
}

/**
 * The honest empty state for the two tabs that would need a registry.
 * Naming the missing piece beats a spinner that never resolves.
 */
function NoRegistry({ what }: { what: "browse" | "updates" }) {
  return (
    <EmptyState
      title={what === "browse" ? "No extension registry configured" : "No updates to check"}
      body={
        what === "browse"
          ? "OneCAD has no place to download extensions from yet. When a registry is configured, published extensions appear here."
          : "Update checking needs an extension registry, which is not configured yet. Built-in modules ship with the application and update with it."
      }
    />
  );
}

function EmptyState({ title, body }: { title: string; body: string }) {
  return (
    <div className="px-2 py-12 text-center" data-testid="extensions-empty">
      <div className="mx-auto mb-2.5 flex h-10 w-10 items-center justify-center rounded-full bg-well">
        <Icon name="puzzle" size={18} strokeWidth={1.7} className="text-ink-6" />
      </div>
      <div className="text-[13px] font-semibold text-ink-3">{title}</div>
      <div className="mx-auto mt-1 max-w-[420px] text-[12px] leading-[1.5] text-ink-6">{body}</div>
    </div>
  );
}
