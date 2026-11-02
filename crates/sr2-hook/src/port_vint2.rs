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
//!
//! Round 2 (ports 23 and up, from the "Round 2" banner on): the value setters / assign / free,
//! the resource release helpers, the dataitem / datagroup constructors, the three resource
//! caches, `vint_insert_values_expand`, the object name / document propagation, the raw input
//! index, the project finder, the clip pop and the datagroup delivery / destroy / unsubscribe.
//! Their shadow checks are described at the banner: snapshot shadows where the branch has no
//! side-effecting callee, per-branch call counters (`[branches a/b/c/d]` in the summary) where it
//! does.

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
const COUNT: usize = 47;

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
    port("vint_value_set_int", 0x00B9_3590, SAVED),
    port("vint_value_set_handle", 0x00B9_3490, SAVED),
    port("vint_value_set_name", 0x00B9_3800, SAVED),
    port("vint_value_assign", 0x00B9_3620, SAVED),
    port("vint_value_free", 0x00B9_3790, SAVED),
    port("vint_bitmap_release_field", 0x00B9_2D60, SAVED),
    port("vint_font_release_field", 0x00B9_2DD0, SAVED),
    port("vint_sound_release_handle", 0x00B9_2E50, SAVED),
    port("vint_callback_release", 0x00BA_0850, SAVED),
    port("vint_dataitem_get_or_create", 0x00B9_4FE0, SAVED),
    port("vint_datagroup_create", 0x00B9_4140, SAVED),
    port("vint_bitmap_acquire_by_name", 0x00B9_BD20, SAVED),
    port("vint_font_acquire_by_name", 0x00B9_B6F0, SAVED),
    port("vint_sound_acquire_by_name", 0x00BA_03F0, SAVED),
    port("vint_insert_values_expand", 0x00B9_26D0, SAVED),
    port("vint_object_rehash_name", 0x00B9_7570, EAX | SAVED),
    port("vint_object_set_name", 0x00B9_7620, SAVED),
    port("vint_object_set_document_rec", 0x00B9_79B0, EAX | SAVED),
    port("vint_raw_input_index", 0x00B9_9FE0, SAVED),
    port("vint_project_find_document", 0x00B9_AF50, SAVED),
    port("vint_document_clip_pop", 0x00B8_B990, SAVED),
    port("vint_datagroups_deliver_all", 0x00B9_4560, SAVED),
    port("vint_datagroup_destroy", 0x00B9_3BC0, SAVED),
    port("vint_datagroup_unsubscribe", 0x00B9_3EE0, SAVED),
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

// =====================================================================================================
// Round 2: the value setters / assign / free, the resource release helpers, the dataitem / datagroup
// constructors and the three resource caches.
//
// Checks used here: every port takes a snapshot shadow (run the original, save, restore, run Rust,
// compare) on the branches that call nothing with side effects. The branches that call
// release / addref / platform hooks (or relink many rings) take per-branch call counters only,
// except the dataitem / datagroup constructors, which snapshot the pools, globals and waiting nodes.
// =====================================================================================================

const CALLBACK_FIND: usize = 0x00BA_0980; // EDX = handle -> EAX = callback or 0
const CALLBACK_RELEASE_FN: usize = 0x00BA_0850; // ECX = callback
const BITMAP_ADDREF: usize = 0x00B9_2D30; // ECX = handle
const FONT_ADDREF: usize = 0x00B9_2DA0; // ECX = handle
const SOUND_ADDREF: usize = 0x00B9_2E20; // ECX = handle
const BITMAP_RELEASE_FIELD: usize = 0x00B9_2D60; // ESI = &handle field
const FONT_RELEASE_FIELD: usize = 0x00B9_2DD0; // EDI = &handle field
const SOUND_RELEASE_HANDLE: usize = 0x00B9_2E50; // ECX = handle
const FONT_RECORD_RELEASE: usize = 0x00B9_B8C0; // ESI = record
const SOUND_RECORD_RELEASE: usize = 0x00BA_0570; // ESI = record
const COLOR_BY_NAME: usize = 0x00B9_2130; // EAX = name -> EAX = float[3]* or 0
const BITMAP_ACQUIRE_FN: usize = 0x00B9_BD20; // cdecl (name)
const FONT_ACQUIRE_FN: usize = 0x00B9_B6F0; // cdecl (name)
const SOUND_ACQUIRE_FN: usize = 0x00BA_03F0; // cdecl (name)
const HASH_LOWER: usize = 0x00BD_CB30; // ECX = name, EAX = seed, stdcall (u32* out)
const BITMAP_FIND_CACHED: usize = 0x00B9_BBC0; // EDI = &hash -> EAX = record or 0
const DATAITEM_INIT: usize = 0x00B9_4910; // ESI = item, ECX = name
const DATAGROUP_INIT: usize = 0x00B9_3B00; // ESI = group, ECX = name
const PLATFORM: usize = 0x0252_A1D4; // platform hook table pointer

const P_SET_INT: usize = 23;
const P_SET_HANDLE: usize = 24;
const P_SET_NAME: usize = 25;
const P_VALUE_ASSIGN: usize = 26;
const P_VALUE_FREE: usize = 27;
const P_BITMAP_RELEASE: usize = 28;
const P_FONT_RELEASE: usize = 29;
const P_SOUND_RELEASE: usize = 30;
const P_CALLBACK_RELEASE: usize = 31;
const P_ITEM_GET_OR_CREATE: usize = 32;
const P_GROUP_CREATE: usize = 33;
const P_BITMAP_ACQUIRE: usize = 34;
const P_FONT_ACQUIRE: usize = 35;
const P_SOUND_ACQUIRE: usize = 36;

unsafe fn rw(p: usize) -> u16 {
    unsafe { std::ptr::read_unaligned(p as *const u16) }
}
unsafe fn ww(p: usize, v: u16) {
    unsafe { std::ptr::write_unaligned(p as *mut u16, v) }
}

// --- branch counters (the ports with side-effecting branches) ----------------------------------------

static BR: [AtomicU32; COUNT * 4] = [const { AtomicU32::new(0) }; COUNT * 4];

/// Count a branch that cannot be shadowed (its callees have side effects).
fn br(id: usize, k: usize) {
    BR[id * 4 + k].fetch_add(1, Ordering::Relaxed);
}

/// True while this port's shadow window is open (logs once when it closes).
fn window_open(id: usize) -> bool {
    let p = &PORTS[id];
    if p.original.load(Ordering::Relaxed) == 0 {
        return false;
    }
    let n = p.calls.load(Ordering::Relaxed);
    if n < SHADOW_CALLS {
        return true;
    }
    if n == SHADOW_CALLS {
        shadowing(id);
    }
    false
}

// --- the two-argument shadow call ------------------------------------------------------------------

#[repr(C)]
#[allow(dead_code)]
struct Block2 {
    f: u32,
    r: Regs,
    arg: u32,
    esp: u32,
    arg2: u32,
}

/// `call_full` with two stack arguments (a `ret 8` callee pops both): `arg` is pushed last (at
/// [esp]), `arg2` above it. Captures all seven registers into `r`.
unsafe fn call_full2(f: usize, r: &mut Regs, arg: u32, arg2: u32) {
    let mut b = Block2 { f: f as u32, r: *r, arg, esp: 0, arg2 };
    unsafe {
        std::arch::asm!(
            "push ebp", "push ebx", "push esi", "push edi",
            "mov [eax + 36], esp",
            "push dword ptr [eax]",
            "push eax",
            "push eax",
            "push eax",
            "push dword ptr [eax + 40]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 20]",
            "push ebx",
            "mov ebx, [esp + 12]",
            "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
            "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 36]",
            "pop edi", "pop esi", "pop ebx", "pop ebp",
            inout("eax") &mut b as *mut Block2 => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

fn run_original2(id: usize, orig: usize, p: &Pushad, a1: u32, a2: u32) -> Regs {
    let inp = Regs::of(p);
    let mut r = inp;
    unsafe { call_full2(orig, &mut r, a1, a2) };
    check_preserved(id, &inp, &r);
    r
}

/// `checked_writer` for the two-argument stdcall ports.
fn checked_writer2(id: usize, p: &Pushad, a1: u32, a2: u32, ranges: &[(usize, usize)], ours: impl FnOnce() -> u32) -> u32 {
    let Some((n, orig)) = shadowing(id) else { return ours() };
    unsafe {
        let before = capture(ranges);
        let t = run_original2(id, orig, p, a1, a2);
        let theirs = capture(ranges);
        restore(ranges, &before);
        let e = ours();
        let mine = capture(ranges);
        if e != t.eax || mine != theirs {
            mismatch(
                id,
                n,
                format!("in {:x?} args {a1:#x} {a2:#x}: original eax {:#x} {theirs:x?}, Rust eax {e:#x} {mine:x?} (before {before:x?})", Regs::of(p), t.eax),
            );
            restore(ranges, &theirs);
            return t.eax;
        }
        e
    }
}

// --- 0xB93590 vint_value_set_int (ESI = &value, stdcall (int) -> EAX = &value) -----------------------

/// Tags 0 / 1: tag 1 and store; 2: store only; 3: `fild` (the int as a float); 8: rebind the
/// callback handle (release the old one by `callback_find` + `callback_release`, count the new
/// one, or zero the payload if it is unknown); every other tag untouched. EAX = &value.
unsafe fn set_int(v: u32, x: u32) -> u32 {
    unsafe {
        let a = v as usize;
        match rd(a) {
            0 | 1 => {
                wd(a, 1);
                wd(a + 4, x);
            }
            2 => wd(a + 4, x),
            3 => wf(a + 4, (x as i32 as f64) as f32),
            8 => {
                let cur = rd(a + 4);
                if cur != x {
                    let old = call_regs(CALLBACK_FIND, 0, 0, cur, v, x, &[]);
                    if old != 0 {
                        call_regs(CALLBACK_RELEASE_FN, old, old, cur, v, x, &[]);
                    }
                    let new = call_regs(CALLBACK_FIND, 0, 0, x, v, x, &[]);
                    if new != 0 {
                        wd(a + 4, x);
                        wd(new as usize + 0x1C, rd(new as usize + 0x1C).wrapping_add(1));
                    } else {
                        wd(a + 4, 0);
                    }
                }
            }
            _ => {}
        }
        v
    }
}

extern "C" fn set_int_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let x = unsafe { *args };
    let v = p.esi;
    r.eax = if window_open(P_SET_INT) {
        if unsafe { rd(v as usize) } == 8 {
            br(P_SET_INT, 0);
            unsafe { set_int(v, x) }
        } else {
            checked_writer(P_SET_INT, &p, x, &[(v as usize, 16)], || unsafe { set_int(v, x) })
        }
    } else {
        unsafe { set_int(v, x) }
    };
}
regs_thunk!(set_int_thunk, set_int_entry, "ret 4");

// --- 0xB93490 vint_value_set_handle (stdcall (&value, handle) -> EAX = &value) -----------------------

/// Tags 0 / 2: tag 2 and store; 1: store only; 3: the handle as an unsigned float (`fild` plus
/// 2^32 when negative); 9 / 10 / 11: look the record up first (unknown: zero the payload without
/// releasing the old one), release the old handle, count the new record and store the handle;
/// every other tag untouched.
unsafe fn set_handle(v: u32, h: u32, edi0: u32) -> u32 {
    unsafe {
        let a = v as usize;
        match rd(a) {
            0 | 2 => {
                wd(a + 4, h);
                wd(a, 2);
            }
            1 => wd(a + 4, h),
            3 => wf(a + 4, (h as f64) as f32),
            9 => {
                let rec = bitmap_by_handle(h);
                if rec == 0 {
                    wd(a + 4, 0);
                } else {
                    call_regs(BITMAP_RELEASE_FIELD, rec, h, 0, v + 4, rec, &[]);
                    wd(rec as usize + 0x18, rd(rec as usize + 0x18).wrapping_add(1));
                    wd(a + 4, h);
                }
            }
            10 => {
                let rec = font_by_handle(h);
                if rec == 0 {
                    wd(a + 4, 0);
                } else {
                    call_regs(FONT_RELEASE_FIELD, rec, h, 0, rec, v + 4, &[]);
                    wd(rec as usize + 0x10, rd(rec as usize + 0x10).wrapping_add(1));
                    wd(a + 4, h);
                }
            }
            11 => {
                let rec = sound_by_handle(h);
                if rec == 0 {
                    wd(a + 4, 0);
                } else {
                    call_regs(SOUND_RELEASE_HANDLE, rec, rd(a + 4), 0, rec, edi0, &[]);
                    wd(rec as usize + 0xC, rd(rec as usize + 0xC).wrapping_add(1));
                    wd(a + 4, h);
                }
            }
            _ => {}
        }
        v
    }
}

extern "C" fn set_handle_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (v, h) = unsafe { (*args, *args.add(1)) };
    r.eax = if window_open(P_SET_HANDLE) {
        let tag = unsafe { rd(v as usize) };
        if (9..=11).contains(&tag) {
            br(P_SET_HANDLE, (tag - 9) as usize);
            unsafe { set_handle(v, h, p.edi) }
        } else {
            checked_writer2(P_SET_HANDLE, &p, v, h, &[(v as usize, 16)], || unsafe { set_handle(v, h, p.edi) })
        }
    } else {
        unsafe { set_handle(v, h, p.edi) }
    };
}
regs_thunk!(set_handle_thunk, set_handle_entry, "ret 8");

// --- 0xB93800 vint_value_set_name (stdcall (&value, const char*) -> EAX = &value) --------------------

/// Tag 0: tag 4 and the pointer (not copied); 4: the pointer; 6: the colour by name (three
/// dwords copied, nothing when unknown); 9 / 10 / 11: release the old resource, acquire by name
/// and store its handle (0 when unknown; the sound's old handle is released but not cleared
/// first); every other tag untouched.
unsafe fn set_name(v: u32, s: u32, edi0: u32) -> u32 {
    unsafe {
        let a = v as usize;
        match rd(a) {
            0 => {
                wd(a, 4);
                wd(a + 4, s);
            }
            4 => wd(a + 4, s),
            6 => {
                let c = call_regs(COLOR_BY_NAME, s, 0, 0, 0, 0, &[]) as usize;
                if c != 0 {
                    wd(a + 4, rd(c));
                    wd(a + 8, rd(c + 4));
                    wd(a + 12, rd(c + 8));
                }
            }
            9 => {
                call_regs(BITMAP_RELEASE_FIELD, 9, 0, 0, v + 4, edi0, &[]);
                let rec = call_regs(BITMAP_ACQUIRE_FN, 0, 0, 0, v + 4, edi0, &[s]);
                wd(a + 4, if rec != 0 { rd(rec as usize + 0x14) } else { 0 });
            }
            10 => {
                call_regs(FONT_RELEASE_FIELD, 10, 0, 0, 0, v + 4, &[]);
                let rec = call_regs(FONT_ACQUIRE_FN, 0, 0, 0, 0, v + 4, &[s]);
                wd(a + 4, if rec != 0 { rd(rec as usize + 8) } else { 0 });
            }
            11 => {
                call_regs(SOUND_RELEASE_HANDLE, 11, rd(a + 4), 0, 0, edi0, &[]);
                let rec = call_regs(SOUND_ACQUIRE_FN, 0, 0, 0, 0, edi0, &[s]);
                wd(a + 4, if rec != 0 { rd(rec as usize + 8) } else { 0 });
            }
            _ => {}
        }
        v
    }
}

extern "C" fn set_name_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (v, s) = unsafe { (*args, *args.add(1)) };
    r.eax = if window_open(P_SET_NAME) {
        let tag = unsafe { rd(v as usize) };
        if matches!(tag, 6 | 9 | 10 | 11) {
            br(P_SET_NAME, match tag { 6 => 0, 9 => 1, 10 => 2, _ => 3 });
            unsafe { set_name(v, s, p.edi) }
        } else {
            checked_writer2(P_SET_NAME, &p, v, s, &[(v as usize, 16)], || unsafe { set_name(v, s, p.edi) })
        }
    } else {
        unsafe { set_name(v, s, p.edi) }
    };
}
regs_thunk!(set_name_thunk, set_name_entry, "ret 8");

// --- 0xB93790 vint_value_free (EAX = &value; the tag is NOT cleared) ----------------------------------

/// Tags 8..11 release their callback / resource (tag 8 returns the release's EAX, 9 / 10 / 11
/// zero the payload and return the helper's EAX); every other tag returns `tag - 8` in EAX. `esi`
/// / `edi` are the caller's registers (they reach the callbacks unchanged).
unsafe fn value_free(v: u32, esi: u32, edi: u32) -> u32 {
    unsafe {
        let a = v as usize;
        match rd(a) {
            8 => {
                let h = rd(a + 4);
                let cb = call_regs(CALLBACK_FIND, 0, 0, h, v, edi, &[]);
                if cb == 0 { 0 } else { call_regs(CALLBACK_RELEASE_FN, cb, cb, h, esi, edi, &[]) }
            }
            9 => {
                let r = call_regs(BITMAP_RELEASE_FIELD, 1, 0, 0, v + 4, edi, &[]);
                wd(a + 4, 0);
                r
            }
            10 => {
                let r = call_regs(FONT_RELEASE_FIELD, 2, 0, 0, v, v + 4, &[]);
                wd(a + 4, 0);
                r
            }
            11 => {
                let r = call_regs(SOUND_RELEASE_HANDLE, 3, rd(a + 4), 0, v, edi, &[]);
                wd(a + 4, 0);
                r
            }
            t => t.wrapping_sub(8),
        }
    }
}

extern "C" fn value_free_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let v = p.eax;
    r.eax = if window_open(P_VALUE_FREE) {
        let tag = unsafe { rd(v as usize) };
        if (8..=11).contains(&tag) {
            br(P_VALUE_FREE, (tag - 8) as usize);
            unsafe { value_free(v, p.esi, p.edi) }
        } else {
            checked_writer(P_VALUE_FREE, &p, 0, &[(v as usize, 16)], || unsafe { value_free(v, p.esi, p.edi) })
        }
    } else {
        unsafe { value_free(v, p.esi, p.edi) }
    };
}
regs_thunk!(value_free_thunk, value_free_entry);

// --- 0xB93620 vint_value_assign (ESI = dst, EDI = src -> EAX = dst) -----------------------------------

/// A typed destination (tag != 0) with a different source tag is left alone. Otherwise the
/// destination is freed (tag kept), takes the source tag and copies the payload: 1 / 2 / 4 / 12 a
/// dword, 3 through `fld / fstp` (a signalling NaN comes out quiet), 5 a byte, 6 three dwords,
/// 7 two; 8 / 9 / 10 / 11 copy the handle and count it (callback, bitmap, font, sound); tag 0 and
/// tags above 12 only copy the tag.
unsafe fn value_assign(d: u32, s: u32) -> u32 {
    unsafe {
        let (da, sa) = (d as usize, s as usize);
        let dt = rd(da);
        if dt != 0 && rd(sa) != dt {
            return d;
        }
        value_free(d, d, s);
        let t = rd(sa);
        wd(da, t);
        match t.wrapping_sub(1) {
            0 | 1 | 3 | 11 => wd(da + 4, rd(sa + 4)),
            2 => {
                std::arch::asm!("fld dword ptr [{s}]", "fstp dword ptr [{d}]", s = in(reg) sa + 4, d = in(reg) da + 4, clobber_abi("C"));
            }
            4 => wb(da + 4, rb(sa + 4)),
            5 => {
                wd(da + 4, rd(sa + 4));
                wd(da + 8, rd(sa + 8));
                wd(da + 12, rd(sa + 12));
            }
            6 => {
                wd(da + 4, rd(sa + 4));
                wd(da + 8, rd(sa + 8));
            }
            7 => {
                let h = rd(sa + 4);
                let cb = call_regs(CALLBACK_FIND, 0, 0, h, d, s, &[]);
                if cb != 0 {
                    wd(cb as usize + 0x1C, rd(cb as usize + 0x1C).wrapping_add(1));
                }
                wd(da + 4, rd(sa + 4));
            }
            8 => {
                let h = rd(sa + 4);
                wd(da + 4, h);
                call_regs(BITMAP_ADDREF, 0, h, 0, d, s, &[]);
            }
            9 => {
                let h = rd(sa + 4);
                wd(da + 4, h);
                call_regs(FONT_ADDREF, 0, h, 0, d, s, &[]);
            }
            10 => {
                let h = rd(sa + 4);
                wd(da + 4, h);
                call_regs(SOUND_ADDREF, 0, h, 0, d, s, &[]);
            }
            _ => {}
        }
        d
    }
}

