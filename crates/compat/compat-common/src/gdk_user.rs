//! Local Xbox user (`XUser*`) for the title.
//!
//! Finding (2026-11-17): the title boots against Microsoft's *PC* `xgameruntime.dll` from
//! System32. For this unpackaged, unregistered title that runtime has no user, so the
//! title's login state machine aborts at its first step (`ConsolePrivileges`, "User -1 is
//! not valid"). A console always has a signed-in user; this module provides one.
//!
//! Model: exactly one local, offline user. Its identity is configuration, not an Xbox Live
//! account: `gamertag=`, `xuid=` and `xbl_token=` in `tuxbox.cfg` next to the title (defaults
//! below). Nothing here contacts Microsoft or Epic; the token is a placeholder string that
//! the title forwards to whatever backend the network policy routes it to (see
//! `net_guard`), i.e. the project owner's own server.
//!
//! Mechanism: the statically linked GDK wrappers `XUser*` in the title's EXE are redirected
//! to the implementations below. The wrappers are located by the interface they query and
//! the vtable slot they call (`gdk_wrappers`), not by per-build addresses, so this works
//! for any title built against the same GDK interfaces; the names of the keys come from
//! Fortnite 2.0.1461.7919's symbol table. Signatures are the public GDK ones (XUser.h /
//! XAsync.h). Asynchronous calls are completed through the title's own
//! `XAsyncBegin/Schedule/Complete/GetResult` wrappers (located the same way), which are
//! left untouched. Without all six XAsync wrappers, or without XUserAddAsync/AddResult,
//! nothing is redirected.
//!
//! Not modelled: multiple users, guests, sign-out, change-event delivery (registrations are
//! accepted but no event is ever fired), privilege denial (every privilege is granted),
//! real token signing (signature is empty).

use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

extern "system" {
    fn GetModuleHandleW(n: *const u16) -> *mut u8;
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
}

const S_OK: i32 = 0;
pub(crate) const E_PENDING: i32 = 0x8000_000Au32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const E_NOT_SUFFICIENT_BUFFER: i32 = 0x8007_007Au32 as i32;
const E_NOT_FOUND: i32 = 0x8007_0490u32 as i32; // HRESULT_FROM_WIN32(ERROR_NOT_FOUND); the exact GDK "user not found" code is not verified

/// PE TimeDateStamp of FortniteClient-XSX-Shipping.exe 2.0.1461.7919 (gates the XStore layer,
/// whose addresses are still build-specific).
const BUILD_TIMESTAMP: u32 = 0x5f9b_76c4;

use crate::gdk_wrappers::{Iface, XASYNC, XUSER, XUSER_DEVICE, XUSER_DEVICE_V2, XUSER_V2};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Interface versions that share a table entry: same object, same slots and arguments.
fn versions(iface: Iface) -> &'static [Iface] {
    if iface == XUSER_DEVICE {
        &[XUSER_DEVICE, XUSER_DEVICE_V2]
    } else if iface == XUSER {
        &[XUSER]
    } else if iface == XUSER_V2 {
        &[XUSER_V2]
    } else {
        &[XASYNC]
    }
}

// The title's XAsync wrappers (absolute addresses), found at install.
static XASYNC_BEGIN: AtomicUsize = AtomicUsize::new(0);
static XASYNC_COMPLETE: AtomicUsize = AtomicUsize::new(0);
static XASYNC_GET_RESULT: AtomicUsize = AtomicUsize::new(0);
static XASYNC_GET_RESULT_SIZE: AtomicUsize = AtomicUsize::new(0);
static XASYNC_GET_STATUS: AtomicUsize = AtomicUsize::new(0);
static XASYNC_SCHEDULE: AtomicUsize = AtomicUsize::new(0);

struct User {
    gamertag: String,
    xuid: u64,
    token: Vec<u8>,     // NUL-terminated UTF-8
    token16: Vec<u16>,  // NUL-terminated UTF-16
}
static USER: OnceLock<User> = OnceLock::new();
fn user() -> &'static User {
    USER.get().expect("gdk_user installed")
}

/// The one user handle handed to the title (an address inside this module; never freed).
static HANDLE_ANCHOR: u64 = 0x5842_4F58_5553_4552; // "XBOXUSER"
fn handle() -> usize {
    &HANDLE_ANCHOR as *const u64 as usize
}
const LOCAL_ID: u64 = 1;

