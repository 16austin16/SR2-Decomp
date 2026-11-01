//! Port module: five small nav / vehicle-AI leaves of doc 47 ("Port candidates"), checked
//! against the disassembly. Register-only readers, each behind a `pushad` thunk (all registers
//! come back as the caller left them except EAX, the result). For the first `SHADOW_CALLS` calls
//! of each port a relocated copy of the original runs on the same registers and its EAX is
//! compared with the Rust version's; on a mismatch the original's is kept.
//!
//! Also: nav_cell_scratch_record, vehicle_nav_query_mask, road_lane_by_index / _index / _filter_bit19_clear, nav_route_total_length (ST0 + EAX shadow).
//!
//! Not ported: `nav_cell_walkable` and `nav_cell_to_node_ref` (they call other functions) and
//! `nav_cell_has_link_flag2` (a loop whose leftover EAX upper bits are not worth reproducing).

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;
/// Chunk record table (0x866 entries).
const CHUNKS: usize = 0x00F7_AAE8;

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
const ECX: u8 = 2;
const EDX: u8 = 4;
/// EBX / ESI / EDI / EBP: callee-saved in every original here.
const SAVED: u8 = 8 | 16 | 32 | 64;

#[repr(C)]
struct Block {
    f: u32,
    r: Regs,
    arg: u32,
    esp: u32,
    st0: f64,
    want_st0: u32,
}

/// Call `f` with all seven registers from `r`, then capture all seven back into `r`.
unsafe fn call_full(f: usize, r: &mut Regs, arg: u32, want_st0: bool) -> f64 {
    let mut b = Block { f: f as u32, r: *r, arg, esp: 0, st0: 0.0, want_st0: want_st0 as u32 };
    unsafe {
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
    b.st0
}

macro_rules! regs_thunk {
    ($name:ident, $f:ident) => {
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
                "ret",
                f = sym $f,
            )
        }
    };
}

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

const NODE_REF_TO_CELL: usize = 0;
const CELL_CHUNK_RECORD: usize = 1;
const LINK_FLAGS_TO_MASK: usize = 2;
const ROUTE_CURRENT_ENTRY: usize = 3;
const ORDER_IN_0_1_3: usize = 4;
const SCRATCH_RECORD: usize = 5;
const QUERY_MASK: usize = 6;
const LANE_BY_INDEX: usize = 7;
const LANE_INDEX: usize = 8;
const LANE_FILTER: usize = 9;
const ROUTE_TOTAL_LENGTH: usize = 10;
const COUNT: usize = 11;

static PORTS: [Port; COUNT] = [
    port("nav_node_ref_to_cell", 0x00A0_51D0, SAVED),
    port("nav_cell_chunk_record", 0x00A0_5370, ECX | EDX | SAVED),
    port("nav_link_flags_to_mask", 0x00A0_4A10, EDX | SAVED),
    port("nav_route_current_entry", 0x00A1_F610, ECX | SAVED),
    port("vehicle_ai_order_in_0_1_3", 0x00B2_DC20, ECX | EDX | SAVED),
    port("nav_cell_scratch_record", 0x00A0_2A90, EDX | SAVED),
    port("vehicle_nav_query_mask", 0x00B2_6530, ECX | EDX | SAVED),
    port("road_lane_by_index", 0x00A8_ABE0, SAVED),
    port("road_lane_index", 0x00A8_ABA0, ECX | SAVED),
    port("road_lane_filter_bit19_clear", 0x00A8_AB80, ECX | EDX | SAVED),
    port("nav_route_total_length", 0x00A1_9460, ECX | EDX | SAVED),
];

fn shadowing(id: usize) -> Option<(u32, usize)> {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_nav: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    (orig != 0 && n < SHADOW_CALLS).then_some((n, orig))
}

fn mismatch(id: usize, n: u32, what: String) {
    let m = PORTS[id].mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!("port_nav: {} mismatch (call {n}): {what}", PORTS[id].name));
    }
}

static PRESERVE_LOGS: AtomicU32 = AtomicU32::new(0);

fn check_preserved(id: usize, inp: &Regs, out: &Regs) {
    for i in 0..7 {
        if PORTS[id].preserved & (1 << i) != 0 && inp.get(i) != out.get(i) && PRESERVE_LOGS.fetch_add(1, Ordering::Relaxed) < 10 {
            log(&format!(
                "port_nav: {}: the original changed {} ({:#x} -> {:#x})",
                PORTS[id].name,
                REG_NAMES[i],
                inp.get(i),
                out.get(i)
            ));
        }
    }
}

