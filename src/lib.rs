mod transport;
mod types;

pub use types::{
    Direction, MAX_CDB_LEN, MAX_SENSE_LEN, OpenOpts, ScsiCommand, ScsiDeviceInfo, ScsiDeviceType,
    ScsiResult, ScsiStatus,
};

/// Library-level error type.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    InvalidParameter(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::InvalidParameter(msg) => write!(f, "invalid parameter: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// An open handle to a SCSI device.
///
/// Create one with [`ScsiDevice::open`], then call [`ScsiDevice::execute`] to
/// issue commands.  The handle is closed automatically when dropped.
pub struct ScsiDevice(transport::Device);

impl ScsiDevice {
    /// Open a SCSI device by path.
    ///
    /// | Platform | Example paths                            |
    /// |----------|------------------------------------------|
    /// | Windows  | `\\\\.\\PhysicalDrive0`, `\\\\.\\Scsi0:` |
    /// | Linux    | `/dev/sg0`, `/dev/sda`                   |
    /// | macOS    | `/dev/disk0`                             |
    ///
    /// Pass `OpenOptions::default()` for shared (non-exclusive) access.
    pub fn open(path: &std::path::Path, opts: &OpenOpts) -> Result<Self, Error> {
        transport::Device::open(path, opts).map(ScsiDevice)
    }

    /// Issue a SCSI command and wait for it to complete.
    ///
    /// For `Direction::In` pre-fill `cmd.data` with `vec![0u8; <expected bytes>]`
    /// and read the response from `ScsiResult::data`.
    pub fn execute(&mut self, cmd: ScsiCommand) -> Result<ScsiResult, Error> {
        if cmd.cdb_len == 0 || cmd.cdb_len as usize > MAX_CDB_LEN {
            return Err(Error::InvalidParameter("cdb_len must be 1–16"));
        }
        self.0.execute(cmd)
    }
}

/// Enumerate SCSI devices visible to the current process.
///
/// Returns a list of devices with their paths, types, and identity strings
/// (vendor/product/revision from INQUIRY when available).
///
/// ## Platform-specific behavior
///
/// - **Windows**: uses `SetupDiGetClassDevs` with `GUID_DEVINTERFACE_SCSI_RAW`
///   (or a similar SCSI interface GUID) and queries device properties + INQUIRY.
/// - **Linux**: scans `/sys/class/scsi_device/` and `/dev/sg*` or `/dev/sd*`.
/// - **macOS**: uses IOKit to enumerate SCSI services and their BSD names.
pub fn list_devices() -> Result<Vec<ScsiDeviceInfo>, Error> {
    transport::list_devices()
}

// ── Test helpers ─────────────────────────────────────────────────────────────

#[cfg(test)]
impl ScsiDevice {
    /// Wrap a platform test handle so tests can exercise the public API without
    /// a real SCSI device node.  Only available on Windows for now.
    #[cfg(target_os = "windows")]
    fn new_test() -> std::io::Result<Self> {
        transport::Device::new_test().map(ScsiDevice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Error type ────────────────────────────────────────────────────────────

    #[test]
    fn error_display_invalid_parameter() {
        let e = Error::InvalidParameter("something went wrong");
        assert_eq!(e.to_string(), "invalid parameter: something went wrong");
    }

    #[test]
    fn error_display_io() {
        let e = Error::Io(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(e.to_string().starts_with("I/O error:"));
    }

    #[test]
    fn error_source_io_is_some() {
        use std::error::Error as _;
        let e = Error::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(e.source().is_some());
    }

    #[test]
    fn error_source_invalid_parameter_is_none() {
        use std::error::Error as _;
        let e = Error::InvalidParameter("x");
        assert!(e.source().is_none());
    }

    // ── ScsiDevice::open ──────────────────────────────────────────────────────

    #[test]
    fn open_nonexistent_path_returns_io_error() {
        let result = ScsiDevice::open(
            std::path::Path::new(r"\\.\LibScsiNonexistentDevice999"),
            &OpenOpts::default(),
        );
        assert!(matches!(result, Err(Error::Io(_))));
    }

    // ── ScsiDevice::execute — input validation ────────────────────────────────

    #[cfg(target_os = "windows")]
    #[test]
    fn execute_cdb_len_zero_rejected() {
        let mut dev = ScsiDevice::new_test().expect("temp file");
        let cmd = ScsiCommand {
            cdb: [0u8; MAX_CDB_LEN],
            cdb_len: 0,
            direction: Direction::None,
            data: vec![],
            timeout_secs: None,
        };
        assert!(matches!(dev.execute(cmd), Err(Error::InvalidParameter(_))));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn execute_cdb_len_too_large_rejected() {
        let mut dev = ScsiDevice::new_test().expect("temp file");
        let cmd = ScsiCommand {
            cdb: [0u8; MAX_CDB_LEN],
            cdb_len: (MAX_CDB_LEN + 1) as u8,
            direction: Direction::None,
            data: vec![],
            timeout_secs: None,
        };
        assert!(matches!(dev.execute(cmd), Err(Error::InvalidParameter(_))));
    }

    /// A valid cdb_len on a non-SCSI handle must fail at the IOCTL level
    /// (Error::Io), not at the validation gate.
    #[cfg(target_os = "windows")]
    #[test]
    fn execute_valid_cdb_len_reaches_ioctl() {
        let mut dev = ScsiDevice::new_test().expect("temp file");
        let cmd = ScsiCommand {
            cdb: [0u8; MAX_CDB_LEN],
            cdb_len: 6,
            direction: Direction::None,
            data: vec![],
            timeout_secs: None,
        };
        assert!(matches!(dev.execute(cmd), Err(Error::Io(_))));
    }
}
