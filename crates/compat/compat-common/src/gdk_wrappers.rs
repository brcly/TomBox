//! Locate the GDK API wrappers statically linked into a title.
//!
//! A GDK title does not import `XUser*`/`XAsync*` by name. Each API is a small wrapper in the
//! title's own image that asks `xgameruntime.dll` for an interface (`QueryApiImpl(&guid, &iid,
//! &obj)`) and calls one slot of its vtable:
//!
//! ```text
//!   lea rdx, [rip+IID]        ; or `mov rdx, rcx` when IID == GUID
//!   lea rcx, [rip+GUID]
//!   call QueryApiImpl-thunk
//!   ...
//!   mov rax, [rcx]
//!   mov rax, [rax+slot]       ; the API
//!   call [guard_dispatch]
//!   ...                       ; then slot 0x10 (Release)
//! ```
//!
//! An API is therefore identified by `(GUID, IID, slot)`, independent of the build. The
//! names of the keys below come from Fortnite 2.0.1461.7919, whose symbol table names
//! every wrapper; the same scan over that EXE finds all 34 of them at exactly the
//! symbol-table addresses (`fortnite_symbol_rvas` test). Function bounds come from the
//! image's `.pdata`, so a redirect always starts at a real function entry.
//!
//! Limits: a title that inlines a wrapper, or a compiler that orders the instructions
//! differently, is not found (the API then stays with the real runtime, and the caller
//! logs what is missing). Interface versions with another IID are different keys.

/// A GDK runtime interface: the GUID and IID passed to `QueryApiImpl`, as stored in memory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Iface {
    pub guid: [u8; 16],
    pub iid: [u8; 16],
}

const fn hex16(s: &[u8; 32]) -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = (nib(s[2 * i]) << 4) | nib(s[2 * i + 1]);
        i += 1;
    }
    out
}
const fn nib(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => panic!("hex"),
    }
}

/// `XUser*` (most of the API).
pub(crate) const XUSER: Iface = Iface {
    guid: hex16(b"77d1ac01f9916347a38eccbb55ce32e0"),
    iid: hex16(b"48f99bebdc18824dbbcc40e0a809c4c0"),
};
/// Same object, later interface version (`XUserGetGamertag` with a gamertag component).
pub(crate) const XUSER_V2: Iface = Iface {
    guid: hex16(b"77d1ac01f9916347a38eccbb55ce32e0"),
    iid: hex16(b"c0faf4ce7676944aa1194c43f9eb5b74"),
};
/// `XUserFindForDevice` / device-association registrations.
pub(crate) const XUSER_DEVICE: Iface = Iface {
    guid: hex16(b"9749827ddc10ab4586b72737767c0bf1"),
    iid: hex16(b"9749827ddc10ab4586b72737767c0bf1"),
};
/// The device-association interface again, under the IID a later GDK passes (observed in a
/// second title: same GUID, same slots 0x18/0x20/0x28 and argument shapes as `XUSER_DEVICE`).
pub(crate) const XUSER_DEVICE_V2: Iface = Iface {
    guid: hex16(b"9749827ddc10ab4586b72737767c0bf1"),
    iid: hex16(b"56a9c60ce1e7df4f93419d5da649ebc8"),
};
/// `XAsync*`.
pub(crate) const XASYNC: Iface = Iface {
    guid: hex16(b"cb7d3b07cf1f304094bee3c9eb623428"),
    iid: hex16(b"cb7d3b07cf1f304094bee3c9eb623428"),
};

/// Largest function considered a wrapper (Fortnite's largest is 0xd6 bytes).
const MAX_WRAPPER: u32 = 0x200;

/// One wrapper found in an image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Wrapper {
    pub iface: Iface,
    pub slot: u32,
    pub rva: u32,
}

