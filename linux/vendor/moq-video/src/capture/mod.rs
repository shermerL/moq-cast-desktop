//! Surface capture. [`Config`] is shared; the implementation is per-platform and
//! per-source:
//! - macOS camera -> AVFoundation, screen -> ScreenCaptureKit, both yielding
//!   zero-copy `CVPixelBuffer` surfaces straight to VideoToolbox.
//! - Linux camera -> native V4L2 (YUYV / MJPEG -> CPU I420), Wayland screen ->
//!   xdg-desktop-portal + PipeWire, X11 screen -> XRandR + XShm (RGB -> CPU
//!   I420).
//! - Windows camera -> native Media Foundation (`IMFSourceReader`), screen ->
//!   DXGI Desktop Duplication (BGRA -> CPU I420).
//!
//! [`encode::publish_capture`](crate::encode::publish_capture) consumes [`Config`].

use std::sync::Arc;

use crate::Error;
use crate::frame::Surface;

mod channel;
use channel::FrameChannel;

/// Type-erased keep-alive for a capture backend, dropped to release the device.
///
/// `Send` off macOS (the backend is a pump-thread guard: an `Arc` stop flag plus
/// a `JoinHandle`), which keeps [`publish_capture`](crate::encode::publish_capture)
/// `Send` so a server can `tokio::spawn` it. On macOS the backend is the objc
/// `AVCaptureSession` (plus its delegate), which is `!Send`, so that platform's
/// capture future is `!Send` too.
#[cfg(not(target_os = "macos"))]
type Keepalive = Box<dyn std::any::Any + Send>;
#[cfg(target_os = "macos")]
type Keepalive = Box<dyn std::any::Any>;

#[cfg(target_os = "macos")]
mod avfoundation;
#[cfg(target_os = "macos")]
mod screencapture;
#[cfg(target_os = "macos")]
mod surface;

// Native V4L2 camera capture on Linux.
#[cfg(target_os = "linux")]
mod v4l2;

// Portal + PipeWire screen capture on Linux.
#[cfg(all(target_os = "linux", feature = "pipewire"))]
mod pipewire;

// Native XRandR + XShm screen capture on Linux X11 sessions.
#[cfg(target_os = "linux")]
mod x11;

// Native Media Foundation camera capture on Windows.
#[cfg(target_os = "windows")]
mod mediafoundation;

// DXGI Desktop Duplication screen capture on Windows.
#[cfg(target_os = "windows")]
mod desktopduplication;

// Blocking-device -> async-channel bridge used by V4L2 / Media Foundation.
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod pump;

/// What to capture. Each variant carries the identifier that selects it, so a
/// window can't be captured without saying which one, and a camera id can't
/// reach the display backend.
///
/// The identifiers come from [`cameras`], [`displays`], [`windows`], and
/// [`apps`]; each listed item's `source()` builds the matching variant.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Source {
	/// A camera / webcam. `None` opens the default camera.
	///
	/// The identifiers from [`cameras`] are an AVFoundation `uniqueID` on macOS,
	/// a `/dev/videoN` path on Linux, and a Media Foundation symbolic link on
	/// Windows. Bare numeric indices remain accepted on Linux and Windows.
	Camera(Option<String>),

	/// A whole display. `None` opens the main display.
	///
	/// The id is a bare display index on macOS and Windows. On Linux X11 it is an
	/// XRandR output id; on Wayland the portal picker owns selection and ignores
	/// it.
	Display(Option<String>),

	/// A single window, by the id [`windows`] reports. macOS only.
	Window(String),

	/// Every window belonging to one application, by the id [`apps`] reports
	/// (a bundle identifier). Windows that open later are included. macOS only.
	App(String),
}

/// The default camera, matching the historical `Config::default()`.
impl Default for Source {
	fn default() -> Self {
		Self::Camera(None)
	}
}

