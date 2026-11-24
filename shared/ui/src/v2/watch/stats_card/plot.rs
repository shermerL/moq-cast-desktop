//! Fixed 60-second charts. Missing intervals and decoder changes break both series.
use super::{Labels, Palette};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use moqcast_playback_stats::{MarkKind, Sample, StatsSnapshot};

#[derive(Clone, Copy)]
pub(super) struct Colors {
    pub decoded: Color32,
    pub ui: Color32,
    rebuild: Color32,
}

impl Colors {
    pub(super) fn new(dark: bool) -> Self {
        if dark {
            Self {
                decoded: Color32::from_rgb(96, 165, 250),
                ui: Color32::from_rgb(245, 158, 11),
                rebuild: Color32::from_rgb(167, 139, 250),
            }
        } else {
            Self {
                decoded: Color32::from_rgb(37, 99, 235),
                ui: Color32::from_rgb(217, 119, 6),
                rebuild: Color32::from_rgb(124, 58, 237),
            }
        }
    }
}

fn x(rect: Rect, now: f64, at: f64) -> f32 {
    rect.right() + ((at - now) / 60.0) as f32 * rect.width()
}

fn gap(paint: &Painter, plot: Rect, left: f32, right: f32, p: Palette, t: Labels) {
    let r = Rect::from_min_max(pos2(left, plot.top()), pos2(right, plot.bottom())).intersect(plot);
    if r.width() < 1.0 {
        return;
    }
    let clipped = paint.with_clip_rect(r);
    clipped.rect_filled(r, 0, p.canvas);
    let mut from = r.left() - r.height();
    while from < r.right() {
        clipped.line_segment(
            [pos2(from, r.bottom()), pos2(from + r.height(), r.top())],
            Stroke::new(0.7, p.border),
        );
        from += 10.0;
    }
    if r.width() > 42.0 {
        clipped.text(
            r.center_top() + vec2(0.0, 2.0),
            Align2::CENTER_TOP,
            t.get("缺测", "Missing"),
            FontId::proportional(10.0),
            p.muted,
        );
    }
}

pub(super) fn background(paint: &Painter, r: Rect, data: &StatsSnapshot, p: Palette, t: Labels) {
    // Draw the complement of observed intervals. This includes unsampled time before opt-in.
    let mut previous = r.left();
    for s in &data.samples {
        if s.decoded_fps.is_some() || s.ui_fps.is_some() {
            let start = x(r, data.now, s.start);
            gap(paint, r, previous, start, p, t);
            previous = x(r, data.now, s.at).max(previous);
        }
    }
    gap(paint, r, previous, r.right(), p, t);
    for seconds in [0.0, 15.0, 30.0, 45.0, 60.0] {
        let at = x(r, data.now, data.now - seconds);
        paint.line_segment(
            [pos2(at, r.top()), pos2(at, r.bottom())],
            Stroke::new(0.7, p.border),
        );
    }
}

fn segments(
    samples: &[Sample],
    value: impl Fn(&Sample) -> Option<f64>,
    mut draw: impl FnMut(&[(f64, f64)]),
) {
    let mut points = Vec::with_capacity(samples.len());
    let mut previous: Option<&Sample> = None;
    for sample in samples {
        let active = value(sample);
        if active.is_none()
            || previous.is_some_and(|last| {
                last.codec_generation != sample.codec_generation || last.at != sample.start
            })
        {
            if !points.is_empty() {
                draw(&points);
            }
            points.clear();
        }
        if let Some(v) = active {
            points.push((sample.at, v));
        }
        previous = Some(sample);
    }
    if !points.is_empty() {
        draw(&points);
    }
}

