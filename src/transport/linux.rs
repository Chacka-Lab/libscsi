//! Linux SCSI pass-through via the `SG_IO` ioctl on `/dev/sg*` or block devices.
//!
//! Kernel interface: `<scsi/sg.h>`, `SG_IO (0x2285)`.
//! Reference: <https://www.kernel.org/doc/html/latest/scsi/scsi-generic.html>

use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::io::AsRawFd as _;

use crate::types::{Direction, MAX_SENSE_LEN, OpenOpts, ScsiCommand, ScsiStatus, Sense};
use crate::{Error, ScsiResult};
use std::path::Path;

// ── SG_IO constants ─────────────────────────────────────────────────────────

const SG_IO: libc::c_ulong = 0x2285;
const SG_INTERFACE_ID_ORIG: libc::c_int = b'S' as libc::c_int; // 83
const SG_DXFER_NONE: libc::c_int = -1; // no data transfer
const SG_DXFER_TO_DEV: libc::c_int = -2; // host → device
const SG_DXFER_FROM_DEV: libc::c_int = -3; // device → host

// ── sg_io_hdr layout ────────────────────────────────────────────────────────
//
// Mirrors the Linux kernel sg_io_hdr_t from <scsi/sg.h>.
// Fields marked [i] are inputs; [o] are outputs filled in by the kernel.

#[repr(C)]
struct SgIoHdr {
    interface_id: libc::c_int,     // [i] 'S' for SCSI generic (required)
    dxfer_direction: libc::c_int,  // [i] data transfer direction
    cmd_len: libc::c_uchar,        // [i] SCSI command length (1–16)
    mx_sb_len: libc::c_uchar,      // [i] sense buffer capacity
    iovec_count: libc::c_ushort,   // [i] 0 → no scatter/gather
    dxfer_len: libc::c_uint,       // [i] byte count of data transfer
    dxferp: *mut libc::c_void,     // [i] data buffer pointer
    cmdp: *mut libc::c_uchar,      // [i] CDB pointer
    sbp: *mut libc::c_uchar,       // [i] sense buffer pointer
    timeout: libc::c_uint,         // [i] timeout in milliseconds
    flags: libc::c_uint,           // [i] 0 → defaults
    pack_id: libc::c_int,          // [i→o] unused
    usr_ptr: *mut libc::c_void,    // [i→o] unused
    status: libc::c_uchar,         // [o] SCSI status byte
    masked_status: libc::c_uchar,  // [o] shifted/masked status
    msg_status: libc::c_uchar,     // [o] message level data
    sb_len_wr: libc::c_uchar,      // [o] bytes actually written into sbp
    host_status: libc::c_ushort,   // [o] host adapter errors
    driver_status: libc::c_ushort, // [o] driver-level errors
    resid: libc::c_int,            // [o] dxfer_len − actual_transferred
    duration: libc::c_uint,        // [o] command duration (ms)
    info: libc::c_uint,            // [o] auxiliary info flags
}

// ── Device ──────────────────────────────────────────────────────────────────

pub struct Device {
    fd: std::fs::File,
}

impl Device {
    pub fn open(path: &Path, opts: &OpenOpts) -> Result<Self, Error> {
        // O_NONBLOCK prevents open(2) itself from blocking on device files
        // (e.g. tape drives, some sg nodes); it does not affect SG_IO timeouts.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?;

        // Always take an advisory flock so that exclusive and non-exclusive
        // openers cooperate with each other:
        //   exclusive = true  → LOCK_EX: fails if anyone else holds the device.
        //   exclusive = false → LOCK_SH: coexists with other shared openers but
        //                       fails while an exclusive holder is active.
        // LOCK_NB surfaces contention as an immediate I/O error rather than
        // a hang.  flock is purely advisory; non-cooperating processes (those
        // that don't use this library) bypass it entirely.
        //
        // SAFETY: fd is valid and open for the duration of this call.
        let lock_op = if opts.exclusive {
            libc::LOCK_EX | libc::LOCK_NB
        } else {
            libc::LOCK_SH | libc::LOCK_NB
        };
        let rc = unsafe { libc::flock(file.as_raw_fd(), lock_op) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        Ok(Device { fd: file })
    }

    pub fn execute(&mut self, mut cmd: ScsiCommand) -> Result<ScsiResult, Error> {
        let timeout_ms = cmd.timeout_secs.unwrap_or(30).saturating_mul(1000);

        let dxfer_direction = match cmd.direction {
            Direction::In => SG_DXFER_FROM_DEV,
            Direction::Out => SG_DXFER_TO_DEV,
            Direction::None => SG_DXFER_NONE,
        };

        let dxfer_len = u32::try_from(cmd.data.len())
            .map_err(|_| Error::InvalidParameter("data buffer exceeds u32::MAX bytes"))?;

        let dxferp: *mut libc::c_void = if cmd.data.is_empty() {
            core::ptr::null_mut()
        } else {
            cmd.data.as_mut_ptr().cast()
        };

        // cmdp and sbp must remain alive for the duration of the ioctl call.
        let mut cdb = cmd.cdb.as_array();
        let mut sense = [0u8; MAX_SENSE_LEN];

        let mut hdr = SgIoHdr {
            interface_id: SG_INTERFACE_ID_ORIG,
            dxfer_direction,
            cmd_len: cmd.cdb.len() as libc::c_uchar,
            mx_sb_len: MAX_SENSE_LEN as libc::c_uchar,
            iovec_count: 0,
            dxfer_len,
            dxferp,
            cmdp: cdb.as_mut_ptr(),
            sbp: sense.as_mut_ptr(),
            timeout: timeout_ms,
            flags: 0,
            pack_id: 0,
            usr_ptr: core::ptr::null_mut(),
            // output fields — zeroed before the call
            status: 0,
            masked_status: 0,
            msg_status: 0,
            sb_len_wr: 0,
            host_status: 0,
            driver_status: 0,
            resid: 0,
            duration: 0,
            info: 0,
        };

        // SAFETY: `hdr`, `cdb`, `sense`, and `cmd.data` all outlive this call;
        // `self.fd` is a valid open file descriptor pointing to an sg device.
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), SG_IO, &mut hdr as *mut SgIoHdr) };
        if rc < 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        // resid = requested − transferred; a negative value is unusual but we
        // clamp defensively to the full buffer size rather than panicking.
        let transferred = if hdr.resid >= 0 {
            (dxfer_len as usize).saturating_sub(hdr.resid as usize)
        } else {
            cmd.data.len()
        };
        if transferred > cmd.data.len() {
            return Err(Error::Internal(
                "device reported more transferred bytes than the data buffer capacity",
            ));
        }
        cmd.data.truncate(transferred);

        let sense_len = hdr.sb_len_wr;
        let mut sense_data = [0u8; MAX_SENSE_LEN];
        let copy_len = (sense_len as usize).min(MAX_SENSE_LEN);
        sense_data[..copy_len].copy_from_slice(&sense[..copy_len]);

        Ok(ScsiResult {
            status: ScsiStatus::from(hdr.status),
            sense: Sense::new(sense_data, sense_len),
            data: cmd.data,
        })
    }
}

