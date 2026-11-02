//! Port module: vint value helpers and handle lookups (doc 49, "Port candidates" and the value
//! setters / accessors / stream readers around them), checked against the disassembly.
//!
//! These are small LTCG leaves with many callers, so every port goes through a `pushad` thunk:
//! all registers come back as the caller left them except EAX, which carries the result. That is
//! a superset of what each original preserves. Where an original returns AL only, the upper
//! bytes of EAX are reproduced as the original leaves them (the caller's, a payload dword, a
//! pointer, a stream cursor, a CF mask...), so the whole of EAX is the same.
//!
//! Shadow check: for the first `SHADOW_CALLS` calls of each port a relocated copy of the
//! original runs on the same seven registers (and stack argument), all seven are captured, and
//! its EAX / memory writes are compared with the Rust version's; on a mismatch the original's
//! are kept. The writers (`vint_value_init`, the setters, `vint_value_get_color_bytes`, the
//! stream readers) run on a snapshot of the bytes they may write, restored in between. The
//! functions with a jump table (equals, init, to_bool, to_int) pin the table too; their copy
//! dispatches into the original's code, which is still intact past the 5 patched bytes.
//!
//! Skipped (already ported elsewhere): the datagroup / callback / dataitem-by-hash finders
//! (port_leaves.rs), `vint_value_to_double`, `vint_value_set_float`, `vint_value_set_vec2` and
//! the property getters (port_leaves_fp.rs).

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

const STRING_CRC32_LOWER: usize = 0x00BD_C9B0; // ECX = string (NULL -> 0) -> EAX
const VEC3_EQUAL: usize = 0x00B9_2CF0; // ECX = &a, EDX = &b -> EAX 0 / 1
const VEC2_EQUAL: usize = 0x00B9_2CC0; // ECX = &a, EDX = &b -> EAX 0 / 1
const FTOL2_SSE: usize = 0x00DA_0140; // ST0 -> EAX (truncated)

// --- registers ------------------------------------------------------------------------------------

/// The `pushad` block, lowest address first.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)] // esp is never read
struct Pushad {
    edi: u32,
    esi: u32,
    ebp: u32,
    esp: u32,
    ebx: u32,
    edx: u32,
    ecx: u32,
    eax: u32,
}

/// The seven general registers in and out of `call_full`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Regs {
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esi: u32,
    edi: u32,
    ebp: u32,
}

impl Regs {
    fn of(p: &Pushad) -> Regs {
        Regs { eax: p.eax, ecx: p.ecx, edx: p.edx, ebx: p.ebx, esi: p.esi, edi: p.edi, ebp: p.ebp }
    }
    fn get(&self, i: usize) -> u32 {
        [self.eax, self.ecx, self.edx, self.ebx, self.esi, self.edi, self.ebp][i]
    }
}

const REG_NAMES: [&str; 7] = ["eax", "ecx", "edx", "ebx", "esi", "edi", "ebp"];
const EAX: u8 = 1;
const ECX: u8 = 2;
const EDX: u8 = 4;
/// EBX / ESI / EDI / EBP: callee-saved in every original here.
const SAVED: u8 = 8 | 16 | 32 | 64;

/// The layout `call_full`'s asm reads and writes by offset.
#[repr(C)]
#[allow(dead_code)]
struct Block {
    f: u32,
    r: Regs,
    arg: u32,
    esp: u32,
    st0: f64,
    want_st0: u32,
}

/// Call `f` with all seven registers from `r` and one stack argument (harmless for the callees
/// that take none; a `ret 4` callee pops it), then capture all seven registers into `r`.
unsafe fn call_full(f: usize, r: &mut Regs, arg: u32) {
    let mut b = Block { f: f as u32, r: *r, arg, esp: 0, st0: 0.0, want_st0: 0 };
    unsafe {
        // Above the argument: the target, then the block pointer twice, so that it is at
        // [esp + 12] after `push ebx` whether the callee popped its argument or not.
        std::arch::asm!(
            "push ebp", "push ebx", "push esi", "push edi",
            "mov [eax + 36], esp",
            "push eax",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 4]",
            "push ebx",
            "mov ebx, [esp + 12]",
            "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
            "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "cmp dword ptr [ebx + 48], 0",
            "je 2f",
            "fstp qword ptr [ebx + 40]",
            "2:",
            "mov esp, [ebx + 36]",
            "pop edi", "pop esi", "pop ebx", "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

/// A naked thunk: `pushad`, then `f(&mut block, &stack args)`, then `popad`. The Rust body puts
/// its result in the block's EAX.
macro_rules! regs_thunk {
    ($name:ident, $f:ident) => {
        regs_thunk!($name, $f, "ret");
    };
    ($name:ident, $f:ident, $ret:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "pushad",
                "mov eax, esp",
                "lea ecx, [esp + 36]",
                "push ecx",
                "push eax",
                "call {f}",
                "add esp, 8",
                "popad",
                $ret,
                f = sym $f,
            )
        }
    };
}

// --- the port table -------------------------------------------------------------------------------

struct Port {
    name: &'static str,
    addr: usize,
    /// Registers the original leaves unchanged on every path (checked in the shadow).
    preserved: u8,
    calls: AtomicU32,
    mismatches: AtomicU32,
    original: AtomicUsize,
}

const fn port(name: &'static str, addr: usize, preserved: u8) -> Port {
    Port { name, addr, preserved, calls: AtomicU32::new(0), mismatches: AtomicU32::new(0), original: AtomicUsize::new(0) }
}

const VALUE_EQUALS: usize = 0;
const VALUE_INIT: usize = 1;
const BITMAP_OF_VALUE: usize = 2;
const BITMAP_BY_HANDLE: usize = 3;
const FONT_BY_HANDLE: usize = 4;
const SOUND_BY_HANDLE: usize = 5;
const DATAITEM_BY_HANDLE: usize = 6;
const DOC_FIND: usize = 7;
const COLOR_BYTES: usize = 8;
const FONT_OF_VALUE: usize = 9;
const LUA_CALL_BY_HANDLE: usize = 10;
const LUA_CALL_BY_NAME: usize = 11;
const DATAITEM_BY_NAME: usize = 12;
const FONT_BY_PLATFORM: usize = 13;
const SET_BOOL: usize = 14;
const SET_COLOR_BYTES: usize = 15;
const SET_CALLBACK: usize = 16;
const TO_BOOL: usize = 17;
const TO_INT: usize = 18;
const STREAM_U32: usize = 19;
const STREAM_U32_B: usize = 20;
const STREAM_U32_C: usize = 21;
const STREAM_U8: usize = 22;
const COUNT: usize = 23;

static PORTS: [Port; COUNT] = [
    port("vint_value_equals", 0x00B9_3920, SAVED),
    port("vint_value_init", 0x00B9_3A10, EAX | SAVED),
    port("vint_value_get_bitmap_record", 0x00B9_33E0, SAVED),
    port("vint_bitmap_record_by_handle", 0x00B9_BE50, ECX | SAVED),
    port("vint_font_record_by_handle", 0x00B9_B860, ECX | SAVED),
    port("vint_sound_record_by_handle", 0x00BA_0540, ECX | SAVED),
    port("vint_dataitem_find_by_handle", 0x00B9_5220, ECX | SAVED),
    port("vint_document_find_by_name", 0x00B8_B3F0, SAVED),
    port("vint_value_get_color_bytes", 0x00B8_CFA0, ECX | SAVED),
    port("vint_value_get_font_record", 0x00B9_3420, SAVED),
    port("vint_lua_call_find_by_handle", 0x00B9_1C60, ECX | SAVED),
    port("vint_lua_call_find_by_name", 0x00B9_1C20, SAVED),
    port("vint_dataitem_find_by_name", 0x00B9_51E0, SAVED),
    port("vint_font_find_by_platform_id", 0x00B9_B890, EDX | SAVED),
    port("vint_value_set_bool", 0x0089_BFD0, EAX | EDX | SAVED),
    port("vint_value_set_color_bytes", 0x00B8_A0F0, EAX | SAVED),
    port("vint_value_set_callback", 0x00B9_3720, EAX | EDX | SAVED),
    port("vint_value_to_bool", 0x0077_3120, ECX | EDX | SAVED),
    port("vint_value_to_int", 0x005E_4930, SAVED),
    port("vint_stream_read_u32", 0x00B8_CCC0, ECX | SAVED),
    port("vint_stream_read_u32_b", 0x00B8_CDA0, ECX | SAVED),
    port("vint_stream_read_u32_c", 0x00B9_9060, ECX | SAVED),
    port("vint_stream_read_u8", 0x00B8_CDF0, SAVED),
];

