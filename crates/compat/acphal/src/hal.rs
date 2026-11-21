//! `IAcpHal`: the title's command/message channel to the audio processor (ACP), as a
//! *null device*: commands are validated and acknowledged in order, nothing is executed.
//!
//! Interface (vtable of the console's own `AcpHal.dll`, object created by `AcpHalCreate`):
//!   0 `HRESULT Connect(u32 a, u32 b)`: already connected -> 0x8AC80009, `a == 0` -> E_INVALIDARG
//!   1 `HRESULT Disconnect()`: not connected -> 0x8AC8000A
//!   2 `HRESULT SubmitCommand(u32 cmd, u64 cookie, u32 arg, const void* data, u32 arg2)`:
//!     not connected -> 0x8000000E; `cmd` 1..=0x1B; `data` must be NULL for cmd 4 and only
//!     for cmd 4 (else E_INVALIDARG); writes one 0x100-byte command-ring entry
//!     (+0 valid, +4 arg, +8 cookie, +0x10 cmd, +0x14 arg2, +0x18 payload)
//!   3 `bool PopMessage(Message* out)`: copies one 0x1C-byte message from the message ring
//!   4 `u32 PendingMessageCount()`
//!   5 release (no HRESULT), 6..=9 return 0x8AC80F0E on the console build in the dump
//!
//! Message (0x1C bytes): +0 type, +4 status, +8 time (raw / 853.33), +0x10 u64 cookie, +0x18 u32.
//!
//! Which command answers with which message comes from the two consumers in the dumps:
//!   * FMOD's GameCore build submits cmd 2 with a fresh nonzero cookie during
//!     `FMOD_System_Create`, then spins (no timeout) until a type-0x10 message carries that
//!     cookie; it counts out-of-order cookies, so completions are expected in order.
//!   * FMOD submits cmd 1 (cookie 0) four times per mix block and counts four type-2
//!     messages as one finished block; more type-2 messages than cmd-1 submissions would
//!     drive its counter negative, so type 2 is the cmd-1 completion, not a free-running tick.
//!   * Fortnite's XMA decode (cmd 7) waits for the DSP to write a done flag into the XMA
//!     context memory, with a timeout. That is decode work: this device never writes title
//!     memory, so such waits still time out.
//!
//! Not modelled: the SHAPE DSP (no audio is produced), the message time field (0), status
//! codes other than success, and command-ring backpressure. cmd-1 completions are paced at
//! one audio frame each (`FRAME`, an assumption) so a mixer waiting on them runs in real time.

use core::ffi::c_void;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use compat_common::{record, S_OK};
use crate::M;

pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_ILLEGAL_METHOD_CALL: i32 = 0x8000_000Eu32 as i32;
pub const ACP_E_ALREADY_CONNECTED: i32 = 0x8AC8_0009u32 as i32;
pub const ACP_E_NOT_CONNECTED: i32 = 0x8AC8_000Au32 as i32;
pub const ACP_E_NOT_IMPLEMENTED: i32 = 0x8AC8_0F0Eu32 as i32;

pub const MSG_FLOWGRAPH_DONE: u32 = 2;
pub const MSG_COMMAND_DONE: u32 = 0x10;
const CMD_FLOWGRAPH: u32 = 1;
const CMD_NO_DATA: u32 = 4;
const CMD_LAST: u32 = 0x1B;

/// Assumed length of one audio frame: 128 samples at 48 kHz. Not verified against the APU.
pub const FRAME: Duration = Duration::from_nanos(2_666_667);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub kind: u32,
    pub cookie: u64,
}

impl Message {
    fn write(&self, out: *mut u8) {
        let mut b = [0u8; 0x1c];
        b[0..4].copy_from_slice(&self.kind.to_le_bytes());
        b[0x10..0x18].copy_from_slice(&self.cookie.to_le_bytes());
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len()) };
    }
}

pub struct NullApu {
    connected: bool,
    capacity: usize,
    /// FIFO with nondecreasing due times (the device completes commands in order).
    queue: VecDeque<(Instant, Message)>,
    last_due: Option<Instant>,
    pub dropped: u64,
}

impl NullApu {
    pub const fn new() -> Self {
        NullApu { connected: false, capacity: 0, queue: VecDeque::new(), last_due: None, dropped: 0 }
    }

    pub fn connect(&mut self, a: u32, b: u32) -> i32 {
        if self.connected {
            return ACP_E_ALREADY_CONNECTED;
        }
        if a == 0 {
            return E_INVALIDARG;
        }
        self.connected = true;
        // The second argument sizes the console's message ring; bound our queue by it.
        self.capacity = (b.max(a) as usize).max(1);
        S_OK
    }

    pub fn disconnect(&mut self) -> i32 {
        if !self.connected {
            return ACP_E_NOT_CONNECTED;
        }
        *self = NullApu { dropped: self.dropped, ..NullApu::new() };
        S_OK
    }

