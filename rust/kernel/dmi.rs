// SPDX-License-Identifier: GPL-2.0

//! Abstractions for DMI (Desktop Management Interface).
//!
//! C header: [`include/linux/dmi.h`](srctree/include/linux/dmi.h)

use crate::{
    bindings,
    prelude::*, //
};

/// Corresponds to each DMI identifier type from [`enum dmi_field`].
///
/// # Invariant
///
/// Each variant must correspond to a valid [`enum dmi_field`] variant.
///
/// [`enum dmi_field`]: srctree/include/linux/device-id/dmi.h
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(missing_docs, reason = "C version of these consts are undocumented")]
pub enum Field {
    None = bindings::dmi_field_DMI_NONE,
    BiosVendor = bindings::dmi_field_DMI_BIOS_VENDOR,
    BiosVersion = bindings::dmi_field_DMI_BIOS_VERSION,
    BiosDate = bindings::dmi_field_DMI_BIOS_DATE,
    BiosRelease = bindings::dmi_field_DMI_BIOS_RELEASE,
    EcFirmwareRelease = bindings::dmi_field_DMI_EC_FIRMWARE_RELEASE,
    SysVendor = bindings::dmi_field_DMI_SYS_VENDOR,
    ProductName = bindings::dmi_field_DMI_PRODUCT_NAME,
    ProductVersion = bindings::dmi_field_DMI_PRODUCT_VERSION,
    ProductSerial = bindings::dmi_field_DMI_PRODUCT_SERIAL,
    ProductUuid = bindings::dmi_field_DMI_PRODUCT_UUID,
    ProductSku = bindings::dmi_field_DMI_PRODUCT_SKU,
    ProductFamily = bindings::dmi_field_DMI_PRODUCT_FAMILY,
    BoardVendor = bindings::dmi_field_DMI_BOARD_VENDOR,
    BoardName = bindings::dmi_field_DMI_BOARD_NAME,
    BoardVersion = bindings::dmi_field_DMI_BOARD_VERSION,
    BoardSerial = bindings::dmi_field_DMI_BOARD_SERIAL,
    BoardAssetTag = bindings::dmi_field_DMI_BOARD_ASSET_TAG,
    ChassisVendor = bindings::dmi_field_DMI_CHASSIS_VENDOR,
    ChassisType = bindings::dmi_field_DMI_CHASSIS_TYPE,
    ChassisVersion = bindings::dmi_field_DMI_CHASSIS_VERSION,
    ChassisSerial = bindings::dmi_field_DMI_CHASSIS_SERIAL,
    ChassisAssetTag = bindings::dmi_field_DMI_CHASSIS_ASSET_TAG,
    OemString = bindings::dmi_field_DMI_OEM_STRING,
}

/// Returns the DMI string for `field`, or [`None`] if unavailable or `CONFIG_DMI` is disabled.
///
/// # Examples
///
/// ```
/// # use kernel::dmi::{self, Field};
/// if let Some(vendor) = dmi::get_system_info(Field::SysVendor) {
///     pr_debug!("Vendor string is {vendor}");
/// }
/// ```
pub fn get_system_info(field: Field) -> Option<&'static CStr> {
    // TODO: Replace with cfg-select post 1.95.
    #[cfg(CONFIG_DMI)]
    {
        // SAFETY: `field` is a valid variant of `enum dmi_field`.
        let ptr = unsafe { bindings::dmi_get_system_info(field as c_int) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: If non-null, `dmi_get_system_info` returns a C string that
        // lives for the life of the kernel.
        Some(unsafe { CStr::from_char_ptr(ptr) })
    }
    #[cfg(not(CONFIG_DMI))]
    {
        // Squash `unused_variables` error.
        let _ = field;
        None
    }
}