pub(crate) fn log(func: &str, detail: &str) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    if n < 200 || n % 1000 == 0 {
        crate::record_raw("gdk_user", func, detail);
    }
}

// ---- XAsync plumbing (title's own wrappers) -------------------------------------------------------

#[repr(C)]
pub(crate) struct ProviderData {
    pub(crate) async_block: *mut c_void,
    buffer_size: usize,
    buffer: *mut u8,
    context: *mut c_void,
}
type Provider = unsafe extern "system" fn(op: u32, data: *const ProviderData) -> i32;

unsafe fn xasync_begin(block: *mut c_void, context: *mut c_void, identity: usize, name: &'static [u8], provider: Provider) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, usize, *const u8, Provider) -> i32 =
        core::mem::transmute(XASYNC_BEGIN.load(Ordering::Relaxed));
    f(block, context, identity, name.as_ptr(), provider)
}
unsafe fn xasync_schedule(block: *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, u32) -> i32 = core::mem::transmute(XASYNC_SCHEDULE.load(Ordering::Relaxed));
    f(block, 0)
}
pub(crate) unsafe fn xasync_complete(block: *mut c_void, hr: i32, size: usize) {
    let f: unsafe extern "system" fn(*mut c_void, i32, usize) = core::mem::transmute(XASYNC_COMPLETE.load(Ordering::Relaxed));
    f(block, hr, size)
}
unsafe fn xasync_get_result(block: *mut c_void, identity: usize, size: usize, buf: *mut u8, used: *mut usize) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, usize, usize, *mut u8, *mut usize) -> i32 =
        core::mem::transmute(XASYNC_GET_RESULT.load(Ordering::Relaxed));
    f(block, identity, size, buf, used)
}
unsafe fn xasync_get_result_size(block: *mut c_void, size: *mut usize) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, *mut usize) -> i32 = core::mem::transmute(XASYNC_GET_RESULT_SIZE.load(Ordering::Relaxed));
    f(block, size)
}
pub(crate) unsafe fn xasync_get_status(block: *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, bool) -> i32 = core::mem::transmute(XASYNC_GET_STATUS.load(Ordering::Relaxed));
    f(block, false)
}

/// Start an async operation whose result is produced by `provider` on the block's queue.
pub(crate) unsafe fn start_async(block: *mut c_void, identity: usize, name: &'static [u8], provider: Provider) -> i32 {
    if block.is_null() {
        return E_INVALIDARG;
    }
    let hr = xasync_begin(block, core::ptr::null_mut(), identity, name, provider);
    if hr < 0 {
        return hr;
    }
    xasync_schedule(block)
}

// XAsyncOp: 0 Begin, 1 DoWork, 2 GetResult, 3 Cancel, 4 Cleanup.

unsafe extern "system" fn add_provider(op: u32, data: *const ProviderData) -> i32 {
    match op {
        1 => {
            xasync_complete((*data).async_block, S_OK, core::mem::size_of::<usize>());
            E_PENDING
        }
        2 => {
            if (*data).buffer_size < core::mem::size_of::<usize>() {
                return E_NOT_SUFFICIENT_BUFFER;
            }
            core::ptr::write_unaligned((*data).buffer as *mut usize, handle());
            S_OK
        }
        _ => S_OK,
    }
}

unsafe extern "system" fn no_result_provider(op: u32, data: *const ProviderData) -> i32 {
    if op == 1 {
        xasync_complete((*data).async_block, S_OK, 0);
        return E_PENDING;
    }
    S_OK
}

/// Result layout: { size_t tokenSize; size_t signatureSize; char* token; char* signature; }
/// followed by the token and an empty signature (both NUL-terminated).
fn token_blob_size(wide: bool) -> usize {
    let u = user();
    32 + if wide { (u.token16.len() + 1) * 2 } else { u.token.len() + 1 }
}

