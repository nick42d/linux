// SPDX-License-Identifier: GPL-2.0

//! Clevo Laptop Driver.
//!
//! Currently supports only the 5-step white keyboard backlight on Clevo
//! NLx0AU boards (e.g., Metabox Edge NL57AU).
//!
//! Copyright (C) 2026 Nick Dowsett <nick42d@gmail.com>.
//!
//! Inspired by Tuxedo Computers' drivers.

use kernel::{
    device,
    dmi,
    prelude::*,
    wmi, //
};

/// # Note
///
/// This GUID is non-unique (shared by Uniwill for example) and cannot be trusted
/// to uniquely identify a Clevo device.
const CLEVO_WMI_METHOD_GUID: &CStr = c"ABBC0F6D-8EA1-11D1-00A0-C90629100000";
/// DMI board name allowlist, restricted to tested boards since the WMI GUID is not
/// unique.
const SUPPORTED_BOARDS: &[&CStr] = &[c"NLx0AU"];

module! {
    type: ClevoLaptopModule,
    name: "clevo-laptop",
    authors: ["Nick Dowsett <nick42d@gmail.com>"],
    description: "Clevo Laptop Driver (Keyboard Backlight)",
    license: "GPL",
}

/// RAII guard for Clevo Laptop Module holding all subsystem drivers e.g WMI.
#[pin_data]
struct ClevoLaptopModule {
    #[pin]
    _wmi_driver: wmi::Registration<WmiDriver>,
}

impl kernel::InPlaceModule for ClevoLaptopModule {
    fn init(module: &'static ThisModule) -> impl PinInit<Self, Error> {
        pin_init::pin_init_scope(move || {
            if dmi::get_system_info(dmi::Field::BoardName)
                .is_none_or(|board| !SUPPORTED_BOARDS.contains(&board))
            {
                return Err(ENODEV);
            }
            Ok(try_pin_init!(Self {
                _wmi_driver <- wmi::Registration::new(c"clevo-laptop", module),
            }))
        })
    }
}

/// RAII guard for Clevo WMI driver holding all subsystem devices e.g LED device.
#[pin_data]
struct WmiDriver {}

kernel::wmi_device_table!(WMI_TABLE, [wmi::DeviceId::new(CLEVO_WMI_METHOD_GUID)]);

impl wmi::Driver for WmiDriver {
    type Data<'bound> = WmiDriver;
    const ID_TABLE: wmi::IdTable = &WMI_TABLE;
    fn probe<'bound>(
        _dev: &'bound wmi::Device<device::Core<'_>>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        try_pin_init!(WmiDriver {})
    }
}