extern "C" fn value_assign_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let (d, s) = (p.esi, p.edi);
    r.eax = if window_open(P_VALUE_ASSIGN) {
        let (dt, st) = unsafe { (rd(d as usize), rd(s as usize)) };
        let proceeds = dt == 0 || dt == st;
        if proceeds && ((8..=11).contains(&dt) || (8..=11).contains(&st)) {
            br(P_VALUE_ASSIGN, 0);
            unsafe { value_assign(d, s) }
        } else {
            checked_writer(P_VALUE_ASSIGN, &p, 0, &[(d as usize, 16)], || unsafe { value_assign(d, s) })
        }
    } else {
        unsafe { value_assign(d, s) }
    };
}
regs_thunk!(value_assign_thunk, value_assign_entry);

// --- the release helpers ---------------------------------------------------------------------------------

/// 0xB92D60 (ESI = &handle field): a valid bitmap handle calls the record's vtable[1] (ECX =
/// record); the field is zeroed either way. EAX is the handle, or the callee's result.
unsafe fn bitmap_release_field(f: u32, edi: u32) -> u32 {
    unsafe {
        let h = rd(f as usize);
        let mut eax = h;
        if h != 0 {
            let rec = bitmap_by_handle(h);
            if rec != 0 {
                let vt = rd(rec as usize);
                let fnp = rd(vt as usize + 4);
                eax = call_regs(fnp as usize, vt, rec, fnp, f, edi, &[]);
            }
        }
        wd(f as usize, 0);
        eax
    }
}

/// 0xB92DD0 (EDI = &handle field): a valid font handle (16-bit index below 10) releases the
/// record through 0xB9B8C0; the field is zeroed either way.
unsafe fn font_release_field(f: u32) -> u32 {
    unsafe {
        let h = rd(f as usize);
        let mut eax = h;
        if h != 0 && (h & 0xFFFF) < 0xA {
            let rec = font_by_handle(h);
            if rec != 0 {
                eax = call_regs(FONT_RECORD_RELEASE, h, h & 0xFFFF, 0, rec, f, &[]);
            }
        }
        wd(f as usize, 0);
        eax
    }
}

/// 0xB92E50 (ECX = handle): a valid sound handle releases the record through 0xBA0570. EAX
/// stays the caller's when the handle is 0, the low 16 bits of the handle when it is out of
/// range or stale, else the release's result.
unsafe fn sound_release_handle(h: u32, eax_in: u32, edi: u32) -> u32 {
    unsafe {
        if h == 0 {
            return eax_in;
        }
        let idx = h & 0xFFFF;
        if idx >= 0x32 {
            return idx;
        }
        let rec = sound_by_handle(h);
        if rec == 0 {
            return idx;
        }
        call_regs(SOUND_RECORD_RELEASE, idx, h, 0, rec, edi, &[])
    }
}

extern "C" fn bitmap_release_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let f = p.esi;
    r.eax = if window_open(P_BITMAP_RELEASE) {
        let valid = unsafe { let h = rd(f as usize); h != 0 && bitmap_by_handle(h) != 0 };
        if valid {
            br(P_BITMAP_RELEASE, 0);
            unsafe { bitmap_release_field(f, p.edi) }
        } else {
            checked_writer(P_BITMAP_RELEASE, &p, 0, &[(f as usize, 4)], || unsafe { bitmap_release_field(f, p.edi) })
        }
    } else {
        unsafe { bitmap_release_field(f, p.edi) }
    };
}
regs_thunk!(bitmap_release_thunk, bitmap_release_entry);

extern "C" fn font_release_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let f = p.edi;
    r.eax = if window_open(P_FONT_RELEASE) {
        let valid = unsafe { let h = rd(f as usize); h != 0 && (h & 0xFFFF) < 0xA && font_by_handle(h) != 0 };
        if valid {
            br(P_FONT_RELEASE, 0);
            unsafe { font_release_field(f) }
        } else {
            checked_writer(P_FONT_RELEASE, &p, 0, &[(f as usize, 4)], || unsafe { font_release_field(f) })
        }
    } else {
        unsafe { font_release_field(f) }
    };
}
regs_thunk!(font_release_thunk, font_release_entry);

extern "C" fn sound_release_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let h = p.ecx;
    r.eax = if window_open(P_SOUND_RELEASE) {
        let valid = unsafe { h != 0 && (h & 0xFFFF) < 0x32 && sound_by_handle(h) != 0 };
        if valid {
            br(P_SOUND_RELEASE, 0);
            unsafe { sound_release_handle(h, p.eax, p.edi) }
        } else {
            checked_writer(P_SOUND_RELEASE, &p, 0, &[], || unsafe { sound_release_handle(h, p.eax, p.edi) })
        }
    } else {
        unsafe { sound_release_handle(h, p.eax, p.edi) }
    };
}
regs_thunk!(sound_release_thunk, sound_release_entry);

/// 0xBA0850 (ECX = callback): refcount 1 tail-calls vtable[5] (ECX = this); else decrements.
/// EAX is the vtable function's result, or the new count.
unsafe fn callback_release(c: u32, esi: u32, edi: u32) -> u32 {
    unsafe {
        let a = c as usize;
        let rc = rd(a + 0x1C);
        if rc == 1 {
            let vt = rd(a);
            let f = rd(vt as usize + 0x14);
            call_regs(f as usize, vt, c, f, esi, edi, &[])
        } else {
            let n = rc.wrapping_sub(1);
            wd(a + 0x1C, n);
            n
        }
    }
}

extern "C" fn callback_release_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let c = p.ecx;
    r.eax = if window_open(P_CALLBACK_RELEASE) {
        if unsafe { rd(c as usize + 0x1C) } == 1 {
            br(P_CALLBACK_RELEASE, 0);
            unsafe { callback_release(c, p.esi, p.edi) }
        } else {
            checked_writer(P_CALLBACK_RELEASE, &p, 0, &[(c as usize + 0x1C, 4)], || unsafe { callback_release(c, p.esi, p.edi) })
        }
    } else {
        unsafe { callback_release(c, p.esi, p.edi) }
    };
}
regs_thunk!(callback_release_thunk, callback_release_entry);

// --- 0xB94FE0 vint_dataitem_get_or_create (EAX = name -> EAX = item or 0) -------------------------------

const ITEM_LIVE: usize = 0x0252_A1F4;
const ITEM_FREE: usize = 0x0252_A1F8;
const WAITING: usize = 0x0252_A210;
const GROUP_FREE: usize = 0x0252_A200;
const GROUP_LIVE: usize = 0x0252_A204;
const ITEM_POOL: usize = 0x031A_96F0; // 0x80 records of 0x22C
const GROUP_POOL: usize = 0x031B_ACF0; // 0x20 records of 0x20

/// A miss in the by-name finder takes the head of the free ring, links it into the live ring,
/// initialises it (`dataitem_init`, by address), then moves every waiting node whose key (+4)
/// equals the new item's hash from the waiting ring into the item's subscriber ring (+0x220),
/// storing the item's handle in the node. 0 for a null name or an empty free ring.
unsafe fn dataitem_get_or_create(name: u32) -> u32 {
    unsafe {
        if name == 0 {
            return 0;
        }
        let found = dataitem_by_name(name);
        if found != 0 {
            return found;
        }
        let esi = rd(ITEM_FREE);
        if esi == 0 {
            return 0;
        }
        let s = esi as usize;
        // out of the free ring (links +8 prev, +0xC next)
        let next = rd(s + 0xC);
        if next == esi {
            wd(ITEM_FREE, 0);
        } else {
            let prev = rd(s + 8);
            wd(prev as usize + 0xC, next);
            let n2 = rd(s + 0xC);
            wd(ITEM_FREE, next);
            let p2 = rd(s + 8);
            wd(n2 as usize + 8, p2);
        }
        // into the live ring
        let live = rd(ITEM_LIVE);
        wd(s + 8, 0);
        wd(s + 0xC, 0);
        if live == 0 {
            wd(s + 0xC, esi);
            wd(s + 8, esi);
            wd(ITEM_LIVE, esi);
        } else {
            let lp = rd(live as usize + 8);
            wd(s + 8, lp);
            wd(s + 0xC, live);
            let lp2 = rd(live as usize + 8);
            wd(lp2 as usize + 0xC, esi);
            wd(live as usize + 8, esi);
        }
        call_regs(DATAITEM_INIT, 0, name, 0, esi, name, &[]);
        // adopt the waiting subscribers keyed by this name hash
        let mut ebx = rd(WAITING);
        let mut eax = ebx;
        while eax != 0 {
            let a = eax as usize;
            let mut edx = rd(a + 0x14);
            let key = rd(a + 4);
            let ecx = if edx != ebx { edx } else { 0 };
            if key == rd(s + 0x10) {
                let mut neighbours = true;
                if ebx == eax {
                    if edx == eax {
                        ebx = 0;
                        neighbours = false;
                    } else {
                        ebx = edx;
                    }
                }
                if neighbours {
                    let prev = rd(a + 0x18);
                    if prev != 0 {
                        wd(prev as usize + 0x14, edx);
                    }
                    edx = rd(a + 0x14);
                    if edx != 0 {
                        let prev2 = rd(a + 0x18);
                        wd(edx as usize + 0x18, prev2);
                    }
                }
                wd(a + 0x18, 0);
                wd(a + 0x14, 0);
                wd(a, rd(s + 0x224));
                let head = rd(s + 0x220);
                if head == 0 {
                    wd(a + 0x14, eax);
                    wd(a + 0x18, eax);
                    wd(s + 0x220, eax);
                } else {
                    let tail = rd(head as usize + 0x18);
                    wd(a + 0x18, tail);
                    let h2 = rd(s + 0x220);
                    wd(a + 0x14, h2);
                    let h3 = rd(s + 0x220);
                    let t3 = rd(h3 as usize + 0x18);
                    wd(t3 as usize + 0x14, eax);
                    let h4 = rd(s + 0x220);
                    wd(h4 as usize + 0x18, eax);
                }
            }
            eax = ecx;
        }
        wd(WAITING, ebx);
        esi
    }
}

/// The waiting ring's nodes as snapshot ranges (0x1C bytes each); `None` when the ring looks
/// broken (null link or more than 4096 nodes), so the caller counts the call instead.
unsafe fn waiting_ranges(out: &mut Vec<(usize, usize)>) -> bool {
    unsafe {
        let head = rd(WAITING);
        let mut n = head;
        let mut guard = 0;
        while n != 0 {
            out.push((n as usize, 0x1C));
            n = rd(n as usize + 0x14);
            if n == head {
                break;
            }
            guard += 1;
            if guard > 4096 {
                return false;
            }
        }
        true
    }
}

extern "C" fn item_get_or_create_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let name = p.eax;
    r.eax = if window_open(P_ITEM_GET_OR_CREATE) {
        let mut ranges = vec![(ITEM_LIVE, 0x20), (ITEM_POOL, 0x80 * 0x22C)];
        if unsafe { waiting_ranges(&mut ranges) } {
            checked_writer(P_ITEM_GET_OR_CREATE, &p, 0, &ranges, || unsafe { dataitem_get_or_create(name) })
        } else {
            br(P_ITEM_GET_OR_CREATE, 0);
            unsafe { dataitem_get_or_create(name) }
        }
    } else {
        unsafe { dataitem_get_or_create(name) }
    };
}
regs_thunk!(item_get_or_create_thunk, item_get_or_create_entry);

// --- 0xB94140 vint_datagroup_create (ECX = name -> EAX = group or 0) -------------------------------------

/// Takes the head of the group free ring (links +0 next, +4 prev), links it into the live ring,
/// runs `0xB93B00` (hash, handle with a generation), then moves the waiting nodes keyed (+0) by
/// the group's hash into the group's subscriber ring (+0x18), storing the group's handle (+0x14)
/// in each. 0 for a null name. An empty free ring returns 0 at once (the original would go on
/// to read `[8]` when the waiting ring is not empty, and fault).
unsafe fn datagroup_create(name: u32) -> u32 {
    unsafe {
        if name == 0 {
            return 0;
        }
        let esi = rd(GROUP_FREE);
        if esi == 0 {
            return 0;
        }
        let s = esi as usize;
        let next = rd(s);
        if next == esi {
            wd(GROUP_FREE, 0);
        } else {
            let prev = rd(s + 4);
            wd(prev as usize, next);
            let p2 = rd(s + 4);
            wd(GROUP_FREE, next);
            let n2 = rd(s);
            wd(n2 as usize + 4, p2);
        }
        let live = rd(GROUP_LIVE);
        wd(s + 4, 0);
        wd(s, 0);
        if live == 0 {
            wd(s, esi);
            wd(s + 4, esi);
            wd(GROUP_LIVE, esi);
        } else {
            let lp = rd(live as usize + 4);
            wd(s + 4, lp);
            wd(s, live);
            let lp2 = rd(live as usize + 4);
            wd(lp2 as usize, esi);
            wd(live as usize + 4, esi);
        }
        call_regs(DATAGROUP_INIT, 0, name, 0, esi, 0, &[]);
        let mut ebx = rd(WAITING);
        let mut eax = ebx;
        while eax != 0 {
            let a = eax as usize;
            let mut edx = rd(a + 0x14);
            let key = rd(a);
            let ecx = if edx != ebx { edx } else { 0 };
            if key == rd(s + 8) {
                let mut neighbours = true;
                if ebx == eax {
                    if edx == eax {
                        ebx = 0;
                        neighbours = false;
                    } else {
                        ebx = edx;
                    }
                }
                if neighbours {
                    let prev = rd(a + 0x18);
                    if prev != 0 {
                        wd(prev as usize + 0x14, edx);
                    }
                    edx = rd(a + 0x14);
                    if edx != 0 {
                        let prev2 = rd(a + 0x18);
                        wd(edx as usize + 0x18, prev2);
                    }
                }
                wd(a + 0x18, 0);
                wd(a + 0x14, 0);
                wd(a, rd(s + 0x14));
                let head = rd(s + 0x18);
                if head == 0 {
                    wd(a + 0x14, eax);
                    wd(a + 0x18, eax);
                    wd(s + 0x18, eax);
                } else {
                    let tail = rd(head as usize + 0x18);
                    wd(a + 0x18, tail);
                    let h2 = rd(s + 0x18);
                    wd(a + 0x14, h2);
                    let h3 = rd(s + 0x18);
                    let t3 = rd(h3 as usize + 0x18);
                    wd(t3 as usize + 0x14, eax);
                    let h4 = rd(s + 0x18);
                    wd(h4 as usize + 0x18, eax);
                }
            }
            eax = ecx;
        }
        wd(WAITING, ebx);
        esi
    }
}

extern "C" fn group_create_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let name = p.ecx;
    r.eax = if window_open(P_GROUP_CREATE) {
        // The empty-free-ring fault path of the original is never shadowed.
        let mut ranges = vec![(GROUP_FREE, 0x14), (GROUP_POOL, 0x20 * 0x20)];
        let ok = unsafe { name == 0 || rd(GROUP_FREE) != 0 } && unsafe { waiting_ranges(&mut ranges) };
        if ok {
            checked_writer(P_GROUP_CREATE, &p, 0, &ranges, || unsafe { datagroup_create(name) })
        } else {
            br(P_GROUP_CREATE, 0);
            unsafe { datagroup_create(name) }
        }
    } else {
        unsafe { datagroup_create(name) }
    };
}
regs_thunk!(group_create_thunk, group_create_entry);

// --- the three resource caches (cdecl (const char* name) -> EAX = record or 0) -----------------------------

/// Pop the head of a free ring (links `next` / `prev` as dword offsets) and push the record at
/// the back of the live ring: the common tail of the three `acquire_by_name` functions.
unsafe fn ring_take(esi: u32, free: usize, live: usize, next: usize, prev: usize) -> u32 {
    unsafe {
        let s = esi as usize;
        let mut relink = true;
        if rd(free) == esi {
            let n = rd(s + next);
            if n == esi {
                wd(s + next, 0);
                wd(s + prev, 0);
                wd(free, 0);
                relink = false;
            } else {
                wd(free, n);
            }
        }
        if relink {
            let pv = rd(s + prev);
            if pv != 0 {
                wd(pv as usize + next, rd(s + next));
            }
            let nx = rd(s + next);
            if nx != 0 {
                wd(nx as usize + prev, rd(s + prev));
            }
            wd(s + next, 0);
            wd(s + prev, 0);
        }
        let head = rd(live);
        if head == 0 {
            wd(s + next, esi);
            wd(s + prev, esi);
            wd(live, esi);
        } else {
            let t = rd(head as usize + prev);
            wd(s + next, head);
            wd(s + prev, t);
            let t2 = rd(head as usize + prev);
            wd(t2 as usize + next, esi);
            wd(head as usize + prev, esi);
        }
        esi
    }
}

/// `(pool index) * 0x10000 + generation`: `idx` from the compiler's multiply-by-reciprocal
/// division of the byte offset (`magic` signed, then `sar shift`, plus the sign bit).
fn pool_index(off: u32, magic: u32, shift: u32, add_off: bool) -> u32 {
    let hi = ((off as i32 as i64 * magic as i32 as i64) >> 32) as i32;
    let t = if add_off { hi.wrapping_add(off as i32) } else { hi };
    let q = t >> shift;
    ((q as u32) >> 31).wrapping_add(q as u32)
}

unsafe fn crc_lower(name: u32) -> u32 {
    unsafe { call_regs(STRING_CRC32_LOWER, 0, name, 0, 0, 0, &[]) }
}

/// The ring walk at the top of the font / sound caches: the first record (following `next`, a
/// ring ending on the head or a null) whose key dword (+4) equals the hash.
unsafe fn cache_lookup(head_addr: usize, hash: u32, next: usize) -> u32 {
    unsafe {
        let head = rd(head_addr);
        let mut n = head;
        while n != 0 {
            if rd(n as usize + 4) == hash {
                return n;
            }
            n = rd(n as usize + next);
            if n == head {
                break;
            }
        }
        0
    }
}

const FONT_LIVE: usize = 0x0252_A258;
const FONT_FREE: usize = 0x0252_A25C;
const FONT_GEN: usize = 0x0252_A260;
const BITMAP_LIVE: usize = 0x0252_A264;
const BITMAP_FREE: usize = 0x0252_A268;
const BITMAP_GEN: usize = 0x0252_A26C;
const SOUND_LIVE: usize = 0x0252_A298;
const SOUND_FREE: usize = 0x0252_A29C;
const SOUND_GEN: usize = 0x0252_A2A0;

