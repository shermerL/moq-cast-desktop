use egui::{Color32, Context, Frame, Id, Margin, Modal, Stroke, Ui};

use crate::{COLORS, Radius, Size, Spacing, TypographyRole, typography};

/// Policy controlling which ambient interactions may close a dialog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogClosePolicy {
    /// Escape, backdrop clicks, and explicit close requests close the dialog.
    EscapeAndBackdrop,
    /// Escape and explicit close requests close the dialog.
    EscapeOnly,
    /// Only an explicit close request closes the dialog.
    ExplicitOnly,
}

/// Display and focus configuration for a modal dialog.
#[derive(Clone, Copy, Debug)]
pub struct DialogSpec<'a> {
    id: Id,
    title: &'a str,
    initial_focus_id: Id,
    close_policy: DialogClosePolicy,
}

impl<'a> DialogSpec<'a> {
    /// Creates a dialog with a stable ID, title, and safe initial focus target.
    pub fn new(id: Id, title: &'a str, initial_focus_id: Id) -> Self {
        Self {
            id,
            title,
            initial_focus_id,
            close_policy: DialogClosePolicy::EscapeAndBackdrop,
        }
    }

    /// Sets the dialog's ambient close policy.
    pub fn close_policy(mut self, close_policy: DialogClosePolicy) -> Self {
        self.close_policy = close_policy;
        self
    }
}

/// Result returned after rendering a modal dialog.
pub struct DialogResponse<T> {
    inner: T,
    should_close: bool,
}

impl<T> DialogResponse<T> {
    /// Returns the caller's content result.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Consumes the response and returns the caller's content result.
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Returns whether the active close policy requested dismissal.
    pub fn should_close(&self) -> bool {
        self.should_close
    }
}

/// Renders a true modal that blocks input to content behind its backdrop.
pub fn dialog<T>(
    context: &Context,
    spec: DialogSpec<'_>,
    content: impl FnOnce(&mut Ui) -> T,
) -> DialogResponse<T> {
    let pass = context.cumulative_pass_nr();
    let seen_key = spec.id.with("last_seen_pass");
    let previous = context.data_mut(|data| data.get_temp::<u64>(seen_key));
    context.data_mut(|data| data.insert_temp(seen_key, pass));
    if previous.is_none_or(|previous| previous + 1 < pass) {
        context.memory_mut(|memory| memory.request_focus(spec.initial_focus_id));
    }

    let response = Modal::new(spec.id)
        .backdrop_color(Color32::from_black_alpha(112))
        .frame(
            Frame::new()
                .fill(COLORS.surface.into())
                .stroke(Stroke::new(Size::BORDER, COLORS.border))
                .corner_radius(egui::CornerRadius::same(Radius::LG as u8))
                .inner_margin(Margin::same(Size::DIALOG_PADDING as i8)),
        )
        .show(context, |ui| {
            ui.set_max_width(Size::DIALOG_MAX);
            ui.label(typography(
                spec.title,
                TypographyRole::Section,
                COLORS.text.into(),
            ));
            ui.add_space(Spacing::LG);
            content(ui)
        });

    let explicit = response.response.should_close();
    let backdrop = response.backdrop_response.clicked();
    let escape = response.is_top_modal
        && !response.any_popup_open
        && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    DialogResponse {
        inner: response.inner,
        should_close: close_requested(spec.close_policy, explicit, backdrop, escape),
    }
}

fn close_requested(
    policy: DialogClosePolicy,
    explicit: bool,
    backdrop: bool,
    escape: bool,
) -> bool {
    explicit
        || match policy {
            DialogClosePolicy::EscapeAndBackdrop => backdrop || escape,
            DialogClosePolicy::EscapeOnly => escape,
            DialogClosePolicy::ExplicitOnly => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_policy_is_explicit_and_exhaustive() {
        assert!(close_requested(
            DialogClosePolicy::EscapeAndBackdrop,
            false,
            true,
            false
        ));
        assert!(close_requested(
            DialogClosePolicy::EscapeOnly,
            false,
            false,
            true
        ));
        assert!(!close_requested(
            DialogClosePolicy::EscapeOnly,
            false,
            true,
            false
        ));
        assert!(!close_requested(
            DialogClosePolicy::ExplicitOnly,
            false,
            true,
            true
        ));
        assert!(close_requested(
            DialogClosePolicy::ExplicitOnly,
            true,
            false,
            false
        ));
    }

    #[test]
    fn dialog_spec_preserves_safe_initial_focus_and_policy() {
        let focus = Id::new("safe-cancel-action");
        let spec = DialogSpec::new(Id::new("dialog"), "Confirm", focus)
            .close_policy(DialogClosePolicy::EscapeOnly);
        assert_eq!(spec.initial_focus_id, focus);
        assert_eq!(spec.close_policy, DialogClosePolicy::EscapeOnly);
    }
}