fn run_original(id: usize, orig: usize, p: &Pushad) -> Regs {
    let inp = Regs::of(p);
    let mut r = inp;
    unsafe { call_full(orig, &mut r, 0, false) };
    check_preserved(id, &inp, &r);
    r
}

/// The original copy's EAX wins on a mismatch.
fn checked_eax(id: usize, p: &Pushad, ours: u32) -> u32 {
    let Some((n, orig)) = shadowing(id) else { return ours };
    let r = run_original(id, orig, p);
    if r.eax != ours {
        mismatch(id, n, format!("in {:x?}: original {:#x}, Rust {ours:#x}", Regs::of(p), r.eax));
        return r.eax;
    }
    ours
}

// --- 0xA051D0 nav_node_ref_to_cell (EAX = ref -> EAX = cell* or 0) ---------------------------------

/// ref = chunk << 20 | kind << 19 | index. -1, chunk >= 0x866 (unsigned), a null record, record
/// flags (+0x1C) & 3 and a signed count (+0x6C kind 0 / +0x7C kind 1) <= 0 give 0; otherwise
/// `[rec + 0x240 / 0x248] -> [+0x14] + index * 0x30`. The index is not bounds-checked.
unsafe fn node_ref_to_cell(r: u32) -> u32 {
    unsafe {
        if r == 0xFFFF_FFFF {
            return 0;
        }
        let kind = (r >> 0x13) & 1;
        let chunk = r >> 0x14;
        let idx = r & 0x7FFFF;
        if chunk == 0xFFFF || chunk >= 0x866 {
            return 0;
        }
        let rec = rd(CHUNKS + chunk as usize * 4) as usize;
        if rec == 0 || rb(rec + 0x1C) & 3 != 0 {
            return 0;
        }
        let count = rd(rec + if kind != 0 { 0x7C } else { 0x6C }) as i32;
        if count <= 0 {
            return 0;
        }
        let obj = rd(rec + if kind != 0 { 0x248 } else { 0x240 }) as usize;
        rd(obj + 0x14).wrapping_add(idx.wrapping_mul(3) << 4)
    }
}

extern "C" fn node_ref_to_cell_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(NODE_REF_TO_CELL, &p, unsafe { node_ref_to_cell(p.eax) });
}
regs_thunk!(node_ref_to_cell_thunk, node_ref_to_cell_entry);

// --- 0xA05370 nav_cell_chunk_record (EAX = cell* -> EAX = chunk record or 0) -----------------------

/// `c = u16[cell + 0x2C]`; 0xFFFF or >= 0x866 (unsigned) gives 0, else the table entry.
unsafe fn cell_chunk_record(cell: u32) -> u32 {
    unsafe {
        let c = (rd(cell as usize + 0x2C) & 0xFFFF) as usize;
        if c == 0xFFFF || c >= 0x866 { 0 } else { rd(CHUNKS + c * 4) }
    }
}

extern "C" fn cell_chunk_record_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(CELL_CHUNK_RECORD, &p, unsafe { cell_chunk_record(p.eax) });
}
regs_thunk!(cell_chunk_record_thunk, cell_chunk_record_entry);

// --- 0xA04A10 nav_link_flags_to_mask (EDX = &flag bytes -> EAX = mask) -----------------------------

/// b0 & 8 -> 2, b0 & 0x10 -> |1, b1 & 1 -> |4.
unsafe fn link_flags_to_mask(p: u32) -> u32 {
    unsafe {
        let b0 = rb(p as usize);
        let b1 = rb(p as usize + 1);
        let mut m = 0;
        if b0 & 8 != 0 {
            m = 2;
        }
        if b0 & 0x10 != 0 {
            m |= 1;
        }
        if b1 & 1 != 0 {
            m |= 4;
        }
        m
    }
}

extern "C" fn link_flags_to_mask_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(LINK_FLAGS_TO_MASK, &p, unsafe { link_flags_to_mask(p.edx) });
}
regs_thunk!(link_flags_to_mask_thunk, link_flags_to_mask_entry);

// --- 0xA1F610 nav_route_current_entry (ECX = route -> EAX) -----------------------------------------

