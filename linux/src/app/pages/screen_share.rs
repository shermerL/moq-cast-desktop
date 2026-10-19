//! Capture selection retains the Linux Portal/X11 ownership boundary.
use super::super::{AppSnapshot, CaptureSource, Locale, MediaState, SourceCatalog, UserCommand};
use eframe::egui;
use moqcast_ui::v2::{self, Action, ActionTone, Palette, StatusTone};
pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    audio: &mut bool,
    selected: &mut Option<CaptureSource>,
) -> Option<UserCommand> {
    let mut command = None;
    let editable = snapshot.media == MediaState::Idle;
    // Reconcile before drawing the fixed action area so it cannot act on stale sources.
    match &snapshot.sources {
        SourceCatalog::Failed(_) => *selected = None,
        SourceCatalog::Portal
            if editable
                && !matches!(
                    selected,
                    Some(CaptureSource::Portal | CaptureSource::PortalWindow)
                ) =>
        {
            *selected = Some(CaptureSource::Portal)
        }
        _ => {}
    }
    let outer = ui.available_rect_before_wrap();
    let action_rect = egui::Rect::from_min_max(
        egui::pos2(outer.left(), (outer.bottom() - 56.0).max(outer.top())),
        outer.right_bottom(),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(action_rect), |ui| {
        ui.horizontal_centered(|ui| {
            let (label, enabled, tone) = match snapshot.media {
                MediaState::Publishing => (locale.stop_sharing(), true, ActionTone::Danger),
                MediaState::StoppingPublish => (locale.stopping_share(), false, ActionTone::Danger),
                MediaState::PreparingPublish => {
                    (locale.preparing_share(), false, ActionTone::Neutral)
                }
                MediaState::Idle
                    if !snapshot.discovery.is_active() && !snapshot.has_mesh_session() =>
                {
                    (
                        locale.text("开启附近设备", "Turn on Nearby"),
                        true,
                        ActionTone::Accent,
                    )
                }
                _ => (
                    locale.text("开始共享", "Start sharing"),
                    editable
                        && snapshot.has_mesh_session()
                        && snapshot.sources.selected(selected.as_ref()).is_some(),
                    ActionTone::Accent,
                ),
            };
            if (Action {
                label,
                size: egui::vec2(164.0, 40.0),
                enabled,
                tone,
            })
            .show(ui)
            .clicked()
            {
                command = Some(match snapshot.media {
                    MediaState::Publishing => UserCommand::StopScreenShare,
                    MediaState::Idle
                        if !snapshot.discovery.is_active() && !snapshot.has_mesh_session() =>
                    {
                        UserCommand::StartDiscovery
                    }
                    _ => UserCommand::StartScreenShare {
                        system_audio: *audio,
                        source: snapshot
                            .sources
                            .selected(selected.as_ref())
                            .expect("enabled source selection"),
                    },
                });
            }
        });
    });
    let content = egui::Rect::from_min_max(
        outer.min,
        egui::pos2(outer.right(), (action_rect.top() - 12.0).max(outer.top())),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(content),|ui| {
        egui::ScrollArea::vertical().id_salt("v2-capture-content").max_height(content.height())
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).auto_shrink([false,false]).show(ui,|ui| {
                let p=Palette::for_ui(ui);
                ui.label(egui::RichText::new(locale.text("共享设置","Sharing settings")).size(20.0).variation(b"wght",600.0));ui.add_space(18.0);
                p.card().show(ui,|ui| {
                    ui.set_width(ui.available_width());
                    ui.add_enabled_ui(editable,|ui| {
                        match &snapshot.sources {
                            SourceCatalog::Unloaded=>{if editable {command.get_or_insert(UserCommand::RefreshCaptureSources);}},
                            SourceCatalog::Loading=>{ui.spinner();},
                            SourceCatalog::Portal=>{
                                ui.horizontal(|ui| {
                                    for (source, label) in [(CaptureSource::Portal, locale.text("屏幕", "Screen")), (CaptureSource::PortalWindow, locale.text("窗口", "Window"))] {
                                        if ui.add(egui::Button::selectable(selected.as_ref() == Some(&source), label).min_size(egui::vec2(88.0, 40.0))).clicked() { *selected = Some(source); }
                                    }
                                });
                                ui.add_space(12.0);ui.label(locale.text("开始共享时，由系统选择具体屏幕或窗口。","Choose the screen or window in the system picker when sharing starts."));
                            },
                            SourceCatalog::Ready(choices)=>{
                                for source in choices {
                                    let label=source.label();
                                    if v2::source_row(ui,&label,selected.as_ref()==Some(source),editable).clicked(){*selected=Some(source.clone());}
                                }
                                if choices.is_empty(){ui.label(locale.text("没有可共享的屏幕","No screens available"));}
                                else if selected.is_some() && snapshot.sources.selected(selected.as_ref()).is_none(){v2::notice(ui,StatusTone::Warning,locale.text("来源已变化，请重新选择。","The source changed. Select again."));}
                            },
                            SourceCatalog::Failed(_)=>v2::notice(ui,StatusTone::Error,locale.text("来源不可用，请刷新后重试。","Sources unavailable. Refresh and retry.")),
                        }
                        if matches!(snapshot.sources,SourceCatalog::Ready(_)|SourceCatalog::Failed(_)) && v2::action(ui,locale.text("刷新来源","Refresh sources"),egui::vec2(120.0,40.0),true).clicked(){command.get_or_insert(UserCommand::RefreshCaptureSources);}
                    });
                });
                ui.add_space(16.0);
                v2::settings_row(ui,locale.system_audio(),|ui| {ui.add_enabled(editable,egui::Checkbox::new(audio,"")).on_hover_text(locale.system_audio_hint());});
                ui.label(egui::RichText::new(locale.text("系统声音包含设备输出，不限于所选窗口。","System sound includes device output, not only the selected window.")).small().color(p.muted));
                if editable&&!snapshot.has_mesh_session(){v2::notice(ui,StatusTone::Neutral,locale.connect_first());}
                if matches!(snapshot.media,MediaState::PreparingView{..}|MediaState::Viewing{..}|MediaState::StoppingView{..}) {v2::notice(ui,StatusTone::Neutral,locale.text("停止观看后可开始共享。","Stop watching before sharing."));}
                if snapshot.last_error.is_some(){v2::notice(ui,StatusTone::Error,locale.operation_failed());}
            });
    });
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_window_choice_survives_rendering() {
        let mut selected = Some(CaptureSource::PortalWindow);
        let mut audio = false;
        let snapshot = AppSnapshot {
            sources: SourceCatalog::Portal,
            ..AppSnapshot::default()
        };
        let context = egui::Context::default();
        let frame = context.run_ui(egui::RawInput::default(), |ui| {
            show(ui, Locale::English, &snapshot, &mut audio, &mut selected);
        });
        frame.drop_without_applying_deltas();
        assert_eq!(selected, Some(CaptureSource::PortalWindow));
    }

    #[test]
    fn failed_source_catalog_clears_selection_before_refresh() {
        let source = CaptureSource::Display {
            id: "x11:0".into(),
            name: "DP-1".into(),
            width: 1920,
            height: 1080,
        };
        let mut selected = Some(source.clone());
        let mut audio = false;
        let snapshot = AppSnapshot {
            sources: SourceCatalog::Failed("Source changed. Refresh and select again.".into()),
            ..AppSnapshot::default()
        };
        let context = egui::Context::default();
        let frame = context.run_ui(egui::RawInput::default(), |ui| {
            show(ui, Locale::English, &snapshot, &mut audio, &mut selected);
        });
        frame.drop_without_applying_deltas();
        assert!(selected.is_none());
        assert!(
            SourceCatalog::Ready(vec![source])
                .selected(selected.as_ref())
                .is_none()
        );
    }
}