    pub fn submit(&mut self, now: Instant, cmd: u32, cookie: u64, has_data: bool) -> i32 {
        if !self.connected {
            return E_ILLEGAL_METHOD_CALL;
        }
        if !(1..=CMD_LAST).contains(&cmd) || has_data == (cmd == CMD_NO_DATA) {
            return E_INVALIDARG;
        }
        let start = self.last_due.map_or(now, |d| d.max(now));
        if cmd == CMD_FLOWGRAPH {
            let due = start + FRAME;
            self.push(due, Message { kind: MSG_FLOWGRAPH_DONE, cookie: 0 });
        }
        if cookie != 0 {
            let due = self.last_due.map_or(start, |d| d.max(start));
            self.push(due, Message { kind: MSG_COMMAND_DONE, cookie });
        }
        S_OK
    }

    fn push(&mut self, due: Instant, m: Message) {
        self.last_due = Some(due);
        if self.queue.len() >= self.capacity {
            self.dropped += 1; // the title is not draining messages
            return;
        }
        self.queue.push_back((due, m));
    }

    pub fn pop(&mut self, now: Instant) -> Option<Message> {
        match self.queue.front() {
            Some(&(due, m)) if due <= now => {
                self.queue.pop_front();
                Some(m)
            }
            _ => None,
        }
    }

    pub fn pending(&self, now: Instant) -> u32 {
        self.queue.iter().take_while(|(due, _)| *due <= now).count() as u32
    }
}

static DEVICE: Mutex<NullApu> = Mutex::new(NullApu::new());

fn log_first(slot: usize, msg: impl FnOnce() -> String) {
    static PER: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];
    if PER[slot.min(15)].fetch_add(1, Ordering::Relaxed) < 4 {
        record(M, "IAcpHal", &msg());
    }
}

unsafe extern "system" fn connect(_this: *mut c_void, a: u32, b: u32) -> i32 {
    let hr = DEVICE.lock().unwrap().connect(a, b);
    log_first(0, || format!("Connect({a:#x}, {b:#x}) -> {hr:#010x} (null device: commands are acknowledged, no DSP)"));
    hr
}

unsafe extern "system" fn disconnect(_this: *mut c_void) -> i32 {
    let hr = DEVICE.lock().unwrap().disconnect();
    log_first(1, || format!("Disconnect -> {hr:#010x}"));
    hr
}

unsafe extern "system" fn submit(_this: *mut c_void, cmd: u32, cookie: u64, arg: u32, data: *const c_void, arg2: u32) -> i32 {
    let hr = DEVICE.lock().unwrap().submit(Instant::now(), cmd, cookie, !data.is_null());
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    if n < 8 || hr < 0 {
        record(M, "IAcpHal", &format!(
            "SubmitCommand #{n} cmd={cmd} cookie={cookie:#x} arg={arg:#x} data={data:p} arg2={arg2:#x} -> {hr:#010x}"));
    }
    hr
}

unsafe extern "system" fn pop_message(_this: *mut c_void, out: *mut u8) -> bool {
    if out.is_null() {
        return false;
    }
    match DEVICE.lock().unwrap().pop(Instant::now()) {
        Some(m) => {
            m.write(out);
            true
        }
        None => false,
    }
}

unsafe extern "system" fn pending_messages(_this: *mut c_void) -> u32 {
    DEVICE.lock().unwrap().pending(Instant::now())
}

unsafe extern "system" fn release(_this: *mut c_void) {
    log_first(5, || "release (process-lifetime singleton, kept)".to_string());
}

macro_rules! not_implemented {
    ($($name:ident = $slot:literal),*) => {$(
        unsafe extern "system" fn $name(_this: *mut c_void) -> i32 {
            log_first($slot, || format!("slot {} -> 0x8ac80f0e (not implemented, as on the console)", $slot));
            ACP_E_NOT_IMPLEMENTED
        }
    )*};
}
not_implemented!(s6 = 6, s7 = 7, s8 = 8, s9 = 9);

static VTBL: [AtomicUsize; 10] = [const { AtomicUsize::new(0) }; 10];
static OBJ: AtomicUsize = AtomicUsize::new(0);