// ── Test helpers ─────────────────────────────────────────────────────────────

#[cfg(test)]
impl Device {
    /// Open a real temporary file as a stand-in fd for unit tests that only
    /// exercise logic above the ioctl layer (struct layout, open paths, etc.).
    pub(crate) fn new_test() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join("libscsi_test_dummy_linux.tmp");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;
        Ok(Device { fd: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Cdb, Direction, OpenOpts, ScsiCommand};

    // ── Struct layout ─────────────────────────────────────────────────────────

    /// sg_io_hdr_t is 88 bytes on 64-bit Linux (all pointer fields are 8 bytes).
    #[test]
    fn sg_io_hdr_size() {
        assert_eq!(
            size_of::<SgIoHdr>(),
            88,
            "SgIoHdr must be 88 bytes on 64-bit Linux"
        );
    }

    /// Verify that the first field (interface_id) sits at offset 0 and the
    /// status byte lands at the expected offset (36 on 64-bit).
    #[test]
    fn sg_io_hdr_status_offset() {
        assert_eq!(
            core::mem::offset_of!(SgIoHdr, status),
            36,
            "status byte must be at offset 36 in SgIoHdr"
        );
    }

    // ── Device::open ──────────────────────────────────────────────────────────

    #[test]
    fn open_nonexistent_path_returns_io_error() {
        let result = Device::open(
            Path::new("/dev/libscsi_nonexistent_999"),
            &OpenOpts::default(),
        );
        assert!(matches!(result, Err(Error::Io(_))));
    }

    /// Two shared opens on the same temp file must both succeed.
    #[test]
    fn open_shared_twice_succeeds() {
        let path = std::env::temp_dir().join("libscsi_test_shared.tmp");
        let _ = std::fs::File::create(&path).unwrap();

        let opts = OpenOpts { exclusive: false };
        let _d1 = Device::open(&path, &opts).expect("first shared open");
        let _d2 = Device::open(&path, &opts).expect("second shared open");
    }

    /// An exclusive open must fail while a shared opener holds the file.
    #[test]
    fn open_exclusive_blocked_by_shared() {
        let path = std::env::temp_dir().join("libscsi_test_excl_block.tmp");
        let _ = std::fs::File::create(&path).unwrap();

        let _shared = Device::open(&path, &OpenOpts { exclusive: false }).expect("shared open");

        let result = Device::open(&path, &OpenOpts { exclusive: true });
        assert!(
            matches!(result, Err(Error::Io(_))),
            "exclusive open should be rejected while shared lock is held"
        );
    }

    /// A shared open must fail while an exclusive opener holds the file.
    #[test]
    fn open_shared_blocked_by_exclusive() {
        let path = std::env::temp_dir().join("libscsi_test_shared_block.tmp");
        let _ = std::fs::File::create(&path).unwrap();

        let _excl = Device::open(&path, &OpenOpts { exclusive: true }).expect("exclusive open");

        let result = Device::open(&path, &OpenOpts { exclusive: false });
        assert!(
            matches!(result, Err(Error::Io(_))),
            "shared open should be rejected while exclusive lock is held"
        );
    }

    // ── Device::execute — reaches ioctl layer ────────────────────────────────

    /// A valid CDB issued against a plain file (not an sg node) must fail at
    /// the ioctl level with an I/O error — it must not panic or corrupt memory.
    #[test]
    fn execute_valid_cdb_reaches_ioctl() {
        let mut dev = Device::new_test().expect("temp file");
        let cmd = ScsiCommand {
            cdb: Cdb::new([0u8; 6]).unwrap(),
            direction: Direction::None,
            data: vec![],
            timeout_secs: None,
        };
        assert!(matches!(dev.execute(cmd), Err(Error::Io(_))));
    }
}