/// Count byte (+0x3C) 0 or index byte (+0x24) >= count (unsigned): 0, else `route + index * 12`.
unsafe fn route_current_entry(route: u32) -> u32 {
    unsafe {
        let n = rb(route as usize + 0x3C);
        if n == 0 {
            return 0;
        }
        let i = rb(route as usize + 0x24);
        if i >= n { 0 } else { route.wrapping_add(i as u32 * 12) }
    }
}

extern "C" fn route_current_entry_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(ROUTE_CURRENT_ENTRY, &p, unsafe { route_current_entry(p.ecx) });
}
regs_thunk!(route_current_entry_thunk, route_current_entry_entry);

// --- 0xB2DC20 vehicle_ai_order_in_0_1_3 (EAX = vehicle -> AL) --------------------------------------

/// `o = [vehicle + 0x8554]` (signed): AL = 1 for 0, 1 or 3 (negative gives 0). The rest of EAX is
/// the order value itself (the original loads it into EAX and only sets AL).
unsafe fn order_in_0_1_3(v: u32) -> u32 {
    unsafe {
        let o = rd(v as usize + 0x8554);
        let hit = (o as i32) >= 0 && ((o as i32) <= 1 || o == 3);
        (o & !0xFF) | hit as u32
    }
}

extern "C" fn order_in_0_1_3_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(ORDER_IN_0_1_3, &p, unsafe { order_in_0_1_3(p.eax) });
}
regs_thunk!(order_in_0_1_3_thunk, order_in_0_1_3_entry);

// --- 0xA02A90 nav_cell_scratch_record (EAX = cell* -> EAX = record or 0) ---------------------------

/// `[cell + 0x20]` must equal the generation word 0x2E160BC (else 0); the record is
/// `0x2E160C4 + u16[cell + 0x1E] * 24`.
unsafe fn cell_scratch_record(cell: u32) -> u32 {
    unsafe {
        let c = cell as usize;
        if rd(c + 0x20) != rd(0x02E1_60BC) {
            return 0;
        }
        let i = rb(c + 0x1E) as u32 | (rb(c + 0x1F) as u32) << 8;
        0x02E1_60C4u32.wrapping_add(i * 24)
    }
}

extern "C" fn cell_scratch_record_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(SCRATCH_RECORD, &p, unsafe { cell_scratch_record(p.eax) });
}
regs_thunk!(cell_scratch_record_thunk, cell_scratch_record_entry);

// --- 0xB26530 vehicle_nav_query_mask (EAX = vehicle or 0 -> EAX = mask) ----------------------------

/// Null vehicle: t = -1, else `t = [[v + 0x84E4] + 0x9C]` (no null check). t == 2 -> 0x200000,
/// t == 4 -> 0x40000, anything else 0x400.
unsafe fn query_mask(v: u32) -> u32 {
    unsafe {
        let t = if v == 0 { 0xFFFF_FFFF } else { rd(rd(v as usize + 0x84E4) as usize + 0x9C) };
        match t {
            2 => 0x20_0000,
            4 => 0x4_0000,
            _ => 0x400,
        }
    }
}

extern "C" fn query_mask_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(QUERY_MASK, &p, unsafe { query_mask(p.eax) });
}
regs_thunk!(query_mask_thunk, query_mask_entry);

// --- 0xA8ABE0 road_lane_by_index (EAX = index -> EAX = lane* or 0) ---------------------------------

/// Signed index < 0 or >= `[net]` gives 0 (net = [0x2225B64], not null-checked); else
/// `[net + 4] + index * 0x3C`.
unsafe fn lane_by_index(i: u32) -> u32 {
    unsafe {
        if (i as i32) < 0 {
            return 0;
        }
        let net = rd(0x0222_5B64) as usize;
        if (i as i32) >= rd(net) as i32 {
            return 0;
        }
        rd(net + 4).wrapping_add(i.wrapping_mul(0x3C))
    }
}

extern "C" fn lane_by_index_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(LANE_BY_INDEX, &p, unsafe { lane_by_index(p.eax) });
}
regs_thunk!(lane_by_index_thunk, lane_by_index_entry);

// --- 0xA8ABA0 road_lane_index (EAX = lane* -> EAX = index or -1) -----------------------------------

