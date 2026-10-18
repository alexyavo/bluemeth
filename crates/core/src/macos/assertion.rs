use std::time::Duration;

use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};

use super::ffi::{self, IOPMAssertionID, keys};
use crate::{AssertionKind, Error, Result};

/// One IOKit power assertion, released on drop.
#[derive(Debug)]
pub struct Assertion {
    id: IOPMAssertionID,
}

fn as_cf_type<T: AsRef<CFType>>(value: &T) -> &CFType {
    value.as_ref()
}

impl Assertion {
    /// Create and immediately activate an assertion.
    ///
    /// When `timeout` is given it is attached as `TimeoutSeconds` with
    /// `TimeoutActionRelease`, so `powerd` drops the assertion on its own
    /// even if this process is wedged. This is what `caffeinate -t` does.
    pub fn create(
        kind: AssertionKind,
        name: &str,
        details: &str,
        reason: &str,
        timeout: Option<Duration>,
    ) -> Result<Self> {
        let k_type = CFString::from_static_str(keys::ASSERTION_TYPE);
        let k_name = CFString::from_static_str(keys::ASSERTION_NAME);
        let k_details = CFString::from_static_str(keys::DETAILS);
        let k_reason = CFString::from_static_str(keys::HUMAN_READABLE_REASON);
        let k_timeout = CFString::from_static_str(keys::TIMEOUT_SECONDS);
        let k_action = CFString::from_static_str(keys::TIMEOUT_ACTION);

        let v_type = CFString::from_static_str(kind.iokit_type());
        let v_name = CFString::from_str(name);
        let v_details = CFString::from_str(details);
        let v_reason = CFString::from_str(reason);
        let v_action = CFString::from_static_str(keys::TIMEOUT_ACTION_RELEASE);

        let mut dict_keys: Vec<&CFString> = vec![&k_type, &k_name, &k_details, &k_reason];
        let mut dict_values: Vec<&CFType> = vec![
            as_cf_type(&v_type),
            as_cf_type(&v_name),
            as_cf_type(&v_details),
            as_cf_type(&v_reason),
        ];

        let v_timeout: Option<CFRetained<CFNumber>> = timeout.map(|t| {
            // powerd treats a timeout of 0 as "no timeout"; a positive
            // sub-second value is honoured, so never round down to zero.
            CFNumber::new_f64(t.as_secs_f64().max(0.001))
        });
        if let Some(v_timeout) = v_timeout.as_ref() {
            dict_keys.push(&k_timeout);
            dict_values.push(as_cf_type(v_timeout));
            dict_keys.push(&k_action);
            dict_values.push(as_cf_type(&v_action));
        }

        let props: CFRetained<CFDictionary<CFString, CFType>> =
            CFDictionary::from_slices(&dict_keys, &dict_values);

        let mut id: IOPMAssertionID = ffi::K_IOPM_NULL_ASSERTION_ID;
        // SAFETY: `props` is a live CFDictionary whose keys are CFStrings and
        // whose values are CF objects, exactly as IOPMAssertionCreateWithProperties
        // requires; `id` is a valid out-pointer for the duration of the call.
        let ret = unsafe { ffi::IOPMAssertionCreateWithProperties(&props, &raw mut id) };
        if ret != ffi::K_IORETURN_SUCCESS {
            return Err(Error::Create { kind, code: ret });
        }
        Ok(Self { id })
    }

    /// The raw IOKit id, for diagnostics.
    pub fn id(&self) -> IOPMAssertionID {
        self.id
    }

    /// Release explicitly and report the outcome.
    ///
    /// `Error::Release { code }` with `kIOReturnNotFound` means `powerd` had
    /// already released it (timeout fired); callers usually treat that as
    /// success.
    pub fn release(mut self) -> Result<()> {
        // Take the id and leave the null id behind so Drop does nothing. (Not
        // `mem::forget`: that would also skip dropping any owned fields.)
        let id = std::mem::replace(&mut self.id, ffi::K_IOPM_NULL_ASSERTION_ID);
        drop(self);
        release_id(id)
    }
}

impl Drop for Assertion {
    fn drop(&mut self) {
        if self.id != ffi::K_IOPM_NULL_ASSERTION_ID {
            // Errors cannot be reported from Drop; callers that care use `release()`.
            let _ = release_id(self.id);
        }
    }
}

fn release_id(id: IOPMAssertionID) -> Result<()> {
    // SAFETY: plain FFI call taking an integer handle; IOPMAssertionRelease
    // has no preconditions beyond being called from a process that may own
    // the id, and it reports unknown ids via its return code.
    let ret = unsafe { ffi::IOPMAssertionRelease(id) };
    if ret == ffi::K_IORETURN_SUCCESS {
        Ok(())
    } else {
        Err(Error::Release { code: ret })
    }
}

/// A "user is active" declaration, which turns the display on if it is off
/// and resets the system idle timer, like a key press would.
///
/// Unlike a regular assertion it expires on its own (after the display sleep
/// timer); call [`redeclare`](Self::redeclare) periodically to keep it fresh.
#[derive(Debug)]
pub struct UserActivity {
    id: IOPMAssertionID,
    name: String,
}

impl UserActivity {
    /// Declare user activity now.
    pub fn declare(name: &str) -> Result<Self> {
        let mut this = Self {
            id: ffi::K_IOPM_NULL_ASSERTION_ID,
            name: name.to_owned(),
        };
        this.redeclare()?;
        Ok(this)
    }

    /// Declare user activity again, extending the existing declaration.
    pub fn redeclare(&mut self) -> Result<()> {
        let cf_name = CFString::from_str(&self.name);
        // SAFETY: `cf_name` is a live CFString; `self.id` is either the null id
        // (first call) or the id returned by a previous call, both of which the
        // API documents as valid inputs; the pointer is valid for the call.
        let ret = unsafe {
            ffi::IOPMAssertionDeclareUserActivity(
                &cf_name,
                ffi::K_IOPM_USER_ACTIVE_LOCAL,
                &raw mut self.id,
            )
        };
        if ret != ffi::K_IORETURN_SUCCESS {
            return Err(Error::UserActivity { code: ret });
        }
        Ok(())
    }

    /// The raw IOKit id, for diagnostics.
    pub fn id(&self) -> IOPMAssertionID {
        self.id
    }

    /// Release explicitly and report the outcome (see [`Assertion::release`]).
    pub fn release(mut self) -> Result<()> {
        // As in `Assertion::release`: swap in the null id so Drop is a no-op
        // while `name` is still dropped normally.
        let id = std::mem::replace(&mut self.id, ffi::K_IOPM_NULL_ASSERTION_ID);
        drop(self);
        if id == ffi::K_IOPM_NULL_ASSERTION_ID {
            return Ok(());
        }
        release_id(id)
    }
}

impl Drop for UserActivity {
    fn drop(&mut self) {
        if self.id != ffi::K_IOPM_NULL_ASSERTION_ID {
            // See `Assertion::drop`: single release per id, errors unreportable here.
            let _ = release_id(self.id);
        }
    }
}