impl Source {
	/// A short human-readable name for the source, used in logs and as the
	/// captured device label.
	///
	/// macOS-only: one ScreenCaptureKit backend serves display, window, and app,
	/// so it names the source from the config. The other backends label a stream
	/// with the device they resolved (`/dev/video0`, a Media Foundation friendly
	/// name), which the config doesn't know.
	#[cfg(target_os = "macos")]
	pub(crate) fn label(&self) -> String {
		match self {
			Self::Camera(None) => "camera".to_string(),
			Self::Camera(Some(id)) => format!("camera:{id}"),
			Self::Display(None) => "display".to_string(),
			Self::Display(Some(id)) => format!("display:{id}"),
			Self::Window(id) => format!("window:{id}"),
			Self::App(id) => format!("app:{id}"),
		}
	}
}

/// A camera reported by [`cameras`].
#[derive(Clone, Debug)]
pub struct Camera {
	/// Opaque identifier: pass to [`Source::Camera`].
	pub id: String,
	/// Human-readable name, e.g. "FaceTime HD Camera".
	pub name: String,
}

impl Camera {
	/// The [`Source`] that captures this camera.
	pub fn source(&self) -> Source {
		Source::Camera(Some(self.id.clone()))
	}
}

/// A display reported by [`displays`].
#[derive(Clone, Debug)]
pub struct Display {
	/// Opaque identifier: pass to [`Source::Display`].
	pub id: String,
	/// Human-readable name, e.g. "Display 1".
	pub name: String,
	/// Width in the platform's desktop coordinate space. This is points on
	/// macOS and desktop pixels on Windows.
	pub width: u32,
	/// Height in the platform's desktop coordinate space.
	pub height: u32,
}

impl Display {
	/// The [`Source`] that captures this display.
	pub fn source(&self) -> Source {
		Source::Display(Some(self.id.clone()))
	}
}

/// A window reported by [`windows`].
#[derive(Clone, Debug)]
pub struct Window {
	/// Opaque identifier: pass to [`Source::Window`].
	pub id: String,
	/// The window title, empty if it has none.
	pub title: String,
	/// The name of the application owning the window.
	pub app: String,
	/// Width in points, i.e. the logical size, which is what capture defaults to.
	/// A window on a 2x retina display reports half its native pixel width.
	pub width: u32,
	/// Height in points. See [`width`](Self::width).
	pub height: u32,
}

impl Window {
	/// The [`Source`] that captures this window.
	pub fn source(&self) -> Source {
		Source::Window(self.id.clone())
	}
}

/// An application reported by [`apps`].
#[derive(Clone, Debug)]
pub struct App {
	/// Bundle identifier: pass to [`Source::App`].
	pub id: String,
	/// Human-readable name, e.g. "Safari".
	pub name: String,
}

impl App {
	/// The [`Source`] that captures every window of this application.
	pub fn source(&self) -> Source {
		Source::App(self.id.clone())
	}
}

/// Capture configuration. All fields are hints; the backend picks the closest
/// supported mode.
///
/// `#[non_exhaustive]`: construct via [`Config::default`] and set fields, so
/// new options can be added without breaking callers.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Config {
	/// What to capture.
	pub source: Source,
	pub width: Option<u32>,
	pub height: Option<u32>,
	pub framerate: Option<u32>,
	/// Draw the mouse cursor into captured frames. Screen/window/app sources
	/// only; ignored by cameras. Defaults to `true`.
	pub cursor: bool,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			source: Source::default(),
			width: None,
			height: None,
			framerate: None,
			cursor: true,
		}
	}
}

/// A live, async frame source opened via [`open`].
///
/// Every backend delivers frames through a shared [`FrameChannel`], so the
/// encode loop just `read().await`s regardless of platform. Dropping the stream
/// releases the device (stops the macOS `AVCaptureSession`, joins the V4L2 /
/// Media Foundation pump thread). That is the whole point: because `read` is a
/// real await, cancelling the capture future drops this and the camera turns off
/// promptly, with no blocking task left pinned to the runtime.
pub(crate) struct FrameStream {
	chan: Arc<FrameChannel>,
	width: u32,
	height: u32,
	framerate: Option<u32>,
	device: String,
	/// First frame captured during [`open`] (some backends learn their geometry
	/// only from a frame); returned by the first [`read`](Self::read).
	pending: Option<Surface>,
	/// Keeps the backend alive and releases it on drop. Type-erased because it
	/// differs per platform (objc session + delegate, or pump-thread guard).
	_backend: Keepalive,
}

