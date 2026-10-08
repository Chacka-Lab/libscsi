//! Platform dispatch layer.
//!
//! Each platform module exposes a `Device` struct with the same interface:
//! `Device::open(path: &str) -> Result<Device, Error>` and
//! `Device::execute(cmd: ScsiCommand) -> Result<ScsiResult, Error>`.
//! This module re-exports the right one for the current target.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
compile_error!("libscsi: unsupported platform (only windows/linux/macos are supported)");

#[cfg(target_os = "linux")]
pub use self::linux::Device;
#[cfg(target_os = "macos")]
pub use self::macos::Device;
#[cfg(target_os = "windows")]
pub use self::windows::Device;
