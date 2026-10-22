//! IP19 accessory policy (INPUT.md): Pencil barrel double tap and squeeze map
//! to editor actions by the user's system preference. They never author
//! geometry, and every action has a visible control equivalent in the host.
use super::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessoryEvent {
    BarrelDoubleTap,
    Squeeze,
}

impl AccessoryEvent {
    /// `ONECAD_IOS_PENCIL_*`.
    pub fn from_abi(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::BarrelDoubleTap,
            1 => Self::Squeeze,
            _ => return None,
        })
    }
}

/// `UIPencilPreferredAction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preference {
    Ignore,
    SwitchEraser,
    SwitchPrevious,
    ShowColorPalette,
    ShowInkAttributes,
    ShowContextualPalette,
    RunSystemShortcut,
}

impl Preference {
    /// `ONECAD_IOS_PREF_*`.
    pub fn from_abi(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Ignore,
            1 => Self::SwitchEraser,
            2 => Self::SwitchPrevious,
            3 => Self::ShowColorPalette,
            4 => Self::ShowInkAttributes,
            5 => Self::ShowContextualPalette,
            6 => Self::RunSystemShortcut,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AccessoryAction {
    Nothing,
    /// Toggle to the previous sketch tool (OneCAD has no eraser).
    TogglePreviousTool,
    /// Open the adaptive tool palette at this position.
    OpenPalette(Point),
    /// The system runs the user's shortcut; the app does nothing.
    SystemShortcut,
}

/// Where a palette opens: the hover pose, else the last Pencil position, else
/// the host's fallback (canvas centre).
pub fn palette_anchor(hover: Option<Point>, last_pencil: Option<Point>, fallback: Point) -> Point {
    hover
        .filter(|point| point.is_finite())
        .or(last_pencil.filter(|point| point.is_finite()))
        .unwrap_or(fallback)
}

/// The action for one completed accessory event. A squeeze acts when it ends
/// (`completed`); a cancelled squeeze does nothing.
pub fn action(
    event: AccessoryEvent,
    preference: Preference,
    completed: bool,
    anchor: Point,
) -> AccessoryAction {
    if !completed {
        return AccessoryAction::Nothing;
    }
    match (event, preference) {
        (_, Preference::Ignore) => AccessoryAction::Nothing,
        (
            AccessoryEvent::BarrelDoubleTap,
            Preference::SwitchPrevious | Preference::SwitchEraser,
        ) => AccessoryAction::TogglePreviousTool,
        (
            _,
            Preference::ShowContextualPalette
            | Preference::ShowColorPalette
            | Preference::ShowInkAttributes,
        ) => AccessoryAction::OpenPalette(anchor),
        (AccessoryEvent::Squeeze, Preference::RunSystemShortcut) => AccessoryAction::SystemShortcut,
        // Not in the policy table: a squeeze preferring a tool switch, or a tap
        // preferring a system shortcut.
        _ => AccessoryAction::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Point = Point::new(10.0, 20.0);

    #[test]
    fn policy_table_rows() {
        use AccessoryAction::*;
        use AccessoryEvent::*;
        use Preference::*;
        let cases = [
            (BarrelDoubleTap, Ignore, Nothing),
            (Squeeze, Ignore, Nothing),
            (BarrelDoubleTap, SwitchPrevious, TogglePreviousTool),
            (BarrelDoubleTap, SwitchEraser, TogglePreviousTool),
            (BarrelDoubleTap, ShowContextualPalette, OpenPalette(AT)),
            (BarrelDoubleTap, ShowColorPalette, OpenPalette(AT)),
            (BarrelDoubleTap, ShowInkAttributes, OpenPalette(AT)),
            (Squeeze, ShowContextualPalette, OpenPalette(AT)),
            (Squeeze, ShowColorPalette, OpenPalette(AT)),
            (Squeeze, ShowInkAttributes, OpenPalette(AT)),
            (Squeeze, RunSystemShortcut, SystemShortcut),
            (Squeeze, SwitchPrevious, Nothing),
            (BarrelDoubleTap, RunSystemShortcut, Nothing),
        ];
        for (event, preference, expected) in cases {
            assert_eq!(
                action(event, preference, true, AT),
                expected,
                "{event:?} {preference:?}"
            );
        }
    }

    #[test]
    fn an_incomplete_or_cancelled_squeeze_does_nothing() {
        assert_eq!(
            action(
                AccessoryEvent::Squeeze,
                Preference::ShowContextualPalette,
                false,
                AT
            ),
            AccessoryAction::Nothing
        );
    }

    #[test]
    fn palette_prefers_hover_then_last_pencil_then_fallback() {
        let hover = Point::new(1.0, 1.0);
        let last = Point::new(2.0, 2.0);
        let fallback = Point::new(3.0, 3.0);
        assert_eq!(palette_anchor(Some(hover), Some(last), fallback), hover);
        assert_eq!(palette_anchor(None, Some(last), fallback), last);
        assert_eq!(palette_anchor(None, None, fallback), fallback);
        let nan = Point::new(f64::NAN, 0.0);
        assert_eq!(palette_anchor(Some(nan), None, fallback), fallback);
    }

    #[test]
    fn abi_values_round_trip_and_unknown_values_are_refused() {
        assert_eq!(
            Preference::from_abi(5),
            Some(Preference::ShowContextualPalette)
        );
        assert_eq!(Preference::from_abi(7), None);
        assert_eq!(AccessoryEvent::from_abi(1), Some(AccessoryEvent::Squeeze));
        assert_eq!(AccessoryEvent::from_abi(2), None);
    }
}
