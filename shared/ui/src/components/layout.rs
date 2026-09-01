use egui::{Align, Layout, Rect, Ui, UiBuilder, pos2};

use crate::Size;

/// A semantic maximum width for a centered page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageWidth {
    /// Dense workspaces and wide lists.
    Wide,
    /// Focused tasks and media surfaces.
    Medium,
    /// Settings and form-heavy pages.
    Narrow,
}

impl PageWidth {
    /// Returns the maximum content width in logical points.
    pub const fn max_width(self) -> f32 {
        match self {
            Self::Wide => Size::PAGE_WIDE_MAX,
            Self::Medium => Size::PAGE_MEDIUM_MAX,
            Self::Narrow => Size::PAGE_NARROW_MAX,
        }
    }
}

/// Returns the horizontal page inset for the available viewport width.
pub const fn page_horizontal_inset(available_width: f32) -> f32 {
    if available_width < Size::SPLIT_BREAKPOINT {
        Size::PAGE_HORIZONTAL_NARROW
    } else {
        Size::PAGE_HORIZONTAL_WIDE
    }
}

/// Resolves a centered page rectangle with shared outer insets.
pub fn page_content_rect(available: Rect, width: PageWidth) -> Rect {
    let narrow = available.width() < Size::SPLIT_BREAKPOINT;
    let horizontal = page_horizontal_inset(available.width());
    let top = if narrow {
        Size::PAGE_TOP_NARROW
    } else {
        Size::PAGE_TOP_WIDE
    };
    centered_rect(
        available,
        width.max_width(),
        horizontal,
        top,
        Size::PAGE_BOTTOM,
    )
}

/// Resolves a centered app-bar rectangle aligned to wide page content.
pub fn app_bar_content_rect(available: Rect) -> Rect {
    centered_rect(
        available,
        PageWidth::Wide.max_width(),
        page_horizontal_inset(available.width()),
        0.0,
        0.0,
    )
}

/// Renders a page inside a centered role-based content rectangle.
pub fn page_shell<R>(ui: &mut Ui, width: PageWidth, content: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = page_content_rect(ui.available_rect_before_wrap(), width);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_width(rect.width());
            content(ui)
        },
    )
    .inner
}

fn centered_rect(
    available: Rect,
    max_width: f32,
    horizontal_inset: f32,
    top_inset: f32,
    bottom_inset: f32,
) -> Rect {
    let usable_width = (available.width() - horizontal_inset * 2.0).max(1.0);
    let width = usable_width.min(max_width);
    Rect::from_min_max(
        pos2(
            available.center().x - width / 2.0,
            available.top() + top_inset,
        ),
        pos2(
            available.center().x + width / 2.0,
            (available.bottom() - bottom_inset).max(available.top() + top_inset + 1.0),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_widths_share_centering_but_keep_role_limits() {
        let available = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(1440.0, 900.0));
        for (role, expected_width) in [
            (PageWidth::Wide, 1120.0),
            (PageWidth::Medium, 880.0),
            (PageWidth::Narrow, 720.0),
        ] {
            let rect = page_content_rect(available, role);
            assert_eq!(rect.width(), expected_width);
            assert_eq!(rect.center().x, available.center().x);
        }
    }

    #[test]
    fn minimum_viewport_keeps_symmetric_narrow_insets() {
        let available = Rect::from_min_size(
            pos2(0.0, 0.0),
            egui::vec2(Size::MIN_VIEWPORT[0], Size::MIN_VIEWPORT[1]),
        );
        let rect = page_content_rect(available, PageWidth::Narrow);
        assert_eq!(rect.left(), Size::PAGE_HORIZONTAL_NARROW);
        assert_eq!(
            available.right() - rect.right(),
            Size::PAGE_HORIZONTAL_NARROW
        );
        assert_eq!(rect.top(), Size::PAGE_TOP_NARROW);
        assert_eq!(available.bottom() - rect.bottom(), Size::PAGE_BOTTOM);
    }
}
