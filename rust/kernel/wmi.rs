// SPDX-License-Identifier: GPL-2.0

//! Abstractions for WMI (Windows Management Instrumentation).
//!
//! C header: [`include/linux/wmi.h`](srctree/include/linux/wmi.h)

use crate::{
    bindings,
    device,
    device_id::RawDeviceId,
    driver,
    error,
    prelude::*,
    sync::aref::AlwaysRefCounted,
    types::Opaque,
    ThisModule, //
};
use core::{
    marker::PhantomData,
    ptr::NonNull, //
};

/// Type alias for [`driver::Registration<Adapter<T>>`], the registration of
/// a WMI driver wrapping a [`struct wmi_driver`].
///
/// When this struct is dropped the driver will be unregistered.
///
/// [`struct wmi_driver`]: srctree/include/linux/wmi.h
pub type Registration<T> = driver::Registration<Adapter<T>>;

/// WMI driver trait.
///
/// Drivers must implement this trait in order to register a WMI driver.
///
/// # Examples
///
/// ```
/// # use kernel::{device::Core, prelude::*, wmi};
/// struct MyDriver;
///
/// kernel::wmi_device_table!(
///     MY_DRIVER_WMI_TABLE,
///     [wmi::DeviceId::new(c"00000000-0000-0000-0000-000000000000")]
/// );
///
/// impl wmi::Driver for MyDriver {
///     type Data<'bound> = Self;
///     const ID_TABLE: wmi::IdTable = &MY_DRIVER_WMI_TABLE;
///
///     fn probe<'bound>(
///         _dev: &'bound wmi::Device<Core<'_>>,
///     ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
///         Err(ENODEV)
///     }
/// }
/// ```
pub trait Driver {
    /// The type of the driver's bus device private data.
    type Data<'bound>: Send + 'bound;
    /// The table of WMI device IDs supported by the driver.
    const ID_TABLE: IdTable;
    /// The `probe` callback is called when the WMI driver is bound to a matching WMI device.
    /// Allocating driver-specific data structures and initializing interfaces to other kernel
    /// subsystems should normally be done in this function.
    fn probe<'bound>(
        dev: &'bound Device<device::Core<'_>>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound;
}

/// Adapter to allow [`Driver`] to be used with [`driver::Registration`].
pub struct Adapter<T: Driver>(T);

// SAFETY:
// - `wmi_driver` is a C struct.
// - Since `T` is a driver, `T::Data` is by definition the type of its
//   device private data.
// - `wmi_driver` embeds a `device_driver` at offset `DEVICE_DRIVER_OFFSET`
//   as per `core::mem::offset_of!()`.
unsafe impl<T: Driver> driver::DriverLayout for Adapter<T> {
    type DriverType = bindings::wmi_driver;
    type DriverData<'bound> = T::Data<'bound>;
    const DEVICE_DRIVER_OFFSET: usize = core::mem::offset_of!(Self::DriverType, driver);
}

// SAFETY: It's always valid to unregister a registered `wmi_driver`.
unsafe impl<T: Driver> driver::RegistrationOps for Adapter<T> {
    unsafe fn register(
        reg: &Opaque<Self::DriverType>,
        name: &'static CStr,
        module: &'static ThisModule,
    ) -> Result {
        // SAFETY:
        // - It's safe to set the fields of `wmi_driver` on initialization.
        // - All set fields meet the invariants of `wmi_driver`.
        unsafe {
            // SAFETY: `name` is valid for the static lifetime.
            (*reg.get()).driver.name = name.as_char_ptr();
            // SAFETY: This callback will only be called with a pointer to a `wmi_device`
            // valid for the lifetime of the callback.
            (*reg.get()).probe = Some(Self::probe_callback);
            // SAFETY: `T::ID_TABLE` is valid for the static lifetime.
            (*reg.get()).id_table = T::ID_TABLE.as_ptr();
            // Per the documentation, new WMI drivers should be allowed to be instantiated
            // multiple times (see `Documentation/wmi/driver-development-guide.rst`).
            (*reg.get()).no_singleton = true;
        }

        // SAFETY: `reg` is a valid `wmi_driver`.
        error::to_result(unsafe { bindings::__wmi_driver_register(reg.get(), module.as_ptr()) })
    }
    unsafe fn unregister(wdrv: &Opaque<Self::DriverType>) {
        // SAFETY: `wdrv` is a valid `wmi_driver` previously registered with `register()`.
        unsafe { bindings::wmi_driver_unregister(wdrv.get()) };
    }
}