/// Null lane, `[lane + 0x10]` bit 19 set, or no network (0x2225B64 == 0): -1. Else the signed
/// division of `lane - [net + 4]` by 0x3C as `hi32(d * 0x88888889) + d`, sar 5, plus the sign bit.
unsafe fn lane_index(lane: u32) -> u32 {
    unsafe {
        if lane == 0 || (rd(lane as usize + 0x10) >> 0x13) & 1 != 0 {
            return 0xFFFF_FFFF;
        }
        let net = rd(0x0222_5B64) as usize;
        if net == 0 {
            return 0xFFFF_FFFF;
        }
        let d = lane.wrapping_sub(rd(net + 4)) as i32;
        let hi = ((0x8888_8889u32 as i32 as i64 * d as i64) >> 32) as i32;
        let e = hi.wrapping_add(d) >> 5;
        ((e as u32) >> 31).wrapping_add(e as u32)
    }
}

extern "C" fn lane_index_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(LANE_INDEX, &p, unsafe { lane_index(p.eax) });
}
regs_thunk!(lane_index_thunk, lane_index_entry);

// --- 0xA8AB80 road_lane_filter_bit19_clear (EAX = lane* -> EAX unchanged or 0) ---------------------

/// Null gives 0; bit 19 of `[rec + 0x10]` set gives 0; otherwise EAX is kept.
unsafe fn lane_filter(rec: u32) -> u32 {
    unsafe {
        if rec == 0 || (rd(rec as usize + 0x10) >> 0x13) & 1 != 0 { 0 } else { rec }
    }
}

extern "C" fn lane_filter_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(LANE_FILTER, &p, unsafe { lane_filter(p.eax) });
}
regs_thunk!(lane_filter_thunk, lane_filter_entry);

// --- 0xA19460 nav_route_total_length (ECX = head -> ST0, EAX = last link read) ---------------------

/// `sum = 0; p = head; loop { sum = f32(sum + f32 [p + 0x14]); next = [p + 0x1C]; stop when next ==
/// head or next == 0 }`. The sum goes through `fstp dword` every step (f64 add, then `as f32`);
/// the loads are `fld dword`, so a signalling NaN is quieted. Returns the sum and the final EAX.
unsafe fn route_total_length(head: u32) -> (f32, u32) {
    unsafe {
        let mut sum = 0.0f32;
        if head == 0 {
            return (sum, 0);
        }
        let mut p = head;
        loop {
            let bits = rd(p as usize + 0x14);
            let quiet = if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x7F_FFFF != 0 { 0x40_0000 } else { 0 };
            let x = f32::from_bits(bits | quiet);
            let next = rd(p as usize + 0x1C);
            sum = (x as f64 + sum as f64) as f32;
            if next == head || next == 0 {
                return (sum, next);
            }
            p = next;
        }
    }
}

extern "C" fn route_total_length_entry(r: &mut Pushad, _: *const u32) -> f32 {
    let p = *r;
    let (mut ours, eax) = unsafe { route_total_length(p.ecx) };
    r.eax = eax;
    let Some((n, orig)) = shadowing(ROUTE_TOTAL_LENGTH) else { return ours };
    let inp = Regs::of(&p);
    let mut out = inp;
    let theirs = unsafe { call_full(orig, &mut out, 0, true) } as f32;
    check_preserved(ROUTE_TOTAL_LENGTH, &inp, &out);
    if theirs.to_bits() != ours.to_bits() || out.eax != eax {
        mismatch(ROUTE_TOTAL_LENGTH, n, format!("in {inp:x?}: original {:#x} eax {:#x}, Rust {:#x} eax {eax:#x}", theirs.to_bits(), out.eax, ours.to_bits()));
        r.eax = out.eax;
        ours = theirs;
    }
    ours
}
regs_thunk!(route_total_length_thunk, route_total_length_entry);

// --- install ---------------------------------------------------------------------------------------

