//! `AcpHal.dll` compatibility — Xbox Audio Processor (ACP/APU / SHAPE) memory.
//!
//! `FXboxCommonGDKPlatformMemory::CreateApuHeap` fatally errors if `ApuCreateHeap`
//! fails, so these must succeed. We back the "APU heap" with a single host
//! `VirtualAlloc` region and bump-allocate from it (signatures recovered from the
//! call sites in the EXE):
//!   HRESULT ApuCreateHeap(uint32 sizeBytes, uint32 flags)
//!   HRESULT ApuAlloc(void** ppBlock, uint32* pApuAddr, uint32 size, uint32 align)
//!   HRESULT ApuHeapGetState(struct{u32 free; u32 used; ...}* out, uint32 idx)
//!   UINT32  ApuMapVirtualAddress(const void* va)
//!   bool    ApuIsVirtualAddressValid(const void* va, UINT32 apuAlignment)
//! (the last two: argument/return registers read from the system `AcpHal.dll` exports).
//! Titles that never call `ApuCreateHeap` (the audio middleware only imports `ApuAlloc`)
//! get an implicit default heap on first use.
//! Audio DSP (SHAPE) is not emulated; this only satisfies the memory contract so
//! init proceeds, and `IAcpHal` (hal.rs) acknowledges commands without executing them. The APU address of a block is its offset inside the heap.

use core::ffi::c_void;
use std::sync::Mutex;
use compat_common::{record, S_OK};

mod hal;

pub(crate) const M: &str = "acphal";

extern "system" {
    fn VirtualAlloc(addr: *mut c_void, size: usize, typ: u32, protect: u32) -> *mut c_void;
    fn VirtualFree(addr: *mut c_void, size: usize, typ: u32) -> i32;
}
const MEM_RELEASE: u32 = 0x8000;
/// Size of the heap created when a title allocates without calling `ApuCreateHeap` first.
/// The console's default size is unknown; blocks that do not fit fall back to standalone
/// host allocations (see `ApuAlloc`), so this only bounds the contiguous part.
const IMPLICIT_HEAP_SIZE: usize = 32 << 20;
const MEM_COMMIT_RESERVE: u32 = 0x1000 | 0x2000;
const PAGE_READWRITE: u32 = 0x04;

struct Heap {
    base: usize,
    size: usize,
    offset: usize,
}

static HEAP: Mutex<Option<Heap>> = Mutex::new(None);

/// Reserve+commit an APU heap of `size` bytes from host memory.
fn create_heap(g: &mut Option<Heap>, size: usize) -> Option<usize> {
    let size = size.max(0x1000);
    let base = unsafe { VirtualAlloc(core::ptr::null_mut(), size, MEM_COMMIT_RESERVE, PAGE_READWRITE) } as usize;
    if base == 0 {
        return None;
    }
    *g = Some(Heap { base, size, offset: 0 });
    Some(base)
}

#[no_mangle]
pub extern "system" fn ApuCreateHeap(size_bytes: u32, flags: u32) -> i32 {
    let mut g = HEAP.lock().unwrap();
    if g.is_some() {
        record(M, "ApuCreateHeap", "already created -> S_OK");
        return S_OK;
    }
    match create_heap(&mut g, size_bytes as usize) {
        Some(base) => {
            record(M, "ApuCreateHeap", &format!("size={size_bytes} flags={flags:#x} -> heap @ {base:#x} S_OK"));
            S_OK
        }
        None => {
            record(M, "ApuCreateHeap", &format!("VirtualAlloc({size_bytes}) FAILED -> E_FAIL"));
            compat_common::E_FAIL
        }
    }
}

