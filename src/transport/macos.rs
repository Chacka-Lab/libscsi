//! macOS SCSI pass-through via IOKit (`IOSCSITaskUserClient`).
//!
//! Not yet implemented.  The interface matches the Windows layer so the rest
//! of the crate compiles cleanly on macOS targets.

use crate::types::{OpenOpts, ScsiCommand};
use crate::{Error, ScsiResult};

pub struct Device {
    _reserved: (),
}

impl Device {
    pub fn open(_path: &std::path::Path, _opts: &OpenOpts) -> Result<Self, Error> {
        todo!("macOS IOKit SCSI transport not yet implemented")
    }

    pub fn execute(&mut self, _cmd: ScsiCommand) -> Result<ScsiResult, Error> {
        todo!("macOS IOKit SCSI transport not yet implemented")
    }
}
