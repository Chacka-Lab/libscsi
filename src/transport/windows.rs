//! Windows SCSI pass-through via `IOCTL_SCSI_PASS_THROUGH_DIRECT`.
//!
//! The kernel interface is documented at:
//! <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddscsi/ni-ntddscsi-ioctl_scsi_pass_through_direct>

use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle as _;

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::IO::{DeviceIoControl, OVERLAPPED};
use windows_sys::core::GUID;

use crate::types::{
    Direction, MAX_CDB_LEN, MAX_SENSE_LEN, OpenOpts, ScsiCommand, ScsiDeviceInfo, ScsiDeviceType,
    ScsiStatus,
};
use crate::{Error, ScsiResult};

// ── IOCTL / access constants ─────────────────────────────────────────────────

const IOCTL_SCSI_PASS_THROUGH_DIRECT: u32 = 0x0004_D014;
const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002D_1080;

const SCSI_IOCTL_DATA_OUT: u8 = 0; // host → device
const SCSI_IOCTL_DATA_IN: u8 = 1; // device → host
const SCSI_IOCTL_DATA_UNSPECIFIED: u8 = 2;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;

#[repr(C)]
struct StorageDeviceNumber {
    device_type: u32,
    device_number: u32,
    partition_number: u32,
}

// ── SCSI_PASS_THROUGH_DIRECT layout ─────────────────────────────────────────
//
// Mirrors the Windows SDK struct from ntddscsi.h.  On 64-bit targets the
// compiler adds 4 bytes of implicit padding before `data_buffer` to satisfy
// its 8-byte alignment, giving a total size of 56 bytes.

#[repr(C)]
struct ScsiPassThroughDirect {
    length: u16,
    scsi_status: u8,
    path_id: u8,
    target_id: u8,
    lun: u8,
    cdb_length: u8,
    sense_info_length: u8, // in: capacity allocated; out: bytes written
    data_in: u8,
    data_transfer_length: u32,
    timeout_value: u32,
    // 64-bit: 4 bytes of implicit padding here before the pointer
    data_buffer: *mut core::ffi::c_void,
    sense_info_offset: u32, // byte offset from start of SptdBuffer
    cdb: [u8; MAX_CDB_LEN],
}

// Sense data is appended right after the SPTD struct so that a single
// buffer pointer covers both.
#[repr(C)]
struct SptdBuffer {
    sptd: ScsiPassThroughDirect,
    sense: [u8; MAX_SENSE_LEN],
}

// ── Device ───────────────────────────────────────────────────────────────────

pub(crate) struct Device {
    // Holds the open device handle; closed automatically when dropped.
    file: std::fs::File,
}

impl Device {
    pub(crate) fn open(path: &std::path::Path, opts: &OpenOpts) -> Result<Self, Error> {
        // exclusive=true → share_mode 0: the kernel rejects any concurrent open.
        // exclusive=false → share_mode READ|WRITE: multiple handles are allowed.
        let share = if opts.exclusive {
            0
        } else {
            FILE_SHARE_READ | FILE_SHARE_WRITE
        };

        let file = std::fs::OpenOptions::new()
            .share_mode(share)
            .access_mode(GENERIC_READ | GENERIC_WRITE)
            .open(path)?;

        Ok(Device { file })
    }

    pub(crate) fn execute(&mut self, mut cmd: ScsiCommand) -> Result<ScsiResult, Error> {
        let timeout = cmd.timeout_secs.unwrap_or(30);
        let sense_offset = core::mem::offset_of!(SptdBuffer, sense) as u32;
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

        let mut buf = SptdBuffer {
            sptd: ScsiPassThroughDirect {
                length: size_of::<ScsiPassThroughDirect>() as u16,
                scsi_status: 0,
                path_id: 0,
                target_id: 0,
                lun: 0,
                cdb_length: cmd.cdb_len,
                sense_info_length: MAX_SENSE_LEN as u8,
                data_in: data_in_flag,
                data_transfer_length: data_len,
                timeout_value: timeout,
                data_buffer: data_ptr,
                sense_info_offset: sense_offset,
                cdb: cmd.cdb,
            },
            sense: [0u8; MAX_SENSE_LEN],
        };

        let buf_len = size_of::<SptdBuffer>() as u32;
        let buf_ptr = (&mut buf as *mut SptdBuffer).cast::<core::ffi::c_void>();
        let mut bytes_returned: u32 = 0;

        // RawHandle is *mut c_void on Windows, which matches HANDLE in windows-sys 0.61.
        let handle = self.file.as_raw_handle();

        // SAFETY: `buf` and `cmd.data` both outlive this call; `handle` is a
        // valid, open device handle obtained from `self.file`.
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

        // IOTC failed; received system error.
        if ok == 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        // IOTC ok; transport SCSI data. Don't handle SCSI error.
        let sense_len = buf.sptd.sense_info_length.min(MAX_SENSE_LEN as u8);
        let mut sense = [0u8; MAX_SENSE_LEN];
        sense[..sense_len as usize].copy_from_slice(&buf.sense[..sense_len as usize]);

        Ok(ScsiResult {
            status: ScsiStatus::from(buf.sptd.scsi_status),
            sense,
            sense_len,
            data: cmd.data,
            transferred: buf.sptd.data_transfer_length,
        })
    }
}

