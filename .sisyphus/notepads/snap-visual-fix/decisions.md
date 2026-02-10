# Snap Visual Fix - Decisions

## Session: ses_3c0e89a55ffe3ueOACTmYubemo
## Snap Hint Overlay Decisions
- Used QPainter for text rendering as it's the established pattern for overlays in Viewport.cpp.
- Reused theme colors from preview dimensions for consistency.
- Placed the snap hint overlay after the preview dimensions overlay to ensure it renders on top if they overlap.
