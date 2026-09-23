//! Shared playback information control and session-scoped sampling state.

use std::time::Duration;

use egui::{Color32, CornerRadius, Label, Margin, Popup, RectAlign, Stroke, Ui, vec2};

use crate::{COLORS, Radius, Spacing, TypographyRole, typography};

use super::button::{IconButtonSpec, player_icon_button};

const PANEL_WIDTH: f32 = 300.0;
const PANEL_MIN_WIDTH: f32 = 180.0;
const SAMPLE_INTERVAL: f64 = 1.0;

/// Locale used by the shared playback information control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerInfoLocale {
    /// Simplified Chinese labels.
    Chinese,
    /// English labels.
    English,
}

/// Audio availability shown by the playback information panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayerAudioState {
    /// The current catalog or audio pipeline has not resolved yet.
    #[default]
    Pending,
    /// The selected broadcast has no playable audio track.
    NoAudio,
    /// A playable audio track is active.
    Ready,
    /// A playable audio track is active but locally muted.
    Muted,
    /// The audio path failed while video playback continued.
    Failed,
}

/// Identity of one unique video frame accepted by the player UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerFrameSample {
    /// Playback generation that owns the frame.
    pub view_generation: u64,
    /// Decoder generation that produced the frame.
    pub decoder_generation: u64,
    /// Sequence assigned by the active decoder pipeline.
    pub sequence: u64,
}

impl PlayerFrameSample {
    fn stream_key(self) -> (u64, u64) {
        (self.view_generation, self.decoder_generation)
    }
}

/// Truthful playback fields available to the shared information panel.
#[derive(Clone, Copy, Debug)]
pub struct PlayerInfoSnapshot<'a> {
    /// Playback generation used to reset panel-local state.
    pub generation: u64,
    /// User-facing name of the selected playback source, when available.
    pub source: Option<&'a str>,
    /// Latest unique frame accepted by the UI.
    pub frame: Option<PlayerFrameSample>,
    /// Actual decoded display dimensions.
    pub resolution: Option<(u32, u32)>,
    /// Codec of the currently selected video track.
    pub video_codec: Option<&'a str>,
    /// Backend that decoded the current video frame.
    pub decoder: Option<&'a str>,
    /// Current remote-audio availability.
    pub audio_state: PlayerAudioState,
    /// Codec of the currently selected audio track.
    pub audio_codec: Option<&'a str>,
    /// Sample rate selected for the current audio track, in samples per second.
    pub audio_sample_rate: Option<u32>,
    /// Channel count selected for the current audio track.
    pub audio_channels: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default)]
struct FrameUpdateMeter {
    stream_key: Option<(u64, u64)>,
    last_frame: Option<PlayerFrameSample>,
    window_started_at: Option<f64>,
    accepted_frames: u64,
    updates_per_second: Option<f64>,
}

impl FrameUpdateMeter {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn begin(&mut self, now: f64, frame: Option<PlayerFrameSample>) {
        self.stream_key = frame.map(PlayerFrameSample::stream_key);
        self.last_frame = frame;
        self.window_started_at = Some(now);
        self.accepted_frames = 0;
        self.updates_per_second = None;
    }

    fn observe(&mut self, now: f64, frame: Option<PlayerFrameSample>) {
        let Some(frame) = frame else {
            self.reset();
            return;
        };
        if self.stream_key != Some(frame.stream_key()) || self.window_started_at.is_none() {
            self.begin(now, Some(frame));
            return;
        }
        if self.last_frame != Some(frame) {
            self.last_frame = Some(frame);
            self.accepted_frames = self.accepted_frames.saturating_add(1);
        }
        let started = self.window_started_at.unwrap_or(now);
        let elapsed = now - started;
        if elapsed < 0.0 {
            self.begin(now, Some(frame));
        } else if elapsed >= SAMPLE_INTERVAL {
            self.updates_per_second = Some(self.accepted_frames as f64 / elapsed.max(f64::EPSILON));
            self.window_started_at = Some(now);
            self.accepted_frames = 0;
        }
    }