// ── Device enumeration ───────────────────────────────────────────────────────

const SCSI_RAW_INTERFACE_GUID: GUID = GUID::from_u128(0x53f56309_b6bf_11d0_94f2_00a0c91efb8b);
// const WMI_SCSI_ADDRESS_GUID: GUID = GUID::from_u128(0x53f5630f_b6bf_11d0_94f2_00a0c91efb8b);

/// Enumerate SCSI devices visible to the current process.
///
/// Uses `SetupDiGetClassDevsW` with `GUID_DEVINTERFACE_DISK` to enumerate all
/// disk devices, then attempts to open each and issue an INQUIRY command to
/// retrieve vendor/product/revision and device type.
pub(crate) fn list_devices() -> Result<Vec<ScsiDeviceInfo>, Error> {
    let mut devices = Vec::new();

    eprintln!("[DEBUG] Starting SetupDi enumeration...");

    // SAFETY: passing valid GUID pointer and flags.
    let hdevinfo: HDEVINFO = unsafe {
        SetupDiGetClassDevsW(
            &SCSI_RAW_INTERFACE_GUID as *const GUID,
            core::ptr::null(),
            core::ptr::null_mut(),
            DIGCF_DEVICEINTERFACE | DIGCF_PRESENT,
        )
    };

    if hdevinfo == INVALID_HANDLE_VALUE as HDEVINFO {
        let err = std::io::Error::last_os_error();
        eprintln!("[DEBUG] SetupDiGetClassDevsW failed: {}", err);
        return Err(err.into());
    }

    let _guard = DevInfoGuard(hdevinfo);
    eprintln!("[DEBUG] SetupDiGetClassDevsW succeeded, enumerating interfaces...");

    let mut index = 0u32;
    loop {
        let mut iface_data = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            InterfaceClassGuid: SCSI_RAW_INTERFACE_GUID,
            Flags: 0,
            Reserved: 0,
        };

        // SAFETY: hdevinfo is valid, iface_data is properly initialized.
        let ok = unsafe {
            SetupDiEnumDeviceInterfaces(
                hdevinfo,
                core::ptr::null_mut(),
                &SCSI_RAW_INTERFACE_GUID as *const GUID,
                index,
                &mut iface_data,
            )
        };

        if ok == 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(259) {
                // ERROR_NO_MORE_ITEMS
                eprintln!("[DEBUG] Enumeration complete at index {}.", index);
                break;
            }
            eprintln!(
                "[DEBUG] SetupDiEnumDeviceInterfaces failed at index {}: {}",
                index, err
            );
            return Err(err.into());
        }

        eprintln!("[DEBUG] Interface {} found, getting detail...", index);

        // Query required buffer size.
        let mut required_size = 0u32;
        unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                hdevinfo,
                &iface_data,
                core::ptr::null_mut(),
                0,
                &mut required_size,
                core::ptr::null_mut(),
            )
        };

        if required_size == 0 {
            eprintln!(
                "[DEBUG] Failed to get required size for interface {}",
                index
            );
            index += 1;
            continue;
        }

        // Allocate buffer: SP_DEVICE_INTERFACE_DETAIL_DATA_W has a flexible array member.
        // On 64-bit: cbSize = 8 (4 bytes cbSize + 4 bytes padding + WCHAR DevicePath[1])
        // On 32-bit: cbSize = 6 (4 bytes cbSize + WCHAR DevicePath[1])
        let mut buffer = vec![0u8; required_size as usize];
        let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;

        #[cfg(target_pointer_width = "64")]
        let cb_size = 8u32;
        #[cfg(target_pointer_width = "32")]
        let cb_size = 6u32;

        unsafe {
            (*detail).cbSize = cb_size;
        }

        // SAFETY: buffer is large enough, detail pointer is valid.
        let ok = unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                hdevinfo,
                &iface_data,
                detail,
                required_size,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };

        if ok == 0 {
            eprintln!(
                "[DEBUG] SetupDiGetDeviceInterfaceDetailW failed for interface {}: {}",
                index,
                std::io::Error::last_os_error()
            );
            index += 1;
            continue;
        }

        // Extract device path from DevicePath field (flexible array of WCHARs).
        let device_path_ptr = unsafe { (*detail).DevicePath.as_ptr() };
        let mut len = 0;
        while unsafe { *device_path_ptr.add(len) } != 0 {
            len += 1;
        }
        let device_path_wide = unsafe { std::slice::from_raw_parts(device_path_ptr, len) };
        let device_path = String::from_utf16_lossy(device_path_wide);

        eprintln!("[DEBUG] Interface {} path: {}", index, device_path);

        // Try to open this device interface path to get the device number.
        let path = std::path::Path::new(&device_path);
        let physical_drive_path = match get_physical_drive_path(path) {
            Ok(p) => {
                eprintln!("[DEBUG]   -> Mapped to: {}", p);
                p
            }
            Err(e) => {
                eprintln!("[DEBUG]   -> Failed to map: {}", e);
                index += 1;
                continue;
            }
        };

        // Now open the PhysicalDrive path and probe it.
        let physical_path = std::path::Path::new(&physical_drive_path);
        match Device::open(physical_path, &OpenOpts::default()) {
            Ok(mut dev) => {
                eprintln!("[DEBUG]   -> Opened successfully, probing...");
                match probe_device_handle(&mut dev, physical_drive_path.clone()) {
                    Ok(info) => {
                        eprintln!(
                            "[DEBUG]   -> INQUIRY success: {} {}",
                            info.vendor, info.product
                        );
                        devices.push(info);
                    }
                    Err(e) => {
                        eprintln!("[DEBUG]   -> INQUIRY failed: {}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("[DEBUG]   -> Open failed: {}", e);
            }
        }

        index += 1;
    }

    eprintln!("[DEBUG] Found {} devices total.", devices.len());
    Ok(devices)
}

/// Open a device interface path and query its PhysicalDrive number.
fn get_physical_drive_path(device_path: &std::path::Path) -> Result<String, Error> {
    // Open the device interface path with read access only.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(device_path)?;

    let handle = file.as_raw_handle();
    let mut dev_num = StorageDeviceNumber {
        device_type: 0,
        device_number: 0,
        partition_number: 0,
    };
    let mut bytes_returned: u32 = 0;

    // SAFETY: handle is valid, dev_num is properly initialized.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            core::ptr::null_mut(),
            0,
            (&mut dev_num as *mut StorageDeviceNumber).cast::<core::ffi::c_void>(),
            size_of::<StorageDeviceNumber>() as u32,
            &mut bytes_returned,
            core::ptr::null_mut::<OVERLAPPED>(),
        )
    };

    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(format!(r"\\.\PhysicalDrive{}", dev_num.device_number))
}