pub(super) fn frame_rates(
    paint: &Painter,
    r: Rect,
    data: &StatsSnapshot,
    decoded: bool,
    ui: bool,
    c: Colors,
    p: Palette,
) {
    let peak = data
        .samples
        .iter()
        .flat_map(|s| [s.decoded_fps, s.ui_fps])
        .flatten()
        .fold(0.0_f64, f64::max);
    let max = ((peak / 15.0).ceil() * 15.0).max(30.0);
    let base = r.bottom() - 4.0;
    let y = |v: f64| base - (v / max) as f32 * (base - r.top() - 3.0);
    for val in [0.0, max / 2.0, max] {
        let at = y(val);
        paint.line_segment(
            [pos2(r.left(), at), pos2(r.right(), at)],
            Stroke::new(0.7, p.border),
        );
        paint.text(
            pos2(r.left() - 3.0, at),
            Align2::RIGHT_CENTER,
            val.to_string(),
            FontId::monospace(9.5),
            p.muted,
        );
    }
    let clipped = paint.with_clip_rect(r);
    // Paint the thinner dashed decoder curve last so coincident real values stay legible.
    for (is_decoded, visible) in [(false, ui), (true, decoded)] {
        if !visible {
            continue;
        }
        let color = if is_decoded { c.decoded } else { c.ui };
        let stroke = Stroke::new(if is_decoded { 1.7 } else { 2.5 }, color);
        segments(
            &data.samples,
            |s| if is_decoded { s.decoded_fps } else { s.ui_fps },
            |series| {
                let pts: Vec<Pos2> = series
                    .iter()
                    .map(|(at, v)| pos2(x(r, data.now, *at), y(*v)))
                    .collect();
                if pts.len() >= 2 {
                    if is_decoded {
                        clipped.extend(Shape::dashed_line(&pts, stroke, 4.0, 3.0));
                    } else {
                        clipped.add(Shape::line(pts.clone(), stroke));
                    }
                }
                for (pos, (_, v)) in pts.iter().zip(series) {
                    if pts.len() == 1 || *v == 0.0 {
                        clipped.circle_filled(*pos, if is_decoded { 1.6 } else { 2.5 }, color);
                    }
                }
            },
        );
    }
}

pub(super) fn markers(
    paint: &Painter,
    r: Rect,
    data: &StatsSnapshot,
    cursor: Option<f64>,
    colors: Colors,
    p: Palette,
) {
    let clipped = paint.with_clip_rect(r);
    for m in &data.marks {
        let px = x(r, data.now, m.at);
        let color = match m.kind {
            MarkKind::User => p.accent,
            MarkKind::DecoderChanged => colors.rebuild,
        };
        clipped.extend(Shape::dashed_line(
            &[pos2(px, r.top()), pos2(px, r.bottom())],
            Stroke::new(1.25, color),
            4.0,
            3.0,
        ));
    }
    if let Some(at) = cursor {
        let px = x(r, data.now, at);
        clipped.line_segment(
            [pos2(px, r.top()), pos2(px, r.bottom())],
            Stroke::new(1.25, p.accent),
        );
    }
}

pub(super) fn time_labels(paint: &Painter, r: Rect, p: Palette) {
    for (sec, label, align) in [
        (60.0, "−60s", Align2::LEFT_TOP),
        (45.0, "−45", Align2::CENTER_TOP),
        (30.0, "−30", Align2::CENTER_TOP),
        (15.0, "−15", Align2::CENTER_TOP),
        (0.0, "0s", Align2::RIGHT_TOP),
    ] {
        paint.text(
            pos2(x(r, 0.0, -sec), r.bottom() + 2.0),
            align,
            label,
            FontId::monospace(10.0),
            p.muted,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rates_break_on_unmeasured_time_and_decoder_replacement_but_retain_observed_zero() {
        let make = |start, at, rate, generation| Sample {
            start,
            at,
            decoded_fps: rate,
            ui_fps: rate,
            codec_generation: Some(generation),
        };
        let history = [
            make(1.0, 2.0, Some(30.0), 1),
            make(2.0, 3.0, Some(0.0), 1),
            make(3.0, 8.0, None, 1),
            make(8.0, 9.0, Some(30.0), 1),
            make(9.0, 10.0, Some(31.0), 2),
            make(15.0, 16.0, Some(28.0), 2),
        ];
        let mut observed = Vec::new();
        segments(&history, |s| s.ui_fps, |path| observed.push(path.to_vec()));
        assert_eq!(
            observed,
            vec![
                vec![(2.0, 30.0), (3.0, 0.0)],
                vec![(9.0, 30.0)],
                vec![(10.0, 31.0)],
                vec![(16.0, 28.0)]
            ]
        );
        assert_eq!(
            super::super::sample_at(&history, 6.0).unwrap().decoded_fps,
            None
        );
        assert!(super::super::sample_at(&history, 12.0).is_none());
        assert!(super::super::sample_at(&history, 0.5).is_none());
        assert_eq!(
            super::super::sample_at(&history, 3.0).unwrap().ui_fps,
            Some(0.0)
        );
    }
}
