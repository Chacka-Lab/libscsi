use libscsi::list_devices;
fn main() {
    match list_devices() {
        Ok(devices) => {
            if devices.is_empty() {
                println!("No SCSI devices found.");
            } else {
                println!("Found {} SCSI device(s):\n", devices.len());
                for dev in devices {
                    println!("  Path: {}", dev.path);
                    println!("  Type: {:?}", dev.device_type);
                    println!("  Vendor: {}", dev.vendor);
                    println!("  Product: {}", dev.product);
                    println!("  Revision: {}\n", dev.revision);
                }
            }
        }
        Err(e) => {
            panic!("Error enumerating devices: {}", e);
        }
    }
}
