/**
 * Placeholder grid shown while recents load (recentsStatus === "loading"),
 * mirroring RecentGrid's layout + ProjectCard's card anatomy so the swap to
 * real cards doesn't shift the page. Deliberately exposes NO project-name text
 * and NO `title` attribute — StartScreen.test.tsx queries cards by both.
 */
export function RecentGridSkeleton() {
  return (
    <div
      className="grid animate-pulse grid-cols-3 gap-3"
      aria-hidden="true"
      role="presentation"
    >
      {Array.from({ length: 6 }, (_, i) => (
        <div
          key={i}
          className="overflow-hidden rounded-lg border border-border bg-surface"
        >
          <div className="h-24 border-b border-border-subtle bg-well" />
          <div className="px-[11px] pb-[10px] pt-[9px]">
            <div className="h-[13px] w-2/5 rounded bg-well" />
            <div className="mt-[6px] h-[11px] w-4/5 rounded bg-well" />
          </div>
        </div>
      ))}
    </div>
  );
}
