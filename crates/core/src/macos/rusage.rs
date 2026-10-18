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

/// Live heap accounting from the malloc zones, via `malloc_zone_statistics`.
///
/// Unlike the physical footprint this is byte-exact and independent of page
/// granularity: a leaked allocation shows up as growth in `size_in_use`
/// regardless of whether it happened to need a new page. Rust's global
/// allocator, CoreFoundation and IOKit's client code all allocate through
/// these zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MallocStats {
    /// Number of live allocations.
    pub blocks_in_use: u32,
    /// Bytes in live allocations.
    pub size_in_use: u64,
    /// Bytes the zones have obtained from the kernel.
    pub size_allocated: u64,
}

#[repr(C)]
struct MallocStatisticsT {
    blocks_in_use: libc::c_uint,
    size_in_use: libc::size_t,
    max_size_in_use: libc::size_t,
    size_allocated: libc::size_t,
}

unsafe extern "C" {
    // <mach/mach_init.h>: the current task's port, which `mach_task_self()` reads.
    static mach_task_self_: libc::mach_port_t;
    // From <malloc/malloc.h>. The process has several zones (the "nano" zone
    // for small blocks, a scalable zone for the rest), so statistics have to
    // be summed over all of them.
    fn malloc_get_all_zones(
        task: libc::mach_port_t,
        reader: *mut libc::c_void,
        addresses: *mut *mut *mut libc::c_void,
        count: *mut libc::c_uint,
    ) -> libc::kern_return_t;
    fn malloc_zone_statistics(zone: *mut libc::c_void, stats: *mut MallocStatisticsT);
}

impl MallocStats {
    /// Sample every malloc zone of the calling process and sum them.
    pub fn sample() -> Self {
        let mut total = Self::default();
        for zone in zone_table() {
            let mut stats = MallocStatisticsT {
                blocks_in_use: 0,
                size_in_use: 0,
                max_size_in_use: 0,
                size_allocated: 0,
            };
            // SAFETY: `zone` is a live malloc zone and `stats` a valid,
            // writable struct matching the C layout of malloc_statistics_t.
            unsafe { malloc_zone_statistics(zone, &raw mut stats) };
            total.blocks_in_use = total.blocks_in_use.saturating_add(stats.blocks_in_use);
            total.size_in_use += stats.size_in_use as u64;
            total.size_allocated += stats.size_allocated as u64;
        }
        total
    }
}

/// Live malloc blocks, found by walking every zone with its own introspection
/// enumerator: the same walk Apple's `heap` and `leaks` perform.
///
/// This is the exact measurement. [`MallocStats`] (`malloc_zone_statistics`)
/// is cheaper but over-counts at times: the nano zone can report one whole
/// 16 KiB block (512 × 32 bytes, 341 × 48 bytes, ...) as in use while every
/// slot in it is free, at moments that depend on timing and load. The
/// enumerator reads the per-slot free lists and is not fooled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LiveBlocks {
    /// Number of live allocations.
    pub count: u64,
    /// Bytes in live allocations.
    pub bytes: u64,
}

/// `vm_range_t` from <malloc/malloc.h>.
#[repr(C)]
struct VmRange {
    address: libc::vm_address_t,
    size: libc::vm_size_t,
}

type MemoryReader = unsafe extern "C" fn(
    libc::task_t,
    libc::vm_address_t,
    libc::vm_size_t,
    *mut *mut libc::c_void,
) -> libc::kern_return_t;
type VmRangeRecorder =
    unsafe extern "C" fn(libc::task_t, *mut libc::c_void, libc::c_uint, *mut VmRange, libc::c_uint);
type Enumerator = unsafe extern "C" fn(
    libc::task_t,
    *mut libc::c_void,
    libc::c_uint,
    libc::vm_address_t,
    MemoryReader,
    VmRangeRecorder,
) -> libc::kern_return_t;

/// The leading fields of `malloc_zone_t` (<malloc/malloc.h>), up to the
/// introspection table. Later fields vary by zone version and are not read.
#[repr(C)]
struct MallocZoneHead {
    reserved1: *mut libc::c_void,
    reserved2: *mut libc::c_void,
    size: *const libc::c_void,
    malloc: *const libc::c_void,
    calloc: *const libc::c_void,
    valloc: *const libc::c_void,
    free: *const libc::c_void,
    realloc: *const libc::c_void,
    destroy: *const libc::c_void,
    zone_name: *const libc::c_char,
    batch_malloc: *const libc::c_void,
    batch_free: *const libc::c_void,
    introspect: *const MallocIntrospectionHead,
}

/// The first field of `malloc_introspection_t`; the rest is not needed.
#[repr(C)]
struct MallocIntrospectionHead {
    enumerator: Option<Enumerator>,
}

/// `MALLOC_PTR_IN_USE_RANGE_TYPE`: report allocated blocks only.
const MALLOC_PTR_IN_USE_RANGE_TYPE: libc::c_uint = 1;

/// `memory_reader_t` for our own task: the memory is already mapped where it is.
unsafe extern "C" fn read_own_memory(
    _task: libc::task_t,
    address: libc::vm_address_t,
    _size: libc::vm_size_t,
    local: *mut *mut libc::c_void,
) -> libc::kern_return_t {
    // SAFETY: libmalloc passes a valid out-pointer for the local address.
    unsafe { *local = std::ptr::with_exposed_provenance_mut(address) };
    libc::KERN_SUCCESS
}

