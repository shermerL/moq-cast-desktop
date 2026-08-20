//! Screen publication lifecycle and media pipeline.

#[cfg(target_os = "linux")]
mod audio;
pub(crate) mod session;
