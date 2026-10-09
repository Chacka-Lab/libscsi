# libscsi

Cross-platform SCSI intermediate layer.

But not ready yet.

## Find Devices

This project does not implement the ability to enumerate SCSI devices. This is because trying to enumerate “SCSI devices” varies greatly across platforms, and the implementations are often not very elegant. For example, on Windows, the same device instance can have multiple device interfaces, so enumerating SCSI device instances is easy (using the PnP enumerator is sufficient), but to enumerate its device interfaces one must provide a GUID. GUIDs are divided by device class and there are many of them; enumerating them one by one would degrade performance to an unacceptable level. Even if one managed to enumerate them, the device class field would still be troublesome: different systems divide device classes very differently, with only a small overlap. Therefore, I suggest that each project implement device enumeration on its own. For example, on Windows, enumerating the device interfaces of tape drives only requires querying with the corresponding tape drive GUID (GUID_DEVINTERFACE_TAPE), which is very convenient but specialized.
