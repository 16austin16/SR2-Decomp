//! Port module: the network bit-stream writers and readers, the net-id / handle lookups, the
//! ownership predicates, the packet gates and a few leaves (doc 48, "Port candidates" and the
//! small functions next to them), checked against the disassembly.
//!
//! Every port goes through a `pushad` thunk: all registers come back as the caller left them
//! except EAX, which carries the result (an `f32` result stays on ST0). That is a superset of
//! what each original preserves. Where an original returns AL (or AX), the rest of EAX is
//! reproduced as the original leaves it, so the whole of EAX is the same.
//!
//! Threads: the only network thread (proc 0x814320, started at 0x8095D3 / 0x8143C9 / 0x8144A4)
//! loops on a `select` and a semaphore and calls none of these functions; the readers / writers,
//! the lookups and the gates run on the game thread (packet receive and send, the compressors).
//! The shadows are thread-safe all the same: counters are atomics, every scratch copy is a
//! per-call local, and a stream is private to one packet.
//!
//! Shadow check, for the first `SHADOW_CALLS` calls of each port:
//! - register / memory-reading ports: a relocated copy of the original runs on the same seven
//!   registers and stack arguments; its EAX (and ST0) are compared with the Rust version's, and
//!   on a mismatch the original's are kept. The copies of the original call the *copies* of the
//!   original callees.
//! - readers that move the cursor: the original runs first on the real stream, the cursor (and
//!   the output buffer of the vector / byte readers) it left is saved, the state is put back,
//!   Rust runs, the two are compared and the original's are kept on a mismatch.
//! - bit writers: Rust runs on a scratch copy of the stream header and of the buffer bytes, the
//!   original runs on the real stream, and the cursor, the header and every buffer byte are
//!   compared (the real stream therefore always carries the original's writes).
//! - leaves that only store constants: Rust stores, the original stores, memory is compared.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

const HANDLES: usize = 0x0214_9C64;
const SESSION: usize = 0x0252_8C14;
const NET_ID_MAP: usize = 0x0212_E258;
const NOW: usize = 0x0252_A3CC;
const TWO_POW_32: usize = 0x00E5_BFD8;
const POOL_ALLOC: usize = 0x0080_9220;
const CLOCK_OFFLINE: usize = 0x0212_B160;
const CLOCK_DELTA: usize = 0x0212_B15C;
const TIMEOUT_FLAG: usize = 0x0252_8B89;
const TIMEOUT_AT: usize = 0x00EB_1614;
/// Largest stream buffer the writer shadow copies.
const MAX_STREAM_BYTES: usize = 1 << 20;

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

/// The seven general registers in and out of `call_orig`.
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

#[repr(C)]
struct Block {
    f: u32,
    r: Regs,
    args: [u32; 3],
    esp: u32,
    want_st0: u32,
    st0: [u32; 2],
}

/// Call `f` with all seven registers from `r` and three stack arguments (harmless for the
/// callees that take fewer; a `ret N` callee pops its own), then capture all seven registers
/// into `r` and, when `want_st0`, pop ST0 (returned). The stack is restored from a saved value,
/// and the block pointer is pushed four times so that it is found at the same place whether the
/// callee popped 0, 4, 8 or 12 bytes.
unsafe fn call_orig(f: usize, r: &mut Regs, args: [u32; 3], want_st0: bool) -> f64 {
    let mut b = Block { f: f as u32, r: *r, args, esp: 0, want_st0: want_st0 as u32, st0: [0; 2] };
    unsafe {
        std::arch::asm!(
            "push ebp", "push ebx", "push esi", "push edi",
            "mov [eax + 44], esp",
            "push eax", "push eax", "push eax", "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 40]",
            "push dword ptr [eax + 36]",
            "push dword ptr [eax + 32]",
            "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 12]",
            "push ebx",
            "mov ebx, [esp + 20]",
            "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
            "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
            "pop dword ptr [ebx + 16]",
            "cmp dword ptr [ebx + 48], 0",
            "je 2f",
            "fstp qword ptr [ebx + 52]",
            "2:",
            "mov esp, [ebx + 44]",
            "pop edi", "pop esi", "pop ebx", "pop ebp",
            inout("eax") &mut b as *mut Block => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
    f64::from_bits(b.st0[0] as u64 | (b.st0[1] as u64) << 32)
}

/// A naked thunk: `pushad`, then `f(&mut block, &stack args)`, then `popad`. The Rust body puts
/// its result in the block's EAX; an `f32` return stays on ST0. `$ret` is the `ret` form.
macro_rules! regs_thunk {
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

/// An entry + thunk pair for the handler `$h(id, &mut Pushad, args)`.
macro_rules! port_thunk {
    ($thunk:ident, $entry:ident, $id:expr, $h:ident) => {
        port_thunk!($thunk, $entry, $id, $h, "ret");
    };
    ($thunk:ident, $entry:ident, $id:expr, $h:ident, $ret:literal) => {
        extern "C" fn $entry(r: &mut Pushad, a: *const u32) {
            unsafe { $h($id, r, a) }
        }
        regs_thunk!($thunk, $entry, $ret);
    };
}

/// The same for a handler returning an `f32` on ST0.
macro_rules! port_thunk_f32 {
    ($thunk:ident, $entry:ident, $id:expr, $h:ident) => {
        extern "C" fn $entry(r: &mut Pushad, a: *const u32) -> f32 {
            unsafe { $h($id, r, a) }
        }
        regs_thunk!($thunk, $entry, "ret");
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

const WRITE_AT: usize = 0;
const WRITE_BIT: usize = 1;
const WRITE_U8: usize = 2;
const WRITE_S8: usize = 3;
const WRITE_U8_DUP: usize = 4;
const WRITE_U16: usize = 5;
const WRITE_S16: usize = 6;
const WRITE_U32: usize = 7;
const WRITE_U32_DUP: usize = 8;
const WRITE_U32_STACK: usize = 9;
const WRITE_VEC3: usize = 10;
const WRITE_VEC2: usize = 11;
const WRITE_BYTES: usize = 12;
const READ_AT: usize = 13;
const READ_N: usize = 14;
const READ_BIT: usize = 15;
const READ_U8: usize = 16;
const READ_S8: usize = 17;
const READ_U16: usize = 18;
const READ_S16: usize = 19;
const READ_U32: usize = 20;
const READ_S32: usize = 21;
const READ_U8_DUP: usize = 22;
const READ_F32: usize = 23;
const READ_VEC3: usize = 24;
const READ_VEC2: usize = 25;
const READ_BYTES: usize = 26;
const HANDLE_TO_ID: usize = 27;
const ID_TO_HANDLE: usize = 28;
const MAP_FIND: usize = 29;
const HANDLE_OWNED: usize = 30;
const RECORD_OWNED: usize = 31;
const FIND_CONN: usize = 32;
const GUID_EQUAL: usize = 33;
const STATE_ALLOWS: usize = 34;
const ROUTE_SEND: usize = 35;
const ROUTE_RECV: usize = 36;
const LOSS_RATIO: usize = 37;
const CLOCK_MS: usize = 38;
const CLOCK_RESET: usize = 39;
const TIMEOUT_CLEAR: usize = 40;
const STATE_1_6_8: usize = 41;
const STATE_0_1: usize = 42;
const RECENT_IDS: usize = 43;
const OBJ_OWNER_CONN: usize = 44;
const HANDLE_OWNER_CONN: usize = 45;
const REG_FIND_FIELD: usize = 46;
const COUNT: usize = 47;

static PORTS: [Port; COUNT] = [
    port("net_bits_write_at", 0x007F_D030, SAVED),
    port("net_bits_write_bit", 0x007F_D0B0, ECX | SAVED),
    port("net_bits_write_u8", 0x007F_D100, EAX | SAVED),
    port("net_bits_write_s8", 0x007F_D260, EAX | SAVED),
    port("net_bits_write_u8_dup", 0x007F_D700, EAX | SAVED),
    port("net_bits_write_u16", 0x007F_D3C0, ECX | SAVED),
    port("net_bits_write_s16", 0x007F_D490, ECX | SAVED),
    port("net_bits_write_u32", 0x007F_D560, ECX | SAVED),
    port("net_bits_write_u32_dup", 0x007F_D630, ECX | SAVED),
    port("net_bits_write_u32_stack", 0x007F_D860, ECX | SAVED),
    port("net_bits_write_vec3", 0x007F_D870, ECX | SAVED),
    port("net_bits_write_vec2", 0x007F_D8A0, ECX | SAVED),
    port("net_bits_write_bytes", 0x007F_D8C0, EAX | SAVED),
    port("net_bits_read_at", 0x007F_D8F0, SAVED),
    port("net_bits_read_n", 0x007F_D940, SAVED),
    port("net_bits_read_bit", 0x007F_D960, ECX | SAVED),
    port("net_bits_read_u8", 0x007F_D9B0, SAVED),
    port("net_bits_read_s8", 0x007F_D9D0, SAVED),
    port("net_bits_read_u16", 0x007F_D9F0, SAVED),
    port("net_bits_read_s16", 0x007F_DA10, SAVED),
    port("net_bits_read_u32", 0x007F_DA30, SAVED),
    port("net_bits_read_s32", 0x007F_DA50, SAVED),
    port("net_bits_read_u8_dup", 0x007F_DA70, SAVED),
    port("net_bits_read_f32", 0x007F_DA90, ECX | SAVED),
    port("net_bits_read_vec3", 0x007F_DAC0, SAVED),
    port("net_bits_read_vec2", 0x007F_DB20, SAVED),
    port("net_bits_read_bytes", 0x007F_DB70, SAVED),
    port("net_handle_to_net_id", 0x0080_4F70, ECX | EDX | SAVED),
    port("net_id_to_handle", 0x0080_4FB0, SAVED),
    port("net_remote_handle_map_find", 0x0080_8E30, SAVED),
    port("net_handle_owned_locally", 0x0080_66C0, SAVED),
    port("net_record_owned_locally", 0x0080_6760, ECX | SAVED),
    port("session_find_conn_by_id", 0x0080_FE80, EDX | SAVED),
    port("net_guid_equal", 0x0081_76E0, SAVED),
    port("net_packet_state_allows", 0x0081_4B80, SAVED),
    port("net_packet_route_allows_send", 0x0081_4B00, ECX | SAVED),
    port("net_packet_route_allows_recv", 0x0081_4B40, ECX | SAVED),
    port("net_conn_loss_ratio", 0x007F_8300, SAVED),
    port("net_clock_ms", 0x007F_CA90, ECX | EDX | SAVED),
    port("net_clock_reset", 0x007F_C920, ECX | EDX | SAVED),
    port("net_timeout_clear", 0x007F_C890, EAX | ECX | EDX | SAVED),
    port("session_state_in_1_6_8", 0x0080_CEF0, ECX | EDX | SAVED),
    port("session_state_in_0_1", 0x0080_CF20, ECX | EDX | SAVED),
    port("net_conn_recent_ids_contains", 0x007F_AEF0, EDX | SAVED),
    port("net_object_owner_conn", 0x0080_7230, SAVED),
    port("net_handle_owner_conn", 0x0080_7250, SAVED),
    port("net_registration_find_field", 0x0080_7680, SAVED),
];

/// Count a call; `Some((call number, original copy))` while it should be shadowed.
fn shadowing(id: usize) -> Option<(u32, usize)> {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_net: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    (orig != 0 && n < SHADOW_CALLS).then_some((n, orig))
}

fn mismatch(id: usize, n: u32, what: String) {
    let m = PORTS[id].mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!("port_net: {} mismatch (call {n}): {what}", PORTS[id].name));
    }
}

static PRESERVE_LOGS: AtomicU32 = AtomicU32::new(0);

/// Log when the original changed a register it was found to preserve (the thunks preserve all).
fn check_preserved(id: usize, inp: &Regs, out: &Regs) {
    for i in 0..7 {
        if PORTS[id].preserved & (1 << i) != 0 && inp.get(i) != out.get(i) && PRESERVE_LOGS.fetch_add(1, Ordering::Relaxed) < 10 {
            log(&format!(
                "port_net: {}: the original changed {} ({:#x} -> {:#x})",
                PORTS[id].name,
                REG_NAMES[i],
                inp.get(i),
                out.get(i)
            ));
        }
    }
}

/// The stack arguments (up to three) of the call, as dwords.
unsafe fn sargs(a: *const u32, n: usize) -> [u32; 3] {
    let mut o = [0u32; 3];
    for (i, v) in o.iter_mut().enumerate().take(n) {
        *v = unsafe { *a.add(i) };
    }
    o
}

/// The shadow for the register / memory-reading ports: the original copy's EAX wins.
unsafe fn checked_eax(id: usize, p: &Pushad, args: [u32; 3], ours: u32) -> u32 {
    let Some((n, orig)) = shadowing(id) else { return ours };
    let inp = Regs::of(p);
    let mut o = inp;
    unsafe { call_orig(orig, &mut o, args, false) };
    check_preserved(id, &inp, &o);
    if o.eax != ours {
        mismatch(id, n, format!("in {:x?} args {args:x?}: original {:#x}, Rust {ours:#x}", inp, o.eax));
        return o.eax;
    }
    ours
}

/// EAX with AL replaced.
fn lo(eax: u32, al: u32) -> u32 {
    (eax & !0xFF) | al
}

/// A `fld dword` / `fstp dword` round trip: the bits, except that a signalling NaN is quieted.
fn q32(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

// --- the bit stream ---------------------------------------------------------------------------------
// Stream: +0 buffer pointer, +0xC size in bits, +0x10 cursor in bits. Bit p is bit (p & 7) of
// byte p >> 3 (the original reads the mask from the table 0xDC8558: 1, 2, 4 ... 0x80).

unsafe fn put_bit(buf: u32, pos: u32, bit: bool) {
    unsafe {
        let p = (buf as usize).wrapping_add((pos >> 3) as usize);
        let m = 1u8 << (pos & 7);
        wb(p, if bit { rb(p) | m } else { rb(p) & !m });
    }
}

unsafe fn get_bit(buf: u32, pos: u32) -> bool {
    unsafe { rb((buf as usize).wrapping_add((pos >> 3) as usize)) & (1u8 << (pos & 7)) != 0 }
}

/// `n` bits of `value`, LSB first, at the cursor; the cursor advances even when the write is
/// skipped (cursor + n > size, unsigned).
unsafe fn write_n(s: u32, value: u32, n: u32) {
    unsafe {
        let s = s as usize;
        let c = rd(s + 0x10);
        if c.wrapping_add(n) <= rd(s + 0xC) {
            let buf = rd(s);
            for i in 0..n {
                put_bit(buf, c.wrapping_add(i), (value >> i) & 1 != 0);
            }
        }
        wd(s + 0x10, rd(s + 0x10).wrapping_add(n));
    }
}

unsafe fn write_at(s: u32, start: u32, count: u32, value: u32) {
    unsafe {
        let s = s as usize;
        if start.wrapping_add(count) > rd(s + 0xC) || count > 0x20 || count == 0 {
            return;
        }
        let buf = rd(s);
        for k in 0..count {
            put_bit(buf, start.wrapping_add(k), (value >> k) & 1 != 0);
        }
    }
}

/// Bits `start .. start + count` of the stream, LSB first; 0 when out of range.
unsafe fn read_at(start: u32, count: u32, s: u32) -> u32 {
    unsafe {
        let s = s as usize;
        if start.wrapping_add(count) > rd(s + 0xC) || count > 0x20 {
            return 0;
        }
        let buf = rd(s);
        let mut v = 0u32;
        for k in 0..count {
            v |= (get_bit(buf, start.wrapping_add(k)) as u32) << k;
        }
        v
    }
}

/// Read `n` bits at the cursor; the cursor advances even when the read was rejected.
unsafe fn read_n(s: u32, n: u32) -> u32 {
    unsafe {
        let c = rd(s as usize + 0x10);
        let v = read_at(c, n, s);
        wd(s as usize + 0x10, c.wrapping_add(n));
        v
    }
}

/// A copy of `len` bytes at `addr` (nothing for a length of 0, whatever the address).
unsafe fn snap_bytes(addr: usize, len: usize) -> Vec<u8> {
    if len == 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(addr as *const u8, len) }.to_vec() }
}

/// Writer shadow: Rust on a scratch copy of the stream header and buffer, the original on the
/// real stream, then a comparison. Without a shadow (finished, or an unbounded buffer) Rust runs
/// on the real stream.
unsafe fn shadow_writer(id: usize, p: &Pushad, stream: u32, args: [u32; 3], ours: impl FnOnce(u32)) {
    unsafe {
        let Some((n, orig)) = shadowing(id) else { return ours(stream) };
        let s = stream as usize;
        let size = rd(s + 0xC);
        let nb = (size as usize).div_ceil(8);
        if nb > MAX_STREAM_BYTES {
            return ours(stream);
        }
        let buf = rd(s);
        let mut sbuf = snap_bytes(buf as usize, nb);
        sbuf.reserve(1);
        let mut st = [0u32; 5];
        for (i, v) in st.iter_mut().enumerate() {
            *v = rd(s + i * 4);
        }
        let pre = st;
        st[0] = sbuf.as_mut_ptr() as u32;
        let stp = st.as_mut_ptr();
        ours(stp as u32);
        let inp = Regs::of(p);
        let mut o = inp;
        call_orig(orig, &mut o, args, false);
        check_preserved(id, &inp, &o);
        let mut bad = String::new();
        for i in 0..5 {
            let want = if i == 0 { pre[0] } else { std::ptr::read_volatile(stp.add(i)) };
            let got = rd(s + i * 4);
            if got != want {
                bad = format!("stream dword {i}: original {got:#x}, Rust {want:#x}");
            }
        }
        let real = snap_bytes(buf as usize, nb);
        if let Some(i) = (0..nb).find(|&i| real[i] != sbuf[i]) {
            bad = format!("buffer byte {i}: original {:#x}, Rust {:#x} (stream {:x?}, regs {:x?}, args {args:x?})", real[i], sbuf[i], pre, inp);
        }
        if !bad.is_empty() {
            mismatch(id, n, bad);
        }
    }
}

/// Reader shadow (see the module comment). `out` is the (address, length) of an output buffer the
/// reader fills (length 0 for none). Returns EAX.
unsafe fn shadow_reader(id: usize, p: &Pushad, stream: u32, args: [u32; 3], out: (usize, usize), ours: impl FnOnce() -> u32) -> u32 {
    unsafe {
        let Some((n, orig)) = shadowing(id) else { return ours() };
        let cp = stream as usize + 0x10;
        let cur = rd(cp);
        let pre_out = snap_bytes(out.0, out.1);
        let inp = Regs::of(p);
        let mut o = inp;
        call_orig(orig, &mut o, args, false);
        check_preserved(id, &inp, &o);
        let cur_o = rd(cp);
        let out_o = snap_bytes(out.0, out.1);
        wd(cp, cur);
        if out.1 > 0 {
            std::ptr::copy_nonoverlapping(pre_out.as_ptr(), out.0 as *mut u8, out.1);
        }
        let v = ours();
        let cur_r = rd(cp);
        let out_r = snap_bytes(out.0, out.1);
        if v != o.eax || cur_r != cur_o || out_r != out_o {
            mismatch(id, n, format!("in {inp:x?} args {args:x?} cursor {cur:#x}: original eax {:#x} cursor {cur_o:#x} out {out_o:x?}, Rust eax {v:#x} cursor {cur_r:#x} out {out_r:x?}", o.eax));
            wd(cp, cur_o);
            if out.1 > 0 {
                std::ptr::copy_nonoverlapping(out_o.as_ptr(), out.0 as *mut u8, out.1);
            }
            return o.eax;
        }
        v
    }
}

// --- 0x7FD030 net_bits_write_at (EAX = start, ECX = count, ESI = stream, [esp+4] = value, `ret 4`) ----

unsafe fn h_write_at(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let value = *a;
        shadow_writer(id, &p, p.esi, [value, 0, 0], |s| write_at(s, p.eax, p.ecx, value));
    }
}
port_thunk!(write_at_thunk, write_at_entry, WRITE_AT, h_write_at, "ret 4");

// --- 0x7FD0B0 net_bits_write_bit (ECX = stream, DL = bit; only bit 0 counts) --------------------------

unsafe fn h_write_bit(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        shadow_writer(id, &p, p.ecx, [0; 3], |s| write_n(s, p.edx, 1));
    }
}
port_thunk!(write_bit_thunk, write_bit_entry, WRITE_BIT, h_write_bit);

