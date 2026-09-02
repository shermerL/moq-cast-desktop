//! Small reusable controls for the desktop pages.

use eframe::egui::{self, Response};
use moqcast_ui::{
    COLORS, StatePanelKind, StatePanelSpec, TypographyRole, state_panel, status_strip, typography,
};

use super::{AppSnapshot, DiscoveryState, Locale, PeerDiscoveryState};

#[derive(Clone, Copy)]
pub(super) enum BadgeTone {
    Neutral,
    Info,
    Success,
    Error,
}

pub(super) fn page_header(ui: &mut egui::Ui, title: &str, description: &str) {
    moqcast_ui::page_header(ui, title, (!description.is_empty()).then_some(description));
}

pub(super) fn mesh_summary(ui: &mut egui::Ui, locale: Locale, snapshot: &AppSnapshot) {
    let visible = snapshot
        .peers
        .values()
        .filter(|peer| peer.discovery == PeerDiscoveryState::Found)
        .count();
    let status = match snapshot.discovery {
        DiscoveryState::Idle => locale.discovery_idle(),
        DiscoveryState::Scanning => locale.scanning(),
        DiscoveryState::Ready | DiscoveryState::Empty => locale.discovery_ready(),
        DiscoveryState::Error => locale.discovery_error(),
    };
    ui.label(typography(
        locale.nearby_summary(status, visible),
        TypographyRole::Meta,
        COLORS.muted.into(),
    ));
}

pub(super) fn status_badge(ui: &mut egui::Ui, label: &str, tone: BadgeTone) -> Response {
    moqcast_ui::status_badge(
        ui,
        label,
        match tone {
            BadgeTone::Neutral => moqcast_ui::BadgeTone::Neutral,
            BadgeTone::Info | BadgeTone::Success => moqcast_ui::BadgeTone::Info,
            BadgeTone::Error => moqcast_ui::BadgeTone::Danger,
        },
    )
}

pub(super) fn primary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> Response {
    moqcast_ui::primary_button(ui, label, enabled)
}

pub(super) fn secondary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> Response {
    moqcast_ui::secondary_button(ui, label, enabled)
}

pub(super) fn danger_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> Response {
    moqcast_ui::danger_button(ui, label, enabled)
}

pub(super) fn empty_state(ui: &mut egui::Ui, title: &str, description: &str, busy: bool) {
    state_panel(
        ui,
        StatePanelSpec::new(
            if busy {
                StatePanelKind::Pending
            } else {
                StatePanelKind::Empty
            },
            title,
            description,
        ),
        |ui| {
            if busy {
                ui.spinner();
            }
        },
    );
}

pub(super) fn status_line(ui: &mut egui::Ui, label: &str, tone: BadgeTone) {
    status_badge(ui, label, tone);
}

pub(super) fn error_banner(
    ui: &mut egui::Ui,
    title: &str,
    error: &str,
    action_label: Option<&str>,
) -> bool {
    let mut clicked = false;
    status_strip(
        ui,
        StatePanelSpec::new(StatePanelKind::Failed, title, error),
        |ui| {
            if let Some(label) = action_label {
                clicked = secondary_button(ui, label, true).clicked();
            }
        },
    );
    clicked
}

pub(super) fn stable_status_strip<R>(
    ui: &mut egui::Ui,
    spec: StatePanelSpec<'_>,
    action: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let inner_width = status_strip_inner_width(ui.available_width());
    status_strip(ui, spec, |ui| {
        ui.set_min_width(inner_width);
        action(ui)
    })
}

fn status_strip_inner_width(available_width: f32) -> f32 {
    (available_width - moqcast_ui::Spacing::LG * 2.0).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_status_strip_reserves_the_full_page_width() {
        assert_eq!(status_strip_inner_width(880.0), 848.0);
        assert_eq!(status_strip_inner_width(16.0), 1.0);
    }
}