unsafe fn write_token_blob(buf: *mut u8, wide: bool) {
    let u = user();
    let tok = buf.add(32);
    let (tok_bytes, count) = if wide {
        (u.token16.len() * 2, u.token16.len())
    } else {
        (u.token.len(), u.token.len())
    };
    if wide {
        core::ptr::copy_nonoverlapping(u.token16.as_ptr() as *const u8, tok, tok_bytes);
    } else {
        core::ptr::copy_nonoverlapping(u.token.as_ptr(), tok, tok_bytes);
    }
    let sig = tok.add(tok_bytes);
    core::ptr::write_bytes(sig, 0, if wide { 2 } else { 1 });
    core::ptr::write_unaligned(buf as *mut usize, count); // includes the terminator
    core::ptr::write_unaligned(buf.add(8) as *mut usize, 1);
    core::ptr::write_unaligned(buf.add(16) as *mut usize, tok as usize);
    core::ptr::write_unaligned(buf.add(24) as *mut usize, sig as usize);
}

unsafe extern "system" fn token_provider(op: u32, data: *const ProviderData) -> i32 {
    token_provider_impl(op, data, false)
}
unsafe extern "system" fn token16_provider(op: u32, data: *const ProviderData) -> i32 {
    token_provider_impl(op, data, true)
}
unsafe fn token_provider_impl(op: u32, data: *const ProviderData, wide: bool) -> i32 {
    match op {
        1 => {
            xasync_complete((*data).async_block, S_OK, token_blob_size(wide));
            E_PENDING
        }
        2 => {
            if (*data).buffer_size < token_blob_size(wide) {
                return E_NOT_SUFFICIENT_BUFFER;
            }
            write_token_blob((*data).buffer, wide);
            S_OK
        }
        _ => S_OK,
    }
}

// ---- XUser implementations (public GDK signatures) ------------------------------------------------

unsafe extern "system" fn x_add_async(options: u32, block: *mut c_void) -> i32 {
    let hr = start_async(block, x_add_async as usize, b"XUserAddAsync\0", add_provider);
    log("XUserAddAsync", &format!("options={options:#x} -> hr={hr:#010x} (local user '{}')", user().gamertag));
    hr
}

unsafe extern "system" fn x_add_result(block: *mut c_void, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let hr = xasync_get_result(block, x_add_async as usize, core::mem::size_of::<usize>(), out as *mut u8, core::ptr::null_mut());
    log("XUserAddResult", &format!("-> hr={hr:#010x} user={:#x}", if hr >= 0 { *out } else { 0 }));
    hr
}

unsafe extern "system" fn x_check_privilege(u: usize, options: u32, privilege: u32, has: *mut bool, reason: *mut u32) -> i32 {
    if u == 0 || has.is_null() {
        return E_INVALIDARG;
    }
    *has = true;
    if !reason.is_null() {
        *reason = 0; // XUserPrivilegeDenyReason::None
    }
    log("XUserCheckPrivilege", &format!("privilege={privilege} options={options:#x} -> granted (local user policy)"));
    S_OK
}

unsafe extern "system" fn x_close_handle(_u: usize) {}

unsafe extern "system" fn x_compare(a: usize, b: usize) -> i32 {
    if a == b { 0 } else if a < b { -1 } else { 1 }
}

unsafe extern "system" fn x_duplicate_handle(u: usize, out: *mut usize) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = u;
    S_OK
}

unsafe extern "system" fn x_find_for_device(_device: *const u8, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = handle(); // every input device belongs to the one local user
    static N: AtomicUsize = AtomicUsize::new(0);
    if N.fetch_add(1, Ordering::Relaxed) < 4 {
        log("XUserFindForDevice", "-> local user (every device belongs to it)");
    }
    S_OK
}

unsafe extern "system" fn x_find_by_id(xuid: u64, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    if xuid != user().xuid {
        log("XUserFindUserById", &format!("xuid={xuid} -> not found"));
        return E_NOT_FOUND;
    }
    *out = handle();
    S_OK
}

unsafe extern "system" fn x_find_by_local_id(local: u64, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    if local != LOCAL_ID {
        log("XUserFindUserByLocalId", &format!("localId={local} -> not found"));
        return E_NOT_FOUND;
    }
    *out = handle();
    S_OK
}

unsafe extern "system" fn x_get_gamertag(u: usize, component: u32, size: usize, buf: *mut u8, used: *mut usize) -> i32 {
    if u == 0 {
        return E_INVALIDARG;
    }
    // XUserGamertagComponent: 0 Classic, 1 Modern, 2 ModernSuffix, 3 UniqueModern.
    let s: &str = if component == 2 { "" } else { &user().gamertag };
    let need = s.len() + 1;
    if !used.is_null() {
        *used = need;
    }
    if buf.is_null() || size < need {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    core::ptr::copy_nonoverlapping(s.as_ptr(), buf, s.len());
    *buf.add(s.len()) = 0;
    S_OK
}

unsafe extern "system" fn x_get_id(u: usize, out: *mut u64) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = user().xuid;
    S_OK
}