// --- the 8-bit writers 0x7FD100 / 0x7FD260 (movsx) / 0x7FD700 (EAX = stream, DL = value) ----------------

unsafe fn h_write_u8(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        shadow_writer(id, &p, p.eax, [0; 3], |s| write_n(s, p.edx & 0xFF, 8));
    }
}
port_thunk!(write_u8_thunk, write_u8_entry, WRITE_U8, h_write_u8);
port_thunk!(write_s8_thunk, write_s8_entry, WRITE_S8, h_write_u8);
port_thunk!(write_u8_dup_thunk, write_u8_dup_entry, WRITE_U8_DUP, h_write_u8);

// --- the 16-bit writers 0x7FD3C0 / 0x7FD490 (movsx) (ECX = stream, DX = value) --------------------------

unsafe fn h_write_u16(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        shadow_writer(id, &p, p.ecx, [0; 3], |s| write_n(s, p.edx & 0xFFFF, 16));
    }
}
port_thunk!(write_u16_thunk, write_u16_entry, WRITE_U16, h_write_u16);
port_thunk!(write_s16_thunk, write_s16_entry, WRITE_S16, h_write_u16);

// --- the 32-bit writers 0x7FD560 / 0x7FD630 (ECX = stream, EDX = value), 0x7FD860 (stack, `ret 4`) ----

unsafe fn h_write_u32(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        shadow_writer(id, &p, p.ecx, [0; 3], |s| write_n(s, p.edx, 32));
    }
}
port_thunk!(write_u32_thunk, write_u32_entry, WRITE_U32, h_write_u32);
port_thunk!(write_u32_dup_thunk, write_u32_dup_entry, WRITE_U32_DUP, h_write_u32);

unsafe fn h_write_u32_stack(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let v = *a;
        shadow_writer(id, &p, p.ecx, [v, 0, 0], |s| write_n(s, v, 32));
    }
}
port_thunk!(write_u32_stack_thunk, write_u32_stack_entry, WRITE_U32_STACK, h_write_u32_stack, "ret 4");

// --- 0x7FD870 / 0x7FD8A0 net_bits_write_vec3 / vec2 (ECX = stream, ESI = float*) --------------------------
// Each float goes through `fld dword` / `fstp dword` (a signalling NaN is quieted), then the
// 32-bit writer.

unsafe fn write_floats(s: u32, src: u32, n: usize) {
    for i in 0..n {
        unsafe { write_n(s, q32(rd(src as usize + i * 4)), 32) };
    }
}

unsafe fn h_write_vec3(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        shadow_writer(id, &p, p.ecx, [0; 3], |s| write_floats(s, p.esi, 3));
    }
}
port_thunk!(write_vec3_thunk, write_vec3_entry, WRITE_VEC3, h_write_vec3);

unsafe fn h_write_vec2(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        shadow_writer(id, &p, p.ecx, [0; 3], |s| write_floats(s, p.esi, 2));
    }
}
port_thunk!(write_vec2_thunk, write_vec2_entry, WRITE_VEC2, h_write_vec2);

// --- 0x7FD8C0 net_bits_write_bytes (EAX = stream, EDI = count, [esp+4] = source, `ret 4`) -----------------

unsafe fn h_write_bytes(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let src = *a;
        shadow_writer(id, &p, p.eax, [src, 0, 0], |s| {
            for i in 0..p.edi {
                write_n(s, rb((src as usize).wrapping_add(i as usize)) as u32, 8);
            }
        });
    }
}
port_thunk!(write_bytes_thunk, write_bytes_entry, WRITE_BYTES, h_write_bytes, "ret 4");

// --- 0x7FD8F0 net_bits_read_at (EAX = start, ESI = count, EDX = stream -> EAX) -----------------------------

unsafe fn h_read_at(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let ours = read_at(p.eax, p.esi, p.edx);
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(read_at_thunk, read_at_entry, READ_AT, h_read_at);

// --- 0x7FD940 net_bits_read_n (EAX = count, EDI = stream -> EAX) ----------------------------------------

unsafe fn h_read_n(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = shadow_reader(id, &p, p.edi, [0; 3], (0, 0), || read_n(p.edi, p.eax));
    }
}
port_thunk!(read_n_thunk, read_n_entry, READ_N, h_read_n);

// --- the typed readers (EDI = stream -> EAX raw bits): 8 / 16 / 32 bits ------------------------------------

unsafe fn h_read8(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = shadow_reader(id, &p, p.edi, [0; 3], (0, 0), || read_n(p.edi, 8));
    }
}
unsafe fn h_read16(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = shadow_reader(id, &p, p.edi, [0; 3], (0, 0), || read_n(p.edi, 16));
    }
}
unsafe fn h_read32(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = shadow_reader(id, &p, p.edi, [0; 3], (0, 0), || read_n(p.edi, 32));
    }
}
port_thunk!(read_u8_thunk, read_u8_entry, READ_U8, h_read8);
port_thunk!(read_s8_thunk, read_s8_entry, READ_S8, h_read8);
port_thunk!(read_u8_dup_thunk, read_u8_dup_entry, READ_U8_DUP, h_read8);
port_thunk!(read_u16_thunk, read_u16_entry, READ_U16, h_read16);
port_thunk!(read_s16_thunk, read_s16_entry, READ_S16, h_read16);
port_thunk!(read_u32_thunk, read_u32_entry, READ_U32, h_read32);
port_thunk!(read_s32_thunk, read_s32_entry, READ_S32, h_read32);

// --- 0x7FD960 net_bits_read_bit (ECX = stream -> AL) ----------------------------------------------------------
// EAX = (new cursor & ~0xFF) | bit: the cursor is in EAX when `setne al` runs.

unsafe fn h_read_bit(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = shadow_reader(id, &p, p.ecx, [0; 3], (0, 0), || {
            let s = p.ecx as usize;
            let c = rd(s + 0x10);
            let bit = c.wrapping_add(1) <= rd(s + 0xC) && get_bit(rd(s), c);
            wd(s + 0x10, c.wrapping_add(1));
            lo(c.wrapping_add(1), bit as u32)
        });
    }
}
port_thunk!(read_bit_thunk, read_bit_entry, READ_BIT, h_read_bit);

// --- 0x7FDA90 net_bits_read_f32 (EDI = stream -> ST0; EAX = the raw bits) ------------------------------------

unsafe fn h_read_f32(id: usize, r: &mut Pushad, _a: *const u32) -> f32 {
    unsafe {
        let p = *r;
        let Some((n, orig)) = shadowing(id) else {
            let bits = read_n(p.edi, 32);
            r.eax = bits;
            return f32::from_bits(q32(bits));
        };
        let cp = p.edi as usize + 0x10;
        let cur = rd(cp);
        let inp = Regs::of(&p);
        let mut o = inp;
        let st0 = call_orig(orig, &mut o, [0; 3], true) as f32;
        check_preserved(id, &inp, &o);
        let cur_o = rd(cp);
        wd(cp, cur);
        let bits = read_n(p.edi, 32);
        let ours = f32::from_bits(q32(bits));
        if bits != o.eax || rd(cp) != cur_o || ours.to_bits() != st0.to_bits() {
            mismatch(id, n, format!("cursor {cur:#x}: original eax {:#x} st0 {:#x} cursor {cur_o:#x}, Rust eax {bits:#x} cursor {:#x}", o.eax, st0.to_bits(), rd(cp)));
            wd(cp, cur_o);
            r.eax = o.eax;
            return st0;
        }
        r.eax = bits;
        ours
    }
}
port_thunk_f32!(read_f32_thunk, read_f32_entry, READ_F32, h_read_f32);

// --- 0x7FDAC0 / 0x7FDB20 net_bits_read_vec3 / vec2 (EDI = stream, [esp+4] = out float*, `ret 4`) ----------
// The first float is stored as the raw dword, the others through `fld dword` / `fstp dword`.
// EAX is the last raw read.

unsafe fn read_floats(stream: u32, out: u32, n: usize) -> u32 {
    unsafe {
        let mut v = 0;
        for i in 0..n {
            v = read_n(stream, 32);
            wd(out as usize + i * 4, if i == 0 { v } else { q32(v) });
        }
        v
    }
}

unsafe fn h_read_vec3(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let out = *a;
        r.eax = shadow_reader(id, &p, p.edi, [out, 0, 0], (out as usize, 12), || read_floats(p.edi, out, 3));
    }
}
port_thunk!(read_vec3_thunk, read_vec3_entry, READ_VEC3, h_read_vec3, "ret 4");

unsafe fn h_read_vec2(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let out = *a;
        r.eax = shadow_reader(id, &p, p.edi, [out, 0, 0], (out as usize, 8), || read_floats(p.edi, out, 2));
    }
}
port_thunk!(read_vec2_thunk, read_vec2_entry, READ_VEC2, h_read_vec2, "ret 4");

// --- 0x7FDB70 net_bits_read_bytes ([esp+4] = stream, [esp+8] = out, [esp+0xC] = count, `ret 0xC`) --------
// EAX is the last 8-bit read (unchanged for a count of 0).

unsafe fn h_read_bytes(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let [stream, out, count] = sargs(a, 3);
        // The output buffer is only snapshotted when it is a sane size.
        let len = if count <= 0x1_0000 { count as usize } else { 0 };
        r.eax = shadow_reader(id, &p, stream, [stream, out, count], (out as usize, len), || {
            let mut v = p.eax;
            for i in 0..count {
                v = read_n(stream, 8);
                wb((out as usize).wrapping_add(i as usize), v as u8);
            }
            v
        });
    }
}
port_thunk!(read_bytes_thunk, read_bytes_entry, READ_BYTES, h_read_bytes, "ret 12");

// --- 0x804F70 net_handle_to_net_id (ECX = handle -> AX) ---------------------------------------------------------

/// The object of a handle (slot table 0x2149C64, 16-byte entries): the table entry, and whether
/// its handle (+0x44) matches.
unsafe fn slot(h: u32) -> (u32, bool) {
    unsafe {
        let o = rd(HANDLES + (h as usize & 0xFFF) * 16);
        (o, o != 0 && rd(o as usize + 0x44) == h)
    }
}

/// Handle 0 gives `or ax, 0xFFFF` (the upper half of EAX stays); a stale handle or no net record
/// gives 0xFFFF; else the u16 at record + 2, zero-extended.
unsafe fn handle_to_id(eax: u32, h: u32) -> u32 {
    unsafe {
        if h == 0 {
            return eax | 0xFFFF;
        }
        let (o, ok) = slot(h);
        if !ok {
            return 0xFFFF;
        }
        let rec = rd(o as usize + 0xB0);
        if rec == 0 { 0xFFFF } else { *((rec as usize + 2) as *const u16) as u32 }
    }
}

unsafe fn h_handle_to_id(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], handle_to_id(p.eax, p.ecx));
    }
}
port_thunk!(handle_to_id_thunk, handle_to_id_entry, HANDLE_TO_ID, h_handle_to_id);

// --- 0x808E30 net_remote_handle_map_find (ESI = map, [esp+4] = key, [esp+8] = create u8, `ret 8`) -------------

/// `idiv dword ptr [divisor]` on `h` sign-extended (a zero divisor faults, as in the original).
fn idiv_rem(h: i32, div: i32) -> i32 {
    let rem: i32;
    unsafe {
        std::arch::asm!("cdq", "idiv {d}", d = in(reg) div, inout("eax") h => _, out("edx") rem, options(nomem, nostack));
    }
    rem
}

/// The bucket of `key`: all 32-bit wrapping, `sar` arithmetic, then a signed `idiv`.
unsafe fn map_bucket(map: u32, key: u32) -> u32 {
    let k = key as i32;
    let a = (!k.wrapping_shl(15)).wrapping_add(k);
    let b = (a >> 10) ^ a;
    let b = b.wrapping_mul(9);
    let c = b ^ (b >> 6);
    let d = c.wrapping_add(!c.wrapping_shl(11));
    let h = (d >> 16) ^ d;
    idiv_rem(h, unsafe { rd(map as usize + 0x60) } as i32) as u32
}

