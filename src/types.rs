use crate::Error;

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

/// A validated SCSI CDB (Command Descriptor Block).
///
/// Internally a fixed `[u8; 16]` buffer; only the first `len()` bytes are
/// meaningful.  Constructed via [`Cdb::new`], which rejects out-of-range
/// lengths so an invalid command can never reach the transport.
#[derive(Debug, Clone)]
pub struct Cdb {
    data: [u8; MAX_CDB_LEN],
    len: u8,
}

impl Cdb {
    /// Create a CDB from a byte slice.  Returns an error if the length is
    /// not in the range 1–16.
    pub fn new(bytes: impl AsRef<[u8]>) -> Result<Self, Error> {
        let slice = bytes.as_ref();
        let len = slice.len();
        if len == 0 || len > MAX_CDB_LEN {
            return Err(Error::InvalidParameter("CDB length must be 1–16"));
        }
        let mut data = [0u8; MAX_CDB_LEN];
        data[..len].copy_from_slice(slice);
        Ok(Cdb {
            data,
            len: len as u8,
        })
    }

    /// Number of valid CDB bytes (always 1–16).
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Valid CDB bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }

    /// Raw backing array; the caller is responsible for only reading
    /// `[..self.len()]` bytes.  Used by the transport layer.
    pub(crate) fn as_array(&self) -> [u8; MAX_CDB_LEN] {
        self.data
    }
}

impl AsRef<[u8]> for Cdb {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Sense data returned by the device, typically on `CHECK CONDITION`.
///
/// Internally a fixed `[u8; 32]` buffer; only the first `len()` bytes are
/// meaningful.  Use [`Sense::as_bytes`] to obtain the valid slice.
#[derive(Debug, Clone)]
pub struct Sense {
    data: [u8; MAX_SENSE_LEN],
    len: u8,
}

impl Sense {
    /// Build a `Sense` from the raw buffer and the byte count the transport
    /// reports as valid. `len` is clamped to `MAX_SENSE_LEN`.
    pub(crate) fn new(data: [u8; MAX_SENSE_LEN], len: u8) -> Self {
        Sense {
            data,
            len: len.min(MAX_SENSE_LEN as u8),
        }
    }

    /// Valid sense bytes (empty when no sense data was reported).
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }

    /// Number of valid sense bytes.
    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl AsRef<[u8]> for Sense {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// A SCSI command ready to be issued to a device.
///
/// For `Direction::In` pre-allocate `data` with the expected byte count
/// (e.g. `vec![0u8; 96]`); the transport fills it and truncates to the number
/// of bytes the device actually returned.
///
/// For `Direction::Out` put the payload in `data`.
///
/// For `Direction::None` leave `data` empty.
#[derive(Debug, Clone)]
pub struct ScsiCommand {
    pub cdb: Cdb,
    pub direction: Direction,
    pub data: Vec<u8>,
    /// Command timeout in seconds; `None` → transport default (30 s).
    pub timeout_secs: Option<u32>,
}

/// Result of executing a SCSI command.
///
/// `data` is already truncated to the number of bytes the device actually
/// transferred, so `data.len()` is always authoritative — no separate
/// transfer-length field is exposed.
#[derive(Debug, Clone)]
pub struct ScsiResult {
    pub status: ScsiStatus,
    pub sense: Sense,
    pub data: Vec<u8>,
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

    // ── Cdb ───────────────────────────────────────────────────────────────────

    #[test]
    fn cdb_new_valid_lengths() {
        for len in [1usize, 6, 10, 16] {
            let cdb =
                Cdb::new(vec![0xAB; len]).unwrap_or_else(|_| panic!("len {len} should be valid"));
            assert_eq!(cdb.len(), len);
            assert_eq!(cdb.as_bytes().len(), len);
        }
    }

    #[test]
    fn cdb_new_rejects_empty() {
        let err = Cdb::new(&[]).unwrap_err();
        assert!(matches!(err, Error::InvalidParameter(_)));
    }

    #[test]
    fn cdb_new_rejects_too_long() {
        let bytes = vec![0u8; MAX_CDB_LEN + 1];
        let err = Cdb::new(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidParameter(_)));
    }

    #[test]
    fn cdb_as_ref_returns_bytes() {
        let cdb = Cdb::new([0x12, 0x34]).unwrap();
        assert_eq!(cdb.as_ref(), &[0x12, 0x34]);
    }

    // ── Sense ──────────────────────────────────────────────────────────────────

    #[test]
    fn sense_as_bytes_truncates_to_len() {
        let mut data = [0u8; MAX_SENSE_LEN];
        data[0] = 0x70;
        data[1] = 0x00;
        let sense = Sense::new(data, 2);
        assert_eq!(sense.as_bytes(), &[0x70, 0x00]);
        assert_eq!(sense.len(), 2);
        assert!(!sense.is_empty());
    }

    #[test]
    fn sense_empty_when_len_zero() {
        let sense = Sense::new([0u8; MAX_SENSE_LEN], 0);
        assert!(sense.is_empty());
        assert_eq!(sense.as_bytes(), &[]);
    }

    #[test]
    fn sense_len_clamped_to_max() {
        let sense = Sense::new([0xFF; MAX_SENSE_LEN], (MAX_SENSE_LEN + 1) as u8);
        assert_eq!(sense.len(), MAX_SENSE_LEN);
    }
}
