//! In-process network policy for the title.
//!
//! Project rule: the title must never talk to Fortnite/Epic production services. The title
//! and the SDKs it links use WinHTTP and Winsock directly, so the policy is enforced inside
//! the title process, at the API entry points, before anything leaves the machine.
//!
//! Two modes:
//!
//!   * **deny (default)** — every name resolution, outbound connect/sendto and WinHTTP
//!     connection fails locally. Resolutions that cannot leave the machine are passed
//!     through: a null/empty node (the wildcard or loopback address a socket binds to, e.g.
//!     `getaddrinfo(NULL, port, AI_PASSIVE)`), `localhost`, this machine's own names, and IP
//!     literals. `connect` / `WSAConnect` / `sendto` / `WSASendTo` are allowed towards
//!     loopback only (a title talking to its own sockets). Denying those leaves a title
//!     unable to start even a local UDP peer.
//!   * **redirect** — `TOMBOX_NET_REDIRECT=<host>` (the project owner's own server). Every
//!     host name the title asks for is replaced by `<host>`:
//!       - `getaddrinfo` / `GetAddrInfoW` / `GetAddrInfoExW/A` / `gethostbyname` /
//!         `WSAConnectByNameW/A` resolve `<host>` instead of the requested name;
//!       - `WinHttpConnect` connects to `<host>` (same port), so TLS validates against
//!         `<host>`'s certificate and the Host header names `<host>`;
//!       - `connect` / `WSAConnect` / `sendto` / `WSASendTo` are allowed only towards
//!         loopback or an address `<host>` resolves to; any other destination is refused.
//!     Null/empty names, `localhost` and this machine's own name are passed through.
//!
//! Mechanism: each entry point is overwritten with `mov rax, imm64; jmp rax` to a hook. Where
//! a hook must call the original (redirect mode, and the four resolvers and four socket
//! calls of deny mode) the displaced instructions are relocated into a trampoline (iced-x86
//! block encoder). In deny mode an entry point whose trampoline cannot be built gets the
//! plain failing stub instead.
//!
//! Limits (documented, not hidden): a socket connected through `ConnectEx` to a literal IP
//! address is not covered; DNS-over-anything not using these APIs is not covered; other
//! processes are unaffected. Only one compat DLL per process installs the policy.
//! `TOMBOX_ALLOW_NETWORK=1` disables the policy entirely (never use with this title).

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

extern "system" {
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, protect: u32) -> *mut c_void;
    fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
    fn SetLastError(code: u32);
    fn GetComputerNameExA(kind: u32, buf: *mut u8, size: *mut u32) -> i32;
    fn GetTickCount64() -> u64;
}

const WSAENETUNREACH: u32 = 10051;
const WSAHOST_NOT_FOUND: i32 = 11001;
const ERROR_WINHTTP_CANNOT_CONNECT: u32 = 12029;

static DENIED: AtomicUsize = AtomicUsize::new(0);
static REDIRECTED: AtomicUsize = AtomicUsize::new(0);

fn note(what: &str) {
    let n = DENIED.fetch_add(1, Ordering::Relaxed);
    if n < 40 || n % 500 == 0 {
        crate::record_raw("net_guard", "denied", &format!("{what} (total blocked calls: {})", n + 1));
    }
}

fn note_redirect(api: &str, original: &str) {
    let n = REDIRECTED.fetch_add(1, Ordering::Relaxed);
    if n < 80 || n % 500 == 0 {
        crate::record_raw("net_guard", "redirect", &format!(
            "{api}: '{original}' -> '{}' (total redirected: {})", target().map(|t| t.name.as_str()).unwrap_or("?"), n + 1));
    }
}

// ---- deny stubs (argument-agnostic: the x64 caller cleans up) --------------------------------

unsafe extern "system" fn deny_sock() -> i32 {
    note("socket connect/send");
    SetLastError(WSAENETUNREACH); // WSAGetLastError reads the thread's last-error value
    -1
}
unsafe extern "system" fn deny_sock_bool() -> i32 {
    note("WSAConnectByName/List");
    SetLastError(WSAENETUNREACH);
    0
}
unsafe extern "system" fn deny_resolve() -> i32 {
    note("name resolution");
    SetLastError(WSAHOST_NOT_FOUND as u32);
    WSAHOST_NOT_FOUND
}
unsafe extern "system" fn deny_null() -> usize {
    note("WinHTTP connect / gethostbyname");
    SetLastError(ERROR_WINHTTP_CANNOT_CONNECT);
    0
}
unsafe extern "system" fn deny_false() -> i32 {
    note("WinHttpSendRequest");
    SetLastError(ERROR_WINHTTP_CANNOT_CONNECT);
    0
}

// ---- patching ---------------------------------------------------------------------------------

pub(crate) unsafe fn resolve_export(module: &[u8], func: &[u8]) -> *mut u8 {
    let mut h = GetModuleHandleA(module.as_ptr());
    if h.is_null() {
        h = LoadLibraryA(module.as_ptr());
    }
    if h.is_null() {
        return core::ptr::null_mut();
    }
    GetProcAddress(h, func.as_ptr()) as *mut u8
}

