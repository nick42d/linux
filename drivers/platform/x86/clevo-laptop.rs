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
    self,
    device,
    dmi,
    led::{
        self,
        LedOps,
        Normal, //
    },
    prelude::*,
    time::{
        delay::fsleep,
        Delta, //
    },
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
const CLEVO_EC_CMD_GET_SPECS: u8 = 0x0D;
const CLEVO_EC_CMD_GET_BIOS_FEATURES_1: u8 = 0x52;
const CLEVO_EC_CMD_GET_BIOS_FEATURES_2: u8 = 0x7A;
const CLEVO_EC_CMD_SET_KB_WHITE_LEDS: u8 = 0x27;
const CLEVO_BIOS_FEATURE_CODE_WHITE_ONLY_KB: u32 = 0x4000_0000;
const CLEVO_BIOS_FEATURE_CODE_3_ZONE_RGB_KB: u32 = 0x0040_0000;
const CLEVO_BIOS_FEATURE_CODE_WHITE_ONLY_KB_MAX_5: u32 = 0x4000;
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
    _wmi_driver: wmi::Registration<WmiDriver<'static>>,
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
struct WmiDriver<'a> {
    _led: LedRegistration<'a>,
}

kernel::wmi_device_table!(WMI_TABLE, [wmi::DeviceId::new(CLEVO_WMI_METHOD_GUID)]);

impl wmi::Driver for WmiDriver<'_> {
    type Data<'bound> = WmiDriver<'bound>;
    const ID_TABLE: wmi::IdTable = &WMI_TABLE;
    fn probe<'bound>(
        dev: &'bound wmi::Device<device::Core<'_>>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        pin_init::pin_init_scope(move || {
            let backend = EcInterface::Wmi(dev);

            backend.query(GetBiosFeatures1).map_err(|_| ENODEV)?;

            // MVP: Only the NL57AU 5-step white backlight is supported.
            if backend.detect_backlight_type()? != Some(KbBacklightType::FixedColor) {
                return Err(ENODEV);
            }
            let has_white_5 = backend
                .query(GetBiosFeatures2)
                .is_ok_and(|feat2| feat2.has_white_only_kb_max_5());
            if !has_white_5 {
                return Err(ENODEV);
            }

            if let Err(e) = backend.query(SetWhiteBrightness(Brightness::OFF)) {
                dev_dbg!(
                    dev,
                    "Error {e:?} setting keyboard brightness on initialization.\n"
                );
            }

            let led_dev = KBox::pin_init(
                led::DeviceBuilder::new()
                    .name(c"white:kbd_backlight")
                    .color(led::Color::White)
                    .build(
                        dev.as_ref(),
                        try_pin_init!(FixedColorLedOperations { backend }),
                    ),
                GFP_KERNEL,
            )?;

            Ok(try_pin_init!(WmiDriver {
                _led: LedRegistration::White5Step { _dev: led_dev },
            }))
        })
    }
}

/// RAII guard for Clevo LED device holding the appropriate sysfs device for the backlight type.
enum LedRegistration<'a> {
    White5Step {
        _dev: Pin<
            KBox<led::Device<'a, FixedColorLedOperations<'a, { Brightness::MAX_WHITE_5.raw() }>>>,
        >,
    },
}

/// Represents how to perform LED operations for a Clevo single-color LED keyboard, generic over
/// `MAX_BRIGHTNESS`.
#[pin_data]
struct FixedColorLedOperations<'a, const MAX_BRIGHTNESS: u8> {
    backend: EcInterface<'a>,
}
#[vtable]
impl<'a, const MAX_BRIGHTNESS: u8> LedOps for FixedColorLedOperations<'a, MAX_BRIGHTNESS> {
    const BLOCKING: bool = true;
    const MAX_BRIGHTNESS: u32 = MAX_BRIGHTNESS as u32;
    type Mode = Normal;

