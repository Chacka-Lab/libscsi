//! macOS SCSI pass-through via IOKit (`IOSCSITaskUserClient`).
//!
//! Not yet implemented.  The interface matches the Windows layer so the rest
//! of the crate compiles cleanly on macOS targets.

use crate::types::{OpenOpts, ScsiCommand, ScsiDeviceInfo};
use crate::{Error, ScsiResult};

pub(crate) struct Device {
    _reserved: (),
}

impl Device {
    pub(crate) fn open(_path: &std::path::Path, _opts: &OpenOpts) -> Result<Self, Error> {
        todo!("macOS IOKit SCSI transport not yet implemented")
    }

    pub(crate) fn execute(&mut self, _cmd: ScsiCommand) -> Result<ScsiResult, Error> {
        todo!("macOS IOKit SCSI transport not yet implemented")
    }
}

/// Enumerate SCSI devices on macOS.
///
/// ## Implementation approach
///
/// 1. Use IOKit to enumerate services matching `kIOSCSIPeripheralDeviceNub` or
///    `IOSCSIProtocolServices`.
/// 2. For each service, query the BSD name (e.g., `/dev/disk0`) via
///    `IORegistryEntryCreateCFProperty` with key `BSD Name`.
/// 3. Query the `Peripheral Device Type` property to get the SCSI device type byte.
/// 4. Open the device and issue an INQUIRY command to retrieve vendor/product/revision.
pub(crate) fn list_devices() -> Result<Vec<ScsiDeviceInfo>, Error> {
    todo!("macOS device enumeration not yet implemented")
}
