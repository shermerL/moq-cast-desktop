//! Serialized command processing for runtime-owned resources.

use tokio::sync::{mpsc, watch};

use crate::app::{AppSnapshot, UserCommand};

pub(super) async fn run(
    mut commands: mpsc::Receiver<UserCommand>,
    snapshots: watch::Sender<AppSnapshot>,
) {
    let mut state = AppSnapshot::default();

    while let Some(command) = commands.recv().await {
        let keep_running = match command {
            UserCommand::StartDiscovery => {
                state.start_discovery();
                true
            }
            UserCommand::StopDiscovery => {
                state.stop_discovery();
                true
            }
            UserCommand::ConnectPeer { .. } | UserCommand::Disconnect => {
                state.last_error = Some("Peer connections will be enabled in L2.".into());
                true
            }
            UserCommand::StartScreenShare | UserCommand::StopScreenShare => {
                state.last_error = Some("Screen publishing will be enabled in L3.".into());
                true
            }
            UserCommand::Shutdown => false,
        };

        if !keep_running {
            break;
        }
        snapshots.send_replace(state.clone());
    }

    tracing::info!("desktop runtime stopped");
}