/// `vm_range_recorder_t`: tally the ranges into the `LiveBlocks` behind `context`.
unsafe extern "C" fn record_ranges(
    _task: libc::task_t,
    context: *mut libc::c_void,
    _kind: libc::c_uint,
    ranges: *mut VmRange,
    count: libc::c_uint,
) {
    if count == 0 || ranges.is_null() || context.is_null() {
        return;
    }
    // SAFETY: `context` is the `&mut LiveBlocks` that `LiveBlocks::sample`
    // passed to the enumerator and keeps alive for the whole call; libmalloc
    // hands over `count` initialised ranges.
    let (total, ranges) = unsafe {
        (
            &mut *context.cast::<LiveBlocks>(),
            std::slice::from_raw_parts(ranges, count as usize),
        )
    };
    total.count += u64::from(count);
    total.bytes += ranges.iter().map(|range| range.size as u64).sum::<u64>();
}

impl LiveBlocks {
    /// Walk every malloc zone of the calling process and count live blocks.
    ///
    /// Call it at a quiet moment: zones are read without their locks, as
    /// `heap` does on a suspended task, so a block another thread is
    /// allocating or freeing right now may be counted or missed.
    pub fn sample() -> io::Result<Self> {
        let mut total = Self::default();
        for zone in zone_table() {
            // SAFETY: `zone` is a live malloc zone owned by libmalloc, laid out
            // as `malloc_zone_t`; its introspection table pointer is valid for
            // the life of the process.
            let introspect = unsafe { (*zone.cast::<MallocZoneHead>()).introspect };
            if introspect.is_null() {
                continue;
            }
            // SAFETY: as above; `enumerator` is the table's first field.
            let Some(enumerator) = (unsafe { (*introspect).enumerator }) else {
                continue;
            };
            // SAFETY: an in-process enumeration of our own task. The two
            // callbacks match libmalloc's `memory_reader_t` and
            // `vm_range_recorder_t`, and `total` outlives the call, the only
            // time `context` is dereferenced.
            let kr = unsafe {
                enumerator(
                    mach_task_self_,
                    (&raw mut total).cast(),
                    MALLOC_PTR_IN_USE_RANGE_TYPE,
                    zone.addr(),
                    read_own_memory,
                    record_ranges,
                )
            };
            if kr != libc::KERN_SUCCESS {
                return Err(io::Error::other(format!(
                    "malloc zone enumerator failed: kern_return {kr}"
                )));
            }
        }
        Ok(total)
    }
}

/// The process's malloc zones, straight from libmalloc's table (no copy).
fn zone_table() -> impl Iterator<Item = *mut libc::c_void> {
    let mut zones: *mut *mut libc::c_void = std::ptr::null_mut();
    let mut count: libc::c_uint = 0;
    // SAFETY: `mach_task_self_` is a process-wide constant set up by the
    // kernel before main; querying our own task with a NULL reader (meaning
    // "read directly"); `zones` and `count` are valid out-pointers.
    let kr = unsafe {
        malloc_get_all_zones(
            mach_task_self_,
            std::ptr::null_mut(),
            &raw mut zones,
            &raw mut count,
        )
    };
    let count = if kr == libc::KERN_SUCCESS && !zones.is_null() {
        count as usize
    } else {
        0
    };
    (0..count).filter_map(move |i| {
        // SAFETY: `zones` points at `count` zone pointers owned by libmalloc,
        // valid for the life of the process.
        let zone = unsafe { *zones.add(i) };
        (!zone.is_null()).then_some(zone)
    })
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
    fn live_blocks_see_live_allocations() {
        // As below: other unit tests allocate concurrently, so use margins.
        const BLOCK: u64 = 8 << 20;
        const NOISE: u64 = 1 << 20;
        const SMALL: usize = 5000;
        let before = LiveBlocks::sample().unwrap();
        let block = vec![1u8; BLOCK as usize];
        let small: Vec<Box<[u8; 24]>> = (0..SMALL).map(|_| Box::new([1u8; 24])).collect();
        let during = LiveBlocks::sample().unwrap();
        assert!(
            during.bytes >= before.bytes + BLOCK - NOISE,
            "enumeration should see the {BLOCK}-byte block: {before:?} -> {during:?}"
        );
        assert!(
            during.count >= before.count + (SMALL as u64) * 9 / 10,
            "enumeration should see the {SMALL} nano blocks: {before:?} -> {during:?}"
        );
        drop((block, small));
        let after = LiveBlocks::sample().unwrap();
        assert!(
            after.bytes < before.bytes + NOISE,
            "the block should be gone again: {before:?} -> {after:?}"
        );
        assert!(
            after.count < before.count + (SMALL as u64) / 10,
            "the nano blocks should be gone again: {before:?} -> {after:?}"
        );
    }

    #[test]
    fn malloc_stats_see_live_allocations() {
        // Other unit tests run concurrently and allocate too, so use a block
        // far larger than their combined churn and compare with a margin.
        const BLOCK: u64 = 8 << 20;
        const NOISE: u64 = 1 << 20;
        let before = MallocStats::sample();
        let block = vec![1u8; BLOCK as usize];
        let during = MallocStats::sample();
        drop(block);
        let after = MallocStats::sample();
        assert!(
            during.size_in_use >= before.size_in_use + BLOCK - NOISE,
            "allocation not visible: before {} during {}",
            before.size_in_use,
            during.size_in_use
        );
        assert!(
            after.size_in_use + BLOCK <= during.size_in_use + NOISE,
            "free not visible: during {} after {}",
            during.size_in_use,
            after.size_in_use
        );
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
