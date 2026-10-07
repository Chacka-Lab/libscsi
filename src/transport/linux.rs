//! Linux SCSI pass-through via the `SG_IO` ioctl on `/dev/sg*` or block devices.
//!
//! Not yet implemented.  The interface matches the Windows layer so the rest
//! of the crate compiles cleanly on Linux targets.

use crate::types::{OpenOpts, ScsiCommand, ScsiDeviceInfo};
use crate::{Error, ScsiResult};

pub(crate) struct Device {
    _fd: std::fs::File,
}

impl Device {
    pub(crate) fn open(_path: &std::path::Path, _opts: &OpenOpts) -> Result<Self, Error> {
        todo!("Linux SG_IO transport not yet implemented")
    }

    pub(crate) fn execute(&mut self, _cmd: ScsiCommand) -> Result<ScsiResult, Error> {
        todo!("Linux SG_IO transport not yet implemented")
    }
}

/// Enumerate SCSI devices on Linux.
///
/// ## Implementation approach
///
/// 1. Scan `/sys/class/scsi_device/` for `H:C:T:L` entries.
/// 2. For each entry, read `/sys/class/scsi_device/H:C:T:L/device/type` to get
///    the SCSI peripheral device type byte.
/// 3. Map to `/dev/sg*` via `/sys/class/scsi_device/H:C:T:L/device/generic`.
/// 4. Open the device and issue an INQUIRY command to retrieve vendor/product/revision.
pub(crate) fn list_devices() -> Result<Vec<ScsiDeviceInfo>, Error> {
    todo!("Linux device enumeration not yet implemented")
}