#[no_mangle]
/// `ApuAlloc(void** ppBlock, APU_ADDRESS* pApuAddr, UINT32 size, UINT32 align)`.
/// `APU_ADDRESS` is a **32-bit** handle (title call sites at EXE+0x6def61/0x6defb5 pass a
/// u32 slot placed 4 bytes before the block-pointer slot), so only 4 bytes may be written.
pub extern "system" fn ApuAlloc(pp_block: *mut *mut c_void, p_phys: *mut u32, size: u32, align: u32) -> i32 {
    let mut g = HEAP.lock().unwrap();
    if g.is_none() {
        match create_heap(&mut g, IMPLICIT_HEAP_SIZE) {
            Some(base) => record(M, "ApuAlloc", &format!(
                "no ApuCreateHeap before first ApuAlloc -> implicit {IMPLICIT_HEAP_SIZE}-byte heap @ {base:#x}")),
            None => {
                record(M, "ApuAlloc", "no heap and implicit heap VirtualAlloc FAILED -> E_FAIL");
                return compat_common::E_FAIL;
            }
        }
    }
    let heap = g.as_mut().unwrap();
    let align = (align as usize).max(16);
    let start = (heap.base + heap.offset + (align - 1)) & !(align - 1);
    let end = start + size as usize;
    let ptr = if end <= heap.base + heap.size {
        heap.offset = end - heap.base;
        start
    } else {
        // Heap exhausted: fall back to a standalone committed region.
        let p = unsafe { VirtualAlloc(core::ptr::null_mut(), (size as usize).max(0x1000), MEM_COMMIT_RESERVE, PAGE_READWRITE) } as usize;
        record(M, "ApuAlloc", &format!("heap full; standalone alloc size={size} -> {p:#x}"));
        p
    };
    if !p_phys.is_null() {
        // APU address = offset of the block inside the APU heap (32-bit).
        unsafe { core::ptr::write_unaligned(p_phys, ptr.wrapping_sub(heap.base) as u32) };
    }
    if !pp_block.is_null() {
        unsafe { *pp_block = ptr as *mut c_void };
    }
    S_OK
}

#[no_mangle]
pub extern "system" fn ApuHeapGetState(out: *mut u32, idx: u32) -> i32 {
    let g = HEAP.lock().unwrap();
    let (free, used) = match g.as_ref() {
        Some(h) => ((h.size - h.offset) as u32, h.offset as u32),
        None => (0u32, 0u32),
    };
    // Game reads out[0] and out[1] (offsets 0 and 4). Write exactly those.
    if !out.is_null() {
        unsafe {
            *out = free;
            *out.add(1) = used;
        }
    }
    let _ = idx;
    S_OK
}

#[no_mangle]
pub extern "system" fn ApuFree(_block: *mut c_void) -> i32 {
    // Bump allocator: individual frees are no-ops (APU heap freed at teardown).
    S_OK
}

/// `UINT32 ApuMapVirtualAddress(const void* va)`: the APU address of a CPU pointer into the
/// heap, i.e. the same offset `ApuAlloc` reports. Pointers outside the heap (including the
/// standalone overflow blocks) have no APU address here: logged, returns 0.
#[no_mangle]
pub extern "system" fn ApuMapVirtualAddress(va: *const c_void) -> u32 {
    let g = HEAP.lock().unwrap();
    let v = va as usize;
    match g.as_ref() {
        Some(h) if v >= h.base && v < h.base + h.size => (v - h.base) as u32,
        _ => {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 8 {
                record(M, "ApuMapVirtualAddress", &format!("{va:p} is not inside the APU heap -> 0"));
            }
            0
        }
    }
}

/// `bool ApuIsVirtualAddressValid(const void* va, UINT32 apuAlignment)`: true when `va`
/// lies inside the heap and its APU address is a multiple of `apuAlignment` (0 = any).
#[no_mangle]
pub extern "system" fn ApuIsVirtualAddressValid(va: *const c_void, align: u32) -> bool {
    let g = HEAP.lock().unwrap();
    let v = va as usize;
    match g.as_ref() {
        Some(h) if v >= h.base && v < h.base + h.size => align <= 1 || (v - h.base) % align as usize == 0,
        _ => false,
    }
}

// ---- IAcpHal -----------------------------------------------------------------------------
//
// `AcpHalCreate(IAcpHal** ppAcpHal)` (single out-param: title call site EXE+0x6df048 loads
// only rcx). The object is a null device (see hal.rs): commands are validated and
// acknowledged in order, but there is no SHAPE/XMA DSP behind it, so no audio is produced.

#[no_mangle]
pub extern "system" fn AcpHalCreate(out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return compat_common::E_FAIL;
    }
    let obj = hal::object();
    unsafe { *out = obj };
    record(M, "AcpHalCreate", &format!("-> null-device IAcpHal @ {obj:p}"));
    S_OK
}

