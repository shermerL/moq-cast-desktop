use std::collections::BTreeSet;

use moqcast_ui::{
    COLORS, CheckboxSpec, ControlRole, IconButtonSpec, Interaction, PageWidth, Radius, Size,
    Spacing, SwitchSpec, TypographyRole, install_ui_font, page_content_rect,
    resolve_control_visual, typography, typography_spec,
};
use sha2::{Digest, Sha256};

const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-VF.otf");

#[test]
fn tokens_match_the_frozen_contract() {
    let colors = [
        (COLORS.canvas, "#ECEEEF"),
        (COLORS.chrome, "#F6F7F8"),
        (COLORS.surface, "#FFFFFF"),
        (COLORS.surface_muted, "#F5F6F7"),
        (COLORS.text, "#1D2228"),
        (COLORS.muted, "#626B75"),
        (COLORS.border, "#D9DDE1"),
        (COLORS.border_strong, "#B8C0C8"),
        (COLORS.brand, "#087C82"),
        (COLORS.brand_hover, "#066B70"),
        (COLORS.brand_pressed, "#055B60"),
        (COLORS.brand_soft, "#E4F3F3"),
        (COLORS.secondary_pressed, "#E8EBED"),
        (COLORS.danger, "#B3261E"),
        (COLORS.danger_soft, "#FCE8E6"),
        (COLORS.danger_hover, "#F9DAD7"),
        (COLORS.danger_pressed, "#F5C8C4"),
        (COLORS.warning, "#8A5800"),
        (COLORS.warning_soft, "#FFF4D6"),
        (COLORS.info, "#174F7A"),
        (COLORS.info_soft, "#EDF5FC"),
        (COLORS.live, "#D92D20"),
        (COLORS.focus, "#0067C0"),
        (COLORS.player, "#050607"),
        (COLORS.player_bar, "#17191C"),
        (COLORS.player_text, "#F7F8F9"),
        (COLORS.player_muted, "#B8BEC4"),
    ];
    for (actual, expected) in colors {
        assert_eq!(actual.to_hex(), expected);
    }
    assert_eq!(
        Spacing::ALL,
        [0.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0, 40.0, 48.0]
    );
    assert_eq!(Radius::ALL, [0.0, 4.0, 6.0, 8.0]);
    assert_eq!(Size::APP_BAR, 68.0);
    assert_eq!(Size::APP_BAR_COMPACT, 108.0);
    assert_eq!(Size::NAV, 48.0);
    assert_eq!(Size::CONTROL, 40.0);
    assert_eq!(Size::SWITCH, [44.0, 26.0]);
    assert_eq!(Size::PLAYER_TOOLBAR, 52.0);
    assert_eq!(Size::MIN_VIEWPORT, [680.0, 640.0]);
    assert_eq!(Size::PAGE_HORIZONTAL_WIDE, 40.0);
    assert_eq!(Size::PAGE_HORIZONTAL_NARROW, 24.0);
    assert_eq!(Size::PAGE_TOP_WIDE, 32.0);
    assert_eq!(Size::PAGE_TOP_NARROW, 24.0);
    assert_eq!(Size::PAGE_BOTTOM, 48.0);
    assert_eq!(Size::PAGE_HEADER_MIN, 64.0);
    assert_eq!(Size::PAGE_TITLE_SPACING, 8.0);
    assert_eq!(Size::PAGE_HEADER_SPACING, 40.0);
    assert_eq!(Size::MAJOR_SECTION_SPACING, 40.0);
    assert_eq!(Size::SECTION_CONTENT_SPACING, 16.0);
    assert_eq!(Size::ROW_HORIZONTAL_INSET, 16.0);
    assert_eq!(Size::SETTING_ROW, 68.0);
    assert_eq!(Size::DETAIL_ROW, 48.0);
    assert_eq!(Size::DEVICE_ROW, 72.0);
    assert_eq!(Size::SETTING_CONTROL_MAX, 300.0);
    assert_eq!(Size::SETTINGS_BREAKPOINT, 640.0);
    assert_eq!(Size::PAGE_WIDE_MAX, 1120.0);
    assert_eq!(Size::PAGE_MEDIUM_MAX, 880.0);
    assert_eq!(Size::PAGE_NARROW_MAX, 720.0);
    assert_eq!(Size::PLAYER_SPACING, 0.0);
    assert_eq!(Size::PLAYER_TOOLBAR_ITEM_SPACING, 8.0);
    assert_eq!(Size::PLAYER_ASPECT, [16.0, 9.0]);
    assert_eq!(Size::DIALOG_ACTION_SPACING, 8.0);
    assert_eq!(Size::NEARBY_LIST, 360.0);
    assert_eq!(Size::WORKSPACE_MIN, 360.0);
    assert_eq!(Size::SPLIT_BREAKPOINT, 920.0);
    assert_eq!(Size::VIEWPORT_WINDOWS, [1440.0, 900.0]);
    assert_eq!(Size::VIEWPORT_LINUX, [1024.0, 768.0]);
    assert_eq!(Size::VIEWPORT_MACOS, [720.0, 900.0]);
    assert_eq!(Size::DISABLED_ALPHA, 0.55);
    assert_eq!(Size::BORDER, 1.0);
    assert_eq!(Size::FOCUS, 2.0);
    assert_eq!(Size::FOCUS_OUTSET, 2.0);
    assert_eq!(Size::NAV_UNDERLINE, 3.0);
    assert_eq!(Size::NAV_HORIZONTAL_PADDING, 16.0);
    assert_eq!(Size::NAV_RADIUS, 6.0);
}