    fn repaint_after(self, now: f64) -> Duration {
        let elapsed = self
            .window_started_at
            .map_or(0.0, |started| (now - started).max(0.0));
        Duration::from_secs_f64((SAMPLE_INTERVAL - elapsed).max(0.01))
    }
}

/// Session-scoped state for the playback information popup.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerInfoState {
    generation: Option<u64>,
    open: bool,
    meter: FrameUpdateMeter,
}

impl PlayerInfoState {
    /// Closes and clears the panel when a different playback generation appears.
    pub fn sync_generation(&mut self, generation: u64) -> bool {
        if self.generation == Some(generation) {
            return false;
        }
        self.generation = Some(generation);
        self.open = false;
        self.meter.reset();
        true
    }

    /// Returns whether the information popup is currently open.
    pub fn open(self) -> bool {
        self.open
    }

    fn toggle(&mut self, now: f64, frame: Option<PlayerFrameSample>) {
        self.open = !self.open;
        if self.open {
            self.meter.begin(now, frame);
        } else {
            self.meter.reset();
        }
    }

    fn close(&mut self) {
        self.open = false;
        self.meter.reset();
    }
}

/// Interaction result from one playback information control render.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct PlayerInfoResponse {
    active: bool,
    #[cfg(test)]
    button_rect: egui::Rect,
}

impl PlayerInfoResponse {
    /// Returns whether the toolbar button is hovered, focused, or pressed.
    pub fn active(self) -> bool {
        self.active
    }
}

/// Renders a toolbar information button and a non-resizing playback details popup.
pub fn player_info_control(
    ui: &mut Ui,
    state: &mut PlayerInfoState,
    locale: PlayerInfoLocale,
    now: f64,
    snapshot: PlayerInfoSnapshot<'_>,
    enabled: bool,
) -> PlayerInfoResponse {
    state.sync_generation(snapshot.generation);
    if !enabled && state.open {
        state.close();
    }

    let button = player_icon_button(
        ui,
        IconButtonSpec::player("ⓘ", info_button_label(locale)).enabled(enabled),
    );
    let button_active =
        button.contains_pointer() || button.has_focus() || button.is_pointer_button_down_on();
    if button.clicked() {
        state.toggle(now, snapshot.frame);
    }

    if state.open {
        state.meter.observe(now, snapshot.frame);
        ui.ctx()
            .request_repaint_after(state.meter.repaint_after(now));
    }

    let was_open = state.open;
    let available =
        (ui.ctx().content_rect().width() - Spacing::MD * 2.0).clamp(PANEL_MIN_WIDTH, PANEL_WIDTH);
    let popup_id = button.id.with("playback-information");
    Popup::from_response(&button)
        .id(popup_id)
        .open_bool(&mut state.open)
        .align(RectAlign::TOP_END)
        .width(available)
        .frame(
            egui::Frame::popup(ui.style())
                .fill(COLORS.player_bar.into())
                .stroke(Stroke::new(
                    1.0,
                    Color32::from(COLORS.player_muted).gamma_multiply(0.45),
                ))
                .corner_radius(CornerRadius::same(Radius::MD as u8))
                .inner_margin(Margin::same(Spacing::MD as i8)),
        )
        .show(|ui| show_player_info(ui, locale, snapshot, state.meter.updates_per_second));
    if was_open && !state.open {
        state.meter.reset();
    }

    PlayerInfoResponse {
        active: button_active,
        #[cfg(test)]
        button_rect: button.rect,
    }
}