#[cfg(test)]
mod layout_checks {
    use super::*;
    fn render(
        ctx: &egui::Context,
        snapshot: &AppSnapshot,
        selected: &mut Option<CaptureSource>,
        events: Vec<egui::Event>,
    ) -> (Option<UserCommand>, Option<egui::Rect>) {
        let mut command = None;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(380.0, 420.0),
                )),
                events,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    command = show(ui, Locale::English, snapshot, &mut false, selected);
                });
            },
        );
        let label = if snapshot.has_mesh_session() {
            "Start sharing"
        } else {
            "Turn on Nearby"
        };
        let rect = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => Some(
                text.galley
                    .rect
                    .translate(text.pos.to_vec2())
                    .intersect(shape.clip_rect),
            ),
            _ => None,
        });
        output.drop_without_applying_deltas();
        (command, rect)
    }
    fn click(
        ctx: &egui::Context,
        s: &AppSnapshot,
        selected: &mut Option<CaptureSource>,
        pos: egui::Pos2,
    ) -> Option<UserCommand> {
        let event = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        render(
            ctx,
            s,
            selected,
            vec![egui::Event::PointerMoved(pos), event(true)],
        );
        render(ctx, s, selected, vec![event(false)]).0
    }
    #[test]
    fn explicit_nearby_action_wins_over_automatic_source_refresh() {
        let ctx = egui::Context::default();
        v2::install(&ctx);
        let s = AppSnapshot::default();
        let mut selected = None;
        let (_, rect) = render(&ctx, &s, &mut selected, vec![]);
        assert!(matches!(
            click(&ctx, &s, &mut selected, rect.unwrap().center()),
            Some(UserCommand::StartDiscovery)
        ));
    }
    #[test]
    fn long_source_list_keeps_start_visible_and_clickable() {
        let ctx = egui::Context::default();
        v2::install(&ctx);
        let choices = (0..40)
            .map(|i| CaptureSource::Display {
                id: format!("x11:{i}"),
                name: format!("Display {i}"),
                width: 1920,
                height: 1080,
            })
            .collect::<Vec<_>>();
        let expected = choices[0].clone();
        let mut selected = Some(expected.clone());
        let mut s = AppSnapshot {
            sources: SourceCatalog::Ready(choices),
            ..Default::default()
        };
        s.inbound_session_count = 1;
        let (_, rect) = render(&ctx, &s, &mut selected, vec![]);
        let rect = rect.unwrap();
        assert!(rect.is_positive() && rect.bottom() <= 420.0);
        assert!(
            matches!(click(&ctx,&s,&mut selected,rect.center()),Some(UserCommand::StartScreenShare{source,..}) if source==expected)
        );
    }
}
