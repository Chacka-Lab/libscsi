//! Windows SCSI pass-through via `IOCTL_SCSI_PASS_THROUGH_DIRECT`.
//!
//! The kernel interface is documented at:
//! <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddscsi/ni-ntddscsi-ioctl_scsi_pass_through_direct>

use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle as _;

use windows_sys::Win32::Foundation::FALSE;
use windows_sys::Win32::System::IO::{DeviceIoControl, OVERLAPPED};

use crate::types::{
    Direction, MAX_CDB_LEN, MAX_SENSE_LEN, OpenOpts, ScsiCommand, ScsiStatus, Sense,
};
use crate::{Error, ScsiResult};

// ── IOCTL / access constants ────────────────────────────────────────────────

const IOCTL_SCSI_PASS_THROUGH_DIRECT: u32 = 0x0004_D014;
const SCSI_IOCTL_DATA_OUT: u8 = 0; // host → device
const SCSI_IOCTL_DATA_IN: u8 = 1; // device → host
const SCSI_IOCTL_DATA_UNSPECIFIED: u8 = 2;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;

// ── SCSI_PASS_THROUGH_DIRECT layout ─────────────────────────────────────────
//
// Mirrors the Windows SDK struct from ntddscsi.h.  On 64-bit targets the
// compiler adds 4 bytes of implicit padding before `data_buffer` to satisfy
// its 8-byte alignment, giving a total size of 56 bytes.

#[repr(C)]
struct ScsiPassThroughDirect {
    length: u16,                         // [i] size of this struct (must be set)
    scsi_status: u8,                     // [o] SCSI status byte
    path_id: u8,                         // [i] SCSI bus number (0 for auto)
    target_id: u8,                       // [i] target device ID (0 for auto)
    lun: u8,                             // [i] logical unit number
    cdb_length: u8,                      // [i] CDB length in bytes
    sense_info_length: u8,               // [i] sense buffer capacity; [o] bytes written
    data_in: u8,                         // [i] data transfer direction flag
    data_transfer_length: u32,           // [i] requested bytes; [o] actual transferred
    timeout_value: u32,                  // [i] timeout in seconds
    data_buffer: *mut core::ffi::c_void, // [i] data buffer pointer
    sense_info_offset: u32,              // [i] byte offset from start of SptdBuffer to sense
    cdb: [u8; MAX_CDB_LEN],              // [i] CDB bytes
}

// Sense data is appended right after the SPTD struct so that a single
// buffer pointer covers both.
#[repr(C)]
struct SptdBuffer {
    sptd: ScsiPassThroughDirect,
    sense: [u8; MAX_SENSE_LEN],
}

// ── Device ──────────────────────────────────────────────────────────────────

pub struct Device {
    fd: std::fs::File,
}

impl Device {
    pub fn open(path: &std::path::Path, opts: &OpenOpts) -> Result<Self, Error> {
        let share = if opts.exclusive {
            0
        } else {
            FILE_SHARE_READ | FILE_SHARE_WRITE
        };

        let fd = std::fs::OpenOptions::new()
            .share_mode(share)
            .access_mode(GENERIC_READ | GENERIC_WRITE)
            .open(path)?;

        Ok(Device { fd })
    }

    pub fn execute(&mut self, mut cmd: ScsiCommand) -> Result<ScsiResult, Error> {
        let timeout = cmd.timeout_secs.unwrap_or(30);
        let data_in_flag = match cmd.direction {
            Direction::In => SCSI_IOCTL_DATA_IN,
            Direction::Out => SCSI_IOCTL_DATA_OUT,
            Direction::None => SCSI_IOCTL_DATA_UNSPECIFIED,
        };

        let data_len = u32::try_from(cmd.data.len())
            .map_err(|_| Error::InvalidParameter("data buffer exceeds u32::MAX bytes"))?;

        let data_ptr: *mut core::ffi::c_void = if cmd.data.is_empty() {
            core::ptr::null_mut()
        } else {
            cmd.data.as_mut_ptr().cast()
        };

        let cdb_arr = cmd.cdb.as_array();
        let sense_offset = core::mem::offset_of!(SptdBuffer, sense) as u32;

        let mut buf = SptdBuffer {
            sptd: ScsiPassThroughDirect {
                length: size_of::<ScsiPassThroughDirect>() as u16,
                scsi_status: 0,
                path_id: 0,
                target_id: 0,
                lun: 0,
                cdb_length: cmd.cdb.len() as u8,
                sense_info_length: MAX_SENSE_LEN as u8,
                data_in: data_in_flag,
                data_transfer_length: data_len,
                timeout_value: timeout,
                data_buffer: data_ptr,
                sense_info_offset: sense_offset,
                cdb: cdb_arr,
            },
            sense: [0u8; MAX_SENSE_LEN],
        };

        let buf_len = size_of::<SptdBuffer>() as u32;
        let buf_ptr = (&mut buf as *mut SptdBuffer).cast::<core::ffi::c_void>();
        let mut bytes_returned: u32 = 0;

        // RawHandle is *mut c_void on Windows, matching HANDLE in windows-sys 0.61.
        let handle = self.fd.as_raw_handle();

        // SAFETY: `buf` and `cmd.data` both outlive this call; `handle` is a
        // valid, open device handle obtained from `self.fd`.
        let ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_SCSI_PASS_THROUGH_DIRECT,
                buf_ptr, // in buffer
                buf_len, // in buffer size
                buf_ptr, // out buffer
                buf_len, // out buffer size
                &mut bytes_returned,
                core::ptr::null_mut::<OVERLAPPED>(),
            )
        };

        if ok == FALSE {
            return Err(std::io::Error::last_os_error().into());
        }

        // Validate that the device didn't claim to transfer more bytes than
        // we allocated — this would indicate a kernel or firmware bug.
        let transferred = buf.sptd.data_transfer_length as usize;
        if transferred > cmd.data.len() {
            return Err(Error::Internal(
                "device reported more transferred bytes than the data buffer capacity",
            ));
        }

        // Truncate the data buffer to what was actually transferred so callers
        // don't need to track a separate length.
        cmd.data.truncate(transferred);

        let sense_len = buf.sptd.sense_info_length;
        let mut sense_data = [0u8; MAX_SENSE_LEN];
        let copy_len = (sense_len as usize).min(MAX_SENSE_LEN);
        sense_data[..copy_len].copy_from_slice(&buf.sense[..copy_len]);

        Ok(ScsiResult {
            status: ScsiStatus::from(buf.sptd.scsi_status),
            sense: Sense::new(sense_data, sense_len),
            data: cmd.data,
        })
    }
}

// ── Test helpers ────────────────────────────────────────────────────────────

#[cfg(test)]
impl Device {
    pub(crate) fn new_test() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join("libscsi_test_dummy.tmp");
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

    /// Pin the SPTD struct size to the Windows SDK layout (56 bytes on 64-bit).
    #[test]
    fn sptd_size_matches_sdk() {
        assert_eq!(
            size_of::<ScsiPassThroughDirect>(),
            56,
            "ScsiPassThroughDirect must be 56 bytes on 64-bit Windows"
        );
    }

    /// Sense data must start immediately after ScsiPassThroughDirect (offset 56).
    #[test]
    fn sptd_buffer_sense_offset() {
        assert_eq!(
            core::mem::offset_of!(SptdBuffer, sense),
            56,
            "sense data must start immediately after ScsiPassThroughDirect"
        );
    }
}