/// Count a call; `Some((call number, original copy))` while it should be shadowed.
fn shadowing(id: usize) -> Option<(u32, usize)> {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_vint2: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    (orig != 0 && n < SHADOW_CALLS).then_some((n, orig))
}

fn mismatch(id: usize, n: u32, what: String) {
    let m = PORTS[id].mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!("port_vint2: {} mismatch (call {n}): {what}", PORTS[id].name));
    }
}

static PRESERVE_LOGS: AtomicU32 = AtomicU32::new(0);

/// Log when the original changed a register it was found to preserve (the thunks preserve all).
fn check_preserved(id: usize, inp: &Regs, out: &Regs) {
    for i in 0..7 {
        if PORTS[id].preserved & (1 << i) != 0 && inp.get(i) != out.get(i) && PRESERVE_LOGS.fetch_add(1, Ordering::Relaxed) < 10 {
            log(&format!(
                "port_vint2: {}: the original changed {} ({:#x} -> {:#x})",
                PORTS[id].name,
                REG_NAMES[i],
                inp.get(i),
                out.get(i)
            ));
        }
    }
}

/// Run the original copy on the caller's registers (and stack argument); returns its outputs.
fn run_original(id: usize, orig: usize, p: &Pushad, arg: u32) -> Regs {
    let inp = Regs::of(p);
    let mut r = inp;
    unsafe { call_full(orig, &mut r, arg) };
    check_preserved(id, &inp, &r);
    r
}

/// The shadow for the register-only pure ports: the original copy's EAX wins on a mismatch.
fn checked_eax(id: usize, p: &Pushad, arg: u32, ours: u32) -> u32 {
    let Some((n, orig)) = shadowing(id) else { return ours };
    let r = run_original(id, orig, p, arg);
    if r.eax != ours {
        mismatch(id, n, format!("in {:x?} arg {arg:#x}: original {:#x}, Rust {ours:#x}", Regs::of(p), r.eax));
        return r.eax;
    }
    ours
}

unsafe fn capture(ranges: &[(usize, usize)]) -> Vec<u8> {
    let mut v = Vec::new();
    for &(a, n) in ranges {
        v.extend_from_slice(unsafe { std::slice::from_raw_parts(a as *const u8, n) });
    }
    v
}

unsafe fn restore(ranges: &[(usize, usize)], data: &[u8]) {
    let mut o = 0;
    for &(a, n) in ranges {
        unsafe { std::ptr::copy_nonoverlapping(data[o..].as_ptr(), a as *mut u8, n) };
        o += n;
    }
}

/// The shadow for the writers: the original runs first on the real memory, its bytes in
/// `ranges` and its EAX are taken, the bytes are put back, the Rust version runs and is
/// compared; on a mismatch the original's bytes and EAX stand.
fn checked_writer(id: usize, p: &Pushad, arg: u32, ranges: &[(usize, usize)], ours: impl FnOnce() -> u32) -> u32 {
    let Some((n, orig)) = shadowing(id) else { return ours() };
    unsafe {
        let before = capture(ranges);
        let t = run_original(id, orig, p, arg);
        let theirs = capture(ranges);
        restore(ranges, &before);
        let e = ours();
        let mine = capture(ranges);
        if e != t.eax || mine != theirs {
            mismatch(
                id,
                n,
                format!("in {:x?} arg {arg:#x}: original eax {:#x} {theirs:x?}, Rust eax {e:#x} {mine:x?} (before {before:x?})", Regs::of(p), t.eax),
            );
            restore(ranges, &theirs);
            return t.eax;
        }
        e
    }
}

/// EAX with AL replaced by a bool.
fn lo(eax: u32, al: bool) -> u32 {
    (eax & !0xFF) | al as u32
}

// --- the handle tables ----------------------------------------------------------------------------

const BITMAPS: u32 = 0x031B_C518; // 0xF0 records of 0x24, handle at +0x14
const FONTS: u32 = 0x031B_C400; // 0xA records of 0x1C, handle at +8
const SOUNDS: u32 = 0x031B_E700; // 0x32 records of 0x18, handle at +8
const DATAITEMS: u32 = 0x031A_96F0; // 0x80 records of 0x22C, handle at +0x224
const LUA_CALLS: u32 = 0x031A_4A80; // 0x20 records of 0x260, handle at +0x210
const FONTS_END: u32 = 0x031B_C518;

/// `h == 0` or `(h & 0xFFFF) >= count` (unsigned) gives 0; else the record at `base + idx *
/// stride` only when its handle dword (`hoff`) equals `h` (the `cmp / setne / sub 1 / and` mask).
unsafe fn record(h: u32, base: u32, stride: u32, count: u32, hoff: u32) -> u32 {
    if h == 0 {
        return 0;
    }
    let idx = h & 0xFFFF;
    if idx >= count {
        return 0;
    }
    let rec = base.wrapping_add(idx.wrapping_mul(stride));
    if unsafe { rd(rec.wrapping_add(hoff) as usize) } == h { rec } else { 0 }
}

unsafe fn bitmap_by_handle(h: u32) -> u32 {
    unsafe { record(h, BITMAPS, 0x24, 0xF0, 0x14) }
}
unsafe fn font_by_handle(h: u32) -> u32 {
    unsafe { record(h, FONTS, 0x1C, 0xA, 8) }
}
unsafe fn sound_by_handle(h: u32) -> u32 {
    unsafe { record(h, SOUNDS, 0x18, 0x32, 8) }
}
unsafe fn dataitem_by_handle(h: u32) -> u32 {
    unsafe { record(h, DATAITEMS, 0x22C, 0x80, 0x224) }
}
unsafe fn lua_call_by_handle(h: u32) -> u32 {
    unsafe { record(h, LUA_CALLS, 0x260, 0x20, 0x210) }
}

/// A tag-9 / tag-10 value's resource record: 0 for any other tag (EAX = &value).
unsafe fn bitmap_of_value(v: u32) -> u32 {
    unsafe { if rd(v as usize) == 9 { bitmap_by_handle(rd(v as usize + 4)) } else { 0 } }
}
unsafe fn font_of_value(v: u32) -> u32 {
    unsafe { if rd(v as usize) == 0xA { font_by_handle(rd(v as usize + 4)) } else { 0 } }
}

macro_rules! lookup_entry {
    ($entry:ident, $thunk:ident, $id:ident, $reg:ident, $f:ident) => {
        extern "C" fn $entry(r: &mut Pushad, _: *const u32) {
            let p = *r;
            r.eax = checked_eax($id, &p, 0, unsafe { $f(p.$reg) });
        }
        regs_thunk!($thunk, $entry);
    };
}