unsafe fn map_lookup(map: u32, key: u32) -> u32 {
    unsafe {
        let bucket = map_bucket(map, key);
        let mut node = rd((rd(map as usize + 0x5C) as usize).wrapping_add(bucket as usize * 4));
        while node != 0 {
            if key == rd(node as usize + 0xC) {
                return node;
            }
            node = rd(node as usize + 4);
        }
        0
    }
}

/// A miss with `create` set and `[map+0x1C] > 0` (signed) allocates a node from the pool at
/// map + 0x20 (0x809220, EDI = pool) and links it at the head of its bucket. A null node faults
/// on the first store, as in the original.
unsafe fn map_find(map: u32, key: u32, create: u8) -> u32 {
    unsafe {
        let node = map_lookup(map, key);
        if node != 0 || create == 0 || (rd(map as usize + 0x1C) as i32) <= 0 {
            return node;
        }
        let bucket = map_bucket(map, key);
        let n = call_regs(POOL_ALLOC, 0, 0, 0, map, map + 0x20, &[]);
        wd(n as usize + 0xC, key);
        wd(n as usize + 0x10, bucket);
        let slotp = (rd(map as usize + 0x5C) as usize).wrapping_add(bucket as usize * 4);
        wd(n as usize + 4, rd(slotp));
        wd(n as usize + 8, 0);
        let slotp = (rd(map as usize + 0x5C) as usize).wrapping_add(bucket as usize * 4);
        let head = rd(slotp);
        if head != 0 {
            wd(head as usize + 8, n);
        }
        let slotp = (rd(map as usize + 0x5C) as usize).wrapping_add(bucket as usize * 4);
        wd(slotp, n);
        n
    }
}

unsafe fn h_map_find(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let [key, create, _] = sargs(a, 2);
        let creates = create as u8 != 0 && (rd(p.esi as usize + 0x1C) as i32) > 0 && map_lookup(p.esi, key) == 0;
        if creates {
            // The pool allocator takes a lock and changes the pool: not shadowed.
            let _ = shadowing(id);
            r.eax = map_find(p.esi, key, create as u8);
        } else {
            r.eax = checked_eax(id, &p, [key, create, 0], map_find(p.esi, key, create as u8));
        }
    }
}
port_thunk!(map_find_thunk, map_find_entry, MAP_FIND, h_map_find, "ret 8");

// --- 0x804FB0 net_id_to_handle (AX = net id -> EAX = handle or 0) -----------------------------------------------

unsafe fn id_to_handle(eax: u32) -> u32 {
    unsafe {
        if eax & 0xFFFF == 0xFFFF {
            return 0;
        }
        let node = map_find(rd(NET_ID_MAP), eax & 0xFFFF, 0);
        if node == 0 {
            return 0;
        }
        let rec = rd(node as usize);
        if rec == 0 { 0 } else { rd(rec as usize + 4) }
    }
}

unsafe fn h_id_to_handle(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], id_to_handle(p.eax));
    }
}
port_thunk!(id_to_handle_thunk, id_to_handle_entry, ID_TO_HANDLE, h_id_to_handle);

// --- 0x8066C0 net_handle_owned_locally (ECX = handle -> AL) and 0x806760 (ECX = record* -> AL) ---------------
// AL = 1 in every unknown case. EAX otherwise holds whatever the original had loaded into it.

unsafe fn handle_owned(eax: u32, h: u32) -> u32 {
    unsafe {
        if h == 0 {
            return lo(eax, 1);
        }
        let (o, ok) = slot(h);
        if o == 0 {
            return 1;
        }
        if !ok {
            return lo(o, 1);
        }
        let rec = rd(o as usize + 0xB0);
        if rec == 0 {
            return 1;
        }
        let sess = rd(SESSION);
        if sess == 0 {
            return lo(rec, 1);
        }
        let local = rd(sess as usize + 0x408);
        if local == 0 {
            return lo(rec, 1);
        }
        if rb(rec as usize) != rb(local as usize + 0x1B0) {
            return lo(rec, 0);
        }
        lo(rec, (rb(rec as usize + 0x67C) & 1 == 0) as u32)
    }
}

unsafe fn h_handle_owned(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], handle_owned(p.eax, p.ecx));
    }
}
port_thunk!(handle_owned_thunk, handle_owned_entry, HANDLE_OWNED, h_handle_owned);

unsafe fn record_owned(eax: u32, rec: u32) -> u32 {
    unsafe {
        if rec == 0 {
            return lo(eax, 1);
        }
        let sess = rd(SESSION);
        if sess == 0 {
            return 1;
        }
        let local = rd(sess as usize + 0x408);
        if local == 0 {
            return 1;
        }
        if rb(rec as usize) != rb(local as usize + 0x1B0) {
            return 0;
        }
        (rb(rec as usize + 0x67C) & 1 == 0) as u32
    }
}

unsafe fn h_record_owned(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], record_owned(p.eax, p.ecx));
    }
}
port_thunk!(record_owned_thunk, record_owned_entry, RECORD_OWNED, h_record_owned);

// --- 0x80FE80 session_find_conn_by_id (EAX = session, DL = id -> EAX = connection or 0) --------------------------

unsafe fn find_conn(session: u32, id: u32) -> u32 {
    unsafe {
        let head = rd(session as usize + 0x400);
        if head == 0 {
            return 0;
        }
        let mut node = head;
        loop {
            if rb(node as usize + 0x1B0) == id as u8 {
                return node;
            }
            node = rd(node as usize + 0x22CC);
            if node == head || node == 0 {
                return 0;
            }
        }
    }
}

unsafe fn h_find_conn(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], find_conn(p.eax, p.edx));
    }
}
port_thunk!(find_conn_thunk, find_conn_entry, FIND_CONN, h_find_conn);

// --- 0x8176E0 net_guid_equal (ECX = a, EDX = b -> AL) -------------------------------------------------------------
// EAX: 1 when equal; otherwise the sign of (b byte - a byte) at the first difference decides
// 0 / 0xFFFFFF00; a null b keeps the caller's upper bytes.

unsafe fn guid_equal(eax: u32, a: u32, b: u32) -> u32 {
    unsafe {
        if b == 0 {
            return lo(eax, 0);
        }
        for i in 0..16usize {
            let (x, y) = (rb(b as usize + i), rb(a as usize + i));
            if x != y {
                return if x > y { 0 } else { 0xFFFF_FF00 };
            }
        }
        1
    }
}

unsafe fn h_guid_equal(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], guid_equal(p.eax, p.ecx, p.edx));
    }
}
port_thunk!(guid_equal_thunk, guid_equal_entry, GUID_EQUAL, h_guid_equal);

// --- 0x814B80 net_packet_state_allows (EAX = record, ECX = conn -> AL) ---------------------------------------------
// EAX holds the zero-extended mask word; only AL is replaced.

unsafe fn state_allows(rec: u32, conn: u32) -> u32 {
    unsafe {
        let mask = rd(rec as usize + 0x10) & 0xFFFF;
        if mask == 0xFFFF {
            return lo(mask, 1);
        }
        if conn == 0 {
            return lo(mask, 0);
        }
        let bit = 1u32 << (rd(conn as usize + 0x3C) & 31);
        lo(mask, (mask & bit != 0) as u32)
    }
}

unsafe fn h_state_allows(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], state_allows(p.eax, p.ecx));
    }
}
port_thunk!(state_allows_thunk, state_allows_entry, STATE_ALLOWS, h_state_allows);

// --- 0x814B00 / 0x814B40 net_packet_route_allows_send / _recv (EAX = record, ECX = connection -> AL) ----------
// P = [[conn+0x1AC]+0x404]. EAX is r (= [rec+0xC]) or the connection's [+0x1AC] with AL replaced.

unsafe fn route_allows(rec: u32, conn: u32, send: bool) -> u32 {
    unsafe {
        let r = rd(rec as usize + 0xC);
        // "own": conn == P
        let own = |e: u32| conn == rd(e as usize + 0x404);
        if r == 0 {
            if conn == 0 {
                return if send { 0 } else { 1 };
            }
            let e = rd(conn as usize + 0x1AC);
            return if send { own(e) as u32 } else { !own(e) as u32 };
        }
        if r != 1 {
            return lo(r, 1);
        }
        if conn == 0 {
            return if send { 1 } else { 0 };
        }
        let e = rd(conn as usize + 0x1AC);
        if send { lo(e, !own(e) as u32) } else { lo(e, own(e) as u32) }
    }
}

unsafe fn h_route_send(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], route_allows(p.eax, p.ecx, true));
    }
}
port_thunk!(route_send_thunk, route_send_entry, ROUTE_SEND, h_route_send);

unsafe fn h_route_recv(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], route_allows(p.eax, p.ecx, false));
    }
}
port_thunk!(route_recv_thunk, route_recv_entry, ROUTE_RECV, h_route_recv);

// --- 0x7F8300 net_conn_loss_ratio (EAX = conn -> ST0 f32) ---------------------------------------------------------

/// `fild` of a signed dword, plus 2^32 (the constant at 0xE5BFD8) when negative.
unsafe fn fild_u32(v: u32) -> f64 {
    unsafe { v as i32 as f64 + if (v as i32) < 0 { rf(TWO_POW_32) as f64 } else { 0.0 } }
}

unsafe fn loss_ratio(conn: u32) -> f32 {
    unsafe {
        if conn == 0 {
            return 0.0;
        }
        let mut n = rd(conn as usize + 0x15C0);
        let now = rd(NOW);
        let mut e = conn as usize + 0x1C4;
        let (mut total, mut lost) = (0u32, 0u32);
        if n == 0 {
            return 0.0;
        }
        while n != 0 {
            if rb(e + 5) == 0 && rd(e).wrapping_add(0x3E8) < now {
                total = total.wrapping_add(1);
                if rb(e + 4) == 0 {
                    lost = lost.wrapping_add(1);
                }
            }
            e = e.wrapping_add(0x10);
            n -= 1;
        }
        if total < 0x14 {
            return 0.0;
        }
        (fild_u32(lost) / fild_u32(total)) as f32
    }
}

unsafe fn h_loss_ratio(id: usize, r: &mut Pushad, _a: *const u32) -> f32 {
    unsafe {
        let p = *r;
        let ours = loss_ratio(p.eax);
        let Some((n, orig)) = shadowing(id) else { return ours };
        let inp = Regs::of(&p);
        let mut o = inp;
        let theirs = call_orig(orig, &mut o, [0; 3], true) as f32;
        check_preserved(id, &inp, &o);
        if theirs.to_bits() != ours.to_bits() {
            mismatch(id, n, format!("conn {:#x}: original {:#x}, Rust {:#x}", p.eax, theirs.to_bits(), ours.to_bits()));
            return theirs;
        }
        ours
    }
}
port_thunk_f32!(loss_ratio_thunk, loss_ratio_entry, LOSS_RATIO, h_loss_ratio);

// --- the small leaves ---------------------------------------------------------------------------------------------------

/// 0x7FCA90 net_clock_ms: [session+8] when there is a session, else the offline clock.
unsafe fn h_clock_ms(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let s = rd(SESSION);
        let ours = if s != 0 { rd(s as usize + 8) } else { rd(CLOCK_OFFLINE) };
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(clock_ms_thunk, clock_ms_entry, CLOCK_MS, h_clock_ms);

/// Shadow for a leaf that only stores constants: Rust stores, the original stores, and the
/// regions are compared (the original ran last, so memory always carries its stores).
unsafe fn store_leaf(id: usize, p: &Pushad, regions: &[(usize, usize)], ours: impl FnOnce() -> u32) -> u32 {
    unsafe {
        let v = ours();
        let Some((n, orig)) = shadowing(id) else { return v };
        let mine: Vec<Vec<u8>> = regions.iter().map(|&(a, l)| snap_bytes(a, l)).collect();
        let inp = Regs::of(p);
        let mut o = inp;
        call_orig(orig, &mut o, [0; 3], false);
        check_preserved(id, &inp, &o);
        let theirs: Vec<Vec<u8>> = regions.iter().map(|&(a, l)| snap_bytes(a, l)).collect();
        if mine != theirs || o.eax != v {
            mismatch(id, n, format!("Rust stored {mine:x?} eax {v:#x}, original {theirs:x?} eax {:#x}", o.eax));
            return o.eax;
        }
        v
    }
}

/// 0x7FC920 net_clock_reset: zeroes the offline clock and the frame delta; EAX = 0.
unsafe fn h_clock_reset(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = store_leaf(id, &p, &[(CLOCK_OFFLINE, 4), (CLOCK_DELTA, 4)], || {
            wd(CLOCK_OFFLINE, 0);
            wd(CLOCK_DELTA, 0);
            0
        });
    }
}
port_thunk!(clock_reset_thunk, clock_reset_entry, CLOCK_RESET, h_clock_reset);

/// 0x7FC890 net_timeout_clear: flag byte = 0, deadline = -1; EAX untouched.
unsafe fn h_timeout_clear(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = store_leaf(id, &p, &[(TIMEOUT_FLAG, 1), (TIMEOUT_AT, 4)], || {
            wb(TIMEOUT_FLAG, 0);
            wd(TIMEOUT_AT, 0xFFFF_FFFF);
            p.eax
        });
    }
}
port_thunk!(timeout_clear_thunk, timeout_clear_entry, TIMEOUT_CLEAR, h_timeout_clear);

/// 0x80CEF0 session_state_in_1_6_8 (EAX = session -> AL). EAX after the subtractions is
/// state - 8 on a miss; 0 on a hit, then `mov al, 1`.
unsafe fn h_state_1_6_8(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let s = rd(p.eax as usize + 0x1C);
        let ours = if s == 1 || s == 6 || s == 8 { 1 } else { lo(s.wrapping_sub(8), 0) };
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(state_1_6_8_thunk, state_1_6_8_entry, STATE_1_6_8, h_state_1_6_8);

/// 0x80CF20 session_state_in_0_1 (EAX = session -> EAX 1 or 0).
unsafe fn h_state_0_1(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let s = rd(p.eax as usize + 0x1C);
        r.eax = checked_eax(id, &p, [0; 3], (s == 0 || s == 1) as u32);
    }
}
port_thunk!(state_0_1_thunk, state_0_1_entry, STATE_0_1, h_state_0_1);

/// 0x7FAEF0 net_conn_recent_ids_contains (EDX = conn, SI = id -> AL; the rest of EAX is the
/// caller's, as only AL is ever written).
unsafe fn h_recent_ids(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let c = p.edx as usize;
        let n = rb(c + 0x1DBA) as usize;
        let found = (0..n).any(|i| (rd(c + 0x1CBA + i * 2) & 0xFFFF) as u16 == p.esi as u16);
        r.eax = checked_eax(id, &p, [0; 3], lo(p.eax, found as u32));
    }
}
port_thunk!(recent_ids_thunk, recent_ids_entry, RECENT_IDS, h_recent_ids);

/// The connection owning a net record: id byte [rec] looked up in the session (a null record
/// gives 0; a missing session faults, as in the original).
unsafe fn owner_conn_of_record(rec: u32) -> u32 {
    unsafe {
        if rec == 0 {
            return 0;
        }
        find_conn(rd(SESSION), rb(rec as usize) as u32)
    }
}