/// 0xB9B6F0. A hit in the live ring (key +4 = crc32_lower, next +0x14) adds one to the count
/// (+0x10). A miss takes a free record, asks the platform hook +0x10 for an id (-1 gives 0) and
/// hook +0x24 to fill +0xC, then stamps handle (+8) = generation << 16 + index, id (+0), hash
/// (+4), count = 1 and rings it in.
unsafe fn font_acquire(name: u32) -> u32 {
    unsafe {
        if name == 0 {
            return 0;
        }
        let hit = cache_lookup(FONT_LIVE, crc_lower(name), 0x14);
        if hit != 0 {
            wd(hit as usize + 0x10, rd(hit as usize + 0x10).wrapping_add(1));
            return hit;
        }
        let esi = rd(FONT_FREE);
        if esi == 0 {
            return 0;
        }
        let plat = rd(PLATFORM);
        if plat == 0 {
            return 0;
        }
        let f = rd(plat as usize + 0x10);
        if f == 0 {
            return 0;
        }
        let id = call_regs(f as usize, 0, 0, 0, 0, 0, &[name]);
        let f2 = rd(rd(PLATFORM) as usize + 0x24);
        let s = esi as usize;
        if f2 != 0 {
            let old = rd(s);
            let out = esi + 0xC;
            let al = call_regs(f2 as usize, 0, 0, 0, 0, 0, &[out, old]) as u8;
            if al == 0 {
                wd(s + 0xC, 0);
            }
        } else {
            wd(s + 0xC, 0);
        }
        if id == 0xFFFF_FFFF {
            return 0;
        }
        let di = rw(FONT_GEN).wrapping_add(1);
        let idx = pool_index(esi.wrapping_sub(0x031B_C400), 0x9249_2493, 4, true);
        wd(s + 8, (idx & 0xFFFF).wrapping_add((di as u32) << 16));
        ww(FONT_GEN, di);
        wd(s, id);
        let h = crc_lower(name);
        wd(s + 4, h);
        wd(s + 0x10, 1);
        ring_take(esi, FONT_FREE, FONT_LIVE, 0x14, 0x18)
    }
}

/// 0xBA03F0: as the font cache, with next +0x10, count +0xC, hook +0x2C (a 16-bit id, 0xFFFF
/// gives 0) and a word id at +0.
unsafe fn sound_acquire(name: u32) -> u32 {
    unsafe {
        if name == 0 {
            return 0;
        }
        let hit = cache_lookup(SOUND_LIVE, crc_lower(name), 0x10);
        if hit != 0 {
            wd(hit as usize + 0xC, rd(hit as usize + 0xC).wrapping_add(1));
            return hit;
        }
        let esi = rd(SOUND_FREE);
        if esi == 0 {
            return 0;
        }
        let plat = rd(PLATFORM);
        if plat == 0 {
            return 0;
        }
        let f = rd(plat as usize + 0x2C);
        if f == 0 {
            return 0;
        }
        let id = call_regs(f as usize, 0, 0, 0, 0, 0, &[name]) & 0xFFFF;
        if id == 0xFFFF {
            return 0;
        }
        let s = esi as usize;
        let di = rw(SOUND_GEN).wrapping_add(1);
        let idx = pool_index(esi.wrapping_sub(0x031B_E700), 0x2AAA_AAAB, 2, false);
        wd(s + 8, (idx & 0xFFFF).wrapping_add((di as u32) << 16));
        ww(SOUND_GEN, di);
        ww(s, id as u16);
        let h = crc_lower(name);
        wd(s + 4, h);
        wd(s + 0xC, 1);
        ring_take(esi, SOUND_FREE, SOUND_LIVE, 0x10, 0x14)
    }
}

/// 0xB9BD20: hash = `string_hash_lower` (0xBDCB30, seed 0); 0xB9BBC0 (EDI = &hash, by address)
/// finds or loads a cached record (count +0x18, ring next +0x1C). A zero result takes a free
/// record, asks hook +0 for a texture id (-1 gives 0; the id is stored at +4 even then) and
/// stamps count = 1, hash (+0x10), handle (+0x14), flag (+0xC) = 0, then rings it in.
unsafe fn bitmap_acquire(name: u32, esi0: u32) -> u32 {
    unsafe {
        if name == 0 {
            return 0;
        }
        let mut hash = 0u32;
        let hp = &mut hash as *mut u32 as u32;
        call_regs(HASH_LOWER, 0, name, 0, esi0, 0, &[hp]);
        let found = call_regs(BITMAP_FIND_CACHED, 0, 0, 0, esi0, hp, &[]);
        if found != 0 {
            return found;
        }
        let esi = rd(BITMAP_FREE);
        if esi == 0 {
            return 0;
        }
        let plat = rd(PLATFORM);
        if plat == 0 {
            return 0;
        }
        let f = rd(plat as usize);
        if f == 0 {
            return 0;
        }
        let id = call_regs(f as usize, 0, 0, 0, 0, 0, &[name]);
        let s = esi as usize;
        wd(s + 4, id);
        if id == 0xFFFF_FFFF {
            return 0;
        }
        let di = rw(BITMAP_GEN).wrapping_add(1);
        let idx = pool_index(esi.wrapping_sub(0x031B_C518), 0x38E3_8E39, 3, false);
        let handle = (idx & 0xFFFF).wrapping_add((di as u32) << 16);
        wd(s + 0xC, 0);
        ww(BITMAP_GEN, di);
        wd(s + 0x14, handle);
        wd(s + 0x10, hash);
        wd(s + 0x18, 1);
        ring_take(esi, BITMAP_FREE, BITMAP_LIVE, 0x1C, 0x20)
    }
}

/// The three entries: a hit in the live ring only adds one to a count, so it takes the snapshot
/// shadow (the count word); a miss runs platform hooks and relinks rings, so it is counted.
macro_rules! acquire_entry {
    ($entry:ident, $thunk:ident, $id:ident, $f:ident, $live:ident, $next:literal, $count:literal) => {
        extern "C" fn $entry(r: &mut Pushad, args: *const u32) {
            let p = *r;
            let name = unsafe { *args };
            r.eax = if window_open($id) {
                let hit = unsafe { if name == 0 { 0 } else { cache_lookup($live, crc_lower(name), $next) } };
                if hit != 0 {
                    checked_writer($id, &p, name, &[(hit as usize + $count, 4)], || unsafe { $f(name) })
                } else {
                    br($id, 0);
                    unsafe { $f(name) }
                }
            } else {
                unsafe { $f(name) }
            };
        }
        regs_thunk!($thunk, $entry);
    };
}
acquire_entry!(font_acquire_entry, font_acquire_thunk, P_FONT_ACQUIRE, font_acquire, FONT_LIVE, 0x14, 0x10);
acquire_entry!(sound_acquire_entry, sound_acquire_thunk, P_SOUND_ACQUIRE, sound_acquire, SOUND_LIVE, 0x10, 0xC);

extern "C" fn bitmap_acquire_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let name = unsafe { *args };
    r.eax = if window_open(P_BITMAP_ACQUIRE) {
        // Only a hit in the live ring (0xB9BBC0's first loop: key +0x10, next +0x1C) is shadowed.
        let hit = unsafe {
            if name == 0 {
                0
            } else {
                let mut hash = 0u32;
                call_regs(HASH_LOWER, 0, name, 0, 0, 0, &[&mut hash as *mut u32 as u32]);
                let head = rd(BITMAP_LIVE);
                let mut n = head;
                let mut found = 0;
                while n != 0 {
                    if rd(n as usize + 0x10) == hash {
                        found = n;
                        break;
                    }
                    n = rd(n as usize + 0x1C);
                    if n == head {
                        break;
                    }
                }
                found
            }
        };
        if hit != 0 {
            checked_writer(P_BITMAP_ACQUIRE, &p, name, &[(hit as usize + 0x18, 4)], || unsafe { bitmap_acquire(name, p.esi) })
        } else {
            br(P_BITMAP_ACQUIRE, 0);
            unsafe { bitmap_acquire(name, p.esi) }
        }
    } else {
        unsafe { bitmap_acquire(name, p.esi) }
    };
}
regs_thunk!(bitmap_acquire_thunk, bitmap_acquire_entry);

// --- 0xB926D0 vint_insert_values_expand (cdecl (out, fmt, cap_chars, values, count) -> chars) -------------

const P_EXPAND: usize = 37;

const VALUE_TO_TEXT: usize = 0x00B9_3170; // EAX = &value, ECX = char* dest (ESI forced to 0x1000 inside)
const LOC_FIND: usize = 0x00B9_21E0; // ECX = char* -> wide text or 0 (hook +0)
const LOC_HASH: usize = 0x00B9_2200; // ECX = crc -> wide text or 0 (hook +4)
const LOC_SYMBOL: usize = 0x00B9_2270; // ECX = length, EDX = wide name -> wide text or 0 (hook +0xC)
const DECODE_WIDE: usize = 0x00B9_2670; // EAX = encoded src, EDX = wide dst, cdecl (cap) -> AL
const PARSE_DECIMAL: usize = 0x00B9_1E70; // ESI = wide digits -> EAX
const VALUE_TO_INT_B: usize = 0x0063_4BA0; // ECX = &value -> EAX
const WIDE_STRNCMP: usize = 0x00BF_2D80; // EAX = s1, cdecl (s2, n) -> 0 when equal
const VALUE_ASSIGN_FN: usize = 0x00B9_3620; // ESI = dst, EDI = src
const VALUE_FREE_FN: usize = 0x00B9_3790; // EAX = &value
const MODIFIERS: usize = 0x00E8_6F48; // two wide strings: text_tag_crc, not_localized
const MSG_NO_VALUE: u32 = 0x00E4_CF48;
const MSG_HASH: u32 = 0x00E4_CF88;
const MSG_INVALID: u32 = 0x00E4_CFB0;
const MSG_SYMBOL: u32 = 0x00E4_CFFC;

/// Copy `src` bytes to wide chars (the NUL included, at most 0x1000 chars), then force the last
/// char of the 0x1000-char buffer to 0: the fallback when `DECODE_WIDE` declines.
unsafe fn widen_bytes(src: u32, wp: u32) {
    unsafe {
        for i in 0..0x1000usize {
            let c = rb(src as usize + i);
            ww(wp as usize + i * 2, c as u16);
            if c == 0 {
                break;
            }
        }
        ww(wp as usize + 0x1FFE, 0);
    }
}

unsafe fn wide_len(s: u32) -> u32 {
    unsafe {
        let mut n = 0;
        while rw(s as usize + n * 2) != 0 {
            n += 1;
        }
        n as u32
    }
}

/// `%s` / `%S` (next value as text), `%%`, `{{`, `{}`, `{N}`, `{N:modifier}` and `{name}`
/// expansion of a wide format into `out` (`cap` chars). Returns the chars written, the
/// terminating NUL included when the format ended inside the buffer; the last char of the buffer
/// is always set to 0. Quirks kept: `%%` and `{{` also emit the char after them, `%x` drops the
/// `%`, a `{` with no `}` runs to the NUL, a `:` skips NULs too. The only change: the second
/// char of a `%%` / `{{` pair is not written past the end of the buffer (the original does when
/// exactly one slot is left); the returned count is unchanged.
unsafe fn insert_values_expand(out: u32, fmt: u32, cap: u32, values: u32, count: u32) -> u32 {
    unsafe {
        if out == 0 || fmt == 0 || cap == 0 {
            return 0;
        }
        let mut text_buf = std::mem::MaybeUninit::<[u8; 0x1000]>::uninit();
        let mut wide_buf = std::mem::MaybeUninit::<[u16; 0x1000]>::uninit();
        let mut local_buf = std::mem::MaybeUninit::<[u32; 4]>::uninit();
        let tp = text_buf.as_mut_ptr() as u32;
        let wp = wide_buf.as_mut_ptr() as u32;
        let lp = local_buf.as_mut_ptr() as u32;
        let cap_end = (out as usize).wrapping_add(cap as usize * 2);
        let used = |cur: u32| (cur.wrapping_sub(out) as i32 >> 1) as u32;
        let store = |cur: u32, c: u32| {
            if (cur as usize) < cap_end {
                ww(cur as usize, c as u16);
            }
        };
        let mut cur = out;
        let mut edi = fmt;
        let mut vidx = 0u32;
        let mut raw = false;
        loop {
            let mut ebp = 0u32;
            let ax = rw(edi as usize) as u32;
            if ax == 1 {
                raw = !raw;
            }
            let mut lit: Option<u32> = None;
            let mut apply = false;
            if raw {
                lit = Some(ax);
            } else if ax == 0x25 {
                let c = rw(edi as usize + 2) as u32;
                edi += 2;
                if c == 0x25 {
                    store(cur, c);
                    cur += 2;
                    edi += 2;
                    lit = Some(rw(edi as usize) as u32);
                } else if c == 0x73 || c == 0x53 {
                    if vidx < count {
                        let val = values.wrapping_add(vidx << 4);
                        call_regs(VALUE_TO_TEXT, val, tp, 0, 0, 0, &[]);
                        ebp = call_regs(LOC_FIND, 0, tp, 0, 0, 0, &[]);
                        if ebp == 0 {
                            if call_regs(DECODE_WIDE, tp, 0, wp, 0, 0, &[0x1000]) & 0xFF == 0 {
                                widen_bytes(tp, wp);
                            }
                            ebp = wp;
                        }
                    }
                    vidx += 1;
                    edi += 2;
                    apply = true;
                } else {
                    lit = Some(rw(edi as usize) as u32);
                }
            } else if ax == 0x7b {
                if rw(edi as usize + 2) as u32 == 0x7b {
                    store(cur, ax);
                    cur += 2;
                    edi += 4;
                    lit = Some(rw(edi as usize) as u32);
                } else {
                    let mut c;
                    loop {
                        c = rw(edi as usize + 2) as u32;
                        edi += 2;
                        if c != 0x20 && c != 9 {
                            break;
                        }
                    }
                    if c == 0x7d {
                        if vidx < count {
                            let val = values.wrapping_add(vidx << 4);
                            call_regs(VALUE_TO_TEXT, val, tp, 0, 0, 0, &[]);
                            if call_regs(DECODE_WIDE, tp, 0, wp, 0, 0, &[0x1000]) & 0xFF == 0 {
                                widen_bytes(tp, wp);
                            }
                            ebp = wp;
                        }
                        vidx += 1;
                    } else if (0x30..=0x39).contains(&c) {
                        let n = call_regs(PARSE_DECIMAL, 0, 0, 0, edi, 0, &[]);
                        loop {
                            let d = rw(edi as usize) as u32;
                            if (0x30..=0x39).contains(&d) || d == 0x20 || d == 9 {
                                edi += 2;
                            } else {
                                break;
                            }
                        }
                        let mut saved = edi;
                        if n < count {
                            let mut modidx: i32 = -1;
                            if rw(edi as usize) == 0x3A {
                                loop {
                                    let d = rw(edi as usize + 2) as u32;
                                    edi += 2;
                                    if d != 0x20 && d != 9 && d != 0 {
                                        break;
                                    }
                                }
                                saved = edi;
                                for k in 0..2u32 {
                                    let s = rd(MODIFIERS + k as usize * 4);
                                    let len = wide_len(s);
                                    if call_regs(WIDE_STRNCMP, edi, 0, 0, 0, 0, &[s, len]) == 0 {
                                        modidx = k as i32;
                                        break;
                                    }
                                }
                            }
                            let src = values.wrapping_add(n << 4);
                            wd(lp as usize, 0);
                            call_regs(VALUE_ASSIGN_FN, 0, 0, 0, lp, src, &[]);
                            let tag = rd(lp as usize);
                            if tag == 0 {
                                ebp = MSG_NO_VALUE;
                            } else if modidx == 0 {
                                if tag == 2 {
                                    let h = call_regs(VALUE_TO_INT_B, 0, lp, 0, 0, 0, &[]);
                                    ebp = call_regs(LOC_HASH, 0, h, 0, 0, 0, &[]);
                                    if ebp == 0 {
                                        ebp = MSG_HASH;
                                    }
                                } else {
                                    ebp = MSG_INVALID;
                                }
                            } else if tag == 4 {
                                let s = rd(lp as usize + 4);
                                let mut found = false;
                                if modidx != 1 {
                                    ebp = call_regs(LOC_FIND, 0, s, 0, 0, 0, &[]);
                                    found = ebp != 0;
                                }
                                if !found {
                                    if call_regs(DECODE_WIDE, s, 0, wp, 0, 0, &[0x1000]) & 0xFF == 0 {
                                        widen_bytes(s, wp);
                                    }
                                    ebp = wp;
                                }
                            } else {
                                call_regs(VALUE_TO_TEXT, lp, tp, 0, 0, 0, &[]);
                                widen_bytes(tp, wp);
                                ebp = wp;
                            }
                            call_regs(VALUE_FREE_FN, lp, 0, 0, 0, 0, &[]);
                            wd(lp as usize, 0);
                        }
                        edi = saved;
                    } else {
                        let mut ecx = edi;
                        let mut d = c;
                        loop {
                            if d == 0 {
                                break;
                            }
                            d = rw(ecx as usize + 2) as u32;
                            ecx += 2;
                            if d == 0x7d {
                                break;
                            }
                        }
                        let len = (ecx.wrapping_sub(edi) as i32 >> 1) as u32;
                        ebp = call_regs(LOC_SYMBOL, 0, len, edi, 0, 0, &[]);
                        if ebp == 0 {
                            ebp = MSG_SYMBOL;
                        }
                    }
                    // run to the closing brace
                    loop {
                        let d = rw(edi as usize);
                        if d == 0x7d {
                            edi += 2;
                            break;
                        }
                        if d == 0 {
                            break;
                        }
                        edi += 2;
                    }
                    apply = true;
                }
            } else {
                lit = Some(ax);
            }
            if apply {
                if ebp == 0 {
                    lit = Some(rw(edi as usize) as u32);
                } else {
                    let mut p = ebp;
                    let mut c = rw(p as usize) as u32;
                    if c != 0 {
                        let mut bytes = cur.wrapping_sub(out);
                        loop {
                            if ((bytes as i32 >> 1) as u32) >= cap {
                                break;
                            }
                            p += 2;
                            ww(cur as usize, c as u16);
                            cur += 2;
                            bytes += 2;
                            c = rw(p as usize) as u32;
                            if c == 0 {
                                break;
                            }
                        }
                    }
                }
            }
            if let Some(c) = lit {
                store(cur, c);
                cur += 2;
                if rw(edi as usize) == 0 {
                    break;
                }
                edi += 2;
            }
            if used(cur) >= cap {
                break;
            }
        }
        ww(out as usize + cap as usize * 2 - 2, 0);
        used(cur)
    }
}

// The five-argument shadow call (a cdecl callee pops nothing).
#[repr(C)]
#[allow(dead_code)]
struct Block5 {
    f: u32,
    r: Regs,
    unused: u32,
    esp: u32,
    a: [u32; 5],
}