/// `AcpHalAllocateShapeContexts(SHAPE_CONTEXTS*)`.
///
/// Layout (GDK `SHAPE_CONTEXTS`):
///   +0x00 numSrc, +0x04 numEqComp, +0x08 numFiltVol, +0x0C numDma, +0x10 numXma, +0x14 numPcm
///   +0x18 srcArray, +0x20 eqCompArray, +0x28 filtVolArray, +0x30 dmaArray,
///   +0x38 xmaArray, +0x40 pcmArray
/// Element sizes, from callers that zero each array right after the call
/// (`count << shift` bytes at the returned pointer):
///   XMA 64 bytes (Fortnite's XMA init at EXE+0x6deee6 sets numXma=0x200, then zeroes
///   0x8000 bytes at +0x38),
///   SRC 32 bytes and DMA 16 bytes (FMOD's GameCore build: `<<5` / `<<4`).
/// There is no SHAPE DSP behind the contexts: they are plain memory and nothing decodes
/// or resamples (audio is tracked separately in docs/TECHNICAL_DEBT.md). EQ/compressor,
/// filter/volume and PCM contexts have no observed element size yet, so requesting them
/// fails explicitly.
#[no_mangle]
pub extern "system" fn AcpHalAllocateShapeContexts(ctx: *mut u8, _b: u32, _c: *mut c_void) -> i32 {
    if ctx.is_null() {
        return compat_common::E_FAIL;
    }
    let rd = |o: usize| unsafe { core::ptr::read_unaligned(ctx.add(o) as *const u32) };
    let (src, eq, fv, dma, xma, pcm) = (rd(0), rd(4), rd(8), rd(0xc), rd(0x10), rd(0x14));
    if eq != 0 || fv != 0 || pcm != 0 {
        record(M, "AcpHalAllocateShapeContexts",
            &format!("unsupported context request eq={eq} filtvol={fv} pcm={pcm} (element sizes unknown) -> E_FAIL"));
        return compat_common::E_FAIL;
    }
    let mut regions = CONTEXT_REGIONS.lock().unwrap();
    if !regions.is_empty() {
        record(M, "AcpHalAllocateShapeContexts", "contexts already allocated (no release in between) -> E_FAIL");
        return compat_common::E_FAIL;
    }
    // (count, element size, array-pointer offset)
    let kinds = [(src, 32usize, 0x18usize), (dma, 16, 0x30), (xma, 64, 0x38)];
    let mut arrays = [core::ptr::null_mut::<c_void>(); 3];
    for (i, &(n, elem, _)) in kinds.iter().enumerate() {
        if n == 0 {
            continue;
        }
        let bytes = (n as usize * elem).max(0x1000);
        let p = unsafe { VirtualAlloc(core::ptr::null_mut(), bytes, MEM_COMMIT_RESERVE, PAGE_READWRITE) };
        if p.is_null() {
            for q in arrays.iter().filter(|q| !q.is_null()) {
                unsafe { VirtualFree(*q, 0, MEM_RELEASE) };
            }
            record(M, "AcpHalAllocateShapeContexts", &format!("VirtualAlloc({bytes}) FAILED -> E_FAIL"));
            return compat_common::E_FAIL;
        }
        arrays[i] = p;
    }
    for (i, &(_, _, off)) in kinds.iter().enumerate() {
        unsafe { core::ptr::write_unaligned(ctx.add(off) as *mut *mut c_void, arrays[i]) };
        if !arrays[i].is_null() {
            regions.push(arrays[i] as usize);
        }
    }
    record(M, "AcpHalAllocateShapeContexts", &format!(
        "numSrc={src} numDma={dma} numXma={xma} -> src={:p} dma={:p} xma={:p} (no SHAPE DSP attached)",
        arrays[0], arrays[1], arrays[2]));
    S_OK
}

