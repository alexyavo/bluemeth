//! Hand-written declarations for the handful of IOKit functions we use.
//!
//! Signatures mirror `IOKit/pwr_mgt/IOPMLib.h`. We bind only what we call
//! instead of pulling in a full IOKit binding crate.

#![allow(non_upper_case_globals, missing_docs)]

use objc2_core_foundation::{CFDictionary, CFString, CFType};

/// `IOReturn` is a `kern_return_t`, i.e. a C `int`.
pub type IOReturn = i32;
/// Opaque per-process assertion handle.
pub type IOPMAssertionID = u32;
/// `IOPMUserActiveType` is a C enum; `kIOPMUserActiveLocal = 0`.
pub type IOPMUserActiveType = u32;

pub const K_IOPM_USER_ACTIVE_LOCAL: IOPMUserActiveType = 0;
pub const K_IOPM_NULL_ASSERTION_ID: IOPMAssertionID = 0;

pub const K_IORETURN_SUCCESS: IOReturn = 0;
/// `kIOReturnNotFound` (0xe00002f0).
pub const K_IORETURN_NOT_FOUND: IOReturn = 0xe000_02f0_u32 as i32;
/// `kIOReturnBadArgument` (0xe00002c2). Observed from `IOPMAssertionRelease`
/// when the id was already dropped by powerd's own timeout handling.
pub const K_IORETURN_BAD_ARGUMENT: IOReturn = 0xe000_02c2_u32 as i32;

/// Dictionary keys and values from `IOPMLib.h`, as plain string literals.
pub mod keys {
    pub const ASSERTION_TYPE: &str = "AssertType";
    pub const ASSERTION_NAME: &str = "AssertName";
    pub const DETAILS: &str = "Details";
    pub const HUMAN_READABLE_REASON: &str = "HumanReadableReason";
    pub const TIMEOUT_SECONDS: &str = "TimeoutSeconds";
    pub const TIMEOUT_ACTION: &str = "TimeoutAction";
    pub const TIMEOUT_ACTION_RELEASE: &str = "TimeoutActionRelease";
    /// Assertion type reported for `IOPMAssertionDeclareUserActivity`.
    pub const TYPE_USER_IS_ACTIVE: &str = "UserIsActive";
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    /// Create an assertion from a property dictionary (type, name, timeout…).
    pub fn IOPMAssertionCreateWithProperties(
        assertion_properties: &CFDictionary<CFString, CFType>,
        assertion_id: *mut IOPMAssertionID,
    ) -> IOReturn;

    /// Release an assertion. Returns `kIOReturnNotFound` if `powerd` already
    /// dropped it (e.g. its timeout fired).
    pub fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;

    /// Declare that the user is active. Pass the previously returned id to
    /// extend an existing declaration; the id may change between calls.
    pub fn IOPMAssertionDeclareUserActivity(
        assertion_name: &CFString,
        user_type: IOPMUserActiveType,
        assertion_id: *mut IOPMAssertionID,
    ) -> IOReturn;

    /// Copy the system-wide assertion table, keyed by owning pid.
    pub fn IOPMCopyAssertionsByProcess(assertions_by_pid: *mut *const CFDictionary) -> IOReturn;
}
