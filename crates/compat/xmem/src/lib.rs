//! `xmem.dll` compatibility shim.
//!
//! The documented GDK `XMemAlloc`/`XMemFree`/`XMemVirtualAlloc` family is backed
//! here by the real Windows heap / `VirtualAlloc` — a genuine (if partial)
//! implementation, not a fake. The *physical page* APIs (GPU-visible unified
//! memory on console) have no direct desktop equivalent and are logged stubs for
//! now; they must be routed through the graphics backend's resource model later.
//!
//! x64 Windows uses one calling convention, and the callee does not pop args, so
//! declaring a shim with fewer parameters than the real export is ABI-safe.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, Ordering};
use compat_common::record;

const M: &str = "xmem";

extern "system" {
    fn GetProcessHeap() -> *mut c_void;
    fn HeapAlloc(heap: *mut c_void, flags: u32, bytes: usize) -> *mut c_void;
    fn HeapFree(heap: *mut c_void, flags: u32, mem: *mut c_void) -> i32;
    fn VirtualAlloc(addr: *mut c_void, size: usize, alloc_type: u32, protect: u32) -> *mut c_void;
    fn VirtualFree(addr: *mut c_void, size: usize, free_type: u32) -> i32;
    fn GetLastError() -> u32;
    fn SetLastError(error: u32);
    fn VirtualQueryEx(process: *mut c_void, addr: *const c_void, info: *mut u64, size: usize) -> usize;
}
const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_INVALID_PARAMETER: u32 = 87;
const MEM_DECOMMIT: u32 = 0x0000_4000;
/// Xbox title physical pages are 64 KiB.
const XMEM_PAGE: usize = 0x1_0000;

// Windows VirtualAlloc allocation-type bits. The Xbox `XMemVirtualAlloc`
// allocation-type parameter is NOT bit-compatible with Windows: it carries
// console-specific bits (e.g. 0x20000000) that Windows would misread as
// MEM_LARGE_PAGES (privileged) and reject. We pass through only the bits Windows
// actually defines and drop the rest.
const MEM_COMMIT: u32 = 0x0000_1000;
const MEM_RESERVE: u32 = 0x0000_2000;
const MEM_RESET: u32 = 0x0008_0000;
const MEM_TOP_DOWN: u32 = 0x0010_0000;
const MEM_WRITE_WATCH: u32 = 0x0020_0000;
const MEM_RESET_UNDO: u32 = 0x0100_0000;
const WIN_ALLOC_TYPE_MASK: u32 =
    MEM_COMMIT | MEM_RESERVE | MEM_RESET | MEM_TOP_DOWN | MEM_WRITE_WATCH | MEM_RESET_UNDO;
const PAGE_NOACCESS: u32 = 0x01;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const PAGE_EXECUTE_ANY: u32 = 0x10 | 0x20 | 0x40 | 0x80; // execute bits

// ---- Heap-backed (real) implementations ----

// XALLOC_ATTRIBUTES bit layout (public XDK xmem.h MAKE_XALLOC_ATTRIBUTES, recalled, and
// consistent with every value this title passes):
//   bits  0..13 object type, 14..15 page size, 16..23 allocator id, 24..28 alignment
//   (log2 bytes), 29..32 memory type.
// Observed: 0x0480009e -> alignment 2^4 (ordinary allocations) and 0x30804000 -> alignment
// 2^16, memory type 1, used for texture memory that is then handed to CreatePlacedResourceX.
// Ignoring the alignment gave the title unaligned texture addresses.
fn xalloc_alignment(attributes: u64) -> usize {
    1usize << ((attributes >> 24) & 0x1f)
}

/// Pointers handed out by the page-aligned path (freed with VirtualFree, not HeapFree).
static ALIGNED: std::sync::Mutex<Option<std::collections::HashSet<usize>>> = std::sync::Mutex::new(None);