pub fn install() {
    let list: [(usize, &[u8], &[usize], extern "C" fn()); COUNT] = [
        (NODE_REF_TO_CELL, &NODE_REF_TO_CELL_BODY, &[], node_ref_to_cell_thunk),
        (CELL_CHUNK_RECORD, &CELL_CHUNK_RECORD_BODY, &[], cell_chunk_record_thunk),
        (LINK_FLAGS_TO_MASK, &LINK_FLAGS_TO_MASK_BODY, &[], link_flags_to_mask_thunk),
        (ROUTE_CURRENT_ENTRY, &ROUTE_CURRENT_ENTRY_BODY, &[], route_current_entry_thunk),
        (ORDER_IN_0_1_3, &ORDER_IN_0_1_3_BODY, &[], order_in_0_1_3_thunk),
        (SCRATCH_RECORD, &SCRATCH_RECORD_BODY, &[], cell_scratch_record_thunk),
        (QUERY_MASK, &QUERY_MASK_BODY, &[], query_mask_thunk),
        (LANE_BY_INDEX, &LANE_BY_INDEX_BODY, &[], lane_by_index_thunk),
        (LANE_INDEX, &LANE_INDEX_BODY, &[], lane_index_thunk),
        (LANE_FILTER, &LANE_FILTER_BODY, &[], lane_filter_thunk),
        (ROUTE_TOTAL_LENGTH, &ROUTE_TOTAL_LENGTH_BODY, &[], route_total_length_thunk),
    ];
    let mut ok = 0;
    for (id, body, call_offsets, thunk) in list {
        let port = &PORTS[id];
        match unsafe { detour::relocate(port.addr, body, call_offsets) } {
            Ok(copy) => port.original.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_nav: {}: no shadow copy: {e:?}", port.name)),
        }
        if replace_pinned(port.name, port.addr, body, thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_nav: replaced {ok}/{COUNT} with Rust"));
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
    if parts.is_empty() { String::new() } else { format!("port_nav: {}", parts.join(", ")) }
}

// --- original bodies (tools/body_bytes.py; nav_link_flags_to_mask cut at its ret) -------------------

const NODE_REF_TO_CELL_BODY: [u8; 118] = [
    0x83, 0xF8, 0xFF, 0x74, 0x6E, 0x8B, 0xD0, 0x8B, 0xC8, 0xC1, 0xEA, 0x13, 0xC1, 0xE9, 0x14, 0x83,
    0xE2, 0x01, 0x25, 0xFF, 0xFF, 0x07, 0x00, 0x81, 0xF9, 0xFF, 0xFF, 0x00, 0x00, 0x74, 0x54, 0x81,
    0xF9, 0x66, 0x08, 0x00, 0x00, 0x73, 0x4C, 0x8B, 0x0C, 0x8D, 0xE8, 0xAA, 0xF7, 0x00, 0x85, 0xC9,
    0x74, 0x41, 0xF6, 0x41, 0x1C, 0x03, 0x75, 0x3B, 0x85, 0xD2, 0x53, 0x74, 0x06, 0x83, 0x79, 0x7C,
    0x00, 0xEB, 0x04, 0x83, 0x79, 0x6C, 0x00, 0x0F, 0x9F, 0xC3, 0x84, 0xDB, 0x5B, 0x74, 0x24, 0x85,
    0xD2, 0x74, 0x10, 0x8B, 0x89, 0x48, 0x02, 0x00, 0x00, 0x8D, 0x04, 0x40, 0xC1, 0xE0, 0x04, 0x03,
    0x41, 0x14, 0xC3, 0x8B, 0x89, 0x40, 0x02, 0x00, 0x00, 0x8D, 0x04, 0x40, 0xC1, 0xE0, 0x04, 0x03,
    0x41, 0x14, 0xC3, 0x33, 0xC0, 0xC3,
];
const CELL_CHUNK_RECORD_BODY: [u8; 29] = [
    0x0F, 0xB7, 0x40, 0x2C, 0x3D, 0xFF, 0xFF, 0x00, 0x00, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x3D, 0x66,
    0x08, 0x00, 0x00, 0x73, 0xF6, 0x8B, 0x04, 0x85, 0xE8, 0xAA, 0xF7, 0x00, 0xC3,
];
const LINK_FLAGS_TO_MASK_BODY: [u8; 32] = [
    0x8A, 0x0A, 0x33, 0xC0, 0xF6, 0xC1, 0x08, 0x74, 0x05, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xF6, 0xC1,
    0x10, 0x74, 0x03, 0x83, 0xC8, 0x01, 0xF6, 0x42, 0x01, 0x01, 0x74, 0x03, 0x83, 0xC8, 0x04, 0xC3,
];
const ROUTE_CURRENT_ENTRY_BODY: [u8; 27] = [
    0x8A, 0x51, 0x3C, 0x84, 0xD2, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x8A, 0x41, 0x24, 0x3A, 0xC2, 0x73,
    0xF6, 0x0F, 0xB6, 0xC0, 0x8D, 0x04, 0x40, 0x8D, 0x04, 0x81, 0xC3,
];
const ORDER_IN_0_1_3_BODY: [u8; 26] = [
    0x8B, 0x80, 0x54, 0x85, 0x00, 0x00, 0x85, 0xC0, 0x7C, 0x0D, 0x83, 0xF8, 0x01, 0x7E, 0x05, 0x83,
    0xF8, 0x03, 0x75, 0x03, 0xB0, 0x01, 0xC3, 0x32, 0xC0, 0xC3,
];
const SCRATCH_RECORD_BODY: [u8; 29] = [
    0x8B, 0x48, 0x20, 0x3B, 0x0D, 0xBC, 0x60, 0xE1, 0x02, 0x75, 0x0F, 0x0F, 0xB7, 0x40, 0x1E, 0x8D,
    0x04, 0x40, 0x8D, 0x04, 0xC5, 0xC4, 0x60, 0xE1, 0x02, 0xC3, 0x33, 0xC0, 0xC3,
];
const QUERY_MASK_BODY: [u8; 49] = [
    0x85, 0xC0, 0x75, 0x05, 0x83, 0xC8, 0xFF, 0xEB, 0x0C, 0x8B, 0x80, 0xE4, 0x84, 0x00, 0x00, 0x8B,
    0x80, 0x9C, 0x00, 0x00, 0x00, 0x83, 0xE8, 0x02, 0x74, 0x11, 0x83, 0xE8, 0x02, 0x74, 0x06, 0xB8,
    0x00, 0x04, 0x00, 0x00, 0xC3, 0xB8, 0x00, 0x00, 0x04, 0x00, 0xC3, 0xB8, 0x00, 0x00, 0x20, 0x00,
    0xC3,
];
const LANE_BY_INDEX_BODY: [u8; 31] = [
    0x85, 0xC0, 0x7C, 0x18, 0x8B, 0x0D, 0x64, 0x5B, 0x22, 0x02, 0x3B, 0x01, 0x7D, 0x0E, 0x8B, 0xD0,
    0xC1, 0xE2, 0x04, 0x2B, 0xD0, 0x8B, 0x41, 0x04, 0x8D, 0x04, 0x90, 0xC3, 0x33, 0xC0, 0xC3,
];
const LANE_INDEX_BODY: [u8; 59] = [
    0x51, 0x85, 0xC0, 0x75, 0x05, 0x83, 0xC8, 0xFF, 0x59, 0xC3, 0x8B, 0x48, 0x10, 0xC1, 0xE9, 0x13,
    0xF6, 0xD1, 0x80, 0xE1, 0x01, 0x74, 0xEE, 0x8B, 0x0D, 0x64, 0x5B, 0x22, 0x02, 0x85, 0xC9, 0x74,
    0xE4, 0x2B, 0x41, 0x04, 0x8B, 0xC8, 0xB8, 0x89, 0x88, 0x88, 0x88, 0xF7, 0xE9, 0x03, 0xD1, 0xC1,
    0xFA, 0x05, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x59, 0xC3,
];
const LANE_FILTER_BODY: [u8; 22] = [
    0x51, 0x85, 0xC0, 0x74, 0x0D, 0x8B, 0x48, 0x10, 0xC1, 0xE9, 0x13, 0xF6, 0xD1, 0x80, 0xE1, 0x01,
    0x75, 0x02, 0x33, 0xC0, 0x59, 0xC3,
];
const ROUTE_TOTAL_LENGTH_BODY: [u8; 41] = [
    0x51, 0x85, 0xC9, 0xD9, 0xEE, 0xD9, 0x1C, 0x24, 0x8B, 0xC1, 0x74, 0x18, 0x8D, 0x64, 0x24, 0x00,
    0xD9, 0x40, 0x14, 0x8B, 0x40, 0x1C, 0x3B, 0xC1, 0xD8, 0x04, 0x24, 0xD9, 0x1C, 0x24, 0x74, 0x04,
    0x85, 0xC0, 0x75, 0xEC, 0xD9, 0x04, 0x24, 0x59, 0xC3,
];