// 0xB933E0 (EAX = &value), 0xB93420 (EAX = &value)
lookup_entry!(bitmap_of_value_entry, bitmap_of_value_thunk, BITMAP_OF_VALUE, eax, bitmap_of_value);
lookup_entry!(font_of_value_entry, font_of_value_thunk, FONT_OF_VALUE, eax, font_of_value);
// 0xB9BE50, 0xB9B860, 0xBA0540, 0xB95220, 0xB91C60 (ECX = handle)
lookup_entry!(bitmap_by_handle_entry, bitmap_by_handle_thunk, BITMAP_BY_HANDLE, ecx, bitmap_by_handle);
lookup_entry!(font_by_handle_entry, font_by_handle_thunk, FONT_BY_HANDLE, ecx, font_by_handle);
lookup_entry!(sound_by_handle_entry, sound_by_handle_thunk, SOUND_BY_HANDLE, ecx, sound_by_handle);
lookup_entry!(dataitem_by_handle_entry, dataitem_by_handle_thunk, DATAITEM_BY_HANDLE, ecx, dataitem_by_handle);
lookup_entry!(lua_call_by_handle_entry, lua_call_by_handle_thunk, LUA_CALL_BY_HANDLE, ecx, lua_call_by_handle);

// --- 0xB9B890 vint_font_find_by_platform_id (EDX = platform id -> EAX) ----------------------------

/// The font record whose first dword is the id (a zero id gives 0), walking the 10 records.
unsafe fn font_by_platform(id: u32) -> u32 {
    if id == 0 {
        return 0;
    }
    let mut p = FONTS;
    while p < FONTS_END {
        if unsafe { rd(p as usize) } == id {
            return p;
        }
        p += 0x1C;
    }
    0
}
lookup_entry!(font_by_platform_entry, font_by_platform_thunk, FONT_BY_PLATFORM, edx, font_by_platform);

// --- by-name finders: crc32 then a ring walk ------------------------------------------------------

/// `string_crc32_lower(name)` (the original, by address), then a walk of the ring at `head`
/// (read once; stops on coming round to it or on a null link) comparing the dword at `hoff`.
/// A null name gives 0; a zero hash gives 0 unless `zero_hash_ok` (the documents compare it).
unsafe fn find_by_name(name: u32, head: usize, hoff: u32, next: u32, zero_hash_ok: bool) -> u32 {
    unsafe {
        if name == 0 {
            return 0;
        }
        let hash = call_regs(STRING_CRC32_LOWER, 0, name, 0, 0, 0, &[]);
        if hash == 0 && !zero_hash_ok {
            return 0;
        }
        let first = rd(head);
        if first == 0 {
            return 0;
        }
        let mut n = first;
        loop {
            if rd(n.wrapping_add(hoff) as usize) == hash {
                return n;
            }
            n = rd(n.wrapping_add(next) as usize);
            if n == first || n == 0 {
                return 0;
            }
        }
    }
}

unsafe fn doc_find(name: u32) -> u32 {
    unsafe { find_by_name(name, 0x0252_A1AC, 0x48, 0, true) }
}
unsafe fn lua_call_by_name(name: u32) -> u32 {
    unsafe { find_by_name(name, 0x0252_A1C8, 8, 0, false) }
}
unsafe fn dataitem_by_name(name: u32) -> u32 {
    unsafe { find_by_name(name, 0x0252_A1F4, 0x10, 0xC, false) }
}
// 0xB8B3F0, 0xB91C20, 0xB951E0 (ECX = name)
lookup_entry!(doc_find_entry, doc_find_thunk, DOC_FIND, ecx, doc_find);
lookup_entry!(lua_call_by_name_entry, lua_call_by_name_thunk, LUA_CALL_BY_NAME, ecx, lua_call_by_name);
lookup_entry!(dataitem_by_name_entry, dataitem_by_name_thunk, DATAITEM_BY_NAME, ecx, dataitem_by_name);

// --- 0xB93920 vint_value_equals (EAX = &a, EDX = &b -> AL) ----------------------------------------

/// Different or empty tags, and tags 12+ (`tag - 1 > 10`), are false (AL = 0, the rest of EAX is
/// &a). Tags 1, 2, 8..11 compare the payload dword (EAX = the payload with AL replaced); 3
/// compares the floats with `fucompp / test ah, 0x44 / jp` (equal and ordered only; EAX is 0 /
/// 1); 4 compares the string pointers, then byte pairs (EAX = 1 when equal; a difference leaves
/// the `sbb` sign mask: 0xFFFFFF00 when a's byte is the lower, else 0); 5 the payload byte (EAX =
/// &a with AL replaced); 6 / 7 tail-jump to the original vec3 / vec2 equality (EAX theirs).
unsafe fn value_equals(a: u32, b: u32) -> u32 {
    unsafe {
        let ta = rd(a as usize);
        if ta != rd(b as usize) || ta == 0 || ta.wrapping_sub(1) > 10 {
            return lo(a, false);
        }
        let (pa, pb) = (a as usize + 4, b as usize + 4);
        match ta {
            3 => (f32::from_bits(rd(pa)) == f32::from_bits(rd(pb))) as u32,
            4 => {
                let (mut p, mut q) = (rd(pa), rd(pb));
                if p == q {
                    return lo(p, true);
                }
                if p == 0 || q == 0 {
                    return lo(p, false);
                }
                let differ = |d: u8, e: u8| if d < e { 0xFFFF_FF00 } else { 0 };
                loop {
                    let (d, e) = (rb(p as usize), rb(q as usize));
                    if d != e {
                        return differ(d, e);
                    }
                    if d == 0 {
                        return 1;
                    }
                    let (d, e) = (rb(p as usize + 1), rb(q as usize + 1));
                    if d != e {
                        return differ(d, e);
                    }
                    p = p.wrapping_add(2);
                    q = q.wrapping_add(2);
                    if d == 0 {
                        return 1;
                    }
                }
            }
            5 => lo(a, rb(pa) == rb(pb)),
            6 => call_regs(VEC3_EQUAL, a, pa as u32, pb as u32, 0, 0, &[]),
            7 => call_regs(VEC2_EQUAL, a, pa as u32, pb as u32, 0, 0, &[]),
            _ => {
                let x = rd(pa);
                lo(x, x == rd(pb))
            }
        }
    }
}

extern "C" fn value_equals_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(VALUE_EQUALS, &p, 0, unsafe { value_equals(p.eax, p.edx) });
}
regs_thunk!(value_equals_thunk, value_equals_entry);

// --- 0xB93A10 vint_value_init (EAX = &value, ECX = type; no result) -------------------------------

/// Only on an empty value: tag = type, then the payload zeroed for types 1..11 (3 / 1 / 2 / 4 /
/// 8..11 one dword, 5 one byte, 6 three dwords, 7 two); type 12 and above (and 0) write the tag
/// only. EAX stays &value.
unsafe fn value_init(v: u32, ty: u32) -> u32 {
    unsafe {
        let a = v as usize;
        if rd(a) != 0 {
            return v;
        }
        wd(a, ty);
        match ty {
            1..=4 | 8..=11 => wd(a + 4, 0),
            5 => wb(a + 4, 0),
            6 => {
                wd(a + 4, 0);
                wd(a + 8, 0);
                wd(a + 12, 0);
            }
            7 => {
                wd(a + 4, 0);
                wd(a + 8, 0);
            }
            _ => {}
        }
        v
    }
}

extern "C" fn value_init_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_writer(VALUE_INIT, &p, 0, &[(p.eax as usize, 16)], || unsafe { value_init(p.eax, p.ecx) });
}
regs_thunk!(value_init_thunk, value_init_entry);

// --- 0xB8CFA0 vint_value_get_color_bytes (ECX = u8[4] out, EDX = &value -> EAX = out) ------------

/// `fistp` with the control word forced to chop: the truncated integer, or the "integer
/// indefinite" 0x80000000 when it does not fit (NaN, infinities, huge); only the low byte is kept.
fn chop_byte(x: f64) -> u8 {
    let i: i32 = if x > -2147483649.0 && x < 2147483648.0 { x as i32 } else { i32::MIN };
    i as u8
}

