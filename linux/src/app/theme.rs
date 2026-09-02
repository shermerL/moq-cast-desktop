//! Shared desktop theme installation for the Linux application.

use eframe::egui;

pub(super) fn configure(context: &egui::Context) {
    moqcast_ui::install_ui_font(
        context,
        std::borrow::Cow::Borrowed(moqcast_ui::NOTO_SANS_SC),
    );
    moqcast_ui::Theme.apply(context);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_font_covers_core_simplified_chinese() {
        let context = egui::Context::default();
        configure(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            ui.fonts_mut(|fonts| {
                assert!(fonts.has_glyphs(
                    &egui::FontId::proportional(14.0),
                    "附近设备屏幕共享观看设置"
                ));
            });
        });
        output.textures_delta.clear();
    }
}