unsafe fn call_full5(f: usize, r: &mut Regs, a: [u32; 5]) {
    let mut b = Block5 { f: f as u32, r: *r, unused: 0, esp: 0, a };
    unsafe {
        std::arch::asm!(
            "push ebp", "push ebx", "push esi", "push edi",
            "mov [eax + 36], esp",
            "push dword ptr [eax]",
            "push eax",
            "push dword ptr [eax + 56]",
            "push dword ptr [eax + 52]",
            "push dword ptr [eax + 48]",
            "push dword ptr [eax + 44]",
            "push dword ptr [eax + 40]",
            "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 24]",
            "push ebx",
            "mov ebx, [esp + 24]",
            "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
            "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 36]",
            "pop edi", "pop esi", "pop ebx", "pop ebp",
            inout("eax") &mut b as *mut Block5 => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
}

extern "C" fn expand_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let a = unsafe { [*args, *args.add(1), *args.add(2), *args.add(3), *args.add(4)] };
    let (out, fmt, cap, values, count) = (a[0], a[1], a[2], a[3], a[4]);
    let ours = || unsafe { insert_values_expand(out, fmt, cap, values, count) };
    let Some((n, orig)) = (if window_open(P_EXPAND) && cap as usize * 2 <= 0x10000 { shadowing(P_EXPAND) } else { None }) else {
        r.eax = ours();
        return;
    };
    // The values' handle refcounts are taken and given back inside the call, so the only net
    // write is the output buffer.
    let ranges: &[(usize, usize)] = &[(out as usize, cap as usize * 2)];
    let ranges = if out == 0 || cap == 0 { &[] } else { ranges };
    unsafe {
        let before = capture(ranges);
        let inp = Regs::of(&p);
        let mut t = inp;
        call_full5(orig, &mut t, a);
        check_preserved(P_EXPAND, &inp, &t);
        let theirs = capture(ranges);
        restore(ranges, &before);
        let e = ours();
        let mine = capture(ranges);
        if e != t.eax || mine != theirs {
            mismatch(P_EXPAND, n, format!("args {a:x?}: original {:#x}, Rust {e:#x}, first diff at {:?}", t.eax, mine.iter().zip(theirs.iter()).position(|(x, y)| x != y)));
            restore(ranges, &theirs);
            r.eax = t.eax;
        } else {
            r.eax = e;
        }
    }
}
regs_thunk!(expand_thunk, expand_entry);

// =====================================================================================================
// Round 2b: vint objects (name rehash, set name, document propagation), the raw input index, the
// project document finder, the clip pop and the datagroup delivery / destroy / unsubscribe.
// =====================================================================================================

const P_REHASH: usize = 38;
const P_OBJECT_SET_NAME: usize = 39;
const P_SET_DOCUMENT_REC: usize = 40;
const P_RAW_INPUT_INDEX: usize = 41;
const P_PROJECT_FIND: usize = 42;
const P_CLIP_POP: usize = 43;
const P_DELIVER_ALL: usize = 44;
const P_GROUP_DESTROY: usize = 45;
const P_GROUP_UNSUBSCRIBE: usize = 46;

const NAME_BUCKETS: usize = 0x022E_F1B8; // 64 object-name buckets (ring links +4 next, +8 prev)
const CALLBACK_BUCKETS: usize = 0x022F_2670; // 64 callback buckets (handle +0x18, next +8)
const NODE_FREE: usize = 0x0252_A20C; // subscriber-node free ring (links +0x14 next, +0x18 prev)
const STRNCPY: usize = 0x00C9_90F0;
const STRICMP: usize = 0x00C9_9030;
const DELIVER_ONE: usize = 0x00B9_45E0; // stdcall (group)
const GROUP_DESTROY_FN: usize = 0x00B9_3BC0; // stdcall (group)

/// Count a call of a port that has no shadow (its writes are not bounded).
fn count_only(id: usize) {
    BR[id * 4].fetch_add(1, Ordering::Relaxed);
}

// --- 0xB97570 vint_object_rehash_name (EAX = object, ECX = new name hash -> EAX = object) ------------------

/// Out of the old name bucket's ring (when the old hash is not 0), store the new hash at +0x2C,
/// into the new bucket's ring as its last node (when the new hash is not 0). Nothing when the
/// hash is unchanged.
unsafe fn object_rehash(obj: u32, newh: u32) -> u32 {
    unsafe {
        let o = obj as usize;
        let old = rd(o + 0x2C);
        if old == newh {
            return obj;
        }
        if old != 0 {
            let b = NAME_BUCKETS + (old & 0x3F) as usize * 4;
            let mut unlink = true;
            if rd(b) == obj {
                let nx = rd(o + 4);
                if nx == obj {
                    wd(o + 4, 0);
                    wd(o + 8, 0);
                    wd(b, 0);
                    unlink = false;
                } else {
                    wd(b, nx);
                }
            }
            if unlink {
                let pv = rd(o + 8);
                let nx = rd(o + 4);
                wd(pv as usize + 4, nx);
                let nx2 = rd(o + 4);
                let pv2 = rd(o + 8);
                wd(nx2 as usize + 8, pv2);
                wd(o + 4, 0);
                wd(o + 8, 0);
            }
        }
        wd(o + 0x2C, newh);
        if newh == 0 {
            return obj;
        }
        let b2 = NAME_BUCKETS + (newh & 0x3F) as usize * 4;
        let head = rd(b2);
        if head == 0 {
            wd(o + 4, obj);
            wd(o + 8, obj);
            wd(b2, obj);
        } else {
            let tail = rd(head as usize + 8);
            wd(o + 8, tail);
            wd(o + 4, rd(b2));
            let h3 = rd(b2);
            let t3 = rd(h3 as usize + 8);
            wd(t3 as usize + 4, obj);
            let h4 = rd(b2);
            wd(h4 as usize + 8, obj);
        }
        obj
    }
}

/// Everything `object_rehash` can write: the object's links and hash, the bucket table, and the
/// link words of its ring neighbours and of the new bucket's head and tail.
unsafe fn rehash_ranges(obj: u32, newh: u32, v: &mut Vec<(usize, usize)>) {
    unsafe {
        let o = obj as usize;
        v.push((o + 4, 8));
        v.push((o + 0x2C, 4));
        v.push((NAME_BUCKETS, 256));
        let mut add = |n: u32| {
            if n != 0 {
                v.push((n as usize + 4, 8));
            }
        };
        add(rd(o + 8));
        add(rd(o + 4));
        let head = rd(NAME_BUCKETS + (newh & 0x3F) as usize * 4);
        if head != 0 {
            add(head);
            add(rd(head as usize + 8));
        }
    }
}

extern "C" fn rehash_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let (obj, h) = (p.eax, p.ecx);
    r.eax = if window_open(P_REHASH) {
        let mut ranges = Vec::new();
        unsafe { rehash_ranges(obj, h, &mut ranges) };
        checked_writer(P_REHASH, &p, 0, &ranges, || unsafe { object_rehash(obj, h) })
    } else {
        unsafe { object_rehash(obj, h) }
    };
}
regs_thunk!(rehash_thunk, rehash_entry);

// --- 0xB97620 vint_object_set_name (EDI = name, stdcall (object)) ----------------------------------------------

/// A null name does nothing (EAX stays the caller's). Otherwise `strncpy(object + 0x34, name,
/// 0x20)` (the CRT, by address), a NUL at +0x53, and the rehash with `crc32_lower(name)`; EAX =
/// the object.
unsafe fn object_set_name(obj: u32, name: u32, eax_in: u32) -> u32 {
    unsafe {
        if name == 0 {
            return eax_in;
        }
        call_regs(STRNCPY, 0, 0, 0, 0, 0, &[obj + 0x34, name, 0x20]);
        wb(obj as usize + 0x53, 0);
        let h = crc_lower(name);
        object_rehash(obj, h)
    }
}

extern "C" fn object_set_name_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let obj = unsafe { *args };
    let name = p.edi;
    r.eax = if window_open(P_OBJECT_SET_NAME) {
        let mut ranges = Vec::new();
        if name != 0 {
            ranges.push((obj as usize + 0x34, 0x20));
            unsafe { rehash_ranges(obj, crc_lower(name), &mut ranges) };
        }
        checked_writer(P_OBJECT_SET_NAME, &p, obj, &ranges, || unsafe { object_set_name(obj, name, p.eax) })
    } else {
        unsafe { object_set_name(obj, name, p.eax) }
    };
}
regs_thunk!(object_set_name_thunk, object_set_name_entry, "ret 4");

// --- 0xB979B0 vint_object_set_document_rec (ECX = object, stdcall (document)) ---------------------------------

/// Set the document (+0x30) on the object and, first, on every child (+0x1C first child, +0x20
/// next sibling) unless the document is 0 or already the object's. EAX is never touched.
unsafe fn set_document_rec(obj: u32, doc: u32) {
    unsafe {
        if doc == 0 || rd(obj as usize + 0x30) == doc {
            return;
        }
        let mut c = rd(obj as usize + 0x1C);
        while c != 0 {
            set_document_rec(c, doc);
            c = rd(c as usize + 0x20);
        }
        wd(obj as usize + 0x30, doc);
    }
}

/// The +0x30 words the recursion can write (None when the tree looks broken).
unsafe fn document_ranges(obj: u32, doc: u32, v: &mut Vec<(usize, usize)>, budget: &mut u32) -> bool {
    unsafe {
        if doc == 0 || rd(obj as usize + 0x30) == doc {
            return true;
        }
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        v.push((obj as usize + 0x30, 4));
        let mut c = rd(obj as usize + 0x1C);
        while c != 0 {
            if !document_ranges(c, doc, v, budget) {
                return false;
            }
            c = rd(c as usize + 0x20);
        }
        true
    }
}

extern "C" fn set_document_rec_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (obj, doc) = (p.ecx, unsafe { *args });
    if window_open(P_SET_DOCUMENT_REC) {
        let mut ranges = Vec::new();
        let mut budget = 4096;
        if unsafe { document_ranges(obj, doc, &mut ranges, &mut budget) } {
            r.eax = checked_writer(P_SET_DOCUMENT_REC, &p, doc, &ranges, || {
                unsafe { set_document_rec(obj, doc) };
                p.eax
            });
        } else {
            br(P_SET_DOCUMENT_REC, 0);
            unsafe { set_document_rec(obj, doc) };
        }
    } else {
        unsafe { set_document_rec(obj, doc) };
    }
}
regs_thunk!(set_document_rec_thunk, set_document_rec_entry, "ret 4");

// --- 0xB99FE0 vint_raw_input_index (cdecl (name) -> index or -1) ------------------------------------------------

/// A case-insensitive search (`_stricmp`, by address) of the raw binding names (table pointer
/// 0x252A240, stride 0x18, name pointer at +0x14, count 0x252A23C); a null name or no match gives -1.
unsafe fn raw_input_index(name: u32) -> u32 {
    unsafe {
        if name == 0 {
            return u32::MAX;
        }
        let count = rd(0x0252_A23C);
        let mut off = 0usize;
        for i in 0..count {
            let s = rd(rd(0x0252_A240) as usize + off + 0x14);
            if call_regs(STRICMP, 0, 0, 0, 0, 0, &[name, s]) == 0 {
                return i;
            }
            off += 0x18;
        }
        u32::MAX
    }
}

extern "C" fn raw_input_index_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let name = unsafe { *args };
    r.eax = checked_eax(P_RAW_INPUT_INDEX, &p, name, unsafe { raw_input_index(name) });
}
regs_thunk!(raw_input_index_thunk, raw_input_index_entry);

// --- 0xB9AF50 vint_project_find_document (ECX = name -> EAX = entry + 8 or 0) -------------------------------------

/// `crc32_lower(name)` against the first dword of the project entries (0x88 bytes from 0x22EF628,
/// count 0x252A244, signed); a hit gives `0x22EF630 + index * 0x88`.
unsafe fn project_find(name: u32) -> u32 {
    unsafe {
        let h = crc_lower(name);
        let count = rd(0x0252_A244) as i32;
        if count <= 0 {
            return 0;
        }
        let mut i = 0i32;
        let mut e = 0x022E_F628u32;
        while i < count {
            if rd(e as usize) == h {
                return 0x022E_F630u32.wrapping_add((i as u32).wrapping_mul(0x88));
            }
            i += 1;
            e = e.wrapping_add(0x88);
        }
        0
    }
}

extern "C" fn project_find_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(P_PROJECT_FIND, &p, 0, unsafe { project_find(p.ecx) });
}
regs_thunk!(project_find_thunk, project_find_entry);

// --- 0xB8B990 vint_document_clip_pop (EDX = document) ----------------------------------------------------------------

/// Depth (+0x4E4) 0 does nothing (EAX 0). Else depth -= 1; at 0 the platform slot +0x14 of
/// [0x252A1D8] is tail-called (when slot +0x10 exists); otherwise, below 8, slot +0x10 is called
/// with `document + 0x464 + depth * 16`. EAX is the call's result, the new depth, or the table.
unsafe fn clip_pop(doc: u32, ecx_in: u32) -> u32 {
    unsafe {
        let d = doc as usize;
        let n0 = rd(d + 0x4E4);
        if n0 == 0 {
            return 0;
        }
        let n = n0 - 1;
        wd(d + 0x4E4, n);
        let p = rd(0x0252_A1D8);
        if n == 0 {
            if p == 0 {
                return 0;
            }
            if rd(p as usize + 0x10) == 0 {
                return p;
            }
            let f = rd(p as usize + 0x14);
            return call_regs(f as usize, f, ecx_in, doc, 0, 0, &[]);
        }
        if n >= 8 || p == 0 {
            return n;
        }
        let f = rd(p as usize + 0x10);
        if f == 0 {
            return n;
        }
        let arg = (n << 4).wrapping_add(doc).wrapping_add(0x464);
        call_regs(f as usize, n << 4, f, arg, 0, 0, &[arg])
    }
}

extern "C" fn clip_pop_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let doc = p.edx;
    r.eax = if window_open(P_CLIP_POP) {
        let pure = unsafe {
            let n0 = rd(doc as usize + 0x4E4);
            let n = n0.wrapping_sub(1);
            let plat = rd(0x0252_A1D8);
            n0 == 0
                || (n == 0 && (plat == 0 || rd(plat as usize + 0x10) == 0))
                || (n != 0 && (n >= 8 || plat == 0 || rd(plat as usize + 0x10) == 0))
        };
        if pure {
            checked_writer(P_CLIP_POP, &p, 0, &[(doc as usize + 0x4E4, 4)], || unsafe { clip_pop(doc, p.ecx) })
        } else {
            br(P_CLIP_POP, 0);
            unsafe { clip_pop(doc, p.ecx) }
        }
    } else {
        unsafe { clip_pop(doc, p.ecx) }
    };
}
regs_thunk!(clip_pop_thunk, clip_pop_entry);

// --- the datagroup subscriber lists ------------------------------------------------------------------------------------

/// The callback with this handle in the 64-bucket table (what 0xBA0980 does, inlined by the
/// originals): 0 for a null handle or no match.
unsafe fn callback_lookup(h: u32) -> u32 {
    unsafe {
        if h == 0 {
            return 0;
        }
        let head = rd(CALLBACK_BUCKETS + (h & 0x3F) as usize * 4);
        let mut c = head;
        while c != 0 {
            if rd(c as usize + 0x18) == h {
                return c;
            }
            c = rd(c as usize + 8);
            if c == head {
                break;
            }
        }
        0
    }
}

/// The inlined `callback_release`: a refcount of 1 calls vtable[5] (ECX = callback), else -1.
unsafe fn callback_drop(c: u32, esi: u32, edi: u32) {
    unsafe {
        let rc = rd(c as usize + 0x1C);
        if rc == 1 {
            let vt = rd(c as usize);
            let f = rd(vt as usize + 0x14);
            call_regs(f as usize, vt, c, f, esi, edi, &[]);
        } else {
            wd(c as usize + 0x1C, rc.wrapping_sub(1));
        }
    }
}

/// Push a subscriber node at the back of the ring whose head pointer lives at `ring`.
unsafe fn node_push_back(ring: usize, node: u32) {
    unsafe {
        let s = node as usize;
        let rh = rd(ring);
        if rh == 0 {
            wd(ring, node);
            wd(s + 0x18, node);
            wd(s + 0x14, node);
        } else {
            let t = rd(rh as usize + 0x18);
            wd(s + 0x18, t);
            wd(s + 0x14, rh);
            let t2 = rd(rh as usize + 0x18);
            wd(t2 as usize + 0x14, node);
            wd(rh as usize + 0x18, node);
        }
    }
}

/// Unlink `node` from the subscriber ring whose head is stored at group + 0x18.
unsafe fn node_unlink(group: u32, node: u32) {
    unsafe {
        let (b, s) = (group as usize, node as usize);
        let head = rd(b + 0x18);
        let nx0 = rd(s + 0x14);
        if head == node {
            if nx0 == node {
                wd(s + 0x14, 0);
                wd(s + 0x18, 0);
                wd(b + 0x18, 0);
                return;
            }
            wd(b + 0x18, nx0);
        }
        let pv = rd(s + 0x18);
        if pv != 0 {
            wd(pv as usize + 0x14, rd(s + 0x14));
        }
        let nx = rd(s + 0x14);
        if nx != 0 {
            wd(nx as usize + 0x18, rd(s + 0x18));
        }
        wd(s + 0x14, 0);
        wd(s + 0x18, 0);
    }
}

/// 0xB93BC0 (stdcall (group)). Every subscriber node of the group (+0x18) is unlinked; a group
/// with a hash (+8) hands the nodes back to the waiting ring (node +0 = that hash), one
/// without releases each node's callback (+0xC) and returns the node to the free ring
/// 0x252A20C. Then the group's handle (+0x14) is cleared and the group goes from the live ring
/// 0x252A204 back to the free ring 0x252A200. EAX = the free ring's old head.
unsafe fn datagroup_destroy(g: u32, esi0: u32, edi0: u32) -> u32 {
    unsafe {
        let b = g as usize;
        let mut esi = rd(b + 0x18);
        while esi != 0 {
            let s = esi as usize;
            let nx = rd(s + 0x14);
            let head = rd(b + 0x18);
            let next = if nx != head { nx } else { 0 };
            node_unlink(g, esi);
            let hash = rd(b + 8);
            if hash != 0 {
                wd(s, hash);
                node_push_back(WAITING, esi);
            } else {
                let cb = callback_lookup(rd(s + 0xC));
                if cb != 0 {
                    callback_drop(cb, esi0, edi0);
                }
                node_push_back(NODE_FREE, esi);
            }
            esi = next;
        }
        let live = rd(GROUP_LIVE);
        wd(b + 0x14, 0);
        let mut fix = true;
        if live == g {
            let nx = rd(b);
            if nx == g {
                wd(GROUP_LIVE, 0);
                fix = false;
            } else {
                wd(GROUP_LIVE, nx);
            }
        }
        if fix {
            let pv = rd(b + 4);
            let nx = rd(b);
            wd(pv as usize, nx);
            let nx2 = rd(b);
            let pv2 = rd(b + 4);
            wd(nx2 as usize + 4, pv2);
        }
        let fh = rd(GROUP_FREE);
        wd(b + 4, 0);
        wd(b, 0);
        if fh == 0 {
            wd(b, g);
            wd(b + 4, g);
            wd(GROUP_FREE, g);
        } else {
            let t = rd(fh as usize + 4);
            wd(b + 4, t);
            wd(b, fh);
            let t2 = rd(fh as usize + 4);
            wd(t2 as usize, g);
            wd(fh as usize + 4, g);
        }
        fh
    }
}

extern "C" fn group_destroy_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    count_only(P_GROUP_DESTROY);
    r.eax = unsafe { datagroup_destroy(*args, p.esi, p.edi) };
}
regs_thunk!(group_destroy_thunk, group_destroy_entry, "ret 4");

/// 0xB94560: for every live datagroup (ring 0x252A204, link +0), deliver it (0xB945E0, by
/// address) and, when its byte +0x1C is 1 afterwards, destroy it. The next link is read before
/// the delivery. EAX is the last callee's, or the caller's when no group is live.
unsafe fn deliver_all(eax_in: u32) -> u32 {
    unsafe {
        let mut eax = eax_in;
        let mut edi = rd(GROUP_LIVE);
        while edi != 0 {
            eax = rd(edi as usize);
            let head = rd(GROUP_LIVE);
            let esi = if eax != head { eax } else { 0 };
            eax = call_regs(DELIVER_ONE, eax, 0, 0, esi, edi, &[edi]);
            if rb(edi as usize + 0x1C) == 1 {
                eax = call_regs(GROUP_DESTROY_FN, eax, 0, 0, esi, edi, &[edi]);
            }
            edi = esi;
        }
        eax
    }
}

extern "C" fn deliver_all_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    count_only(P_DELIVER_ALL);
    r.eax = unsafe { deliver_all(p.eax) };
}
regs_thunk!(deliver_all_thunk, deliver_all_entry);

