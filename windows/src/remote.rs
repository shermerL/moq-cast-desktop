//! Remote screen announcement directory over the shared MoQ origin.

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
pub(crate) struct RemoteScreenView {
    pub(crate) peer_id: String,
    pub(crate) availability: ScreenAvailability,
}

pub(crate) struct Update {
    pub(crate) path: String,
    pub(crate) view: RemoteScreenView,
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
                let path = update.path.to_string();
                let Some(peer_id) = announcement_peer(&path, &local_peer_id).map(str::to_owned)
                else {
                    continue;
                };
                if events_tx
                    .send(Event {
                        path,
                        peer_id,
                        broadcast: update.broadcast,
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
            view: RemoteScreenView {
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
    crate::screen_path::peer_id(path).filter(|peer_id| *peer_id != local_peer_id)
}

#[cfg(test)]
mod tests {
    use super::announcement_peer;

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
}