#[test]
fn centered_page_roles_keep_stable_widths_without_minimum_viewport_overflow() {
    let review = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1120.0, 792.0));
    let expected = [
        (PageWidth::Wide, 1040.0),
        (PageWidth::Medium, 880.0),
        (PageWidth::Narrow, 720.0),
    ];
    for (role, width) in expected {
        let rect = page_content_rect(review, role);
        assert_eq!(rect.width(), width);
        assert_eq!(rect.center().x, review.center().x);
    }

    let minimum = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(680.0, 640.0));
    for role in [PageWidth::Wide, PageWidth::Medium, PageWidth::Narrow] {
        let rect = page_content_rect(minimum, role);
        assert_eq!(rect.width(), 632.0);
        assert_eq!(rect.left(), 24.0);
        assert_eq!(minimum.right() - rect.right(), 24.0);
    }
}

#[test]
fn typography_roles_use_real_variable_weights() {
    let expected = [
        (TypographyRole::PageTitle, 30.0, 38.0, 600.0),
        (TypographyRole::Section, 18.0, 26.0, 600.0),
        (TypographyRole::Row, 15.0, 22.0, 500.0),
        (TypographyRole::Button, 14.0, 20.0, 600.0),
        (TypographyRole::Body, 14.0, 20.0, 400.0),
        (TypographyRole::Help, 13.0, 18.0, 400.0),
        (TypographyRole::Meta, 12.0, 16.0, 500.0),
        (TypographyRole::Mono, 12.0, 18.0, 400.0),
    ];

    for (role, size, line_height, weight) in expected {
        let spec = typography_spec(role);
        assert_eq!(
            (spec.size, spec.line_height, spec.weight),
            (size, line_height, weight)
        );
    }
}

#[test]
fn interaction_resolution_keeps_selected_distinct_from_primary() {
    let selected = resolve_control_visual(ControlRole::Secondary, Interaction::Selected);
    let primary = resolve_control_visual(ControlRole::Primary, Interaction::Rest);
    let disabled = resolve_control_visual(ControlRole::Primary, Interaction::Disabled);
    let focused = resolve_control_visual(ControlRole::Secondary, Interaction::Focused);

    assert_eq!(selected.fill, COLORS.brand_soft);
    assert_ne!(selected.fill, primary.fill);
    assert_eq!(disabled.opacity, 0.55);
    assert_eq!(focused.focus.unwrap().width, 2.0);
    assert_eq!(focused.focus_outset, 2.0);
}

#[test]
fn navigation_has_rest_hover_pressed_focus_and_selected_contracts() {
    let rest = resolve_control_visual(ControlRole::Nav, Interaction::Rest);
    let hovered = resolve_control_visual(ControlRole::Nav, Interaction::Hovered);
    let pressed = resolve_control_visual(ControlRole::Nav, Interaction::Pressed);
    let focused = resolve_control_visual(ControlRole::Nav, Interaction::Focused);
    let selected = resolve_control_visual(ControlRole::Nav, Interaction::Selected);

    assert_eq!(rest.fill, COLORS.chrome);
    assert_eq!(hovered.fill, COLORS.brand_soft);
    assert_eq!(hovered.border, COLORS.brand);
    assert_eq!(pressed.fill, COLORS.secondary_pressed);
    assert_eq!(pressed.border, COLORS.brand_pressed);
    assert!(focused.focus.is_some());
    assert_eq!(selected.fill, COLORS.brand_soft);
    assert_eq!(selected.text, COLORS.brand);
    assert_eq!(selected.underline, Size::NAV_UNDERLINE);
}

#[test]
fn enabled_button_roles_change_both_fill_and_border_across_pointer_states() {
    for role in [
        ControlRole::Primary,
        ControlRole::Secondary,
        ControlRole::Icon,
    ] {
        let rest = resolve_control_visual(role, Interaction::Rest);
        let hovered = resolve_control_visual(role, Interaction::Hovered);
        let pressed = resolve_control_visual(role, Interaction::Pressed);
        assert_ne!(hovered.fill, rest.fill);
        assert_ne!(hovered.border, rest.border);
        assert_ne!(pressed.fill, hovered.fill);
    }
    let danger_rest = resolve_control_visual(ControlRole::Danger, Interaction::Rest);
    let danger_hovered = resolve_control_visual(ControlRole::Danger, Interaction::Hovered);
    let danger_pressed = resolve_control_visual(ControlRole::Danger, Interaction::Pressed);
    assert_ne!(danger_hovered.fill, danger_rest.fill);
    assert_ne!(danger_pressed.fill, danger_hovered.fill);
}