pub(crate) unsafe fn write_jump(p: *mut u8, stub: usize) -> bool {
    let mut code = [0u8; 12]; // mov rax, imm64 ; jmp rax
    code[0] = 0x48;
    code[1] = 0xB8;
    code[2..10].copy_from_slice(&(stub as u64).to_le_bytes());
    code[10] = 0xFF;
    code[11] = 0xE0;
    let mut old = 0u32;
    if VirtualProtect(p as *mut c_void, code.len(), 0x40, &mut old) == 0 {
        return false;
    }
    core::ptr::copy_nonoverlapping(code.as_ptr(), p, code.len());
    let mut tmp = 0u32;
    VirtualProtect(p as *mut c_void, code.len(), old, &mut tmp);
    FlushInstructionCache(GetCurrentProcess(), p as *const c_void, code.len());
    true
}

/// Executable page within +-1 GiB of `near` (so rip-relative operands can be re-encoded).
unsafe fn alloc_near(near: usize) -> *mut u8 {
    let base = near & !0xFFFF;
    for i in 1..0x3F00usize {
        for cand in [base.wrapping_sub(i << 16), base.wrapping_add(i << 16)] {
            let p = VirtualAlloc(cand as *mut c_void, 0x1000, 0x3000, 0x40);
            if !p.is_null() {
                return p as *mut u8;
            }
        }
    }
    core::ptr::null_mut()
}

/// Relocate the first >= 12 bytes of `p` into a trampoline that continues in the original.
unsafe fn make_trampoline(p: *mut u8) -> Option<usize> {
    use iced_x86::{BlockEncoder, BlockEncoderOptions, Decoder, DecoderOptions, FlowControl, InstructionBlock};
    let bytes = core::slice::from_raw_parts(p, 32);
    let mut dec = Decoder::with_ip(64, bytes, p as u64, DecoderOptions::NONE);
    let mut insns = Vec::new();
    let mut len = 0usize;
    while len < 12 {
        let i = dec.decode();
        if i.is_invalid() {
            return None;
        }
        // A return or indirect branch inside the displaced bytes means the function is too
        // short / oddly shaped to hook safely.
        if matches!(i.flow_control(), FlowControl::Return | FlowControl::IndirectBranch | FlowControl::Interrupt | FlowControl::Exception) {
            return None;
        }
        len += i.len();
        insns.push(i);
    }
    let tramp = alloc_near(p as usize);
    if tramp.is_null() {
        return None;
    }
    let block = InstructionBlock::new(&insns, tramp as u64);
    let enc = BlockEncoder::encode(64, block, BlockEncoderOptions::NONE).ok()?;
    let code = enc.code_buffer;
    if code.len() + 14 > 0x1000 {
        return None;
    }
    core::ptr::copy_nonoverlapping(code.as_ptr(), tramp, code.len());
    // jmp qword ptr [rip+0] ; dq original+len
    let tail = tramp.add(code.len());
    let j: [u8; 6] = [0xFF, 0x25, 0, 0, 0, 0];
    core::ptr::copy_nonoverlapping(j.as_ptr(), tail, 6);
    core::ptr::write_unaligned(tail.add(6) as *mut u64, p as u64 + len as u64);
    FlushInstructionCache(GetCurrentProcess(), tramp as *const c_void, 0x1000);
    Some(tramp as usize)
}

// ---- redirect mode ------------------------------------------------------------------------------

struct Target {
    name: String,
    ansi: Vec<u8>,  // NUL-terminated
    wide: Vec<u16>, // NUL-terminated
}
static TARGET: OnceLock<Option<Target>> = OnceLock::new();

fn target() -> Option<&'static Target> {
    TARGET.get().and_then(|t| t.as_ref())
}

/// Trampolines to the original functions, indexed by `H_*`.
const H_GETADDRINFO: usize = 0;
const H_GETADDRINFOW: usize = 1;
const H_GETADDRINFOEXW: usize = 2;
const H_GETADDRINFOEXA: usize = 3;
const H_GETHOSTBYNAME: usize = 4;
const H_CONNECT: usize = 5;
const H_WSACONNECT: usize = 6;
const H_CONNECTBYNAMEW: usize = 7;
const H_CONNECTBYNAMEA: usize = 8;
const H_SENDTO: usize = 9;
const H_WSASENDTO: usize = 10;
const H_WINHTTPCONNECT: usize = 11;
static ORIG: [AtomicUsize; 12] = [const { AtomicUsize::new(0) }; 12];

macro_rules! orig {
    ($idx:expr, fn($($at:ty),*) -> $rt:ty) => {{
        let f: unsafe extern "system" fn($($at),*) -> $rt = core::mem::transmute(ORIG[$idx].load(Ordering::Relaxed));
        f
    }};
}

unsafe fn cstr(p: *const u8) -> String {
    let mut n = 0;
    while n < 1024 && *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf8_lossy(core::slice::from_raw_parts(p, n)).into_owned()
}
unsafe fn wstr(p: *const u16) -> String {
    let mut n = 0;
    while n < 1024 && *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
}