/// The four bytes are zeroed (the tag was read before that); a colour (tag 6) then gets byte 3 =
/// 0xFF and bytes 0..2 = chop(channel * 255.0) in the original's order (r, g, b, each channel
/// read after the previous store). The product of an f32 and 255 is exact in f64.
unsafe fn color_bytes(out: u32, v: u32) -> u32 {
    unsafe {
        let (o, a) = (out as usize, v as usize);
        let tag = rd(a);
        for i in 0..4 {
            wb(o + i, 0);
        }
        if tag != 6 {
            return out;
        }
        let r = rf(a + 4) as f64;
        wb(o + 3, 0xFF);
        wb(o, chop_byte(r * 255.0));
        let g = rf(a + 8) as f64;
        wb(o + 1, chop_byte(g * 255.0));
        let b = rf(a + 12) as f64;
        wb(o + 2, chop_byte(b * 255.0));
        out
    }
}

extern "C" fn color_bytes_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_writer(COLOR_BYTES, &p, 0, &[(p.ecx as usize, 4)], || unsafe { color_bytes(p.ecx, p.edx) });
}
regs_thunk!(color_bytes_thunk, color_bytes_entry);

// --- the small setters (EAX = &value) -------------------------------------------------------------

/// 0x89BFD0 (stdcall (u8 b)): an empty value becomes tag 5; tag 5 stores the low byte of the
/// argument at +4; any other tag is left alone. EAX stays &value.
unsafe fn set_bool(v: u32, b: u32) -> u32 {
    unsafe {
        let a = v as usize;
        let t = rd(a);
        if t == 0 || t == 5 {
            if t == 0 {
                wd(a, 5);
            }
            wb(a + 4, b as u8);
        }
        v
    }
}

extern "C" fn set_bool_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let b = unsafe { *args };
    r.eax = checked_writer(SET_BOOL, &p, b, &[(p.eax as usize, 16)], || unsafe { set_bool(p.eax, b) });
}
regs_thunk!(set_bool_thunk, set_bool_entry, "ret 4");

/// 0xB8A0F0 (stdcall (u32 packed rgb)): an empty value becomes tag 6, tag 6 gets r / g / b =
/// byte0 / 1 / 2 as a double divided by 255.0 (53-bit x87), stored as f32; byte 3 is ignored.
unsafe fn set_color_bytes(v: u32, packed: u32) -> u32 {
    unsafe {
        let a = v as usize;
        let t = rd(a);
        if t == 0 || t == 6 {
            if t == 0 {
                wd(a, 6);
            }
            wf(a + 4, ((packed & 0xFF) as f64 / 255.0) as f32);
            wf(a + 8, (((packed >> 8) & 0xFF) as f64 / 255.0) as f32);
            wf(a + 12, (((packed >> 16) & 0xFF) as f64 / 255.0) as f32);
        }
        v
    }
}

extern "C" fn set_color_bytes_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let c = unsafe { *args };
    r.eax = checked_writer(SET_COLOR_BYTES, &p, c, &[(p.eax as usize, 16)], || unsafe { set_color_bytes(p.eax, c) });
}
regs_thunk!(set_color_bytes_thunk, set_color_bytes_entry, "ret 4");

/// 0xB93720 (EDX = callback or 0): an empty value or a tag-8 one is set to tag 8 (even when it
/// already was); a callback gives its handle (+0x18) after `refcount (+0x1C) += 1`, none gives
/// handle 0. EAX stays &value.
unsafe fn set_callback(v: u32, cb: u32) -> u32 {
    unsafe {
        let a = v as usize;
        let t = rd(a);
        if t == 0 || t == 8 {
            wd(a, 8);
            if cb != 0 {
                let h = rd(cb as usize + 0x18);
                wd(cb as usize + 0x1C, rd(cb as usize + 0x1C).wrapping_add(1));
                wd(a + 4, h);
            } else {
                wd(a + 4, 0);
            }
        }
        v
    }
}

extern "C" fn set_callback_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let mut ranges = vec![(p.eax as usize, 16)];
    if unsafe { matches!(rd(p.eax as usize), 0 | 8) } && p.edx != 0 {
        ranges.push((p.edx as usize + 0x18, 8));
    }
    r.eax = checked_writer(SET_CALLBACK, &p, 0, &ranges, || unsafe { set_callback(p.eax, p.edx) });
}
regs_thunk!(set_callback_thunk, set_callback_entry);

// --- the readers (ECX = &value) -------------------------------------------------------------------

/// `fldz; fcomp dword [p]; fnstsw ax`, exactly as the original.
unsafe fn fcomp_zero(p: usize) -> u16 {
    let sw: u16;
    unsafe {
        std::arch::asm!("fldz", "fcomp dword ptr [{p}]", "fnstsw ax", p = in(reg) p, out("ax") sw, clobber_abi("C"));
    }
    sw
}

/// 0x773120. Index = tag - 1: tags 1, 2 give payload != 0; tag 3 `fldz / fcomp / test ah, 0x44 /
/// jnp` (0 only when equal to 0.0 and ordered, so a NaN is true); tag 5 the payload byte; every
/// other tag has AL = 0 with the rest of EAX = tag - 1 (so tag 0 gives 0xFFFFFF00).
unsafe fn to_bool(v: u32) -> u32 {
    unsafe {
        let a = v as usize;
        let idx = rd(a).wrapping_sub(1);
        match idx {
            0 | 1 => (rd(a + 4) != 0) as u32,
            2 => {
                let ah = (fcomp_zero(a + 4) >> 8) & 0x44;
                (ah.count_ones() % 2 == 0) as u32
            }
            4 => rb(a + 4) as u32,
            _ => idx & !0xFF,
        }
    }
}
lookup_entry!(to_bool_entry, to_bool_thunk, TO_BOOL, ecx, to_bool);

/// `fld dword [p]` then the CRT's `_ftol2_sse` (the original, by address).
unsafe fn ftol_dword(p: usize) -> u32 {
    let r: u32;
    unsafe {
        std::arch::asm!("fld dword ptr [{p}]", "call {f}", p = in(reg) p, f = in(reg) FTOL2_SSE, out("eax") r, clobber_abi("C"));
    }
    r
}

/// 0x5E4930. Tags 1, 2: the payload dword; 3: the float truncated by `_ftol2_sse`; 5: a true
/// bool gives -1 (`neg al / sbb eax, eax`), false 0; every other tag 0.
unsafe fn to_int(v: u32) -> u32 {
    unsafe {
        let a = v as usize;
        match rd(a).wrapping_sub(1) {
            0 | 1 => rd(a + 4),
            2 => ftol_dword(a + 4),
            4 => {
                if rb(a + 4) != 0 {
                    u32::MAX
                } else {
                    0
                }
            }
            _ => 0,
        }
    }
}
lookup_entry!(to_int_entry, to_int_thunk, TO_INT, ecx, to_int);

// --- the stream readers (stream = {base, size, cur}) ----------------------------------------------

/// Fails (AL = 0, EAX = cur) when `cur + 4 >= base + size` (unsigned, strict: the last four bytes
/// can never be read). Otherwise a little-endian dword to `*out`, cur += 4, EAX = out with AL =
/// 1. `early`: the `_b` / `_c` copies store cur + 4 before the out write; the first copy adds 4
/// to the live cursor after it (the two differ only if out aliases the cursor).
unsafe fn stream_u32(s: u32, out: u32, early: bool) -> u32 {
    unsafe {
        let a = s as usize;
        let end = rd(a + 4).wrapping_add(rd(a));
        let cur = rd(a + 8);
        let next = cur.wrapping_add(4);
        if next >= end {
            return cur & !0xFF;
        }
        let v = u32::from_le_bytes([rb(cur as usize), rb(cur as usize + 1), rb(cur as usize + 2), rb(cur as usize + 3)]);
        if early {
            wd(a + 8, next);
            wd(out as usize, v);
        } else {
            wd(out as usize, v);
            wd(a + 8, rd(a + 8).wrapping_add(4));
        }
        lo(out, true)
    }
}

