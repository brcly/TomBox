//! `xapu.dll` compatibility — command queue to the console's audio processors.
//!
//! Imported by FMOD's GameCore build (`fmod.dll`), which uses it only for hardware Opus
//! decoding (`fmod_codec_opus.cpp`). On a PC there is no audio processor to queue work
//! to, so the connection is **refused** and the caller takes its own failure path (FMOD's
//! Opus codec reports an error from its init; other codecs are unaffected).
//!
//! Signatures, from the system `xapu.dll` exports and FMOD's call site:
//!   HRESULT XApuConnect(const PARAMS* params, OUT16* info, HANDLE* connection)
//!     — clears `*connection` and the 16 bytes at `info` before validating `params`;
//!   HRESULT XApuEnqueueCommand(HANDLE connection, const void* command, UINT32* sequence)
//!   HRESULT XApuDequeueResult(HANDLE connection, void* result)
//!   HRESULT XApuDisconnect(HANDLE connection)
//! The contents of `PARAMS`, the command and the result are not decoded here. No
//! connection is ever handed out, so the other three only see handles we never created
//! and fail with `E_HANDLE`.

use core::ffi::c_void;
use compat_common::{record, E_NOTIMPL};

const M: &str = "xapu";
const E_HANDLE: i32 = 0x8007_0006u32 as i32;

#[no_mangle]
pub extern "system" fn XApuConnect(params: *const c_void, info: *mut u8, connection: *mut *mut c_void) -> i32 {
    unsafe {
        if !connection.is_null() {
            *connection = core::ptr::null_mut();
        }
        if !info.is_null() {
            core::ptr::write_bytes(info, 0, 16);
        }
    }
    record(M, "XApuConnect", &format!("params={params:p}: no audio processor on the host -> E_NOTIMPL"));
    E_NOTIMPL
}

#[no_mangle]
pub extern "system" fn XApuEnqueueCommand(connection: *mut c_void, _command: *const c_void, _sequence: *mut u32) -> i32 {
    record(M, "XApuEnqueueCommand", &format!("connection={connection:p} was never created -> E_HANDLE"));
    E_HANDLE
}

#[no_mangle]
pub extern "system" fn XApuDequeueResult(connection: *mut c_void, _result: *mut c_void) -> i32 {
    record(M, "XApuDequeueResult", &format!("connection={connection:p} was never created -> E_HANDLE"));
    E_HANDLE
}

#[no_mangle]
pub extern "system" fn XApuDisconnect(connection: *mut c_void) -> i32 {
    record(M, "XApuDisconnect", &format!("connection={connection:p} was never created -> E_HANDLE"));
    E_HANDLE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_is_refused_and_clears_outputs() {
        let mut info = [0xffu8; 16];
        let mut conn: *mut c_void = 1usize as *mut c_void;
        let params = [0u32; 8];
        let hr = XApuConnect(params.as_ptr() as *const c_void, info.as_mut_ptr(), &mut conn);
        assert!(hr < 0, "must fail so the caller takes its error path");
        assert_eq!(hr, E_NOTIMPL);
        assert!(conn.is_null());
        assert_eq!(info, [0u8; 16]);
        // Null outputs are tolerated.
        assert_eq!(XApuConnect(core::ptr::null(), core::ptr::null_mut(), core::ptr::null_mut()), E_NOTIMPL);
    }

    #[test]
    fn queue_calls_reject_unknown_handles() {
        let h = 0x1234usize as *mut c_void;
        let mut seq = 7u32;
        assert_eq!(XApuEnqueueCommand(h, core::ptr::null(), &mut seq), E_HANDLE);
        assert_eq!(seq, 7, "sequence untouched on failure");
        assert_eq!(XApuDequeueResult(h, core::ptr::null_mut()), E_HANDLE);
        assert_eq!(XApuDisconnect(h), E_HANDLE);
    }
}
