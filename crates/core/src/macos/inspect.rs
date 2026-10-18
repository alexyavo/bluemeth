//! Read back the system-wide assertion table (what `pmset -g assertions`
//! prints), so tests and benchmarks can verify assertions really exist.

use std::ptr::{self, NonNull};

use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType};

use super::ffi::{self, keys};
use crate::{Error, Result};

/// One row of the assertion table.
#[derive(Debug, Clone, PartialEq)]
pub struct AssertionInfo {
    /// Owning process id.
    pub pid: i32,
    /// IOKit assertion type, e.g. `PreventUserIdleDisplaySleep`.
    pub kind: String,
    /// The `AssertName` the owner supplied.
    pub name: String,
    /// The `Details` string, if any.
    pub details: Option<String>,
    /// `TimeoutSeconds`, if the assertion has a timeout.
    pub timeout_secs: Option<f64>,
}

impl AssertionInfo {
    /// Whether this row is one of the kinds in [`crate::AssertionKind`].
    pub fn kind_enum(&self) -> Option<crate::AssertionKind> {
        crate::AssertionKind::ALL
            .into_iter()
            .find(|k| k.iokit_type() == self.kind)
    }

    /// Whether this row is a user-activity declaration.
    pub fn is_user_active(&self) -> bool {
        self.kind == keys::TYPE_USER_IS_ACTIVE
    }
}

/// A dictionary whose keys and values are arbitrary CF objects.
type CFObjectDictionary = CFDictionary<CFType, CFType>;

/// Every assertion currently held by any process.
pub fn all_assertions() -> Result<Vec<AssertionInfo>> {
    let mut raw: *const CFDictionary = ptr::null();
    // SAFETY: `raw` is a valid out-pointer. On success the function stores a
    // +1 retained dictionary (or NULL when the table is empty) into it.
    let ret = unsafe { ffi::IOPMCopyAssertionsByProcess(&raw mut raw) };
    if ret != ffi::K_IORETURN_SUCCESS {
        return Err(Error::Query { code: ret });
    }
    let Some(raw) = NonNull::new(raw.cast_mut()) else {
        return Ok(Vec::new());
    };
    // SAFETY: the "Copy" naming convention means we own one retain on this
    // dictionary; CFRetained::from_raw takes over that retain and releases it
    // on drop, so the object is released exactly once.
    let by_pid: CFRetained<CFDictionary> = unsafe { CFRetained::from_raw(raw) };
    // SAFETY: powerd builds this dictionary with the standard kCFType
    // callbacks, so every key and value is a CF object, which is all that
    // `CFType` asserts. Concrete types are checked below with `downcast`.
    let by_pid: &CFObjectDictionary = unsafe { by_pid.cast_unchecked() };

    let (pids, lists) = by_pid.to_vecs();
    let mut rows = Vec::new();
    for (pid, list) in pids.into_iter().zip(lists) {
        let Some(pid) = pid.downcast::<CFNumber>().ok().and_then(|n| n.as_i32()) else {
            continue;
        };
        let Ok(list) = list.downcast::<CFArray>() else {
            continue;
        };
        // SAFETY: as above, the array holds CF objects (one property
        // dictionary per assertion); each is downcast before use.
        let list: &CFArray<CFType> = unsafe { list.cast_unchecked() };
        for entry in list.iter() {
            let Ok(props) = entry.downcast::<CFDictionary>() else {
                continue;
            };
            // SAFETY: assertion property dictionaries are CFString-keyed with
            // CF object values (IOPMLib.h documents every key).
            let props: &CFObjectDictionary = unsafe { props.cast_unchecked() };
            if let Some(row) = row_from_props(pid, props) {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

/// Assertions held by one process.
pub fn assertions_for_pid(pid: i32) -> Result<Vec<AssertionInfo>> {
    let mut rows = all_assertions()?;
    rows.retain(|row| row.pid == pid);
    Ok(rows)
}

/// Assertions held by the calling process.
pub fn own_assertions() -> Result<Vec<AssertionInfo>> {
    assertions_for_pid(std::process::id() as i32)
}

fn row_from_props(pid: i32, props: &CFObjectDictionary) -> Option<AssertionInfo> {
    let lookup = |key: &'static str| -> Option<CFRetained<CFType>> {
        let key = CFString::from_static_str(key);
        let key: &CFType = key.as_ref();
        props.get(key)
    };
    let string = |key: &'static str| -> Option<String> {
        lookup(key)?
            .downcast::<CFString>()
            .ok()
            .map(|s| s.to_string())
    };
    let number = |key: &'static str| -> Option<f64> {
        lookup(key)?
            .downcast::<CFNumber>()
            .ok()
            .and_then(|n| n.as_f64())
    };
    Some(AssertionInfo {
        pid,
        kind: string(keys::ASSERTION_TYPE)?,
        name: string(keys::ASSERTION_NAME).unwrap_or_default(),
        details: string(keys::DETAILS),
        timeout_secs: number(keys::TIMEOUT_SECONDS).filter(|t| *t > 0.0),
    })
}