/// The writes of `stream_u32` / `stream_u8`: nothing on a failed read.
unsafe fn stream_ranges(s: u32, out: u32, n: usize, width: u32) -> Vec<(usize, usize)> {
    unsafe {
        let a = s as usize;
        let end = rd(a + 4).wrapping_add(rd(a));
        if rd(a + 8).wrapping_add(width) >= end {
            Vec::new()
        } else {
            vec![(a + 8, 4), (out as usize, n)]
        }
    }
}

macro_rules! stream_u32_entry {
    ($entry:ident, $thunk:ident, $id:ident, $early:literal) => {
        extern "C" fn $entry(r: &mut Pushad, args: *const u32) {
            let p = *r;
            let out = unsafe { *args };
            let ranges = unsafe { stream_ranges(p.ecx, out, 4, 4) };
            r.eax = checked_writer($id, &p, out, &ranges, || unsafe { stream_u32(p.ecx, out, $early) });
        }
        regs_thunk!($thunk, $entry, "ret 4");
    };
}
// 0xB8CCC0 (cursor added after), 0xB8CDA0 and 0xB99060 (cursor stored first); ECX = stream, stdcall (u32* out)
stream_u32_entry!(stream_u32_entry_a, stream_u32_thunk_a, STREAM_U32, false);
stream_u32_entry!(stream_u32_entry_b, stream_u32_thunk_b, STREAM_U32_B, true);
stream_u32_entry!(stream_u32_entry_c, stream_u32_thunk_c, STREAM_U32_C, true);

/// 0xB8CDF0 (EAX = stream, stdcall (u8* out)): fails (AL = 0, EAX = stream) when `cur + 1 >=
/// base + size`; else one byte to `*out`, cur += 1, EAX = stream with AL = 1.
unsafe fn stream_u8(s: u32, out: u32) -> u32 {
    unsafe {
        let a = s as usize;
        let end = rd(a + 4).wrapping_add(rd(a));
        let cur = rd(a + 8);
        let next = cur.wrapping_add(1);
        if next >= end {
            return lo(s, false);
        }
        wb(out as usize, rb(cur as usize));
        wd(a + 8, next);
        lo(s, true)
    }
}

extern "C" fn stream_u8_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let out = unsafe { *args };
    let ranges = unsafe { stream_ranges(p.eax, out, 1, 1) };
    r.eax = checked_writer(STREAM_U8, &p, out, &ranges, || unsafe { stream_u8(p.eax, out) });
}
regs_thunk!(stream_u8_thunk, stream_u8_entry, "ret 4");

// --- install ------------------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let list: [(usize, &[u8], &[usize], extern "C" fn()); COUNT] = [
        (VALUE_EQUALS, &VALUE_EQUALS_BODY, &VALUE_EQUALS_BODY_CALLS, value_equals_thunk),
        (VALUE_INIT, &VALUE_INIT_BODY, &VALUE_INIT_BODY_CALLS, value_init_thunk),
        (BITMAP_OF_VALUE, &BITMAP_OF_VALUE_BODY, &BITMAP_OF_VALUE_BODY_CALLS, bitmap_of_value_thunk),
        (BITMAP_BY_HANDLE, &BITMAP_BY_HANDLE_BODY, &BITMAP_BY_HANDLE_BODY_CALLS, bitmap_by_handle_thunk),
        (FONT_BY_HANDLE, &FONT_BY_HANDLE_BODY, &FONT_BY_HANDLE_BODY_CALLS, font_by_handle_thunk),
        (SOUND_BY_HANDLE, &SOUND_BY_HANDLE_BODY, &SOUND_BY_HANDLE_BODY_CALLS, sound_by_handle_thunk),
        (DATAITEM_BY_HANDLE, &DATAITEM_BY_HANDLE_BODY, &DATAITEM_BY_HANDLE_BODY_CALLS, dataitem_by_handle_thunk),
        (DOC_FIND, &DOC_FIND_BODY, &DOC_FIND_BODY_CALLS, doc_find_thunk),
        (COLOR_BYTES, &COLOR_BYTES_BODY, &COLOR_BYTES_BODY_CALLS, color_bytes_thunk),
        (FONT_OF_VALUE, &FONT_OF_VALUE_BODY, &FONT_OF_VALUE_BODY_CALLS, font_of_value_thunk),
        (LUA_CALL_BY_HANDLE, &LUA_CALL_BY_HANDLE_BODY, &LUA_CALL_BY_HANDLE_BODY_CALLS, lua_call_by_handle_thunk),
        (LUA_CALL_BY_NAME, &LUA_CALL_BY_NAME_BODY, &LUA_CALL_BY_NAME_BODY_CALLS, lua_call_by_name_thunk),
        (DATAITEM_BY_NAME, &DATAITEM_BY_NAME_BODY, &DATAITEM_BY_NAME_BODY_CALLS, dataitem_by_name_thunk),
        (FONT_BY_PLATFORM, &FONT_BY_PLATFORM_BODY, &FONT_BY_PLATFORM_BODY_CALLS, font_by_platform_thunk),
        (SET_BOOL, &SET_BOOL_BODY, &SET_BOOL_BODY_CALLS, set_bool_thunk),
        (SET_COLOR_BYTES, &SET_COLOR_BYTES_BODY, &SET_COLOR_BYTES_BODY_CALLS, set_color_bytes_thunk),
        (SET_CALLBACK, &SET_CALLBACK_BODY, &SET_CALLBACK_BODY_CALLS, set_callback_thunk),
        (TO_BOOL, &TO_BOOL_BODY, &TO_BOOL_BODY_CALLS, to_bool_thunk),
        (TO_INT, &TO_INT_BODY, &TO_INT_BODY_CALLS, to_int_thunk),
        (STREAM_U32, &STREAM_U32_BODY, &STREAM_U32_BODY_CALLS, stream_u32_thunk_a),
        (STREAM_U32_B, &STREAM_U32_B_BODY, &STREAM_U32_B_BODY_CALLS, stream_u32_thunk_b),
        (STREAM_U32_C, &STREAM_U32_C_BODY, &STREAM_U32_C_BODY_CALLS, stream_u32_thunk_c),
        (STREAM_U8, &STREAM_U8_BODY, &STREAM_U8_BODY_CALLS, stream_u8_thunk),
    ];
    let mut ok = 0;
    for (id, body, call_offsets, thunk) in list {
        let port = &PORTS[id];
        // The shadow copy first: replacing patches the entry.
        match unsafe { detour::relocate(port.addr, body, call_offsets) } {
            Ok(copy) => port.original.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_vint2: {}: no shadow copy: {e:?}", port.name)),
        }
        if replace_pinned(port.name, port.addr, body, thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_vint2: replaced {ok}/{COUNT} with Rust"));
}

/// One line for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .filter_map(|p| {
            let c = p.calls.load(Ordering::Relaxed);
            (c > 0).then(|| format!("{} {c} ({})", p.name, p.mismatches.load(Ordering::Relaxed)))
        })
        .collect();
    if parts.is_empty() { String::new() } else { format!("port_vint2: {}", parts.join(", ")) }
}

// --- original bodies (tools/body_bytes.py) ---------------------------------------------------------

