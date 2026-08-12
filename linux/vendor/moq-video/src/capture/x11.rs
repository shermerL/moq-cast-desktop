//! Native X11 display capture using XRandR and MIT-SHM.

use std::ptr::NonNull;
use std::time::{Duration, Instant};

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::shm::{self, ConnectionExt as _};
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat, ImageOrder, Window};
use x11rb::rust_connection::RustConnection;

use super::channel::FrameChannel;
use super::pump::{self, Geometry};
use super::{Config, Display, FrameStream};
use crate::Error;
use crate::frame::{I420, Surface};

const DEFAULT_FRAMERATE: u32 = 30;

pub(super) async fn open(config: &Config, selector: Option<&str>) -> Result<FrameStream, Error> {
	let config = config.clone();
	let selector = selector.map(str::to_string);
	let chan = FrameChannel::new();
	let (geo, guard) = pump::spawn(
		chan.clone(),
		move || {
			let capture = Capture::open(&config, selector.as_deref())?;
			let geometry = Geometry {
				width: capture.target.width.into(),
				height: capture.target.height.into(),
				framerate: Some(capture.framerate),
				device: capture.target.name.clone(),
			};
			Ok((capture, geometry))
		},
		Capture::read,
	)
	.await?;

	Ok(FrameStream::new(
		chan,
		geo.width,
		geo.height,
		geo.framerate,
		geo.device,
		None,
		Box::new(guard),
	))
}

pub(super) fn displays() -> Result<Vec<Display>, Error> {
	let (conn, screen_num) = connect()?;
	let screen = conn
		.setup()
		.roots
		.get(screen_num)
		.ok_or_else(|| codec("X11 returned an invalid default screen"))?;
	let targets = query_targets(&conn, screen.root, screen.width_in_pixels, screen.height_in_pixels)?;
	Ok(targets
		.into_iter()
		.map(|target| Display {
			id: target.id.to_string(),
			name: target.name,
			width: target.width.into(),
			height: target.height.into(),
		})
		.collect())
}

struct Capture {
	conn: RustConnection,
	root: Window,
	target: Target,
	stride: u32,
	bits_per_pixel: u8,
	byte_order: ImageOrder,
	shm: Option<SharedMemory>,
	framerate: u32,
	next_frame: Instant,
	cursor: bool,
}

impl Capture {
	fn open(config: &Config, selector: Option<&str>) -> Result<Self, Error> {
		let (conn, screen_num) = connect()?;
		let screen = conn
			.setup()
			.roots
			.get(screen_num)
			.ok_or_else(|| codec("X11 returned an invalid default screen"))?;
		let root = screen.root;
		let root_depth = screen.root_depth;
		let root_width = screen.width_in_pixels;
		let root_height = screen.height_in_pixels;
		let byte_order = conn.setup().image_byte_order;
		let format = conn
			.setup()
			.pixmap_formats
			.iter()
			.find(|format| format.depth == root_depth)
			.ok_or_else(|| codec(format!("X11 has no pixel format for root depth {root_depth}")))?;

		if !matches!(format.bits_per_pixel, 24 | 32) {
			return Err(Error::Unsupported(format!(
				"X11 root uses {} bits per pixel; only 24-bit and 32-bit TrueColor are supported",
				format.bits_per_pixel
			)));
		}
		let bits_per_pixel = format.bits_per_pixel;
		let scanline_pad = format.scanline_pad;

		let targets = query_targets(&conn, root, root_width, root_height)?;
		let target = select_target(targets, selector)?;
		let row_bits = u32::from(target.width) * u32::from(bits_per_pixel);
		let pad = u32::from(scanline_pad);
		let stride = row_bits.div_ceil(pad) * pad / 8;
		let size = stride
			.checked_mul(u32::from(target.height))
			.ok_or_else(|| codec("X11 capture buffer size overflow"))?;
		let shm = SharedMemory::open(&conn, size)
			.map_err(|err| {
				tracing::warn!(error = %err, "XShm unavailable; falling back to XGetImage");
				err
			})
			.ok();
		let framerate = config.framerate.unwrap_or(DEFAULT_FRAMERATE);
		if framerate == 0 {
			return Err(Error::InvalidFramerate(framerate));
		}

		Ok(Self {
			conn,
			root,
			target,
			stride,
			bits_per_pixel,
			byte_order,
			shm,
			framerate,
			next_frame: Instant::now(),
			cursor: config.cursor,
		})
	}

	fn read(&mut self) -> Result<Option<Surface>, Error> {
		let now = Instant::now();
		if self.next_frame > now {
			std::thread::sleep(self.next_frame - now);
		}
		self.next_frame = Instant::now() + Duration::from_secs_f64(1.0 / f64::from(self.framerate));

		let mut pixels = match self.capture_shm() {
			Ok(Some(pixels)) => pixels,
			Ok(None) => self.capture_socket()?,
			Err(err) => {
				tracing::warn!(error = %err, "XShm capture failed; switching to XGetImage");
				self.close_shm();
				self.capture_socket()?
			}
		};
		if self.cursor {
			self.composite_cursor(&mut pixels);
		}
		let i420 = I420::from_bgra(
			&pixels,
			u32::from(self.target.width) * 4,
			self.target.width.into(),
			self.target.height.into(),
		)?;
		Ok(Some(Surface::I420(i420)))
	}