/// `(GUID, IID, slot)` of the single interface call in `code` (a function at `rva`), or
/// `None` if there is not exactly one. `read16(rva)` returns 16 bytes of the image.
pub(crate) fn analyse(code: &[u8], rva: u32, read16: &dyn Fn(u32) -> Option<[u8; 16]>) -> Option<(Iface, u32)> {
    let ripref = |k: usize| -> Option<[u8; 16]> {
        let disp = i32::from_le_bytes(code.get(k + 3..k + 7)?.try_into().ok()?);
        let target = (rva as i64 + k as i64 + 7 + disp as i64) as u32;
        read16(target)
    };
    let mut found = None;
    let mut i = 0;
    while i + 7 <= code.len() {
        if code[i..i + 3] != [0x48, 0x8D, 0x0D] {
            i += 1;
            continue;
        }
        let guid = ripref(i);
        // IID: `lea rdx,[rip+X]` within 0x20 bytes, or a later `mov rdx, rcx` (IID == GUID).
        let mut iid = None;
        for k in i.saturating_sub(0x20)..(i + 0x20).min(code.len().saturating_sub(7)) {
            if code[k..k + 3] == [0x48, 0x8D, 0x15] {
                iid = ripref(k);
                break;
            }
            if k > i && code[k..k + 3] == [0x48, 0x8B, 0xD1] {
                iid = guid;
                break;
            }
        }
        // Slot: the first `mov rax, [rax+disp8/disp32]` after the GUID load.
        let mut slot = None;
        for j in i + 7..(i + 0x80).min(code.len().saturating_sub(4)) {
            if code[j..j + 3] == [0x48, 0x8B, 0x40] {
                slot = Some(code[j + 3] as u32);
                break;
            }
            if code[j..j + 3] == [0x48, 0x8B, 0x80] && j + 7 <= code.len() {
                slot = Some(u32::from_le_bytes(code[j + 3..j + 7].try_into().unwrap()));
                break;
            }
        }
        if let (Some(guid), Some(iid), Some(slot)) = (guid, iid, slot) {
            if found.is_some() {
                return None; // more than one interface call: not a plain wrapper
            }
            found = Some((Iface { guid, iid }, slot));
        }
        i += 7;
    }
    found
}

