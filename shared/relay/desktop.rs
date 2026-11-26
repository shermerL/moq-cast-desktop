//! Relay state shared by the Windows and Linux supervisors.
include!("lifecycle.rs");
include!("transport.rs");

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MediaRoute {
    #[default]
    Nearby,
    Watching(Selection, u64),
    Publishing {
        connection: Generation,
        path: String,
    },
}

impl MediaRoute {
    pub(crate) fn is_relay(&self) -> bool {
        !matches!(self, Self::Nearby)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Connect(Config),
    Disconnect(Generation),
    Watch(Selection),
    Stop(MediaRoute),
}

#[derive(Default)]
pub(crate) struct Link {
    owner: Option<Owner>,
    request: Option<Request>,
}

pub(crate) enum Update {
    Directory,
    Broadcast(Result<moq_net::broadcast::Consumer, String>),
    Closed(String),
}

impl Link {
    pub(crate) fn connect(&mut self, state: &mut Snapshot, config: Config) {
        if self.owner.is_some() {
            return;
        }
        state.server = config.summary();
        state.error = None;
        state.entries.clear();
        state.lifecycle.begin(Phase::Connecting);
        match Owner::start(config) {
            Ok(owner) => self.owner = Some(owner),
            Err(error) => {
                state.lifecycle.begin(Phase::Failed);
                state.error = Some(error);
            }
        }
    }

    pub(crate) fn publish_origin(
        &self,
        state: &Snapshot,
        connection: Generation,
    ) -> Option<moq_net::origin::Producer> {
        if state.lifecycle.phase() != Phase::Connected || state.lifecycle.generation() != connection
        {
            return None;
        }
        Some(self.owner.as_ref()?.publish_origin())
    }

    pub(crate) fn request(&mut self, state: &Snapshot, selection: &Selection) -> bool {
        if !state.contains(selection) || self.request.is_some() {
            return false;
        }
        self.request = self.owner.as_ref().and_then(|o| o.request(selection));
        self.request.is_some()
    }

    pub(crate) fn cancel_request(&mut self) {
        self.request = None;
    }

    pub(crate) async fn next(&mut self, state: &mut Snapshot) -> Update {
        let Some(owner) = &mut self.owner else {
            return std::future::pending().await;
        };
        tokio::select! {
            event = owner.recv() => match event {
                Event::Connected => {
                    let generation = state.lifecycle.generation();
                    state.lifecycle.apply(generation, Phase::Connected);
                    tracing::info!(relay_generation = generation.value(), "manual relay connected");
                    state.version = owner.version();
                    Update::Directory
                }
                Event::Announcement(event) => {
                    owner.apply(state, event);
                    Update::Directory
                }
                Event::Closed(error) => Update::Closed(error),
            },
            result = async { self.request.as_mut().unwrap().await }, if self.request.is_some() => {
                self.request = None;
                Update::Broadcast(result.map_err(|_| "The selected Relay broadcast is no longer available.".to_owned()))
            }
        }
    }

    pub(crate) async fn close(&mut self, state: &mut Snapshot, error: Option<String>) {
        self.cancel_request();
        let Some(owner) = self.owner.take() else {
            return;
        };
        state.lifecycle.begin(Phase::Disconnecting);
        let closed = owner.close().await;
        state.error = match (error, closed) {
            (Some(reason), Err(cleanup)) => Some(format!("{reason}; {cleanup}")),
            (reason, Ok(())) => reason,
            (None, Err(cleanup)) => Some(cleanup),
        };
        state.entries.clear();
        state.version = None;
        state.lifecycle.begin(if state.error.is_some() {
            Phase::Failed
        } else {
            Phase::Disconnected
        });
    }
}

#[path = "controls.rs"]
pub(crate) mod controls;

#[cfg(test)]
#[path = "test_server.rs"]
pub(crate) mod test_server;
