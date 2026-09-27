//! Announcement CTA product telemetry events.
//!
//! Upstream also carried the subscription-upsell and credit-limit conversion
//! events here (plan funnels). The fork ships no subscription plans, so those
//! are gone; what remains is the announcement CTA funnel.

use serde::Serialize;

/// Which surface painted an announcement's CTA button.
/// Lets the funnel attribute the click to the welcome hero vs the in-session header vs
/// the banner vs the dashboard. Also distinguishes keyboard (`Ctrl+O`) activations from pointer/OSC 8 ones. Ord/Eq exist
/// so the pager can track which (announcement, surface) pairs already showed the CTA.
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AnnouncementCtaSurface {
    Banner,
    Welcome,
    Header,
    Dashboard,
    Keyboard,
}

/// A promo announcement's CTA button was painted on a surface: the impression half of the per-surface CTR funnel with [`AnnouncementCtaClicked`].
/// Emitted once per (announcement, surface) per pager process (cleared on logout); never emitted for `Keyboard` (a click-only surface).
#[derive(Serialize)]
pub struct AnnouncementCtaShown {
    /// Announcement `id` from the server push (`None` for id-less items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Which surface painted the button.
    pub source: AnnouncementCtaSurface,
}

/// User activated a promo announcement's CTA button (the `[label]` open).
#[derive(Serialize)]
pub struct AnnouncementCtaClicked {
    /// Announcement `id` from the server push (`None` for id-less items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Which surface the activation came from (per-surface conversion signal).
    pub source: AnnouncementCtaSurface,
}
