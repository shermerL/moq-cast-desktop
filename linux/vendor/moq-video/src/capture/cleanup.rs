//! Awaitable ownership of asynchronous capture cleanup after child cancellation.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::{Mutex as AsyncMutex, oneshot};
use tokio::task::JoinHandle;

#[derive(Default, Debug)]
struct Registry {
	pending: VecDeque<Arc<AsyncMutex<Pending>>>,
	error: Option<String>,
}

#[derive(Debug)]
struct Pending {
	task: JoinHandle<Result<(), String>>,
	result: Option<Result<(), String>>,
}

/// Parent-owned completion barrier. Keep this outside the capture future.
#[derive(Default, Debug)]
pub struct Owner {
	handle: Handle,
}

impl Owner {
	/// Registration handle to put in the capture configuration.
	pub fn handle(&self) -> Handle {
		self.handle.clone()
	}

	/// Wait for every registered close after the capture future has been dropped.
	pub async fn finish(self) -> Result<(), String> {
		self.handle.wait().await
	}
}

/// Cloneable registration scope for one publication's capture resources.
#[derive(Clone, Default, Debug)]
pub struct Handle(Arc<Mutex<Registry>>);

/// Release only after synchronous backend teardown (including thread join).
pub(crate) struct Release {
	_signal: oneshot::Sender<()>,
}

impl Release {
	pub(crate) fn new() -> (Self, oneshot::Receiver<()>) {
		let (signal, released) = oneshot::channel();
		(Self { _signal: signal }, released)
	}
}

impl Handle {
	pub(crate) fn fail(&self, error: String) {
		self.0.lock().unwrap().error.get_or_insert(error);
	}

	pub(crate) fn run(&self, cleanup: impl Future<Output = Result<(), String>> + Send + 'static) {
		let task = tokio::spawn(cleanup);
		self.0
			.lock()
			.unwrap()
			.pending
			.push_back(Arc::new(AsyncMutex::new(Pending { task, result: None })));
	}

