//! Mach CPU/VM counters, APFS data-volume capacity and IOKit GPU statistics.
//! The GPU property is driver-supplied: absent or malformed is unavailable,
//! never an invented 0%. For multiple GPUs report the busiest accelerator.
use std::{
    ffi::{c_char, c_void},
    mem::MaybeUninit,
    ptr,
};

type CfRef = *const c_void;
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CfRef;
    fn IOServiceGetMatchingServices(port: u32, matching: CfRef, iterator: *mut u32) -> i32;
    fn IOIteratorNext(iterator: u32) -> u32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: CfRef,
        allocator: CfRef,
        options: u32,
    ) -> CfRef;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(allocator: CfRef, text: *const c_char, encoding: u32) -> CfRef;
    fn CFRelease(value: CfRef);
    fn CFGetTypeID(value: CfRef) -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFDictionaryGetValue(dictionary: CfRef, key: CfRef) -> CfRef;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: CfRef, kind: isize, value: *mut c_void) -> bool;
}
extern "C" {
    fn mach_port_deallocate(task: u32, name: u32) -> i32;
}
struct Cf(CfRef);
impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CFRelease(self.0);
            }
        }
    }
}
struct Io(u32);
impl Drop for Io {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                IOObjectRelease(self.0);
            }
        }
    }
}
struct Host(u32);
impl Drop for Host {
    #[allow(deprecated)]
    fn drop(&mut self) {
        unsafe {
            mach_port_deallocate(libc::mach_task_self_, self.0);
        }
    }
}

#[allow(deprecated)] // libc recommends mach2; these stable C calls need no extra dependency.
pub(super) fn cpu_ticks() -> Option<[u32; 4]> {
    unsafe {
        let host = Host(libc::mach_host_self());
        let mut data = MaybeUninit::<libc::host_cpu_load_info>::zeroed();
        let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
        let result = libc::host_statistics(
            host.0,
            libc::HOST_CPU_LOAD_INFO,
            data.as_mut_ptr().cast(),
            &mut count,
        );
        (result == 0 && count == libc::HOST_CPU_LOAD_INFO_COUNT)
            .then(|| data.assume_init().cpu_ticks)
    }
}
#[allow(deprecated)]
pub(super) fn memory() -> Option<(u64, u64)> {
    unsafe {
        let mut total = 0u64;
        let mut length = std::mem::size_of_val(&total);
        if libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut total as *mut u64).cast(),
            &mut length,
            ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        let page = libc::sysconf(libc::_SC_PAGESIZE);
        if page <= 0 || total == 0 {
            return None;
        }
        let host = Host(libc::mach_host_self());
        let mut stats = MaybeUninit::<libc::vm_statistics64>::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        if libc::host_statistics64(
            host.0,
            libc::HOST_VM_INFO64,
            stats.as_mut_ptr().cast(),
            &mut count,
        ) != 0
            || count != libc::HOST_VM_INFO64_COUNT
        {
            return None;
        }
        let v = stats.assume_init();
        // App-private pages excluding purgeable caches, wired pages and the
        // actual compressed-memory footprint (not its uncompressed size).
        let pages = u64::from(v.internal_page_count.saturating_sub(v.purgeable_count))
            + u64::from(v.wire_count)
            + u64::from(v.compressor_page_count);
        Some((pages.saturating_mul(page as u64).min(total), total))
    }
}
pub(super) fn disk() -> Option<(u64, u64)> {
    unsafe {
        let mut stats = MaybeUninit::<libc::statfs>::zeroed();
        // The writable startup volume, rather than the read-only system snapshot.
        if libc::statfs(c"/System/Volumes/Data".as_ptr(), stats.as_mut_ptr()) != 0 {
            return None;
        }
        let s = stats.assume_init();
        if s.f_bsize == 0 || s.f_blocks == 0 {
            return None;
        }
        Some((
            s.f_bavail.saturating_mul(s.f_bsize as u64),
            s.f_blocks.saturating_mul(s.f_bsize as u64),
        ))
    }
}
pub(super) fn gpu() -> Option<u8> {
    // IOKit consumes the matching dictionary; each returned object/property
    // is released exactly once. Nested dictionary values are borrowed.
    // https://developer.apple.com/documentation/iokit/1514494-ioservicegetmatchingservices
    unsafe {
        let matching = IOServiceMatching(c"IOAccelerator".as_ptr());
        if matching.is_null() {
            return None;
        }
        let mut iterator = 0;
        if IOServiceGetMatchingServices(0, matching, &mut iterator) != 0 {
            return None;
        }
        let iterator = Io(iterator);
        let key = Cf(CFStringCreateWithCString(
            ptr::null(),
            c"PerformanceStatistics".as_ptr(),
            0x0800_0100,
        ));
        let load_key = Cf(CFStringCreateWithCString(
            ptr::null(),
            c"Device Utilization %".as_ptr(),
            0x0800_0100,
        ));
        if key.0.is_null() || load_key.0.is_null() {
            return None;
        }
        let mut busiest = None;
        loop {
            let entry = Io(IOIteratorNext(iterator.0));
            if entry.0 == 0 {
                break;
            }
            let stats = Cf(IORegistryEntryCreateCFProperty(
                entry.0,
                key.0,
                ptr::null(),
                0,
            ));
            if stats.0.is_null() || CFGetTypeID(stats.0) != CFDictionaryGetTypeID() {
                continue;
            }
            let value = CFDictionaryGetValue(stats.0, load_key.0);
            if value.is_null() || CFGetTypeID(value) != CFNumberGetTypeID() {
                continue;
            }
            let mut load = 0f64;
            if CFNumberGetValue(value, 6, (&mut load as *mut f64).cast())
                && load.is_finite()
                && (0.0..=100.0).contains(&load)
            {
                busiest = Some(busiest.unwrap_or(0).max(load.round() as u8));
            }
        }
        busiest
    }
}