/// 0xB93EE0 (ECX = group, EDX = owner, EDI = kind, stdcall (callback handle)): the subscriber
/// node of the group with that callback (+0xC), kind (+0x10) and owner (+8) is unlinked, its
/// callback released, and the node returned to the free ring. EAX is the group's list head when
/// nothing matched, else the free ring's old head.
unsafe fn datagroup_unsubscribe(g: u32, owner: u32, kind: u32, cb: u32, esi0: u32) -> u32 {
    unsafe {
        let head = rd(g as usize + 0x18);
        let mut esi = head;
        if esi == 0 {
            return head;
        }
        loop {
            let s = esi as usize;
            if rd(s + 0xC) == cb && rd(s + 0x10) == kind && rd(s + 8) == owner {
                break;
            }
            esi = rd(s + 0x14);
            if esi == head || esi == 0 {
                return head;
            }
        }
        node_unlink(g, esi);
        let c = callback_lookup(rd(esi as usize + 0xC));
        if c != 0 {
            callback_drop(c, esi0, kind);
        }
        let fh = rd(NODE_FREE);
        if fh == 0 {
            wd(esi as usize + 0x14, esi);
            wd(esi as usize + 0x18, esi);
            wd(NODE_FREE, esi);
            return 0;
        }
        let s = esi as usize;
        let t = rd(fh as usize + 0x18);
        wd(s + 0x18, t);
        wd(s + 0x14, fh);
        let t2 = rd(fh as usize + 0x18);
        wd(t2 as usize + 0x14, esi);
        wd(fh as usize + 0x18, esi);
        fh
    }
}

