import { MonoValue } from "@/ui/MonoValue";
import { TitleBarButton } from "@/features/shell/TitleBarButton";
import { usePaletteStore } from "@/stores/paletteStore";

/**
 * Title-bar affordance for ⌘K (prototype 2a).
 *
 * A discoverability control, not a second way to do something: the palette is
 * a keyboard surface, and a user who never learns the chord never finds it. The
 * chord is printed ON the button for the same reason.
 */
export function PaletteButton() {
  const show = usePaletteStore((s) => s.show);
  return (
    <TitleBarButton
      icon="search"
      aria-label="Search commands (⌘K)"
      data-testid="palette-button"
      onClick={show}
    >
      <MonoValue className="text-[11px] text-ink-4">⌘K</MonoValue>
    </TitleBarButton>
  );
}