fn show_player_info(
    ui: &mut Ui,
    locale: PlayerInfoLocale,
    snapshot: PlayerInfoSnapshot<'_>,
    updates_per_second: Option<f64>,
) {
    ui.label(typography(
        panel_title(locale),
        TypographyRole::Section,
        COLORS.player_text.into(),
    ));
    ui.add_space(Spacing::XS);
    if let Some(source) = snapshot.source {
        info_row(ui, source_label(locale), source.to_owned());
        ui.add_space(Spacing::SM);
        ui.separator();
        ui.add_space(Spacing::SM);
    }
    info_section(ui, video_section_label(locale));
    info_row(
        ui,
        resolution_label(locale),
        snapshot
            .resolution
            .map(|(width, height)| format!("{width} × {height}"))
            .unwrap_or_else(|| unavailable_label(locale).to_owned()),
    );
    info_row(
        ui,
        codec_label(locale),
        snapshot
            .video_codec
            .unwrap_or_else(|| unavailable_label(locale))
            .to_owned(),
    );
    info_row(
        ui,
        decoder_label(locale),
        snapshot
            .decoder
            .unwrap_or_else(|| unavailable_label(locale))
            .to_owned(),
    );
    let update_rate = updates_per_second
        .map(|rate| frame_rate_value(locale, rate))
        .unwrap_or_else(|| {
            if snapshot.frame.is_some() {
                measuring_label(locale).to_owned()
            } else {
                unavailable_label(locale).to_owned()
            }
        });
    info_row(ui, frame_rate_label(locale), update_rate);

    ui.add_space(Spacing::SM);
    ui.separator();
    ui.add_space(Spacing::SM);
    info_section(ui, audio_section_label(locale));
    info_row(
        ui,
        status_label(locale),
        audio_state_label(locale, snapshot.audio_state).to_owned(),
    );
    info_row(
        ui,
        codec_label(locale),
        snapshot
            .audio_codec
            .unwrap_or_else(|| unavailable_label(locale))
            .to_owned(),
    );
    info_row(
        ui,
        format_label(locale),
        audio_format(locale, snapshot.audio_sample_rate, snapshot.audio_channels),
    );
}

fn info_section(ui: &mut Ui, label: &str) {
    ui.label(typography(
        label,
        TypographyRole::Meta,
        COLORS.player_text.into(),
    ));
    ui.add_space(Spacing::XS);
}

fn info_row(ui: &mut Ui, label: &str, value: String) {
    let response = ui
        .horizontal(|ui| {
            let label_width = (ui.available_width() * 0.38).min(104.0);
            ui.add_sized(
                vec2(label_width, 0.0),
                Label::new(typography(
                    label,
                    TypographyRole::Meta,
                    COLORS.player_muted.into(),
                )),
            );
            ui.add_sized(
                ui.available_size(),
                Label::new(typography(
                    value.as_str(),
                    TypographyRole::Meta,
                    COLORS.player_text.into(),
                ))
                .truncate(),
            );
        })
        .response;
    response.on_hover_text(value);
}

fn audio_format(
    locale: PlayerInfoLocale,
    sample_rate: Option<u32>,
    channels: Option<u32>,
) -> String {
    let sample_rate = sample_rate.map(|rate| {
        if rate.is_multiple_of(1_000) {
            format!("{} kHz", rate / 1_000)
        } else {
            format!("{rate} Hz")
        }
    });
    let channels = channels.map(|channels| match locale {
        PlayerInfoLocale::Chinese => format!("{channels} 声道"),
        PlayerInfoLocale::English => format!("{channels} channels"),
    });
    match (sample_rate, channels) {
        (Some(sample_rate), Some(channels)) => format!("{sample_rate} · {channels}"),
        (Some(sample_rate), None) => sample_rate,
        (None, Some(channels)) => channels,
        (None, None) => unavailable_label(locale).to_owned(),
    }
}

fn frame_rate_value(locale: PlayerInfoLocale, rate: f64) -> String {
    match locale {
        PlayerInfoLocale::Chinese => format!("{rate:.1} 帧/秒"),
        PlayerInfoLocale::English => format!("{rate:.1} frames/s"),
    }
}

fn info_button_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "播放信息",
        PlayerInfoLocale::English => "Playback information",
    }
}