/// Every wrapper of an object (GUID) named in `ifaces` in a mapped PE image (`image` =
/// SizeOfImage bytes from the image base), whatever IID it passes, so a caller can report
/// interface versions it does not know. Returns `None` if the headers or the exception directory are malformed.
pub(crate) fn scan(image: &[u8], ifaces: &[Iface]) -> Option<Vec<Wrapper>> {
    let rd32 = |o: usize| image.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let rd16 = |o: usize| image.get(o..o + 2).map(|b| u16::from_le_bytes(b.try_into().unwrap()));
    let pe = rd32(0x3c)? as usize;
    if image.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let opt = pe + 24;
    if rd16(opt)? != 0x20b {
        return None; // PE32+ only
    }
    let (exc_rva, exc_size) = (rd32(opt + 112 + 3 * 8)? as usize, rd32(opt + 112 + 3 * 8 + 4)? as usize);
    let read16 = |rva: u32| -> Option<[u8; 16]> { image.get(rva as usize..rva as usize + 16)?.try_into().ok() };
    let mut out = Vec::new();
    for e in 0..exc_size / 12 {
        let o = exc_rva + 12 * e;
        let (begin, end) = (rd32(o)?, rd32(o + 4)?);
        if end <= begin || end - begin > MAX_WRAPPER {
            continue;
        }
        let Some(code) = image.get(begin as usize..end as usize) else { continue };
        if let Some((iface, slot)) = analyse(code, begin, &read16) {
            if ifaces.iter().any(|k| k.guid == iface.guid) {
                out.push(Wrapper { iface, slot, rva: begin });
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wrapper shaped like Fortnite's `XUserGetId` (separate IID, slot 0x58), placed at
    /// RVA 0x1000 with GUID at 0x2000 and IID at 0x2010.
    fn wrapper(slot_insn: &[u8], iid_insn: Option<u32>) -> Vec<u8> {
        let mut c = vec![0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x30];
        let rip = |c: &Vec<u8>, target: u32| (target as i64 - (0x1000 + c.len() as i64 + 7)) as i32;
        if let Some(iid) = iid_insn {
            let d = rip(&c, iid);
            c.extend_from_slice(&[0x48, 0x8D, 0x15]);
            c.extend_from_slice(&d.to_le_bytes());
        }
        let d = rip(&c, 0x2000);
        c.extend_from_slice(&[0x48, 0x8D, 0x0D]);
        c.extend_from_slice(&d.to_le_bytes());
        if iid_insn.is_none() {
            c.extend_from_slice(&[0x48, 0x8B, 0xD1]); // mov rdx, rcx
        }
        c.extend_from_slice(&[0xE8, 0, 0, 0, 0, 0x8B, 0xD8, 0x48, 0x8B, 0x4C, 0x24, 0x50, 0x48, 0x8B, 0x01]);
        c.extend_from_slice(slot_insn);
        c.extend_from_slice(&[0xFF, 0x15, 0, 0, 0, 0, 0x48, 0x8B, 0x40, 0x10, 0xC3]); // ...Release, ret
        c
    }
    fn reader(guid: [u8; 16], iid: [u8; 16]) -> impl Fn(u32) -> Option<[u8; 16]> {
        move |rva| match rva {
            0x2000 => Some(guid),
            0x2010 => Some(iid),
            _ => None,
        }
    }

    #[test]
    fn finds_slot_with_separate_iid() {
        let code = wrapper(&[0x48, 0x8B, 0x40, 0x58], Some(0x2010));
        let r = reader(XUSER.guid, XUSER.iid);
        assert_eq!(analyse(&code, 0x1000, &r), Some((XUSER, 0x58)));
    }

    #[test]
    fn iid_equal_to_guid_and_disp32_slot() {
        let code = wrapper(&[0x48, 0x8B, 0x80, 0x08, 0x01, 0, 0], None);
        let r = reader(XASYNC.guid, [0; 16]);
        assert_eq!(analyse(&code, 0x1000, &r), Some((XASYNC, 0x108)));
    }

    #[test]
    fn interface_version_is_part_of_the_key() {
        let code = wrapper(&[0x48, 0x8B, 0x40, 0x18], Some(0x2010));
        let r = reader(XUSER_V2.guid, XUSER_V2.iid);
        let (iface, slot) = analyse(&code, 0x1000, &r).unwrap();
        assert_eq!((iface, slot), (XUSER_V2, 0x18));
        assert_ne!(iface, XUSER, "same GUID, other IID: a different interface");
    }

    #[test]
    fn rejects_non_wrappers() {
        let r = reader(XUSER.guid, XUSER.iid);
        assert_eq!(analyse(&[0x90; 64], 0x1000, &r), None, "no interface call");
        let mut two = wrapper(&[0x48, 0x8B, 0x40, 0x58], Some(0x2010));
        let second = wrapper(&[0x48, 0x8B, 0x40, 0x60], Some(0x2010));
        // Second call placed right after the first: rip displacements must be rebased.
        let base = two.len() as u32;
        let shifted: Vec<u8> = {
            let mut s = second.clone();
            for k in [10usize, 17] {
                let d = i32::from_le_bytes(s[k + 3..k + 7].try_into().unwrap()) - base as i32;
                s[k + 3..k + 7].copy_from_slice(&d.to_le_bytes());
            }
            s
        };
        two.extend_from_slice(&shifted);
        assert_eq!(analyse(&two, 0x1000, &r), None, "two interface calls: not a plain wrapper");
        let truncated = &wrapper(&[0x48, 0x8B, 0x40, 0x58], Some(0x2010))[..20];
        assert_eq!(analyse(truncated, 0x1000, &r), None, "no slot load");
    }

    #[test]
    fn scan_uses_pdata_bounds() {
        // Minimal PE32+ image: headers, .pdata at 0x3000 listing one wrapper at 0x1000.
        let mut img = vec![0u8; 0x4000];
        img[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        img[0x80..0x84].copy_from_slice(b"PE\0\0");
        let opt = 0x80 + 24;
        img[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        let dir = opt + 112 + 3 * 8;
        img[dir..dir + 4].copy_from_slice(&0x3000u32.to_le_bytes());
        img[dir + 4..dir + 8].copy_from_slice(&24u32.to_le_bytes());
        let code = wrapper(&[0x48, 0x8B, 0x40, 0x40], Some(0x2010));
        img[0x1000..0x1000 + code.len()].copy_from_slice(&code);
        img[0x2000..0x2010].copy_from_slice(&XUSER.guid);
        img[0x2010..0x2020].copy_from_slice(&XUSER.iid);
        let mut put = |o: usize, v: u32| img[o..o + 4].copy_from_slice(&v.to_le_bytes());
        put(0x3000, 0x1000);
        put(0x3004, 0x1000 + code.len() as u32);
        // A second, oversized entry is ignored.
        put(0x300c, 0x1000);
        put(0x3010, 0x1000 + MAX_WRAPPER + 1);
        let found = scan(&img, &[XUSER, XASYNC]).unwrap();
        assert_eq!(found, vec![Wrapper { iface: XUSER, slot: 0x40, rva: 0x1000 }]);
        assert_eq!(scan(&img, &[XASYNC]).unwrap(), vec![], "only requested interfaces");
        assert!(scan(&img[..0x40], &[XUSER]).is_none(), "truncated headers");

        // Another IID of a requested object is returned (so it can be reported), keyed by
        // what the wrapper passes: here the later device-association interface version.
        img[0x2000..0x2010].copy_from_slice(&XUSER_DEVICE.guid);
        img[0x2010..0x2020].copy_from_slice(&XUSER_DEVICE_V2.iid);
        assert_eq!(scan(&img, &[XUSER_DEVICE]).unwrap(), vec![Wrapper { iface: XUSER_DEVICE_V2, slot: 0x40, rva: 0x1000 }]);
        assert_eq!(scan(&img, &[XUSER]).unwrap(), vec![], "other objects stay out");
    }

    /// Opt-in check against a real title: `TOMBOX_FORTNITE_EXE=<path to
    /// FortniteClient-XSX-Shipping.exe 2.0.1461.7919> cargo test -p compat-common -- --ignored`.
    /// Every wrapper the scan names must sit at the address the build's symbol table gives.
    #[test]
    #[ignore]
    fn fortnite_symbol_rvas() {
        let Some(path) = std::env::var_os("TOMBOX_FORTNITE_EXE") else {
            panic!("set TOMBOX_FORTNITE_EXE");
        };
        let image = map_file(&std::fs::read(path).unwrap());
        let found = scan(&image, &[XUSER, XUSER_V2, XUSER_DEVICE, XASYNC]).unwrap();
        let expected = crate::gdk_user::fortnite_symbol_rvas();
        for (name, rva) in expected {
            let (iface, slot) = crate::gdk_user::key_of(name).unwrap();
            let hits: Vec<u32> = found.iter().filter(|w| w.iface == iface && w.slot == slot).map(|w| w.rva).collect();
            assert_eq!(hits, vec![rva as u32], "{name}");
        }
    }

    /// Lay a PE file out at its section RVAs (what the loader does, minus relocations).
    fn map_file(file: &[u8]) -> Vec<u8> {
        let rd32 = |o: usize| u32::from_le_bytes(file[o..o + 4].try_into().unwrap()) as usize;
        let rd16 = |o: usize| u16::from_le_bytes(file[o..o + 2].try_into().unwrap()) as usize;
        let pe = rd32(0x3c);
        let (nsec, optsz) = (rd16(pe + 6), rd16(pe + 20));
        let size_of_image = rd32(pe + 24 + 56);
        let size_of_headers = rd32(pe + 24 + 60);
        let mut img = vec![0u8; size_of_image];
        img[..size_of_headers].copy_from_slice(&file[..size_of_headers]);
        for s in 0..nsec {
            let h = pe + 24 + optsz + 40 * s;
            let (va, raw_size, raw_ptr) = (rd32(h + 12), rd32(h + 16), rd32(h + 20));
            let n = raw_size.min(size_of_image - va).min(file.len() - raw_ptr);
            img[va..va + n].copy_from_slice(&file[raw_ptr..raw_ptr + n]);
        }
        img
    }
}