/// Context arrays handed out by `AcpHalAllocateShapeContexts`, freed on release.
static CONTEXT_REGIONS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// `HRESULT AcpHalReleaseShapeContexts(void)`: frees every context array. Callers clear
/// their own copies of the array pointers after a successful release.
#[no_mangle]
pub extern "system" fn AcpHalReleaseShapeContexts() -> i32 {
    let mut regions = CONTEXT_REGIONS.lock().unwrap();
    let n = regions.len();
    for p in regions.drain(..) {
        unsafe { VirtualFree(p as *mut c_void, 0, MEM_RELEASE) };
    }
    record(M, "AcpHalReleaseShapeContexts", &format!("released {n} context arrays -> S_OK"));
    S_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test owns the global heap so the checks run in a fixed order.
    #[test]
    fn implicit_heap_addresses_and_validity() {
        let mut a: *mut c_void = core::ptr::null_mut();
        let mut a_apu = 0xdead_beefu32;
        assert_eq!(ApuAlloc(&mut a, &mut a_apu, 0x5000, 4), S_OK, "first ApuAlloc creates a heap");
        let mut b: *mut c_void = core::ptr::null_mut();
        let mut b_apu = 0u32;
        assert_eq!(ApuAlloc(&mut b, &mut b_apu, 0x100, 0x800), S_OK);
        assert_eq!(b as usize % 0x800, 0, "alignment honoured");
        assert!(b as usize >= a as usize + 0x5000, "blocks do not overlap");
        // The memory is real and writable.
        unsafe { core::ptr::write_bytes(b as *mut u8, 0xab, 0x100) };

        assert_eq!(ApuMapVirtualAddress(a), a_apu, "map agrees with ApuAlloc's APU address");
        assert_eq!(ApuMapVirtualAddress(b), b_apu);
        assert_eq!(ApuMapVirtualAddress((b as usize + 0x10) as *const c_void), b_apu + 0x10);
        let outside = 0x10u8;
        assert_eq!(ApuMapVirtualAddress(&outside as *const u8 as *const c_void), 0);

        assert!(ApuIsVirtualAddressValid(b, 0x800));
        assert!(ApuIsVirtualAddressValid(b, 0), "alignment 0 = any");
        assert!(!ApuIsVirtualAddressValid((b as usize + 4) as *const c_void, 0x800), "misaligned");
        assert!(!ApuIsVirtualAddressValid(&outside as *const u8 as *const c_void, 1), "outside heap");
    }

    fn ctx(counts: [u32; 6]) -> [u8; 0x48] {
        let mut c = [0u8; 0x48];
        for (i, n) in counts.iter().enumerate() {
            c[i * 4..i * 4 + 4].copy_from_slice(&n.to_le_bytes());
        }
        c
    }
    fn ptr_at(c: &[u8; 0x48], off: usize) -> usize {
        usize::from_le_bytes(c[off..off + 8].try_into().unwrap())
    }

    /// One test owns the global context list (allocate, refuse a second allocation, release).
    #[test]
    fn shape_contexts_allocate_release() {
        // Refused kinds leave nothing allocated.
        let mut bad = ctx([0, 1, 0, 0, 0, 0]);
        assert_eq!(AcpHalAllocateShapeContexts(bad.as_mut_ptr(), 0, core::ptr::null_mut()), compat_common::E_FAIL);

        // FMOD's request: src=0x200, dma=0x400, xma=0x200.
        let mut c = ctx([0x200, 0, 0, 0x400, 0x200, 0]);
        assert_eq!(AcpHalAllocateShapeContexts(c.as_mut_ptr(), 0, core::ptr::null_mut()), S_OK);
        let (src, dma, xma) = (ptr_at(&c, 0x18), ptr_at(&c, 0x30), ptr_at(&c, 0x38));
        assert!(src != 0 && dma != 0 && xma != 0);
        assert_eq!((ptr_at(&c, 0x20), ptr_at(&c, 0x28), ptr_at(&c, 0x40)), (0, 0, 0), "unrequested kinds stay null");
        // Each array is as large as the caller zeroes it (count << shift).
        unsafe {
            core::ptr::write_bytes(src as *mut u8, 0, 0x200 << 5);
            core::ptr::write_bytes(dma as *mut u8, 0, 0x400 << 4);
            core::ptr::write_bytes(xma as *mut u8, 0, 0x200 << 6);
        }
        let mut again = ctx([0, 0, 0, 0, 1, 0]);
        assert_eq!(AcpHalAllocateShapeContexts(again.as_mut_ptr(), 0, core::ptr::null_mut()), compat_common::E_FAIL,
            "second allocation without release is refused");
        assert_eq!(AcpHalReleaseShapeContexts(), S_OK);
        assert!(CONTEXT_REGIONS.lock().unwrap().is_empty());
        // After release a new allocation succeeds (XMA only, the Fortnite request).
        let mut xonly = ctx([0, 0, 0, 0, 0x200, 0]);
        assert_eq!(AcpHalAllocateShapeContexts(xonly.as_mut_ptr(), 0, core::ptr::null_mut()), S_OK);
        assert!(ptr_at(&xonly, 0x38) != 0 && ptr_at(&xonly, 0x18) == 0);
        assert_eq!(AcpHalReleaseShapeContexts(), S_OK);
    }
}