impl FrameStream {
	/// Build a stream from a backend's channel, geometry, and keep-alive guard.
	fn new(
		chan: Arc<FrameChannel>,
		width: u32,
		height: u32,
		framerate: Option<u32>,
		device: String,
		pending: Option<Surface>,
		backend: Keepalive,
	) -> Self {
		Self {
			chan,
			width,
			height,
			framerate,
			device,
			pending,
			_backend: backend,
		}
	}

	/// Await the next frame, or `None` once the source ends. Cancel-safe: drop
	/// the future to stop reading and release the device.
	pub(crate) async fn read(&mut self) -> Option<Surface> {
		if let Some(frame) = self.pending.take() {
			return Some(frame);
		}
		self.chan.recv().await
	}

	pub(crate) fn width(&self) -> u32 {
		self.width
	}

	pub(crate) fn height(&self) -> u32 {
		self.height
	}

	/// The negotiated frame rate, or `None` if the source doesn't report one.
	pub(crate) fn framerate(&self) -> Option<u32> {
		self.framerate
	}

	/// The first frame's declared color space, when its capture backend knows it.
	pub(crate) fn color(&self) -> Option<crate::Color> {
		self.pending.as_ref().and_then(Surface::color)
	}

	pub(crate) fn device(&self) -> &str {
		&self.device
	}
}

/// Open the capture source described by `config`.
pub(crate) async fn open(config: &Config) -> Result<FrameStream, Error> {
	match &config.source {
		Source::Camera(device) => {
			let _ = device;
			#[cfg(target_os = "macos")]
			{
				avfoundation::open(config, device.as_deref()).await
			}
			#[cfg(target_os = "linux")]
			{
				v4l2::open(config, device.as_deref()).await
			}
			#[cfg(target_os = "windows")]
			{
				mediafoundation::open(config, device.as_deref()).await
			}
			#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
			{
				Err(Error::Unsupported("camera capture".to_string()))
			}
		}
		Source::Display(device) => {
			let _ = device;
			#[cfg(target_os = "macos")]
			{
				screencapture::open_display(config, device.as_deref()).await
			}
			#[cfg(target_os = "windows")]
			{
				desktopduplication::open(config, device.as_deref()).await
			}
			#[cfg(target_os = "linux")]
			{
				match linux_display_backend_from_env() {
					LinuxDisplayBackend::X11 => x11::open(config, device.as_deref()).await,
					LinuxDisplayBackend::Portal => {
						#[cfg(feature = "pipewire")]
						{
							pipewire::open(config, device.as_deref()).await
						}
						#[cfg(not(feature = "pipewire"))]
						{
							Err(Error::Unsupported(
								"Wayland screen capture without the `pipewire` feature".to_string(),
							))
						}
					}
				}
			}
			#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
			{
				Err(Error::Unsupported("screen capture".to_string()))
			}
		}
		Source::Window(id) => {
			let _ = id;
			#[cfg(target_os = "macos")]
			{
				screencapture::open_window(config, id).await
			}
			#[cfg(not(target_os = "macos"))]
			{
				Err(Error::Unsupported("window capture".to_string()))
			}
		}
		Source::App(id) => {
			let _ = id;
			#[cfg(target_os = "macos")]
			{
				screencapture::open_app(config, id).await
			}
			#[cfg(not(target_os = "macos"))]
			{
				Err(Error::Unsupported("application capture".to_string()))
			}
		}
	}
}

/// List the available cameras and the identifiers [`Source::Camera`] accepts.
pub async fn cameras() -> Result<Vec<Camera>, Error> {
	#[cfg(target_os = "macos")]
	{
		avfoundation::cameras()
	}
	#[cfg(target_os = "linux")]
	{
		blocking(v4l2::cameras).await
	}
	#[cfg(target_os = "windows")]
	{
		blocking(mediafoundation::cameras).await
	}
	#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
	{
		Err(Error::Unsupported("listing cameras".to_string()))
	}
}

