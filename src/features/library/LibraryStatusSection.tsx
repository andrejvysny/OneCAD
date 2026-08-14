/*
 * `Slots.StatusSection`'s first real producer (WP-1.6) — closes a named
 * platform debt (the slot existed with zero consumers since MODULAR-PLATFORM).
 * Deliberately minimal per the plan: a static indexed-component count, not a
 * feature. Fetches once on mount rather than sharing a store with
 * `LibraryModal` — nothing here needs to live-track the modal's search/filter
 * state, only the index size.
 */
import { useEffect, useState } from "react";
import { createClient } from "@/ipc/client";

export function LibraryStatusSection() {
  const [count, setCount] = useState<number | null>(null);

  useEffect(() => {
    createClient()
      .listLibraryComponents()
      .then((list) => setCount(list.length))
      .catch(() => setCount(null));
  }, []);

  if (count === null) return null;

  return (
    <span className="text-ink-6">
      {count} {count === 1 ? "component" : "components"}
    </span>
  );
}
