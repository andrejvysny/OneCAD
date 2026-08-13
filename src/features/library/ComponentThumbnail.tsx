/*
 * ComponentThumbnail — a library card's picture (WP-B6).
 *
 * Renders the component's real geometry, rasterized once by the shared
 * offscreen renderer (`componentPreviewScene`) and cached across mounts. Falls
 * back to the generic icon whenever there is no picture to show — no worker, no
 * WebGL, a component the kernel cannot build — which is exactly what every card
 * showed before this existed, so the failure mode is the old behaviour rather
 * than a hole in the grid.
 */
import { useEffect, useState } from "react";
import { Icon } from "@/icons/Icon";
import { componentThumbnail } from "./componentPreviewScene";

export interface ComponentThumbnailProps {
  componentId: string;
  componentVersion: string;
  /** Rasterization size in device-independent pixels. */
  size?: number;
  className?: string;
}

export function ComponentThumbnail({
  componentId,
  componentVersion,
  size = 256,
  className,
}: ComponentThumbnailProps) {
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void componentThumbnail(componentId, componentVersion, undefined, size).then((next) => {
      if (alive) setUrl(next);
    });
    return () => {
      alive = false;
    };
  }, [componentId, componentVersion, size]);

  if (!url) {
    return (
      <div
        data-testid="component-thumbnail-fallback"
        className={className ?? "flex aspect-square items-center justify-center rounded bg-panel text-ink-8"}
      >
        <Icon name="cube" size={22} strokeWidth={1.5} />
      </div>
    );
  }
  return (
    <img
      src={url}
      alt=""
      data-testid="component-thumbnail"
      className={className ?? "aspect-square w-full rounded bg-panel object-contain"}
    />
  );
}