    fn brightness_set<'bound>(
        self: &<Self::Mode as led::Mode>::Device<'bound, Self>,
        brightness: u32,
    ) -> Result<()> {
        let clamped = u8::try_from(brightness)
            .unwrap_or(MAX_BRIGHTNESS)
            .min(MAX_BRIGHTNESS);
        self.backend
            .query(SetWhiteBrightness(Brightness::new(clamped)))
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
    /// Detect backlight type e.g white.
    fn detect_backlight_type(&self) -> Result<Option<KbBacklightType>> {
        const MAX_RETRIES: usize = 3;
        const RETRY_SLEEP_DURATION: Delta = Delta::from_millis(50);

        for attempt in 0..MAX_RETRIES {
            if attempt != 0 {
                pr_debug!(
                    "KbBacklightType::None probably wrong, retrying - attempt {} of {}\n",
                    attempt,
                    MAX_RETRIES - 1
                );
                fsleep(RETRY_SLEEP_DURATION);
            }
            if let Ok(specs) = self.query(GetSpecs) {
                let backlight_type = specs.kb_backlight_type();
                if backlight_type != Ok(None) {
                    return backlight_type.map_err(|_| ENODEV);
                }
            }
        }

        // Check for devices <= Intel 8th gen (only white only, 3 zone RGB,
        // or no backlight on these devices)
        let feat1 = self.query(GetBiosFeatures1).map_err(|_| ENODEV)?;
        if feat1.has_3_zone_rgb_kb() {
            Ok(Some(KbBacklightType::ThreeZoneRgb))
        } else if feat1.has_white_only_kb() {
            Ok(Some(KbBacklightType::FixedColor))
        } else {
            Ok(None)
        }
    }
}

/// Keyboard backlight brightness level.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
struct Brightness(u8);
impl Brightness {
    const OFF: Self = Self(0);
    const MAX_WHITE_5: Self = Self(5);
    const fn new(val: u8) -> Self {
        Self(val)
    }
    const fn raw(self) -> u8 {
        self.0
    }
}

/// Keyboard backlight type - e.g Fixed Color
#[derive(PartialEq)]
#[repr(u8)]
enum KbBacklightType {
    FixedColor = 0x01,
    ThreeZoneRgb = 0x02,
    OneZoneRgb = 0x06,
    PerKeyRgb = 0xF3,
}
impl KbBacklightType {
    const fn from_raw(raw: u8) -> Result<Option<Self>, u8> {
        match raw {
            0x01 => Ok(Some(KbBacklightType::FixedColor)),
            0x02 => Ok(Some(KbBacklightType::ThreeZoneRgb)),
            0x06 => Ok(Some(KbBacklightType::OneZoneRgb)),
            0xF3 => Ok(Some(KbBacklightType::PerKeyRgb)),
            0x00 => Ok(None),
            other => Err(other),
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
struct SetWhiteBrightness(Brightness);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GetSpecs;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GetBiosFeatures1;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GetBiosFeatures2;

impl EcQuery for SetWhiteBrightness {
    type Output = ();
    fn cmd(&self) -> u8 {
        CLEVO_EC_CMD_SET_KB_WHITE_LEDS
    }
    fn args(&self) -> EcMethodArgs {
        EcMethodArgs::Scalar(self.0.raw().into())
    }
    fn deserialize_output(_output: &[u8]) -> Result<Self::Output> {
        Ok(())
    }
}
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
impl EcQuery for GetBiosFeatures2 {
    type Output = BiosFeatures2;
    fn cmd(&self) -> u8 {
        CLEVO_EC_CMD_GET_BIOS_FEATURES_2
    }
    fn args(&self) -> EcMethodArgs {
        EcMethodArgs::Scalar(0)
    }
    fn deserialize_output(output: &[u8]) -> Result<Self::Output> {
        let (feat, _) = BiosFeatures2::read_from_prefix(output).map_err(|_| ENODATA)?;
        Ok(feat)
    }
}
impl EcQuery for GetSpecs {
    type Output = Specs;
    fn cmd(&self) -> u8 {
        CLEVO_EC_CMD_GET_SPECS
    }
    fn args(&self) -> EcMethodArgs {
        EcMethodArgs::Scalar(0)
    }
    fn deserialize_output(output: &[u8]) -> Result<Self::Output> {
        let (specs, _) = Specs::read_from_prefix(output).map_err(|_| ENODATA)?;
        Ok(specs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes)]
struct BiosFeatures1(u32);
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes)]
struct BiosFeatures2(u32);
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes)]
struct Specs([u8; 16]);

impl BiosFeatures1 {
    const INVALID: Self = Self(0xFFFF_FFFF);
    const fn has_white_only_kb(self) -> bool {
        (self.0 & CLEVO_BIOS_FEATURE_CODE_WHITE_ONLY_KB) != 0
    }
    const fn has_3_zone_rgb_kb(self) -> bool {
        (self.0 & CLEVO_BIOS_FEATURE_CODE_3_ZONE_RGB_KB) != 0
    }
}
impl BiosFeatures2 {
    const fn has_white_only_kb_max_5(self) -> bool {
        (self.0 & CLEVO_BIOS_FEATURE_CODE_WHITE_ONLY_KB_MAX_5) != 0
    }
}
impl Specs {
    const fn kb_backlight_type(&self) -> Result<Option<KbBacklightType>, u8> {
        KbBacklightType::from_raw(self.0[0x0F])
    }
}