/// RAII guard to ensure SetupDiDestroyDeviceInfoList is called.
struct DevInfoGuard(HDEVINFO);

impl Drop for DevInfoGuard {
    fn drop(&mut self) {
        unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

/// Open a device and issue an INQUIRY command to retrieve identity strings.
fn probe_device_handle(dev: &mut Device, path: String) -> Result<ScsiDeviceInfo, Error> {
    // INQUIRY command: opcode 0x12, allocate 96 bytes.
    let mut cdb = [0u8; MAX_CDB_LEN];
    cdb[0] = 0x12; // INQUIRY
    cdb[4] = 96; // allocation length

    let cmd = ScsiCommand {
        cdb,
        cdb_len: 6,
        direction: Direction::In,
        data: vec![0u8; 96],
        timeout_secs: Some(5),
    };

    let result = dev.execute(cmd)?;

    if result.status != ScsiStatus::Good || result.data.len() < 36 {
        return Err(Error::InvalidParameter(
            "INQUIRY failed or response too short",
        ));
    }

    let inq = &result.data;
    let device_type = ScsiDeviceType::from(inq[0] & 0x1F);

    let vendor = String::from_utf8_lossy(&inq[8..16]).trim().to_string();
    let product = String::from_utf8_lossy(&inq[16..32]).trim().to_string();
    let revision = String::from_utf8_lossy(&inq[32..36]).trim().to_string();

    Ok(ScsiDeviceInfo {
        path,
        device_type,
        vendor,
        product,
        revision,
    })
}

// ── Test helpers ─────────────────────────────────────────────────────────────

/// Open a temp file so tests can hold a valid `Device` without a real SCSI node.
/// Any `DeviceIoControl` call against this handle will fail at the IOCTL level,
/// but input validation in the public API fires before the IOCTL is issued.
#[cfg(test)]
impl Device {
    pub(crate) fn new_test() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join("libscsi_test_dummy.tmp");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;
        Ok(Device { file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin the SPTD struct size to the Windows SDK layout (56 bytes on 64-bit).
    /// If this ever changes, the IOCTL ABI breaks silently without this guard.
    #[test]
    fn sptd_size_matches_sdk() {
        assert_eq!(
            size_of::<ScsiPassThroughDirect>(),
            56,
            "ScsiPassThroughDirect must be 56 bytes on 64-bit Windows"
        );
    }

    /// `sense_info_offset` must equal `offset_of!(SptdBuffer, sense)` — the
    /// value we store in the struct at runtime — so verify the offset is
    /// exactly what the kernel expects (56).
    #[test]
    fn sptd_buffer_sense_offset() {
        assert_eq!(
            core::mem::offset_of!(SptdBuffer, sense),
            56,
            "sense data must start immediately after ScsiPassThroughDirect"
        );
    }
}