/// The process-lifetime `IAcpHal` object (one vtable pointer).
pub fn object() -> *mut c_void {
    let mut obj = OBJ.load(Ordering::Acquire);
    if obj == 0 {
        let slots: [usize; 10] = [
            connect as *const () as usize, disconnect as *const () as usize, submit as *const () as usize,
            pop_message as *const () as usize, pending_messages as *const () as usize, release as *const () as usize,
            s6 as *const () as usize, s7 as *const () as usize, s8 as *const () as usize, s9 as *const () as usize,
        ];
        for (v, f) in VTBL.iter().zip(slots) {
            v.store(f, Ordering::Relaxed);
        }
        let b: &'static mut usize = Box::leak(Box::new(VTBL.as_ptr() as usize));
        let new = b as *mut usize as usize;
        obj = match OBJ.compare_exchange(0, new, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => new,
            Err(existing) => existing,
        };
    }
    obj as *mut c_void
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected() -> NullApu {
        let mut d = NullApu::new();
        assert_eq!(d.connect(0x804, 0x804), S_OK);
        d
    }

    #[test]
    fn connect_and_disconnect_follow_the_console_errors() {
        let mut d = NullApu::new();
        let t = Instant::now();
        assert_eq!(d.submit(t, 2, 1, true), E_ILLEGAL_METHOD_CALL);
        assert_eq!(d.disconnect(), ACP_E_NOT_CONNECTED);
        assert_eq!(d.connect(0, 0x804), E_INVALIDARG);
        assert_eq!(d.connect(0x804, 0x804), S_OK);
        assert_eq!(d.connect(0x804, 0x804), ACP_E_ALREADY_CONNECTED);
        assert_eq!(d.disconnect(), S_OK);
        assert_eq!(d.connect(0x1000, 0x1000), S_OK);
    }

    #[test]
    fn command_validation() {
        let mut d = connected();
        let t = Instant::now();
        assert_eq!(d.submit(t, 0, 0, true), E_INVALIDARG);
        assert_eq!(d.submit(t, 0x1c, 0, true), E_INVALIDARG);
        assert_eq!(d.submit(t, 2, 0, false), E_INVALIDARG, "data required");
        assert_eq!(d.submit(t, 4, 0, true), E_INVALIDARG, "cmd 4 takes no data");
        assert_eq!(d.submit(t, 4, 0, false), S_OK);
        assert_eq!(d.submit(t, 0x1b, 0, true), S_OK);
        assert_eq!(d.pop(t + FRAME * 10), None, "no cookie, not cmd 1: no message");
    }

    /// FMOD_System_Create: cmd 2 with cookie 1, then wait for COMMAND_DONE(1).
    #[test]
    fn cookie_completes_immediately_with_command_done() {
        let mut d = connected();
        let t = Instant::now();
        assert_eq!(d.submit(t, 2, 1, true), S_OK);
        assert_eq!(d.pending(t), 1);
        assert_eq!(d.pop(t), Some(Message { kind: MSG_COMMAND_DONE, cookie: 1 }));
        assert_eq!(d.pop(t), None);
    }

    /// cmd 1 completes one frame after the previous completion; later cookies wait behind it.
    #[test]
    fn flowgraphs_are_paced_and_completions_stay_in_order() {
        let mut d = connected();
        let t = Instant::now();
        for _ in 0..4 {
            assert_eq!(d.submit(t, 1, 0, true), S_OK);
        }
        assert_eq!(d.submit(t, 2, 7, true), S_OK);
        assert_eq!(d.pop(t), None, "first frame not finished yet");
        assert_eq!(d.pending(t + FRAME * 2), 2);
        for i in 1..=4u32 {
            assert_eq!(d.pop(t + FRAME * i - Duration::from_nanos(1)), None);
            assert_eq!(d.pop(t + FRAME * i), Some(Message { kind: MSG_FLOWGRAPH_DONE, cookie: 0 }));
        }
        assert_eq!(d.pop(t + FRAME * 4), Some(Message { kind: MSG_COMMAND_DONE, cookie: 7 }));
        // An idle device starts the next frame from now, not from the old schedule.
        let later = t + Duration::from_secs(1);
        assert_eq!(d.submit(later, 1, 0, true), S_OK);
        assert_eq!(d.pop(later), None);
        assert!(d.pop(later + FRAME).is_some());
    }

    #[test]
    fn messages_beyond_the_ring_capacity_are_dropped() {
        let mut d = NullApu::new();
        assert_eq!(d.connect(2, 2), S_OK);
        let t = Instant::now();
        for c in 1..=3 {
            assert_eq!(d.submit(t, 7, c, true), S_OK);
        }
        assert_eq!(d.dropped, 1);
        assert_eq!(d.pop(t).unwrap().cookie, 1);
        assert_eq!(d.pop(t).unwrap().cookie, 2);
        assert_eq!(d.pop(t), None);
    }

    #[test]
    fn message_layout() {
        let mut b = [0xffu8; 0x20];
        Message { kind: MSG_COMMAND_DONE, cookie: 0x1122_3344_5566_7788 }.write(b.as_mut_ptr());
        assert_eq!(&b[0..4], &0x10u32.to_le_bytes());
        assert_eq!(&b[4..0x10], &[0u8; 12]);
        assert_eq!(&b[0x10..0x18], &0x1122_3344_5566_7788u64.to_le_bytes());
        assert_eq!(&b[0x18..0x1c], &[0u8; 4]);
        assert_eq!(&b[0x1c..], &[0xff; 4], "nothing past the 0x1c-byte message");
    }
}