fn alloc_aligned(size: usize, align: usize) -> *mut c_void {
    const MEM_RELEASE: u32 = 0x8000;
    let size = size.max(1);
    let p = if align <= 0x1_0000 {
        // VirtualAlloc returns addresses aligned to the 64 KiB allocation granularity.
        unsafe { VirtualAlloc(core::ptr::null_mut(), size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) }
    } else {
        // Larger alignments: find a hole with a throwaway reservation, then claim the
        // aligned address inside it (retry if another thread takes it first).
        let mut out = core::ptr::null_mut();
        for _ in 0..16 {
            let probe = unsafe { VirtualAlloc(core::ptr::null_mut(), size + align, MEM_RESERVE, PAGE_NOACCESS) };
            if probe.is_null() {
                break;
            }
            unsafe { VirtualFree(probe, 0, MEM_RELEASE) };
            let want = ((probe as usize + align - 1) & !(align - 1)) as *mut c_void;
            out = unsafe { VirtualAlloc(want, size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
            if !out.is_null() {
                break;
            }
        }
        out
    };
    if !p.is_null() {
        ALIGNED.lock().unwrap().get_or_insert_with(Default::default).insert(p as usize);
    }
    p
}

/// Returns true if `address` came from `alloc_aligned` (and has now been released).
fn free_aligned(address: *mut c_void) -> bool {
    const MEM_RELEASE: u32 = 0x8000;
    let was = ALIGNED.lock().unwrap().as_mut().map_or(false, |s| s.remove(&(address as usize)));
    if was {
        unsafe { VirtualFree(address, 0, MEM_RELEASE) };
    }
    was
}

#[no_mangle]
pub extern "system" fn XMemAlloc(size: usize, attributes: u64) -> *mut c_void {
    let align = xalloc_alignment(attributes);
    // HeapAlloc guarantees 16-byte alignment on x64.
    let p = if align > 16 { alloc_aligned(size, align) } else { unsafe { HeapAlloc(GetProcessHeap(), 0, size.max(1)) } };
    record(M, "XMemAlloc", &format!("size={size} attrs={attributes:#x} -> {p:p}"));
    p
}

#[no_mangle]
pub extern "system" fn XMemAllocDefault(size: usize, attributes: u64) -> *mut c_void {
    let p = unsafe { HeapAlloc(GetProcessHeap(), 0, size.max(1)) };
    record(M, "XMemAllocDefault", &format!("size={size} attrs={attributes:#x} -> {p:p}"));
    p
}

#[no_mangle]
pub extern "system" fn XMemFree(address: *mut c_void, attributes: u64) {
    record(M, "XMemFree", &format!("addr={address:p} attrs={attributes:#x}"));
    if !address.is_null() && !free_aligned(address) {
        unsafe { HeapFree(GetProcessHeap(), 0, address) };
    }
}

#[no_mangle]
pub extern "system" fn XMemFreeDefault(address: *mut c_void, attributes: u64) {
    record(M, "XMemFreeDefault", &format!("addr={address:p} attrs={attributes:#x}"));
    if !address.is_null() {
        unsafe { HeapFree(GetProcessHeap(), 0, address) };
    }
}

#[no_mangle]
pub extern "system" fn XMemVirtualAlloc(
    address: *mut c_void,
    size: usize,
    alloc_type: u32,
    protect: u32,
    attributes: u64,
) -> *mut c_void {
    // Keep only Windows-defined allocation-type bits; drop Xbox-only bits.
    let mut win_type = alloc_type & WIN_ALLOC_TYPE_MASK;
    if win_type == 0 {
        // No recognizable intent: default to a reserve+commit.
        win_type = MEM_RESERVE | MEM_COMMIT;
    }
    // Page protection: the Xbox `protect` value is NOT reliably Win32 PAGE_*.
    // Evidence (crash): the game commits a page with protect=0x1 and then WRITES
    // to it immediately, so on Xbox a committed page is accessible regardless of
    // that value. We therefore drive protection from behavior, not the raw value:
    //  * committing  -> at least READWRITE (EXECUTE_READWRITE if an execute bit
    //    was requested), because the game uses committed memory.
    //  * reserve-only -> NOACCESS (protection is irrelevant until commit).
    let committing = win_type & MEM_COMMIT != 0;
    let win_protect = if committing {
        if protect & PAGE_EXECUTE_ANY != 0 {
            PAGE_EXECUTE_READWRITE
        } else {
            PAGE_READWRITE
        }
    } else {
        PAGE_NOACCESS
    };

    let p = unsafe { VirtualAlloc(address, size, win_type, win_protect) };
    let err = if p.is_null() { unsafe { GetLastError() } } else { 0 };
    record(
        M,
        "XMemVirtualAlloc",
        &format!(
            "addr={address:p} size={size} xbox_type={alloc_type:#x}->win_type={win_type:#x} prot={protect:#x}->{win_protect:#x} attrs={attributes:#x} -> {p:p}{}",
            if p.is_null() { format!(" (GetLastError={err})") } else { String::new() }
        ),
    );
    p
}

/// GXDK 250405 BVH callers supply a process, an address and a 64-byte output.
/// The x64 desktop MEMORY_BASIC_INFORMATION prefix supplies State and Protect
/// at offsets 32 and 36. The builder separately checks console GPU access at
/// offset 48. A desktop process mapping alone establishes no GPU access.
///
/// # Safety
/// A non-null `information` must point to 64 writable bytes.
#[no_mangle]
pub unsafe extern "system" fn XMemVirtualQuery(
    process: *mut c_void, address: *const c_void, information: *mut [u64; 8],
) -> i32 {
    if information.is_null() {
        SetLastError(87); // ERROR_INVALID_PARAMETER
        return 0;
    }
    let mut info = [0u64; 8];
    const HOST_INFO_SIZE: usize = 48;
    if VirtualQueryEx(process, address, info.as_mut_ptr(), HOST_INFO_SIZE) != HOST_INFO_SIZE {
        return 0;
    }
    information.write_unaligned(info);
    1
}

// ---- Physical-page / unified-memory APIs ----
//
// Xbox titles manage physical 64 KiB pages explicitly: reserve a virtual range,
// allocate physical pages (PFN array), then map pages into the range. PC has no
// app-managed physical pages without AWE (which needs SeLockMemoryPrivilege), so
// we emulate: XMemAllocatePhysicalPages grants pages and hands back opaque PFN
// tokens; the real backing is committed when XMemMapPhysicalPages maps them into
// the (already reserved) virtual range. This preserves the observable contract —
// a mapped VA becomes real, writable memory — without real physical pages.
//
// Signature (reversed from FXboxCommonMultiPageAllocator call sites):
//   int XMemAllocatePhysicalPages(UINT64 flags, SIZE_T* pNumPages, UINT64* pPageArray)
//   -> nonzero on success, pPageArray filled with pNumPages PFN tokens.
static NEXT_PFN: AtomicU64 = AtomicU64::new(0x1000);

#[no_mangle]
pub extern "system" fn XMemAllocatePhysicalPages(
    flags: u64, p_num_pages: *mut usize, p_page_array: *mut u64,
) -> i32 {
    if p_num_pages.is_null() {
        record(M, "XMemAllocatePhysicalPages", "pNumPages=null -> fail");
        return 0;
    }
    let n = unsafe { *p_num_pages };
    if !p_page_array.is_null() {
        // Hand back unique, nonzero tokens; backing is deferred to map time.
        let base = NEXT_PFN.fetch_add(n as u64, Ordering::Relaxed);
        for i in 0..n {
            unsafe { *p_page_array.add(i) = base + i as u64 };
        }
    }
    record(M, "XMemAllocatePhysicalPages", &format!("flags={flags:#x} pages={n} -> OK"));
    1
}

#[no_mangle]
pub extern "system" fn XMemFreePhysicalPages(address: *mut c_void, _p: *mut u64) -> i32 {
    record(M, "XMemFreePhysicalPages", &format!("addr={address:p}"));
    0
}

/// void* XMemMapPhysicalPages(void* va, SIZE_T numPages, UINT64* pageArray)
/// Maps (commits) `numPages` 64 KiB pages at `va`. A null pageArray unmaps.
#[no_mangle]
pub extern "system" fn XMemMapPhysicalPages(
    va: *mut c_void, num_pages: usize, page_array: *mut u64,
) -> *mut c_void {
    const MEM_COMMIT: u32 = 0x0000_1000;
    let size = num_pages.max(1) * XMEM_PAGE;
    if page_array.is_null() {
        unsafe { VirtualFree(va, size, MEM_DECOMMIT) };
        record(M, "XMemMapPhysicalPages", &format!("UNMAP va={va:p} pages={num_pages}"));
        return va;
    }
    // Commit the (reserved) range so the mapped VA becomes real memory.
    let p = unsafe { VirtualAlloc(va, size, MEM_COMMIT, PAGE_READWRITE) };
    let err = if p.is_null() { unsafe { GetLastError() } } else { 0 };
    record(M, "XMemMapPhysicalPages",
        &format!("va={va:p} pages={num_pages} size={size:#x} -> {p:p}{}",
            if p.is_null() { format!(" (err={err})") } else { String::new() }));
    if p.is_null() { core::ptr::null_mut() } else { va }
}

/// `BOOL XMemTransferMemory(UINT32 kind, SIZE_T bytes, ...)`: Win32-style, FALSE plus
/// `SetLastError` on failure. It moves memory budget between the title and the console's
/// system partition. The system `xmem.dll` checks in this order: kinds 1 and 2 fail with
/// ERROR_NOT_SUPPORTED when an internal system query returns zero (not identified; taken
/// as zero here); a zero byte count succeeds without work; a kind above 2 is
/// ERROR_INVALID_PARAMETER. A PC process has no system partition, so every remaining
/// request fails with ERROR_NOT_SUPPORTED.
/// (Returning an HRESULT here would read as TRUE.)
#[no_mangle]
pub extern "system" fn XMemTransferMemory(kind: u32, bytes: usize) -> i32 {
    let error = if kind == 1 || kind == 2 {
        ERROR_NOT_SUPPORTED
    } else if bytes == 0 {
        record(M, "XMemTransferMemory", &format!("kind={kind} bytes=0: nothing to transfer -> TRUE"));
        return 1;
    } else if kind > 2 {
        ERROR_INVALID_PARAMETER
    } else {
        ERROR_NOT_SUPPORTED
    };
    record(M, "XMemTransferMemory", &format!(
        "kind={kind} bytes={bytes:#x}: no system memory partition on the host -> FALSE (error {error})"));
    unsafe { SetLastError(error) };
    0
}

/// Size of the statistics block for ids with class 0x1 in the top nibble; the system
/// `xmem.dll` zeroes exactly this much before filling it.
const WORKING_SET_STATS_CLASS1_BYTES: usize = 0x50;

/// `BOOL XMemGetWorkingSetStatistics(UINT32 id, STATS* out)`: Win32-style BOOL. The
/// console figures it reports (title working set budgets) do not exist on a PC, so this
/// fails with ERROR_NOT_SUPPORTED. Callers that read the block without checking the
/// result get zeros for class-1 ids, as from the system DLL's own pre-clear; other
/// classes are left untouched because their block size is unknown.
/// (Returning an HRESULT here would read as TRUE and expose uninitialised memory.)
#[no_mangle]
pub extern "system" fn XMemGetWorkingSetStatistics(id: u32, out: *mut u8) -> i32 {
    if !out.is_null() && id & 0xf000_0000 == 0x1000_0000 {
        unsafe { core::ptr::write_bytes(out, 0, WORKING_SET_STATS_CLASS1_BYTES) };
    }
    record(M, "XMemGetWorkingSetStatistics", &format!(
        "id={id:#x}: no console working-set statistics on the host -> FALSE (ERROR_NOT_SUPPORTED)"));
    unsafe { SetLastError(ERROR_NOT_SUPPORTED) };
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_set_statistics_fails_as_win32_bool() {
        let mut block = [0xccu8; 0x60];
        unsafe { SetLastError(0) };
        assert_eq!(XMemGetWorkingSetStatistics(0x1000_0001, block.as_mut_ptr()), 0, "FALSE, not an HRESULT");
        assert_eq!(unsafe { GetLastError() }, ERROR_NOT_SUPPORTED);
        assert!(block[..0x50].iter().all(|&b| b == 0), "class-1 block cleared");
        assert!(block[0x50..].iter().all(|&b| b == 0xcc), "nothing written past the block");

        let mut other = [0xccu8; 0x60];
        assert_eq!(XMemGetWorkingSetStatistics(0x2000_0001, other.as_mut_ptr()), 0);
        assert!(other.iter().all(|&b| b == 0xcc), "unknown-size classes untouched");
        assert_eq!(XMemGetWorkingSetStatistics(0x1000_0001, core::ptr::null_mut()), 0, "null output tolerated");
    }

    #[test]
    fn transfer_memory_fails_as_win32_bool() {
        // Kinds 1 and 2 fail before the byte count is looked at.
        for kind in [1, 2] {
            unsafe { SetLastError(0) };
            assert_eq!(XMemTransferMemory(kind, 0), 0);
            assert_eq!(unsafe { GetLastError() }, ERROR_NOT_SUPPORTED);
        }
        // An empty transfer of any other kind succeeds, even an unknown one.
        assert_eq!(XMemTransferMemory(0, 0), 1);
        assert_eq!(XMemTransferMemory(7, 0), 1);
        unsafe { SetLastError(0) };
        assert_eq!(XMemTransferMemory(7, 0x1000), 0);
        assert_eq!(unsafe { GetLastError() }, ERROR_INVALID_PARAMETER);
        unsafe { SetLastError(0) };
        assert_eq!(XMemTransferMemory(0, 0x800_0000), 0);
        assert_eq!(unsafe { GetLastError() }, ERROR_NOT_SUPPORTED);
    }
}