impl<T: Driver> Adapter<T> {
    /// C wrapper for [`Driver::probe`] function to use it as a C function pointer.
    ///
    /// # Safety
    /// - The caller must ensure that `wdev` is a pointer to a `wmi_device` valid for
    ///   the lifetime of the callback.
    unsafe extern "C" fn probe_callback(
        wdev: *mut bindings::wmi_device,
        _context: *const c_void,
    ) -> c_int {
        // SAFETY:
        // - As per the safety requirement for the function, `wdev` is a pointer to a
        //   `struct wmi_device` valid for the lifetime of the callback.
        //   This is the definition of the Core and CoreInternal marker lifetime.
        // - `Device` is `#[repr(transparent)]` over `struct wmi_device`.
        let wdev = unsafe { &*wdev.cast::<Device<device::CoreInternal<'_>>>() };
        let data = T::probe(wdev);
        // As per the C driver subsystem documentation, the return type of a driver
        // probe callback is 0 on success or a kernel error code on failure.
        error::from_result(|| {
            wdev.as_ref().set_drvdata(data)?;
            Ok(0)
        })
    }
}

/// IdTable type for WMI drivers.
pub type IdTable = &'static dyn crate::device_id::IdTable<DeviceId, ()>;

/// A WMI device ID wrapping [`struct wmi_device_id`].
///
/// [`struct wmi_device_id`]: srctree/include/linux/device-id/wmi.h
///
/// # Invariants
/// `self.0.guid_string` is a valid, null-terminated string of length [`bindings::UUID_STRING_LEN`].
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct DeviceId(bindings::wmi_device_id);
// SAFETY: `DeviceId` is a `#[repr(transparent)]` wrapper of `wmi_device_id`.
// `wmi_device_id` does not have any invariants, so it's safe to transmute to it.
unsafe impl RawDeviceId for DeviceId {
    type RawType = bindings::wmi_device_id;
}
impl DeviceId {
    const WMI_UUID_LEN: usize = crate::num::casts::u32_as_usize(bindings::UUID_STRING_LEN);
    /// Creates a new [`DeviceId`].
    ///
    /// Fails at compile time if `guid.len()` is not [`bindings::UUID_STRING_LEN`].
    #[inline(always)]
    pub const fn new(guid: &'static CStr) -> Self {
        let src = guid.to_bytes_with_nul();
        // NOTE: Function is marked `#[inline(always)]` to help compiler optimise the error path.
        build_assert!(
            src.len() == Self::WMI_UUID_LEN + 1,
            "WMI DeviceId length incorrect"
        );
        let mut id: bindings::wmi_device_id = pin_init::zeroed();
        // TODO: Upgrade to core::slice::copy_from_slice(src) once rustc>=1.87
        let mut i = 0;
        while i < src.len() {
            id.guid_string[i] = src[i];
            i += 1;
        }
        // INVARIANT:
        // - All bytes were copied from a Rust `&'static CStr`.
        // - The build assert above guarantees `id` to meet the length invariant.
        Self(id)
    }
}

/// Create a WMI [`IdTable`] with an "alias" for modpost.
#[macro_export]
macro_rules! wmi_device_table {
    ($table_name:ident, [$($id:expr),* $(,)?]) => {
        $crate::module_device_table!(
            "wmi",
            $crate::wmi::DeviceId,
            $table_name,
            @none,
            [$($id),*]
        );
    };
}

/// A WMI bus device ([`struct wmi_device`]).
///
/// # Invariants
/// - `Device` wraps an in-place `Opaque<bindings::wmi_device>` allocated and always
///   reference-counted in a thread-safe way by the C subsystem.
/// - Instances are never constructed or moved by value in Rust, they are only accessed behind
///   `&Device<Ctx>` or `ARef<Device<device::Normal>>`.
///
/// [`struct wmi_device`]: srctree/include/linux/wmi.h
#[repr(transparent)]
pub struct Device<Ctx: device::DeviceContext = device::Normal> {
    ptr: Opaque<bindings::wmi_device>,
    _marker: PhantomData<Ctx>,
}

// SAFETY: `Device<Ctx>` is a transparent wrapper of `Opaque<bindings::wmi_device>` whose
// representation does not depend on `Ctx`.
crate::impl_device_context_deref!(unsafe { Device });
crate::impl_device_context_into_aref!(Device);

// SAFETY: By the invariant, `Device<Normal>` is never moved by value in Rust.
// If sent to another thread as `ARef<Device<Normal>>`, the reference-counting
// is performed in a thread-safe way.
unsafe impl Send for Device<device::Normal> {}

// SAFETY:
// - All that `&Device<Normal>` can do is increment its reference count (thread-safe)
// and perform a reference conversion to `&device::Device<Normal>`.
// - All methods on `&device::Device<Normal>` are thread safe.
unsafe impl Sync for Device<device::Normal> {}

// SAFETY:
// - By the type invariant, all instances of `Device` are reference-counted.
// - Increments to the reference count keep the object alive in memory at least
//   until matching decrements (`dec_ref` via `put_device`) are performed.
unsafe impl AlwaysRefCounted for Device<device::Normal> {
    #[inline]
    fn inc_ref(&self) {
        self.as_ref().inc_ref();
    }
    #[inline]
    unsafe fn dec_ref(obj: NonNull<Self>) {
        // CAST: `Device` is a `#[repr(transparent)]` wrapper of `Opaque<bindings::wmi_device>`.
        let wdev: *mut bindings::wmi_device = obj.cast().as_ptr();
        // SAFETY:
        // - The safety requirements of `dec_ref` guarantee that `obj` is a valid pointer with a
        //   non-zero reference count.
        // - `put_device` safely decrements the refcount and triggers release cleanup when it
        //   reaches 0.
        unsafe { bindings::put_device(&raw mut (*wdev).dev) };
    }
}
impl<Ctx: device::DeviceContext> Device<Ctx> {
    /// Returns the raw pointer to the underlying `bindings::wmi_device`.
    #[inline]
    fn as_raw(&self) -> *mut bindings::wmi_device {
        self.ptr.get()
    }
}

impl<Ctx: device::DeviceContext> AsRef<device::Device<Ctx>> for Device<Ctx> {
    #[inline]
    fn as_ref(&self) -> &device::Device<Ctx> {
        // SAFETY:
        // - By the type invariant on `Device<Ctx>`, `self.as_raw()` is a valid pointer to
        //   a `wmi_device` which embeds a `device`.
        // - If you have a reference to a `Device<Ctx>`, then the reference count must be
        //   non-zero.
        // - If you have a reference to a `Device<Ctx>`, it is either:
        //   - Borrowed from an `ARef<Device<Normal>>` that cannot be dropped and decrement
        //     reference count to zero whilst the reference is held.
        //   - Derived from a scope (such as a callback) that guarantees that reference count
        //     will not be reduced to zero whilst that reference is held.
        unsafe { device::Device::from_raw(&raw mut (*self.as_raw()).dev) }
    }
}
