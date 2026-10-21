//! Viewport overlays (g03): view cluster, nav pill, session strip, value tag/HUD and
//! feature card. Cards c-view, c-nav, c-strip, c-valtag, c-card.

mod card;
mod dialog;
mod nav;
mod strip;
mod tab_close;
mod tab_switcher;
mod valtag;
mod view;

pub use card::{FeatureCard, FeatureCardOutput, FeatureCardRows};
pub use dialog::{Dialog, DialogIntent, DialogOutcome, affected_list, details, undo_hint};
pub use nav::{NavButton, NavPill};
pub use strip::{SessionStrip, StripAction, StripVariant};
pub use tab_close::{TabCloseOutcome, TabClosePopover};
pub use tab_switcher::{SwitcherItem, TabSwitcher, TabSwitcherOutput};
pub use valtag::{ValueTag, ValueTagTone};
pub use view::{SquareButton, ViewCluster, ViewClusterOutput};

#[cfg(test)]
mod tab_tests;
#[cfg(test)]
mod tests;