unsafe extern "system" fn x_get_local_id(u: usize, out: *mut u64) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = LOCAL_ID;
    S_OK
}

unsafe extern "system" fn x_get_max_users(out: *mut u32) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = 1;
    S_OK
}

unsafe extern "system" fn x_get_state(u: usize, out: *mut u32) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = 0; // XUserState::SignedIn
    S_OK
}

unsafe fn describe_request(method: *const u8, url: *const u8, wide: bool) -> String {
    let rd = |p: *const u8| -> String {
        if p.is_null() {
            return "null".into();
        }
        if wide {
            let p = p as *const u16;
            let mut n = 0;
            while n < 512 && *p.add(n) != 0 { n += 1; }
            String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
        } else {
            let mut n = 0;
            while n < 512 && *p.add(n) != 0 { n += 1; }
            String::from_utf8_lossy(core::slice::from_raw_parts(p, n)).into_owned()
        }
    };
    format!("{} {}", rd(method), rd(url))
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn x_token_async(
    u: usize, options: u32, method: *const u8, url: *const u8, _hc: usize, _headers: *const c_void, _body_size: usize, _body: *const c_void, block: *mut c_void,
) -> i32 {
    if u == 0 {
        return E_INVALIDARG;
    }
    let hr = start_async(block, x_token_async as usize, b"XUserGetTokenAndSignatureAsync\0", token_provider);
    log("XUserGetTokenAndSignatureAsync", &format!("options={options:#x} for '{}' -> hr={hr:#010x} (placeholder token from config)", describe_request(method, url, false)));
    hr
}
unsafe extern "system" fn x_token_result(block: *mut c_void, size: usize, buf: *mut u8, ptr: *mut *mut u8, used: *mut usize) -> i32 {
    let hr = xasync_get_result(block, x_token_async as usize, size, buf, used);
    if hr >= 0 && !ptr.is_null() {
        *ptr = buf;
    }
    hr
}
#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn x_token16_async(
    u: usize, options: u32, method: *const u8, url: *const u8, _hc: usize, _headers: *const c_void, _body_size: usize, _body: *const c_void, block: *mut c_void,
) -> i32 {
    if u == 0 {
        return E_INVALIDARG;
    }
    let hr = start_async(block, x_token16_async as usize, b"XUserGetTokenAndSignatureUtf16Async\0", token16_provider);
    log("XUserGetTokenAndSignatureUtf16Async", &format!("options={options:#x} for '{}' -> hr={hr:#010x} (placeholder token from config)", describe_request(method, url, true)));
    hr
}
unsafe extern "system" fn x_token16_result(block: *mut c_void, size: usize, buf: *mut u8, ptr: *mut *mut u8, used: *mut usize) -> i32 {
    let hr = xasync_get_result(block, x_token16_async as usize, size, buf, used);
    if hr >= 0 && !ptr.is_null() {
        *ptr = buf;
    }
    hr
}
unsafe extern "system" fn x_token_result_size(block: *mut c_void, size: *mut usize) -> i32 {
    xasync_get_result_size(block, size)
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
unsafe extern "system" fn x_register(_queue: usize, _ctx: *mut c_void, _cb: *const c_void, token: *mut u64) -> i32 {
    if !token.is_null() {
        *token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    }
    log("XUserRegisterFor*", "registration accepted (no events are ever delivered)");
    S_OK
}
unsafe extern "system" fn x_unregister(_token: u64, _wait: bool) -> bool {
    true
}

unsafe extern "system" fn x_resolve_issue_async(_u: usize, _url: *const u8, block: *mut c_void) -> i32 {
    log("XUserResolveIssueWithUiAsync", "completed without UI");
    start_async(block, x_resolve_issue_async as usize, b"XUserResolveIssueWithUiAsync\0", no_result_provider)
}
unsafe extern "system" fn x_resolve_privilege_async(_u: usize, _options: u32, privilege: u32, block: *mut c_void) -> i32 {
    log("XUserResolvePrivilegeWithUiAsync", &format!("privilege={privilege} completed without UI"));
    start_async(block, x_resolve_privilege_async as usize, b"XUserResolvePrivilegeWithUiAsync\0", no_result_provider)
}
unsafe extern "system" fn x_resolve_result(block: *mut c_void) -> i32 {
    xasync_get_status(block)
}

// ---- installation -----------------------------------------------------------------------------------

pub(crate) unsafe fn write_jump(p: *mut u8, target: usize) -> bool {
    let mut code = [0u8; 12]; // mov rax, imm64 ; jmp rax
    code[0] = 0x48;
    code[1] = 0xB8;
    code[2..10].copy_from_slice(&(target as u64).to_le_bytes());
    code[10] = 0xFF;
    code[11] = 0xE0;
    let mut old = 0u32;
    if VirtualProtect(p as *mut c_void, 12, 0x40, &mut old) == 0 {
        return false;
    }
    core::ptr::copy_nonoverlapping(code.as_ptr(), p, 12);
    let mut tmp = 0u32;
    VirtualProtect(p as *mut c_void, 12, old, &mut tmp);
    FlushInstructionCache(GetCurrentProcess(), p as *const c_void, 12);
    true
}

/// Wrappers redirected to this module: (name, interface, vtable slot, implementation).
/// Keys from Fortnite 2.0.1461.7919, where the symbol table names each wrapper.
fn wrappers() -> [(&'static str, Iface, u32, usize); 28] {
    [
        ("XUserAddAsync", XUSER, 0x38, x_add_async as usize),
        ("XUserAddResult", XUSER, 0x40, x_add_result as usize),
        ("XUserCheckPrivilege", XUSER, 0xa0, x_check_privilege as usize),
        ("XUserCloseHandle", XUSER, 0x20, x_close_handle as usize),
        ("XUserCompare", XUSER, 0x28, x_compare as usize),
        ("XUserDuplicateHandle", XUSER, 0x18, x_duplicate_handle as usize),
        ("XUserFindForDevice", XUSER_DEVICE, 0x18, x_find_for_device as usize),
        ("XUserFindUserById", XUSER, 0x60, x_find_by_id as usize),
        ("XUserFindUserByLocalId", XUSER, 0x50, x_find_by_local_id as usize),
        ("XUserGetGamertag", XUSER_V2, 0x18, x_get_gamertag as usize),
        ("XUserGetId", XUSER, 0x58, x_get_id as usize),
        ("XUserGetLocalId", XUSER, 0x48, x_get_local_id as usize),
        ("XUserGetMaxUsers", XUSER, 0x30, x_get_max_users as usize),
        ("XUserGetState", XUSER, 0x70, x_get_state as usize),
        ("XUserGetTokenAndSignatureAsync", XUSER, 0xb8, x_token_async as usize),
        ("XUserGetTokenAndSignatureResult", XUSER, 0xc8, x_token_result as usize),
        ("XUserGetTokenAndSignatureResultSize", XUSER, 0xc0, x_token_result_size as usize),
        ("XUserGetTokenAndSignatureUtf16Async", XUSER, 0xd0, x_token16_async as usize),
        ("XUserGetTokenAndSignatureUtf16Result", XUSER, 0xe0, x_token16_result as usize),
        ("XUserGetTokenAndSignatureUtf16ResultSize", XUSER, 0xd8, x_token_result_size as usize),
        ("XUserRegisterForChangeEvent", XUSER, 0x108, x_register as usize),
        ("XUserRegisterForDeviceAssociationChanged", XUSER_DEVICE, 0x20, x_register as usize),
        ("XUserResolveIssueWithUiAsync", XUSER, 0xe8, x_resolve_issue_async as usize),
        ("XUserResolveIssueWithUiResult", XUSER, 0xf0, x_resolve_result as usize),
        ("XUserResolvePrivilegeWithUiAsync", XUSER, 0xa8, x_resolve_privilege_async as usize),
        ("XUserResolvePrivilegeWithUiResult", XUSER, 0xb0, x_resolve_result as usize),
        ("XUserUnregisterForChangeEvent", XUSER, 0x110, x_unregister as usize),
        ("XUserUnregisterForDeviceAssociationChanged", XUSER_DEVICE, 0x28, x_unregister as usize),
    ]
}

/// The title's XAsync wrappers this module calls: (name, slot, where the address goes).
fn xasync_slots() -> [(&'static str, u32, &'static AtomicUsize); 6] {
    [
        ("XAsyncBegin", 0x38, &XASYNC_BEGIN),
        ("XAsyncComplete", 0x50, &XASYNC_COMPLETE),
        ("XAsyncGetResult", 0x58, &XASYNC_GET_RESULT),
        ("XAsyncGetResultSize", 0x20, &XASYNC_GET_RESULT_SIZE),
        ("XAsyncGetStatus", 0x18, &XASYNC_GET_STATUS),
        ("XAsyncSchedule", 0x48, &XASYNC_SCHEDULE),
    ]
}

/// Interface key of a redirected or used wrapper, by name.
#[cfg(test)]
pub(crate) fn key_of(name: &str) -> Option<(Iface, u32)> {
    wrappers().iter().find(|w| w.0 == name).map(|w| (w.1, w.2))
        .or_else(|| xasync_slots().iter().find(|x| x.0 == name).map(|x| (XASYNC, x.1)))
}

/// Fortnite 2.0.1461.7919 symbol-table addresses of the same wrappers (test reference).
#[cfg(test)]
pub(crate) fn fortnite_symbol_rvas() -> [(&'static str, usize); 34] {
    [
        ("XUserAddAsync", 0x1dcba30), ("XUserAddResult", 0x1dcbaa0), ("XUserCheckPrivilege", 0x1dcbb14),
        ("XUserCloseHandle", 0x1dcbbb8), ("XUserCompare", 0x1dcbc10), ("XUserDuplicateHandle", 0x1dcbc84),
        ("XUserFindForDevice", 0x1dcbcf8), ("XUserFindUserById", 0x1dcbd6c), ("XUserFindUserByLocalId", 0x1dcbde4),
        ("XUserGetGamertag", 0x1dcbf00), ("XUserGetId", 0x1dcbfa0), ("XUserGetLocalId", 0x1dcc014),
        ("XUserGetMaxUsers", 0x1dcc088), ("XUserGetState", 0x1dcc0ec), ("XUserGetTokenAndSignatureAsync", 0x1dcc160),
        ("XUserGetTokenAndSignatureResult", 0x1dcc238), ("XUserGetTokenAndSignatureResultSize", 0x1dcc2dc),
        ("XUserGetTokenAndSignatureUtf16Async", 0x1dcc350), ("XUserGetTokenAndSignatureUtf16Result", 0x1dcc428),
        ("XUserGetTokenAndSignatureUtf16ResultSize", 0x1dcc4cc), ("XUserRegisterForChangeEvent", 0x1dcc540),
        ("XUserRegisterForDeviceAssociationChanged", 0x1dcc5dc), ("XUserResolveIssueWithUiAsync", 0x1dcc674),
        ("XUserResolveIssueWithUiResult", 0x1dcc6f8), ("XUserResolvePrivilegeWithUiAsync", 0x1dcc75c),
        ("XUserResolvePrivilegeWithUiResult", 0x1dcc7f4), ("XUserUnregisterForChangeEvent", 0x1dcc858),
        ("XUserUnregisterForDeviceAssociationChanged", 0x1dcc8d4),
        ("XAsyncBegin", 0x1dc87c8), ("XAsyncComplete", 0x1dc88b4), ("XAsyncGetResult", 0x1dc8928),
        ("XAsyncGetResultSize", 0x1dc89c4), ("XAsyncGetStatus", 0x1dc8a34), ("XAsyncSchedule", 0x1dc8aa4),
    ]
}

/// Install the local user into the host EXE. Called once.
pub fn install() {
    if crate::cfg("local_user").as_deref() != Some("1") {
        return;
    }
    unsafe {
        let base = GetModuleHandleW(core::ptr::null());
        if base.is_null() {
            return;
        }
        let pe = core::ptr::read_unaligned(base.add(0x3c) as *const u32) as usize;
        let stamp = core::ptr::read_unaligned(base.add(pe + 8) as *const u32);
        let size_of_image = core::ptr::read_unaligned(base.add(pe + 24 + 56) as *const u32) as usize;
        let image = core::slice::from_raw_parts(base as *const u8, size_of_image);
        let Some(found) = crate::gdk_wrappers::scan(image, &[XUSER, XUSER_V2, XUSER_DEVICE, XASYNC]) else {
            crate::record_raw("gdk_user", "install", "host EXE headers/.pdata unreadable - local XUser layer NOT installed");
            return;
        };
        // name -> unique RVA; missing and ambiguous names are reported, never guessed.
        let unique = |iface: Iface, slot: u32| -> Result<usize, usize> {
            let hits: Vec<u32> = found.iter().filter(|w| versions(iface).contains(&w.iface) && w.slot == slot).map(|w| w.rva).collect();
            if hits.len() == 1 { Ok(hits[0] as usize) } else { Err(hits.len()) }
        };
        let mut problems = Vec::new();
        let mut xasync = Vec::new();
        for (name, slot, cell) in xasync_slots() {
            match unique(XASYNC, slot) {
                Ok(rva) => xasync.push((cell, rva)),
                Err(n) => problems.push(format!("{name} ({n} candidates)")),
            }
        }
        for name in ["XUserAddAsync", "XUserAddResult"] {
            let (_, iface, slot, _) = wrappers().into_iter().find(|w| w.0 == name).unwrap();
            if let Err(n) = unique(iface, slot) {
                problems.push(format!("{name} ({n} candidates)"));
            }
        }
        if !problems.is_empty() {
            crate::record_raw("gdk_user", "install", &format!(
                "required GDK wrappers not found unambiguously: {} - local XUser layer NOT installed", problems.join(", ")));
            return;
        }
        for (cell, rva) in xasync {
            cell.store(base as usize + rva, Ordering::SeqCst);
        }

        let gamertag = crate::cfg("gamertag").filter(|g| !g.is_empty() && g.len() <= 32 && g.is_ascii()).unwrap_or_else(|| "TomBoxPlayer".into());
        let xuid = crate::cfg("xuid").and_then(|x| x.parse::<u64>().ok()).unwrap_or(2533274790395905);
        let token = crate::cfg("xbl_token").filter(|t| !t.is_empty()).unwrap_or_else(|| format!("XBL3.0 x=tombox;local-offline-user-{xuid}"));
        let mut t8 = token.clone().into_bytes();
        t8.push(0);
        let t16: Vec<u16> = token.encode_utf16().chain(Some(0)).collect();
        let _ = USER.set(User { gamertag: gamertag.clone(), xuid, token: t8, token16: t16 });

        let table = wrappers();
        let mut ok = 0;
        let mut skipped = Vec::new();
        for (name, iface, slot, target) in table {
            match unique(iface, slot) {
                Ok(rva) if write_jump(base.add(rva), target) => ok += 1,
                Ok(_) => skipped.push(format!("{name} (patch failed)")),
                Err(0) => skipped.push(format!("{name} (not linked)")),
                Err(n) => skipped.push(format!("{name} ({n} candidates)")),
            }
        }
        // Wrappers of these interfaces with slots this module does not implement keep
        // calling the real runtime, which does not know the local user handle.
        let known: Vec<(Iface, u32)> = table.iter().flat_map(|w| versions(w.1).iter().map(move |v| (*v, w.2))).collect();
        let mut other: Vec<String> = found.iter()
            .filter(|w| w.iface != XASYNC && !known.contains(&(w.iface, w.slot)))
            .map(|w| format!("{}slot {:#x} @ {:#x}", if w.iface == XUSER { String::new() } else { format!("IID {} ", hex(&w.iface.iid)) }, w.slot, w.rva))
            .collect();
        other.sort();
        other.dedup();
        crate::record_raw("gdk_user", "install", &format!(
            "local offline user installed: gamertag='{gamertag}' xuid={xuid} ({ok}/{} XUser wrappers redirected{}{})",
            table.len(),
            if skipped.is_empty() { String::new() } else { format!("; not redirected: {}", skipped.join(", ")) },
            if other.is_empty() { String::new() } else { format!("; unhandled XUser wrappers: {}", other.join(", ")) }));

        // The XStore layer is still keyed to one build's addresses.
        if stamp == BUILD_TIMESTAMP {
            crate::gdk_store::install(base);
        } else {
            crate::record_raw("gdk_user", "install", &format!(
                "host EXE timestamp {stamp:#x} is not Fortnite 2.0.1461.7919 - XStore layer NOT installed"));
        }
    }
}