/// 0x807230 net_object_owner_conn (EAX = object -> EAX = connection or 0).
unsafe fn h_obj_owner_conn(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let ours = if p.eax == 0 { 0 } else { owner_conn_of_record(rd(p.eax as usize + 0xB0)) };
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(obj_owner_conn_thunk, obj_owner_conn_entry, OBJ_OWNER_CONN, h_obj_owner_conn);

/// 0x807250 net_handle_owner_conn (ECX = handle -> EAX = connection or 0).
unsafe fn h_handle_owner_conn(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let ours = if p.ecx == 0 {
            0
        } else {
            let (o, ok) = slot(p.ecx);
            if ok { owner_conn_of_record(rd(o as usize + 0xB0)) } else { 0 }
        };
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(handle_owner_conn_thunk, handle_owner_conn_entry, HANDLE_OWNER_CONN, h_handle_owner_conn);

/// 0x807680 net_registration_find_field (ESI = registration, EDI = key -> EAX = entry or 0): the
/// entries are 0x44 bytes from reg + 0x60, the key is the first dword, the entry returned
/// starts 0x40 before the key.
unsafe fn h_reg_find_field(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let n = rd(p.esi as usize + 0x18);
        let ours = (0..n).find(|&i| rd(p.esi as usize + 0x60 + i as usize * 0x44) == p.edi).map_or(0, |i| p.esi + 0x20 + i * 0x44);
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(reg_find_field_thunk, reg_find_field_entry, REG_FIND_FIELD, h_reg_find_field);

// --- install ------------------------------------------------------------------------------------------------------------------

/// Re-aim the rel32 of the `call` / `jmp` at `off` in a relocated copy.
unsafe fn set_rel32(copy: usize, off: usize, dest: usize) {
    let rel = (dest as i32).wrapping_sub((copy + off + 5) as i32);
    unsafe { std::ptr::write_unaligned((copy + off + 1) as *mut i32, rel) };
}

/// The copies' calls (and tail jumps) into other ported functions: (port, offset, callee port).
/// They are pointed at the callee's original copy, so a shadow never runs Rust code.
const DEPS: [(usize, usize, usize); 25] = [
    (READ_N, 0xB, READ_AT),
    (READ_U8, 0xE, READ_AT),
    (READ_S8, 0xE, READ_AT),
    (READ_U16, 0xE, READ_AT),
    (READ_S16, 0xE, READ_AT),
    (READ_U32, 0xE, READ_AT),
    (READ_S32, 0xE, READ_AT),
    (READ_U8_DUP, 0xE, READ_AT),
    (READ_F32, 0xF, READ_AT),
    (READ_VEC3, 0x13, READ_AT),
    (READ_VEC3, 0x27, READ_AT),
    (READ_VEC3, 0x44, READ_AT),
    (READ_VEC2, 0x13, READ_AT),
    (READ_VEC2, 0x27, READ_AT),
    (READ_BYTES, 0x1C, READ_AT),
    (WRITE_U32_STACK, 0x4, WRITE_U32),
    (WRITE_VEC3, 0x9, WRITE_U32),
    (WRITE_VEC3, 0x17, WRITE_U32),
    (WRITE_VEC3, 0x28, WRITE_U32),
    (WRITE_VEC2, 0x9, WRITE_U32),
    (WRITE_VEC2, 0x1A, WRITE_U32),
    (WRITE_BYTES, 0x13, WRITE_U8),
    (ID_TO_HANDLE, 0x16, MAP_FIND),
    (OBJ_OWNER_CONN, 0x18, FIND_CONN),
    (HANDLE_OWNER_CONN, 0x31, FIND_CONN),
];

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let list: [(usize, &[u8], &[usize], extern "C" fn()); COUNT] = [
        (WRITE_AT, &B_WRITE_AT, &B_WRITE_AT_CALLS, write_at_thunk),
        (WRITE_BIT, &B_WRITE_BIT, &B_WRITE_BIT_CALLS, write_bit_thunk),
        (WRITE_U8, &B_WRITE_U8, &B_WRITE_U8_CALLS, write_u8_thunk),
        (WRITE_S8, &B_WRITE_S8, &B_WRITE_S8_CALLS, write_s8_thunk),
        (WRITE_U8_DUP, &B_WRITE_U8_DUP, &B_WRITE_U8_DUP_CALLS, write_u8_dup_thunk),
        (WRITE_U16, &B_WRITE_U16, &B_WRITE_U16_CALLS, write_u16_thunk),
        (WRITE_S16, &B_WRITE_S16, &B_WRITE_S16_CALLS, write_s16_thunk),
        (WRITE_U32, &B_WRITE_U32, &B_WRITE_U32_CALLS, write_u32_thunk),
        (WRITE_U32_DUP, &B_WRITE_U32_DUP, &B_WRITE_U32_DUP_CALLS, write_u32_dup_thunk),
        (WRITE_U32_STACK, &B_WRITE_U32_STACK, &B_WRITE_U32_STACK_CALLS, write_u32_stack_thunk),
        (WRITE_VEC3, &B_WRITE_VEC3, &B_WRITE_VEC3_CALLS, write_vec3_thunk),
        (WRITE_VEC2, &B_WRITE_VEC2, &B_WRITE_VEC2_CALLS, write_vec2_thunk),
        (WRITE_BYTES, &B_WRITE_BYTES, &B_WRITE_BYTES_CALLS, write_bytes_thunk),
        (READ_AT, &B_READ_AT, &B_READ_AT_CALLS, read_at_thunk),
        (READ_N, &B_READ_N, &B_READ_N_CALLS, read_n_thunk),
        (READ_BIT, &B_READ_BIT, &B_READ_BIT_CALLS, read_bit_thunk),
        (READ_U8, &B_READ_U8, &B_READ_U8_CALLS, read_u8_thunk),
        (READ_S8, &B_READ_S8, &B_READ_S8_CALLS, read_s8_thunk),
        (READ_U16, &B_READ_U16, &B_READ_U16_CALLS, read_u16_thunk),
        (READ_S16, &B_READ_S16, &B_READ_S16_CALLS, read_s16_thunk),
        (READ_U32, &B_READ_U32, &B_READ_U32_CALLS, read_u32_thunk),
        (READ_S32, &B_READ_S32, &B_READ_S32_CALLS, read_s32_thunk),
        (READ_U8_DUP, &B_READ_U8_DUP, &B_READ_U8_DUP_CALLS, read_u8_dup_thunk),
        (READ_F32, &B_READ_F32, &B_READ_F32_CALLS, read_f32_thunk),
        (READ_VEC3, &B_READ_VEC3, &B_READ_VEC3_CALLS, read_vec3_thunk),
        (READ_VEC2, &B_READ_VEC2, &B_READ_VEC2_CALLS, read_vec2_thunk),
        (READ_BYTES, &B_READ_BYTES, &B_READ_BYTES_CALLS, read_bytes_thunk),
        (HANDLE_TO_ID, &B_HANDLE_TO_ID, &B_HANDLE_TO_ID_CALLS, handle_to_id_thunk),
        (ID_TO_HANDLE, &B_ID_TO_HANDLE, &B_ID_TO_HANDLE_CALLS, id_to_handle_thunk),
        (MAP_FIND, &B_MAP_FIND, &B_MAP_FIND_CALLS, map_find_thunk),
        (HANDLE_OWNED, &B_HANDLE_OWNED, &B_HANDLE_OWNED_CALLS, handle_owned_thunk),
        (RECORD_OWNED, &B_RECORD_OWNED, &B_RECORD_OWNED_CALLS, record_owned_thunk),
        (FIND_CONN, &B_FIND_CONN, &B_FIND_CONN_CALLS, find_conn_thunk),
        (GUID_EQUAL, &B_GUID_EQUAL, &B_GUID_EQUAL_CALLS, guid_equal_thunk),
        (STATE_ALLOWS, &B_STATE_ALLOWS, &B_STATE_ALLOWS_CALLS, state_allows_thunk),
        (ROUTE_SEND, &B_ROUTE_SEND, &B_ROUTE_SEND_CALLS, route_send_thunk),
        (ROUTE_RECV, &B_ROUTE_RECV, &B_ROUTE_RECV_CALLS, route_recv_thunk),
        (LOSS_RATIO, &B_LOSS_RATIO, &B_LOSS_RATIO_CALLS, loss_ratio_thunk),
        (CLOCK_MS, &B_CLOCK_MS, &B_CLOCK_MS_CALLS, clock_ms_thunk),
        (CLOCK_RESET, &B_CLOCK_RESET, &B_CLOCK_RESET_CALLS, clock_reset_thunk),
        (TIMEOUT_CLEAR, &B_TIMEOUT_CLEAR, &B_TIMEOUT_CLEAR_CALLS, timeout_clear_thunk),
        (STATE_1_6_8, &B_STATE_1_6_8, &B_STATE_1_6_8_CALLS, state_1_6_8_thunk),
        (STATE_0_1, &B_STATE_0_1, &B_STATE_0_1_CALLS, state_0_1_thunk),
        (RECENT_IDS, &B_RECENT_IDS, &B_RECENT_IDS_CALLS, recent_ids_thunk),
        (OBJ_OWNER_CONN, &B_OBJ_OWNER_CONN, &B_OBJ_OWNER_CONN_CALLS, obj_owner_conn_thunk),
        (HANDLE_OWNER_CONN, &B_HANDLE_OWNER_CONN, &B_HANDLE_OWNER_CONN_CALLS, handle_owner_conn_thunk),
        (REG_FIND_FIELD, &B_REG_FIND_FIELD, &B_REG_FIND_FIELD_CALLS, reg_find_field_thunk),
    ];
    // The shadow copies first: replacing patches the entry.
    for &(id, body, call_offsets, _) in &list {
        let port = &PORTS[id];
        match unsafe { detour::relocate(port.addr, body, call_offsets) } {
            Ok(copy) => port.original.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_net: {}: no shadow copy: {e:?}", port.name)),
        }
    }
    for &(id, off, dep) in &DEPS {
        let (copy, callee) = (PORTS[id].original.load(Ordering::Relaxed), PORTS[dep].original.load(Ordering::Relaxed));
        if copy != 0 && callee != 0 {
            unsafe { set_rel32(copy, off, callee) };
        } else if copy != 0 {
            // No copy of the callee: do not shadow this port rather than call Rust from the copy.
            PORTS[id].original.store(0, Ordering::Relaxed);
        }
    }
    let mut ok = 0;
    for &(id, body, _, thunk) in &list {
        if replace_pinned(PORTS[id].name, PORTS[id].addr, body, thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_net: replaced {ok}/{COUNT} with Rust"));
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
    if parts.is_empty() { String::new() } else { format!("port_net: {}", parts.join(", ")) }
}

// --- original bodies (tools/body_bytes.py) ----------------------------------------------------------------------------------

const B_WRITE_AT: [u8; 81] = [
    0x8B, 0x54, 0x24, 0x04, 0x57, 0x8D, 0x3C, 0x08, 0x3B, 0x7E, 0x0C, 0x77, 0x40, 0x83, 0xF9, 0x20,
    0x77, 0x3B, 0x85, 0xC9, 0x76, 0x37, 0x8B, 0xF9, 0x53, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x1E, 0x8B, 0xC8, 0xC1, 0xE9, 0x03, 0x03, 0xCB, 0x8B, 0xD8, 0x83, 0xE3, 0x07, 0xF6, 0xC2,
    0x01, 0x8A, 0x1C, 0x9D, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x19, 0xEB, 0x04, 0xF6, 0xD3,
    0x20, 0x19, 0xD1, 0xEA, 0x83, 0xC0, 0x01, 0x83, 0xEF, 0x01, 0x75, 0xD4, 0x5B, 0x5F, 0xC2, 0x04,
    0x00,
];
const B_WRITE_AT_CALLS: [usize; 0] = [];
const B_WRITE_BIT: [u8; 73] = [
    0x8B, 0x41, 0x10, 0x56, 0x8D, 0x70, 0x01, 0x3B, 0x71, 0x0C, 0x0F, 0xB6, 0xD2, 0x77, 0x34, 0x8B,
    0x31, 0xF6, 0xC2, 0x01, 0x8B, 0xD0, 0x74, 0x18, 0xC1, 0xEA, 0x03, 0x83, 0xE0, 0x07, 0x8A, 0x04,
    0x85, 0x58, 0x85, 0xDC, 0x00, 0x08, 0x04, 0x32, 0x03, 0xD6, 0x83, 0x41, 0x10, 0x01, 0x5E, 0xC3,
    0x83, 0xE0, 0x07, 0x8A, 0x04, 0x85, 0x58, 0x85, 0xDC, 0x00, 0xC1, 0xEA, 0x03, 0x03, 0xD6, 0xF6,
    0xD0, 0x20, 0x02, 0x83, 0x41, 0x10, 0x01, 0x5E, 0xC3,
];
const B_WRITE_BIT_CALLS: [usize; 0] = [];
const B_WRITE_U8: [u8; 340] = [
    0x56, 0x8B, 0x70, 0x10, 0x8D, 0x4E, 0x08, 0x3B, 0x48, 0x0C, 0x0F, 0xB6, 0xD2, 0x0F, 0x87, 0x3B,
    0x01, 0x00, 0x00, 0x53, 0x57, 0x8B, 0x38, 0x8B, 0xCE, 0xC1, 0xE9, 0x03, 0x03, 0xF9, 0x8B, 0xCE,
    0x83, 0xE1, 0x07, 0xF6, 0xC2, 0x01, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08,
    0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x01, 0x8B, 0xF9, 0xC1, 0xEF,
    0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6,
    0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E,
    0x02, 0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85,
    0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20,
    0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x03, 0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07,
    0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F,
    0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x04, 0x8B, 0xF9, 0xC1, 0xEF, 0x03,
    0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2,
    0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x05,
    0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC,
    0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F,
    0x8B, 0x18, 0x8D, 0x4E, 0x06, 0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A,
    0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB,
    0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8D, 0x4E, 0x07, 0x8B, 0x30, 0x5F, 0xF6, 0xC2, 0x02, 0x5B, 0x8B,
    0xD1, 0x74, 0x18, 0xC1, 0xEA, 0x03, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00,
    0x08, 0x0C, 0x32, 0x03, 0xD6, 0x83, 0x40, 0x10, 0x08, 0x5E, 0xC3, 0x83, 0xE1, 0x07, 0x8A, 0x0C,
    0x8D, 0x58, 0x85, 0xDC, 0x00, 0xC1, 0xEA, 0x03, 0x03, 0xD6, 0xF6, 0xD1, 0x20, 0x0A, 0x83, 0x40,
    0x10, 0x08, 0x5E, 0xC3,
];
const B_WRITE_U8_CALLS: [usize; 0] = [];
const B_WRITE_S8: [u8; 340] = [
    0x56, 0x8B, 0x70, 0x10, 0x8D, 0x4E, 0x08, 0x3B, 0x48, 0x0C, 0x0F, 0xBE, 0xD2, 0x0F, 0x87, 0x3B,
    0x01, 0x00, 0x00, 0x53, 0x57, 0x8B, 0x38, 0x8B, 0xCE, 0xC1, 0xE9, 0x03, 0x03, 0xF9, 0x8B, 0xCE,
    0x83, 0xE1, 0x07, 0xF6, 0xC2, 0x01, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08,
    0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x01, 0x8B, 0xF9, 0xC1, 0xEF,
    0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6,
    0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E,
    0x02, 0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85,
    0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20,
    0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x03, 0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07,
    0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F,
    0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x04, 0x8B, 0xF9, 0xC1, 0xEF, 0x03,
    0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2,
    0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x05,
    0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC,
    0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F,
    0x8B, 0x18, 0x8D, 0x4E, 0x06, 0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A,
    0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB,
    0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8D, 0x4E, 0x07, 0x8B, 0x30, 0x5F, 0xF6, 0xC2, 0x02, 0x5B, 0x8B,
    0xD1, 0x74, 0x18, 0xC1, 0xEA, 0x03, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00,
    0x08, 0x0C, 0x32, 0x03, 0xD6, 0x83, 0x40, 0x10, 0x08, 0x5E, 0xC3, 0x83, 0xE1, 0x07, 0x8A, 0x0C,
    0x8D, 0x58, 0x85, 0xDC, 0x00, 0xC1, 0xEA, 0x03, 0x03, 0xD6, 0xF6, 0xD1, 0x20, 0x0A, 0x83, 0x40,
    0x10, 0x08, 0x5E, 0xC3,
];
const B_WRITE_S8_CALLS: [usize; 0] = [];
const B_WRITE_U8_DUP: [u8; 340] = [
    0x56, 0x8B, 0x70, 0x10, 0x8D, 0x4E, 0x08, 0x3B, 0x48, 0x0C, 0x0F, 0xB6, 0xD2, 0x0F, 0x87, 0x3B,
    0x01, 0x00, 0x00, 0x53, 0x57, 0x8B, 0x38, 0x8B, 0xCE, 0xC1, 0xE9, 0x03, 0x03, 0xF9, 0x8B, 0xCE,
    0x83, 0xE1, 0x07, 0xF6, 0xC2, 0x01, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08,
    0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x01, 0x8B, 0xF9, 0xC1, 0xEF,
    0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6,
    0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E,
    0x02, 0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85,
    0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20,
    0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x03, 0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07,
    0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F,
    0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x04, 0x8B, 0xF9, 0xC1, 0xEF, 0x03,
    0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2,
    0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8B, 0x18, 0x8D, 0x4E, 0x05,
    0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC,
    0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB, 0x04, 0xF6, 0xD1, 0x20, 0x0F,
    0x8B, 0x18, 0x8D, 0x4E, 0x06, 0x8B, 0xF9, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE1, 0x07, 0x8A,
    0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x74, 0x04, 0x08, 0x0F, 0xEB,
    0x04, 0xF6, 0xD1, 0x20, 0x0F, 0x8D, 0x4E, 0x07, 0x8B, 0x30, 0x5F, 0xF6, 0xC2, 0x02, 0x5B, 0x8B,
    0xD1, 0x74, 0x18, 0xC1, 0xEA, 0x03, 0x83, 0xE1, 0x07, 0x8A, 0x0C, 0x8D, 0x58, 0x85, 0xDC, 0x00,
    0x08, 0x0C, 0x32, 0x03, 0xD6, 0x83, 0x40, 0x10, 0x08, 0x5E, 0xC3, 0x83, 0xE1, 0x07, 0x8A, 0x0C,
    0x8D, 0x58, 0x85, 0xDC, 0x00, 0xC1, 0xEA, 0x03, 0x03, 0xD6, 0xF6, 0xD1, 0x20, 0x0A, 0x83, 0x40,
    0x10, 0x08, 0x5E, 0xC3,
];
const B_WRITE_U8_DUP_CALLS: [usize; 0] = [];
const B_WRITE_U16: [u8; 201] = [
    0x8B, 0x41, 0x10, 0x56, 0x8D, 0x70, 0x10, 0x3B, 0x71, 0x0C, 0x0F, 0xB7, 0xD2, 0x0F, 0x87, 0xB0,
    0x00, 0x00, 0x00, 0x53, 0x55, 0x57, 0x83, 0xC0, 0x01, 0xBD, 0x04, 0x00, 0x00, 0x00, 0x8B, 0xFF,
    0x8B, 0x39, 0x8D, 0x70, 0xFF, 0xC1, 0xEE, 0x03, 0x03, 0xF7, 0x8D, 0x78, 0xFF, 0x83, 0xE7, 0x07,
    0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xBD, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1E, 0xEB, 0x04,
    0xF6, 0xD3, 0x20, 0x1E, 0x8B, 0x39, 0x8B, 0xF0, 0xC1, 0xEE, 0x03, 0x03, 0xF7, 0x8B, 0xF8, 0xD1,
    0xEA, 0x83, 0xE7, 0x07, 0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xBD, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04,
    0x08, 0x1E, 0xEB, 0x04, 0xF6, 0xD3, 0x20, 0x1E, 0x8B, 0x19, 0x8D, 0x70, 0x01, 0x8B, 0xFE, 0xC1,
    0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE6, 0x07, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xB5, 0x58,
    0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1F, 0xEB, 0x04, 0xF6, 0xD3, 0x20, 0x1F, 0x8B, 0x19, 0x8D,
    0x70, 0x02, 0x8B, 0xFE, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE6, 0x07, 0x03, 0xFB, 0xF6, 0xC2,
    0x01, 0x8A, 0x1C, 0xB5, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1F, 0xEB, 0x04, 0xF6, 0xD3,
    0x20, 0x1F, 0xD1, 0xEA, 0x83, 0xC0, 0x04, 0x83, 0xED, 0x01, 0x0F, 0x85, 0x60, 0xFF, 0xFF, 0xFF,
    0x5F, 0x5D, 0x5B, 0x83, 0x41, 0x10, 0x10, 0x5E, 0xC3,
];
const B_WRITE_U16_CALLS: [usize; 0] = [];
const B_WRITE_S16: [u8; 201] = [
    0x8B, 0x41, 0x10, 0x56, 0x8D, 0x70, 0x10, 0x3B, 0x71, 0x0C, 0x0F, 0xBF, 0xD2, 0x0F, 0x87, 0xB0,
    0x00, 0x00, 0x00, 0x53, 0x55, 0x57, 0x83, 0xC0, 0x01, 0xBD, 0x04, 0x00, 0x00, 0x00, 0x8B, 0xFF,
    0x8B, 0x39, 0x8D, 0x70, 0xFF, 0xC1, 0xEE, 0x03, 0x03, 0xF7, 0x8D, 0x78, 0xFF, 0x83, 0xE7, 0x07,
    0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xBD, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1E, 0xEB, 0x04,
    0xF6, 0xD3, 0x20, 0x1E, 0x8B, 0x39, 0x8B, 0xF0, 0xC1, 0xEE, 0x03, 0x03, 0xF7, 0x8B, 0xF8, 0xD1,
    0xEA, 0x83, 0xE7, 0x07, 0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xBD, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04,
    0x08, 0x1E, 0xEB, 0x04, 0xF6, 0xD3, 0x20, 0x1E, 0x8B, 0x19, 0x8D, 0x70, 0x01, 0x8B, 0xFE, 0xC1,
    0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE6, 0x07, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xB5, 0x58,
    0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1F, 0xEB, 0x04, 0xF6, 0xD3, 0x20, 0x1F, 0x8B, 0x19, 0x8D,
    0x70, 0x02, 0x8B, 0xFE, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE6, 0x07, 0x03, 0xFB, 0xF6, 0xC2,
    0x01, 0x8A, 0x1C, 0xB5, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1F, 0xEB, 0x04, 0xF6, 0xD3,
    0x20, 0x1F, 0xD1, 0xEA, 0x83, 0xC0, 0x04, 0x83, 0xED, 0x01, 0x0F, 0x85, 0x60, 0xFF, 0xFF, 0xFF,
    0x5F, 0x5D, 0x5B, 0x83, 0x41, 0x10, 0x10, 0x5E, 0xC3,
];
const B_WRITE_S16_CALLS: [usize; 0] = [];
const B_WRITE_U32: [u8; 201] = [
    0x8B, 0x41, 0x10, 0x56, 0x8D, 0x70, 0x20, 0x3B, 0x71, 0x0C, 0x0F, 0x87, 0xB3, 0x00, 0x00, 0x00,
    0x53, 0x55, 0x57, 0x83, 0xC0, 0x01, 0xBD, 0x08, 0x00, 0x00, 0x00, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x8B, 0x39, 0x8D, 0x70, 0xFF, 0xC1, 0xEE, 0x03, 0x03, 0xF7, 0x8D, 0x78, 0xFF, 0x83, 0xE7, 0x07,
    0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xBD, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1E, 0xEB, 0x04,
    0xF6, 0xD3, 0x20, 0x1E, 0x8B, 0x39, 0x8B, 0xF0, 0xC1, 0xEE, 0x03, 0x03, 0xF7, 0x8B, 0xF8, 0xD1,
    0xEA, 0x83, 0xE7, 0x07, 0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xBD, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04,
    0x08, 0x1E, 0xEB, 0x04, 0xF6, 0xD3, 0x20, 0x1E, 0x8B, 0x19, 0x8D, 0x70, 0x01, 0x8B, 0xFE, 0xC1,
    0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE6, 0x07, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xB5, 0x58,
    0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1F, 0xEB, 0x04, 0xF6, 0xD3, 0x20, 0x1F, 0x8B, 0x19, 0x8D,
    0x70, 0x02, 0x8B, 0xFE, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE6, 0x07, 0x03, 0xFB, 0xF6, 0xC2,
    0x01, 0x8A, 0x1C, 0xB5, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1F, 0xEB, 0x04, 0xF6, 0xD3,
    0x20, 0x1F, 0xD1, 0xEA, 0x83, 0xC0, 0x04, 0x83, 0xED, 0x01, 0x0F, 0x85, 0x60, 0xFF, 0xFF, 0xFF,
    0x5F, 0x5D, 0x5B, 0x83, 0x41, 0x10, 0x20, 0x5E, 0xC3,
];
const B_WRITE_U32_CALLS: [usize; 0] = [];
const B_WRITE_U32_DUP: [u8; 201] = [
    0x8B, 0x41, 0x10, 0x56, 0x8D, 0x70, 0x20, 0x3B, 0x71, 0x0C, 0x0F, 0x87, 0xB3, 0x00, 0x00, 0x00,
    0x53, 0x55, 0x57, 0x83, 0xC0, 0x01, 0xBD, 0x08, 0x00, 0x00, 0x00, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x8B, 0x39, 0x8D, 0x70, 0xFF, 0xC1, 0xEE, 0x03, 0x03, 0xF7, 0x8D, 0x78, 0xFF, 0x83, 0xE7, 0x07,
    0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xBD, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1E, 0xEB, 0x04,
    0xF6, 0xD3, 0x20, 0x1E, 0x8B, 0x39, 0x8B, 0xF0, 0xC1, 0xEE, 0x03, 0x03, 0xF7, 0x8B, 0xF8, 0xD1,
    0xEA, 0x83, 0xE7, 0x07, 0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xBD, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04,
    0x08, 0x1E, 0xEB, 0x04, 0xF6, 0xD3, 0x20, 0x1E, 0x8B, 0x19, 0x8D, 0x70, 0x01, 0x8B, 0xFE, 0xC1,
    0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE6, 0x07, 0x03, 0xFB, 0xF6, 0xC2, 0x01, 0x8A, 0x1C, 0xB5, 0x58,
    0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1F, 0xEB, 0x04, 0xF6, 0xD3, 0x20, 0x1F, 0x8B, 0x19, 0x8D,
    0x70, 0x02, 0x8B, 0xFE, 0xC1, 0xEF, 0x03, 0xD1, 0xEA, 0x83, 0xE6, 0x07, 0x03, 0xFB, 0xF6, 0xC2,
    0x01, 0x8A, 0x1C, 0xB5, 0x58, 0x85, 0xDC, 0x00, 0x74, 0x04, 0x08, 0x1F, 0xEB, 0x04, 0xF6, 0xD3,
    0x20, 0x1F, 0xD1, 0xEA, 0x83, 0xC0, 0x04, 0x83, 0xED, 0x01, 0x0F, 0x85, 0x60, 0xFF, 0xFF, 0xFF,
    0x5F, 0x5D, 0x5B, 0x83, 0x41, 0x10, 0x20, 0x5E, 0xC3,
];
const B_WRITE_U32_DUP_CALLS: [usize; 0] = [];
const B_WRITE_U32_STACK: [u8; 12] = [
    0x8B, 0x54, 0x24, 0x04, 0xE8, 0xF7, 0xFC, 0xFF, 0xFF, 0xC2, 0x04, 0x00,
];
const B_WRITE_U32_STACK_CALLS: [usize; 1] = [0x4];
const B_WRITE_VEC3: [u8; 45] = [
    0x51, 0xD9, 0x06, 0xD9, 0x1C, 0x24, 0x8B, 0x14, 0x24, 0xE8, 0xE2, 0xFC, 0xFF, 0xFF, 0xD9, 0x46,
    0x04, 0xD9, 0x1C, 0x24, 0x8B, 0x14, 0x24, 0xE8, 0xD4, 0xFC, 0xFF, 0xFF, 0xD9, 0x46, 0x08, 0xD9,
    0x1C, 0x24, 0x8B, 0x14, 0x24, 0x83, 0xC4, 0x04, 0xE9, 0xC3, 0xFC, 0xFF, 0xFF,
];
const B_WRITE_VEC3_CALLS: [usize; 2] = [0x9, 0x17];
const B_WRITE_VEC2: [u8; 31] = [
    0x51, 0xD9, 0x06, 0xD9, 0x1C, 0x24, 0x8B, 0x14, 0x24, 0xE8, 0xB2, 0xFC, 0xFF, 0xFF, 0xD9, 0x46,
    0x04, 0xD9, 0x1C, 0x24, 0x8B, 0x14, 0x24, 0x83, 0xC4, 0x04, 0xE9, 0xA1, 0xFC, 0xFF, 0xFF,
];
const B_WRITE_VEC2_CALLS: [usize; 1] = [0x9];
const B_WRITE_BYTES: [u8; 36] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x33, 0xF6, 0x85, 0xFF, 0x76, 0x13, 0x8D, 0x64, 0x24, 0x00,
    0x8A, 0x14, 0x1E, 0xE8, 0x28, 0xF8, 0xFF, 0xFF, 0x83, 0xC6, 0x01, 0x3B, 0xF7, 0x72, 0xF1, 0x5E,
    0x5B, 0xC2, 0x04, 0x00,
];
const B_WRITE_BYTES_CALLS: [usize; 1] = [0x13];
const B_READ_AT: [u8; 76] = [
    0x8D, 0x0C, 0x30, 0x3B, 0x4A, 0x0C, 0x76, 0x03, 0x33, 0xC0, 0xC3, 0x83, 0xFE, 0x20, 0x77, 0xF8,
    0x57, 0x33, 0xFF, 0x33, 0xC9, 0x85, 0xF6, 0x76, 0x2F, 0x53, 0x55, 0x8B, 0x2A, 0x8D, 0x49, 0x00,
    0x8D, 0x14, 0x01, 0x8B, 0xDA, 0xC1, 0xEB, 0x03, 0x8A, 0x1C, 0x2B, 0x83, 0xE2, 0x07, 0x22, 0x1C,
    0x95, 0x58, 0x85, 0xDC, 0x00, 0xF6, 0xDB, 0x1B, 0xDB, 0xF7, 0xDB, 0xD3, 0xE3, 0x83, 0xC1, 0x01,
    0x0B, 0xFB, 0x3B, 0xCE, 0x72, 0xDA, 0x5D, 0x5B, 0x8B, 0xC7, 0x5F, 0xC3,
];
const B_READ_AT_CALLS: [usize; 0] = [];
const B_READ_N: [u8; 24] = [
    0x53, 0x8B, 0x5F, 0x10, 0x56, 0x8B, 0xF0, 0x8B, 0xC3, 0x8B, 0xD7, 0xE8, 0xA0, 0xFF, 0xFF, 0xFF,
    0x03, 0xDE, 0x5E, 0x89, 0x5F, 0x10, 0x5B, 0xC3,
];
const B_READ_N_CALLS: [usize; 1] = [0xb];
const B_READ_BIT: [u8; 67] = [
    0x8B, 0x41, 0x10, 0x8D, 0x50, 0x01, 0x3B, 0x51, 0x0C, 0x76, 0x0E, 0x83, 0xC0, 0x01, 0x33, 0xD2,
    0x85, 0xD2, 0x89, 0x41, 0x10, 0x0F, 0x95, 0xC0, 0xC3, 0x56, 0x8B, 0x31, 0x8B, 0xD0, 0xC1, 0xEA,
    0x03, 0x8A, 0x14, 0x32, 0x8B, 0xF0, 0x83, 0xE6, 0x07, 0x22, 0x14, 0xB5, 0x58, 0x85, 0xDC, 0x00,
    0x5E, 0xF6, 0xDA, 0x1B, 0xD2, 0x83, 0xC0, 0x01, 0xF7, 0xDA, 0x85, 0xD2, 0x89, 0x41, 0x10, 0x0F,
    0x95, 0xC0, 0xC3,
];
const B_READ_BIT_CALLS: [usize; 0] = [];
const B_READ_U8: [u8; 27] = [
    0x53, 0x8B, 0x5F, 0x10, 0x56, 0xBE, 0x08, 0x00, 0x00, 0x00, 0x8B, 0xC3, 0x8B, 0xD7, 0xE8, 0x2D,
    0xFF, 0xFF, 0xFF, 0x03, 0xDE, 0x5E, 0x89, 0x5F, 0x10, 0x5B, 0xC3,
];
const B_READ_U8_CALLS: [usize; 1] = [0xe];
const B_READ_S8: [u8; 27] = [
    0x53, 0x8B, 0x5F, 0x10, 0x56, 0xBE, 0x08, 0x00, 0x00, 0x00, 0x8B, 0xC3, 0x8B, 0xD7, 0xE8, 0x0D,
    0xFF, 0xFF, 0xFF, 0x03, 0xDE, 0x5E, 0x89, 0x5F, 0x10, 0x5B, 0xC3,
];
const B_READ_S8_CALLS: [usize; 1] = [0xe];
const B_READ_U16: [u8; 27] = [
    0x53, 0x8B, 0x5F, 0x10, 0x56, 0xBE, 0x10, 0x00, 0x00, 0x00, 0x8B, 0xC3, 0x8B, 0xD7, 0xE8, 0xED,
    0xFE, 0xFF, 0xFF, 0x03, 0xDE, 0x5E, 0x89, 0x5F, 0x10, 0x5B, 0xC3,
];
const B_READ_U16_CALLS: [usize; 1] = [0xe];
const B_READ_S16: [u8; 27] = [
    0x53, 0x8B, 0x5F, 0x10, 0x56, 0xBE, 0x10, 0x00, 0x00, 0x00, 0x8B, 0xC3, 0x8B, 0xD7, 0xE8, 0xCD,
    0xFE, 0xFF, 0xFF, 0x03, 0xDE, 0x5E, 0x89, 0x5F, 0x10, 0x5B, 0xC3,
];
const B_READ_S16_CALLS: [usize; 1] = [0xe];
const B_READ_U32: [u8; 27] = [
    0x53, 0x8B, 0x5F, 0x10, 0x56, 0xBE, 0x20, 0x00, 0x00, 0x00, 0x8B, 0xC3, 0x8B, 0xD7, 0xE8, 0xAD,
    0xFE, 0xFF, 0xFF, 0x03, 0xDE, 0x5E, 0x89, 0x5F, 0x10, 0x5B, 0xC3,
];
const B_READ_U32_CALLS: [usize; 1] = [0xe];
const B_READ_S32: [u8; 27] = [
    0x53, 0x8B, 0x5F, 0x10, 0x56, 0xBE, 0x20, 0x00, 0x00, 0x00, 0x8B, 0xC3, 0x8B, 0xD7, 0xE8, 0x8D,
    0xFE, 0xFF, 0xFF, 0x03, 0xDE, 0x5E, 0x89, 0x5F, 0x10, 0x5B, 0xC3,
];
const B_READ_S32_CALLS: [usize; 1] = [0xe];
const B_READ_U8_DUP: [u8; 27] = [
    0x53, 0x8B, 0x5F, 0x10, 0x56, 0xBE, 0x08, 0x00, 0x00, 0x00, 0x8B, 0xC3, 0x8B, 0xD7, 0xE8, 0x6D,
    0xFE, 0xFF, 0xFF, 0x03, 0xDE, 0x5E, 0x89, 0x5F, 0x10, 0x5B, 0xC3,
];
const B_READ_U8_DUP_CALLS: [usize; 1] = [0xe];
const B_READ_F32: [u8; 37] = [
    0x51, 0x53, 0x8B, 0x5F, 0x10, 0x56, 0xBE, 0x20, 0x00, 0x00, 0x00, 0x8B, 0xC3, 0x8B, 0xD7, 0xE8,
    0x4C, 0xFE, 0xFF, 0xFF, 0x03, 0xDE, 0x5E, 0x89, 0x5F, 0x10, 0x89, 0x44, 0x24, 0x04, 0xD9, 0x44,
    0x24, 0x04, 0x5B, 0x59, 0xC3,
];
const B_READ_F32_CALLS: [usize; 1] = [0xf];
const B_READ_VEC2: [u8; 66] = [
    0x53, 0x8B, 0x5F, 0x10, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x56, 0xBE, 0x20, 0x00, 0x00, 0x00, 0x8B,
    0xC3, 0x8B, 0xD7, 0xE8, 0xB8, 0xFD, 0xFF, 0xFF, 0x03, 0xDE, 0x89, 0x5F, 0x10, 0x89, 0x45, 0x00,
    0x8B, 0x5F, 0x10, 0x8B, 0xC3, 0x8B, 0xD7, 0xE8, 0xA4, 0xFD, 0xFF, 0xFF, 0x03, 0xDE, 0x89, 0x44,
    0x24, 0x10, 0xD9, 0x44, 0x24, 0x10, 0x89, 0x5F, 0x10, 0xD9, 0x5D, 0x04, 0x5E, 0x5D, 0x5B, 0xC2,
    0x04, 0x00,
];
const B_READ_VEC2_CALLS: [usize; 2] = [0x13, 0x27];
const B_READ_BYTES: [u8; 61] = [
    0x55, 0x8B, 0x6C, 0x24, 0x08, 0x57, 0x33, 0xFF, 0x39, 0x7C, 0x24, 0x14, 0x76, 0x2A, 0x53, 0x56,
    0x8B, 0x5D, 0x10, 0xBE, 0x08, 0x00, 0x00, 0x00, 0x8B, 0xC3, 0x8B, 0xD5, 0xE8, 0x5F, 0xFD, 0xFF,
    0xFF, 0x8B, 0x4C, 0x24, 0x18, 0x03, 0xDE, 0x89, 0x5D, 0x10, 0x88, 0x04, 0x0F, 0x83, 0xC7, 0x01,
    0x3B, 0x7C, 0x24, 0x1C, 0x72, 0xDA, 0x5E, 0x5B, 0x5F, 0x5D, 0xC2, 0x0C, 0x00,
];
const B_READ_BYTES_CALLS: [usize; 1] = [0x1c];
const B_HANDLE_TO_ID: [u8; 55] = [
    0x85, 0xC9, 0x75, 0x05, 0x66, 0x0D, 0xFF, 0xFF, 0xC3, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00,
    0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x05, 0x39, 0x48, 0x44,
    0x74, 0x06, 0xB8, 0xFF, 0xFF, 0x00, 0x00, 0xC3, 0x8B, 0x80, 0xB0, 0x00, 0x00, 0x00, 0x85, 0xC0,
    0x74, 0xF0, 0x0F, 0xB7, 0x40, 0x02, 0xC3,
];
const B_HANDLE_TO_ID_CALLS: [usize; 0] = [];
const B_ID_TO_HANDLE: [u8; 42] = [
    0x66, 0x3D, 0xFF, 0xFF, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x56, 0x8B, 0x35, 0x58, 0xE2, 0x12, 0x02,
    0x0F, 0xB7, 0xC0, 0x6A, 0x00, 0x50, 0xE8, 0x65, 0x3E, 0x00, 0x00, 0x85, 0xC0, 0x5E, 0x74, 0xE6,
    0x8B, 0x00, 0x85, 0xC0, 0x74, 0xE0, 0x8B, 0x40, 0x04, 0xC3,
];
const B_ID_TO_HANDLE_CALLS: [usize; 1] = [0x16];
const B_MAP_FIND: [u8; 157] = [
    0x53, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x8B, 0xC5, 0xC1, 0xE0, 0x0F, 0xF7, 0xD0, 0x03, 0xC5, 0x8B,
    0xC8, 0xC1, 0xF9, 0x0A, 0x33, 0xC8, 0x8D, 0x0C, 0xC9, 0x8B, 0xC1, 0xC1, 0xF8, 0x06, 0x33, 0xC8,
    0x8B, 0xD1, 0xC1, 0xE2, 0x0B, 0xF7, 0xD2, 0x03, 0xCA, 0x8B, 0xC1, 0xC1, 0xF8, 0x10, 0x33, 0xC1,
    0x99, 0xF7, 0x7E, 0x60, 0x8B, 0x46, 0x5C, 0x57, 0x8B, 0xDA, 0x8B, 0x04, 0x98, 0x85, 0xC0, 0x74,
    0x0C, 0x3B, 0x68, 0x0C, 0x74, 0x51, 0x8B, 0x40, 0x04, 0x85, 0xC0, 0x75, 0xF4, 0x80, 0x7C, 0x24,
    0x14, 0x00, 0x74, 0x41, 0x83, 0x7E, 0x1C, 0x00, 0x7E, 0x3B, 0x8D, 0x7E, 0x20, 0xE8, 0x8E, 0x03,
    0x00, 0x00, 0x89, 0x68, 0x0C, 0x89, 0x58, 0x10, 0x8B, 0x4E, 0x5C, 0x8B, 0x14, 0x99, 0x89, 0x50,
    0x04, 0xC7, 0x40, 0x08, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x4E, 0x5C, 0x83, 0x3C, 0x99, 0x00, 0x74,
    0x08, 0x8B, 0xD1, 0x8B, 0x0C, 0x9A, 0x89, 0x41, 0x08, 0x8B, 0x56, 0x5C, 0x89, 0x04, 0x9A, 0x5F,
    0x5D, 0x5B, 0xC2, 0x08, 0x00, 0x33, 0xC0, 0x5F, 0x5D, 0x5B, 0xC2, 0x08, 0x00,
];
const B_MAP_FIND_CALLS: [usize; 1] = [0x5d];
const B_HANDLE_OWNED: [u8; 84] = [
    0x85, 0xC9, 0x74, 0x4D, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80,
    0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x39, 0x39, 0x48, 0x44, 0x75, 0x34, 0x8B, 0x80, 0xB0,
    0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x2A, 0x8B, 0x0D, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC9, 0x74,
    0x20, 0x8B, 0x89, 0x08, 0x04, 0x00, 0x00, 0x85, 0xC9, 0x74, 0x16, 0x8A, 0x10, 0x3A, 0x91, 0xB0,
    0x01, 0x00, 0x00, 0x75, 0x09, 0xF6, 0x80, 0x7C, 0x06, 0x00, 0x00, 0x01, 0x74, 0x03, 0x32, 0xC0,
    0xC3, 0xB0, 0x01, 0xC3,
];
const B_HANDLE_OWNED_CALLS: [usize; 0] = [];
const B_RECORD_OWNED: [u8; 52] = [
    0x85, 0xC9, 0x74, 0x2D, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x24, 0x8B, 0x80, 0x08,
    0x04, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x1A, 0x8A, 0x11, 0x3A, 0x90, 0xB0, 0x01, 0x00, 0x00, 0x75,
    0x0D, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x84, 0x81, 0x7C, 0x06, 0x00, 0x00, 0x74, 0x05, 0x33, 0xC0,
    0xC3, 0xB0, 0x01, 0xC3,
];
const B_RECORD_OWNED_CALLS: [usize; 0] = [];
const B_FIND_CONN: [u8; 41] = [
    0x8B, 0x88, 0x00, 0x04, 0x00, 0x00, 0x8B, 0xC1, 0x85, 0xC0, 0x74, 0x1A, 0x8D, 0x64, 0x24, 0x00,
    0x38, 0x90, 0xB0, 0x01, 0x00, 0x00, 0x74, 0x10, 0x8B, 0x80, 0xCC, 0x22, 0x00, 0x00, 0x3B, 0xC1,
    0x74, 0x04, 0x85, 0xC0, 0x75, 0xEA, 0x33, 0xC0, 0xC3,
];
const B_FIND_CONN_CALLS: [usize; 0] = [];
const B_GUID_EQUAL: [u8; 157] = [
    0x85, 0xD2, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x56, 0xB8, 0x10, 0x00, 0x00, 0x00, 0x57, 0x8B, 0xFF,
    0x8B, 0x32, 0x3B, 0x31, 0x75, 0x12, 0x83, 0xE8, 0x04, 0x83, 0xC1, 0x04, 0x83, 0xC2, 0x04, 0x83,
    0xF8, 0x04, 0x73, 0xEC, 0x85, 0xC0, 0x74, 0x67, 0x0F, 0xB6, 0x32, 0x0F, 0xB6, 0x39, 0x2B, 0xF7,
    0x75, 0x45, 0x83, 0xE8, 0x01, 0x83, 0xC1, 0x01, 0x83, 0xC2, 0x01, 0x85, 0xC0, 0x74, 0x50, 0x0F,
    0xB6, 0x32, 0x0F, 0xB6, 0x39, 0x2B, 0xF7, 0x75, 0x2E, 0x83, 0xE8, 0x01, 0x83, 0xC1, 0x01, 0x83,
    0xC2, 0x01, 0x85, 0xC0, 0x74, 0x39, 0x0F, 0xB6, 0x32, 0x0F, 0xB6, 0x39, 0x2B, 0xF7, 0x75, 0x17,
    0x83, 0xE8, 0x01, 0x83, 0xC1, 0x01, 0x83, 0xC2, 0x01, 0x85, 0xC0, 0x74, 0x22, 0x0F, 0xB6, 0x32,
    0x0F, 0xB6, 0x01, 0x2B, 0xF0, 0x74, 0x18, 0x85, 0xF6, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x7F, 0x11,
    0x83, 0xC8, 0xFF, 0x33, 0xC9, 0x85, 0xC0, 0x0F, 0x94, 0xC1, 0x5F, 0x8A, 0xC1, 0x5E, 0xC3, 0x33,
    0xC0, 0x33, 0xC9, 0x85, 0xC0, 0x0F, 0x94, 0xC1, 0x5F, 0x8A, 0xC1, 0x5E, 0xC3,
];
const B_GUID_EQUAL_CALLS: [usize; 0] = [];
const B_STATE_ALLOWS: [u8; 39] = [
    0x0F, 0xB7, 0x40, 0x10, 0x66, 0x3D, 0xFF, 0xFF, 0x75, 0x03, 0xB0, 0x01, 0xC3, 0x85, 0xC9, 0x75,
    0x03, 0x32, 0xC0, 0xC3, 0x8B, 0x49, 0x3C, 0xBA, 0x01, 0x00, 0x00, 0x00, 0xD3, 0xE2, 0x0F, 0xB7,
    0xC0, 0x85, 0xD0, 0x0F, 0x95, 0xC0, 0xC3,
];
const B_STATE_ALLOWS_CALLS: [usize; 0] = [];
const B_ROUTE_SEND: [u8; 54] = [
    0x8B, 0x40, 0x0C, 0x85, 0xC0, 0x75, 0x15, 0x85, 0xC9, 0x74, 0x0E, 0x8B, 0x91, 0xAC, 0x01, 0x00,
    0x00, 0x3B, 0x8A, 0x04, 0x04, 0x00, 0x00, 0x74, 0x1A, 0x32, 0xC0, 0xC3, 0x83, 0xF8, 0x01, 0x75,
    0x12, 0x85, 0xC9, 0x74, 0x0E, 0x8B, 0x81, 0xAC, 0x01, 0x00, 0x00, 0x3B, 0x88, 0x04, 0x04, 0x00,
    0x00, 0x74, 0xE6, 0xB0, 0x01, 0xC3,
];
const B_ROUTE_SEND_CALLS: [usize; 0] = [];
const B_ROUTE_RECV: [u8; 54] = [
    0x8B, 0x40, 0x0C, 0x85, 0xC0, 0x75, 0x15, 0x85, 0xC9, 0x74, 0x28, 0x8B, 0x91, 0xAC, 0x01, 0x00,
    0x00, 0x3B, 0x8A, 0x04, 0x04, 0x00, 0x00, 0x75, 0x1A, 0x32, 0xC0, 0xC3, 0x83, 0xF8, 0x01, 0x75,
    0x12, 0x85, 0xC9, 0x74, 0xF4, 0x8B, 0x81, 0xAC, 0x01, 0x00, 0x00, 0x3B, 0x88, 0x04, 0x04, 0x00,
    0x00, 0x75, 0xE6, 0xB0, 0x01, 0xC3,
];
const B_ROUTE_RECV_CALLS: [usize; 0] = [];
const B_LOSS_RATIO: [u8; 134] = [
    0x51, 0x85, 0xC0, 0x75, 0x04, 0xD9, 0xEE, 0x59, 0xC3, 0x8B, 0x88, 0xC0, 0x15, 0x00, 0x00, 0x57,
    0x33, 0xFF, 0x33, 0xD2, 0x85, 0xC9, 0x76, 0x3A, 0x53, 0x56, 0x8B, 0x35, 0xCC, 0xA3, 0x52, 0x02,
    0x05, 0xC4, 0x01, 0x00, 0x00, 0x80, 0x78, 0x05, 0x00, 0x75, 0x18, 0x8B, 0x18, 0x81, 0xC3, 0xE8,
    0x03, 0x00, 0x00, 0x3B, 0xDE, 0x73, 0x0C, 0x83, 0xC2, 0x01, 0x80, 0x78, 0x04, 0x00, 0x75, 0x03,
    0x83, 0xC7, 0x01, 0x83, 0xC0, 0x10, 0x83, 0xE9, 0x01, 0x75, 0xDA, 0x83, 0xFA, 0x14, 0x5E, 0x5B,
    0x73, 0x05, 0xD9, 0xEE, 0x5F, 0x59, 0xC3, 0x85, 0xFF, 0x89, 0x7C, 0x24, 0x04, 0xDB, 0x44, 0x24,
    0x04, 0x7D, 0x06, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0x85, 0xD2, 0x89, 0x54, 0x24, 0x04, 0xDB,
    0x44, 0x24, 0x04, 0x7D, 0x06, 0xD8, 0x05, 0xD8, 0xBF, 0xE5, 0x00, 0xDE, 0xF9, 0x5F, 0xD9, 0x1C,
    0x24, 0xD9, 0x04, 0x24, 0x59, 0xC3,
];
const B_LOSS_RATIO_CALLS: [usize; 0] = [];
const B_CLOCK_MS: [u8; 19] = [
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x04, 0x8B, 0x40, 0x08, 0xC3, 0xA1, 0x60, 0xB1,
    0x12, 0x02, 0xC3,
];
const B_CLOCK_MS_CALLS: [usize; 0] = [];
const B_CLOCK_RESET: [u8; 13] = [
    0x33, 0xC0, 0xA3, 0x60, 0xB1, 0x12, 0x02, 0xA3, 0x5C, 0xB1, 0x12, 0x02, 0xC3,
];
const B_CLOCK_RESET_CALLS: [usize; 0] = [];
const B_TIMEOUT_CLEAR: [u8; 18] = [
    0xC6, 0x05, 0x89, 0x8B, 0x52, 0x02, 0x00, 0xC7, 0x05, 0x14, 0x16, 0xEB, 0x00, 0xFF, 0xFF, 0xFF,
    0xFF, 0xC3,
];
const B_TIMEOUT_CLEAR_CALLS: [usize; 0] = [];
const B_STATE_1_6_8: [u8; 24] = [
    0x8B, 0x40, 0x1C, 0x83, 0xE8, 0x01, 0x74, 0x0D, 0x83, 0xE8, 0x05, 0x74, 0x08, 0x83, 0xE8, 0x02,
    0x74, 0x03, 0x32, 0xC0, 0xC3, 0xB0, 0x01, 0xC3,
];
const B_STATE_1_6_8_CALLS: [usize; 0] = [];
const B_STATE_0_1: [u8; 21] = [
    0x8B, 0x40, 0x1C, 0x85, 0xC0, 0x74, 0x08, 0x83, 0xF8, 0x01, 0x74, 0x03, 0x33, 0xC0, 0xC3, 0xB8,
    0x01, 0x00, 0x00, 0x00, 0xC3,
];
const B_STATE_0_1_CALLS: [usize; 0] = [];
const B_RECENT_IDS: [u8; 43] = [
    0x8A, 0x8A, 0xBA, 0x1D, 0x00, 0x00, 0x32, 0xC0, 0x84, 0xC9, 0x57, 0x76, 0x16, 0x8D, 0x49, 0x00,
    0x0F, 0xB6, 0xF8, 0x66, 0x39, 0xB4, 0x7A, 0xBA, 0x1C, 0x00, 0x00, 0x74, 0x0A, 0x04, 0x01, 0x3A,
    0xC1, 0x72, 0xED, 0x32, 0xC0, 0x5F, 0xC3, 0xB0, 0x01, 0x5F, 0xC3,
];
const B_RECENT_IDS_CALLS: [usize; 0] = [];
const B_OBJ_OWNER_CONN: [u8; 29] = [
    0x85, 0xC0, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x8B, 0x80, 0xB0, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74,
    0xF3, 0x8A, 0x10, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0xE9, 0x33, 0x8C, 0x00, 0x00,
];
const B_OBJ_OWNER_CONN_CALLS: [usize; 0] = [];
const B_HANDLE_OWNER_CONN: [u8; 54] = [
    0x85, 0xC9, 0x74, 0x19, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80,
    0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x05, 0x39, 0x48, 0x44, 0x74, 0x03, 0x33, 0xC0, 0xC3,
    0x8B, 0x80, 0xB0, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0xF3, 0x8A, 0x10, 0xA1, 0x14, 0x8C, 0x52,
    0x02, 0xE9, 0xFA, 0x8B, 0x00, 0x00,
];
const B_HANDLE_OWNER_CONN_CALLS: [usize; 0] = [];
const B_REG_FIND_FIELD: [u8; 45] = [
    0x8B, 0x4E, 0x18, 0x33, 0xC0, 0x85, 0xC9, 0x76, 0x15, 0x8D, 0x56, 0x60, 0x8D, 0x64, 0x24, 0x00,
    0x39, 0x3A, 0x74, 0x0D, 0x83, 0xC0, 0x01, 0x83, 0xC2, 0x44, 0x3B, 0xC1, 0x72, 0xF2, 0x33, 0xC0,
    0xC3, 0x8B, 0xC8, 0xC1, 0xE1, 0x04, 0x03, 0xC8, 0x8D, 0x44, 0x8E, 0x20, 0xC3,
];
const B_REG_FIND_FIELD_CALLS: [usize; 0] = [];
const B_READ_VEC3: [u8; 96] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x55, 0x8B, 0x6F, 0x10, 0x56, 0xBE, 0x20, 0x00, 0x00, 0x00, 0x8B,
    0xC5, 0x8B, 0xD7, 0xE8, 0x18, 0xFE, 0xFF, 0xFF, 0x83, 0xC5, 0x20, 0x89, 0x6F, 0x10, 0x89, 0x03,
    0x8B, 0x6F, 0x10, 0x8B, 0xC5, 0x8B, 0xD7, 0xE8, 0x04, 0xFE, 0xFF, 0xFF, 0x89, 0x44, 0x24, 0x10,
    0xD9, 0x44, 0x24, 0x10, 0x83, 0xC5, 0x20, 0x89, 0x6F, 0x10, 0xD9, 0x5B, 0x04, 0x8B, 0x6F, 0x10,
    0x8B, 0xC5, 0x8B, 0xD7, 0xE8, 0xE7, 0xFD, 0xFF, 0xFF, 0x83, 0xC5, 0x20, 0x5E, 0x89, 0x6F, 0x10,
    0x89, 0x44, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x0C, 0x5D, 0xD9, 0x5B, 0x08, 0x5B, 0xC2, 0x04, 0x00,
];
const B_READ_VEC3_CALLS: [usize; 3] = [0x13, 0x27, 0x44];

// --- differential tests: the Rust bodies against the original machine code, on random data ---------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_void;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

    struct Rng(u64);
    impl Rng {
        fn u32(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
        fn below(&mut self, n: u32) -> u32 {
            self.u32() % n
        }
    }

    fn fixed(addr: usize, len: usize) {
        let mut p = unsafe { VirtualAlloc(addr as *const c_void, len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
        if p.is_null() {
            // Inside an existing reservation: commit the pages only.
            p = unsafe { VirtualAlloc(addr as *const c_void, len, MEM_COMMIT, PAGE_EXECUTE_READWRITE) };
        }
        assert_eq!(p as usize, addr, "cannot map {addr:#x}: error {}", unsafe { windows_sys::Win32::Foundation::GetLastError() });
    }

    /// The bit-mask table 0xDC8558 lives in a reserved region of the test process: the copies read a
    /// table of our own instead.
    fn mask_table() -> u32 {
        static TABLE: [u32; 8] = [1, 2, 4, 8, 0x10, 0x20, 0x40, 0x80];
        TABLE.as_ptr() as u32
    }

    fn exec(body: &[u8]) -> usize {
        let mut body = body.to_vec();
        for i in 0..body.len().saturating_sub(3) {
            if body[i..i + 4] == [0x58, 0x85, 0xDC, 0x00] {
                body[i..i + 4].copy_from_slice(&mask_table().to_le_bytes());
            }
        }
        let body = &body[..];
        let p = unsafe { VirtualAlloc(std::ptr::null(), body.len() + 16, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
        unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), p as *mut u8, body.len()) };
        p
    }

    fn run(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: [u32; 3]) -> Regs {
        let mut r = Regs { eax, ecx, edx, ebx: 0xB1B1B1B1, esi, edi, ebp: 0xE9E9E9E9 };
        unsafe { call_orig(f, &mut r, args, false) };
        r
    }

    /// A zeroed block with random bytes at the given offsets' neighbourhood.
    fn block(len: usize, rng: &mut Rng) -> Vec<u8> {
        (0..len).map(|_| rng.u32() as u8).collect()
    }

    fn put32(b: &mut [u8], off: usize, v: u32) {
        b[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn addr(b: &[u8]) -> u32 {
        b.as_ptr() as u32
    }

    /// Maps the game's globals at their real addresses, so it needs a test exe that does not sit on
    /// them: RUSTFLAGS="-C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
    /// -p sr2-hook --target i686-pc-windows-msvc port_net -- --ignored
    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the doc comment)"]
    fn rust_matches_original() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for (a, l) in [(0x214_0000usize, 0x2_0000usize), (0x212_0000, 0x2_0000), (0x252_0000, 0x2_0000)] {
            fixed(a, l);
        }

        // ---- the bit stream -------------------------------------------------------------------
        let w_at = exec(&B_WRITE_AT);
        let w_bit = exec(&B_WRITE_BIT);
        let w_u8 = exec(&B_WRITE_U8);
        let w_s8 = exec(&B_WRITE_S8);
        let w_u16 = exec(&B_WRITE_U16);
        let w_s16 = exec(&B_WRITE_S16);
        let w_u32 = exec(&B_WRITE_U32);
        let w_u32d = exec(&B_WRITE_U32_DUP);
        let r_at = exec(&B_READ_AT);
        let r_bit = exec(&B_READ_BIT);
        let r_n = exec(&B_READ_N);
        let r_u8 = exec(&B_READ_U8);
        let r_u16 = exec(&B_READ_U16);
        let r_u32 = exec(&B_READ_U32);
        let r_f32 = exec(&B_READ_F32);
        let r_v3 = exec(&B_READ_VEC3);
        let r_v2 = exec(&B_READ_VEC2);
        let r_by = exec(&B_READ_BYTES);
        let w_v3 = exec(&B_WRITE_VEC3);
        let w_v2 = exec(&B_WRITE_VEC2);
        let w_by = exec(&B_WRITE_BYTES);
        let w_u32s = exec(&B_WRITE_U32_STACK);
        unsafe {
            for (c, o) in [(r_n, 0xB), (r_u8, 0xE), (r_u16, 0xE), (r_u32, 0xE), (r_f32, 0xF), (r_v3, 0x13), (r_v3, 0x27), (r_v3, 0x44), (r_v2, 0x13), (r_v2, 0x27), (r_by, 0x1C)] {
                set_rel32(c, o, r_at);
            }
            for (c, o) in [(w_v3, 0x9), (w_v3, 0x17), (w_v3, 0x28), (w_v2, 0x9), (w_v2, 0x1A), (w_u32s, 0x4)] {
                set_rel32(c, o, w_u32);
            }
            set_rel32(w_by, 0x13, w_u8);
        }

        for iter in 0..30000u32 {
            let size = rng.below(200);
            let nbytes = size.div_ceil(8) as usize + 8;
            let init = block(nbytes, &mut rng);
            let cursor = rng.below(size + 12);
            let value = rng.u32();
            let mk = |buf: &mut Vec<u8>| -> [u32; 5] { [buf.as_mut_ptr() as u32, 0, 0, size, cursor] };
            // writers on two copies
            #[allow(clippy::type_complexity)]
            let cases: [(usize, &str, u32, fn(u32, u32)); 7] = [
                (w_bit, "bit", 1, |s, v| unsafe { write_n(s, v, 1) }),
                (w_u8, "u8", 8, |s, v| unsafe { write_n(s, v & 0xFF, 8) }),
                (w_s8, "s8", 8, |s, v| unsafe { write_n(s, v & 0xFF, 8) }),
                (w_u16, "u16", 16, |s, v| unsafe { write_n(s, v & 0xFFFF, 16) }),
                (w_s16, "s16", 16, |s, v| unsafe { write_n(s, v & 0xFFFF, 16) }),
                (w_u32, "u32", 32, |s, v| unsafe { write_n(s, v, 32) }),
                (w_u32d, "u32d", 32, |s, v| unsafe { write_n(s, v, 32) }),
            ];
            for (f, name, _n, ours) in cases {
                let (mut ba, mut bb) = (init.clone(), init.clone());
                let (mut sa, mut sb) = (mk(&mut ba), mk(&mut bb));
                ours(sa.as_mut_ptr() as u32, value);
                let (eax, ecx) = if name.starts_with('u') && name.contains('8') || name == "s8" { (sb.as_mut_ptr() as u32, 0) } else { (0, sb.as_mut_ptr() as u32) };
                run(f, eax, ecx, value, 0, 0, [0; 3]);
                assert_eq!(sa[4], sb[4], "{name} cursor iter {iter}");
                assert_eq!(ba, bb, "{name} buffer iter {iter} size {size} cursor {cursor} value {value:#x}");
            }
            // write_at
            {
                let start = rng.below(size + 8);
                let count = rng.below(40);
                let (mut ba, mut bb) = (init.clone(), init.clone());
                let (mut sa, mut sb) = (mk(&mut ba), mk(&mut bb));
                unsafe { write_at(sa.as_mut_ptr() as u32, start, count, value) };
                run(w_at, start, count, 0, sb.as_mut_ptr() as u32, 0, [value, 0, 0]);
                assert_eq!(ba, bb, "write_at iter {iter}");
                assert_eq!(sa[4], sb[4]);
            }
            // readers
            let mut buf = init.clone();
            let mut s = mk(&mut buf);
            let sp = s.as_mut_ptr() as u32;
            {
                let start = rng.below(size + 8);
                let count = rng.below(40);
                let o = run(r_at, start, 0, sp, count, 0, [0; 3]);
                assert_eq!(o.eax, unsafe { read_at(start, count, sp) }, "read_at iter {iter} start {start} count {count} size {size}");
            }
            for (f, n, name) in [(r_u8, 8u32, "u8"), (r_u16, 16, "u16"), (r_u32, 32, "u32")] {
                s[4] = cursor;
                let o = run(f, 0, 0, 0, 0, sp, [0; 3]);
                let oc = s[4];
                s[4] = cursor;
                let v = unsafe { read_n(sp, n) };
                assert_eq!((o.eax, oc), (v, s[4]), "read_{name} iter {iter}");
            }
            {
                let n = rng.below(40);
                s[4] = cursor;
                let o = run(r_n, n, 0, 0, 0, sp, [0; 3]);
                let oc = s[4];
                s[4] = cursor;
                let v = unsafe { read_n(sp, n) };
                assert_eq!((o.eax, oc), (v, s[4]), "read_n iter {iter}");
            }
            {
                s[4] = cursor;
                let o = run(r_bit, 0, sp, 0, 0, 0, [0; 3]);
                let oc = s[4];
                s[4] = cursor;
                let (c, bit) = (cursor, cursor.wrapping_add(1) <= size && unsafe { get_bit(s[0], cursor) });
                s[4] = c.wrapping_add(1);
                assert_eq!((o.eax, oc), (lo(c.wrapping_add(1), bit as u32), s[4]), "read_bit iter {iter}");
            }
            {
                // f32, vec3, vec2, bytes
                s[4] = cursor;
                let mut r = Regs { eax: 0, ecx: 0, edx: 0, ebx: 0, esi: 0, edi: sp, ebp: 0 };
                let st0 = unsafe { call_orig(r_f32, &mut r, [0; 3], true) } as f32;
                let oc = s[4];
                s[4] = cursor;
                let bits = unsafe { read_n(sp, 32) };
                assert_eq!((r.eax, oc, st0.to_bits()), (bits, s[4], q32(bits)), "read_f32 iter {iter}");
                for (f, cnt) in [(r_v3, 3usize), (r_v2, 2)] {
                    s[4] = cursor;
                    let mut oa = [0u32; 3];
                    let o = run(f, 0, 0, 0, 0, sp, [oa.as_mut_ptr() as u32, 0, 0]);
                    let oc = s[4];
                    s[4] = cursor;
                    let mut ob = [0u32; 3];
                    let v = unsafe { read_floats(sp, ob.as_mut_ptr() as u32, cnt) };
                    assert_eq!((oa, o.eax, oc), (ob, v, s[4]), "read_vec{cnt} iter {iter}");
                }
                let cnt = rng.below(12);
                s[4] = cursor;
                let (mut oa, mut ob) = ([0u8; 16], [0u8; 16]);
                let o = run(r_by, 0x1234_5678, 0, 0, 0, 0, [sp, oa.as_mut_ptr() as u32, cnt]);
                let oc = s[4];
                s[4] = cursor;
                let mut v = 0x1234_5678;
                for i in 0..cnt {
                    v = unsafe { read_n(sp, 8) };
                    ob[i as usize] = v as u8;
                }
                assert_eq!((oa, o.eax, oc), (ob, v, s[4]), "read_bytes iter {iter}");
            }
            // vec / bytes / stack writers
            {
                let fl = [rng.u32(), rng.u32() | 0x7F80_0001, rng.u32()];
                for (f, cnt) in [(w_v3, 3usize), (w_v2, 2)] {
                    let (mut ba, mut bb) = (init.clone(), init.clone());
                    let (mut sa, mut sb) = (mk(&mut ba), mk(&mut bb));
                    unsafe { write_floats(sa.as_mut_ptr() as u32, fl.as_ptr() as u32, cnt) };
                    run(f, 0, sb.as_mut_ptr() as u32, 0, fl.as_ptr() as u32, 0, [0; 3]);
                    assert_eq!((ba, sa[4]), (bb, sb[4]), "write_vec{cnt} iter {iter}");
                }
                let src = block(16, &mut rng);
                let cnt = rng.below(12);
                let (mut ba, mut bb) = (init.clone(), init.clone());
                let (mut sa, mut sb) = (mk(&mut ba), mk(&mut bb));
                for i in 0..cnt {
                    unsafe { write_n(sa.as_mut_ptr() as u32, src[i as usize] as u32, 8) };
                }
                run(w_by, sb.as_mut_ptr() as u32, 0, 0, 0, cnt, [addr(&src), 0, 0]);
                assert_eq!((ba, sa[4]), (bb, sb[4]), "write_bytes iter {iter}");
                let (mut ba, mut bb) = (init.clone(), init.clone());
                let (mut sa, mut sb) = (mk(&mut ba), mk(&mut bb));
                unsafe { write_n(sa.as_mut_ptr() as u32, value, 32) };
                run(w_u32s, 0, sb.as_mut_ptr() as u32, 0, 0, 0, [value, 0, 0]);
                assert_eq!((ba, sa[4]), (bb, sb[4]), "write_u32_stack iter {iter}");
            }
        }

        // ---- handles, records, sessions -------------------------------------------------------------
        let f_h2id = exec(&B_HANDLE_TO_ID);
        let f_howned = exec(&B_HANDLE_OWNED);
        let f_rowned = exec(&B_RECORD_OWNED);
        let f_find = exec(&B_FIND_CONN);
        let f_guid = exec(&B_GUID_EQUAL);
        let f_state = exec(&B_STATE_ALLOWS);
        let f_send = exec(&B_ROUTE_SEND);
        let f_recv = exec(&B_ROUTE_RECV);
        let f_loss = exec(&B_LOSS_RATIO);
        let f_168 = exec(&B_STATE_1_6_8);
        let f_01 = exec(&B_STATE_0_1);
        let f_recent = exec(&B_RECENT_IDS);
        let f_regf = exec(&B_REG_FIND_FIELD);
        let f_map = exec(&B_MAP_FIND);
        let f_i2h = exec(&B_ID_TO_HANDLE);
        let f_oown = exec(&B_OBJ_OWNER_CONN);
        let f_hown = exec(&B_HANDLE_OWNER_CONN);
        let f_clock = exec(&B_CLOCK_MS);
        unsafe {
            set_rel32(f_i2h, 0x16, f_map);
            set_rel32(f_oown, 0x18, f_find);
            set_rel32(f_hown, 0x31, f_find);
        }

        // connections in a ring
        let mut conns: Vec<Vec<u8>> = (0..5).map(|_| block(0x2400, &mut rng)).collect();
        let n = conns.len();
        for i in 0..n {
            let next = addr(&conns[(i + 1) % n]);
            let id = i as u8 * 3 + 1;
            conns[i][0x1B0] = id;
            put32(&mut conns[i], 0x22CC, next);
        }
        let mut session = block(0x2400, &mut rng);
        put32(&mut session, 0x400, addr(&conns[0]));
        put32(&mut session, 0x408, addr(&conns[2]));
        put32(&mut session, 0x1C, 6);
        put32(&mut session, 8, 123456);
        unsafe { wd(SESSION, addr(&session)) };
        for i in 0..n {
            put32(&mut conns[i], 0x1AC, addr(&session));
        }
        // 0x404: "the peer" is sometimes one of the connections
        // objects and records
        let mut recs: Vec<Vec<u8>> = (0..8).map(|_| block(0x700, &mut rng)).collect();
        let mut objs: Vec<Vec<u8>> = (0..8).map(|_| block(0x100, &mut rng)).collect();
        for i in 0..8 {
            let r = addr(&recs[i]);
            recs[i][0] = [1u8, 4, 7, 10, 13, 99][i % 6];
            let flag = rng.below(2) as u8;
            recs[i][0x67C] = (recs[i][0x67C] & !1) | flag;
            put32(&mut objs[i], 0xB0, if i == 6 { 0 } else { r });
        }
        for iter in 0..20000u32 {
            // handle table
            let h = if rng.below(8) == 0 { 0 } else { (rng.below(4096)) | (rng.below(0x100) << 12) };
            let oi = rng.below(8) as usize;
            let slotn = (h & 0xFFF) as usize;
            unsafe {
                wd(HANDLES + slotn * 16, if rng.below(6) == 0 { 0 } else { addr(&objs[oi]) });
            }
            put32(&mut objs[oi], 0x44, if rng.below(5) == 0 { h ^ 0x1000 } else { h });
            put32(&mut session, 0x408, if rng.below(7) == 0 { 0 } else { addr(&conns[rng.below(5) as usize]) });
            unsafe { wd(SESSION, if rng.below(9) == 0 { 0 } else { addr(&session) }) };
            let eax = rng.u32();
            let rec = addr(&recs[rng.below(8) as usize]);
            let recarg = if rng.below(6) == 0 { 0 } else { rec };
            assert_eq!(run(f_h2id, eax, h, 0, 0, 0, [0; 3]).eax, unsafe { handle_to_id(eax, h) }, "handle_to_id {iter}");
            assert_eq!(run(f_howned, eax, h, 0, 0, 0, [0; 3]).eax, unsafe { handle_owned(eax, h) }, "handle_owned {iter} h {h:#x}");
            assert_eq!(run(f_rowned, eax, recarg, 0, 0, 0, [0; 3]).eax, unsafe { record_owned(eax, recarg) }, "record_owned {iter}");
            if unsafe { rd(SESSION) } != 0 {
                let id = rng.below(16) as u8;
                assert_eq!(run(f_find, addr(&session), 0, id as u32 | (rng.u32() << 8), 0, 0, [0; 3]).eax, unsafe { find_conn(addr(&session), id as u32) }, "find_conn {iter}");
                let ob = if rng.below(5) == 0 { 0 } else { addr(&objs[oi]) };
                let want_o = if ob == 0 { 0 } else { unsafe { owner_conn_of_record(rd(ob as usize + 0xB0)) } };
                assert_eq!(run(f_oown, ob, 0, 0, 0, 0, [0; 3]).eax, want_o, "obj_owner_conn {iter}");
                let ho = run(f_hown, 0, h, 0, 0, 0, [0; 3]).eax;
                let want = if h == 0 { 0 } else { let (o, ok) = unsafe { slot(h) }; if ok { unsafe { owner_conn_of_record(rd(o as usize + 0xB0)) } } else { 0 } };
                assert_eq!(ho, want, "handle_owner_conn {iter}");
            }
            // guid
            let a = block(16, &mut rng);
            let mut b = a.clone();
            if rng.below(3) != 0 {
                let i = rng.below(16) as usize;
                b[i] = rng.u32() as u8;
            }
            let bz = if rng.below(10) == 0 { 0 } else { addr(&b) };
            assert_eq!(run(f_guid, eax, addr(&a), bz, 0, 0, [0; 3]).eax, unsafe { guid_equal(eax, addr(&a), bz) }, "guid {iter}");
            // packet gates
            let mut prec = block(0x20, &mut rng);
            put32(&mut prec, 0xC, [0u32, 1, 2, 5][rng.below(4) as usize]);
            let mask = [0xFFFFu32, 0x1FF00, 0xFF00, rng.u32()][rng.below(4) as usize];
            put32(&mut prec, 0x10, mask);
            let conn_i = rng.below(6) as usize;
            let conn = if conn_i == 5 { 0 } else { addr(&conns[conn_i]) };
            put32(&mut session, 0x404, if rng.below(2) == 0 { addr(&conns[rng.below(5) as usize]) } else { 0x1234 });
            if conn != 0 {
                put32(&mut conns[conn_i], 0x3C, rng.below(40));
            }
            let pa = addr(&prec);
            assert_eq!(run(f_state, pa, conn, 0, 0, 0, [0; 3]).eax, unsafe { state_allows(pa, conn) }, "state_allows {iter}");
            assert_eq!(run(f_send, pa, conn, 0, 0, 0, [0; 3]).eax, unsafe { route_allows(pa, conn, true) }, "route_send {iter}");
            assert_eq!(run(f_recv, pa, conn, 0, 0, 0, [0; 3]).eax, unsafe { route_allows(pa, conn, false) }, "route_recv {iter}");
            // session states
            put32(&mut session, 0x1C, rng.below(12));
            let sa = addr(&session);
            let st = unsafe { rd(sa as usize + 0x1C) };
            assert_eq!(run(f_168, sa, 0, 0, 0, 0, [0; 3]).eax, {
                if st == 1 || st == 6 || st == 8 { 1 } else { lo(st.wrapping_sub(8), 0) }
            }, "state_1_6_8 {iter}");
            assert_eq!(run(f_01, sa, 0, 0, 0, 0, [0; 3]).eax, (st == 0 || st == 1) as u32, "state_0_1 {iter}");
            unsafe { wd(SESSION, sa) };
            assert_eq!(run(f_clock, eax, 0, 0, 0, 0, [0; 3]).eax, 123456, "clock_ms {iter}");
            unsafe { wd(SESSION, 0) };
            unsafe { wd(CLOCK_OFFLINE, iter) };
            assert_eq!(run(f_clock, eax, 0, 0, 0, 0, [0; 3]).eax, iter, "clock_ms offline {iter}");
            // recent ids
            let mut cn = block(0x1E00, &mut rng);
            let cntb = rng.below(8) as u8 + if rng.below(4) == 0 { 0xF0 } else { 0 };
            cn[0x1DBA] = cntb.min(0x7F).max(if cntb == 0 { 0 } else { 1 });
            let want_id = rng.below(8) as u16;
            for k in 0..0x80usize {
                let v = rng.below(8) as u16;
                cn[0x1CBA + k * 2..0x1CBA + k * 2 + 2].copy_from_slice(&v.to_le_bytes());
            }
            let esi = (rng.u32() & 0xFFFF_0000) | want_id as u32;
            let o = run(f_recent, eax, 0, addr(&cn), esi, 0, [0; 3]);
            let cnt = cn[0x1DBA] as usize;
            let found = (0..cnt).any(|k| u16::from_le_bytes([cn[0x1CBA + k * 2], cn[0x1CBA + k * 2 + 1]]) == want_id);
            assert_eq!(o.eax, lo(eax, found as u32), "recent_ids {iter}");
            // registration fields
            let mut reg = block(0x60 + 0x44 * 6, &mut rng);
            let cnt = rng.below(7);
            put32(&mut reg, 0x18, cnt);
            let key = rng.below(5);
            for k in 0..6usize {
                put32(&mut reg, 0x60 + k * 0x44, rng.below(5));
            }
            let ra = addr(&reg);
            let want = (0..cnt as usize).find(|&k| u32::from_le_bytes(reg[0x60 + k * 0x44..0x64 + k * 0x44].try_into().unwrap()) == key).map_or(0, |k| ra + 0x20 + k as u32 * 0x44);
            assert_eq!(run(f_regf, 0, 0, 0, ra, key, [0; 3]).eax, want, "reg_find_field {iter}");
            // loss ratio
            let ne = rng.below(60);
            let mut cl = block(0x1600, &mut rng);
            put32(&mut cl, 0x15C0, ne);
            let now = rng.below(5000);
            unsafe { wd(NOW, now) };
            for k in 0..64usize {
                let t = rng.below(4000);
                put32(&mut cl, 0x1C4 + k * 16, t);
                cl[0x1C4 + k * 16 + 4] = (rng.below(3) == 0) as u8;
                cl[0x1C4 + k * 16 + 5] = (rng.below(5) == 0) as u8;
            }
            let ca = if rng.below(10) == 0 { 0 } else { addr(&cl) };
            let mut r = Regs { eax: ca, ecx: 0, edx: 0, ebx: 0, esi: 0, edi: 0, ebp: 0 };
            let st0 = unsafe { call_orig(f_loss, &mut r, [0; 3], true) } as f32;
            assert_eq!(st0.to_bits(), unsafe { loss_ratio(ca) }.to_bits(), "loss_ratio {iter}");
        }

        // ---- the net-id map ---------------------------------------------------------------------------------
        let keys: Vec<u32> = (0..12).map(|i| [0u32, 1, 2, 77, 0xFFFE, 0x1234, 0x7FFF_FFFF, 0x8000_0000, 0xDEAD_BEEF, 5, 6, 40000][i]).collect();
        for buckets in [1u32, 7, 64, 1021] {
            let mut slots = vec![0u32; buckets as usize];
            let mut nodes: Vec<Vec<u8>> = Vec::new();
            let mut recs2: Vec<Vec<u8>> = Vec::new();
            for b in 0..buckets as usize {
                for &k in &keys {
                    let mut node = block(0x20, &mut rng);
                    put32(&mut node, 0xC, k);
                    put32(&mut node, 4, slots[b]);
                    let mut rec = block(0x10, &mut rng);
                    put32(&mut rec, 4, k ^ 0x5555 | 1);
                    put32(&mut node, 0, if k == 6 { 0 } else { addr(&rec) });
                    slots[b] = addr(&node);
                    nodes.push(node);
                    recs2.push(rec);
                }
            }
            let mut map = block(0x80, &mut rng);
            put32(&mut map, 0x5C, slots.as_ptr() as u32);
            put32(&mut map, 0x60, buckets);
            put32(&mut map, 0x1C, 0);
            let ma = addr(&map);
            unsafe { wd(NET_ID_MAP, ma) };
            for &k in keys.iter().chain([3u32, 99, 0xFFFF_FFFF, 0x4000_0000].iter()) {
                let o = run(f_map, 0, 0, 0, ma, 0, [k, 0, 0]).eax;
                assert_eq!(o, unsafe { map_find(ma, k, 0) }, "map_find {buckets} {k:#x}");
                let id = k;
                let want = run(f_i2h, id, 0, 0, 0, 0, [0; 3]).eax;
                assert_eq!(want, unsafe { id_to_handle(id) }, "id_to_handle {buckets} {id:#x}");
            }
            drop(nodes);
            drop(slots);
        }
    }
}