	fn capture_shm(&self) -> Result<Option<Vec<u8>>, Error> {
		let Some(shm) = &self.shm else {
			return Ok(None);
		};
		self.conn
			.shm_get_image(
				self.root,
				self.target.x,
				self.target.y,
				self.target.width,
				self.target.height,
				u32::MAX,
				u8::from(ImageFormat::Z_PIXMAP),
				shm.segment,
				0,
			)
			.map_err(|err| codec(format!("XShm request failed: {err}")))?
			.reply()
			.map_err(|err| codec(format!("XShm reply failed: {err}")))?;
		let bytes = unsafe { std::slice::from_raw_parts(shm.ptr.as_ptr(), shm.len) };
		self.to_bgra(bytes).map(Some)
	}

	fn capture_socket(&self) -> Result<Vec<u8>, Error> {
		let reply = self
			.conn
			.get_image(
				ImageFormat::Z_PIXMAP,
				self.root,
				self.target.x,
				self.target.y,
				self.target.width,
				self.target.height,
				u32::MAX,
			)
			.map_err(|err| codec(format!("XGetImage request failed: {err}")))?
			.reply()
			.map_err(|err| codec(format!("XGetImage reply failed: {err}")))?;
		self.to_bgra(&reply.data)
	}

	fn to_bgra(&self, source: &[u8]) -> Result<Vec<u8>, Error> {
		let width = usize::from(self.target.width);
		let height = usize::from(self.target.height);
		let stride = self.stride as usize;
		let required = stride
			.checked_mul(height)
			.ok_or_else(|| codec("X11 source buffer size overflow"))?;
		if source.len() < required {
			return Err(codec(format!(
				"X11 returned {} bytes, expected at least {required}",
				source.len()
			)));
		}

		let mut bgra = vec![0; width * height * 4];
		let bytes_per_pixel = usize::from(self.bits_per_pixel / 8);
		for row in 0..height {
			let src = &source[row * stride..row * stride + width * bytes_per_pixel];
			let dst = &mut bgra[row * width * 4..(row + 1) * width * 4];
			for (src, dst) in src.chunks_exact(bytes_per_pixel).zip(dst.chunks_exact_mut(4)) {
				let (b, g, r) = if self.byte_order == ImageOrder::LSB_FIRST {
					(src[0], src[1], src[2])
				} else {
					(
						src[bytes_per_pixel - 1],
						src[bytes_per_pixel - 2],
						src[bytes_per_pixel - 3],
					)
				};
				dst.copy_from_slice(&[b, g, r, 255]);
			}
		}
		Ok(bgra)
	}

	fn composite_cursor(&self, bgra: &mut [u8]) {
		let Ok(cookie) = self.conn.xfixes_get_cursor_image() else {
			return;
		};
		let Ok(cursor) = cookie.reply() else {
			return;
		};
		let left = i32::from(cursor.x) - i32::from(cursor.xhot) - i32::from(self.target.x);
		let top = i32::from(cursor.y) - i32::from(cursor.yhot) - i32::from(self.target.y);
		let width = i32::from(self.target.width);
		let height = i32::from(self.target.height);
		for cy in 0..i32::from(cursor.height) {
			for cx in 0..i32::from(cursor.width) {
				let x = left + cx;
				let y = top + cy;
				if x < 0 || y < 0 || x >= width || y >= height {
					continue;
				}
				let pixel = cursor.cursor_image[(cy * i32::from(cursor.width) + cx) as usize];
				let alpha = (pixel >> 24) as u8;
				if alpha == 0 {
					continue;
				}
				let offset = ((y * width + x) * 4) as usize;
				for (channel, source) in [(0, pixel as u8), (1, (pixel >> 8) as u8), (2, (pixel >> 16) as u8)] {
					let dest = bgra[offset + channel];
					bgra[offset + channel] = blend(source, dest, alpha);
				}
			}
		}
	}

	fn close_shm(&mut self) {
		if let Some(shm) = self.shm.take() {
			let _ = self.conn.shm_detach(shm.segment);
			let _ = self.conn.flush();
			drop(shm);
		}
	}
}

impl Drop for Capture {
	fn drop(&mut self) {
		self.close_shm();
	}
}

struct SharedMemory {
	segment: shm::Seg,
	ptr: NonNull<u8>,
	len: usize,
}

