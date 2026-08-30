//! Main-thread bridge to the native ScreenCaptureKit content picker.

use std::sync::{Arc, mpsc};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_core_graphics::{
    CGMainDisplayID, CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess,
};
use objc2_foundation::{NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCContentSharingPicker, SCContentSharingPickerConfiguration,
    SCContentSharingPickerMode, SCContentSharingPickerObserver, SCShareableContentStyle, SCStream,
};

use crate::publication::Selection;

pub(super) enum Event {
    Selected(Selection),
    Cancelled,
    Failed,
}

struct ObserverIvars {
    events: mpsc::Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MoqCastContentSharingPickerObserver"]
    #[ivars = ObserverIvars]
    struct Observer;

    unsafe impl NSObjectProtocol for Observer {}

    unsafe impl SCContentSharingPickerObserver for Observer {
        #[unsafe(method(contentSharingPicker:didCancelForStream:))]
        unsafe fn did_cancel(&self, _picker: &SCContentSharingPicker, _stream: Option<&SCStream>) {
            let _ = self.ivars().events.send(Event::Cancelled);
            (self.ivars().wake)();
        }

        #[unsafe(method(contentSharingPicker:didUpdateWithFilter:forStream:))]
        unsafe fn did_update(
            &self,
            _picker: &SCContentSharingPicker,
            filter: &SCContentFilter,
            _stream: Option<&SCStream>,
        ) {
            let event = selection(filter).map_or(Event::Failed, Event::Selected);
            let _ = self.ivars().events.send(event);
            (self.ivars().wake)();
        }

        #[unsafe(method(contentSharingPickerStartDidFailWithError:))]
        unsafe fn did_fail(&self, error: &NSError) {
            tracing::warn!(error = %error.localizedDescription(), "content picker could not start");
            let _ = self.ivars().events.send(Event::Failed);
            (self.ivars().wake)();
        }
    }
);

impl Observer {
    fn new(events: mpsc::Sender<Event>, wake: impl Fn() + Send + Sync + 'static) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ObserverIvars {
            events,
            wake: Arc::new(wake),
        });
        unsafe { msg_send![super(this), init] }
    }
}

pub(super) struct Picker {
    picker: Retained<SCContentSharingPicker>,
    observer: Retained<Observer>,
    events: mpsc::Receiver<Event>,
}

impl Picker {
    pub(super) fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (events_tx, events) = mpsc::channel();
        let observer = Observer::new(events_tx, wake);
        let picker = unsafe { SCContentSharingPicker::sharedPicker() };
        let configuration = unsafe { SCContentSharingPickerConfiguration::new() };
        let modes =
            SCContentSharingPickerMode::SingleDisplay | SCContentSharingPickerMode::SingleWindow;
        unsafe {
            configuration.setAllowedPickerModes(modes);
            configuration.setAllowsChangingSelectedContent(false);
            picker.setDefaultConfiguration(&configuration);
            picker.addObserver(ProtocolObject::from_ref(&*observer));
            picker.setActive(true);
        }
        Self {
            picker,
            observer,
            events,
        }
    }

    pub(super) fn present(&self) {
        unsafe { self.picker.present() };
    }

    pub(super) fn poll(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        unsafe {
            self.picker
                .removeObserver(ProtocolObject::from_ref(&*self.observer));
            self.picker.setActive(false);
        }
    }
}

pub(super) fn permission_allowed() -> bool {
    CGPreflightScreenCaptureAccess()
}

pub(super) fn request_permission() -> bool {
    CGRequestScreenCaptureAccess()
}

fn selection(filter: &SCContentFilter) -> Option<Selection> {
    match unsafe { filter.style() } {
        SCShareableContentStyle::Display => {
            let display = unsafe { filter.includedDisplays() }.firstObject()?;
            let display_id = unsafe { display.displayID() };
            Some(Selection::Display {
                display_id,
                primary: display_id == CGMainDisplayID(),
                label: format!("Display {display_id}"),
            })
        }
        SCShareableContentStyle::Window => {
            let window = unsafe { filter.includedWindows() }.firstObject()?;
            let window_id = unsafe { window.windowID() };
            let title = unsafe { window.title() }
                .map(|title| title.to_string())
                .filter(|title| !title.trim().is_empty());
            let application = unsafe { window.owningApplication() }
                .map(|application| unsafe { application.applicationName() }.to_string())
                .filter(|application| !application.trim().is_empty());
            let label = title.or(application).unwrap_or_else(|| "Window".to_owned());
            Some(Selection::Window { window_id, label })
        }
        _ => None,
    }
}