extern "C" fn group_unsubscribe_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    count_only(P_GROUP_UNSUBSCRIBE);
    r.eax = unsafe { datagroup_unsubscribe(p.ecx, p.edx, p.edi, *args, p.esi) };
}
regs_thunk!(group_unsubscribe_thunk, group_unsubscribe_entry, "ret 4");

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
        (P_SET_INT, &SET_INT_BODY, &SET_INT_BODY_CALLS, set_int_thunk),
        (P_SET_HANDLE, &SET_HANDLE_BODY, &SET_HANDLE_BODY_CALLS, set_handle_thunk),
        (P_SET_NAME, &SET_NAME_BODY, &SET_NAME_BODY_CALLS, set_name_thunk),
        (P_VALUE_ASSIGN, &ASSIGN_BODY, &ASSIGN_BODY_CALLS, value_assign_thunk),
        (P_VALUE_FREE, &FREE_BODY, &FREE_BODY_CALLS, value_free_thunk),
        (P_BITMAP_RELEASE, &BITMAP_RELEASE_BODY, &BITMAP_RELEASE_BODY_CALLS, bitmap_release_thunk),
        (P_FONT_RELEASE, &FONT_RELEASE_BODY, &FONT_RELEASE_BODY_CALLS, font_release_thunk),
        (P_SOUND_RELEASE, &SOUND_RELEASE_BODY, &SOUND_RELEASE_BODY_CALLS, sound_release_thunk),
        (P_CALLBACK_RELEASE, &CALLBACK_RELEASE_BODY, &CALLBACK_RELEASE_BODY_CALLS, callback_release_thunk),
        (P_ITEM_GET_OR_CREATE, &ITEM_GET_OR_CREATE_BODY, &ITEM_GET_OR_CREATE_BODY_CALLS, item_get_or_create_thunk),
        (P_GROUP_CREATE, &GROUP_CREATE_BODY, &GROUP_CREATE_BODY_CALLS, group_create_thunk),
        (P_BITMAP_ACQUIRE, &BITMAP_ACQUIRE_BODY, &BITMAP_ACQUIRE_BODY_CALLS, bitmap_acquire_thunk),
        (P_FONT_ACQUIRE, &FONT_ACQUIRE_BODY, &FONT_ACQUIRE_BODY_CALLS, font_acquire_thunk),
        (P_SOUND_ACQUIRE, &SOUND_ACQUIRE_BODY, &SOUND_ACQUIRE_BODY_CALLS, sound_acquire_thunk),
        (P_EXPAND, &EXPAND_BODY, &EXPAND_BODY_CALLS, expand_thunk),
        (P_REHASH, &REHASH_NAME_BODY, &REHASH_NAME_BODY_CALLS, rehash_thunk),
        (P_OBJECT_SET_NAME, &OBJECT_SET_NAME_BODY, &OBJECT_SET_NAME_BODY_CALLS, object_set_name_thunk),
        (P_SET_DOCUMENT_REC, &SET_DOCUMENT_REC_BODY, &SET_DOCUMENT_REC_BODY_CALLS, set_document_rec_thunk),
        (P_RAW_INPUT_INDEX, &RAW_INPUT_INDEX_BODY, &RAW_INPUT_INDEX_BODY_CALLS, raw_input_index_thunk),
        (P_PROJECT_FIND, &PROJECT_FIND_BODY, &PROJECT_FIND_BODY_CALLS, project_find_thunk),
        (P_CLIP_POP, &CLIP_POP_BODY, &CLIP_POP_BODY_CALLS, clip_pop_thunk),
        (P_DELIVER_ALL, &DELIVER_ALL_BODY, &DELIVER_ALL_BODY_CALLS, deliver_all_thunk),
        (P_GROUP_DESTROY, &GROUP_DESTROY_BODY, &GROUP_DESTROY_BODY_CALLS, group_destroy_thunk),
        (P_GROUP_UNSUBSCRIBE, &GROUP_UNSUBSCRIBE_BODY, &GROUP_UNSUBSCRIBE_BODY_CALLS, group_unsubscribe_thunk),
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
        .enumerate()
        .filter_map(|(idx, p)| {
            let c = p.calls.load(Ordering::Relaxed);
            let b: Vec<u32> = (0..4).map(|k| BR[idx * 4 + k].load(Ordering::Relaxed)).collect();
            let extra = if b.iter().any(|&x| x > 0) { format!(" [branches {}/{}/{}/{}]", b[0], b[1], b[2], b[3]) } else { String::new() };
            (c > 0 || !extra.is_empty()).then(|| format!("{} {c} ({}){extra}", p.name, p.mismatches.load(Ordering::Relaxed)))
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

const SET_HANDLE_BODY: [u8; 252] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x8B, 0x03, 0x83, 0xF8, 0x0B, 0x55, 0x8B, 0x6C, 0x24, 0x10, 0x56,
    0x57, 0x0F, 0x87, 0xAB, 0x00, 0x00, 0x00, 0xFF, 0x24, 0x85, 0x5C, 0x35, 0xB9, 0x00, 0x5F, 0x5E,
    0x89, 0x6B, 0x04, 0x5D, 0xC7, 0x03, 0x02, 0x00, 0x00, 0x00, 0x8B, 0xC3, 0x5B, 0xC2, 0x08, 0x00,
    0x85, 0xED, 0x89, 0x6C, 0x24, 0x18, 0xDB, 0x44, 0x24, 0x18, 0x7D, 0x06, 0xD8, 0x05, 0xD8, 0xBF,
    0xE5, 0x00, 0x5F, 0xD9, 0x5B, 0x04, 0x5E, 0x5D, 0x8B, 0xC3, 0x5B, 0xC2, 0x08, 0x00, 0x8B, 0xCD,
    0xE8, 0x6B, 0x89, 0x00, 0x00, 0x8B, 0xF8, 0x85, 0xFF, 0x74, 0x60, 0x8D, 0x73, 0x04, 0xE8, 0x6D,
    0xF8, 0xFF, 0xFF, 0x83, 0x47, 0x18, 0x01, 0x5F, 0x89, 0x2E, 0x5E, 0x5D, 0x8B, 0xC3, 0x5B, 0xC2,
    0x08, 0x00, 0x8B, 0xCD, 0xE8, 0x57, 0x83, 0x00, 0x00, 0x8B, 0xF0, 0x85, 0xF6, 0x74, 0x3C, 0x8D,
    0x7B, 0x04, 0xE8, 0xB9, 0xF8, 0xFF, 0xFF, 0x83, 0x46, 0x10, 0x01, 0x89, 0x2F, 0x5F, 0x5E, 0x5D,
    0x8B, 0xC3, 0x5B, 0xC2, 0x08, 0x00, 0x8B, 0xCD, 0xE8, 0x13, 0xD0, 0x00, 0x00, 0x8B, 0xF0, 0x85,
    0xF6, 0x74, 0x18, 0x8B, 0x4B, 0x04, 0xE8, 0x15, 0xF9, 0xFF, 0xFF, 0x83, 0x46, 0x0C, 0x01, 0x5F,
    0x5E, 0x89, 0x6B, 0x04, 0x5D, 0x8B, 0xC3, 0x5B, 0xC2, 0x08, 0x00, 0xC7, 0x43, 0x04, 0x00, 0x00,
    0x00, 0x00, 0x5F, 0x5E, 0x5D, 0x8B, 0xC3, 0x5B, 0xC2, 0x08, 0x00, 0x90, 0xAE, 0x34, 0xB9, 0x00,
    0x3F, 0x35, 0xB9, 0x00, 0xAE, 0x34, 0xB9, 0x00, 0xC0, 0x34, 0xB9, 0x00, 0x52, 0x35, 0xB9, 0x00,
    0x52, 0x35, 0xB9, 0x00, 0x52, 0x35, 0xB9, 0x00, 0x52, 0x35, 0xB9, 0x00, 0x52, 0x35, 0xB9, 0x00,
    0xDE, 0x34, 0xB9, 0x00, 0x02, 0x35, 0xB9, 0x00, 0x26, 0x35, 0xB9, 0x00,
];
const SET_HANDLE_BODY_CALLS: [usize; 6] = [0x50, 0x5e, 0x74, 0x82, 0x98, 0xa6];
const SET_NAME_BODY: [u8; 288] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x8B, 0x03, 0x83, 0xF8, 0x0B, 0x55, 0x8B, 0x6C, 0x24, 0x10, 0x56,
    0x57, 0x0F, 0x87, 0xCD, 0x00, 0x00, 0x00, 0xFF, 0x24, 0x85, 0xF0, 0x38, 0xB9, 0x00, 0xC7, 0x03,
    0x04, 0x00, 0x00, 0x00, 0x5F, 0x5E, 0x89, 0x6B, 0x04, 0x5D, 0x8B, 0xC3, 0x5B, 0xC2, 0x08, 0x00,
    0x8D, 0x7B, 0x04, 0xE8, 0x98, 0xF5, 0xFF, 0xFF, 0x55, 0xE8, 0xB2, 0x7E, 0x00, 0x00, 0x83, 0xC4,
    0x04, 0x85, 0xC0, 0x74, 0x0E, 0x8B, 0x40, 0x08, 0x89, 0x07, 0x5F, 0x5E, 0x5D, 0x8B, 0xC3, 0x5B,
    0xC2, 0x08, 0x00, 0xC7, 0x07, 0x00, 0x00, 0x00, 0x00, 0x5F, 0x5E, 0x5D, 0x8B, 0xC3, 0x5B, 0xC2,
    0x08, 0x00, 0x8D, 0x73, 0x04, 0xE8, 0xF6, 0xF4, 0xFF, 0xFF, 0x55, 0xE8, 0xB0, 0x84, 0x00, 0x00,
    0x83, 0xC4, 0x04, 0x85, 0xC0, 0x74, 0x0E, 0x8B, 0x48, 0x14, 0x5F, 0x89, 0x0E, 0x5E, 0x5D, 0x8B,
    0xC3, 0x5B, 0xC2, 0x08, 0x00, 0x5F, 0xC7, 0x06, 0x00, 0x00, 0x00, 0x00, 0x5E, 0x5D, 0x8B, 0xC3,
    0x5B, 0xC2, 0x08, 0x00, 0x8B, 0x4B, 0x04, 0xE8, 0xB4, 0xF5, 0xFF, 0xFF, 0x55, 0xE8, 0x4E, 0xCB,
    0x00, 0x00, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x74, 0x0F, 0x8B, 0x50, 0x08, 0x5F, 0x5E, 0x5D, 0x89,
    0x53, 0x04, 0x8B, 0xC3, 0x5B, 0xC2, 0x08, 0x00, 0x5F, 0x5E, 0x5D, 0xC7, 0x43, 0x04, 0x00, 0x00,
    0x00, 0x00, 0x8B, 0xC3, 0x5B, 0xC2, 0x08, 0x00, 0x8B, 0xC5, 0xE8, 0x61, 0xE8, 0xFF, 0xFF, 0x85,
    0xC0, 0x74, 0x11, 0x8B, 0x08, 0x89, 0x4B, 0x04, 0x8B, 0x50, 0x04, 0x89, 0x53, 0x08, 0x8B, 0x40,
    0x08, 0x89, 0x43, 0x0C, 0x5F, 0x5E, 0x5D, 0x8B, 0xC3, 0x5B, 0xC2, 0x08, 0x00, 0x8D, 0x49, 0x00,
    0x1E, 0x38, 0xB9, 0x00, 0xE4, 0x38, 0xB9, 0x00, 0xE4, 0x38, 0xB9, 0x00, 0xE4, 0x38, 0xB9, 0x00,
    0x24, 0x38, 0xB9, 0x00, 0xE4, 0x38, 0xB9, 0x00, 0xC8, 0x38, 0xB9, 0x00, 0xE4, 0x38, 0xB9, 0x00,
    0xE4, 0x38, 0xB9, 0x00, 0x62, 0x38, 0xB9, 0x00, 0x30, 0x38, 0xB9, 0x00, 0x94, 0x38, 0xB9, 0x00,
];
const SET_NAME_BODY_CALLS: [usize; 7] = [0x33, 0x39, 0x65, 0x6b, 0x97, 0x9d, 0xca];
const ASSIGN_BODY: [u8; 236] = [
    0x8B, 0x06, 0x85, 0xC0, 0x74, 0x08, 0x39, 0x07, 0x0F, 0x85, 0xA9, 0x00, 0x00, 0x00, 0x8B, 0xC6,
    0xE8, 0x5B, 0x01, 0x00, 0x00, 0x8B, 0x07, 0x89, 0x06, 0x83, 0xC0, 0xFF, 0x83, 0xF8, 0x0B, 0x0F,
    0x87, 0x92, 0x00, 0x00, 0x00, 0xFF, 0x24, 0x85, 0xDC, 0x36, 0xB9, 0x00, 0x8B, 0x47, 0x04, 0x89,
    0x46, 0x04, 0x8B, 0xC6, 0xC3, 0x8B, 0x4F, 0x04, 0x89, 0x4E, 0x04, 0xE8, 0xD0, 0xF6, 0xFF, 0xFF,
    0x8B, 0xC6, 0xC3, 0x8B, 0x4F, 0x04, 0x89, 0x4E, 0x04, 0xE8, 0x32, 0xF7, 0xFF, 0xFF, 0x8B, 0xC6,
    0xC3, 0x8B, 0x4F, 0x04, 0x89, 0x4E, 0x04, 0xE8, 0xA4, 0xF7, 0xFF, 0xFF, 0x8B, 0xC6, 0xC3, 0x8B,
    0x57, 0x04, 0xE8, 0xF9, 0xD2, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x04, 0x83, 0x40, 0x1C, 0x01, 0x8B,
    0x4F, 0x04, 0x89, 0x4E, 0x04, 0x8B, 0xC6, 0xC3, 0x8B, 0x57, 0x04, 0x89, 0x56, 0x04, 0x8B, 0xC6,
    0xC3, 0xD9, 0x47, 0x04, 0x8B, 0xC6, 0xD9, 0x5E, 0x04, 0xC3, 0x8A, 0x4F, 0x04, 0x88, 0x4E, 0x04,
    0x8B, 0xC6, 0xC3, 0x8B, 0x57, 0x04, 0x8D, 0x47, 0x04, 0x89, 0x56, 0x04, 0x8B, 0x48, 0x04, 0x89,
    0x4E, 0x08, 0x8B, 0x50, 0x08, 0x89, 0x56, 0x0C, 0x8B, 0xC6, 0xC3, 0x8B, 0x47, 0x04, 0x89, 0x46,
    0x04, 0x8B, 0x4F, 0x08, 0x89, 0x4E, 0x08, 0x8B, 0xC6, 0xC3, 0x8B, 0xFF, 0x4C, 0x36, 0xB9, 0x00,
    0x98, 0x36, 0xB9, 0x00, 0xA1, 0x36, 0xB9, 0x00, 0x4C, 0x36, 0xB9, 0x00, 0xAA, 0x36, 0xB9, 0x00,
    0xB3, 0x36, 0xB9, 0x00, 0xCB, 0x36, 0xB9, 0x00, 0x7F, 0x36, 0xB9, 0x00, 0x55, 0x36, 0xB9, 0x00,
    0x63, 0x36, 0xB9, 0x00, 0x71, 0x36, 0xB9, 0x00, 0x4C, 0x36, 0xB9, 0x00,
];
const ASSIGN_BODY_CALLS: [usize; 5] = [0x10, 0x3b, 0x49, 0x57, 0x62];
const FREE_BODY: [u8; 108] = [
    0x56, 0x8B, 0xF0, 0x8B, 0x06, 0x83, 0xC0, 0xF8, 0x83, 0xF8, 0x03, 0x77, 0x15, 0xFF, 0x24, 0x85,
    0xEC, 0x37, 0xB9, 0x00, 0x83, 0xC6, 0x04, 0xE8, 0xB4, 0xF5, 0xFF, 0xFF, 0xC7, 0x06, 0x00, 0x00,
    0x00, 0x00, 0x5E, 0xC3, 0x57, 0x8D, 0x7E, 0x04, 0xE8, 0x13, 0xF6, 0xFF, 0xFF, 0xC7, 0x07, 0x00,
    0x00, 0x00, 0x00, 0x5F, 0x5E, 0xC3, 0x8B, 0x4E, 0x04, 0xE8, 0x82, 0xF6, 0xFF, 0xFF, 0xC7, 0x46,
    0x04, 0x00, 0x00, 0x00, 0x00, 0x5E, 0xC3, 0x8B, 0x56, 0x04, 0xE8, 0xA1, 0xD1, 0x00, 0x00, 0x85,
    0xC0, 0x74, 0xCF, 0x8B, 0xC8, 0x5E, 0xE9, 0x65, 0xD0, 0x00, 0x00, 0x90, 0xD7, 0x37, 0xB9, 0x00,
    0xA4, 0x37, 0xB9, 0x00, 0xB4, 0x37, 0xB9, 0x00, 0xC6, 0x37, 0xB9, 0x00,
];
const FREE_BODY_CALLS: [usize; 4] = [0x17, 0x28, 0x39, 0x4a];
const BITMAP_RELEASE_BODY: [u8; 50] = [
    0x8B, 0x06, 0x85, 0xC0, 0x74, 0x25, 0x0F, 0xB7, 0xC8, 0x81, 0xF9, 0xF0, 0x00, 0x00, 0x00, 0x73,
    0x1A, 0x8D, 0x0C, 0xC9, 0x39, 0x04, 0x8D, 0x2C, 0xC5, 0x1B, 0x03, 0x8D, 0x0C, 0x8D, 0x18, 0xC5,
    0x1B, 0x03, 0x75, 0x07, 0x8B, 0x01, 0x8B, 0x50, 0x04, 0xFF, 0xD2, 0xC7, 0x06, 0x00, 0x00, 0x00,
    0x00, 0xC3,
];
const BITMAP_RELEASE_BODY_CALLS: [usize; 0] = [];
const FONT_RELEASE_BODY: [u8; 54] = [
    0x8B, 0x07, 0x85, 0xC0, 0x74, 0x29, 0x66, 0x3D, 0x0A, 0x00, 0x73, 0x23, 0x0F, 0xB7, 0xC8, 0x56,
    0x8D, 0x34, 0xCD, 0x00, 0x00, 0x00, 0x00, 0x2B, 0xF1, 0x39, 0x04, 0xB5, 0x08, 0xC4, 0x1B, 0x03,
    0x8D, 0x34, 0xB5, 0x00, 0xC4, 0x1B, 0x03, 0x75, 0x05, 0xE8, 0xC2, 0x8A, 0x00, 0x00, 0x5E, 0xC7,
    0x07, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const FONT_RELEASE_BODY_CALLS: [usize; 1] = [0x29];
const SOUND_RELEASE_BODY: [u8; 39] = [
    0x85, 0xC9, 0x74, 0x22, 0x0F, 0xB7, 0xC1, 0x83, 0xF8, 0x32, 0x73, 0x1A, 0x56, 0x8D, 0x34, 0x40,
    0x39, 0x0C, 0xF5, 0x08, 0xE7, 0x1B, 0x03, 0x8D, 0x34, 0xF5, 0x00, 0xE7, 0x1B, 0x03, 0x75, 0x05,
    0xE8, 0xFB, 0xD6, 0x00, 0x00, 0x5E, 0xC3,
];
const SOUND_RELEASE_BODY_CALLS: [usize; 1] = [0x20];
const CALLBACK_RELEASE_BODY: [u8; 22] = [
    0x8B, 0x41, 0x1C, 0x83, 0xF8, 0x01, 0x75, 0x07, 0x8B, 0x01, 0x8B, 0x50, 0x14, 0xFF, 0xE2, 0x83,
    0xC0, 0xFF, 0x89, 0x41, 0x1C, 0xC3,
];
const CALLBACK_RELEASE_BODY_CALLS: [usize; 0] = [];
const ITEM_GET_OR_CREATE_BODY: [u8; 306] = [
    0x55, 0x57, 0x8B, 0xF8, 0x33, 0xED, 0x3B, 0xFD, 0x75, 0x05, 0x5F, 0x33, 0xC0, 0x5D, 0xC3, 0x8B,
    0xCF, 0xE8, 0xEA, 0x01, 0x00, 0x00, 0x85, 0xC0, 0x75, 0xF0, 0x56, 0x8B, 0x35, 0xF8, 0xA1, 0x52,
    0x02, 0x3B, 0xF5, 0x0F, 0x84, 0x03, 0x01, 0x00, 0x00, 0x8B, 0x46, 0x0C, 0x3B, 0xC6, 0x75, 0x08,
    0x89, 0x2D, 0xF8, 0xA1, 0x52, 0x02, 0xEB, 0x14, 0x8B, 0x4E, 0x08, 0x89, 0x41, 0x0C, 0x8B, 0x56,
    0x0C, 0xA3, 0xF8, 0xA1, 0x52, 0x02, 0x8B, 0x46, 0x08, 0x89, 0x42, 0x08, 0xA1, 0xF4, 0xA1, 0x52,
    0x02, 0x3B, 0xC5, 0x89, 0x6E, 0x08, 0x89, 0x6E, 0x0C, 0x75, 0x0E, 0x89, 0x76, 0x0C, 0x89, 0x76,
    0x08, 0x89, 0x35, 0xF4, 0xA1, 0x52, 0x02, 0xEB, 0x12, 0x8B, 0x48, 0x08, 0x89, 0x4E, 0x08, 0x89,
    0x46, 0x0C, 0x8B, 0x50, 0x08, 0x89, 0x72, 0x0C, 0x89, 0x70, 0x08, 0x53, 0x8B, 0xCF, 0xE8, 0xAD,
    0xF8, 0xFF, 0xFF, 0x8B, 0x1D, 0x10, 0xA2, 0x52, 0x02, 0x8B, 0xC3, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x3B, 0xC5, 0x0F, 0x84, 0x8D, 0x00, 0x00, 0x00, 0x8B, 0x50, 0x14, 0x8B, 0x78, 0x04, 0x8B, 0xCA,
    0x2B, 0xCB, 0xF7, 0xD9, 0x1B, 0xC9, 0x23, 0xCA, 0x3B, 0x7E, 0x10, 0x75, 0x71, 0x3B, 0xD8, 0x75,
    0x0A, 0x3B, 0xD0, 0x75, 0x04, 0x33, 0xDB, 0xEB, 0x19, 0x8B, 0xDA, 0x8B, 0x78, 0x18, 0x3B, 0xFD,
    0x74, 0x03, 0x89, 0x57, 0x14, 0x8B, 0x50, 0x14, 0x3B, 0xD5, 0x74, 0x06, 0x8B, 0x78, 0x18, 0x89,
    0x7A, 0x18, 0x89, 0x68, 0x18, 0x89, 0x68, 0x14, 0x8B, 0x96, 0x24, 0x02, 0x00, 0x00, 0x89, 0x10,
    0x8B, 0x96, 0x20, 0x02, 0x00, 0x00, 0x3B, 0xD5, 0x75, 0x10, 0x89, 0x40, 0x14, 0x89, 0x40, 0x18,
    0x89, 0x86, 0x20, 0x02, 0x00, 0x00, 0x8B, 0xC1, 0xEB, 0x96, 0x8B, 0x52, 0x18, 0x89, 0x50, 0x18,
    0x8B, 0x96, 0x20, 0x02, 0x00, 0x00, 0x89, 0x50, 0x14, 0x8B, 0x96, 0x20, 0x02, 0x00, 0x00, 0x8B,
    0x52, 0x18, 0x89, 0x42, 0x14, 0x8B, 0x96, 0x20, 0x02, 0x00, 0x00, 0x89, 0x42, 0x18, 0x8B, 0xC1,
    0xE9, 0x6B, 0xFF, 0xFF, 0xFF, 0x89, 0x1D, 0x10, 0xA2, 0x52, 0x02, 0x5B, 0x8B, 0xC6, 0x5E, 0x5F,
    0x5D, 0xC3,
];
const ITEM_GET_OR_CREATE_BODY_CALLS: [usize; 2] = [0x11, 0x7e];
const GROUP_CREATE_BODY: [u8; 248] = [
    0x55, 0x33, 0xED, 0x3B, 0xCD, 0x75, 0x04, 0x33, 0xC0, 0x5D, 0xC3, 0x53, 0x56, 0x8B, 0x35, 0x00,
    0xA2, 0x52, 0x02, 0x3B, 0xF5, 0x57, 0x74, 0x50, 0x8B, 0x06, 0x3B, 0xC6, 0x75, 0x08, 0x89, 0x2D,
    0x00, 0xA2, 0x52, 0x02, 0xEB, 0x12, 0x8B, 0x56, 0x04, 0x89, 0x02, 0x8B, 0x56, 0x04, 0xA3, 0x00,
    0xA2, 0x52, 0x02, 0x8B, 0x06, 0x89, 0x50, 0x04, 0xA1, 0x04, 0xA2, 0x52, 0x02, 0x3B, 0xC5, 0x89,
    0x6E, 0x04, 0x89, 0x2E, 0x75, 0x0D, 0x89, 0x36, 0x89, 0x76, 0x04, 0x89, 0x35, 0x04, 0xA2, 0x52,
    0x02, 0xEB, 0x10, 0x8B, 0x50, 0x04, 0x89, 0x56, 0x04, 0x89, 0x06, 0x8B, 0x50, 0x04, 0x89, 0x32,
    0x89, 0x70, 0x04, 0xE8, 0x58, 0xF9, 0xFF, 0xFF, 0x8B, 0x1D, 0x10, 0xA2, 0x52, 0x02, 0x8B, 0xC3,
    0x3B, 0xC5, 0x74, 0x77, 0x8B, 0x50, 0x14, 0x8B, 0x38, 0x8B, 0xCA, 0x2B, 0xCB, 0xF7, 0xD9, 0x1B,
    0xC9, 0x23, 0xCA, 0x3B, 0x7E, 0x08, 0x75, 0x5F, 0x3B, 0xD8, 0x75, 0x0A, 0x3B, 0xD0, 0x75, 0x04,
    0x33, 0xDB, 0xEB, 0x19, 0x8B, 0xDA, 0x8B, 0x78, 0x18, 0x3B, 0xFD, 0x74, 0x03, 0x89, 0x57, 0x14,
    0x8B, 0x50, 0x14, 0x3B, 0xD5, 0x74, 0x06, 0x8B, 0x78, 0x18, 0x89, 0x7A, 0x18, 0x89, 0x68, 0x18,
    0x89, 0x68, 0x14, 0x8B, 0x56, 0x14, 0x89, 0x10, 0x8B, 0x56, 0x18, 0x3B, 0xD5, 0x75, 0x0D, 0x89,
    0x40, 0x14, 0x89, 0x40, 0x18, 0x89, 0x46, 0x18, 0x8B, 0xC1, 0xEB, 0xA4, 0x8B, 0x52, 0x18, 0x89,
    0x50, 0x18, 0x8B, 0x56, 0x18, 0x89, 0x50, 0x14, 0x8B, 0x56, 0x18, 0x8B, 0x52, 0x18, 0x89, 0x42,
    0x14, 0x8B, 0x56, 0x18, 0x89, 0x42, 0x18, 0x8B, 0xC1, 0xEB, 0x85, 0x5F, 0x8B, 0xC6, 0x5E, 0x89,
    0x1D, 0x10, 0xA2, 0x52, 0x02, 0x5B, 0x5D, 0xC3,
];
const GROUP_CREATE_BODY_CALLS: [usize; 1] = [0x63];
const FONT_ACQUIRE_BODY: [u8; 368] = [
    0x55, 0x8B, 0x6C, 0x24, 0x08, 0x85, 0xED, 0x75, 0x04, 0x33, 0xC0, 0x5D, 0xC3, 0x8B, 0xCD, 0xE8,
    0xAC, 0x12, 0x04, 0x00, 0x8B, 0x0D, 0x58, 0xA2, 0x52, 0x02, 0x8B, 0xD0, 0x8B, 0xC1, 0x85, 0xC0,
    0x74, 0x10, 0x39, 0x50, 0x04, 0x74, 0x60, 0x8B, 0x40, 0x14, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0,
    0x75, 0xF0, 0x56, 0x8B, 0x35, 0x5C, 0xA2, 0x52, 0x02, 0x85, 0xF6, 0x0F, 0x84, 0x2A, 0x01, 0x00,
    0x00, 0xA1, 0xD4, 0xA1, 0x52, 0x02, 0x85, 0xC0, 0x0F, 0x84, 0x1D, 0x01, 0x00, 0x00, 0x8B, 0x40,
    0x10, 0x85, 0xC0, 0x0F, 0x84, 0x12, 0x01, 0x00, 0x00, 0x53, 0x57, 0x55, 0xFF, 0xD0, 0x8B, 0xD8,
    0xA1, 0xD4, 0xA1, 0x52, 0x02, 0x8B, 0x40, 0x24, 0x83, 0xC4, 0x04, 0x85, 0xC0, 0x74, 0x1E, 0x8B,
    0x0E, 0x51, 0x8D, 0x7E, 0x0C, 0x57, 0xFF, 0xD0, 0x83, 0xC4, 0x08, 0x84, 0xC0, 0x75, 0x15, 0xC7,
    0x07, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x0D, 0x83, 0x40, 0x10, 0x01, 0x5D, 0xC3, 0xC7, 0x46, 0x0C,
    0x00, 0x00, 0x00, 0x00, 0x83, 0xFB, 0xFF, 0x75, 0x07, 0x5F, 0x5B, 0x5E, 0x33, 0xC0, 0x5D, 0xC3,
    0x66, 0x8B, 0x3D, 0x60, 0xA2, 0x52, 0x02, 0x8B, 0xCE, 0x81, 0xE9, 0x00, 0xC4, 0x1B, 0x03, 0xB8,
    0x93, 0x24, 0x49, 0x92, 0xF7, 0xE9, 0x03, 0xD1, 0xC1, 0xFA, 0x04, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F,
    0x03, 0xC2, 0x66, 0x83, 0xC7, 0x01, 0x0F, 0xB7, 0xD7, 0x0F, 0xB7, 0xC8, 0xC1, 0xE2, 0x10, 0x03,
    0xCA, 0x89, 0x4E, 0x08, 0x8B, 0xCD, 0x66, 0x89, 0x3D, 0x60, 0xA2, 0x52, 0x02, 0x89, 0x1E, 0xE8,
    0xDC, 0x11, 0x04, 0x00, 0x39, 0x35, 0x5C, 0xA2, 0x52, 0x02, 0x89, 0x46, 0x04, 0xC7, 0x46, 0x10,
    0x01, 0x00, 0x00, 0x00, 0x75, 0x1C, 0x8B, 0x46, 0x14, 0x3B, 0xC6, 0x75, 0x10, 0x33, 0xC9, 0x89,
    0x4E, 0x14, 0x89, 0x4E, 0x18, 0x89, 0x0D, 0x5C, 0xA2, 0x52, 0x02, 0xEB, 0x29, 0xA3, 0x5C, 0xA2,
    0x52, 0x02, 0x8B, 0x46, 0x18, 0x33, 0xC9, 0x3B, 0xC1, 0x74, 0x06, 0x8B, 0x56, 0x14, 0x89, 0x50,
    0x14, 0x8B, 0x46, 0x14, 0x3B, 0xC1, 0x74, 0x06, 0x8B, 0x56, 0x18, 0x89, 0x50, 0x18, 0x89, 0x4E,
    0x14, 0x89, 0x4E, 0x18, 0x33, 0xC9, 0xA1, 0x58, 0xA2, 0x52, 0x02, 0x3B, 0xC1, 0x75, 0x13, 0x5F,
    0x5B, 0x89, 0x76, 0x14, 0x89, 0x76, 0x18, 0x89, 0x35, 0x58, 0xA2, 0x52, 0x02, 0x8B, 0xC6, 0x5E,
    0x5D, 0xC3, 0x8B, 0x48, 0x18, 0x89, 0x46, 0x14, 0x89, 0x4E, 0x18, 0x8B, 0x50, 0x18, 0x5F, 0x89,
    0x72, 0x14, 0x89, 0x70, 0x18, 0x5B, 0x8B, 0xC6, 0x5E, 0x5D, 0xC3, 0x5E, 0x33, 0xC0, 0x5D, 0xC3,
];
const FONT_ACQUIRE_BODY_CALLS: [usize; 2] = [0xf, 0xdf];
const BITMAP_ACQUIRE_BODY: [u8; 298] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x55, 0x33, 0xED, 0x3B, 0xDD, 0x75, 0x05, 0x5D, 0x33, 0xC0, 0x5B,
    0xC3, 0x56, 0x57, 0x8D, 0x44, 0x24, 0x14, 0x50, 0x33, 0xC0, 0x8B, 0xCB, 0xE8, 0xEF, 0x0D, 0x04,
    0x00, 0x8D, 0x7C, 0x24, 0x14, 0xE8, 0x76, 0xFE, 0xFF, 0xFF, 0x8B, 0xF0, 0x3B, 0xF5, 0x0F, 0x85,
    0xCF, 0x00, 0x00, 0x00, 0x8B, 0x35, 0x68, 0xA2, 0x52, 0x02, 0x3B, 0xF5, 0x0F, 0x84, 0xE1, 0x00,
    0x00, 0x00, 0xA1, 0xD4, 0xA1, 0x52, 0x02, 0x3B, 0xC5, 0x0F, 0x84, 0xD4, 0x00, 0x00, 0x00, 0x8B,
    0x00, 0x3B, 0xC5, 0x0F, 0x84, 0xCA, 0x00, 0x00, 0x00, 0x53, 0xFF, 0xD0, 0x83, 0xC4, 0x04, 0x83,
    0xF8, 0xFF, 0x89, 0x46, 0x04, 0x0F, 0x84, 0xB8, 0x00, 0x00, 0x00, 0x66, 0x8B, 0x3D, 0x6C, 0xA2,
    0x52, 0x02, 0x8B, 0xCE, 0x81, 0xE9, 0x18, 0xC5, 0x1B, 0x03, 0xB8, 0x39, 0x8E, 0xE3, 0x38, 0xF7,
    0xE9, 0x8B, 0x44, 0x24, 0x14, 0xC1, 0xFA, 0x03, 0x8B, 0xCA, 0xC1, 0xE9, 0x1F, 0x66, 0x83, 0xC7,
    0x01, 0x03, 0xCA, 0x0F, 0xB7, 0xD7, 0xC1, 0xE2, 0x10, 0x03, 0xCA, 0x39, 0x35, 0x68, 0xA2, 0x52,
    0x02, 0x89, 0x6E, 0x0C, 0x66, 0x89, 0x3D, 0x6C, 0xA2, 0x52, 0x02, 0x89, 0x4E, 0x14, 0x89, 0x46,
    0x10, 0xC7, 0x46, 0x18, 0x01, 0x00, 0x00, 0x00, 0x75, 0x14, 0x8B, 0x46, 0x1C, 0x3B, 0xC6, 0x75,
    0x08, 0x89, 0x2D, 0x68, 0xA2, 0x52, 0x02, 0xEB, 0x1F, 0xA3, 0x68, 0xA2, 0x52, 0x02, 0x8B, 0x46,
    0x20, 0x3B, 0xC5, 0x74, 0x06, 0x8B, 0x4E, 0x1C, 0x89, 0x48, 0x1C, 0x8B, 0x46, 0x1C, 0x3B, 0xC5,
    0x74, 0x06, 0x8B, 0x56, 0x20, 0x89, 0x50, 0x20, 0xA1, 0x64, 0xA2, 0x52, 0x02, 0x3B, 0xC5, 0x89,
    0x6E, 0x20, 0x89, 0x6E, 0x1C, 0x75, 0x13, 0x89, 0x76, 0x1C, 0x89, 0x76, 0x20, 0x89, 0x35, 0x64,
    0xA2, 0x52, 0x02, 0x5F, 0x8B, 0xC6, 0x5E, 0x5D, 0x5B, 0xC3, 0x8B, 0x48, 0x20, 0x89, 0x46, 0x1C,
    0x89, 0x4E, 0x20, 0x8B, 0x50, 0x20, 0x89, 0x72, 0x1C, 0x5F, 0x89, 0x70, 0x20, 0x8B, 0xC6, 0x5E,
    0x5D, 0x5B, 0xC3, 0x5F, 0x5E, 0x5D, 0x33, 0xC0, 0x5B, 0xC3,
];
const BITMAP_ACQUIRE_BODY_CALLS: [usize; 2] = [0x1c, 0x25];
const SOUND_ACQUIRE_BODY: [u8; 325] = [
    0x55, 0x8B, 0x6C, 0x24, 0x08, 0x85, 0xED, 0x75, 0x04, 0x33, 0xC0, 0x5D, 0xC3, 0x8B, 0xCD, 0xE8,
    0xAC, 0xC5, 0x03, 0x00, 0x8B, 0x0D, 0x98, 0xA2, 0x52, 0x02, 0x8B, 0xD0, 0x8B, 0xC1, 0x85, 0xC0,
    0x74, 0x10, 0x39, 0x50, 0x04, 0x74, 0x49, 0x8B, 0x40, 0x10, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0,
    0x75, 0xF0, 0x56, 0x8B, 0x35, 0x9C, 0xA2, 0x52, 0x02, 0x85, 0xF6, 0x0F, 0x84, 0xFF, 0x00, 0x00,
    0x00, 0xA1, 0xD4, 0xA1, 0x52, 0x02, 0x85, 0xC0, 0x0F, 0x84, 0xF2, 0x00, 0x00, 0x00, 0x8B, 0x40,
    0x2C, 0x85, 0xC0, 0x0F, 0x84, 0xE7, 0x00, 0x00, 0x00, 0x53, 0x55, 0xFF, 0xD0, 0x0F, 0xB7, 0xD8,
    0x83, 0xC4, 0x04, 0x66, 0x81, 0xFB, 0xFF, 0xFF, 0x75, 0x0C, 0x5B, 0x5E, 0x33, 0xC0, 0x5D, 0xC3,
    0x83, 0x40, 0x0C, 0x01, 0x5D, 0xC3, 0x8B, 0xCE, 0x81, 0xE9, 0x00, 0xE7, 0x1B, 0x03, 0xB8, 0xAB,
    0xAA, 0xAA, 0x2A, 0xF7, 0xE9, 0xC1, 0xFA, 0x02, 0x8B, 0xC2, 0x57, 0x66, 0x8B, 0x3D, 0xA0, 0xA2,
    0x52, 0x02, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x66, 0x83, 0xC7, 0x01, 0x0F, 0xB7, 0xD7, 0x0F, 0xB7,
    0xC8, 0xC1, 0xE2, 0x10, 0x03, 0xCA, 0x89, 0x4E, 0x08, 0x8B, 0xCD, 0x66, 0x89, 0x3D, 0xA0, 0xA2,
    0x52, 0x02, 0x66, 0x89, 0x1E, 0xE8, 0x06, 0xC5, 0x03, 0x00, 0x39, 0x35, 0x9C, 0xA2, 0x52, 0x02,
    0x89, 0x46, 0x04, 0xC7, 0x46, 0x0C, 0x01, 0x00, 0x00, 0x00, 0x5F, 0x75, 0x1C, 0x8B, 0x46, 0x10,
    0x3B, 0xC6, 0x75, 0x10, 0x33, 0xC9, 0x89, 0x4E, 0x10, 0x89, 0x4E, 0x14, 0x89, 0x0D, 0x9C, 0xA2,
    0x52, 0x02, 0xEB, 0x29, 0xA3, 0x9C, 0xA2, 0x52, 0x02, 0x8B, 0x46, 0x14, 0x33, 0xC9, 0x3B, 0xC1,
    0x74, 0x06, 0x8B, 0x56, 0x10, 0x89, 0x50, 0x10, 0x8B, 0x46, 0x10, 0x3B, 0xC1, 0x74, 0x06, 0x8B,
    0x56, 0x14, 0x89, 0x50, 0x14, 0x89, 0x4E, 0x10, 0x89, 0x4E, 0x14, 0x33, 0xC9, 0xA1, 0x98, 0xA2,
    0x52, 0x02, 0x3B, 0xC1, 0x75, 0x12, 0x5B, 0x89, 0x76, 0x10, 0x89, 0x76, 0x14, 0x89, 0x35, 0x98,
    0xA2, 0x52, 0x02, 0x8B, 0xC6, 0x5E, 0x5D, 0xC3, 0x8B, 0x48, 0x14, 0x89, 0x46, 0x10, 0x89, 0x4E,
    0x14, 0x8B, 0x50, 0x14, 0x89, 0x72, 0x10, 0x89, 0x70, 0x14, 0x5B, 0x8B, 0xC6, 0x5E, 0x5D, 0xC3,
    0x5E, 0x33, 0xC0, 0x5D, 0xC3,
];
const SOUND_ACQUIRE_BODY_CALLS: [usize; 2] = [0xf, 0xb5];
const EXPAND_BODY: [u8; 1505] = [
    0xB8, 0x28, 0x30, 0x00, 0x00, 0xE8, 0x46, 0xE1, 0x10, 0x00, 0x8B, 0x8C, 0x24, 0x34, 0x30, 0x00,
    0x00, 0x8B, 0x84, 0x24, 0x30, 0x30, 0x00, 0x00, 0x53, 0x8B, 0x9C, 0x24, 0x30, 0x30, 0x00, 0x00,
    0x85, 0xDB, 0x55, 0x56, 0x57, 0x0F, 0x84, 0xA9, 0x05, 0x00, 0x00, 0x85, 0xC0, 0x0F, 0x84, 0xA1,
    0x05, 0x00, 0x00, 0x85, 0xC9, 0x0F, 0x86, 0x99, 0x05, 0x00, 0x00, 0x33, 0xF6, 0x85, 0xC9, 0x8B,
    0xF8, 0x89, 0x5C, 0x24, 0x1C, 0x89, 0x74, 0x24, 0x20, 0xC6, 0x44, 0x24, 0x17, 0x00, 0x0F, 0x86,
    0x5A, 0x05, 0x00, 0x00, 0x0F, 0xB7, 0x07, 0x33, 0xED, 0x66, 0x3D, 0x01, 0x00, 0x75, 0x0C, 0x80,
    0x7C, 0x24, 0x17, 0x00, 0x0F, 0x94, 0xC1, 0x88, 0x4C, 0x24, 0x17, 0x80, 0x7C, 0x24, 0x17, 0x01,
    0x75, 0x1C, 0x66, 0x89, 0x03, 0x83, 0xC3, 0x02, 0x66, 0x83, 0x3F, 0x00, 0x89, 0x5C, 0x24, 0x1C,
    0x0F, 0x84, 0x28, 0x05, 0x00, 0x00, 0x83, 0xC7, 0x02, 0xE9, 0x08, 0x05, 0x00, 0x00, 0x66, 0x3D,
    0x25, 0x00, 0x0F, 0x85, 0xF1, 0x00, 0x00, 0x00, 0x0F, 0xB7, 0x47, 0x02, 0x83, 0xC7, 0x02, 0x66,
    0x3D, 0x25, 0x00, 0x75, 0x0E, 0x66, 0x89, 0x03, 0x83, 0xC3, 0x02, 0x83, 0xC7, 0x02, 0x66, 0x8B,
    0x07, 0xEB, 0xBF, 0x66, 0x3D, 0x73, 0x00, 0x74, 0x06, 0x66, 0x3D, 0x53, 0x00, 0x75, 0xEF, 0x3B,
    0xB4, 0x24, 0x4C, 0x30, 0x00, 0x00, 0x0F, 0x83, 0xB1, 0x00, 0x00, 0x00, 0x8B, 0xC6, 0xC1, 0xE0,
    0x04, 0x03, 0x84, 0x24, 0x48, 0x30, 0x00, 0x00, 0x8D, 0x4C, 0x24, 0x38, 0xE8, 0xBF, 0x09, 0x00,
    0x00, 0x8D, 0x4C, 0x24, 0x38, 0xE8, 0x26, 0xFA, 0xFF, 0xFF, 0x8B, 0xE8, 0x85, 0xED, 0x0F, 0x85,
    0x89, 0x00, 0x00, 0x00, 0x68, 0x00, 0x10, 0x00, 0x00, 0x8D, 0x44, 0x24, 0x3C, 0x8D, 0x94, 0x24,
    0x3C, 0x10, 0x00, 0x00, 0xE8, 0x97, 0xFE, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x66,
    0x33, 0xC0, 0x8A, 0x4C, 0x04, 0x38, 0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66, 0x89, 0x94, 0x44,
    0x38, 0x10, 0x00, 0x00, 0x74, 0x46, 0x8A, 0x4C, 0x04, 0x39, 0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1,
    0x66, 0x89, 0x94, 0x44, 0x3A, 0x10, 0x00, 0x00, 0x74, 0x32, 0x8A, 0x4C, 0x04, 0x3A, 0x84, 0xC9,
    0x66, 0x0F, 0xB6, 0xD1, 0x66, 0x89, 0x94, 0x44, 0x3C, 0x10, 0x00, 0x00, 0x74, 0x1E, 0x8A, 0x4C,
    0x04, 0x3B, 0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66, 0x89, 0x94, 0x44, 0x3E, 0x10, 0x00, 0x00,
    0x74, 0x0A, 0x83, 0xC0, 0x04, 0x3D, 0x00, 0x10, 0x00, 0x00, 0x72, 0xA6, 0x66, 0xC7, 0x84, 0x24,
    0x36, 0x30, 0x00, 0x00, 0x00, 0x00, 0x8D, 0xAC, 0x24, 0x38, 0x10, 0x00, 0x00, 0x83, 0xC6, 0x01,
    0x89, 0x74, 0x24, 0x20, 0xE9, 0xC7, 0x03, 0x00, 0x00, 0x66, 0x3D, 0x7B, 0x00, 0x0F, 0x85, 0x1B,
    0xFF, 0xFF, 0xFF, 0x66, 0x39, 0x47, 0x02, 0x75, 0x12, 0x66, 0x89, 0x03, 0x66, 0x8B, 0x47, 0x04,
    0x83, 0xC3, 0x02, 0x83, 0xC7, 0x04, 0xE9, 0xC7, 0xFE, 0xFF, 0xFF, 0x0F, 0xB7, 0x47, 0x02, 0x83,
    0xC7, 0x02, 0x66, 0x3D, 0x20, 0x00, 0x74, 0xF3, 0x66, 0x3D, 0x09, 0x00, 0x74, 0xED, 0x0F, 0xB7,
    0x07, 0x66, 0x3D, 0x7D, 0x00, 0x0F, 0x85, 0xBC, 0x00, 0x00, 0x00, 0x3B, 0xB4, 0x24, 0x4C, 0x30,
    0x00, 0x00, 0x0F, 0x83, 0xA3, 0x00, 0x00, 0x00, 0x8B, 0xC6, 0xC1, 0xE0, 0x04, 0x03, 0x84, 0x24,
    0x48, 0x30, 0x00, 0x00, 0x8D, 0x4C, 0x24, 0x38, 0xE8, 0xB3, 0x08, 0x00, 0x00, 0x68, 0x00, 0x10,
    0x00, 0x00, 0x8D, 0x44, 0x24, 0x3C, 0x8D, 0x94, 0x24, 0x3C, 0x10, 0x00, 0x00, 0xE8, 0x9E, 0xFD,
    0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x6B, 0x33, 0xC0, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x8A, 0x4C, 0x04, 0x38, 0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66, 0x89, 0x94, 0x44, 0x38, 0x10,
    0x00, 0x00, 0x74, 0x46, 0x8A, 0x4C, 0x04, 0x39, 0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66, 0x89,
    0x94, 0x44, 0x3A, 0x10, 0x00, 0x00, 0x74, 0x32, 0x8A, 0x4C, 0x04, 0x3A, 0x84, 0xC9, 0x66, 0x0F,
    0xB6, 0xD1, 0x66, 0x89, 0x94, 0x44, 0x3C, 0x10, 0x00, 0x00, 0x74, 0x1E, 0x8A, 0x4C, 0x04, 0x3B,
    0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66, 0x89, 0x94, 0x44, 0x3E, 0x10, 0x00, 0x00, 0x74, 0x0A,
    0x83, 0xC0, 0x04, 0x3D, 0x00, 0x10, 0x00, 0x00, 0x72, 0xA6, 0x66, 0xC7, 0x84, 0x24, 0x36, 0x30,
    0x00, 0x00, 0x00, 0x00, 0x8D, 0xAC, 0x24, 0x38, 0x10, 0x00, 0x00, 0x83, 0xC6, 0x01, 0x89, 0x74,
    0x24, 0x20, 0xE9, 0x71, 0x02, 0x00, 0x00, 0x66, 0x3D, 0x30, 0x00, 0x0F, 0x82, 0x84, 0x02, 0x00,
    0x00, 0x66, 0x3D, 0x39, 0x00, 0x0F, 0x87, 0x7A, 0x02, 0x00, 0x00, 0x8B, 0xF7, 0xE8, 0xFE, 0xF4,
    0xFF, 0xFF, 0x8B, 0xC8, 0x89, 0x4C, 0x24, 0x34, 0xEB, 0x06, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x0F, 0xB7, 0x07, 0x66, 0x3D, 0x30, 0x00, 0x72, 0x06, 0x66, 0x3D, 0x39, 0x00, 0x76, 0x0C, 0x66,
    0x3D, 0x20, 0x00, 0x74, 0x06, 0x66, 0x3D, 0x09, 0x00, 0x75, 0x05, 0x83, 0xC7, 0x02, 0xEB, 0xE0,
    0x3B, 0x8C, 0x24, 0x4C, 0x30, 0x00, 0x00, 0x89, 0x7C, 0x24, 0x18, 0x0F, 0x83, 0x13, 0x02, 0x00,
    0x00, 0x83, 0xCB, 0xFF, 0x66, 0x83, 0x3F, 0x3A, 0x75, 0x61, 0x0F, 0xB7, 0x47, 0x02, 0x83, 0xC7,
    0x02, 0x66, 0x3D, 0x20, 0x00, 0x74, 0xF3, 0x66, 0x3D, 0x09, 0x00, 0x74, 0xED, 0x66, 0x85, 0xC0,
    0x74, 0xE8, 0x89, 0x7C, 0x24, 0x18, 0x33, 0xF6, 0xEB, 0x06, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x14, 0xB5, 0x48, 0x6F, 0xE8, 0x00, 0x8B, 0xC2, 0x8D, 0x68, 0x02, 0x8D, 0x64, 0x24, 0x00,
    0x66, 0x8B, 0x08, 0x83, 0xC0, 0x02, 0x66, 0x85, 0xC9, 0x75, 0xF5, 0x2B, 0xC5, 0xD1, 0xF8, 0x50,
    0x52, 0x8B, 0xC7, 0xE8, 0x78, 0x03, 0x06, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x0A, 0x83,
    0xC6, 0x01, 0x83, 0xFE, 0x02, 0x7C, 0xC9, 0xEB, 0x02, 0x8B, 0xDE, 0x8B, 0x7C, 0x24, 0x34, 0xC1,
    0xE7, 0x04, 0x03, 0xBC, 0x24, 0x48, 0x30, 0x00, 0x00, 0x8D, 0x74, 0x24, 0x24, 0xC7, 0x44, 0x24,
    0x24, 0x00, 0x00, 0x00, 0x00, 0xE8, 0xE6, 0x0B, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x24, 0x85, 0xC0,
    0x75, 0x0A, 0xBD, 0x48, 0xCF, 0xE4, 0x00, 0xE9, 0x5F, 0x01, 0x00, 0x00, 0x85, 0xDB, 0x75, 0x33,
    0x83, 0xF8, 0x02, 0x75, 0x24, 0x8D, 0x4C, 0x24, 0x24, 0xE8, 0x42, 0x21, 0xAA, 0xFF, 0x8B, 0xC8,
    0xE8, 0x9B, 0xF7, 0xFF, 0xFF, 0x8B, 0xE8, 0x85, 0xED, 0x0F, 0x85, 0x3C, 0x01, 0x00, 0x00, 0xBD,
    0x88, 0xCF, 0xE4, 0x00, 0xE9, 0x32, 0x01, 0x00, 0x00, 0xBD, 0xB0, 0xCF, 0xE4, 0x00, 0xE9, 0x28,
    0x01, 0x00, 0x00, 0x83, 0xF8, 0x04, 0x0F, 0x85, 0xA1, 0x00, 0x00, 0x00, 0x83, 0xFB, 0x01, 0x8B,
    0x74, 0x24, 0x28, 0x74, 0x11, 0x8B, 0xCE, 0xE8, 0x44, 0xF7, 0xFF, 0xFF, 0x8B, 0xE8, 0x85, 0xED,
    0x0F, 0x85, 0x05, 0x01, 0x00, 0x00, 0x68, 0x00, 0x10, 0x00, 0x00, 0x8B, 0xC6, 0x8D, 0x94, 0x24,
    0x3C, 0x10, 0x00, 0x00, 0xE8, 0xB7, 0xFB, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x0F, 0x85,
    0xE0, 0x00, 0x00, 0x00, 0x33, 0xC0, 0x8A, 0x0C, 0x30, 0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66,
    0x89, 0x94, 0x44, 0x38, 0x10, 0x00, 0x00, 0x0F, 0x84, 0xBD, 0x00, 0x00, 0x00, 0x8A, 0x4C, 0x30,
    0x01, 0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66, 0x89, 0x94, 0x44, 0x3A, 0x10, 0x00, 0x00, 0x0F,
    0x84, 0xA5, 0x00, 0x00, 0x00, 0x8A, 0x4C, 0x30, 0x02, 0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66,
    0x89, 0x94, 0x44, 0x3C, 0x10, 0x00, 0x00, 0x0F, 0x84, 0x8D, 0x00, 0x00, 0x00, 0x8A, 0x4C, 0x30,
    0x03, 0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66, 0x89, 0x94, 0x44, 0x3E, 0x10, 0x00, 0x00, 0x74,
    0x79, 0x83, 0xC0, 0x04, 0x3D, 0x00, 0x10, 0x00, 0x00, 0x72, 0x9B, 0xEB, 0x6D, 0x8D, 0x4C, 0x24,
    0x38, 0x8D, 0x44, 0x24, 0x24, 0xE8, 0x36, 0x06, 0x00, 0x00, 0x33, 0xC0, 0x8D, 0x64, 0x24, 0x00,
    0x8A, 0x4C, 0x04, 0x38, 0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66, 0x89, 0x94, 0x44, 0x38, 0x10,
    0x00, 0x00, 0x74, 0x46, 0x8A, 0x4C, 0x04, 0x39, 0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66, 0x89,
    0x94, 0x44, 0x3A, 0x10, 0x00, 0x00, 0x74, 0x32, 0x8A, 0x4C, 0x04, 0x3A, 0x84, 0xC9, 0x66, 0x0F,
    0xB6, 0xD1, 0x66, 0x89, 0x94, 0x44, 0x3C, 0x10, 0x00, 0x00, 0x74, 0x1E, 0x8A, 0x4C, 0x04, 0x3B,
    0x84, 0xC9, 0x66, 0x0F, 0xB6, 0xD1, 0x66, 0x89, 0x94, 0x44, 0x3E, 0x10, 0x00, 0x00, 0x74, 0x0A,
    0x83, 0xC0, 0x04, 0x3D, 0x00, 0x10, 0x00, 0x00, 0x72, 0xA6, 0x66, 0xC7, 0x84, 0x24, 0x36, 0x30,
    0x00, 0x00, 0x00, 0x00, 0x8D, 0xAC, 0x24, 0x38, 0x10, 0x00, 0x00, 0x8D, 0x44, 0x24, 0x24, 0xE8,
    0xDC, 0x0B, 0x00, 0x00, 0x8B, 0x7C, 0x24, 0x18, 0x8B, 0x5C, 0x24, 0x1C, 0xC7, 0x44, 0x24, 0x24,
    0x00, 0x00, 0x00, 0x00, 0x8B, 0x74, 0x24, 0x20, 0x0F, 0xB7, 0x07, 0x66, 0x3D, 0x7D, 0x00, 0x74,
    0x4F, 0x66, 0x85, 0xC0, 0x74, 0x44, 0x0F, 0xB7, 0x47, 0x02, 0x83, 0xC7, 0x02, 0x66, 0x3D, 0x7D,
    0x00, 0x75, 0xEE, 0xEB, 0x3B, 0x8B, 0xCF, 0x0F, 0xB7, 0xC0, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x66, 0x85, 0xC0, 0x74, 0x0D, 0x0F, 0xB7, 0x41, 0x02, 0x83, 0xC1, 0x02, 0x66, 0x3D, 0x7D, 0x00,
    0x75, 0xEE, 0x2B, 0xCF, 0xD1, 0xF9, 0x8B, 0xD7, 0xE8, 0x63, 0xF6, 0xFF, 0xFF, 0x8B, 0xE8, 0x85,
    0xED, 0x75, 0xB5, 0xBD, 0xFC, 0xCF, 0xE4, 0x00, 0xEB, 0xAE, 0x66, 0x83, 0x3F, 0x7D, 0x75, 0x03,
    0x83, 0xC7, 0x02, 0x85, 0xED, 0x0F, 0x84, 0x53, 0xFB, 0xFF, 0xFF, 0x0F, 0xB7, 0x45, 0x00, 0x66,
    0x85, 0xC0, 0x74, 0x32, 0x8B, 0xCB, 0x2B, 0x8C, 0x24, 0x3C, 0x30, 0x00, 0x00, 0x8D, 0x49, 0x00,
    0x8B, 0xD1, 0xD1, 0xFA, 0x3B, 0x94, 0x24, 0x44, 0x30, 0x00, 0x00, 0x73, 0x15, 0x83, 0xC5, 0x02,
    0x66, 0x89, 0x03, 0x0F, 0xB7, 0x45, 0x00, 0x83, 0xC3, 0x02, 0x83, 0xC1, 0x02, 0x66, 0x85, 0xC0,
    0x75, 0xDE, 0x89, 0x5C, 0x24, 0x1C, 0x8B, 0xCB, 0x2B, 0x8C, 0x24, 0x3C, 0x30, 0x00, 0x00, 0xD1,
    0xF9, 0x3B, 0x8C, 0x24, 0x44, 0x30, 0x00, 0x00, 0x0F, 0x82, 0xA6, 0xFA, 0xFF, 0xFF, 0x8B, 0x8C,
    0x24, 0x3C, 0x30, 0x00, 0x00, 0x8B, 0x94, 0x24, 0x44, 0x30, 0x00, 0x00, 0x8B, 0xC3, 0x2B, 0xC1,
    0x66, 0xC7, 0x44, 0x51, 0xFE, 0x00, 0x00, 0xD1, 0xF8, 0x5F, 0x5E, 0x5D, 0x5B, 0x81, 0xC4, 0x28,
    0x30, 0x00, 0x00, 0xC3, 0x5F, 0x5E, 0x5D, 0x33, 0xC0, 0x5B, 0x81, 0xC4, 0x28, 0x30, 0x00, 0x00,
    0xC3,
];
const EXPAND_BODY_CALLS: [usize; 16] = [0x5, 0xdc, 0xe5, 0x104, 0x1e8, 0x1fd, 0x29d, 0x333, 0x365, 0x389, 0x390, 0x3c7, 0x3e4, 0x465, 0x4df, 0x538];
const SET_INT_BODY: [u8; 144] = [
    0x8B, 0x06, 0x83, 0xF8, 0x08, 0x57, 0x8B, 0x7C, 0x24, 0x08, 0x77, 0x59, 0xFF, 0x24, 0x85, 0xFC,
    0x35, 0xB9, 0x00, 0xC7, 0x06, 0x01, 0x00, 0x00, 0x00, 0x89, 0x7E, 0x04, 0x8B, 0xC6, 0x5F, 0xC2,
    0x04, 0x00, 0xDB, 0x44, 0x24, 0x08, 0x8B, 0xC6, 0x5F, 0xD9, 0x5E, 0x04, 0xC2, 0x04, 0x00, 0x8B,
    0x56, 0x04, 0x3B, 0xD7, 0x74, 0x2F, 0xE8, 0xB5, 0xD3, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x07, 0x8B,
    0xC8, 0xE8, 0x7A, 0xD2, 0x00, 0x00, 0x8B, 0xD7, 0xE8, 0xA3, 0xD3, 0x00, 0x00, 0x85, 0xC0, 0x74,
    0x0D, 0x89, 0x7E, 0x04, 0x83, 0x40, 0x1C, 0x01, 0x8B, 0xC6, 0x5F, 0xC2, 0x04, 0x00, 0xC7, 0x46,
    0x04, 0x00, 0x00, 0x00, 0x00, 0x8B, 0xC6, 0x5F, 0xC2, 0x04, 0x00, 0x90, 0xA3, 0x35, 0xB9, 0x00,
    0xA3, 0x35, 0xB9, 0x00, 0xA9, 0x35, 0xB9, 0x00, 0xB2, 0x35, 0xB9, 0x00, 0xF5, 0x35, 0xB9, 0x00,
    0xF5, 0x35, 0xB9, 0x00, 0xF5, 0x35, 0xB9, 0x00, 0xF5, 0x35, 0xB9, 0x00, 0xBF, 0x35, 0xB9, 0x00,
];
const SET_INT_BODY_CALLS: [usize; 3] = [0x36, 0x41, 0x48];

