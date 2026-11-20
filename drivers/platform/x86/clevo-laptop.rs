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
const CLEVO_EC_CMD_GET_BIOS_FEATURES_1: u8 = 0x52;
const CLEVO_EC_WMI_INSTANCE: u8 = 0;

module! {
    type: ClevoLaptopModule,
    name: "clevo-laptop",
    authors: ["Nick Dowsett <nick42d@gmail.com>"],
    description: "Clevo Laptop Driver (Keyboard Backlight)",
    license: "GPL",
}

/// Represents a query that can run on the Clevo Embedded Controller (EC).
///
/// # Endianness note
/// Clevo EC sends and receives little endian data.
/// As this is a `platform/x86` driver, no conversion is required.
trait EcQuery {
    type Output;
    /// EC method code.
    fn cmd(&self) -> u8;
    /// EC method args.
    fn args(&self) -> EcMethodArgs;
    /// Each command must know how to deserialize itself from the raw EC output.
    fn deserialize_output(output: &[u8]) -> Result<Self::Output>;
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
        dev: &'bound wmi::Device<device::Core<'_>>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        pin_init::pin_init_scope(move || {
            let backend = EcInterface::Wmi(dev);

            backend.query(GetBiosFeatures1).map_err(|_| ENODEV)?;

            Ok(try_pin_init!(WmiDriver {}))
        })
    }
}

/// Interface to Clevo Embedded Controller.
enum EcInterface<'a> {
    Wmi(&'a wmi::Device<device::Bound>),
}
impl<'a> EcInterface<'a> {
    /// Send a query to the EC, receiving a response.
    fn query<Q>(&self, query: Q) -> Result<Q::Output>
    where
        Q: EcQuery + core::fmt::Debug,
        Q::Output: core::fmt::Debug,
    {
        let cmd = query.cmd();
        let arg = query.args();

        match self {
            Self::Wmi(wdev) => {
                dev_dbg!(
                    wdev.as_ref(),
                    "ec cmd {query:?} method_id 0x{cmd:02x} arg {arg:x?}\n"
                );
                let buf = wdev.invoke_method(
                    CLEVO_EC_WMI_INSTANCE,
                    cmd.into(),
                    arg.as_bytes(),
                    core::mem::size_of::<Q::Output>(),
                )?;

                let res = Q::deserialize_output(&buf)?;
                dev_dbg!(wdev.as_ref(), "ec cmd {query:?} -> {res:?}\n");
                Ok(res)
            }
        }
    }
}

/// Args for a command sent to the EC.
#[derive(Debug)]
enum EcMethodArgs {
    Scalar(u32),
}
impl EcMethodArgs {
    /// Convert to the type expected by the EC.
    fn as_bytes(&self) -> &[u8] {
        match self {
            // Clevo EC expects little-endian integers; x86 is always little-endian.
            Self::Scalar(val) => val.as_bytes(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GetBiosFeatures1;

impl EcQuery for GetBiosFeatures1 {
    type Output = BiosFeatures1;
    fn cmd(&self) -> u8 {
        CLEVO_EC_CMD_GET_BIOS_FEATURES_1
    }
    fn args(&self) -> EcMethodArgs {
        EcMethodArgs::Scalar(0)
    }
    fn deserialize_output(output: &[u8]) -> Result<Self::Output> {
        let (feat, _) = BiosFeatures1::read_from_prefix(output).map_err(|_| ENODATA)?;
        if feat == BiosFeatures1::INVALID {
            return Err(ENODEV);
        }
        Ok(feat)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes)]
struct BiosFeatures1(u32);

impl BiosFeatures1 {
    const INVALID: Self = Self(0xFFFF_FFFF);
}
