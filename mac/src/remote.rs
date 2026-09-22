//! Remote screen announcements received through the direct-session origin.

use std::collections::BTreeMap;

use moq_tokio::moq_net;
use tokio::{sync::mpsc, task::JoinHandle};

const EVENT_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ScreenAvailability {
    #[default]
    Unavailable,
    Available,
    Withdrawn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScreenView {
    pub(crate) peer_id: String,
    pub(crate) availability: ScreenAvailability,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Update {
    pub(crate) path: String,
    pub(crate) view: ScreenView,
}

struct Event {
    path: String,
    peer_id: String,
    broadcast: Option<moq_net::broadcast::Consumer>,
}

pub(crate) struct Directory {
    broadcasts: BTreeMap<String, moq_net::broadcast::Consumer>,
    events: mpsc::Receiver<Event>,
    task: JoinHandle<()>,
}

impl Directory {
    pub(crate) fn start(origin: moq_net::origin::Producer, local_peer_id: String) -> Self {
        let (events_tx, events) = mpsc::channel(EVENT_CAPACITY);
        let task = tokio::spawn(async move {
            let mut announcements = origin.consume().announced();
            while let Some(update) = announcements.next().await {
                let path = update.prefix.to_string();
                let Some(peer_id) = announcement_peer(&path, &local_peer_id).map(str::to_owned)
                else {
                    continue;
                };
                let broadcast = if update.kind.is_active() {
                    match origin.consume().request_broadcast(path.as_str()).await {
                        Ok(broadcast) => Some(broadcast),
                        Err(error) => {
                            tracing::warn!(%error, "remote screen announcement could not resolve");
                            continue;
                        }
                    }
                } else {
                    None
                };
                if events_tx
                    .send(Event {
                        path,
                        peer_id,
                        broadcast,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            broadcasts: BTreeMap::new(),
            events,
            task,
        }
    }

    pub(crate) async fn recv(&mut self) -> Option<Update> {
        let event = self.events.recv().await?;
        let availability = if let Some(broadcast) = event.broadcast {
            self.broadcasts.insert(event.path.clone(), broadcast);
            ScreenAvailability::Available
        } else {
            self.broadcasts.remove(&event.path);
            ScreenAvailability::Withdrawn
        };
        Some(Update {
            path: event.path,
            view: ScreenView {
                peer_id: event.peer_id,
                availability,
            },
        })
    }

    pub(crate) fn broadcast(&self, path: &str) -> Option<moq_net::broadcast::Consumer> {
        self.broadcasts.get(path).cloned()
    }

    pub(crate) async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

fn announcement_peer<'a>(path: &'a str, local_peer_id: &str) -> Option<&'a str> {
    crate::contract::screen_peer_id(path).filter(|peer_id| *peer_id != local_peer_id)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Directory, ScreenAvailability, announcement_peer};

    #[test]
    fn directory_accepts_only_canonical_remote_screen_paths() {
        assert_eq!(
            announcement_peer("moqcast.screen/peer", "local"),
            Some("peer")
        );
        assert_eq!(announcement_peer("moqcast.screen/local", "local"), None);
        assert_eq!(
            announcement_peer("moqcast.screen/peer/extra", "local"),
            None
        );
        assert_eq!(announcement_peer("other/peer", "local"), None);
    }

    #[tokio::test]
    async fn announced_broadcast_resolves_and_withdraws() {
        let origin = moq_tokio::origin::spawn();
        let mut directory = Directory::start(origin.clone(), "local".to_owned());
        let broadcast = origin
            .create_broadcast("moqcast.screen/remote")
            .expect("broadcast");
        broadcast
            .announce(moq_tokio::moq_net::origin::Route::default())
            .expect("announcement");

        let available = tokio::time::timeout(Duration::from_secs(3), directory.recv())
            .await
            .expect("announcement bounded")
            .expect("available");
        assert_eq!(available.view.availability, ScreenAvailability::Available);
        assert!(directory.broadcast(&available.path).is_some());

        broadcast.finish();
        let withdrawn = tokio::time::timeout(Duration::from_secs(3), directory.recv())
            .await
            .expect("withdrawal bounded")
            .expect("withdrawn");
        assert_eq!(withdrawn.view.availability, ScreenAvailability::Withdrawn);
        assert!(directory.broadcast(&withdrawn.path).is_none());
        directory.stop().await;
    }
}