const REHASH_NAME_BODY: [u8; 162] = [
    0x8B, 0x50, 0x2C, 0x3B, 0xD1, 0x0F, 0x84, 0x96, 0x00, 0x00, 0x00, 0x57, 0x33, 0xFF, 0x3B, 0xD7,
    0x74, 0x43, 0x83, 0xE2, 0x3F, 0x39, 0x04, 0x95, 0xB8, 0xF1, 0x2E, 0x02, 0x56, 0x75, 0x1D, 0x8B,
    0x70, 0x04, 0x3B, 0xF0, 0x75, 0x0F, 0x89, 0x78, 0x04, 0x89, 0x78, 0x08, 0x89, 0x3C, 0x95, 0xB8,
    0xF1, 0x2E, 0x02, 0xEB, 0x1F, 0x89, 0x34, 0x95, 0xB8, 0xF1, 0x2E, 0x02, 0x8B, 0x50, 0x08, 0x8B,
    0x70, 0x04, 0x89, 0x72, 0x04, 0x8B, 0x50, 0x04, 0x8B, 0x70, 0x08, 0x89, 0x72, 0x08, 0x89, 0x78,
    0x04, 0x89, 0x78, 0x08, 0x5E, 0x3B, 0xCF, 0x89, 0x48, 0x2C, 0x74, 0x44, 0x83, 0xE1, 0x3F, 0x8B,
    0x14, 0x8D, 0xB8, 0xF1, 0x2E, 0x02, 0x3B, 0xD7, 0x75, 0x0F, 0x89, 0x40, 0x04, 0x89, 0x40, 0x08,
    0x89, 0x04, 0x8D, 0xB8, 0xF1, 0x2E, 0x02, 0x5F, 0xC3, 0x8B, 0x52, 0x08, 0x89, 0x50, 0x08, 0x8B,
    0x14, 0x8D, 0xB8, 0xF1, 0x2E, 0x02, 0x89, 0x50, 0x04, 0x8B, 0x14, 0x8D, 0xB8, 0xF1, 0x2E, 0x02,
    0x8B, 0x52, 0x08, 0x89, 0x42, 0x04, 0x8B, 0x0C, 0x8D, 0xB8, 0xF1, 0x2E, 0x02, 0x89, 0x41, 0x08,
    0x5F, 0xC3,
];
const REHASH_NAME_BODY_CALLS: [usize; 0] = [];
const OBJECT_SET_NAME_BODY: [u8; 50] = [
    0x85, 0xFF, 0x53, 0x8B, 0x5C, 0x24, 0x08, 0x74, 0x25, 0x56, 0x6A, 0x20, 0x8D, 0x73, 0x34, 0x57,
    0x56, 0xE8, 0xBA, 0x1A, 0x10, 0x00, 0x83, 0xC4, 0x0C, 0x8B, 0xCF, 0xC6, 0x46, 0x1F, 0x00, 0xE8,
    0x6C, 0x53, 0x04, 0x00, 0x8B, 0xC8, 0x8B, 0xC3, 0xE8, 0x23, 0xFF, 0xFF, 0xFF, 0x5E, 0x5B, 0xC2,
    0x04, 0x00,
];
const OBJECT_SET_NAME_BODY_CALLS: [usize; 3] = [0x11, 0x1f, 0x28];
const SET_DOCUMENT_REC_BODY: [u8; 56] = [
    0x53, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x85, 0xFF, 0x8B, 0xD9, 0x74, 0x27, 0x39, 0x7B, 0x30, 0x74,
    0x22, 0x56, 0x8B, 0x73, 0x1C, 0x85, 0xF6, 0x74, 0x16, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x57, 0x8B, 0xCE, 0xE8, 0xD8, 0xFF, 0xFF, 0xFF, 0x8B, 0x76, 0x20, 0x85, 0xF6, 0x75, 0xF1, 0x89,
    0x7B, 0x30, 0x5E, 0x5F, 0x5B, 0xC2, 0x04, 0x00,
];
const SET_DOCUMENT_REC_BODY_CALLS: [usize; 1] = [0x23];
const RAW_INPUT_INDEX_BODY: [u8; 82] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x85, 0xDB, 0x75, 0x05, 0x83, 0xC8, 0xFF, 0x5B, 0xC3, 0x56, 0x33,
    0xF6, 0x39, 0x35, 0x3C, 0xA2, 0x52, 0x02, 0x57, 0x76, 0x2B, 0x33, 0xFF, 0x8D, 0x64, 0x24, 0x00,
    0xA1, 0x40, 0xA2, 0x52, 0x02, 0x8B, 0x4C, 0x07, 0x14, 0x51, 0x53, 0xE8, 0x20, 0xF0, 0x0F, 0x00,
    0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x15, 0x83, 0xC6, 0x01, 0x83, 0xC7, 0x18, 0x3B, 0x35, 0x3C,
    0xA2, 0x52, 0x02, 0x72, 0xDB, 0x5F, 0x5E, 0x83, 0xC8, 0xFF, 0x5B, 0xC3, 0x5F, 0x8B, 0xC6, 0x5E,
    0x5B, 0xC3,
];
const RAW_INPUT_INDEX_BODY_CALLS: [usize; 1] = [0x2b];
const PROJECT_FIND_BODY: [u8; 66] = [
    0x56, 0xE8, 0x5A, 0x1A, 0x04, 0x00, 0x8B, 0x15, 0x44, 0xA2, 0x52, 0x02, 0x8B, 0xF0, 0x33, 0xC0,
    0x85, 0xD2, 0x7E, 0x1D, 0xB9, 0x28, 0xF6, 0x2E, 0x02, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x31, 0x74, 0x11, 0x83, 0xC0, 0x01, 0x81, 0xC1, 0x88, 0x00, 0x00, 0x00, 0x3B, 0xC2, 0x7C,
    0xEF, 0x33, 0xC0, 0x5E, 0xC3, 0x69, 0xC0, 0x88, 0x00, 0x00, 0x00, 0x05, 0x30, 0xF6, 0x2E, 0x02,
    0x5E, 0xC3,
];
const PROJECT_FIND_BODY_CALLS: [usize; 1] = [0x1];
const CLIP_POP_BODY: [u8; 78] = [
    0x8B, 0x82, 0xE4, 0x04, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x43, 0x83, 0xC0, 0xFF, 0x89, 0x82, 0xE4,
    0x04, 0x00, 0x00, 0x75, 0x14, 0xA1, 0xD8, 0xA1, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x2F, 0x83, 0x78,
    0x10, 0x00, 0x74, 0x29, 0x8B, 0x40, 0x14, 0xFF, 0xE0, 0x83, 0xF8, 0x08, 0x73, 0x1F, 0x8B, 0x0D,
    0xD8, 0xA1, 0x52, 0x02, 0x85, 0xC9, 0x74, 0x15, 0x8B, 0x49, 0x10, 0x85, 0xC9, 0x74, 0x0E, 0xC1,
    0xE0, 0x04, 0x8D, 0x94, 0x10, 0x64, 0x04, 0x00, 0x00, 0x52, 0xFF, 0xD1, 0x59, 0xC3,
];
const CLIP_POP_BODY_CALLS: [usize; 0] = [];
const DELIVER_ALL_BODY: [u8; 53] = [
    0x56, 0x57, 0x8B, 0x3D, 0x04, 0xA2, 0x52, 0x02, 0x85, 0xFF, 0x74, 0x26, 0x8B, 0x07, 0x8B, 0xF0,
    0x2B, 0x35, 0x04, 0xA2, 0x52, 0x02, 0x57, 0xF7, 0xDE, 0x1B, 0xF6, 0x23, 0xF0, 0xE8, 0x5E, 0x00,
    0x00, 0x00, 0x80, 0x7F, 0x1C, 0x01, 0x75, 0x06, 0x57, 0xE8, 0x32, 0xF6, 0xFF, 0xFF, 0x8B, 0xFE,
    0xEB, 0xD6, 0x5F, 0x5E, 0xC3,
];
const DELIVER_ALL_BODY_CALLS: [usize; 2] = [0x1d, 0x29];
const GROUP_DESTROY_BODY: [u8; 353] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x55, 0x56, 0x8B, 0x73, 0x18, 0x33, 0xED, 0x57, 0x8D, 0x49, 0x00,
    0x3B, 0xF5, 0x0F, 0x84, 0xE6, 0x00, 0x00, 0x00, 0x8B, 0x46, 0x14, 0x8B, 0x4B, 0x18, 0x8B, 0xF8,
    0x2B, 0xF9, 0xF7, 0xDF, 0x1B, 0xFF, 0x23, 0xF8, 0x3B, 0xCE, 0x75, 0x12, 0x3B, 0xC6, 0x75, 0x0B,
    0x89, 0x6E, 0x14, 0x89, 0x6E, 0x18, 0x89, 0x6B, 0x18, 0xEB, 0x23, 0x89, 0x43, 0x18, 0x8B, 0x46,
    0x18, 0x3B, 0xC5, 0x74, 0x06, 0x8B, 0x4E, 0x14, 0x89, 0x48, 0x14, 0x8B, 0x46, 0x14, 0x3B, 0xC5,
    0x74, 0x06, 0x8B, 0x56, 0x18, 0x89, 0x50, 0x18, 0x89, 0x6E, 0x14, 0x89, 0x6E, 0x18, 0x8B, 0x43,
    0x08, 0x3B, 0xC5, 0x74, 0x1B, 0x89, 0x06, 0xA1, 0x10, 0xA2, 0x52, 0x02, 0x3B, 0xC5, 0x75, 0x75,
    0x89, 0x35, 0x10, 0xA2, 0x52, 0x02, 0x89, 0x76, 0x18, 0x89, 0x76, 0x14, 0x8B, 0xF7, 0xEB, 0x90,
    0x8B, 0x56, 0x0C, 0x3B, 0xD5, 0x74, 0x42, 0x8B, 0xC2, 0x83, 0xE0, 0x3F, 0x8B, 0x04, 0x85, 0x70,
    0x26, 0x2F, 0x02, 0x8B, 0xC8, 0x3B, 0xCD, 0x74, 0x30, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x51, 0x18, 0x74, 0x0D, 0x8B, 0x49, 0x08, 0x3B, 0xC8, 0x74, 0x1D, 0x3B, 0xCD, 0x75, 0xF0,
    0xEB, 0x17, 0x8B, 0x41, 0x1C, 0x83, 0xF8, 0x01, 0x75, 0x09, 0x8B, 0x01, 0x8B, 0x50, 0x14, 0xFF,
    0xD2, 0xEB, 0x06, 0x83, 0xC0, 0xFF, 0x89, 0x41, 0x1C, 0xA1, 0x0C, 0xA2, 0x52, 0x02, 0x3B, 0xC5,
    0x75, 0x13, 0x89, 0x35, 0x0C, 0xA2, 0x52, 0x02, 0x89, 0x76, 0x18, 0x89, 0x76, 0x14, 0x8B, 0xF7,
    0xE9, 0x2B, 0xFF, 0xFF, 0xFF, 0x8B, 0x48, 0x18, 0x89, 0x4E, 0x18, 0x89, 0x46, 0x14, 0x8B, 0x50,
    0x18, 0x89, 0x72, 0x14, 0x89, 0x70, 0x18, 0x8B, 0xF7, 0xE9, 0x12, 0xFF, 0xFF, 0xFF, 0x39, 0x1D,
    0x04, 0xA2, 0x52, 0x02, 0x89, 0x6B, 0x14, 0x5F, 0x75, 0x13, 0x8B, 0x03, 0x3B, 0xC3, 0x75, 0x08,
    0x89, 0x2D, 0x04, 0xA2, 0x52, 0x02, 0xEB, 0x14, 0xA3, 0x04, 0xA2, 0x52, 0x02, 0x8B, 0x43, 0x04,
    0x8B, 0x0B, 0x89, 0x08, 0x8B, 0x13, 0x8B, 0x43, 0x04, 0x89, 0x42, 0x04, 0xA1, 0x00, 0xA2, 0x52,
    0x02, 0x3B, 0xC5, 0x89, 0x6B, 0x04, 0x89, 0x2B, 0x75, 0x11, 0x5E, 0x5D, 0x89, 0x1B, 0x89, 0x5B,
    0x04, 0x89, 0x1D, 0x00, 0xA2, 0x52, 0x02, 0x5B, 0xC2, 0x04, 0x00, 0x8B, 0x48, 0x04, 0x89, 0x4B,
    0x04, 0x89, 0x03, 0x8B, 0x50, 0x04, 0x5E, 0x89, 0x1A, 0x5D, 0x89, 0x58, 0x04, 0x5B, 0xC2, 0x04,
    0x00,
];
const GROUP_DESTROY_BODY_CALLS: [usize; 0] = [];
const GROUP_UNSUBSCRIBE_BODY: [u8; 236] = [
    0x8B, 0x41, 0x18, 0x53, 0x8B, 0x5C, 0x24, 0x08, 0x55, 0x56, 0x8B, 0xF0, 0x33, 0xED, 0x3B, 0xF5,
    0x0F, 0x84, 0xD0, 0x00, 0x00, 0x00, 0x39, 0x5E, 0x0C, 0x75, 0x0A, 0x39, 0x7E, 0x10, 0x75, 0x05,
    0x39, 0x56, 0x08, 0x74, 0x15, 0x8B, 0x76, 0x14, 0x3B, 0xF0, 0x0F, 0x84, 0xB6, 0x00, 0x00, 0x00,
    0x3B, 0xF5, 0x75, 0xE2, 0x5E, 0x5D, 0x5B, 0xC2, 0x04, 0x00, 0x3B, 0xC6, 0x75, 0x15, 0x8B, 0x46,
    0x14, 0x3B, 0xC6, 0x75, 0x0B, 0x89, 0x6E, 0x14, 0x89, 0x6E, 0x18, 0x89, 0x69, 0x18, 0xEB, 0x23,
    0x89, 0x41, 0x18, 0x8B, 0x46, 0x18, 0x3B, 0xC5, 0x74, 0x06, 0x8B, 0x4E, 0x14, 0x89, 0x48, 0x14,
    0x8B, 0x46, 0x14, 0x3B, 0xC5, 0x74, 0x06, 0x8B, 0x56, 0x18, 0x89, 0x50, 0x18, 0x89, 0x6E, 0x14,
    0x89, 0x6E, 0x18, 0x8B, 0x56, 0x0C, 0x3B, 0xD5, 0x74, 0x3F, 0x8B, 0xC2, 0x83, 0xE0, 0x3F, 0x8B,
    0x04, 0x85, 0x70, 0x26, 0x2F, 0x02, 0x8B, 0xC8, 0x3B, 0xCD, 0x74, 0x2D, 0x8D, 0x64, 0x24, 0x00,
    0x39, 0x51, 0x18, 0x74, 0x0D, 0x8B, 0x49, 0x08, 0x3B, 0xC8, 0x74, 0x1D, 0x3B, 0xCD, 0x75, 0xF0,
    0xEB, 0x17, 0x8B, 0x41, 0x1C, 0x83, 0xF8, 0x01, 0x75, 0x09, 0x8B, 0x01, 0x8B, 0x50, 0x14, 0xFF,
    0xD2, 0xEB, 0x06, 0x83, 0xC0, 0xFF, 0x89, 0x41, 0x1C, 0xA1, 0x0C, 0xA2, 0x52, 0x02, 0x3B, 0xC5,
    0x75, 0x12, 0x89, 0x76, 0x14, 0x89, 0x76, 0x18, 0x89, 0x35, 0x0C, 0xA2, 0x52, 0x02, 0x5E, 0x5D,
    0x5B, 0xC2, 0x04, 0x00, 0x8B, 0x48, 0x18, 0x89, 0x4E, 0x18, 0x89, 0x46, 0x14, 0x8B, 0x50, 0x18,
    0x89, 0x72, 0x14, 0x89, 0x70, 0x18, 0x5E, 0x5D, 0x5B, 0xC2, 0x04, 0x00,
];
const GROUP_UNSUBSCRIBE_BODY_CALLS: [usize; 0] = [];