fn panel_title(locale: PlayerInfoLocale) -> &'static str {
    info_button_label(locale)
}

fn video_section_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "视频",
        PlayerInfoLocale::English => "Video",
    }
}

fn source_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "来源",
        PlayerInfoLocale::English => "Source",
    }
}

fn audio_section_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "音频",
        PlayerInfoLocale::English => "Audio",
    }
}

fn resolution_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "分辨率",
        PlayerInfoLocale::English => "Resolution",
    }
}

fn codec_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "编解码器",
        PlayerInfoLocale::English => "Codec",
    }
}

fn decoder_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "解码后端",
        PlayerInfoLocale::English => "Decoder backend",
    }
}

fn frame_rate_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "画面更新率",
        PlayerInfoLocale::English => "Frame update rate",
    }
}

fn status_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "状态",
        PlayerInfoLocale::English => "Status",
    }
}

fn format_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "格式",
        PlayerInfoLocale::English => "Format",
    }
}

fn unavailable_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "不可用",
        PlayerInfoLocale::English => "Unavailable",
    }
}

fn measuring_label(locale: PlayerInfoLocale) -> &'static str {
    match locale {
        PlayerInfoLocale::Chinese => "测量中",
        PlayerInfoLocale::English => "Measuring",
    }
}

fn audio_state_label(locale: PlayerInfoLocale, state: PlayerAudioState) -> &'static str {
    match (locale, state) {
        (PlayerInfoLocale::Chinese, PlayerAudioState::Pending) => "正在准备",
        (PlayerInfoLocale::Chinese, PlayerAudioState::NoAudio) => "无音轨",
        (PlayerInfoLocale::Chinese, PlayerAudioState::Ready) => "可用",
        (PlayerInfoLocale::Chinese, PlayerAudioState::Muted) => "已静音",
        (PlayerInfoLocale::Chinese, PlayerAudioState::Failed) => "不可用",
        (PlayerInfoLocale::English, PlayerAudioState::Pending) => "Preparing",
        (PlayerInfoLocale::English, PlayerAudioState::NoAudio) => "No audio track",
        (PlayerInfoLocale::English, PlayerAudioState::Ready) => "Available",
        (PlayerInfoLocale::English, PlayerAudioState::Muted) => "Muted",
        (PlayerInfoLocale::English, PlayerAudioState::Failed) => "Unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click_input(pos: egui::Pos2) -> egui::RawInput {
        egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        }
    }

    fn render_info_fixture(
        context: &egui::Context,
        input: egui::RawInput,
        state: &mut PlayerInfoState,
        decoder: Option<&str>,
    ) -> PlayerInfoResponse {
        let mut output = None;
        let frame = context.run_ui(input, |ui| {
            output = Some(player_info_control(
                ui,
                state,
                PlayerInfoLocale::English,
                1.0,
                snapshot(None, decoder),
                true,
            ));
        });
        frame.drop_without_applying_deltas();
        output.expect("information fixture renders")
    }

    fn snapshot<'a>(
        frame: Option<PlayerFrameSample>,
        decoder: Option<&'a str>,
    ) -> PlayerInfoSnapshot<'a> {
        PlayerInfoSnapshot {
            generation: 1,
            source: Some("Studio Mac"),
            frame,
            resolution: Some((1920, 1080)),
            video_codec: Some("avc1.640028"),
            decoder,
            audio_state: PlayerAudioState::Ready,
            audio_codec: Some("opus"),
            audio_sample_rate: Some(48_000),
            audio_channels: Some(2),
        }
    }

    #[test]
    fn playback_information_resets_when_the_view_generation_changes() {
        let mut info = PlayerInfoState::default();
        let first = PlayerFrameSample {
            view_generation: 4,
            decoder_generation: 1,
            sequence: 1,
        };
        assert!(info.sync_generation(4));
        info.toggle(1.0, Some(first));
        assert!(info.open());

        assert!(info.sync_generation(5));
        assert!(!info.open());
        assert_eq!(info.meter.updates_per_second, None);
        assert_eq!(info.meter.last_frame, None);
    }

    #[test]
    fn playback_information_counts_unique_ui_frames_without_filling_sequence_gaps() {
        let mut info = PlayerInfoState::default();
        let first = PlayerFrameSample {
            view_generation: 7,
            decoder_generation: 2,
            sequence: 10,
        };
        info.sync_generation(7);
        info.toggle(1.0, Some(first));

        info.meter.observe(1.2, Some(first));
        info.meter.observe(
            1.6,
            Some(PlayerFrameSample {
                sequence: 40,
                ..first
            }),
        );
        info.meter.observe(
            2.0,
            Some(PlayerFrameSample {
                sequence: 40,
                ..first
            }),
        );

        assert_eq!(info.meter.updates_per_second, Some(1.0));
    }

    #[test]
    fn a_static_frame_reports_zero_after_a_complete_window() {
        let mut info = PlayerInfoState::default();
        let frame = PlayerFrameSample {
            view_generation: 7,
            decoder_generation: 2,
            sequence: 10,
        };
        info.sync_generation(7);
        info.toggle(1.0, Some(frame));
        info.meter.observe(2.0, Some(frame));

        assert_eq!(info.meter.updates_per_second, Some(0.0));
    }

    #[test]
    fn playback_information_starts_a_fresh_window_for_a_replaced_decoder() {
        let mut info = PlayerInfoState::default();
        let first = PlayerFrameSample {
            view_generation: 9,
            decoder_generation: 1,
            sequence: 1,
        };
        info.sync_generation(9);
        info.toggle(1.0, Some(first));
        info.meter.observe(
            1.5,
            Some(PlayerFrameSample {
                sequence: 2,
                ..first
            }),
        );
        info.meter.observe(
            1.6,
            Some(PlayerFrameSample {
                decoder_generation: 2,
                sequence: 1,
                ..first
            }),
        );

        assert_eq!(info.meter.updates_per_second, None);
        assert_eq!(info.meter.accepted_frames, 0);
        assert_eq!(info.meter.window_started_at, Some(1.6));
    }

    #[test]
    fn closing_playback_information_discards_the_sampling_window() {
        let mut info = PlayerInfoState::default();
        let frame = PlayerFrameSample {
            view_generation: 3,
            decoder_generation: 1,
            sequence: 1,
        };
        info.sync_generation(3);
        info.toggle(1.0, Some(frame));
        info.toggle(1.5, Some(frame));

        assert!(!info.open());
        assert_eq!(info.meter.window_started_at, None);
        assert_eq!(info.meter.last_frame, None);
    }

    #[test]
    fn popup_opens_and_closes_from_the_toolbar_button() {
        let context = egui::Context::default();
        let mut state = PlayerInfoState::default();
        let initial = render_info_fixture(&context, egui::RawInput::default(), &mut state, None);
        assert!(!state.open());

        let _ = render_info_fixture(
            &context,
            click_input(initial.button_rect.center()),
            &mut state,
            None,
        );
        assert!(state.open());

        let _ = render_info_fixture(
            &context,
            click_input(initial.button_rect.center()),
            &mut state,
            None,
        );
        assert!(!state.open());
    }

    #[test]
    fn long_backend_names_render_at_narrow_width_without_expanding_the_ui() {
        egui::__run_test_ui(|ui| {
            ui.set_width(PANEL_MIN_WIDTH);
            show_player_info(
                ui,
                PlayerInfoLocale::English,
                snapshot(
                    None,
                    Some("An intentionally long decoder backend name that must truncate"),
                ),
                Some(0.0),
            );
            assert!(ui.min_rect().width() <= PANEL_MIN_WIDTH);
        });
    }

    #[test]
    fn playback_source_label_is_localized() {
        assert_eq!(source_label(PlayerInfoLocale::Chinese), "来源");
        assert_eq!(source_label(PlayerInfoLocale::English), "Source");
    }
}
