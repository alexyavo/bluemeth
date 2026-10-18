//! Per-process resource accounting via `proc_pid_rusage`, the same counters
//! Activity Monitor shows (memory footprint, CPU time, idle wake-ups, energy).

use std::io;
use std::mem::MaybeUninit;

/// A snapshot of one process's cumulative resource usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProcessUsage {
    /// User CPU time, nanoseconds.
    pub user_time_ns: u64,
    /// System CPU time, nanoseconds.
    pub system_time_ns: u64,
    /// Package idle wake-ups (the "Idle Wake Ups" column in Activity Monitor).
    pub pkg_idle_wakeups: u64,
    /// Interrupt wake-ups.
    pub interrupt_wakeups: u64,
    /// Resident set size, bytes.
    pub resident_size: u64,
    /// Physical footprint, bytes (the "Memory" column in Activity Monitor).
    pub phys_footprint: u64,
    /// Lifetime maximum physical footprint, bytes.
    pub lifetime_max_phys_footprint: u64,
    /// Energy billed to the process, nanojoules (0 where unsupported).
    pub billed_energy_nj: u64,
    /// Instructions retired (0 where unsupported).
    pub instructions: u64,
}

impl ProcessUsage {
    /// Sample `pid`. Works for any process owned by the same user.
    pub fn sample(pid: i32) -> io::Result<Self> {
        let mut info = MaybeUninit::<libc::rusage_info_v4>::uninit();
        // SAFETY: RUSAGE_INFO_V4 tells the kernel to fill exactly a
        // `rusage_info_v4`, which is what `info` has room for; the pointer is
        // valid for the duration of the call.
        let rc = unsafe {
            libc::proc_pid_rusage(
                pid,
                libc::RUSAGE_INFO_V4,
                info.as_mut_ptr().cast::<libc::rusage_info_t>(),
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call succeeded, so the kernel initialised the struct.
        let info = unsafe { info.assume_init() };
        Ok(Self {
            user_time_ns: mach_to_ns(info.ri_user_time),
            system_time_ns: mach_to_ns(info.ri_system_time),
            pkg_idle_wakeups: info.ri_pkg_idle_wkups,
            interrupt_wakeups: info.ri_interrupt_wkups,
            resident_size: info.ri_resident_size,
            phys_footprint: info.ri_phys_footprint,
            lifetime_max_phys_footprint: info.ri_lifetime_max_phys_footprint,
            billed_energy_nj: info.ri_billed_energy,
            instructions: info.ri_instructions,
        })
    }

    /// Sample the calling process.
    pub fn sample_self() -> io::Result<Self> {
        Self::sample(std::process::id() as i32)
    }

    /// Total CPU time (user + system), nanoseconds.
    pub fn cpu_time_ns(&self) -> u64 {
        self.user_time_ns + self.system_time_ns
    }

    /// Counters that grew between `earlier` and `self` (saturating).
    #[must_use]
    pub fn delta_since(&self, earlier: &Self) -> Self {
        Self {
            user_time_ns: self.user_time_ns.saturating_sub(earlier.user_time_ns),
            system_time_ns: self.system_time_ns.saturating_sub(earlier.system_time_ns),
            pkg_idle_wakeups: self
                .pkg_idle_wakeups
                .saturating_sub(earlier.pkg_idle_wakeups),
            interrupt_wakeups: self
                .interrupt_wakeups
                .saturating_sub(earlier.interrupt_wakeups),
            resident_size: self.resident_size,
            phys_footprint: self.phys_footprint,
            lifetime_max_phys_footprint: self.lifetime_max_phys_footprint,
            billed_energy_nj: self
                .billed_energy_nj
                .saturating_sub(earlier.billed_energy_nj),
            instructions: self.instructions.saturating_sub(earlier.instructions),
        }
    }
}

/// `ri_user_time`/`ri_system_time` are in mach absolute time units.
fn mach_to_ns(ticks: u64) -> u64 {
    let (numer, denom) = timebase();
    // u128 avoids overflow for long-running processes.
    (u128::from(ticks) * u128::from(numer) / u128::from(denom)) as u64
}

#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

unsafe extern "C" {
    // From <mach/mach_time.h>; part of libSystem, always linked.
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> libc::c_int;
}

fn timebase() -> (u32, u32) {
    static TIMEBASE: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
    *TIMEBASE.get_or_init(|| {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: `info` is a valid, writable MachTimebaseInfo matching the C
        // struct layout (two u32s).
        let rc = unsafe { mach_timebase_info(&raw mut info) };
        if rc != 0 || info.denom == 0 {
            // On every shipping Darwin this call succeeds; fall back to 1:1
            // rather than divide by zero.
            return (1, 1);
        }
        (info.numer, info.denom)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_sample_self() {
        let usage = ProcessUsage::sample_self().expect("proc_pid_rusage on self");
        assert!(usage.phys_footprint > 0);
        assert!(usage.lifetime_max_phys_footprint >= usage.phys_footprint);
    }

    #[test]
    fn delta_saturates() {
        let a = ProcessUsage {
            user_time_ns: 10,
            ..Default::default()
        };
        let b = ProcessUsage {
            user_time_ns: 5,
            ..Default::default()
        };
        assert_eq!(b.delta_since(&a).user_time_ns, 0);
        assert_eq!(a.delta_since(&b).user_time_ns, 5);
    }

    #[test]
    fn nonexistent_pid_errors() {
        assert!(ProcessUsage::sample(i32::MAX).is_err());
    }
}