/// List the available displays and the identifiers [`Source::Display`] accepts.
///
/// Linux X11 sessions expose XRandR outputs. On Wayland the portal picker owns
/// display selection, so there is no list or stable identifier to expose.
pub async fn displays() -> Result<Vec<Display>, Error> {
	#[cfg(target_os = "macos")]
	{
		screencapture::displays().await
	}
	#[cfg(target_os = "windows")]
	{
		blocking(desktopduplication::displays).await
	}
	#[cfg(target_os = "linux")]
	{
		match linux_display_backend_from_env() {
			LinuxDisplayBackend::X11 => blocking(x11::displays).await,
			LinuxDisplayBackend::Portal => Err(Error::Unsupported(
				"listing displays while the Wayland portal owns selection".to_string(),
			)),
		}
	}
	#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
	{
		Err(Error::Unsupported("listing displays".to_string()))
	}
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinuxDisplayBackend {
	Portal,
	X11,
}

#[cfg(target_os = "linux")]
fn linux_display_backend_from_env() -> LinuxDisplayBackend {
	let session_type = std::env::var("XDG_SESSION_TYPE").ok();
	linux_display_backend(
		session_type.as_deref(),
		std::env::var_os("WAYLAND_DISPLAY").is_some(),
		std::env::var_os("DISPLAY").is_some(),
	)
}

#[cfg(target_os = "linux")]
fn linux_display_backend(
	session_type: Option<&str>,
	has_wayland_display: bool,
	has_x11_display: bool,
) -> LinuxDisplayBackend {
	match session_type.map(str::to_ascii_lowercase).as_deref() {
		Some("x11") => LinuxDisplayBackend::X11,
		Some("wayland") => LinuxDisplayBackend::Portal,
		_ if has_wayland_display => LinuxDisplayBackend::Portal,
		_ if has_x11_display => LinuxDisplayBackend::X11,
		_ => LinuxDisplayBackend::Portal,
	}
}

/// List the on-screen windows. macOS only.
pub async fn windows() -> Result<Vec<Window>, Error> {
	#[cfg(target_os = "macos")]
	{
		screencapture::windows().await
	}
	#[cfg(not(target_os = "macos"))]
	{
		Err(Error::Unsupported("listing windows".to_string()))
	}
}

/// List the applications with at least one on-screen window. macOS only.
pub async fn apps() -> Result<Vec<App>, Error> {
	#[cfg(target_os = "macos")]
	{
		screencapture::apps().await
	}
	#[cfg(not(target_os = "macos"))]
	{
		Err(Error::Unsupported("listing applications".to_string()))
	}
}

/// Run synchronous platform enumeration off the async runtime's worker threads.
#[cfg(any(target_os = "linux", target_os = "windows"))]
async fn blocking<T, F>(f: F) -> Result<T, Error>
where
	F: FnOnce() -> Result<T, Error> + Send + 'static,
	T: Send + 'static,
{
	tokio::task::spawn_blocking(f)
		.await
		.map_err(|err| Error::Codec(anyhow::anyhow!("capture enumeration thread failed: {err}")))?
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
	use super::{LinuxDisplayBackend, linux_display_backend};

	#[test]
	fn x11_session_uses_native_capture() {
		assert_eq!(linux_display_backend(Some("x11"), true, true), LinuxDisplayBackend::X11);
	}

	#[test]
	fn wayland_session_uses_portal_capture() {
		assert_eq!(
			linux_display_backend(Some("wayland"), true, true),
			LinuxDisplayBackend::Portal
		);
	}

	#[test]
	fn display_environment_is_the_fallback_when_session_type_is_missing() {
		assert_eq!(linux_display_backend(None, false, true), LinuxDisplayBackend::X11);
		assert_eq!(linux_display_backend(None, true, true), LinuxDisplayBackend::Portal);
	}
}
