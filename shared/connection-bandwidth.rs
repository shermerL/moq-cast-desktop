//! Observe existing connections without changing media or dial policy.

use moq_tokio::{Connection, moq_net};
use moqcast_lan_state::bandwidth::{Observation, Sample};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

// Listener-local inbound IDs and outbound generations can restart after Nearby Off.
static NEXT_OBSERVATION: AtomicU64 = AtomicU64::new(1);

fn read(
    send: Option<&moq_net::bandwidth::Consumer>,
    recv: Option<&moq_net::bandwidth::Consumer>,
) -> Sample {
    Sample {
        send_bps: send
            .and_then(|value| value.peek())
            .map(|rate| rate.as_bps()),
        recv_bps: recv
            .and_then(|value| value.peek())
            .map(|rate| rate.as_bps()),
    }
}

pub(crate) async fn outbound(
    connection: &Connection,
    observation: Observation,
    peer_id: &str,
    generation: u64,
) -> moq_tokio::Result<()> {
    let observation_id = NEXT_OBSERVATION.fetch_add(1, Ordering::Relaxed);
    let monitor = connection.monitor();
    let mut status = connection.clone();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut epoch = connection.epoch();
    observation.reset(epoch);
    loop {
        tokio::select! {
            biased;
            result = connection.closed() => return result,
            changed = status.status() => {
                epoch = connection.epoch();
                observation.reset(epoch);
                if changed.is_err() {
                    return connection.closed().await;
                }
            }
            _ = tick.tick() => {
                let current = connection.epoch();
                if current != epoch {
                    epoch = current;
                    observation.reset(epoch);
                }
                // Read the current session, not the persistent forwarding cache:
                // a handover publishes its epoch before forwarding new estimates.
                let sample = monitor.stats().map(|stats| Sample {
                    send_bps: stats.estimated_send_rate.map(|rate| rate.as_bps()),
                    recv_bps: stats.estimated_recv_rate.map(|rate| rate.as_bps()),
                }).unwrap_or_default();
                let sample = if connection.connected() && connection.epoch() == epoch {
                    sample
                } else {
                    Sample::default()
                };
                observation.update(epoch, sample);
                tracing::debug!(stage = "bandwidth", direction = "outbound", observation_id, peer_id,
                    generation, epoch, send_bps = ?sample.send_bps, recv_bps = ?sample.recv_bps,
                    "connection bandwidth estimate");
            }
        }
    }
}

pub(crate) async fn inbound(session: &moq_net::Session, inbound_id: u64) -> moq_net::Error {
    let observation_id = NEXT_OBSERVATION.fetch_add(1, Ordering::Relaxed);
    // An inbound ID is local to this listener; it is not a discovered peer ID.
    let mut estimates = None;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            reason = session.closed() => return reason,
            _ = tick.tick() => {
                if !tracing::enabled!(tracing::Level::DEBUG) {
                    estimates = None;
                    continue;
                }
                let (send, recv) = estimates.get_or_insert_with(|| (session.send_bandwidth(), session.recv_bandwidth()));
                let sample = read(send.as_ref(), recv.as_ref());
                tracing::debug!(stage = "bandwidth", direction = "inbound", observation_id, inbound_id,
                    send_bps = ?sample.send_bps, recv_bps = ?sample.recv_bps,
                    "unattributed connection bandwidth estimate");
            }
        }
    }
}
