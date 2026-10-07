/// Data transfer direction for a SCSI command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Device → host.
    In,
    /// Host → device.
    Out,
    /// No data phase.
    None,
}

/// SCSI status byte returned in the command response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScsiStatus {
    Good,
    CheckCondition,
    ConditionMet,
    Busy,
    ReservationConflict,
    TaskSetFull,
    AcaActive,
    TaskAborted,
    /// Any status byte not covered above.
    Unknown(u8),
}

impl From<u8> for ScsiStatus {
    fn from(v: u8) -> Self {
        match v {
            0x00 => ScsiStatus::Good,
            0x02 => ScsiStatus::CheckCondition,
            0x04 => ScsiStatus::ConditionMet,
            0x08 => ScsiStatus::Busy,
            0x18 => ScsiStatus::ReservationConflict,
            0x28 => ScsiStatus::TaskSetFull,
            0x30 => ScsiStatus::AcaActive,
            0x40 => ScsiStatus::TaskAborted,
            v => ScsiStatus::Unknown(v),
        }
    }
}

/// SCSI peripheral device type.
///
/// Derived from the INQUIRY response byte 0 bits 4–0, or from platform-specific
/// device properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScsiDeviceType {
    Disk,
    Tape,
    Printer,
    Processor,
    CdDvd,
    Scanner,
    OpticalMemory,
    MediumChanger,
    Communications,
    /// Any other device type; inner value is the raw peripheral device type byte.
    Unknown(u8),
}

impl From<u8> for ScsiDeviceType {
    fn from(v: u8) -> Self {
        match v {
            0x00 => ScsiDeviceType::Disk,
            0x01 => ScsiDeviceType::Tape,
            0x02 => ScsiDeviceType::Printer,
            0x03 => ScsiDeviceType::Processor,
            0x05 => ScsiDeviceType::CdDvd,
            0x06 => ScsiDeviceType::Scanner,
            0x07 => ScsiDeviceType::OpticalMemory,
            0x08 => ScsiDeviceType::MediumChanger,
            0x09 => ScsiDeviceType::Communications,
            v => ScsiDeviceType::Unknown(v),
        }
    }
}

/// Options controlling how a device is opened.
///
/// ## Platform semantics
///
/// | Platform | `exclusive = true`                                                            |
/// |----------|-------------------------------------------------------------------------------|
/// | Windows  | `share_mode = 0` — the kernel rejects any concurrent open attempt.            |
/// | Linux    | `flock(LOCK_EX \| LOCK_NB)` — advisory; ignored by non-cooperating processes. |
/// | macOS    | Same as Linux.                                                                |
#[derive(Debug, Clone)]
pub struct OpenOpts {
    /// If `true`, attempt to prevent other processes from opening the device
    /// concurrently.  See the platform semantics table above.
    pub exclusive: bool,
}

impl Default for OpenOpts {
    fn default() -> Self {
        OpenOpts { exclusive: false }
    }
}

/// Maximum CDB size this library supports.
pub const MAX_CDB_LEN: usize = 16;
/// Sense buffer size allocated per command.
pub const MAX_SENSE_LEN: usize = 32;

/// A SCSI command ready to be issued to a device.
///
/// For `Direction::In` pre-allocate `data` with the expected byte count:
/// `data: vec![0u8; 96]`.  The transport fills it and reports the actual
/// transfer length in `ScsiResult::transferred`.
///
/// For `Direction::Out` put the payload in `data`.
///
/// For `Direction::None` leave `data` empty.
#[derive(Debug, Clone)]
pub struct ScsiCommand {
    /// CDB bytes; padded to 16 bytes internally.
    pub cdb: [u8; MAX_CDB_LEN],
    /// Number of valid bytes in `cdb` (1–16).
    pub cdb_len: u8,
    pub direction: Direction,
    pub data: Vec<u8>,
    /// Command timeout in seconds; None → transport default (30 s).
    pub timeout_secs: Option<u32>,
}

/// Result of executing a SCSI command.
#[derive(Debug, Clone)]
pub struct ScsiResult {
    pub status: ScsiStatus,
    /// Raw sense data; meaningful bytes are `sense[sense_len as usize]`.
    pub sense: [u8; MAX_SENSE_LEN],
    pub sense_len: u8,
    /// For `Direction::In`, the data received from the device.
    pub data: Vec<u8>,
    /// Bytes actually transferred (reported by the host adapter).
    pub transferred: u32,
}

/// Information about a SCSI device discovered by enumeration.
#[derive(Debug, Clone)]
pub struct ScsiDeviceInfo {
    /// Platform-specific device path.
    ///
    /// | Platform | Example                          |
    /// |----------|----------------------------------|
    /// | Windows  | `\\\\.\\PhysicalDrive0`          |
    /// | Linux    | `/dev/sg0`                       |
    /// | macOS    | `/dev/disk0`                     |
    pub path: String,
    /// Device type derived from INQUIRY or platform properties.
    pub device_type: ScsiDeviceType,
    /// Vendor ID from INQUIRY (trimmed); may be empty if unavailable.
    pub vendor: String,
    /// Product ID from INQUIRY (trimmed); may be empty if unavailable.
    pub product: String,
    /// Revision from INQUIRY (trimmed); may be empty if unavailable.
    pub revision: String,
}

// ── Test helpers ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scsi_status_known_values() {
        let cases: &[(u8, ScsiStatus)] = &[
            (0x00, ScsiStatus::Good),
            (0x02, ScsiStatus::CheckCondition),
            (0x04, ScsiStatus::ConditionMet),
            (0x08, ScsiStatus::Busy),
            (0x18, ScsiStatus::ReservationConflict),
            (0x28, ScsiStatus::TaskSetFull),
            (0x30, ScsiStatus::AcaActive),
            (0x40, ScsiStatus::TaskAborted),
        ];
        for &(byte, ref expected) in cases {
            assert_eq!(ScsiStatus::from(byte), *expected, "byte {byte:#04x}");
        }
    }

    #[test]
    fn scsi_status_unknown_falls_through() {
        assert_eq!(ScsiStatus::from(0xFF), ScsiStatus::Unknown(0xFF));
        assert_eq!(ScsiStatus::from(0x01), ScsiStatus::Unknown(0x01));
    }
}
