use super::*;

fn secs(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn with_decoder() -> (History, DecoderCounter) {
    let counts = Arc::<Counts>::default();
    let mut h = History {
        decoder: Some(Decoder {
            generation: 1,
            counts: counts.clone(),
            last_sequence: None,
            accepted: 0,
        }),
        ..History::default()
    };
    h.set_enabled(Duration::ZERO, true);
    (h, DecoderCounter(counts))
}

#[test]
fn interval_uses_actual_elapsed_and_independent_delivery() {
    let (mut h, reader) = with_decoder();
    for _ in 0..33 {
        reader.decoded();
    }
    h.tick(secs(995), false);
    assert!(h.samples.is_empty());
    h.tick(secs(1100), false);
    let point = h.samples[0];
    assert!((point.decoded_fps.unwrap() - 30.0).abs() < 1e-10);
    assert_eq!(point.ui_fps, Some(0.0));
    h.decoder.as_mut().unwrap().accepted = 9;
    h.tick(secs(2000), false);
    assert_eq!(h.samples[1].decoded_fps, Some(0.0));
    assert_eq!(h.samples[1].ui_fps, Some(10.0));
    assert_eq!(h.missed, 0);
}

#[test]
fn missed_deadlines_leave_one_gap_without_catch_up() {
    let (mut h, _) = with_decoder();
    h.tick(secs(1000), false);
    h.tick(secs(86_400_200), false);
    assert_eq!(h.samples.len(), 1);
    let gap = h.samples[0];
    assert_eq!((gap.decoded_fps, gap.ui_fps), (None, None));
    assert!((gap.at - gap.start - 60.0).abs() < 1e-9);
    assert_eq!(h.missed, 86_398);
    h.tick(secs(86_401_000), false);
    assert_eq!(h.samples.back().unwrap().ui_fps, Some(0.0));
}

#[test]
fn age_and_capacity_bound_history_and_marker_spam() {
    let (mut h, _) = with_decoder();
    for second in 1..=200 {
        h.tick(Duration::from_secs(second), false);
    }
    assert_eq!(h.samples.len(), 60);
    assert_eq!(h.samples[0].start, 140.0);
    for _ in 0..500 {
        h.mark(Duration::from_secs(200), MarkKind::User);
    }
    assert_eq!(h.marks.len(), EVENTS);
    assert_eq!(h.overwritten_marks, 436);
    h.prune(Duration::from_secs(261));
    assert!(h.marks.is_empty() && h.samples.is_empty());
}

#[test]
fn inactive_stale_and_duplicate_callbacks_cannot_pollute_new_measurements() {
    let owner = PlaybackStats::default();
    let old = owner.begin(3);
    let old_reader = old.decoder(1);
    old_reader.decoded();
    assert!(!owner.is_sampling(3));
    assert_eq!(old_reader.0.decoded.load(Relaxed), 0);
    assert!(!old.session.state.lock().unwrap().worker);
    owner.set_sampling(3, true);
    owner.mark(3);
    let watcher = owner.begin(4);
    let fresh = owner.snapshot(4).unwrap();
    assert!(!fresh.enabled && fresh.marks.is_empty() && fresh.samples.is_empty());
    assert!(!old_reader.0.enabled.load(Relaxed));
    let first = watcher.decoder(5);
    owner.accepted(4, 5, 1);
    owner.set_sampling(4, true);
    owner.accepted(4, 5, 1);
    owner.accepted(4, 5, 3);
    owner.accepted(4, 5, 2);
    owner.accepted(3, 5, 4);
    assert_eq!(
        watcher
            .session
            .state
            .lock()
            .unwrap()
            .decoder
            .as_ref()
            .unwrap()
            .accepted,
        1
    );
    watcher.clear_decoder();
    let second = watcher.decoder(6);
    first.decoded();
    old_reader.decoded();
    second.decoded();
    owner.accepted(4, 5, 9);
    owner.mark(3);
    owner.mark(4);
    drop(old);
    let h = watcher.session.state.lock().unwrap();
    let d = h.decoder.as_ref().unwrap();
    assert_eq!((d.counts.decoded.load(Relaxed), d.accepted), (1, 0));
    assert_eq!(
        h.marks.iter().filter(|m| m.kind == MarkKind::User).count(),
        1
    );
    assert!(h.samples.back().unwrap().ui_fps.is_none());
    drop(h);
    drop(watcher);
    assert!(owner.snapshot(4).is_none());
    assert!(!second.0.enabled.load(Relaxed));
}

#[test]
fn unobserved_decoder_and_counter_reset_are_unavailable() {
    let mut h = History::default();
    h.set_enabled(Duration::ZERO, true);
    h.tick(PERIOD, false);
    assert_eq!(h.samples[0].ui_fps, None);
    let (mut h, _) = with_decoder();
    h.baseline = Some((Duration::ZERO, 99, 4));
    h.tick(PERIOD, false);
    assert_eq!(h.samples[0].decoded_fps, None);
    assert_eq!(h.samples[0].ui_fps, None);
    h.tick(PERIOD * 2, false);
    assert_eq!(h.samples[1].decoded_fps, Some(0.0));
}

#[test]
fn verified_interrupt_breaks_an_otherwise_on_time_interval() {
    let (mut h, _) = with_decoder();
    h.tick(PERIOD, true);
    assert_eq!(h.samples[0].decoded_fps, None);
    assert_eq!(h.missed, 0); // the deadline was observed, but the interval is incomplete
    h.tick(PERIOD * 2, false);
    assert_eq!(h.samples[1].ui_fps, Some(0.0));
}

#[test]
fn stopping_sampling_freezes_marks_and_window_until_explicit_restart() {
    let owner = PlaybackStats::default();
    let session = owner.begin(7);
    session.decoder(1);
    let mut h = session.session.state.lock().unwrap();
    h.set_enabled(Duration::ZERO, true);
    h.tick(PERIOD, false);
    h.mark(PERIOD, MarkKind::User);
    h.set_enabled(PERIOD * 2, false);
    drop(h);
    let snap = owner.snapshot(7).unwrap();
    assert_eq!(snap.now, 2.0);
    assert_eq!(snap.samples.len(), 1);
    assert_eq!(snap.marks.len(), 1);
    let mut h = session.session.state.lock().unwrap();
    h.set_enabled(PERIOD * 100, true);
    assert!(h.marks.is_empty() && h.samples.is_empty() && h.frozen_at.is_none());
}
