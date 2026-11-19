// SPDX-License-Identifier: GPL-2.0

//! Abstractions for WMI (Windows Management Instrumentation).
//!
//! C header: [`include/linux/wmi.h`](srctree/include/linux/wmi.h)

use crate::{
    bindings,
    device,
    sync::aref::AlwaysRefCounted,
    types::Opaque, //
};
use core::{
    marker::PhantomData,
    ptr::NonNull, //
};

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
        //   a `wmi_device which embeds a `device`.
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