#[test]
fn every_control_role_resolves_every_interaction_state() {
    let roles = [
        ControlRole::Nav,
        ControlRole::Primary,
        ControlRole::Secondary,
        ControlRole::Danger,
        ControlRole::Icon,
        ControlRole::PlayerIcon,
    ];
    let states = [
        Interaction::Rest,
        Interaction::Hovered,
        Interaction::Pressed,
        Interaction::Selected,
        Interaction::Focused,
        Interaction::Disabled,
    ];

    for role in roles {
        for state in states {
            let visual = resolve_control_visual(role, state);
            assert_eq!(visual.focus.is_some(), state == Interaction::Focused);
            assert_eq!(
                visual.opacity,
                if state == Interaction::Disabled {
                    0.55
                } else {
                    1.0
                }
            );
            assert_eq!(
                visual.underline,
                if role == ControlRole::Nav && state == Interaction::Selected {
                    3.0
                } else {
                    0.0
                }
            );
        }
    }

    assert_eq!(
        resolve_control_visual(ControlRole::Secondary, Interaction::Pressed).fill,
        COLORS.secondary_pressed
    );
    assert_eq!(
        resolve_control_visual(ControlRole::Danger, Interaction::Hovered).fill,
        COLORS.danger_hover
    );
    assert_eq!(
        resolve_control_visual(ControlRole::Danger, Interaction::Pressed).fill,
        COLORS.danger_pressed
    );
    assert_eq!(
        resolve_control_visual(ControlRole::PlayerIcon, Interaction::Selected).fill,
        COLORS.brand
    );
}

#[test]
fn font_asset_is_pinned_and_has_weight_axis() {
    assert_eq!(FONT_BYTES.len(), 15_054_748);
    assert_eq!(
        format!("{:x}", Sha256::digest(FONT_BYTES)),
        "d13ed01ec8aa45d6178999b648e96fb92150683e9f8e2a581f2acf208dcbe44b"
    );

    let axes = egui::FontData::from_static(FONT_BYTES)
        .variation_axes()
        .into_iter();
    let weight = axes
        .into_iter()
        .find(|axis| axis.tag.to_be_bytes() == *b"wght")
        .expect("Noto Sans SC exposes a weight axis");
    assert!(weight.range.min <= 400.0);
    assert!(weight.range.max >= 600.0);
}

#[test]
fn shared_font_covers_chinese_and_english_in_both_families() {
    let context = egui::Context::default();
    install_ui_font(&context, std::borrow::Cow::Borrowed(FONT_BYTES));
    let output = context.run_ui(egui::RawInput::default(), |ui| {
        ui.label(typography(
            "附近设备 Nearby",
            TypographyRole::Body,
            COLORS.text.into(),
        ));
        ui.label(typography(
            "诊断日志 diagnostics.log",
            TypographyRole::Mono,
            COLORS.text.into(),
        ));
    });
    output.drop_without_applying_deltas();
    context.fonts_mut(|fonts| {
        let monospace = &fonts.definitions().families[&egui::FontFamily::Monospace];
        assert_eq!(monospace.first().map(String::as_str), Some("Hack"));
        assert_eq!(
            monospace.last().map(String::as_str),
            Some("Noto Sans SC Variable"),
        );
        for character in "附近设备 Nearby".chars() {
            if character.is_whitespace() {
                continue;
            }
            assert!(
                fonts.has_glyph(&egui::FontId::proportional(14.0), character),
                "missing proportional glyph {character:?}",
            );
        }
        for character in "诊断日志".chars() {
            assert!(
                fonts.has_glyph(&egui::FontId::monospace(12.0), character),
                "missing monospace glyph {character:?}",
            );
        }
    });
}

#[test]
fn production_dependencies_are_business_neutral() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("read shared UI manifest");
    let document = manifest
        .parse::<toml_edit::DocumentMut>()
        .expect("parse shared UI manifest");
    let dependencies = document["dependencies"]
        .as_table()
        .expect("dependencies table")
        .iter()
        .map(|(name, _)| name)
        .collect::<BTreeSet<_>>();
    assert_eq!(dependencies, BTreeSet::from(["egui"]));
}

#[test]
fn interactive_components_keep_a_forty_point_hit_target() {
    egui::__run_test_ui(|ui| {
        let primary = moqcast_ui::primary_button(ui, "Action", true);
        let icon = moqcast_ui::icon_button(ui, IconButtonSpec::new("?", "Help"));
        let mut switched = false;
        let switch = moqcast_ui::switch(ui, &mut switched, SwitchSpec::new("Toggle"));
        let mut checked = false;
        let checkbox = moqcast_ui::checkbox(ui, &mut checked, CheckboxSpec::new("Choice"));

        for response in [primary, icon, switch, checkbox] {
            assert!(response.rect.height() >= 40.0);
        }
    });
}