fn local_names() -> &'static Vec<String> {
    static N: OnceLock<Vec<String>> = OnceLock::new();
    N.get_or_init(|| {
        let mut v = Vec::new();
        for kind in [0u32, 1, 3] { // NetBIOS, DnsHostname, DnsFullyQualified
            let mut buf = [0u8; 256];
            let mut n = buf.len() as u32;
            if unsafe { GetComputerNameExA(kind, buf.as_mut_ptr(), &mut n) } != 0 && n > 0 {
                v.push(String::from_utf8_lossy(&buf[..n as usize]).to_ascii_lowercase());
            }
        }
        v
    })
}

/// Should this requested name be left alone (not a remote host)?
/// Deny mode: a name whose resolution never queries a name server.
fn local_only(name: &str) -> bool {
    let n = name.trim();
    n.is_empty() || n.eq_ignore_ascii_case("localhost") || n.parse::<std::net::IpAddr>().is_ok()
}

/// This machine's own names resolve from its interface list, without a name server.
fn local_or_own(name: &str) -> bool {
    local_only(name) || local_names().iter().any(|l| l.eq_ignore_ascii_case(name.trim()))
}

/// Deny mode: loopback destinations only (and no explicit destination on a connected socket).
unsafe fn loopback_dest(sa: *const u8, len: i32) -> bool {
    sa.is_null() || sockaddr_ip(sa, len).is_some_and(|ip| is_loopback(&ip))
}

unsafe extern "system" fn d_connect(s: usize, name: *const u8, len: i32) -> i32 {
    if !loopback_dest(name, len) {
        deny_dest_why("connect", name, len, "not loopback");
        return -1;
    }
    orig!(H_CONNECT, fn(usize, *const u8, i32) -> i32)(s, name, len)
}

