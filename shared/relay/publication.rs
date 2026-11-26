//! Report native video readiness from the publisher's own catalog. This opens
//! no network connection and says nothing about visibility at a remote relay.
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(crate) async fn run<F: std::future::Future>(
    video: F,
    catalog: &moq_mux::catalog::Producer,
    ready: Option<impl std::future::Future<Output = ()>>,
) -> F::Output {
    let Some(ready) = ready else {
        return video.await;
    };
    let signal = async {
        if let Ok(mut local) = catalog.consume() {
            while let Ok(Some(snapshot)) = local.next().await {
                if !snapshot.video.renditions.is_empty() {
                    ready.await;
                    return;
                }
            }
        }
    };
    tokio::pin!(video);
    tokio::select! {
        biased;
        result = &mut video => result,
        () = signal => video.await,
    }
}

/// A finished or unavailable audio source cannot restart or end the video future.
pub(crate) async fn with_audio<V: std::future::Future, A: std::future::Future<Output = ()>>(
    video: V,
    enabled: bool,
    start_audio: impl FnOnce() -> A,
) -> V::Output {
    if !enabled {
        return video.await;
    }
    let audio = start_audio();
    tokio::pin!(video);
    tokio::select! {
        biased;
        result = &mut video => result,
        () = audio => video.await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn optional_audio_is_opened_only_on_request_and_never_owns_video() {
        for enabled in [false, true] {
            let mut broadcast = moq_tokio::moq_net::broadcast::Info::new().produce();
            let catalog =
                moq_mux::catalog::Producer::<()>::new(&mut broadcast, Default::default()).unwrap();
            let opened = std::cell::Cell::new(0);
            let video_starts = std::cell::Cell::new(0);
            let (ready, started) = tokio::sync::oneshot::channel();
            let (fail, disconnect) = tokio::sync::oneshot::channel();
            let (released, ended) = tokio::sync::oneshot::channel();
            let video = async {
                video_starts.set(video_starts.get() + 1);
                if enabled {
                    started.await.unwrap();
                    assert_eq!(catalog.snapshot().audio.renditions.len(), 1);
                    fail.send(()).unwrap();
                    ended.await.unwrap();
                }
                assert!(catalog.snapshot().audio.renditions.is_empty());
                17
            };
            let result = with_audio(video, enabled, || {
                opened.set(opened.get() + 1);
                let track = broadcast
                    .create_track("fixture.opus", catalog.track_info(1))
                    .unwrap();
                let rendition = catalog
                    .audio(
                        track,
                        moq_mux::container::legacy::Wire(moq_mux::container::Kind::Audio),
                        hang::catalog::AudioConfig::new(hang::catalog::AudioCodec::Opus, 48_000, 2),
                    )
                    .unwrap();
                async move {
                    ready.send(()).unwrap();
                    disconnect.await.unwrap();
                    drop(rendition);
                    released.send(()).unwrap();
                }
            })
            .await;
            assert_eq!(result, 17);
            assert_eq!(video_starts.get(), 1);
            assert_eq!(opened.get(), usize::from(enabled));
        }
    }
}
