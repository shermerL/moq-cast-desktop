use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use mdns_sd::{DaemonEvent, ServiceDaemon, ServiceEvent};

use crate::registry::{Lifecycle, Registry, RegistryChange, ResolvedRecord, Snapshot};

const SERVICE_TYPE: &str = "_moq._udp.local.";
// ResolvedService omits record TTLs. mdns-sd still emits ServiceRemoved from its
// cache; this matching RFC 6762 PTR/TXT default is an additional stale-entry cap.
const PRESENCE_LEASE: Duration = Duration::from_secs(75 * 60);
const EVENT_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunExit {
    StopRequested,
    DiscoveryFailed,
}

#[derive(Clone)]
pub(crate) struct DiscoveryView {
    latest: Arc<Mutex<Snapshot>>,
}

impl DiscoveryView {
    #[cfg(test)]
    pub(crate) fn from_snapshot(snapshot: Snapshot) -> Self {
        Self {
            latest: Arc::new(Mutex::new(snapshot)),
        }
    }

    pub(crate) fn latest(&self) -> Snapshot {
        self.latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

pub(crate) struct DiscoveryOwner {
    stop: mpsc::SyncSender<()>,
    worker: Option<thread::JoinHandle<()>>,
}

impl DiscoveryOwner {
    pub(crate) fn start(wake: impl Fn() + Send + 'static) -> (Self, DiscoveryView) {
        let latest = Arc::new(Mutex::new(Snapshot::starting()));
        let worker_latest = latest.clone();
        let (stop, stop_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || run(worker_latest, stop_rx, wake));

        (
            Self {
                stop,
                worker: Some(worker),
            },
            DiscoveryView { latest },
        )
    }

    pub(crate) fn stop(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        let _ = self.stop.try_send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for DiscoveryOwner {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

fn run(latest: Arc<Mutex<Snapshot>>, stop: mpsc::Receiver<()>, wake: impl Fn() + Send + 'static) {
    let mut registry = Registry::default();
    let mut revision = 0;
    let daemon = match ServiceDaemon::new() {
        Ok(daemon) => daemon,
        Err(_) => {
            publish(&latest, &registry, &mut revision, Lifecycle::Failed, &wake);
            return;
        }
    };
    let monitor = daemon.monitor().ok();
    let events = match daemon.browse(SERVICE_TYPE) {
        Ok(events) => events,
        Err(_) => {
            let _ = daemon.shutdown();
            publish(&latest, &registry, &mut revision, Lifecycle::Failed, &wake);
            return;
        }
    };

    let mut lifecycle = Lifecycle::Browsing;
    publish(&latest, &registry, &mut revision, lifecycle, &wake);

    let exit = loop {
        match stop.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break RunExit::StopRequested,
            Err(mpsc::TryRecvError::Empty) => {}
        }

        if let Some(monitor) = &monitor {
            while let Ok(event) = monitor.try_recv() {
                if matches!(event, DaemonEvent::Error(_)) && lifecycle != Lifecycle::Degraded {
                    lifecycle = Lifecycle::Degraded;
                    publish(&latest, &registry, &mut revision, lifecycle, &wake);
                }
            }
        }

        let changed = match events.recv_timeout(EVENT_POLL_INTERVAL) {
            Ok(ServiceEvent::ServiceFound(_, fullname)) => {
                registry.found(&fullname, Instant::now(), PRESENCE_LEASE)
            }
            Ok(ServiceEvent::ServiceResolved(service)) => {
                let record = ResolvedRecord {
                    fullname: service.get_fullname().to_owned(),
                    port: service.get_port(),
                    fingerprint: service.get_property_val_str("fp").map(str::to_owned),
                    credential: service.get_property_val_str("n").map(str::to_owned),
                    has_shared_secret: service.get_properties().get("a").is_some(),
                    addresses: service.get_addresses_v4().into_iter().collect(),
                };
                registry.resolved_record(record, Instant::now(), PRESENCE_LEASE)
            }
            Ok(ServiceEvent::ServiceRemoved(_, fullname)) => registry.removed(&fullname),
            Ok(ServiceEvent::SearchStarted(_)) => {
                lifecycle = Lifecycle::Browsing;
                RegistryChange::Updated
            }
            Ok(ServiceEvent::SearchStopped(_)) => break RunExit::DiscoveryFailed,
            Ok(_) | Err(mdns_sd::RecvTimeoutError::Timeout) => registry.expire(Instant::now()),
            Err(mdns_sd::RecvTimeoutError::Disconnected) => break RunExit::DiscoveryFailed,
        };

        if changed != RegistryChange::Unchanged {
            publish(&latest, &registry, &mut revision, lifecycle, &wake);
        }
    };

    if exit == RunExit::StopRequested {
        publish(
            &latest,
            &registry,
            &mut revision,
            Lifecycle::Stopping,
            &wake,
        );
    }
    let _ = daemon.stop_browse(SERVICE_TYPE);
    let _ = daemon.shutdown();
    finish_run(&latest, &mut registry, &mut revision, exit, &wake);
}

fn finish_run(
    latest: &Mutex<Snapshot>,
    registry: &mut Registry,
    revision: &mut u64,
    exit: RunExit,
    wake: &impl Fn(),
) {
    registry.clear();
    let lifecycle = match exit {
        RunExit::StopRequested => Lifecycle::Stopped,
        RunExit::DiscoveryFailed => Lifecycle::Failed,
    };
    publish(latest, registry, revision, lifecycle, wake);
}

fn publish(
    latest: &Mutex<Snapshot>,
    registry: &Registry,
    revision: &mut u64,
    lifecycle: Lifecycle,
    wake: &impl Fn(),
) {
    *revision = revision.wrapping_add(1);
    *latest
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = registry.snapshot(*revision, lifecycle);
    wake();
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn published_snapshot_is_latest_only() {
        let latest = Mutex::new(Snapshot::starting());
        let registry = Registry::default();
        let mut revision = 0;
        let wakes = Cell::new(0);
        let wake = || wakes.set(wakes.get() + 1);

        publish(
            &latest,
            &registry,
            &mut revision,
            Lifecycle::Browsing,
            &wake,
        );
        publish(
            &latest,
            &registry,
            &mut revision,
            Lifecycle::Degraded,
            &wake,
        );
        let snapshot = latest.into_inner().expect("snapshot lock");

        assert_eq!(wakes.get(), 2);
        assert_eq!(snapshot.revision, 2);
        assert_eq!(snapshot.lifecycle, Lifecycle::Degraded);
    }

    #[test]
    fn discovery_failure_clears_devices_and_remains_failed() {
        let latest = Mutex::new(Snapshot::starting());
        let mut registry = Registry::default();
        let mut revision = 0;
        registry.resolved(
            "peer_a._moq._udp.local.",
            Instant::now(),
            Duration::from_secs(30),
        );

        finish_run(
            &latest,
            &mut registry,
            &mut revision,
            RunExit::DiscoveryFailed,
            &|| {},
        );

        let snapshot = latest.into_inner().expect("snapshot lock");
        assert_eq!(snapshot.lifecycle, Lifecycle::Failed);
        assert!(snapshot.devices.is_empty());
    }

    #[test]
    fn requested_stop_reaches_stopped_with_no_devices() {
        let latest = Mutex::new(Snapshot::starting());
        let mut registry = Registry::default();
        let mut revision = 0;
        registry.resolved(
            "peer_a._moq._udp.local.",
            Instant::now(),
            Duration::from_secs(30),
        );

        finish_run(
            &latest,
            &mut registry,
            &mut revision,
            RunExit::StopRequested,
            &|| {},
        );

        let snapshot = latest.into_inner().expect("snapshot lock");
        assert_eq!(snapshot.lifecycle, Lifecycle::Stopped);
        assert!(snapshot.devices.is_empty());
    }
}