unsafe extern "system" fn d_wsaconnect(s: usize, name: *const u8, len: i32, a: *mut c_void, b: *mut c_void, c: *mut c_void, d: *mut c_void) -> i32 {
    if !loopback_dest(name, len) {
        deny_dest_why("WSAConnect", name, len, "not loopback");
        return -1;
    }
    orig!(H_WSACONNECT, fn(usize, *const u8, i32, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32)(s, name, len, a, b, c, d)
}

unsafe extern "system" fn d_sendto(s: usize, buf: *const u8, len: i32, flags: i32, to: *const u8, tolen: i32) -> i32 {
    if !loopback_dest(to, tolen) {
        deny_dest_why("sendto", to, tolen, "not loopback");
        return -1;
    }
    orig!(H_SENDTO, fn(usize, *const u8, i32, i32, *const u8, i32) -> i32)(s, buf, len, flags, to, tolen)
}

unsafe extern "system" fn d_wsasendto(
    s: usize, bufs: *const c_void, count: u32, sent: *mut u32, flags: u32, to: *const u8, tolen: i32, ov: *mut c_void, routine: *const c_void,
) -> i32 {
    if !loopback_dest(to, tolen) {
        deny_dest_why("WSASendTo", to, tolen, "not loopback");
        return -1;
    }
    orig!(H_WSASENDTO, fn(usize, *const c_void, u32, *mut u32, u32, *const u8, i32, *mut c_void, *const c_void) -> i32)(s, bufs, count, sent, flags, to, tolen, ov, routine)
}

fn deny_resolution(api: &str, name: &str) -> i32 {
    note(&format!("name resolution ({api} '{name}')"));
    unsafe { SetLastError(WSAHOST_NOT_FOUND as u32) };
    WSAHOST_NOT_FOUND
}

unsafe extern "system" fn d_getaddrinfo(node: *const u8, service: *const u8, hints: *const c_void, res: *mut *mut u8) -> i32 {
    let f = orig!(H_GETADDRINFO, fn(*const u8, *const u8, *const c_void, *mut *mut u8) -> i32);
    if node.is_null() || local_or_own(&cstr(node)) {
        return f(node, service, hints, res);
    }
    deny_resolution("getaddrinfo", &cstr(node))
}

unsafe extern "system" fn d_getaddrinfow(node: *const u16, service: *const u16, hints: *const c_void, res: *mut *mut u8) -> i32 {
    let f = orig!(H_GETADDRINFOW, fn(*const u16, *const u16, *const c_void, *mut *mut u8) -> i32);
    if node.is_null() || local_or_own(&wstr(node)) {
        return f(node, service, hints, res);
    }
    deny_resolution("GetAddrInfoW", &wstr(node))
}

unsafe extern "system" fn d_getaddrinfoexw(
    name: *const u16, service: *const u16, ns: u32, nsp: *const c_void, hints: *const c_void, res: *mut *mut u8,
    timeout: *const c_void, ov: *mut c_void, routine: *const c_void, handle: *mut *mut c_void,
) -> i32 {
    let f: Ex<u16> = core::mem::transmute(ORIG[H_GETADDRINFOEXW].load(Ordering::Relaxed));
    if name.is_null() || local_or_own(&wstr(name)) {
        return f(name, service, ns, nsp, hints, res, timeout, ov, routine, handle);
    }
    deny_resolution("GetAddrInfoExW", &wstr(name))
}

unsafe extern "system" fn d_getaddrinfoexa(
    name: *const u8, service: *const u8, ns: u32, nsp: *const c_void, hints: *const c_void, res: *mut *mut u8,
    timeout: *const c_void, ov: *mut c_void, routine: *const c_void, handle: *mut *mut c_void,
) -> i32 {
    let f: Ex<u8> = core::mem::transmute(ORIG[H_GETADDRINFOEXA].load(Ordering::Relaxed));
    if name.is_null() || local_or_own(&cstr(name)) {
        return f(name, service, ns, nsp, hints, res, timeout, ov, routine, handle);
    }
    deny_resolution("GetAddrInfoExA", &cstr(name))
}

fn passthrough(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    n.is_empty() || n == "localhost" || n == "127.0.0.1" || n == "::1" || n == "0.0.0.0" || n == "::"
        || local_names().iter().any(|l| *l == n)
        || target().map_or(false, |t| t.name.eq_ignore_ascii_case(&n))
}

// Allowed destination addresses (IPv4 stored v4-mapped).
static ALLOWED: Mutex<Vec<[u8; 16]>> = Mutex::new(Vec::new());
static LAST_REFRESH: AtomicUsize = AtomicUsize::new(0);

fn v4(a: [u8; 4]) -> [u8; 16] {
    let mut o = [0u8; 16];
    o[10] = 0xff;
    o[11] = 0xff;
    o[12..].copy_from_slice(&a);
    o
}

unsafe fn sockaddr_ip(sa: *const u8, len: i32) -> Option<[u8; 16]> {
    if sa.is_null() || len < 8 {
        return None;
    }
    match core::ptr::read_unaligned(sa as *const u16) {
        2 if len >= 8 => Some(v4([*sa.add(4), *sa.add(5), *sa.add(6), *sa.add(7)])),
        23 if len >= 24 => {
            let mut o = [0u8; 16];
            core::ptr::copy_nonoverlapping(sa.add(8), o.as_mut_ptr(), 16);
            Some(o)
        }
        _ => None,
    }
}

fn is_loopback(ip: &[u8; 16]) -> bool {
    let mut l6 = [0u8; 16];
    l6[15] = 1;
    *ip == l6 || (ip[..10] == [0u8; 10] && ip[10] == 0xff && ip[11] == 0xff && ip[12] == 127)
}

/// Resolve the redirect host with the original getaddrinfo and remember its addresses.
unsafe fn refresh_allowed() {
    let Some(t) = target() else { return };
    let f = orig!(H_GETADDRINFO, fn(*const u8, *const u8, *const c_void, *mut *mut u8) -> i32);
    // Winsock may not be initialised yet when the policy is installed.
    static WSA: std::sync::Once = std::sync::Once::new();
    WSA.call_once(|| {
        let p = resolve_export(b"ws2_32.dll\0", b"WSAStartup\0");
        if !p.is_null() {
            let startup: unsafe extern "system" fn(u16, *mut u8) -> i32 = core::mem::transmute(p);
            let mut data = [0u8; 512];
            startup(0x0202, data.as_mut_ptr());
        }
    });
    let mut res: *mut u8 = core::ptr::null_mut();
    let rc = f(t.ansi.as_ptr(), core::ptr::null(), core::ptr::null(), &mut res);
    LAST_REFRESH.store(GetTickCount64() as usize, Ordering::Relaxed);
    if rc != 0 || res.is_null() {
        crate::record_raw("net_guard", "resolve", &format!("'{}' did not resolve (error {rc}) - outbound sockets stay refused", t.name));
        return;
    }
    let mut found = Vec::new();
    let mut p = res;
    while !p.is_null() {
        // addrinfo: ai_addrlen @16 (size_t), ai_addr @32, ai_next @40
        let alen = core::ptr::read_unaligned(p.add(16) as *const usize) as i32;
        let addr = core::ptr::read_unaligned(p.add(32) as *const *const u8);
        if let Some(ip) = sockaddr_ip(addr, alen) {
            if !found.contains(&ip) {
                found.push(ip);
            }
        }
        p = core::ptr::read_unaligned(p.add(40) as *const *mut u8);
    }
    let free = resolve_export(b"ws2_32.dll\0", b"freeaddrinfo\0");
    if !free.is_null() {
        let free: unsafe extern "system" fn(*mut u8) = core::mem::transmute(free);
        free(res);
    }
    let mut g = ALLOWED.lock().unwrap();
    let before = g.len();
    for ip in found {
        if !g.contains(&ip) {
            g.push(ip);
        }
    }
    if g.len() != before {
        crate::record_raw("net_guard", "resolve", &format!("'{}' -> {} address(es) allowed for outbound sockets", t.name, g.len()));
    }
}

/// May the title send to this socket address?
unsafe fn dest_allowed(sa: *const u8, len: i32) -> bool {
    if sa.is_null() {
        return true; // connected socket / no explicit destination
    }
    let Some(ip) = sockaddr_ip(sa, len) else {
        return false; // unknown address family
    };
    if is_loopback(&ip) || ALLOWED.lock().unwrap().contains(&ip) {
        return true;
    }
    // The redirect host's addresses may have changed: refresh at most every 10 s.
    let now = GetTickCount64() as usize;
    if now.wrapping_sub(LAST_REFRESH.load(Ordering::Relaxed)) > 10_000 {
        refresh_allowed();
        return ALLOWED.lock().unwrap().contains(&ip);
    }
    false
}

fn deny_dest(api: &str, sa: *const u8, len: i32) {
    deny_dest_why(api, sa, len, "not the redirect host");
}

fn deny_dest_why(api: &str, sa: *const u8, len: i32, why: &str) {
    let ip = unsafe { sockaddr_ip(sa, len) };
    let s = match ip {
        Some(ip) if ip[..10] == [0u8; 10] && ip[10] == 0xff => format!("{}.{}.{}.{}", ip[12], ip[13], ip[14], ip[15]),
        Some(ip) => ip.chunks(2).map(|c| format!("{:02x}{:02x}", c[0], c[1])).collect::<Vec<_>>().join(":"),
        None => "unknown-family".into(),
    };
    note(&format!("{api} to {s} ({why})"));
    unsafe { SetLastError(WSAENETUNREACH) };
}

unsafe extern "system" fn h_getaddrinfo(node: *const u8, service: *const u8, hints: *const c_void, res: *mut *mut u8) -> i32 {
    let f = orig!(H_GETADDRINFO, fn(*const u8, *const u8, *const c_void, *mut *mut u8) -> i32);
    if node.is_null() || passthrough(&cstr(node)) {
        return f(node, service, hints, res);
    }
    note_redirect("getaddrinfo", &cstr(node));
    f(target().unwrap().ansi.as_ptr(), service, hints, res)
}

unsafe extern "system" fn h_getaddrinfow(node: *const u16, service: *const u16, hints: *const c_void, res: *mut *mut u8) -> i32 {
    let f = orig!(H_GETADDRINFOW, fn(*const u16, *const u16, *const c_void, *mut *mut u8) -> i32);
    if node.is_null() || passthrough(&wstr(node)) {
        return f(node, service, hints, res);
    }
    note_redirect("GetAddrInfoW", &wstr(node));
    f(target().unwrap().wide.as_ptr(), service, hints, res)
}

type Ex<T> = unsafe extern "system" fn(*const T, *const T, u32, *const c_void, *const c_void, *mut *mut u8, *const c_void, *mut c_void, *const c_void, *mut *mut c_void) -> i32;

unsafe extern "system" fn h_getaddrinfoexw(
    name: *const u16, service: *const u16, ns: u32, nsp: *const c_void, hints: *const c_void, res: *mut *mut u8,
    timeout: *const c_void, ov: *mut c_void, routine: *const c_void, handle: *mut *mut c_void,
) -> i32 {
    let f: Ex<u16> = core::mem::transmute(ORIG[H_GETADDRINFOEXW].load(Ordering::Relaxed));
    if name.is_null() || passthrough(&wstr(name)) {
        return f(name, service, ns, nsp, hints, res, timeout, ov, routine, handle);
    }
    note_redirect("GetAddrInfoExW", &wstr(name));
    f(target().unwrap().wide.as_ptr(), service, ns, nsp, hints, res, timeout, ov, routine, handle)
}

unsafe extern "system" fn h_getaddrinfoexa(
    name: *const u8, service: *const u8, ns: u32, nsp: *const c_void, hints: *const c_void, res: *mut *mut u8,
    timeout: *const c_void, ov: *mut c_void, routine: *const c_void, handle: *mut *mut c_void,
) -> i32 {
    let f: Ex<u8> = core::mem::transmute(ORIG[H_GETADDRINFOEXA].load(Ordering::Relaxed));
    if name.is_null() || passthrough(&cstr(name)) {
        return f(name, service, ns, nsp, hints, res, timeout, ov, routine, handle);
    }
    note_redirect("GetAddrInfoExA", &cstr(name));
    f(target().unwrap().ansi.as_ptr(), service, ns, nsp, hints, res, timeout, ov, routine, handle)
}

unsafe extern "system" fn h_gethostbyname(name: *const u8) -> *mut c_void {
    let f = orig!(H_GETHOSTBYNAME, fn(*const u8) -> *mut c_void);
    if name.is_null() || passthrough(&cstr(name)) {
        return f(name);
    }
    note_redirect("gethostbyname", &cstr(name));
    f(target().unwrap().ansi.as_ptr())
}

unsafe extern "system" fn h_connect(s: usize, name: *const u8, len: i32) -> i32 {
    if !dest_allowed(name, len) {
        deny_dest("connect", name, len);
        return -1;
    }
    orig!(H_CONNECT, fn(usize, *const u8, i32) -> i32)(s, name, len)
}

unsafe extern "system" fn h_wsaconnect(s: usize, name: *const u8, len: i32, a: *mut c_void, b: *mut c_void, c: *mut c_void, d: *mut c_void) -> i32 {
    if !dest_allowed(name, len) {
        deny_dest("WSAConnect", name, len);
        return -1;
    }
    orig!(H_WSACONNECT, fn(usize, *const u8, i32, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32)(s, name, len, a, b, c, d)
}

type ByName<T> = unsafe extern "system" fn(usize, *const T, *const T, *mut u32, *mut c_void, *mut u32, *mut c_void, *const c_void, *mut c_void) -> i32;

unsafe extern "system" fn h_connectbynamew(
    s: usize, node: *const u16, service: *const u16, ll: *mut u32, la: *mut c_void, rl: *mut u32, ra: *mut c_void, timeout: *const c_void, reserved: *mut c_void,
) -> i32 {
    let f: ByName<u16> = core::mem::transmute(ORIG[H_CONNECTBYNAMEW].load(Ordering::Relaxed));
    if node.is_null() || passthrough(&wstr(node)) {
        return f(s, node, service, ll, la, rl, ra, timeout, reserved);
    }
    note_redirect("WSAConnectByNameW", &wstr(node));
    f(s, target().unwrap().wide.as_ptr(), service, ll, la, rl, ra, timeout, reserved)
}

unsafe extern "system" fn h_connectbynamea(
    s: usize, node: *const u8, service: *const u8, ll: *mut u32, la: *mut c_void, rl: *mut u32, ra: *mut c_void, timeout: *const c_void, reserved: *mut c_void,
) -> i32 {
    let f: ByName<u8> = core::mem::transmute(ORIG[H_CONNECTBYNAMEA].load(Ordering::Relaxed));
    if node.is_null() || passthrough(&cstr(node)) {
        return f(s, node, service, ll, la, rl, ra, timeout, reserved);
    }
    note_redirect("WSAConnectByNameA", &cstr(node));
    f(s, target().unwrap().ansi.as_ptr(), service, ll, la, rl, ra, timeout, reserved)
}

unsafe extern "system" fn h_sendto(s: usize, buf: *const u8, len: i32, flags: i32, to: *const u8, tolen: i32) -> i32 {
    if !dest_allowed(to, tolen) {
        deny_dest("sendto", to, tolen);
        return -1;
    }
    orig!(H_SENDTO, fn(usize, *const u8, i32, i32, *const u8, i32) -> i32)(s, buf, len, flags, to, tolen)
}

unsafe extern "system" fn h_wsasendto(
    s: usize, bufs: *const c_void, count: u32, sent: *mut u32, flags: u32, to: *const u8, tolen: i32, ov: *mut c_void, routine: *const c_void,
) -> i32 {
    if !dest_allowed(to, tolen) {
        deny_dest("WSASendTo", to, tolen);
        return -1;
    }
    orig!(H_WSASENDTO, fn(usize, *const c_void, u32, *mut u32, u32, *const u8, i32, *mut c_void, *const c_void) -> i32)(s, bufs, count, sent, flags, to, tolen, ov, routine)
}

unsafe extern "system" fn h_winhttpconnect(session: *mut c_void, server: *const u16, port: u16, reserved: u32) -> *mut c_void {
    let f = orig!(H_WINHTTPCONNECT, fn(*mut c_void, *const u16, u16, u32) -> *mut c_void);
    if server.is_null() || passthrough(&wstr(server)) {
        return f(session, server, port, reserved);
    }
    note_redirect(&format!("WinHttpConnect port {port}"), &wstr(server));
    f(session, target().unwrap().wide.as_ptr(), port, reserved)
}

// ---- installation -------------------------------------------------------------------------------

fn install_deny() {
    let ws = b"ws2_32.dll\0";
    let wh = b"winhttp.dll\0";
    // Resolvers keep local-only lookups working (see the module docs). They need the
    // original, so they go through a trampoline; without one they fail closed.
    let resolvers: [(&[u8], usize, usize, usize); 8] = [
        (b"getaddrinfo\0", H_GETADDRINFO, d_getaddrinfo as usize, deny_resolve as usize),
        (b"GetAddrInfoW\0", H_GETADDRINFOW, d_getaddrinfow as usize, deny_resolve as usize),
        (b"GetAddrInfoExW\0", H_GETADDRINFOEXW, d_getaddrinfoexw as usize, deny_resolve as usize),
        (b"GetAddrInfoExA\0", H_GETADDRINFOEXA, d_getaddrinfoexa as usize, deny_resolve as usize),
        (b"connect\0", H_CONNECT, d_connect as usize, deny_sock as usize),
        (b"WSAConnect\0", H_WSACONNECT, d_wsaconnect as usize, deny_sock as usize),
        (b"sendto\0", H_SENDTO, d_sendto as usize, deny_sock as usize),
        (b"WSASendTo\0", H_WSASENDTO, d_wsasendto as usize, deny_sock as usize),
    ];
    let mut ok = 0;
    let mut local = 0;
    let mut missing = Vec::new();
    for (f, idx, hook, deny) in resolvers {
        let name = String::from_utf8_lossy(&f[..f.len() - 1]).into_owned();
        let p = unsafe { resolve_export(ws, f) };
        if p.is_null() {
            missing.push(name);
            continue;
        }
        let patched = match unsafe { make_trampoline(p) } {
            Some(t) => {
                ORIG[idx].store(t, Ordering::SeqCst);
                let done = unsafe { write_jump(p, hook) };
                local += usize::from(done);
                done
            }
            None => unsafe { write_jump(p, deny) },
        };
        if patched { ok += 1 } else { missing.push(name) }
    }
    let table: [(&[u8], &[u8], usize); 6] = [
        (ws, b"gethostbyname\0", deny_null as usize),
        (ws, b"WSAConnectByNameW\0", deny_sock_bool as usize),
        (ws, b"WSAConnectByNameA\0", deny_sock_bool as usize),
        (wh, b"WinHttpConnect\0", deny_null as usize),
        (wh, b"WinHttpOpenRequest\0", deny_null as usize),
        (wh, b"WinHttpSendRequest\0", deny_false as usize),
    ];
    for (m, f, stub) in table {
        let p = unsafe { resolve_export(m, f) };
        if !p.is_null() && unsafe { write_jump(p, stub) } {
            ok += 1;
        } else {
            missing.push(String::from_utf8_lossy(&f[..f.len() - 1]).into_owned());
        }
    }
    crate::record_raw("net_guard", "installed", &format!(
        "{ok}/14 network entry points blocked in-process ({local}/8 resolver and socket entry points pass local-only names and loopback){}",
        if missing.is_empty() { String::new() } else { format!(" (not patched: {})", missing.join(", ")) }));
}

fn install_redirect(host: &str) {
    let ws = b"ws2_32.dll\0";
    let wh = b"winhttp.dll\0";
    // (module, export, index, hook, deny stub used if the original cannot be trampolined)
    let table: [(&[u8], &[u8], usize, usize, usize); 12] = [
        (ws, b"getaddrinfo\0", H_GETADDRINFO, h_getaddrinfo as usize, deny_resolve as usize),
        (ws, b"GetAddrInfoW\0", H_GETADDRINFOW, h_getaddrinfow as usize, deny_resolve as usize),
        (ws, b"GetAddrInfoExW\0", H_GETADDRINFOEXW, h_getaddrinfoexw as usize, deny_resolve as usize),
        (ws, b"GetAddrInfoExA\0", H_GETADDRINFOEXA, h_getaddrinfoexa as usize, deny_resolve as usize),
        (ws, b"gethostbyname\0", H_GETHOSTBYNAME, h_gethostbyname as usize, deny_null as usize),
        (ws, b"connect\0", H_CONNECT, h_connect as usize, deny_sock as usize),
        (ws, b"WSAConnect\0", H_WSACONNECT, h_wsaconnect as usize, deny_sock as usize),
        (ws, b"WSAConnectByNameW\0", H_CONNECTBYNAMEW, h_connectbynamew as usize, deny_sock_bool as usize),
        (ws, b"WSAConnectByNameA\0", H_CONNECTBYNAMEA, h_connectbynamea as usize, deny_sock_bool as usize),
        (ws, b"sendto\0", H_SENDTO, h_sendto as usize, deny_sock as usize),
        (ws, b"WSASendTo\0", H_WSASENDTO, h_wsasendto as usize, deny_sock as usize),
        (wh, b"WinHttpConnect\0", H_WINHTTPCONNECT, h_winhttpconnect as usize, deny_null as usize),
    ];
    let (mut hooked, mut denied, mut missing) = (0, Vec::new(), Vec::new());
    for (m, f, idx, hook, deny) in table {
        let name = String::from_utf8_lossy(&f[..f.len() - 1]).into_owned();
        let p = unsafe { resolve_export(m, f) };
        if p.is_null() {
            missing.push(name);
            continue;
        }
        match unsafe { make_trampoline(p) } {
            Some(t) => {
                ORIG[idx].store(t, Ordering::SeqCst);
                if unsafe { write_jump(p, hook) } { hooked += 1 } else { missing.push(name) }
            }
            None => {
                // Cannot call the original safely: fail closed for this entry point.
                if unsafe { write_jump(p, deny) } { denied.push(name) } else { missing.push(name) }
            }
        }
    }
    crate::record_raw("net_guard", "installed", &format!(
        "REDIRECT mode: every host -> '{host}'; {hooked}/12 entry points hooked{}{}",
        if denied.is_empty() { String::new() } else { format!("; fail-closed (no trampoline): {}", denied.join(", ")) },
        if missing.is_empty() { String::new() } else { format!("; NOT PATCHED: {}", missing.join(", ")) }));
    if ORIG[H_GETADDRINFO].load(Ordering::Relaxed) != 0 {
        unsafe { refresh_allowed() };
    }
}

/// Install the policy once per process (the first compat DLL to log wins).
pub fn ensure() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Several compat DLLs link this crate; only one may patch the process.
        let marker = format!("{}", unsafe { GetCurrentProcessId() });
        if std::env::var("TOMBOX_NET_GUARD_OWNER").ok().as_deref() == Some(marker.as_str()) {
            let _ = TARGET.set(None);
            return;
        }
        std::env::set_var("TOMBOX_NET_GUARD_OWNER", &marker);

        // Process-wide one-time setup that is not network policy: the local Xbox user.
        crate::gdk_user::install();
        // Optional `locale=` override (user language reported to the title).
        if let Some(locale) = crate::cfg("locale").filter(|v| !v.is_empty()) {
            crate::locale::install(&locale);
        }

        if std::env::var_os("TOMBOX_ALLOW_NETWORK").map(|v| v == "1").unwrap_or(false) {
            let _ = TARGET.set(None);
            crate::record_raw("net_guard", "disabled", "TOMBOX_ALLOW_NETWORK=1 — title network access NOT restricted");
            return;
        }
        // Redirect host: environment first, else a `net_redirect=<host>` line in `tuxbox.cfg`
        // next to the title executable (so a plain double-click launch gets the same policy).
        let from_cfg = || crate::cfg("net_redirect");
        let host = std::env::var("TOMBOX_NET_REDIRECT").ok().or_else(from_cfg).map(|h| h.trim().to_string()).filter(|h| !h.is_empty());
        match host {
            Some(h) if h.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') => {
                let mut ansi = h.clone().into_bytes();
                ansi.push(0);
                let wide: Vec<u16> = h.encode_utf16().chain(Some(0)).collect();
                let _ = TARGET.set(Some(Target { name: h.clone(), ansi, wide }));
                install_redirect(&h);
            }
            Some(h) => {
                let _ = TARGET.set(None);
                crate::record_raw("net_guard", "config", &format!("TOMBOX_NET_REDIRECT='{h}' is not a plain host name - falling back to deny-all"));
                install_deny();
            }
            None => {
                let _ = TARGET.set(None);
                install_deny();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{local_only, loopback_dest};

    fn sockaddr_in(ip: [u8; 4]) -> [u8; 16] {
        let mut sa = [0u8; 16];
        sa[0] = 2; // AF_INET
        sa[2..4].copy_from_slice(&9u16.to_be_bytes());
        sa[4..8].copy_from_slice(&ip);
        sa
    }

    fn sockaddr_in6(ip: std::net::Ipv6Addr) -> [u8; 28] {
        let mut sa = [0u8; 28];
        sa[0] = 23; // AF_INET6 on Windows
        sa[2..4].copy_from_slice(&9u16.to_be_bytes());
        sa[8..24].copy_from_slice(&ip.octets());
        sa
    }

    #[test]
    fn deny_mode_destinations_are_loopback_only() {
        unsafe {
            assert!(loopback_dest(core::ptr::null(), 0), "connected socket, no destination");
            for ip in [[127, 0, 0, 1], [127, 5, 6, 7]] {
                let sa = sockaddr_in(ip);
                assert!(loopback_dest(sa.as_ptr(), sa.len() as i32), "{ip:?}");
            }
            // Broadcast, private and wildcard addresses, and a short sockaddr_in.
            for ip in [[192, 168, 0, 255], [10, 0, 0, 1], [0, 0, 0, 0], [192, 0, 2, 1], [255, 255, 255, 255]] {
                let sa = sockaddr_in(ip);
                assert!(!loopback_dest(sa.as_ptr(), sa.len() as i32), "{ip:?}");
            }
            assert!(!loopback_dest(sockaddr_in([127, 0, 0, 1]).as_ptr(), 4), "truncated");
            for ip in ["::1", "::ffff:127.0.0.1"] {
                let sa = sockaddr_in6(ip.parse().unwrap());
                assert!(loopback_dest(sa.as_ptr(), sa.len() as i32), "{ip}");
            }
            for ip in ["ff02::1", "fe80::1", "::", "::ffff:8.8.8.8", "2001:db8::1"] {
                let sa = sockaddr_in6(ip.parse().unwrap());
                assert!(!loopback_dest(sa.as_ptr(), sa.len() as i32), "{ip}");
            }
            let short = sockaddr_in6("::1".parse().unwrap());
            assert!(!loopback_dest(short.as_ptr(), 20), "truncated sockaddr_in6");
            let mut other = sockaddr_in([127, 0, 0, 1]);
            other[0] = 6; // neither AF_INET nor AF_INET6
            assert!(!loopback_dest(other.as_ptr(), other.len() as i32), "unknown family");
        }
    }

    #[test]
    fn only_names_that_cannot_reach_a_name_server_are_local() {
        for n in ["", "  ", "localhost", "LocalHost", "0.0.0.0", "127.0.0.1", "::", "::1", "192.0.2.20", "fe80::1"] {
            assert!(local_only(n), "{n:?} needs no name server");
        }
        for n in ["title-1234.services.example.net", "localhost.example.com", "127.0.0.1.nip.io", "myhost"] {
            assert!(!local_only(n), "{n:?} would be a DNS query");
        }
    }
}