	/// Cancel-safe: the registry retains the JoinHandle while a waiter is dropped.
	pub(crate) async fn wait(&self) -> Result<(), String> {
		loop {
			let next = { self.0.lock().unwrap().pending.front().cloned() };
			let Some(next) = next else {
				return self.0.lock().unwrap().error.clone().map_or(Ok(()), Err);
			};
			let result = {
				let mut pending = next.lock().await;
				if pending.result.is_none() {
					pending.result = Some(
						(&mut pending.task)
							.await
							.unwrap_or_else(|error| Err(format!("capture cleanup task failed: {error}"))),
					);
				}
				pending.result.clone().unwrap()
			};
			let mut registry = self.0.lock().unwrap();
			if let Err(error) = result {
				registry.error.get_or_insert(error);
			}
			if registry.pending.front().is_some_and(|front| Arc::ptr_eq(front, &next)) {
				registry.pending.pop_front();
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn on_release(handle: &Handle, close: impl Future<Output = Result<(), String>> + Send + 'static) -> Release {
		let (release, released) = Release::new();
		handle.run(async move {
			let _ = released.await;
			close.await
		});
		release
	}

	#[tokio::test]
	async fn cancellation_during_acquisition_still_closes_the_late_resource() {
		let owner = Owner::default();
		let handle = owner.handle();
		let (grant, granted) = oneshot::channel();
		let (release, released) = Release::new();
		let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
		handle.run({
			let closed = closed.clone();
			async move {
				granted.await.unwrap();
				let _ = released.await;
				closed.store(true, std::sync::atomic::Ordering::SeqCst);
				Ok(())
			}
		});
		drop(release);
		let finished = tokio::spawn(owner.finish());
		tokio::task::yield_now().await;
		assert!(!finished.is_finished());
		grant.send(()).unwrap();
		finished.await.unwrap().unwrap();
		assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
	}

	#[tokio::test]
	async fn synchronous_backend_failure_is_not_reported_as_success() {
		let owner = Owner::default();
		owner.handle().fail("capture thread join failed".to_owned());
		assert_eq!(owner.finish().await.unwrap_err(), "capture thread join failed");
	}

	#[tokio::test]
	async fn source_loss_remains_terminal_when_demand_idle_wins() {
		let owner = Owner::default();
		let handle = owner.handle();
		let release = on_release(&handle, async { Ok(()) });
		handle.fail("screen capture source closed".to_owned());
		drop(release);
		assert_eq!(handle.wait().await.unwrap_err(), "screen capture source closed");
		assert_eq!(owner.finish().await.unwrap_err(), "screen capture source closed");
	}

	#[tokio::test]
	async fn close_follows_backend_join_and_parent_waits_for_acknowledgement() {
		let owner = Owner::default();
		let handle = owner.handle();
		let order = Arc::new(Mutex::new(Vec::new()));
		let (ack, acknowledged) = oneshot::channel();
		let (closing, started) = oneshot::channel();
		let release = on_release(&handle, {
			let order = order.clone();
			async move {
				order.lock().unwrap().push("close");
				let _ = closing.send(());
				let _ = acknowledged.await;
				order.lock().unwrap().push("ack");
				Ok(())
			}
		});
		let wait = tokio::spawn(owner.finish());
		tokio::task::yield_now().await;
		assert!(order.lock().unwrap().is_empty());
		order.lock().unwrap().push("join");
		drop(release);
		started.await.unwrap();
		assert!(!wait.is_finished());
		ack.send(()).unwrap();
		wait.await.unwrap().unwrap();
		assert_eq!(*order.lock().unwrap(), ["join", "close", "ack"]);
	}

	#[tokio::test]
	async fn cancelled_child_wait_does_not_lose_the_parent_cleanup_task() {
		let owner = Owner::default();
		let handle = owner.handle();
		let (ack, acknowledged) = oneshot::channel();
		let (closing, started) = oneshot::channel();
		let release = on_release(&handle, async move {
			closing.send(()).unwrap();
			acknowledged.await.unwrap();
			Err("portal close denied".to_owned())
		});
		drop(release);
		started.await.unwrap();
		let child = tokio::spawn(async move { handle.wait().await });
		tokio::task::yield_now().await;
		child.abort();
		assert!(child.await.unwrap_err().is_cancelled());
		ack.send(()).unwrap();
		assert_eq!(owner.finish().await.unwrap_err(), "portal close denied");
	}

	#[tokio::test]
	async fn cancelled_child_releases_its_registered_session() {
		let owner = Owner::default();
		let handle = owner.handle();
		let (created, ready) = oneshot::channel();
		let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
		let child = tokio::spawn({
			let closed = closed.clone();
			async move {
				let _release = on_release(&handle, async move {
					closed.store(true, std::sync::atomic::Ordering::SeqCst);
					Ok(())
				});
				created.send(()).unwrap();
				std::future::pending::<()>().await;
			}
		});
		ready.await.unwrap();
		child.abort();
		assert!(child.await.unwrap_err().is_cancelled());
		owner.finish().await.unwrap();
		assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
	}

	#[tokio::test]
	async fn demand_idle_can_close_then_register_another_session() {
		let owner = Owner::default();
		let handle = owner.handle();
		for _ in 0..3 {
			drop(on_release(&handle, async { Ok(()) }));
			handle.wait().await.unwrap();
			assert!(handle.0.lock().unwrap().pending.is_empty());
		}
		owner.finish().await.unwrap();
	}

	#[tokio::test]
	async fn cleanup_failure_is_sticky_and_other_resources_are_still_closed() {
		let owner = Owner::default();
		let handle = owner.handle();
		drop(on_release(&handle, async { Err("close failed".to_owned()) }));
		drop(on_release(&handle, async { Ok(()) }));
		assert_eq!(handle.wait().await.unwrap_err(), "close failed");
		assert!(handle.0.lock().unwrap().pending.is_empty());
		assert_eq!(owner.finish().await.unwrap_err(), "close failed");
	}
}