const VALUE_EQUALS_BODY: [u8; 237] = [
    0x8B, 0x08, 0x3B, 0x0A, 0x0F, 0x85, 0xB1, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x0F, 0x84, 0xA9, 0x00,
    0x00, 0x00, 0x83, 0xC1, 0xFF, 0x83, 0xF9, 0x0A, 0x0F, 0x87, 0x9D, 0x00, 0x00, 0x00, 0xFF, 0x24,
    0x8D, 0xE0, 0x39, 0xB9, 0x00, 0x8B, 0x40, 0x04, 0x33, 0xC9, 0x3B, 0x42, 0x04, 0x0F, 0x94, 0xC1,
    0x8A, 0xC1, 0xC3, 0xD9, 0x40, 0x04, 0xD9, 0x42, 0x04, 0xDA, 0xE9, 0xDF, 0xE0, 0xF6, 0xC4, 0x44,
    0x7A, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3, 0x8B, 0x40, 0x04, 0x8B, 0x4A,
    0x04, 0x3B, 0xC1, 0x75, 0x03, 0xB0, 0x01, 0xC3, 0x85, 0xC0, 0x74, 0x5F, 0x85, 0xC9, 0x74, 0x5B,
    0x8A, 0x10, 0x3A, 0x11, 0x75, 0x22, 0x84, 0xD2, 0x74, 0x12, 0x8A, 0x50, 0x01, 0x3A, 0x51, 0x01,
    0x75, 0x16, 0x83, 0xC0, 0x02, 0x83, 0xC1, 0x02, 0x84, 0xD2, 0x75, 0xE4, 0x33, 0xC0, 0x33, 0xD2,
    0x85, 0xC0, 0x0F, 0x94, 0xC2, 0x8A, 0xC2, 0xC3, 0x1B, 0xC0, 0x83, 0xD8, 0xFF, 0x33, 0xD2, 0x85,
    0xC0, 0x0F, 0x94, 0xC2, 0x8A, 0xC2, 0xC3, 0x8A, 0x40, 0x04, 0x33, 0xC9, 0x3A, 0x42, 0x04, 0x0F,
    0x94, 0xC1, 0x8A, 0xC1, 0xC3, 0x8D, 0x48, 0x04, 0x83, 0xC2, 0x04, 0xE9, 0x20, 0xF3, 0xFF, 0xFF,
    0x8D, 0x48, 0x04, 0x83, 0xC2, 0x04, 0xE9, 0xE5, 0xF2, 0xFF, 0xFF, 0x32, 0xC0, 0xC3, 0x8B, 0xFF,
    0x45, 0x39, 0xB9, 0x00, 0x45, 0x39, 0xB9, 0x00, 0x53, 0x39, 0xB9, 0x00, 0x6B, 0x39, 0xB9, 0x00,
    0xB7, 0x39, 0xB9, 0x00, 0xC5, 0x39, 0xB9, 0x00, 0xD0, 0x39, 0xB9, 0x00, 0x45, 0x39, 0xB9, 0x00,
    0x45, 0x39, 0xB9, 0x00, 0x45, 0x39, 0xB9, 0x00, 0x45, 0x39, 0xB9, 0x00, 0xCC,
];
const VALUE_EQUALS_BODY_CALLS: [usize; 0] = [];
const VALUE_INIT_BODY: [u8; 105] = [
    0x33, 0xD2, 0x39, 0x10, 0x75, 0x33, 0x89, 0x08, 0x83, 0xC1, 0xFF, 0x83, 0xF9, 0x0A, 0x77, 0x29,
    0xFF, 0x24, 0x8D, 0x4C, 0x3A, 0xB9, 0x00, 0xD9, 0xEE, 0xD9, 0x58, 0x04, 0xC3, 0x88, 0x50, 0x04,
    0xC3, 0xD9, 0xEE, 0xD9, 0x50, 0x04, 0xD9, 0x50, 0x08, 0xD9, 0x58, 0x0C, 0xC3, 0xD9, 0xEE, 0xD9,
    0x50, 0x04, 0xD9, 0x58, 0x08, 0xC3, 0x89, 0x50, 0x04, 0xC3, 0x8B, 0xFF, 0x46, 0x3A, 0xB9, 0x00,
    0x46, 0x3A, 0xB9, 0x00, 0x27, 0x3A, 0xB9, 0x00, 0x46, 0x3A, 0xB9, 0x00, 0x2D, 0x3A, 0xB9, 0x00,
    0x31, 0x3A, 0xB9, 0x00, 0x3D, 0x3A, 0xB9, 0x00, 0x46, 0x3A, 0xB9, 0x00, 0x46, 0x3A, 0xB9, 0x00,
    0x46, 0x3A, 0xB9, 0x00, 0x46, 0x3A, 0xB9, 0x00, 0xCC,
];
const VALUE_INIT_BODY_CALLS: [usize; 0] = [];
const BITMAP_OF_VALUE_BODY: [u8; 49] = [
    0x83, 0x38, 0x09, 0x74, 0x03, 0x33, 0xC0, 0xC3, 0x8B, 0x48, 0x04, 0x85, 0xC9, 0x74, 0xF6, 0x0F,
    0xB7, 0xC1, 0x3D, 0xF0, 0x00, 0x00, 0x00, 0x73, 0xEC, 0x8D, 0x04, 0xC0, 0x8D, 0x04, 0x85, 0x18,
    0xC5, 0x1B, 0x03, 0x33, 0xD2, 0x39, 0x48, 0x14, 0x0F, 0x95, 0xC2, 0x83, 0xEA, 0x01, 0x23, 0xC2,
    0xC3,
];
const BITMAP_OF_VALUE_BODY_CALLS: [usize; 0] = [];
const BITMAP_BY_HANDLE_BODY: [u8; 41] = [
    0x85, 0xC9, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x0F, 0xB7, 0xC1, 0x3D, 0xF0, 0x00, 0x00, 0x00, 0x73,
    0xF3, 0x8D, 0x04, 0xC0, 0x8D, 0x04, 0x85, 0x18, 0xC5, 0x1B, 0x03, 0x33, 0xD2, 0x39, 0x48, 0x14,
    0x0F, 0x95, 0xC2, 0x83, 0xEA, 0x01, 0x23, 0xC2, 0xC3,
];
const BITMAP_BY_HANDLE_BODY_CALLS: [usize; 0] = [];
const FONT_BY_HANDLE_BODY: [u8; 46] = [
    0x85, 0xC9, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x66, 0x83, 0xF9, 0x0A, 0x73, 0xF7, 0x0F, 0xB7, 0xC1,
    0x8D, 0x14, 0xC5, 0x00, 0x00, 0x00, 0x00, 0x2B, 0xD0, 0x8D, 0x04, 0x95, 0x00, 0xC4, 0x1B, 0x03,
    0x33, 0xD2, 0x39, 0x48, 0x08, 0x0F, 0x95, 0xC2, 0x83, 0xEA, 0x01, 0x23, 0xC2, 0xC3,
];
const FONT_BY_HANDLE_BODY_CALLS: [usize; 0] = [];
const SOUND_BY_HANDLE_BODY: [u8; 39] = [
    0x85, 0xC9, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x0F, 0xB7, 0xC1, 0x83, 0xF8, 0x32, 0x73, 0xF5, 0x8D,
    0x04, 0x40, 0x8D, 0x04, 0xC5, 0x00, 0xE7, 0x1B, 0x03, 0x33, 0xD2, 0x39, 0x48, 0x08, 0x0F, 0x95,
    0xC2, 0x83, 0xEA, 0x01, 0x23, 0xC2, 0xC3,
];
const SOUND_BY_HANDLE_BODY_CALLS: [usize; 0] = [];
const DATAITEM_BY_HANDLE_BODY: [u8; 45] = [
    0x85, 0xC9, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x0F, 0xB7, 0xC1, 0x3D, 0x80, 0x00, 0x00, 0x00, 0x73,
    0xF3, 0x69, 0xC0, 0x2C, 0x02, 0x00, 0x00, 0x05, 0xF0, 0x96, 0x1A, 0x03, 0x33, 0xD2, 0x39, 0x88,
    0x24, 0x02, 0x00, 0x00, 0x0F, 0x95, 0xC2, 0x83, 0xEA, 0x01, 0x23, 0xC2, 0xC3,
];
const DATAITEM_BY_HANDLE_BODY_CALLS: [usize; 0] = [];
const DOC_FIND_BODY: [u8; 41] = [
    0x85, 0xC9, 0x74, 0x22, 0xE8, 0xB7, 0x15, 0x05, 0x00, 0x8B, 0x0D, 0xAC, 0xA1, 0x52, 0x02, 0x85,
    0xC9, 0x8B, 0xD0, 0x8B, 0xC1, 0x74, 0x0F, 0x39, 0x50, 0x48, 0x74, 0x0C, 0x8B, 0x00, 0x3B, 0xC1,
    0x74, 0x04, 0x85, 0xC0, 0x75, 0xF1, 0x33, 0xC0, 0xC3,
];
const DOC_FIND_BODY_CALLS: [usize; 1] = [0x4];
const COLOR_BYTES_BODY: [u8; 168] = [
    0x32, 0xC0, 0x83, 0xEC, 0x08, 0x83, 0x3A, 0x06, 0x88, 0x01, 0x88, 0x41, 0x01, 0x88, 0x41, 0x02,
    0x88, 0x41, 0x03, 0x0F, 0x85, 0x89, 0x00, 0x00, 0x00, 0xD9, 0x42, 0x04, 0xC6, 0x41, 0x03, 0xFF,
    0xDD, 0x05, 0x38, 0xC0, 0xE5, 0x00, 0xD9, 0x7C, 0x24, 0x02, 0x0F, 0xB7, 0x44, 0x24, 0x02, 0xDC,
    0xC9, 0x0D, 0x00, 0x0C, 0x00, 0x00, 0xD9, 0xC9, 0x89, 0x44, 0x24, 0x04, 0xD9, 0x6C, 0x24, 0x04,
    0xDB, 0x5C, 0x24, 0x04, 0x0F, 0xB6, 0x44, 0x24, 0x04, 0x88, 0x01, 0xD9, 0x6C, 0x24, 0x02, 0xD9,
    0x42, 0x08, 0xD9, 0x7C, 0x24, 0x02, 0xD8, 0xC9, 0x0F, 0xB7, 0x44, 0x24, 0x02, 0x0D, 0x00, 0x0C,
    0x00, 0x00, 0x89, 0x44, 0x24, 0x04, 0xD9, 0x6C, 0x24, 0x04, 0xDB, 0x5C, 0x24, 0x04, 0x0F, 0xB6,
    0x44, 0x24, 0x04, 0x88, 0x41, 0x01, 0xD9, 0x6C, 0x24, 0x02, 0xD8, 0x4A, 0x0C, 0xD9, 0x7C, 0x24,
    0x02, 0x0F, 0xB7, 0x44, 0x24, 0x02, 0x0D, 0x00, 0x0C, 0x00, 0x00, 0x89, 0x44, 0x24, 0x04, 0xD9,
    0x6C, 0x24, 0x04, 0xDB, 0x5C, 0x24, 0x04, 0x8A, 0x54, 0x24, 0x04, 0x88, 0x51, 0x02, 0xD9, 0x6C,
    0x24, 0x02, 0x8B, 0xC1, 0x83, 0xC4, 0x08, 0xC3,
];
const COLOR_BYTES_BODY_CALLS: [usize; 0] = [];
const FONT_OF_VALUE_BODY: [u8; 56] = [
    0x83, 0x38, 0x0A, 0x74, 0x03, 0x33, 0xC0, 0xC3, 0x8B, 0x40, 0x04, 0x85, 0xC0, 0x74, 0xF6, 0x66,
    0x3D, 0x0A, 0x00, 0x73, 0xF0, 0x0F, 0xB7, 0xC8, 0x8D, 0x14, 0xCD, 0x00, 0x00, 0x00, 0x00, 0x2B,
    0xD1, 0x8D, 0x0C, 0x95, 0x00, 0xC4, 0x1B, 0x03, 0x33, 0xD2, 0x39, 0x41, 0x08, 0x0F, 0x95, 0xC2,
    0x83, 0xEA, 0x01, 0x23, 0xD1, 0x8B, 0xC2, 0xC3,
];
const FONT_OF_VALUE_BODY_CALLS: [usize; 0] = [];
const LUA_CALL_BY_HANDLE_BODY: [u8; 43] = [
    0x85, 0xC9, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x0F, 0xB7, 0xC1, 0x83, 0xF8, 0x20, 0x73, 0xF5, 0x69,
    0xC0, 0x60, 0x02, 0x00, 0x00, 0x05, 0x80, 0x4A, 0x1A, 0x03, 0x33, 0xD2, 0x39, 0x88, 0x10, 0x02,
    0x00, 0x00, 0x0F, 0x95, 0xC2, 0x83, 0xEA, 0x01, 0x23, 0xC2, 0xC3,
];
const LUA_CALL_BY_HANDLE_BODY_CALLS: [usize; 0] = [];
const LUA_CALL_BY_NAME_BODY: [u8; 53] = [
    0x85, 0xC9, 0x74, 0x2B, 0xE8, 0x87, 0xAD, 0x04, 0x00, 0x85, 0xC0, 0x74, 0x22, 0x8B, 0x15, 0xC8,
    0xA1, 0x52, 0x02, 0x8B, 0xCA, 0x85, 0xC9, 0x74, 0x16, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x41, 0x08, 0x74, 0x0D, 0x8B, 0x09, 0x3B, 0xCA, 0x74, 0x04, 0x85, 0xC9, 0x75, 0xF1, 0x33,
    0xC0, 0xC3, 0x8B, 0xC1, 0xC3,
];
const LUA_CALL_BY_NAME_BODY_CALLS: [usize; 1] = [0x4];
const DATAITEM_BY_NAME_BODY: [u8; 54] = [
    0x85, 0xC9, 0x74, 0x2C, 0xE8, 0xC7, 0x77, 0x04, 0x00, 0x85, 0xC0, 0x74, 0x23, 0x8B, 0x15, 0xF4,
    0xA1, 0x52, 0x02, 0x8B, 0xCA, 0x85, 0xC9, 0x74, 0x17, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x41, 0x10, 0x74, 0x0E, 0x8B, 0x49, 0x0C, 0x3B, 0xCA, 0x74, 0x04, 0x85, 0xC9, 0x75, 0xF0,
    0x33, 0xC0, 0xC3, 0x8B, 0xC1, 0xC3,
];
const DATAITEM_BY_NAME_BODY_CALLS: [usize; 1] = [0x4];
const FONT_BY_PLATFORM_BODY: [u8; 35] = [
    0x33, 0xC0, 0x85, 0xD2, 0x74, 0x1C, 0xB9, 0x00, 0xC4, 0x1B, 0x03, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x39, 0x11, 0x74, 0x0C, 0x83, 0xC1, 0x1C, 0x81, 0xF9, 0x18, 0xC5, 0x1B, 0x03, 0x7C, 0xF1, 0xC3,
    0x8B, 0xC1, 0xC3,
];
const FONT_BY_PLATFORM_BODY_CALLS: [usize; 0] = [];
const SET_BOOL_BODY: [u8; 30] = [
    0x8B, 0x08, 0x85, 0xC9, 0x74, 0x08, 0x83, 0xF9, 0x05, 0x74, 0x09, 0xC2, 0x04, 0x00, 0xC7, 0x00,
    0x05, 0x00, 0x00, 0x00, 0x8A, 0x4C, 0x24, 0x04, 0x88, 0x48, 0x04, 0xC2, 0x04, 0x00,
];
const SET_BOOL_BODY_CALLS: [usize; 0] = [];
const SET_COLOR_BYTES_BODY: [u8; 91] = [
    0x8B, 0x08, 0x85, 0xC9, 0x74, 0x08, 0x83, 0xF9, 0x06, 0x74, 0x09, 0xC2, 0x04, 0x00, 0xC7, 0x00,
    0x06, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x04, 0x0F, 0xB6, 0xD1, 0x89, 0x54, 0x24, 0x04, 0x8B,
    0xD1, 0xC1, 0xEA, 0x08, 0xDB, 0x44, 0x24, 0x04, 0x0F, 0xB6, 0xD2, 0xDD, 0x05, 0x38, 0xC0, 0xE5,
    0x00, 0xDC, 0xF9, 0x89, 0x54, 0x24, 0x04, 0xC1, 0xE9, 0x10, 0x0F, 0xB6, 0xC9, 0xD9, 0xC9, 0xD9,
    0x58, 0x04, 0xDB, 0x44, 0x24, 0x04, 0x89, 0x4C, 0x24, 0x04, 0xD8, 0xF1, 0xD9, 0x58, 0x08, 0xDB,
    0x44, 0x24, 0x04, 0xDE, 0xF1, 0xD9, 0x58, 0x0C, 0xC2, 0x04, 0x00,
];
const SET_COLOR_BYTES_BODY_CALLS: [usize; 0] = [];
const SET_CALLBACK_BODY: [u8; 40] = [
    0x8B, 0x08, 0x85, 0xC9, 0x74, 0x05, 0x83, 0xF9, 0x08, 0x75, 0x1C, 0x85, 0xD2, 0xC7, 0x00, 0x08,
    0x00, 0x00, 0x00, 0x74, 0x0B, 0x8B, 0x4A, 0x18, 0x83, 0x42, 0x1C, 0x01, 0x89, 0x48, 0x04, 0xC3,
    0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const SET_CALLBACK_BODY_CALLS: [usize; 0] = [];
const TO_BOOL_BODY: [u8; 76] = [
    0x8B, 0x01, 0x83, 0xC0, 0xFF, 0x83, 0xF8, 0x04, 0x77, 0x29, 0xFF, 0x24, 0x85, 0x58, 0x31, 0x77,
    0x00, 0xD9, 0xEE, 0xD8, 0x59, 0x04, 0xDF, 0xE0, 0xF6, 0xC4, 0x44, 0x7B, 0x06, 0xB8, 0x01, 0x00,
    0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3, 0x33, 0xC0, 0x39, 0x41, 0x04, 0x0F, 0x95, 0xC0, 0xC3, 0x8A,
    0x41, 0x04, 0xC3, 0x32, 0xC0, 0xC3, 0x8B, 0xFF, 0x46, 0x31, 0x77, 0x00, 0x46, 0x31, 0x77, 0x00,
    0x31, 0x31, 0x77, 0x00, 0x53, 0x31, 0x77, 0x00, 0x4F, 0x31, 0x77, 0x00,
];
const TO_BOOL_BODY_CALLS: [usize; 0] = [];
const TO_INT_BODY: [u8; 61] = [
    0x8B, 0x01, 0x83, 0xC0, 0xFF, 0x83, 0xF8, 0x04, 0x77, 0x1B, 0xFF, 0x24, 0x85, 0x58, 0x49, 0x5E,
    0x00, 0xD9, 0x41, 0x04, 0xE9, 0xF7, 0xB7, 0x7B, 0x00, 0x8B, 0x41, 0x04, 0xC3, 0x8A, 0x41, 0x04,
    0xF6, 0xD8, 0x1B, 0xC0, 0xC3, 0x33, 0xC0, 0xC3, 0x49, 0x49, 0x5E, 0x00, 0x49, 0x49, 0x5E, 0x00,
    0x41, 0x49, 0x5E, 0x00, 0x55, 0x49, 0x5E, 0x00, 0x4D, 0x49, 0x5E, 0x00, 0xCC,
];
const TO_INT_BODY_CALLS: [usize; 0] = [];
const STREAM_U32_BODY: [u8; 63] = [
    0x8B, 0x51, 0x04, 0x8B, 0x41, 0x08, 0x03, 0x11, 0x56, 0x8D, 0x70, 0x04, 0x3B, 0xF2, 0x72, 0x06,
    0x32, 0xC0, 0x5E, 0xC2, 0x04, 0x00, 0x0F, 0xB6, 0x70, 0x01, 0x33, 0xD2, 0x8A, 0x70, 0x03, 0x8A,
    0x50, 0x02, 0x0F, 0xB6, 0x00, 0xC1, 0xE2, 0x08, 0x0B, 0xD6, 0xC1, 0xE2, 0x08, 0x0B, 0xD0, 0x8B,
    0x44, 0x24, 0x08, 0x89, 0x10, 0x83, 0x41, 0x08, 0x04, 0xB0, 0x01, 0x5E, 0xC2, 0x04, 0x00,
];
const STREAM_U32_BODY_CALLS: [usize; 0] = [];
const STREAM_U32_B_BODY: [u8; 65] = [
    0x8B, 0x51, 0x04, 0x8B, 0x41, 0x08, 0x03, 0x11, 0x56, 0x8D, 0x70, 0x04, 0x3B, 0xF2, 0x57, 0x72,
    0x07, 0x32, 0xC0, 0x5F, 0x5E, 0xC2, 0x04, 0x00, 0x0F, 0xB6, 0x78, 0x01, 0x33, 0xD2, 0x8A, 0x70,
    0x03, 0x8A, 0x50, 0x02, 0x0F, 0xB6, 0x00, 0x89, 0x71, 0x08, 0xC1, 0xE2, 0x08, 0x0B, 0xD7, 0xC1,
    0xE2, 0x08, 0x0B, 0xD0, 0x8B, 0x44, 0x24, 0x0C, 0x5F, 0x89, 0x10, 0xB0, 0x01, 0x5E, 0xC2, 0x04,
    0x00,
];
const STREAM_U32_B_BODY_CALLS: [usize; 0] = [];
const STREAM_U32_C_BODY: [u8; 65] = [
    0x8B, 0x51, 0x04, 0x8B, 0x41, 0x08, 0x03, 0x11, 0x56, 0x8D, 0x70, 0x04, 0x3B, 0xF2, 0x57, 0x72,
    0x07, 0x32, 0xC0, 0x5F, 0x5E, 0xC2, 0x04, 0x00, 0x0F, 0xB6, 0x78, 0x01, 0x33, 0xD2, 0x8A, 0x70,
    0x03, 0x8A, 0x50, 0x02, 0x0F, 0xB6, 0x00, 0x89, 0x71, 0x08, 0xC1, 0xE2, 0x08, 0x0B, 0xD7, 0xC1,
    0xE2, 0x08, 0x0B, 0xD0, 0x8B, 0x44, 0x24, 0x0C, 0x5F, 0x89, 0x10, 0xB0, 0x01, 0x5E, 0xC2, 0x04,
    0x00,
];
const STREAM_U32_C_BODY_CALLS: [usize; 0] = [];
const STREAM_U8_BODY: [u8; 39] = [
    0x8B, 0x48, 0x08, 0x56, 0x8B, 0x70, 0x04, 0x03, 0x30, 0x8D, 0x51, 0x01, 0x3B, 0xD6, 0x72, 0x06,
    0x32, 0xC0, 0x5E, 0xC2, 0x04, 0x00, 0x8A, 0x09, 0x8B, 0x74, 0x24, 0x08, 0x88, 0x0E, 0x89, 0x50,
    0x08, 0xB0, 0x01, 0x5E, 0xC2, 0x04, 0x00,
];
const STREAM_U8_BODY_CALLS: [usize; 0] = [];