impl SharedMemory {
	fn open(conn: &RustConnection, size: u32) -> Result<Self, Error> {
		let extension = conn
			.extension_information(shm::X11_EXTENSION_NAME)
			.map_err(|err| codec(format!("could not query XShm extension: {err}")))?;
		if extension.is_none() {
			return Err(Error::Unsupported("XShm extension is unavailable".to_string()));
		}
		let version = conn
			.shm_query_version()
			.map_err(|err| codec(format!("XShm version request failed: {err}")))?
			.reply()
			.map_err(|err| codec(format!("XShm version reply failed: {err}")))?;
		if (version.major_version, version.minor_version) < (1, 2) {
			return Err(Error::Unsupported(format!(
				"XShm {}.{} lacks file-descriptor segments",
				version.major_version, version.minor_version
			)));
		}

		let segment = conn
			.generate_id()
			.map_err(|err| codec(format!("could not allocate XShm segment id: {err}")))?;
		let reply = conn
			.shm_create_segment(segment, size, false)
			.map_err(|err| codec(format!("XShm segment request failed: {err}")))?
			.reply()
			.map_err(|err| codec(format!("XShm segment reply failed: {err}")))?;
		let ptr = unsafe {
			libc::mmap(
				std::ptr::null_mut(),
				size as usize,
				libc::PROT_READ | libc::PROT_WRITE,
				libc::MAP_SHARED,
				std::os::fd::AsRawFd::as_raw_fd(&reply.shm_fd),
				0,
			)
		};
		if ptr == libc::MAP_FAILED {
			let _ = conn.shm_detach(segment);
			return Err(codec(format!(
				"could not mmap XShm buffer: {}",
				std::io::Error::last_os_error()
			)));
		}
		let Some(ptr) = NonNull::new(ptr.cast::<u8>()) else {
			let _ = conn.shm_detach(segment);
			return Err(codec("mmap returned a null XShm buffer"));
		};
		Ok(Self {
			segment,
			ptr,
			len: size as usize,
		})
	}
}

impl Drop for SharedMemory {
	fn drop(&mut self) {
		unsafe {
			libc::munmap(self.ptr.as_ptr().cast(), self.len);
		}
	}
}

struct Target {
	id: randr::Output,
	name: String,
	x: i16,
	y: i16,
	width: u16,
	height: u16,
	primary: bool,
}

fn query_targets(conn: &RustConnection, root: Window, root_width: u16, root_height: u16) -> Result<Vec<Target>, Error> {
	conn.randr_query_version(1, 3)
		.map_err(|err| codec(format!("XRandR version request failed: {err}")))?
		.reply()
		.map_err(|err| codec(format!("XRandR version reply failed: {err}")))?;
	let resources = conn
		.randr_get_screen_resources_current(root)
		.map_err(|err| codec(format!("XRandR resources request failed: {err}")))?
		.reply()
		.map_err(|err| codec(format!("XRandR resources reply failed: {err}")))?;
	let primary = conn
		.randr_get_output_primary(root)
		.ok()
		.and_then(|cookie| cookie.reply().ok())
		.map(|reply| reply.output);
	let mut targets = Vec::new();
	for output in resources.outputs {
		let Ok(info) = conn.randr_get_output_info(output, resources.config_timestamp) else {
			continue;
		};
		let Ok(info) = info.reply() else {
			continue;
		};
		if info.connection != randr::Connection::CONNECTED || info.crtc == 0 {
			continue;
		}
		let Ok(crtc) = conn.randr_get_crtc_info(info.crtc, resources.config_timestamp) else {
			continue;
		};
		let Ok(crtc) = crtc.reply() else {
			continue;
		};
		let width = crtc.width & !1;
		let height = crtc.height & !1;
		if width < 2 || height < 2 {
			continue;
		}
		targets.push(Target {
			id: output,
			name: String::from_utf8_lossy(&info.name).into_owned(),
			x: crtc.x,
			y: crtc.y,
			width,
			height,
			primary: primary == Some(output),
		});
	}
	if targets.is_empty() {
		let width = root_width & !1;
		let height = root_height & !1;
		if width < 2 || height < 2 {
			return Err(codec("X11 root display is too small to capture"));
		}
		targets.push(Target {
			id: 0,
			name: "X11 desktop".to_string(),
			x: 0,
			y: 0,
			width,
			height,
			primary: true,
		});
	}
	Ok(targets)
}

fn select_target(mut targets: Vec<Target>, selector: Option<&str>) -> Result<Target, Error> {
	if let Some(selector) = selector {
		let id = selector
			.parse::<u32>()
			.map_err(|_| Error::Unsupported(format!("invalid XRandR output id: {selector}")))?;
		return targets
			.into_iter()
			.find(|target| target.id == id)
			.ok_or_else(|| Error::Unsupported(format!("XRandR output {selector} was not found")));
	}
	let index = targets.iter().position(|target| target.primary).unwrap_or(0);
	Ok(targets.swap_remove(index))
}

fn connect() -> Result<(RustConnection, usize), Error> {
	x11rb::connect(None).map_err(|err| codec(format!("could not connect to X11 display: {err}")))
}

fn blend(source: u8, dest: u8, alpha: u8) -> u8 {
	let alpha = u16::from(alpha);
	((u16::from(source) * alpha + u16::from(dest) * (255 - alpha) + 127) / 255) as u8
}

fn codec(message: impl Into<String>) -> Error {
	Error::Codec(anyhow::anyhow!(message.into()))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cursor_blend_handles_transparent_and_opaque_pixels() {
		assert_eq!(blend(200, 50, 0), 50);
		assert_eq!(blend(200, 50, 255), 200);
		assert_eq!(blend(200, 50, 128), 125);
	}
}
