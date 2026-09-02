//! Native macOS sleep and wake notifications owned by the application shell.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter};

use crate::runtime::SystemLifecycle;

type ObserverToken = Retained<ProtocolObject<dyn NSObjectProtocol>>;

pub(crate) struct Observer {
    center: Retained<NSNotificationCenter>,
    tokens: Vec<ObserverToken>,
}

impl Observer {
    pub(crate) fn new(lifecycle: SystemLifecycle) -> Self {
        let workspace = NSWorkspace::sharedWorkspace();
        let center = workspace.notificationCenter();
        let mut tokens = Vec::with_capacity(2);

        let suspend = lifecycle.clone();
        let block = RcBlock::new(move |_: NonNull<NSNotification>| {
            if !suspend.suspend() {
                tracing::debug!("ignored system sleep after the runtime stopped");
            }
        });
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceWillSleepNotification),
                None,
                None,
                &block,
            )
        };
        tokens.push(token);

        let block = RcBlock::new(move |_: NonNull<NSNotification>| {
            if !lifecycle.resume() {
                tracing::debug!("ignored system wake after the runtime stopped");
            }
        });
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceDidWakeNotification),
                None,
                None,
                &block,
            )
        };
        tokens.push(token);

        Self { center, tokens }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        for token in &self.tokens {
            unsafe { self.center.removeObserver(token.as_ref()) };
        }
    }
}
