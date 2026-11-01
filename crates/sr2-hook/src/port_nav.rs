//! Port module: five small nav / vehicle-AI leaves of doc 47 ("Port candidates"), checked
//! against the disassembly. Register-only readers, each behind a `pushad` thunk (all registers
//! come back as the caller left them except EAX, the result). For the first `SHADOW_CALLS` calls
//! of each port a relocated copy of the original runs on the same registers and its EAX is
//! compared with the Rust version's; on a mismatch the original's is kept.
//!
//! Also: nav_cell_scratch_record, vehicle_nav_query_mask, road_lane_by_index / _index / _filter_bit19_clear, nav_route_total_length (ST0 + EAX shadow).
//!
//! Also (callees called by address through `call_regs`): nav_cell_has_link_flag2 (exact leftover EAX),
//! nav_cell_walkable, nav_cell_to_node_ref (-> 0xA02C10), vehicle_ai_set_speed_override /
//! vehicle_ai_clear_speed_override (-> 0xB2B8A0, shadowed up to the call) and nav_route_clear
//! (-> 0xA18500, no shadow: the list free writes too much to snapshot).

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
const EAX: u8 = 1;
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
const HAS_LINK_FLAG2: usize = 11;
const CELL_WALKABLE: usize = 12;
const CELL_TO_NODE_REF: usize = 13;
const SET_SPEED_OVERRIDE: usize = 14;
const CLEAR_SPEED_OVERRIDE: usize = 15;
const ROUTE_CLEAR: usize = 16;
const PAIR_MARK: usize = 17;
const PATH_RECYCLE: usize = 18;
const ROAD_RECYCLE: usize = 19;
const PATH_FREE_ALL: usize = 20;
const RECORD_FREE: usize = 21;
const LIST_FREE: usize = 22;
const CLAIM_ALLOC: usize = 23;
const CLAIM_FREE: usize = 24;
const CLAIM_ADD: usize = 25;
const CLAIM_REMOVE: usize = 26;
const ROUTE_PUSH: usize = 27;
const DETOUR_CREATE: usize = 28;
const DETOUR_DESTROY: usize = 29;
const RECORD_ALLOC: usize = 30;
const OBSTACLE_ALLOC: usize = 31;
const OBSTACLE_RELEASE: usize = 32;
const CHUNK_CELL_INDEX: usize = 33;
const CELL_LOCATE: usize = 34;
const SPLINE_EVAL_POS: usize = 35;
const COUNT: usize = 36;

const NAV_CELL_LOCATE: usize = 0x00A0_2C10;
const NAV_ROUTE_LIST_FREE: usize = 0x00A1_8500;
const AI_SET_FORCE_FLAG: usize = 0x00B2_B8A0;
const MEMMOVE: usize = 0x00C9_6060;
const NAV_PATH_RECYCLE: usize = 0x00A1_5B20;
const NAV_ROUTE_RECORD_FREE: usize = 0x00A1_8460;

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
    port("nav_cell_has_link_flag2", 0x00A0_5460, SAVED),
    port("nav_cell_walkable", 0x00A0_49F0, SAVED),
    port("nav_cell_to_node_ref", 0x00A0_5250, SAVED),
    port("vehicle_ai_set_speed_override", 0x00B2_74F0, ECX | EDX | SAVED),
    port("vehicle_ai_clear_speed_override", 0x00B2_7530, ECX | EDX | SAVED),
    port("nav_route_clear", 0x00A1_F730, 0),
    port("nav_cell_pair_mark", 0x00A0_2EA0, ECX | EDX | SAVED),
    port("nav_path_node_recycle", 0x00A1_5B20, EAX | SAVED),
    port("road_path_node_recycle", 0x00B1_CCD0, EAX | SAVED),
    port("nav_path_free_all", 0x00A1_5B00, SAVED),
    port("nav_route_record_free", 0x00A1_8460, ECX | SAVED),
    port("nav_route_list_free", 0x00A1_8500, SAVED),
    port("road_claim_alloc", 0x00A8_8D10, SAVED),
    port("road_claim_free", 0x00A8_8DB0, SAVED),
    port("road_lane_claim_add", 0x00A8_B6F0, ECX | SAVED),
    port("road_lane_claim_remove", 0x00A8_B7B0, SAVED),
    port("nav_route_push", 0x00A1_F770, SAVED),
    port("nav_detour_create", 0x00A0_DF80, SAVED),
    port("nav_detour_destroy", 0x00A0_E0C0, SAVED),
    port("nav_route_record_alloc", 0x00A1_7F90, SAVED),
    port("vehicle_ai_obstacle_record_alloc", 0x00B2_DB90, SAVED),
    port("vehicle_ai_obstacle_chain_release", 0x00B2_DC40, EAX | SAVED),
    port("nav_chunk_cell_index", 0x00A0_52D0, SAVED),
    port("nav_cell_locate", 0x00A0_2C10, ECX | SAVED),
    port("spline_eval_position", 0x00C0_3910, SAVED),
];

fn shadowing(id: usize) -> Option<(u32, usize)> {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_nav: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    // after a disagreement the port keeps being checked (and the original's result keeps winning)
    (orig != 0 && (n < SHADOW_CALLS || p.mismatches.load(Ordering::Relaxed) > 0)).then_some((n, orig))
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

// --- 0xA05460 nav_cell_has_link_flag2 (EAX = cell* -> AL, upper EAX = loop leftover) ----------------

/// `n = u8[cell + 0x1C]` link refs at `[cell + 0x18]`. Ref 0, chunk 0xFFFF / >= 0x866 (unsigned) or a
/// null record skip; else `link = (((ref & 0xFFFF) - 1) << 5) + [rec + 0x150]` (a zero sum skips) and
/// `u8[link + 0x18] & 2` gives 1. Upper EAX: 0 / the ref / lo - 1 / the link sum from the last step.
unsafe fn cell_has_link_flag2(cell: u32) -> u32 {
    unsafe {
        let n = rb(cell as usize + 0x1C) as u32;
        if n == 0 {
            return cell & !0xFF;
        }
        let mut p = rd(cell as usize + 0x18) as usize;
        let mut eax = cell;
        for _ in 0..n {
            let r = rd(p);
            p += 4;
            eax = r;
            if r == 0 {
                continue;
            }
            let hi = r >> 16;
            eax = (r & 0xFFFF).wrapping_sub(1);
            if hi == 0xFFFF || hi >= 0x866 {
                continue;
            }
            let rec = rd(CHUNKS + hi as usize * 4);
            if rec == 0 {
                continue;
            }
            eax = eax.wrapping_shl(5).wrapping_add(rd(rec as usize + 0x150));
            if eax == 0 {
                continue;
            }
            if rb(eax as usize + 0x18) & 2 != 0 {
                return (eax & !0xFF) | 1;
            }
        }
        eax & !0xFF
    }
}

extern "C" fn has_link_flag2_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(HAS_LINK_FLAG2, &p, unsafe { cell_has_link_flag2(p.eax) });
}
regs_thunk!(has_link_flag2_thunk, has_link_flag2_entry);

// --- 0xA049F0 nav_cell_walkable (EAX = cell* -> AL, upper EAX as the callee left it) ----------------

/// Byte +0x1D: bit 0x10 -> 0; bit 2 clear -> 1; else the result of nav_cell_has_link_flag2 (our
/// Rust version, which reproduces its EAX). Only AL is set; the rest of EAX is the input's or the callee's.
unsafe fn cell_walkable(cell: u32) -> u32 {
    unsafe {
        let f = rb(cell as usize + 0x1D);
        if f & 0x10 != 0 {
            return cell & !0xFF;
        }
        if f & 2 != 0 {
            let e = cell_has_link_flag2(cell);
            return if e & 0xFF == 0 { e } else { (e & !0xFF) | 1 };
        }
        (cell & !0xFF) | 1
    }
}

extern "C" fn cell_walkable_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(CELL_WALKABLE, &p, unsafe { cell_walkable(p.eax) });
}
regs_thunk!(cell_walkable_thunk, cell_walkable_entry);

// --- 0xA05250 nav_cell_to_node_ref (EAX = cell* -> EAX = ref or -1) ---------------------------------

/// Null cell or `u16[cell + 0x2C] == 0xFFFF`: -1. Else nav_cell_locate (0xA02C10) with EAX = cell,
/// ECX = &kind, EDX = &chunk, EDI = &idx and stack (&chunk, &kind), locals preset to 0xFFF / 1 / 0x7FFFF;
/// AL == 0 gives -1, else `chunk << 20 | (kind & 1) << 19 | idx & 0x7FFFF`.
unsafe fn cell_to_node_ref(cell: u32) -> u32 {
    unsafe {
        if cell == 0 || (rd(cell as usize + 0x2C) & 0xFFFF) == 0xFFFF {
            return 0xFFFF_FFFF;
        }
        let mut t = [0xFFFu32, 1, 0x7FFFF]; // chunk, kind, idx
        let b = t.as_mut_ptr();
        let (pc, pk, pi) = (b as u32, b.add(1) as u32, b.add(2) as u32);
        let al = call_regs(NAV_CELL_LOCATE, cell, pk, pc, 0, pi, &[pc, pk]) & 0xFF;
        if al == 0 {
            return 0xFFFF_FFFF;
        }
        let chunk = std::ptr::read_volatile(b);
        let kind = std::ptr::read_volatile(b.add(1));
        let idx = std::ptr::read_volatile(b.add(2));
        (chunk << 0x14) | ((kind & 1) << 0x13) | (idx & 0x7FFFF)
    }
}

extern "C" fn cell_to_node_ref_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(CELL_TO_NODE_REF, &p, unsafe { cell_to_node_ref(p.eax) });
}
regs_thunk!(cell_to_node_ref_thunk, cell_to_node_ref_entry);

// --- 0xB274F0 / 0xB27530 vehicle_ai_set / clear_speed_override (EAX = ai block) -----------------------

const F32_1000: u32 = 0x447A_0000;

/// The part before the call: speed < 1000.0 is kept, anything else (>= 1000, NaN) becomes 1000.0;
/// `[ai + 0x88] = speed` and bit 4 of byte `[ai + 0x16]` = CL bit 0.
unsafe fn speed_override_set(ai: u32, cl: u32, arg: u32) {
    unsafe {
        let s = if f32::from_bits(arg) < 1000.0 { arg } else { F32_1000 };
        wd(ai as usize + 0x88, s);
        let b = rb(ai as usize + 0x16);
        wb(ai as usize + 0x16, b ^ ((((cl as u8) << 4) ^ b) & 0x10));
    }
}

unsafe fn speed_override_clear(ai: u32) {
    unsafe {
        wd(ai as usize + 0x88, 0);
        wb(ai as usize + 0x16, rb(ai as usize + 0x16) & 0xEF);
    }
}

/// Run the shadow copy (the original cut before its call), put the memory back, run `ours`; on a
/// mismatch the original's two stores are kept. The caller makes the real call afterwards.
unsafe fn shadow_ai_stores(id: usize, p: &Pushad, arg: u32, ours: impl FnOnce()) {
    unsafe {
        let Some((n, orig)) = shadowing(id) else {
            ours();
            return;
        };
        let a = p.eax as usize;
        let (o88, o16) = (rd(a + 0x88), rb(a + 0x16));
        let inp = Regs::of(p);
        let mut out = inp;
        call_full(orig, &mut out, arg, false);
        check_preserved(id, &inp, &out);
        let (s88, s16) = (rd(a + 0x88), rb(a + 0x16));
        wd(a + 0x88, o88);
        wb(a + 0x16, o16);
        ours();
        let (r88, r16) = (rd(a + 0x88), rb(a + 0x16));
        if (r88, r16) != (s88, s16) {
            mismatch(id, n, format!("in {inp:x?} arg {arg:#x}: original {s88:#x}/{s16:#x}, Rust {r88:#x}/{r16:#x}"));
            wd(a + 0x88, s88);
            wb(a + 0x16, s16);
        }
    }
}

extern "C" fn set_speed_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let arg = *args;
        shadow_ai_stores(SET_SPEED_OVERRIDE, &p, arg, || speed_override_set(p.eax, p.ecx, arg));
        r.eax = call_regs(AI_SET_FORCE_FLAG, 0, 0, 0, 0, p.eax, &[1]);
    }
}
regs_thunk!(set_speed_thunk, set_speed_entry);

extern "C" fn clear_speed_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    unsafe {
        shadow_ai_stores(CLEAR_SPEED_OVERRIDE, &p, 0, || speed_override_clear(p.eax));
        r.eax = call_regs(AI_SET_FORCE_FLAG, 0, 0, 0, 0, p.eax, &[0]);
    }
}
regs_thunk!(clear_speed_thunk, clear_speed_entry);

// --- 0xA1F730 nav_route_clear (ESI = route or 0) ----------------------------------------------------

/// For each of the `u8[route + 0x3C]` entries (count re-read every step, signed compare) with bit 2 of
/// `[entry + 4]` set, nav_route_list_free(EAX = &entry) (entry = route + 4 + 12 * i); then the bytes
/// +0x3C, +0x24, +0x30 = 0. EAX leaves as the last count read (unchanged if there was none).
/// No shadow: the list free writes too much to snapshot.
extern "C" fn route_clear_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    PORTS[ROUTE_CLEAR].calls.fetch_add(1, Ordering::Relaxed);
    let route = p.esi as usize;
    if route == 0 {
        return;
    }
    unsafe {
        let mut eax = p.eax;
        if rb(route + 0x3C) != 0 {
            let mut i = 0u32;
            let mut e = route + 4;
            loop {
                if rb(e + 4) & 4 != 0 {
                    call_regs(NAV_ROUTE_LIST_FREE, e as u32, 0, 0, p.esi, i, &[]);
                }
                let n = rb(route + 0x3C) as u32;
                eax = n;
                i += 1;
                e += 12;
                if (i as i32) >= n as i32 {
                    break;
                }
            }
        }
        wb(route + 0x3C, 0);
        wb(route + 0x24, 0);
        wb(route + 0x30, 0);
        r.eax = eax;
    }
}
regs_thunk!(route_clear_thunk, route_clear_entry);


// --- shared shadow helpers for the memory writers ------------------------------------------------------

#[repr(C)]
struct Block4 {
    f: u32,
    r: Regs,
    esp: u32,
    args: [u32; 4],
}

/// Call `f` (a plain-`ret` function) with all seven registers from `r` and up to four stack
/// arguments, then capture all seven back into `r`. Returns the four stack slots as the callee left them.
unsafe fn call_full4(f: usize, r: &mut Regs, args: [u32; 4]) -> [u32; 4] {
    let mut b = Block4 { f: f as u32, r: *r, esp: 0, args };
    unsafe {
        std::arch::asm!(
            "push ebp", "push ebx", "push esi", "push edi",
            "mov [eax + 32], esp",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 48]", "push dword ptr [eax + 44]", "push dword ptr [eax + 40]", "push dword ptr [eax + 36]",
            "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 16]",
            "push ebx",
            "mov ebx, [esp + 24]",
            "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
            "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
            "mov ecx, [esp + 4]", "mov [ebx + 36], ecx",
            "mov ecx, [esp + 8]", "mov [ebx + 40], ecx",
            "mov ecx, [esp + 12]", "mov [ebx + 44], ecx",
            "mov ecx, [esp + 16]", "mov [ebx + 48], ecx",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 32]",
            "pop edi", "pop esi", "pop ebx", "pop ebp",
            inout("eax") &mut b as *mut Block4 => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
    b.args
}

/// Like [`call_full4`] for a callee that pops its first stack argument (`ret 4`).
/// Call `f` (a `ret 4` function) with all seven registers from `r` and up to four stack
/// arguments, then capture all seven back into `r`. Returns the four stack slots as the callee left them.
unsafe fn call_full4_ret4(f: usize, r: &mut Regs, args: [u32; 4]) -> [u32; 4] {
    let mut b = Block4 { f: f as u32, r: *r, esp: 0, args };
    unsafe {
        std::arch::asm!(
            "push ebp", "push ebx", "push esi", "push edi",
            "mov [eax + 32], esp",
            "push eax",
            "push dword ptr [eax]",
            "push dword ptr [eax + 48]", "push dword ptr [eax + 44]", "push dword ptr [eax + 40]", "push dword ptr [eax + 36]",
            "mov ecx, [eax + 8]", "mov edx, [eax + 12]", "mov ebx, [eax + 16]",
            "mov esi, [eax + 20]", "mov edi, [eax + 24]", "mov ebp, [eax + 28]",
            "mov eax, [eax + 4]",
            "call dword ptr [esp + 16]",
            "sub esp, 4",
            "push ebx",
            "mov ebx, [esp + 24]",
            "mov [ebx + 4], eax", "mov [ebx + 8], ecx", "mov [ebx + 12], edx",
            "mov [ebx + 20], esi", "mov [ebx + 24], edi", "mov [ebx + 28], ebp",
            "mov ecx, [esp + 4]", "mov [ebx + 36], ecx",
            "mov ecx, [esp + 8]", "mov [ebx + 40], ecx",
            "mov ecx, [esp + 12]", "mov [ebx + 44], ecx",
            "mov ecx, [esp + 16]", "mov [ebx + 48], ecx",
            "pop dword ptr [ebx + 16]",
            "mov esp, [ebx + 32]",
            "pop edi", "pop esi", "pop ebx", "pop ebp",
            inout("eax") &mut b as *mut Block4 => _,
            lateout("ecx") _,
            lateout("edx") _,
            clobber_abi("C"),
        );
    }
    *r = b.r;
    b.args
}

unsafe fn snap(regions: &[(usize, usize)]) -> Vec<Vec<u8>> {
    regions.iter().map(|&(a, n)| unsafe { std::slice::from_raw_parts(a as *const u8, n) }.to_vec()).collect()
}

unsafe fn restore(regions: &[(usize, usize)], s: &[Vec<u8>]) {
    for (&(a, _), b) in regions.iter().zip(s) {
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), a as *mut u8, b.len()) };
    }
}

/// Shadow for a memory writer. While the port is still being checked: snapshot `regions` (every
/// byte the original can write, computed from the pre-state; `None` = cannot be bounded, then
/// `ours` just runs), run the relocated original, save what it wrote, restore, run `ours`, compare
/// the memory, the first `nargs` stack-argument dwords at `argp` (the originals scribble on them;
/// the copy the original saw is compared with the real slots `ours` left) and EAX when `cmp_eax`.
/// On a mismatch the original's writes are put back and its EAX is returned. Returns EAX as `ours`
/// / the original produced it.
unsafe fn shadow_run(
    id: usize,
    p: &Pushad,
    argp: *mut u32,
    nargs: usize,
    cmp_eax: bool,
    regions: impl FnOnce() -> Option<Vec<(usize, usize)>>,
    ours: impl FnOnce() -> u32,
) -> u32 {
    unsafe {
        let Some((n, orig)) = shadowing(id) else { return ours() };
        let Some(reg) = regions() else { return ours() };
        let mut args = [0u32; 4];
        for (i, a) in args.iter_mut().enumerate().take(nargs) {
            *a = *argp.add(i);
        }
        let pre = snap(&reg);
        let inp = Regs::of(p);
        let mut o = inp;
        let oargs = if id == SPLINE_EVAL_POS { call_full4_ret4(orig, &mut o, args) } else { call_full4(orig, &mut o, args) };
        check_preserved(id, &inp, &o);
        let after_orig = snap(&reg);
        restore(&reg, &pre);
        let e = ours();
        let after_ours = snap(&reg);
        let args_ok = (0..nargs).all(|i| *argp.add(i) == oargs[i]);
        if after_ours != after_orig || !args_ok || (cmp_eax && e != o.eax) {
            let diff = after_ours.iter().zip(&after_orig).position(|(a, b)| a != b);
            mismatch(id, n, format!("in {inp:x?} args {args:x?}: eax original {:#x}, Rust {e:#x}; first differing region {diff:?}, args ok {args_ok}", o.eax));
            restore(&reg, &after_orig);
            for (i, &a) in oargs.iter().enumerate().take(nargs) {
                *argp.add(i) = a;
            }
            return o.eax;
        }
        e
    }
}

const ROUTE_POOL: usize = 0x02E4_00D0;
const ROUTE_POOL_SIZE: usize = 0x6800;
const ROUTE_FREE_HEAD: usize = 0x0220_3AB8;
const PATH_RING: usize = 0x021F_C19C;
const ROAD_RING: usize = 0x022A_9C7C;

// --- 0xA02EA0 nav_cell_pair_mark (ECX = cell A, EDX = cell B; AL = B's flag byte, | 0x20 when set) -----

/// B's flag byte (+0x1D) with bit 0x20 already set, or A's with it set: nothing. Else B = B | 0x20 and
/// A |= 0x20. Returns EAX (only AL is touched: B's byte, with 0x20 added when the marks were written).
unsafe fn pair_mark(a: u32, b: u32, eax: u32) -> u32 {
    unsafe {
        let al = rb(b as usize + 0x1D);
        if al & 0x20 != 0 || rb(a as usize + 0x1D) & 0x20 != 0 {
            return (eax & !0xFF) | al as u32;
        }
        wb(b as usize + 0x1D, al | 0x20);
        wb(a as usize + 0x1D, rb(a as usize + 0x1D) | 0x20);
        (eax & !0xFF) | (al | 0x20) as u32
    }
}

extern "C" fn pair_mark_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = unsafe {
        shadow_run(
            PAIR_MARK,
            &p,
            std::ptr::null_mut(),
            0,
            true,
            || Some(vec![(p.ecx as usize + 0x1D, 1), (p.edx as usize + 0x1D, 1)]),
            || pair_mark(p.ecx, p.edx, p.eax),
        )
    };
}
regs_thunk!(pair_mark_thunk, pair_mark_entry);

// --- 0xA15B20 nav_path_node_recycle / 0xB1CCD0 road_path_node_recycle (EAX = node, EDX = &owner head slot) --

/// Unlink `node` (links +0 next, +4 prev) from the owner ring (head slot `slot`), then put it before the
/// head of the global ring `ring` (the head stays). A single-node owner ring zeroes the node and the slot
/// and skips the generic unlink. Same code, different ring: 0x21FC19C (path) / 0x22A9C7C (road).
unsafe fn ring_recycle(node: u32, slot: u32, ring: usize) {
    unsafe {
        let (n, s) = (node as usize, slot as usize);
        let mut generic = true;
        if rd(s) == node {
            let c = rd(n);
            if c == node {
                wd(n, 0);
                wd(n + 4, 0);
                wd(s, 0);
                generic = false;
            } else {
                wd(s, c);
            }
        }
        if generic {
            let prev = rd(n + 4) as usize;
            if prev != 0 {
                wd(prev, rd(n));
            }
            let next = rd(n) as usize;
            if next != 0 {
                wd(next + 4, rd(n + 4));
            }
            wd(n, 0);
            wd(n + 4, 0);
        }
        let h = rd(ring) as usize;
        if h == 0 {
            wd(n, node);
            wd(n + 4, node);
            wd(ring, node);
            return;
        }
        wd(n + 4, rd(h + 4));
        wd(n, h as u32);
        let t = rd(h + 4) as usize;
        wd(t, node);
        wd(h + 4, node);
    }
}

fn recycle_regions(node: u32, slot: u32, ring: usize) -> Option<Vec<(usize, usize)>> {
    unsafe {
        let n = node as usize;
        let mut v = vec![(n, 8), (slot as usize, 4), (ring, 4)];
        let (prev, next, h) = (rd(n + 4) as usize, rd(n) as usize, rd(ring) as usize);
        if prev != 0 {
            v.push((prev, 4));
        }
        if next != 0 {
            v.push((next + 4, 4));
        }
        if h != 0 {
            v.push((h + 4, 4));
            let t = rd(h + 4) as usize;
            if t != 0 {
                v.push((t, 4));
            }
        }
        Some(v)
    }
}

extern "C" fn path_recycle_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    unsafe {
        shadow_run(PATH_RECYCLE, &p, std::ptr::null_mut(), 0, false, || recycle_regions(p.eax, p.edx, PATH_RING), || {
            ring_recycle(p.eax, p.edx, PATH_RING);
            0
        })
    };
}
regs_thunk!(path_recycle_thunk, path_recycle_entry);

extern "C" fn road_recycle_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    unsafe {
        shadow_run(ROAD_RECYCLE, &p, std::ptr::null_mut(), 0, false, || recycle_regions(p.eax, p.edx, ROAD_RING), || {
            ring_recycle(p.eax, p.edx, ROAD_RING);
            0
        })
    };
}
regs_thunk!(road_recycle_thunk, road_recycle_entry);

// --- 0xA15B00 nav_path_free_all (EDI = &head slot; EAX leaves 0) ------------------------------------------

/// `node = [slot]`; while non-null: `next = [node]` unless it equals the head slot's current value (the
/// `sub / neg / sbb / and` idiom, read BEFORE the recycle); nav_path_node_recycle(EAX = node, EDX = slot);
/// node = next. The loop exits with EAX = 0.
unsafe fn path_free_all(slot: u32) {
    unsafe {
        let mut node = rd(slot as usize);
        while node != 0 {
            let c = rd(node as usize);
            let next = if c != rd(slot as usize) { c } else { 0 };
            call_regs(NAV_PATH_RECYCLE, node, 0, slot, next, slot, &[]);
            node = next;
        }
    }
}

extern "C" fn path_free_all_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let slot = p.edi;
    unsafe {
        shadow_run(
            PATH_FREE_ALL,
            &p,
            std::ptr::null_mut(),
            0,
            false,
            || {
                // every node of the owner ring (links +0 / +4), the slot, the global ring head and its tail
                let head = rd(slot as usize);
                let mut v = vec![(slot as usize, 4), (PATH_RING, 4)];
                let mut n = head;
                let mut steps = 0;
                while n != 0 {
                    v.push((n as usize, 8));
                    n = rd(n as usize);
                    steps += 1;
                    if n == head {
                        break;
                    }
                    if steps > 4096 {
                        return None;
                    }
                }
                let h = rd(PATH_RING) as usize;
                if h != 0 {
                    v.push((h + 4, 4));
                    let t = rd(h + 4) as usize;
                    if t != 0 {
                        v.push((t, 4));
                    }
                }
                Some(v)
            },
            || {
                path_free_all(slot);
                0
            },
        )
    };
    r.eax = 0;
}
regs_thunk!(path_free_all_thunk, path_free_all_entry);

// --- 0xA18460 nav_route_record_free (ECX = record, EDI = &owner slot) --------------------------------------

/// Pool record test: `rec - 0x2E400D0` not negative, < 0x6800, a multiple of 0x34, byte +0x30 bit 0x20 set.
unsafe fn route_rec_valid(rec: u32) -> bool {
    unsafe {
        let d = rec.wrapping_sub(ROUTE_POOL as u32);
        (d as i32) >= 0 && (d as usize) < ROUTE_POOL_SIZE && d % 0x34 == 0 && rb(rec as usize + 0x30) & 0x20 != 0
    }
}

/// Not valid: nothing. Valid: if `[slot] == rec` the slot takes the next link (a single-node ring zeroes
/// the links and the slot and skips the generic unlink); generic unlink through links +0x1C next / +0x20
/// prev; clear flag 0x20; insert before the head of the free ring 0x2203AB8 (an empty ring takes it).
unsafe fn route_record_free(rec: u32, slot: u32) {
    unsafe {
        if !route_rec_valid(rec) {
            return;
        }
        let (r, s) = (rec as usize, slot as usize);
        let mut generic = true;
        if rd(s) == rec {
            let nx = rd(r + 0x1C);
            if nx == rec {
                wd(r + 0x1C, 0);
                wd(r + 0x20, 0);
                wd(s, 0);
                generic = false;
            } else {
                wd(s, nx);
            }
        }
        if generic {
            let prev = rd(r + 0x20) as usize;
            if prev != 0 {
                wd(prev + 0x1C, rd(r + 0x1C));
            }
            let next = rd(r + 0x1C) as usize;
            if next != 0 {
                wd(next + 0x20, rd(r + 0x20));
            }
            wd(r + 0x1C, 0);
            wd(r + 0x20, 0);
        }
        let h = rd(ROUTE_FREE_HEAD) as usize;
        if h == 0 {
            wb(r + 0x30, rb(r + 0x30) & 0xDF);
            wd(r + 0x1C, rec);
            wd(r + 0x20, rec);
            wd(ROUTE_FREE_HEAD, rec);
            return;
        }
        wd(r + 0x20, rd(h + 0x20));
        wd(r + 0x1C, h as u32);
        let t = rd(h + 0x20) as usize;
        wd(t + 0x1C, rec);
        wd(h + 0x20, rec);
        wb(r + 0x30, rb(r + 0x30) & 0xDF);
    }
}

/// The whole record pool, the free-ring head and the slot cover every write of record_free / list_free
/// (all the ring neighbours are pool records).
fn route_pool_regions(slot: u32) -> Option<Vec<(usize, usize)>> {
    let mut v = vec![(ROUTE_POOL, ROUTE_POOL_SIZE), (ROUTE_FREE_HEAD, 4)];
    if slot != 0 {
        v.push((slot as usize, 4));
    }
    Some(v)
}

extern "C" fn record_free_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    unsafe {
        shadow_run(RECORD_FREE, &p, std::ptr::null_mut(), 0, false, || route_pool_regions(p.edi), || {
            route_record_free(p.ecx, p.edi);
            0
        })
    };
}
regs_thunk!(record_free_thunk, record_free_entry);

// --- 0xA18500 nav_route_list_free (EAX = &slot holding a record ptr) ----------------------------------------

/// slot 0 or `[slot] == 0`: nothing. A first record that fails the pool test clears the slot. Otherwise
/// every record of the ring is freed: `next = [rec + 0x1C]` unless it equals the slot's CURRENT value (the
/// sbb idiom, read before the free), nav_route_record_free(ECX = rec, EDI = slot) through the original
/// address (EAX = next on entry, as in the original), rec = next. Later records are validated by the callee.
unsafe fn route_list_free(slot: u32) {
    unsafe {
        if slot == 0 {
            return;
        }
        let mut rec = rd(slot as usize);
        if rec == 0 {
            return;
        }
        if !route_rec_valid(rec) {
            wd(slot as usize, 0);
            return;
        }
        while rec != 0 {
            let nx = rd(rec as usize + 0x1C);
            let esi = if nx != rd(slot as usize) { nx } else { 0 };
            call_regs(NAV_ROUTE_RECORD_FREE, nx, rec, 0, esi, slot, &[]);
            rec = esi;
        }
    }
}

extern "C" fn list_free_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    unsafe {
        shadow_run(LIST_FREE, &p, std::ptr::null_mut(), 0, false, || route_pool_regions(p.eax), || {
            route_list_free(p.eax);
            0
        })
    };
}
regs_thunk!(list_free_thunk, list_free_entry);


// --- road lane claims (12-byte entries {u16 next, u16 prev, u32 owner, u32}, 107 of them) --------------------

const CLAIMS: usize = 0x0223_E410;
const CLAIM_POOL_BYTES: usize = 107 * 12;
/// u16 head of the free claim ring.
const CLAIM_FREE_HEAD: usize = 0x00E8_7B4C;
const ROAD_NET: usize = 0x0222_5B64;
const CLAIM_ALLOC_ADDR: usize = 0x00A8_8D10;
const CLAIM_FREE_ADDR: usize = 0x00A8_8DB0;

unsafe fn rw(p: usize) -> u16 {
    unsafe { std::ptr::read_unaligned(p as *const u16) }
}

unsafe fn ww(p: usize, v: u16) {
    unsafe { std::ptr::write_unaligned(p as *mut u16, v) }
}

/// The compiler's `(claim - pool) / 12`: `hi32(d * 0x2AAAAAAB)`, sar 1, plus the sign bit (low 16 bits used).
fn claim_index(claim: u32) -> u16 {
    let d = claim.wrapping_sub(CLAIMS as u32) as i32;
    let hi = ((d as i64 * 0x2AAA_AAAB) >> 32) as i32;
    let q = (hi >> 1) as u32;
    q.wrapping_add(q >> 31) as u16
}

/// Pop the head of the free claim ring (u16 head 0xE87B4C, 0xFFFF = empty, then 0). A single-entry ring
/// leaves the head 0xFFFF; otherwise the head takes the next index and the entry is unlinked
/// (prev.next = next, next.prev = prev). The claim comes back with both links 0xFFFF.
unsafe fn claim_alloc() -> u32 {
    unsafe {
        let si = rw(CLAIM_FREE_HEAD);
        if si == 0xFFFF {
            return 0;
        }
        let ecx = (CLAIMS + si as usize * 12) as u32;
        let ax = claim_index(ecx);
        let c = ecx as usize;
        if si == ax {
            let nx = rw(c);
            if nx == ax {
                ww(CLAIM_FREE_HEAD, 0xFFFF);
                ww(c + 2, 0xFFFF);
                ww(c, 0xFFFF);
                return ecx;
            }
            ww(CLAIM_FREE_HEAD, nx);
        }
        let prev = rw(c + 2) as usize;
        let next = rw(c);
        ww(CLAIMS + prev * 12, next);
        let next2 = rw(c) as usize;
        let prev2 = rw(c + 2);
        ww(CLAIMS + next2 * 12 + 2, prev2);
        ww(c + 2, 0xFFFF);
        ww(c, 0xFFFF);
        ecx
    }
}

/// The claim pool, the free head and the lane head word cover every write of the four claim functions.
fn claim_regions(lane_head: u32) -> Option<Vec<(usize, usize)>> {
    let mut v = vec![(CLAIMS, CLAIM_POOL_BYTES), (CLAIM_FREE_HEAD, 2)];
    if lane_head != 0 {
        v.push((lane_head as usize, 2));
    }
    Some(v)
}

extern "C" fn claim_alloc_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = unsafe { shadow_run(CLAIM_ALLOC, &p, std::ptr::null_mut(), 0, true, || claim_regions(0), || claim_alloc()) };
}
regs_thunk!(claim_alloc_thunk, claim_alloc_entry);

/// EAX = the lane's u16 head word (lane + 0x2A), ECX = claim. Marks claim+4 = -1, unlinks the claim from
/// the lane ring (the head word takes the next index, or 0xFFFF for a single entry) and appends it to the
/// free ring before its head (an empty free ring takes it as head with both links to itself).
unsafe fn claim_free(head: u32, claim: u32) {
    unsafe {
        let h = head as usize;
        let c = claim as usize;
        let ax = claim_index(claim);
        wd(c + 4, 0xFFFF_FFFF);
        let mut generic = true;
        if rw(h) == ax {
            let dx = rw(c);
            if dx == ax {
                ww(c, 0xFFFF);
                ww(c + 2, 0xFFFF);
                ww(h, 0xFFFF);
                generic = false;
            } else {
                ww(h, dx);
            }
        }
        if generic {
            let prev = rw(c + 2) as usize;
            let next = rw(c) as usize;
            ww(CLAIMS + prev * 12, next as u16);
            let next2 = rw(c) as usize;
            let prev2 = rw(c + 2);
            ww(CLAIMS + next2 * 12 + 2, prev2);
            ww(c, 0xFFFF);
            ww(c + 2, 0xFFFF);
        }
        let si = rw(CLAIM_FREE_HEAD);
        if si == 0xFFFF {
            ww(c, ax);
            ww(c + 2, ax);
            ww(CLAIM_FREE_HEAD, ax);
            return;
        }
        let e = CLAIMS + si as usize * 12;
        let di = rw(e + 2);
        ww(c + 2, di);
        ww(c, si);
        let tail = rw(e + 2) as usize;
        ww(CLAIMS + tail * 12, ax);
        ww(e + 2, ax);
    }
}

extern "C" fn claim_free_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    unsafe {
        shadow_run(CLAIM_FREE, &p, std::ptr::null_mut(), 0, false, || claim_regions(p.eax), || {
            claim_free(p.eax, p.ecx);
            0
        })
    };
}
regs_thunk!(claim_free_thunk, claim_free_entry);

/// ESI = lane (0 or a lane whose `[lane + 0x10]` bit 19 is set: nothing), stack arg = owner. Allocates a
/// claim (0xA88D10, ESI kept); owner at +4, 0 at +8; an empty lane ring (u16 `[lane + 0x2A]` = 0xFFFF)
/// takes the claim as its only member (both links to itself); otherwise the claim goes in before the
/// ring head (claim.prev = head.prev, claim.next = head, old tail.next = head.prev = claim).
unsafe fn claim_add(lane: u32, owner: u32) {
    unsafe {
        if lane == 0 || (rd(lane as usize + 0x10) >> 0x13) & 1 != 0 {
            return;
        }
        let claim = call_regs(CLAIM_ALLOC_ADDR, 0, 0, 0, lane, 0, &[]);
        if claim == 0 {
            return;
        }
        let c = claim as usize;
        let lh = lane as usize + 0x2A;
        wd(c + 4, owner);
        wd(c + 8, 0);
        let head = rw(lh);
        let idx = claim_index(claim);
        if head != 0xFFFF {
            let dx = rw(CLAIMS + head as usize * 12 + 2);
            ww(c + 2, dx);
            ww(c, rw(lh));
            let oldtail = rw(CLAIMS + rw(lh) as usize * 12 + 2) as usize;
            ww(CLAIMS + oldtail * 12, idx);
            ww(CLAIMS + rw(lh) as usize * 12 + 2, idx);
        } else {
            ww(c, idx);
            ww(c + 2, idx);
            ww(lh, idx);
        }
    }
}

extern "C" fn claim_add_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let owner = *args;
        shadow_run(
            CLAIM_ADD,
            &p,
            args as *mut u32,
            1,
            false,
            || claim_regions(if p.esi != 0 { p.esi + 0x2A } else { 0 }),
            || {
                claim_add(p.esi, owner);
                0
            },
        )
    };
}
regs_thunk!(claim_add_thunk, claim_add_entry);

/// EAX = lane, EDI = owner. Nothing without a lane, with bit 19 of `[lane + 0x10]` set, or without a road
/// network (`[0x2225B64] == 0`). Walks the lane's claim ring from the u16 head `[lane + 0x2A]` (stops when
/// the next index equals the head index) for `claim+4 == owner` and tail-calls road_claim_free(EAX = head
/// word address, ECX = claim) (0xA88DB0, ESI kept).
unsafe fn claim_remove(lane: u32, owner: u32, esi: u32) {
    unsafe {
        if lane == 0 || (rd(lane as usize + 0x10) >> 0x13) & 1 != 0 || rd(ROAD_NET) == 0 {
            return;
        }
        let head = lane + 0x2A;
        let si = rw(head as usize);
        let mut ecx = if si == 0xFFFF { 0 } else { (CLAIMS + si as usize * 12) as u32 };
        while ecx != 0 {
            let dx = rw(ecx as usize);
            let edx = if dx == si { 0 } else { (CLAIMS + dx as usize * 12) as u32 };
            if rd(ecx as usize + 4) == owner {
                call_regs(CLAIM_FREE_ADDR, head, ecx, edx, esi, owner, &[]);
                return;
            }
            ecx = edx;
        }
    }
}

extern "C" fn claim_remove_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    unsafe {
        shadow_run(CLAIM_REMOVE, &p, std::ptr::null_mut(), 0, false, || claim_regions(if p.eax != 0 { p.eax + 0x2A } else { 0 }), || {
            claim_remove(p.eax, p.edi, p.esi);
            0
        })
    };
}
regs_thunk!(claim_remove_thunk, claim_remove_entry);

// --- 0xA1F770 nav_route_push (ESI = route; stack: record*, bool, bool) --------------------------------------

/// A float loaded and stored back by x87 (`fld dword / fstp dword`): a signalling NaN comes out quiet.
fn f32_through_x87(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x7F_FFFF != 0 { bits | 0x40_0000 } else { bits }
}

/// Route entries are 12 bytes at `route + 12 * i`: {f32 cost, record*, flags byte (+8)}; count byte +0x3C.
/// Record 0: nothing. Count < 3: an entry already holding the record means nothing changes, else the
/// count grows and the new entry is the next slot. Count >= 3 (unsigned): the first entry's list is freed
/// when its flag 4 is set (list_free on `route + 4`), entries 1..2 move down by memmove(route, route + 12,
/// 24), bytes +0x30 and +0x24 are decremented and the new entry is the third (route + 0x18). Flags byte:
/// (arg3 & 1) << 2 | (arg2 & 1) << 1 | old & 0xE0, then bit 5 = (record byte +0x30 << 2) & 0x20 when
/// `arg3 & 0xFF` is non-zero, else cleared. [entry + 4] = record. Cost f32: record `[+0x10]` bit 19 set
/// gives 1000.0 (0xE98030); clear: lane type = byte `[+0x28] >> 4`, >= 3 gives 10.0 (0xDD0550), else the
/// f32 table 0x223E3E8[type]. The cost also lands in the (dead) first stack-argument slot.
unsafe fn route_push(route: u32, args: *mut u32) {
    unsafe {
        let rec = *args;
        let a2 = *args.add(1) as u8;
        let a3 = *args.add(2) as u8;
        if rec == 0 {
            return;
        }
        let rt = route as usize;
        let cnt = rb(rt + 0x3C);
        let e;
        if cnt < 3 {
            for i in 0..cnt as usize {
                if rd(rt + 4 + 12 * i) == rec {
                    return;
                }
            }
            wb(rt + 0x3C, cnt + 1);
            e = rt + 12 * cnt as usize;
        } else {
            if rb(rt + 8) & 4 != 0 {
                call_regs(NAV_ROUTE_LIST_FREE, route + 4, 0, 0, route, 0, &[]);
            }
            call_regs(MEMMOVE, 0, 0, 0, 0, 0, &[route, route + 12, 0x18]);
            wb(rt + 0x30, rb(rt + 0x30).wrapping_add(0xFF));
            wb(rt + 0x24, rb(rt + 0x24).wrapping_add(0xFF));
            e = rt + 0x18;
        }
        let old = rb(e + 8);
        let mut f = (((a3 & 1) << 1 | (a2 & 1)) << 1) | (old & 0xE0);
        if a3 != 0 {
            let d = rb(rec as usize + 0x30).wrapping_shl(2);
            f = ((d ^ f) & 0x20) ^ f;
        } else {
            f &= 0xDF;
        }
        wb(e + 8, f);
        wd(e + 4, rec);
        let bits = if (rd(rec as usize + 0x10) >> 0x13) & 1 != 0 {
            rd(0x00E9_8030)
        } else {
            let t = rb(rec as usize + 0x28) >> 4;
            if t >= 3 { rd(0x00DD_0550) } else { rd(0x0223_E3E8 + t as usize * 4) }
        };
        let q = f32_through_x87(bits);
        *args = q;
        wd(e, q);
    }
}

extern "C" fn route_push_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        shadow_run(
            ROUTE_PUSH,
            &p,
            args as *mut u32,
            3,
            false,
            || {
                let mut v = vec![(p.esi as usize, 0x40)];
                if rb(p.esi as usize + 0x3C) >= 3 && rb(p.esi as usize + 8) & 4 != 0 {
                    v.push((ROUTE_POOL, ROUTE_POOL_SIZE));
                    v.push((ROUTE_FREE_HEAD, 4));
                }
                Some(v)
            },
            || {
                route_push(p.esi, args as *mut u32);
                0
            },
        )
    };
}
regs_thunk!(route_push_thunk, route_push_entry);

// --- nav detours: 40-byte entries at 0x2DEE0B8, rings 0x21F2C04 (free) / 0x21F2C08 (active) ------------------

const DET_TABLE: usize = 0x02DE_E0B8;
const DET_GEN: usize = 0x021F_2C00;
const DET_FREE: usize = 0x021F_2C04;
const DET_ACTIVE: usize = 0x021F_2C08;
const DET_RELEASE_HULL: usize = 0x00A0_AED0;

/// EAX = owner handle, EDX = kind, stack = u16 extra. Kind 0: object table 0x2149C64 (record
/// `[((h & 0xFFF) << 4) + base]`, non-null, `[rec + 0x44] == h`); kind 1: `0x2AE50E0 + (h & 0xFFFF) * 0x134`
/// with `[+8] == h`; kind 2: `0x2674DE0 + (h & 0x1FF) * 0x208` with `[+0x10] == h`; a zero handle or any other
/// kind gives -1. Then the head of the free ring (-1 when empty) becomes the entry: +0x10 = kind, +0xC =
/// owner, u16 +0x14 = extra, handle = index | old generation << 12 (index = (entry - table) / 40, the
/// generation counter 0x21F2C00 advances by one, & 0x7FFFF), +0x18 = 0; it leaves the free ring (head
/// moves on, 0 when it was the only one) and goes in at the FRONT of the active ring (head = entry,
/// entry.next = old head, old tail.next = old head.prev = entry; an empty ring links the entry to itself).
unsafe fn detour_create(owner: u32, kind: u32, extra: u16) -> u32 {
    unsafe {
        let ok = match kind {
            0 => {
                owner != 0 && {
                    let rec = rd(0x0214_9C64 + ((owner & 0xFFF) << 4) as usize);
                    rec != 0 && rd(rec as usize + 0x44) == owner
                }
            }
            1 => owner != 0 && rd(0x02AE_50E0 + (owner & 0xFFFF) as usize * 0x134 + 8) == owner,
            2 => owner != 0 && rd(0x0267_4DE0 + (owner & 0x1FF) as usize * 0x208 + 0x10) == owner,
            _ => false,
        };
        if !ok {
            return 0xFFFF_FFFF;
        }
        let e = rd(DET_FREE) as usize;
        if e == 0 {
            return 0xFFFF_FFFF;
        }
        wd(e + 0x10, kind);
        wd(e + 0xC, owner);
        ww(e + 0x14, extra);
        let d = (e as u32).wrapping_sub(DET_TABLE as u32) as i32;
        let hi = ((d as i64 * 0x6666_6667) >> 32) as i32;
        let q = (hi >> 4) as u32;
        let idx = q.wrapping_add(q >> 31);
        let gen_old = rd(DET_GEN);
        wd(DET_GEN, gen_old.wrapping_add(1) & 0x7FFFF);
        let next = rd(e);
        let handle = idx | (gen_old << 12);
        wd(e + 0x18, 0);
        wd(e + 8, handle);
        if next == e as u32 {
            wd(DET_FREE, 0);
        } else {
            let prev = rd(e + 4) as usize;
            wd(DET_FREE, next);
            if prev != 0 {
                wd(prev, rd(e));
            }
            let nx = rd(e) as usize;
            if nx != 0 {
                wd(nx + 4, rd(e + 4));
            }
        }
        let head = rd(DET_ACTIVE) as usize;
        wd(e + 4, 0);
        wd(e, 0);
        if head == 0 {
            wd(e + 4, e as u32);
            wd(e, e as u32);
            wd(DET_ACTIVE, e as u32);
            return rd(e + 8);
        }
        let tail = rd(head + 4) as usize;
        wd(tail, e as u32);
        wd(e + 4, rd(head + 4));
        wd(head + 4, e as u32);
        wd(e, head as u32);
        wd(DET_ACTIVE, e as u32);
        rd(e + 8)
    }
}

/// Entries touched by a detour create / destroy: the entry, its ring neighbours, both ring heads and the
/// tails, plus the three globals.
fn detour_regions(e: usize, linked: bool) -> Vec<(usize, usize)> {
    unsafe {
        let mut v = vec![(DET_GEN, 12)];
        let mut add = |p: usize| {
            if p != 0 {
                v.push((p, 40));
            }
        };
        add(e);
        if e != 0 && linked {
            add(rd(e + 4) as usize);
            add(rd(e) as usize);
        }
        for ring in [DET_FREE, DET_ACTIVE] {
            let h = rd(ring) as usize;
            add(h);
            if h != 0 {
                add(rd(h + 4) as usize);
            }
        }
        v
    }
}

extern "C" fn detour_create_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let extra = *args as u16;
        r.eax = shadow_run(
            DETOUR_CREATE,
            &p,
            args as *mut u32,
            1,
            true,
            || Some(detour_regions(rd(DET_FREE) as usize, true)),
            || detour_create(p.eax, p.edx, extra),
        );
    }
}
regs_thunk!(detour_create_thunk, detour_create_entry);

/// ECX = handle. -1, or an entry whose +8 is not the handle (generation / index check), or one already
/// holding -1: nothing. Else +8 = -1 and, when the hull pointer +0x18 is non-null, nav_detour_release_hull
/// (0xA0AED0; EDI = entry, which also needs EAX = index, ECX = handle, EDX = -1 as set up here) runs;
/// then the entry leaves the active ring (head = next, or 0 for a lone entry which skips the unlink) and
/// goes in before the head of the free ring (an empty one takes it as head, linked to itself).
unsafe fn detour_destroy(h: u32, esi: u32, call_release: bool) {
    unsafe {
        if h == 0xFFFF_FFFF {
            return;
        }
        let idx = h & 0xFFF;
        let e = DET_TABLE + idx as usize * 40;
        if rd(e + 8) != h || rd(e + 8) == 0xFFFF_FFFF {
            return;
        }
        let hull = rd(e + 0x18);
        wd(e + 8, 0xFFFF_FFFF);
        if hull != 0 && call_release {
            call_regs(DET_RELEASE_HULL, idx, h, 0xFFFF_FFFF, esi, e as u32, &[]);
        }
        let mut unlink = true;
        if rd(DET_ACTIVE) == e as u32 {
            let nx = rd(e);
            if nx == e as u32 {
                wd(DET_ACTIVE, 0);
                unlink = false;
            } else {
                wd(DET_ACTIVE, nx);
            }
        }
        if unlink {
            let prev = rd(e + 4) as usize;
            if prev != 0 {
                wd(prev, rd(e));
            }
            let nx = rd(e) as usize;
            if nx != 0 {
                wd(nx + 4, rd(e + 4));
            }
        }
        let f = rd(DET_FREE) as usize;
        wd(e + 4, 0);
        wd(e, 0);
        if f == 0 {
            wd(e, e as u32);
            wd(e + 4, e as u32);
            wd(DET_FREE, e as u32);
            return;
        }
        wd(e + 4, rd(f + 4));
        wd(e, f as u32);
        let t = rd(f + 4) as usize;
        wd(t, e as u32);
        wd(f + 4, e as u32);
    }
}

/// The relocated original with a real `call 0xA0AED0` (used when the shadow disagrees).
static DESTROY_REAL: AtomicUsize = AtomicUsize::new(0);

/// The hull release is unbounded (it reaches into the Havok / hull rings), so the shadow runs the original
/// copy with that call replaced by nops against `ours` with the call left out (they must write the same
/// bytes), puts the memory back, and then runs `ours` for real. A disagreement runs the whole original.
extern "C" fn detour_destroy_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    unsafe {
        if let Some((n, orig)) = shadowing(DETOUR_DESTROY) {
            let h = p.ecx;
            let e = if h != 0xFFFF_FFFF { DET_TABLE + (h & 0xFFF) as usize * 40 } else { 0 };
            let reg = detour_regions(e, e != 0 && rd(e + 8) == h);
            let pre = snap(&reg);
            let inp = Regs::of(&p);
            let mut o = inp;
            call_full4(orig, &mut o, [0; 4]);
            check_preserved(DETOUR_DESTROY, &inp, &o);
            let after_orig = snap(&reg);
            restore(&reg, &pre);
            detour_destroy(h, p.esi, false);
            let after_ours = snap(&reg);
            restore(&reg, &pre);
            if after_ours != after_orig {
                let diff = after_ours.iter().zip(&after_orig).position(|(a, b)| a != b);
                mismatch(DETOUR_DESTROY, n, format!("in {inp:x?}: first differing region {diff:?}"));
                let real = DESTROY_REAL.load(Ordering::Relaxed);
                if real != 0 {
                    let mut o2 = inp;
                    call_full4(real, &mut o2, [0; 4]);
                    return;
                }
            }
        }
        detour_destroy(p.ecx, p.esi, true);
    }
}
regs_thunk!(detour_destroy_thunk, detour_destroy_entry);


// --- 0xA17F90 nav_route_record_alloc (no args -> EAX = record or 0) -------------------------------------------

/// Pops the head of the free ring 0x2203AB8 (0 = empty: EAX 0). The record gets +0x24 / +0x28 / +0x2C = -1
/// and flag byte +0x30 = 0x20 (cleared, then 0x20 set); a single-node ring leaves the head 0, otherwise the
/// head takes the next link and the record is unlinked (+0x1C next, +0x20 prev); its links end 0.
unsafe fn route_record_alloc() -> u32 {
    unsafe {
        let e = rd(ROUTE_FREE_HEAD);
        if e == 0 {
            return 0;
        }
        let r = e as usize;
        wd(r + 0x28, 0xFFFF_FFFF);
        wd(r + 0x24, 0xFFFF_FFFF);
        wd(r + 0x2C, 0xFFFF_FFFF);
        wb(r + 0x30, 0);
        let nx = rd(r + 0x1C);
        if nx == e {
            wb(r + 0x30, rb(r + 0x30) | 0x20);
            wd(ROUTE_FREE_HEAD, 0);
            wd(r + 0x20, 0);
            wd(r + 0x1C, 0);
            return e;
        }
        wd(ROUTE_FREE_HEAD, nx);
        let prev = rd(r + 0x20) as usize;
        if prev != 0 {
            wd(prev + 0x1C, rd(r + 0x1C));
        }
        let next = rd(r + 0x1C) as usize;
        if next != 0 {
            wd(next + 0x20, rd(r + 0x20));
        }
        wb(r + 0x30, rb(r + 0x30) | 0x20);
        wd(r + 0x20, 0);
        wd(r + 0x1C, 0);
        e
    }
}

extern "C" fn record_alloc_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = unsafe { shadow_run(RECORD_ALLOC, &p, std::ptr::null_mut(), 0, true, || route_pool_regions(0), || route_record_alloc()) };
}
regs_thunk!(record_alloc_thunk, record_alloc_entry);

// --- 0xB2DB90 vehicle_ai_obstacle_record_alloc (EDI = vehicle -> EAX = record or 0) --------------------------

const OBSTACLE_RING: usize = 0x022B_1458;

/// Pops the head of the obstacle record pool ring (global 0x22B1458, links +0x14 next / +0x18 prev; empty =
/// EAX 0) and appends it at the END of the vehicle's chain (`[vehicle + 0x856C]` = head): a single-record
/// ring empties the global, otherwise the global takes the next and the record is unlinked; the record's
/// links are cleared, an empty chain makes it the head linked to itself, otherwise
/// record.prev = tail, record.next = head, tail.next = head.prev = record.
unsafe fn obstacle_record_alloc(vehicle: u32) -> u32 {
    unsafe {
        let e = rd(OBSTACLE_RING);
        if e == 0 {
            return 0;
        }
        let r = e as usize;
        let next = rd(r + 0x14);
        if next == e {
            wd(OBSTACLE_RING, 0);
        } else {
            let prev = rd(r + 0x18) as usize;
            wd(OBSTACLE_RING, next);
            if prev != 0 {
                wd(prev + 0x14, rd(r + 0x14));
            }
            let nx = rd(r + 0x14) as usize;
            if nx != 0 {
                wd(nx + 0x18, rd(r + 0x18));
            }
        }
        wd(r + 0x18, 0);
        wd(r + 0x14, 0);
        let hp = vehicle as usize + 0x856C;
        let head = rd(hp) as usize;
        if head == 0 {
            wd(r + 0x14, e);
            wd(r + 0x18, e);
            wd(hp, e);
            return e;
        }
        let tail = rd(head + 0x18);
        wd(r + 0x18, tail);
        wd(r + 0x14, rd(hp));
        let h2 = rd(hp) as usize;
        let t2 = rd(h2 + 0x18) as usize;
        wd(t2 + 0x14, e);
        let h3 = rd(hp) as usize;
        wd(h3 + 0x18, e);
        e
    }
}

/// Obstacle records are 0x1C bytes; every record the alloc / release can touch, as whole records.
fn obstacle_regions(extra_head: usize) -> Vec<(usize, usize)> {
    unsafe {
        let mut v = vec![(OBSTACLE_RING, 4)];
        let mut add = |p: usize| {
            if p != 0 {
                v.push((p, 0x1C));
            }
        };
        let h = rd(OBSTACLE_RING) as usize;
        add(h);
        if h != 0 {
            add(rd(h + 0x14) as usize);
            add(rd(h + 0x18) as usize);
        }
        if extra_head != 0 {
            let ch = rd(extra_head) as usize;
            add(ch);
            if ch != 0 {
                add(rd(ch + 0x18) as usize);
                add(rd(ch + 0x14) as usize);
            }
        }
        v
    }
}

extern "C" fn obstacle_alloc_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = unsafe {
        shadow_run(
            OBSTACLE_ALLOC,
            &p,
            std::ptr::null_mut(),
            0,
            true,
            || {
                let mut v = obstacle_regions(p.edi as usize + 0x856C);
                v.push((p.edi as usize + 0x856C, 4));
                Some(v)
            },
            || obstacle_record_alloc(p.edi),
        )
    };
}
regs_thunk!(obstacle_alloc_thunk, obstacle_alloc_entry);

// --- 0xB2DC40 vehicle_ai_obstacle_chain_release (EAX = block, chain head at [block + 0x18]) --------------------

/// Hands the block's obstacle chain back to the pool ring: an empty ring simply becomes the chain (global =
/// head, block slot 0). Otherwise (non-empty chain) the whole chain is spliced in before the ring head:
/// ring tail.next = chain head, chain tail.next = ring head, chain head.prev = ring tail, ring head.prev =
/// chain tail. The block's slot ends 0.
unsafe fn obstacle_chain_release(block: u32) {
    unsafe {
        let b = block as usize;
        let ring = rd(OBSTACLE_RING);
        if ring == 0 {
            wd(OBSTACLE_RING, rd(b + 0x18));
            wd(b + 0x18, 0);
            return;
        }
        let ch = rd(b + 0x18) as usize;
        if ch != 0 {
            let rc = ring as usize;
            let rtail = rd(rc + 0x18) as usize;
            wd(rtail + 0x14, ch as u32);
            let ch2 = rd(b + 0x18) as usize;
            let ctail = rd(ch2 + 0x18) as usize;
            wd(ctail + 0x14, ring);
            let ch3 = rd(b + 0x18) as usize;
            let rtail2 = rd(rc + 0x18);
            let ctail2 = rd(ch3 + 0x18);
            wd(ch3 + 0x18, rtail2);
            wd(rc + 0x18, ctail2);
        }
        wd(b + 0x18, 0);
    }
}

extern "C" fn obstacle_release_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    unsafe {
        shadow_run(
            OBSTACLE_RELEASE,
            &p,
            std::ptr::null_mut(),
            0,
            false,
            || {
                let mut v = obstacle_regions(p.eax as usize + 0x18);
                v.push((p.eax as usize + 0x18, 4));
                Some(v)
            },
            || {
                obstacle_chain_release(p.eax);
                0
            },
        )
    };
}
regs_thunk!(obstacle_release_thunk, obstacle_release_entry);

// --- 0xA052D0 nav_chunk_cell_index (EAX = cell, stack: chunk record, &out -> EAX 0 / 1 / 2) ------------------

const NAV_CHUNK_CELL_INDEX: usize = 0x00A0_52D0;

/// `u16[cell + 0x2C] != u16[rec + 8]` gives 0. Else the two cell arrays in turn (`[rec + 0x240]`, then
/// `[rec + 0x248]`, skipped when null): `q = (cell - [arr + 0x14]) / 48` (hi32(d * 0x2AAAAAAB) sar 3 plus the
/// sign bit); a q that is negative (signed) or >= u16[arr + 0x10] (signed compare) rejects. The first array
/// that accepts stores q at `*out` and returns 1, the second returns 2, none returns 0.
unsafe fn chunk_cell_index(cell: u32, rec: u32, out: u32) -> u32 {
    unsafe {
        let r = rec as usize;
        if rd(cell as usize + 0x2C) as u16 != rd(r + 8) as u16 {
            return 0;
        }
        for (slot, ret) in [(0x240usize, 1u32), (0x248, 2)] {
            let arr = rd(r + slot) as usize;
            if arr == 0 {
                continue;
            }
            let d = cell.wrapping_sub(rd(arr + 0x14)) as i32;
            let hi = ((d as i64 * 0x2AAA_AAAB) >> 32) as i32;
            let q0 = (hi >> 3) as u32;
            let q = q0.wrapping_add(q0 >> 31) as i32;
            if q < 0 {
                continue;
            }
            if q >= (rd(arr + 0x10) & 0xFFFF) as i32 {
                continue;
            }
            wd(out as usize, q as u32);
            return ret;
        }
        0
    }
}

extern "C" fn chunk_cell_index_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (rec, out) = (*args, *args.add(1));
        r.eax = shadow_run(
            CHUNK_CELL_INDEX,
            &p,
            args as *mut u32,
            2,
            true,
            || Some(vec![(out as usize, 4)]),
            || chunk_cell_index(p.eax, rec, out),
        );
    }
}
regs_thunk!(chunk_cell_index_thunk, chunk_cell_index_entry);

// --- 0xA02C10 nav_cell_locate (EAX = cell, EDI = &idx, stack: &chunk, &kind -> AL) -----------------------------

/// `c = u16[cell + 0x2C]`; 0xFFFF, >= 0x866 (unsigned) or a null record give AL 0. Else
/// nav_chunk_cell_index(EAX = cell, rec, &tmp): 0 gives AL 0, 1 stores chunk = u16[rec + 8], kind 0, idx = tmp
/// (EAX = 1), 2 the same with kind 1 (EAX = tmp with AL 1). Returns EAX as the original leaves it (the
/// upper bits are the cell's, or the callee's leftovers).
unsafe fn cell_locate(cell: u32, idx_p: u32, chunk_p: u32, kind_p: u32) -> u32 {
    unsafe {
        let c = rd(cell as usize + 0x2C) & 0xFFFF;
        if c == 0xFFFF || c >= 0x866 {
            return cell & !0xFF;
        }
        let rec = rd(CHUNKS + c as usize * 4);
        if rec == 0 {
            return cell & !0xFF;
        }
        let mut tmp = 0u32;
        let tp = &mut tmp as *mut u32;
        let r = call_regs(NAV_CHUNK_CELL_INDEX, cell, 0, 0, 0, 0, &[rec, tp as u32]);
        let t = std::ptr::read_volatile(tp);
        match r {
            0 => 0,
            1 => {
                wd(chunk_p as usize, rd(rec as usize + 8) & 0xFFFF);
                wd(kind_p as usize, 0);
                wd(idx_p as usize, t);
                1
            }
            2 => {
                wd(chunk_p as usize, rd(rec as usize + 8) & 0xFFFF);
                wd(kind_p as usize, 1);
                wd(idx_p as usize, t);
                (t & !0xFF) | 1
            }
            _ => r.wrapping_sub(2) & !0xFF,
        }
    }
}

extern "C" fn cell_locate_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (chunk_p, kind_p) = (*args, *args.add(1));
        r.eax = shadow_run(
            CELL_LOCATE,
            &p,
            args as *mut u32,
            2,
            true,
            || Some(vec![(chunk_p as usize, 4), (kind_p as usize, 4), (p.edi as usize, 4)]),
            || cell_locate(p.eax, p.edi, chunk_p, kind_p),
        );
    }
}
regs_thunk!(cell_locate_thunk, cell_locate_entry);


// --- 0xC03910 spline_eval_position (EAX = spline, ESI = &out vec3, stack f32 t, `ret 4`) -----------------------

/// Like `regs_thunk!` but the original pops its one stack argument (`ret 4`).
macro_rules! regs_thunk_ret4 {
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
                "ret 4",
                f = sym $f,
            )
        }
    };
}

const SPLINE_DEFAULT: usize = 0x0252_A6D0;
const SPLINE_C1: usize = 0x00E5_BFF8;
const SPLINE_C2: usize = 0x00E5_BF40;

unsafe fn rd_f64(p: usize) -> f64 {
    unsafe { f64::from_bits(rd(p) as u64 | (rd(p + 4) as u64) << 32) }
}

/// `cvttss2si`: truncation, NaN and out-of-range values give 0x80000000.
fn cvtt(t: f32) -> i32 {
    if t.is_nan() || t >= 2147483648.0 || t < -2147483648.0 { i32::MIN } else { t as i32 }
}

unsafe fn copy_vec3(dst: usize, src: usize) {
    unsafe {
        for k in 0..3 {
            wd(dst + 4 * k, rd(src + 4 * k));
        }
    }
}

/// Spline = {points vec3*, tangent-A vec3* (+4), tangent-B vec3* (+8), int count (+0xC)}. Count < 2 (signed)
/// copies the default vec3 at 0x252A6D0. Otherwise i = cvttss2si(t): negative (including NaN / huge) copies
/// point 0, i >= count - 1 (signed) copies the last point. Interior: u = f32(t - i), F2 = f32(u*u),
/// F3 = f32(F2*u), H0 = f32((F2 - u)*3 - F3 + 1), N = f32((u + F3 - 2*F2)*3), P = f32(3*(F2 - F3)); per axis
/// (pts = point i / i+1, tb = tangent-B[i], ta = tangent-A[i+1]):
///   y: Q1 = f32(pi*H0), Q2 = f32(Q1 + pj*F3), Q3 = f32(tb*N + Q2), out = f32(ta*P + Q3)
///   z: the same chain; x: X1 = f32(H0*pi), X2 = f32(F3*pj + X1), X3 = f32(N*tb + X2), out = f32(ta*P + X3).
/// Every intermediate that the original stores to the stack rounds to f32, the rest stays f64 (53-bit
/// precision). The dead stack-argument slot ends up holding X3.
unsafe fn spline_eval_position(spline: u32, out: u32, args: *mut u32) {
    unsafe {
        let (s, o) = (spline as usize, out as usize);
        let count = rd(s + 0xC) as i32;
        if count < 2 {
            copy_vec3(o, SPLINE_DEFAULT);
            return;
        }
        let t = rf(args as usize);
        let i = cvtt(t);
        let pts = rd(s) as usize;
        if i < 0 {
            copy_vec3(o, pts);
            return;
        }
        if i >= count - 1 {
            copy_vec3(o, pts.wrapping_add((count as usize).wrapping_mul(12)).wrapping_sub(12));
            return;
        }
        let (c1, c2) = (rd_f64(SPLINE_C1), rd_f64(SPLINE_C2));
        let iu = i as usize;
        let pi = pts + iu * 12;
        let pj = pts + (iu + 1) * 12;
        let tb = rd(s + 8) as usize + iu * 12;
        let ta = rd(s + 4) as usize + (iu + 1) * 12;
        let u = ((t as f64) - (i as f64)) as f32;
        let ud = u as f64;
        let f2 = (ud * ud) as f32;
        let f2d = f2 as f64;
        let f3 = (f2d * ud) as f32;
        let f3d = f3 as f64;
        let h0d = ((f2d - ud) * c1 - f3d + c2) as f32 as f64;
        let nd = ((ud + (f3d - f2d - f2d)) * c1) as f32 as f64;
        let pd = (c1 * (f2d - f3d)) as f32 as f64;
        // y
        let q1 = (rf(pi + 4) as f64 * h0d) as f32;
        let q2 = (q1 as f64 + rf(pj + 4) as f64 * f3d) as f32;
        let q3 = (rf(tb + 4) as f64 * nd + q2 as f64) as f32;
        let y = (rf(ta + 4) as f64 * pd + q3 as f64) as f32;
        // z
        let z1 = (rf(pi + 8) as f64 * h0d) as f32;
        let z2 = (z1 as f64 + rf(pj + 8) as f64 * f3d) as f32;
        let z3 = (z2 as f64 + rf(tb + 8) as f64 * nd) as f32;
        let z = (z3 as f64 + rf(ta + 8) as f64 * pd) as f32;
        // x
        let x1 = (h0d * rf(pi) as f64) as f32;
        let x2 = (rf(pj) as f64 * f3d + x1 as f64) as f32;
        let x3 = (nd * rf(tb) as f64 + x2 as f64) as f32;
        let x = (rf(ta) as f64 * pd + x3 as f64) as f32;
        wf(o + 4, y);
        wf(o + 8, z);
        *args = x3.to_bits();
        wf(o, x);
    }
}

extern "C" fn spline_eval_position_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        shadow_run(
            SPLINE_EVAL_POS,
            &p,
            args as *mut u32,
            1,
            false,
            || Some(vec![(p.esi as usize, 12)]),
            || {
                spline_eval_position(p.eax, p.esi, args as *mut u32);
                0
            },
        )
    };
}
regs_thunk_ret4!(spline_eval_position_thunk, spline_eval_position_entry);

// --- install ---------------------------------------------------------------------------------------

/// `(port, offset of an E8 / E9 rel32 in its body, port whose relocated original that branch must reach, the
/// branch's original absolute target)`:
/// the shadow copy of a caller then runs the callee's ORIGINAL code instead of our port. The callee's
/// port must come earlier in the install list.
const CHAINS: [(usize, usize, usize, usize); 6] = [
    (CELL_LOCATE, 0x31, CHUNK_CELL_INDEX, 0xA0_52D0),
    (PATH_FREE_ALL, 0x15, PATH_RECYCLE, 0xA1_5B20),
    (LIST_FREE, 0x51, RECORD_FREE, 0xA1_8460),
    (CLAIM_ADD, 0x19, CLAIM_ALLOC, 0xA8_8D10),
    (CLAIM_REMOVE, 0x6A, CLAIM_FREE, 0xA8_8DB0),
    (ROUTE_PUSH, 0x58, LIST_FREE, 0xA1_8500),
];

pub fn install() {
    let list: Vec<(usize, &[u8], &[usize], extern "C" fn())> = vec![
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
        (HAS_LINK_FLAG2, &HAS_LINK_FLAG2_BODY, &[], has_link_flag2_thunk),
        (CELL_WALKABLE, &CELL_WALKABLE_BODY, &[0x10], cell_walkable_thunk),
        (CELL_TO_NODE_REF, &CELL_TO_NODE_REF_BODY, &[0x3C], cell_to_node_ref_thunk),
        (SET_SPEED_OVERRIDE, &SET_SPEED_BODY, &[0x37], set_speed_thunk),
        (CLEAR_SPEED_OVERRIDE, &CLEAR_SPEED_BODY, &[0x11], clear_speed_thunk),
        (ROUTE_CLEAR, &ROUTE_CLEAR_BODY, &[0x19], route_clear_thunk),
        (PAIR_MARK, &PAIR_MARK_BODY, &[], pair_mark_thunk),
        (PATH_RECYCLE, &PATH_RECYCLE_BODY, &[], path_recycle_thunk),
        (ROAD_RECYCLE, &ROAD_RECYCLE_BODY, &[], road_recycle_thunk),
        (RECORD_FREE, &RECORD_FREE_BODY, &[], record_free_thunk),
        (PATH_FREE_ALL, &PATH_FREE_ALL_BODY, &[0x15], path_free_all_thunk),
        (LIST_FREE, &LIST_FREE_BODY, &[0x51], list_free_thunk),
        (CLAIM_ALLOC, &CLAIM_ALLOC_BODY, &[], claim_alloc_thunk),
        (CLAIM_FREE, &CLAIM_FREE_BODY, &[], claim_free_thunk),
        (CLAIM_ADD, &CLAIM_ADD_BODY, &[0x19], claim_add_thunk),
        (CLAIM_REMOVE, &CLAIM_REMOVE_BODY, &[], claim_remove_thunk),
        (ROUTE_PUSH, &ROUTE_PUSH_BODY, &[0x58, 0x64], route_push_thunk),
        (DETOUR_CREATE, &DETOUR_CREATE_BODY, &[], detour_create_thunk),
        (DETOUR_DESTROY, &DETOUR_DESTROY_BODY, &[0x34], detour_destroy_thunk),
        (RECORD_ALLOC, &RECORD_ALLOC_BODY, &[], record_alloc_thunk),
        (OBSTACLE_ALLOC, &OBSTACLE_ALLOC_BODY, &[], obstacle_alloc_thunk),
        (OBSTACLE_RELEASE, &OBSTACLE_RELEASE_BODY, &[], obstacle_release_thunk),
        (CHUNK_CELL_INDEX, &CHUNK_CELL_INDEX_BODY, &[], chunk_cell_index_thunk),
        (CELL_LOCATE, &CELL_LOCATE_BODY, &[0x31], cell_locate_thunk),
        (SPLINE_EVAL_POS, &SPLINE_EVAL_POS_BODY, &[], spline_eval_position_thunk),
    ];
    let mut ok = 0;
    for (id, body, call_offsets, thunk) in list {
        let port = &PORTS[id];
        match unsafe { detour::relocate(port.addr, body, call_offsets) } {
            // nav_route_clear has no shadow. The two speed-override shadows stop before their call:
            // `add esp, 4` (undoing the push) replaces `call 0xB2B8A0`.
            Ok(_) if id == ROUTE_CLEAR => {}
            Ok(copy) => {
                if id == SET_SPEED_OVERRIDE || id == CLEAR_SPEED_OVERRIDE {
                    let o = if id == SET_SPEED_OVERRIDE { 0x37 } else { 0x11 };
                    unsafe { std::ptr::copy_nonoverlapping([0x83u8, 0xC4, 0x04, 0x90, 0x90].as_ptr(), (copy + o) as *mut u8, 5) };
                }
                for &(_, off, callee, target) in CHAINS.iter().filter(|c| c.0 == id) {
                    let mut dest = PORTS[callee].original.load(Ordering::Relaxed);
                    if dest == 0 {
                        dest = target;
                    }
                    let rel = (dest as u32).wrapping_sub((copy + off + 5) as u32);
                    unsafe { std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (copy + off + 1) as *mut u8, 4) };
                }
                if id == DETOUR_DESTROY {
                    // keep a plain copy for a disagreement; the shadow copy gets the hull release call nopped
                    DESTROY_REAL.store(unsafe { detour::relocate(port.addr, body, call_offsets) }.unwrap_or(0), Ordering::Relaxed);
                    unsafe { std::ptr::copy_nonoverlapping([0x90u8; 5].as_ptr(), (copy + 0x34) as *mut u8, 5) };
                }
                port.original.store(copy, Ordering::Relaxed)
            }
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
const HAS_LINK_FLAG2_BODY: [u8; 97] = [
    0x56, 0x57, 0x0F, 0xB6, 0x78, 0x1C, 0x33, 0xD2, 0x85, 0xFF, 0x7E, 0x4B, 0x8B, 0x70, 0x18, 0x90,
    0x8B, 0x06, 0x85, 0xC0, 0x74, 0x37, 0x8B, 0xC8, 0x0F, 0xB7, 0xC0, 0xC1, 0xE9, 0x10, 0x83, 0xE8,
    0x01, 0x81, 0xF9, 0xFF, 0xFF, 0x00, 0x00, 0x74, 0x24, 0x81, 0xF9, 0x66, 0x08, 0x00, 0x00, 0x73,
    0x1C, 0x8B, 0x0C, 0x8D, 0xE8, 0xAA, 0xF7, 0x00, 0x85, 0xC9, 0x74, 0x11, 0xC1, 0xE0, 0x05, 0x03,
    0x81, 0x50, 0x01, 0x00, 0x00, 0x74, 0x06, 0xF6, 0x40, 0x18, 0x02, 0x75, 0x0F, 0x83, 0xC2, 0x01,
    0x83, 0xC6, 0x04, 0x3B, 0xD7, 0x7C, 0xB9, 0x5F, 0x32, 0xC0, 0x5E, 0xC3, 0x5F, 0xB0, 0x01, 0x5E,
    0xC3,
];
const CELL_WALKABLE_BODY: [u8; 28] = [
    0x8A, 0x48, 0x1D, 0xF6, 0xC1, 0x10, 0x74, 0x03, 0x32, 0xC0, 0xC3, 0xF6, 0xC1, 0x02, 0x74, 0x09,
    0xE8, 0x5B, 0x0A, 0x00, 0x00, 0x84, 0xC0, 0x74, 0xEF, 0xB0, 0x01, 0xC3,
];
const CELL_TO_NODE_REF_BODY: [u8; 106] = [
    0x83, 0xEC, 0x0C, 0x85, 0xC0, 0x75, 0x07, 0x83, 0xC8, 0xFF, 0x83, 0xC4, 0x0C, 0xC3, 0x66, 0x81,
    0x78, 0x2C, 0xFF, 0xFF, 0xC7, 0x04, 0x24, 0xFF, 0x0F, 0x00, 0x00, 0xC7, 0x44, 0x24, 0x04, 0x01,
    0x00, 0x00, 0x00, 0xC7, 0x44, 0x24, 0x08, 0xFF, 0xFF, 0x07, 0x00, 0x74, 0xDA, 0x57, 0x8D, 0x4C,
    0x24, 0x08, 0x51, 0x8D, 0x54, 0x24, 0x08, 0x52, 0x8D, 0x7C, 0x24, 0x14, 0xE8, 0x7F, 0xD9, 0xFF,
    0xFF, 0x83, 0xC4, 0x08, 0x84, 0xC0, 0x5F, 0x74, 0xBE, 0x8B, 0x4C, 0x24, 0x04, 0x8B, 0x44, 0x24,
    0x08, 0x8B, 0x14, 0x24, 0x83, 0xE1, 0x01, 0xC1, 0xE1, 0x13, 0x25, 0xFF, 0xFF, 0x07, 0x00, 0xC1,
    0xE2, 0x14, 0x0B, 0xC1, 0x0B, 0xC2, 0x83, 0xC4, 0x0C, 0xC3,
];
const SET_SPEED_BODY: [u8; 63] = [
    0x51, 0xD9, 0x44, 0x24, 0x08, 0x57, 0xDC, 0x1D, 0xD8, 0xC0, 0xE5, 0x00, 0x8B, 0xF8, 0xDF, 0xE0,
    0xF6, 0xC4, 0x05, 0x7B, 0x0A, 0xD9, 0x05, 0x30, 0x80, 0xE9, 0x00, 0xD9, 0x5C, 0x24, 0x0C, 0xD9,
    0x44, 0x24, 0x0C, 0xC0, 0xE1, 0x04, 0x32, 0x4F, 0x16, 0xD9, 0x9F, 0x88, 0x00, 0x00, 0x00, 0x6A,
    0x01, 0x80, 0xE1, 0x10, 0x30, 0x4F, 0x16, 0xE8, 0x74, 0x43, 0x00, 0x00, 0x5F, 0x59, 0xC3,
];
const CLEAR_SPEED_BODY: [u8; 24] = [
    0xD9, 0xEE, 0x57, 0x8B, 0xF8, 0xD9, 0x9F, 0x88, 0x00, 0x00, 0x00, 0x80, 0x67, 0x16, 0xEF, 0x6A,
    0x00, 0xE8, 0x5A, 0x43, 0x00, 0x00, 0x5F, 0xC3,
];
const ROUTE_CLEAR_BODY: [u8; 59] = [
    0x85, 0xF6, 0x74, 0x36, 0x57, 0x33, 0xFF, 0x80, 0x7E, 0x3C, 0x00, 0x76, 0x20, 0x55, 0x8D, 0x6E,
    0x04, 0xF6, 0x45, 0x04, 0x04, 0x74, 0x07, 0x8B, 0xC5, 0xE8, 0xB2, 0x8D, 0xFF, 0xFF, 0x0F, 0xB6,
    0x46, 0x3C, 0x83, 0xC7, 0x01, 0x83, 0xC5, 0x0C, 0x3B, 0xF8, 0x7C, 0xE5, 0x5D, 0xC6, 0x46, 0x3C,
    0x00, 0xC6, 0x46, 0x24, 0x00, 0xC6, 0x46, 0x30, 0x00, 0x5F, 0xC3,
];

const PAIR_MARK_BODY: [u8; 23] = [
    0x8A, 0x42, 0x1D, 0xA8, 0x20, 0x75, 0x0F, 0xF6, 0x41, 0x1D, 0x20, 0x75, 0x09, 0x0C, 0x20, 0x88,
    0x42, 0x1D, 0x80, 0x49, 0x1D, 0x20, 0xC3,
];
const PATH_RECYCLE_BODY: [u8; 107] = [
    0x39, 0x02, 0x75, 0x1D, 0x8B, 0x08, 0x3B, 0xC8, 0x75, 0x15, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x02, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x26, 0x89,
    0x0A, 0x8B, 0x48, 0x04, 0x85, 0xC9, 0x74, 0x04, 0x8B, 0x10, 0x89, 0x11, 0x8B, 0x08, 0x85, 0xC9,
    0x74, 0x06, 0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x40,
    0x04, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x0D, 0x9C, 0xC1, 0x1F, 0x02, 0x85, 0xC9, 0x75, 0x0B, 0x89,
    0x00, 0x89, 0x40, 0x04, 0xA3, 0x9C, 0xC1, 0x1F, 0x02, 0xC3, 0x8B, 0x51, 0x04, 0x89, 0x50, 0x04,
    0x89, 0x08, 0x8B, 0x51, 0x04, 0x89, 0x02, 0x89, 0x41, 0x04, 0xC3,
];
const ROAD_RECYCLE_BODY: [u8; 107] = [
    0x39, 0x02, 0x75, 0x1D, 0x8B, 0x08, 0x3B, 0xC8, 0x75, 0x15, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x02, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x26, 0x89,
    0x0A, 0x8B, 0x48, 0x04, 0x85, 0xC9, 0x74, 0x04, 0x8B, 0x10, 0x89, 0x11, 0x8B, 0x08, 0x85, 0xC9,
    0x74, 0x06, 0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x40,
    0x04, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x0D, 0x7C, 0x9C, 0x2A, 0x02, 0x85, 0xC9, 0x75, 0x0B, 0x89,
    0x00, 0x89, 0x40, 0x04, 0xA3, 0x7C, 0x9C, 0x2A, 0x02, 0xC3, 0x8B, 0x51, 0x04, 0x89, 0x50, 0x04,
    0x89, 0x08, 0x8B, 0x51, 0x04, 0x89, 0x02, 0x89, 0x41, 0x04, 0xC3,
];
const PATH_FREE_ALL_BODY: [u8; 32] = [
    0x8B, 0x07, 0x56, 0x85, 0xC0, 0x74, 0x17, 0x8B, 0x08, 0x8B, 0xF1, 0x2B, 0x37, 0x8B, 0xD7, 0xF7,
    0xDE, 0x1B, 0xF6, 0x23, 0xF1, 0xE8, 0x06, 0x00, 0x00, 0x00, 0x8B, 0xC6, 0xEB, 0xE5, 0x5E, 0xC3,
];
const RECORD_FREE_BODY: [u8; 160] = [
    0x56, 0x8B, 0xF1, 0x81, 0xEE, 0xD0, 0x00, 0xE4, 0x02, 0x0F, 0x88, 0x8F, 0x00, 0x00, 0x00, 0x81,
    0xFE, 0x00, 0x68, 0x00, 0x00, 0x0F, 0x83, 0x83, 0x00, 0x00, 0x00, 0xB8, 0x4F, 0xEC, 0xC4, 0x4E,
    0xF7, 0xE6, 0xC1, 0xEA, 0x04, 0x6B, 0xD2, 0x34, 0x2B, 0xF2, 0x75, 0x72, 0xF6, 0x41, 0x30, 0x20,
    0x74, 0x6C, 0x39, 0x0F, 0x75, 0x15, 0x8B, 0x41, 0x1C, 0x3B, 0xC1, 0x75, 0x0C, 0x33, 0xD2, 0x89,
    0x51, 0x1C, 0x89, 0x51, 0x20, 0x89, 0x17, 0xEB, 0x24, 0x89, 0x07, 0x8B, 0x41, 0x20, 0x33, 0xD2,
    0x3B, 0xC2, 0x74, 0x06, 0x8B, 0x71, 0x1C, 0x89, 0x70, 0x1C, 0x8B, 0x41, 0x1C, 0x3B, 0xC2, 0x74,
    0x06, 0x8B, 0x71, 0x20, 0x89, 0x70, 0x20, 0x89, 0x51, 0x1C, 0x89, 0x51, 0x20, 0xA1, 0xB8, 0x3A,
    0x20, 0x02, 0x3B, 0xC2, 0x75, 0x12, 0x80, 0x61, 0x30, 0xDF, 0x89, 0x49, 0x1C, 0x89, 0x49, 0x20,
    0x89, 0x0D, 0xB8, 0x3A, 0x20, 0x02, 0x5E, 0xC3, 0x8B, 0x50, 0x20, 0x89, 0x51, 0x20, 0x89, 0x41,
    0x1C, 0x8B, 0x50, 0x20, 0x89, 0x4A, 0x1C, 0x89, 0x48, 0x20, 0x80, 0x61, 0x30, 0xDF, 0x5E, 0xC3,
];
const LIST_FREE_BODY: [u8; 90] = [
    0x57, 0x8B, 0xF8, 0x85, 0xFF, 0x74, 0x37, 0x8B, 0x0F, 0x85, 0xC9, 0x74, 0x31, 0x56, 0x8B, 0xF1,
    0x81, 0xEE, 0xD0, 0x00, 0xE4, 0x02, 0x78, 0x1F, 0x81, 0xFE, 0x00, 0x68, 0x00, 0x00, 0x73, 0x17,
    0xB8, 0x4F, 0xEC, 0xC4, 0x4E, 0xF7, 0xE6, 0xC1, 0xEA, 0x04, 0x6B, 0xD2, 0x34, 0x2B, 0xF2, 0x75,
    0x06, 0xF6, 0x41, 0x30, 0x20, 0x75, 0x09, 0xC7, 0x07, 0x00, 0x00, 0x00, 0x00, 0x5E, 0x5F, 0xC3,
    0x85, 0xC9, 0x74, 0xF9, 0x8B, 0x41, 0x1C, 0x8B, 0xF0, 0x2B, 0x37, 0xF7, 0xDE, 0x1B, 0xF6, 0x23,
    0xF0, 0xE8, 0x0A, 0xFF, 0xFF, 0xFF, 0x8B, 0xCE, 0xEB, 0xE6,
];

const CLAIM_ALLOC_BODY: [u8; 155] = [
    0x56, 0x66, 0x8B, 0x35, 0x4C, 0x7B, 0xE8, 0x00, 0x66, 0x81, 0xFE, 0xFF, 0xFF, 0x75, 0x04, 0x33,
    0xC0, 0x5E, 0xC3, 0x0F, 0xB7, 0xC6, 0x8D, 0x0C, 0x40, 0x8D, 0x0C, 0x8D, 0x10, 0xE4, 0x23, 0x02,
    0x85, 0xC9, 0x74, 0x73, 0x8B, 0xD1, 0x81, 0xEA, 0x10, 0xE4, 0x23, 0x02, 0xB8, 0xAB, 0xAA, 0xAA,
    0x2A, 0xF7, 0xEA, 0xD1, 0xFA, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x66, 0x3B, 0xF0, 0x75,
    0x27, 0x0F, 0xB7, 0x11, 0x66, 0x3B, 0xD0, 0x75, 0x18, 0x66, 0xC7, 0x05, 0x4C, 0x7B, 0xE8, 0x00,
    0xFF, 0xFF, 0x66, 0xC7, 0x41, 0x02, 0xFF, 0xFF, 0x66, 0xC7, 0x01, 0xFF, 0xFF, 0x8B, 0xC1, 0x5E,
    0xC3, 0x66, 0x89, 0x15, 0x4C, 0x7B, 0xE8, 0x00, 0x0F, 0xB7, 0x41, 0x02, 0x0F, 0xB7, 0x11, 0x8D,
    0x04, 0x40, 0x66, 0x89, 0x14, 0x85, 0x10, 0xE4, 0x23, 0x02, 0x0F, 0xB7, 0x01, 0x0F, 0xB7, 0x51,
    0x02, 0x8D, 0x04, 0x40, 0x66, 0x89, 0x14, 0x85, 0x12, 0xE4, 0x23, 0x02, 0x66, 0xC7, 0x41, 0x02,
    0xFF, 0xFF, 0x66, 0xC7, 0x01, 0xFF, 0xFF, 0x8B, 0xC1, 0x5E, 0xC3,
];
const CLAIM_FREE_BODY: [u8; 196] = [
    0x56, 0x8B, 0xF0, 0x8B, 0xD1, 0x81, 0xEA, 0x10, 0xE4, 0x23, 0x02, 0xB8, 0xAB, 0xAA, 0xAA, 0x2A,
    0xF7, 0xEA, 0xD1, 0xFA, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0xC7, 0x41, 0x04, 0xFF, 0xFF,
    0xFF, 0xFF, 0x66, 0x39, 0x06, 0x75, 0x1D, 0x0F, 0xB7, 0x11, 0x66, 0x3B, 0xD0, 0x75, 0x12, 0x66,
    0xC7, 0x01, 0xFF, 0xFF, 0x66, 0xC7, 0x41, 0x02, 0xFF, 0xFF, 0x66, 0xC7, 0x06, 0xFF, 0xFF, 0xEB,
    0x32, 0x66, 0x89, 0x16, 0x0F, 0xB7, 0x51, 0x02, 0x0F, 0xB7, 0x31, 0x8D, 0x14, 0x52, 0x66, 0x89,
    0x34, 0x95, 0x10, 0xE4, 0x23, 0x02, 0x0F, 0xB7, 0x11, 0x0F, 0xB7, 0x71, 0x02, 0x8D, 0x14, 0x52,
    0x66, 0x89, 0x34, 0x95, 0x12, 0xE4, 0x23, 0x02, 0x66, 0xC7, 0x01, 0xFF, 0xFF, 0x66, 0xC7, 0x41,
    0x02, 0xFF, 0xFF, 0x66, 0x8B, 0x35, 0x4C, 0x7B, 0xE8, 0x00, 0x66, 0x81, 0xFE, 0xFF, 0xFF, 0x75,
    0x0F, 0x66, 0x89, 0x01, 0x66, 0x89, 0x41, 0x02, 0x66, 0xA3, 0x4C, 0x7B, 0xE8, 0x00, 0x5E, 0xC3,
    0x0F, 0xB7, 0xD6, 0x8D, 0x14, 0x52, 0x57, 0x66, 0x8B, 0x3C, 0x95, 0x12, 0xE4, 0x23, 0x02, 0x66,
    0x89, 0x79, 0x02, 0x66, 0x89, 0x31, 0x0F, 0xB7, 0x0C, 0x95, 0x12, 0xE4, 0x23, 0x02, 0x8D, 0x0C,
    0x49, 0x5F, 0x66, 0x89, 0x04, 0x8D, 0x10, 0xE4, 0x23, 0x02, 0x66, 0x89, 0x04, 0x95, 0x12, 0xE4,
    0x23, 0x02, 0x5E, 0xC3,
];
const CLAIM_ADD_BODY: [u8; 191] = [
    0x51, 0x85, 0xF6, 0x0F, 0x84, 0xB4, 0x00, 0x00, 0x00, 0x8B, 0x46, 0x10, 0xC1, 0xE8, 0x13, 0xF6,
    0xD0, 0x24, 0x01, 0x0F, 0x84, 0xA4, 0x00, 0x00, 0x00, 0xE8, 0x02, 0xD6, 0xFF, 0xFF, 0x8B, 0xC8,
    0x85, 0xC9, 0x0F, 0x84, 0x95, 0x00, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x08, 0x89, 0x51, 0x04, 0xC7,
    0x41, 0x08, 0x00, 0x00, 0x00, 0x00, 0x0F, 0xB7, 0x46, 0x2A, 0x66, 0x3D, 0xFF, 0xFF, 0x75, 0x25,
    0x8B, 0xD1, 0x81, 0xEA, 0x10, 0xE4, 0x23, 0x02, 0xB8, 0xAB, 0xAA, 0xAA, 0x2A, 0xF7, 0xEA, 0xD1,
    0xFA, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x66, 0x89, 0x01, 0x66, 0x89, 0x41, 0x02, 0x66,
    0x89, 0x46, 0x2A, 0x59, 0xC3, 0x0F, 0xB7, 0xC0, 0x8D, 0x04, 0x40, 0x66, 0x8B, 0x14, 0x85, 0x12,
    0xE4, 0x23, 0x02, 0x66, 0x89, 0x51, 0x02, 0x66, 0x8B, 0x46, 0x2A, 0x66, 0x89, 0x01, 0x81, 0xE9,
    0x10, 0xE4, 0x23, 0x02, 0xB8, 0xAB, 0xAA, 0xAA, 0x2A, 0xF7, 0xE9, 0x0F, 0xB7, 0x4E, 0x2A, 0xD1,
    0xFA, 0x8D, 0x0C, 0x49, 0x0F, 0xB7, 0x0C, 0x8D, 0x12, 0xE4, 0x23, 0x02, 0x8B, 0xC2, 0xC1, 0xE8,
    0x1F, 0x03, 0xC2, 0x8D, 0x14, 0x49, 0x66, 0x89, 0x04, 0x95, 0x10, 0xE4, 0x23, 0x02, 0x0F, 0xB7,
    0x4E, 0x2A, 0x8D, 0x0C, 0x49, 0x66, 0x89, 0x04, 0x8D, 0x12, 0xE4, 0x23, 0x02, 0x59, 0xC3,
];
const CLAIM_REMOVE_BODY: [u8; 114] = [
    0x51, 0x85, 0xC0, 0x74, 0x6B, 0x8B, 0x48, 0x10, 0xC1, 0xE9, 0x13, 0xF6, 0xD1, 0x80, 0xE1, 0x01,
    0x74, 0x5E, 0x83, 0x3D, 0x64, 0x5B, 0x22, 0x02, 0x00, 0x74, 0x55, 0x83, 0xC0, 0x2A, 0x56, 0x0F,
    0xB7, 0x30, 0x66, 0x81, 0xFE, 0xFF, 0xFF, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x13, 0x0F, 0xB7, 0xCE,
    0x8D, 0x0C, 0x49, 0x8D, 0x0C, 0x8D, 0x10, 0xE4, 0x23, 0x02, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x85, 0xC9, 0x74, 0x2B, 0x0F, 0xB7, 0x11, 0x66, 0x3B, 0xD6, 0x75, 0x04, 0x33, 0xD2, 0xEB, 0x0D,
    0x0F, 0xB7, 0xD2, 0x8D, 0x14, 0x52, 0x8D, 0x14, 0x95, 0x10, 0xE4, 0x23, 0x02, 0x39, 0x79, 0x04,
    0x74, 0x04, 0x8B, 0xCA, 0xEB, 0xDA, 0x5E, 0x83, 0xC4, 0x04, 0xE9, 0x91, 0xD5, 0xFF, 0xFF, 0x5E,
    0x59, 0xC3,
];
const ROUTE_PUSH_BODY: [u8; 264] = [
    0x53, 0x8A, 0x5C, 0x24, 0x0C, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x85, 0xED, 0x0F, 0x84, 0xF3, 0x00,
    0x00, 0x00, 0x8A, 0x56, 0x3C, 0x80, 0xFA, 0x03, 0x57, 0x73, 0x34, 0x0F, 0xB6, 0xFA, 0x33, 0xC0,
    0x85, 0xFF, 0x7E, 0x1D, 0x8D, 0x4E, 0x04, 0x3B, 0x29, 0x0F, 0x84, 0xD5, 0x00, 0x00, 0x00, 0x0F,
    0xB6, 0x5E, 0x3C, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x0C, 0x3B, 0xC3, 0x7C, 0xEA, 0x8A, 0x5C, 0x24,
    0x14, 0x8D, 0x04, 0x7F, 0x80, 0xC2, 0x01, 0x8D, 0x0C, 0x86, 0x88, 0x56, 0x3C, 0xEB, 0x28, 0xF6,
    0x46, 0x08, 0x04, 0x74, 0x08, 0x8D, 0x46, 0x04, 0xE8, 0x33, 0x8D, 0xFF, 0xFF, 0x6A, 0x18, 0x8D,
    0x4E, 0x0C, 0x51, 0x56, 0xE8, 0x87, 0x68, 0x27, 0x00, 0xB0, 0xFF, 0x00, 0x46, 0x30, 0x83, 0xC4,
    0x0C, 0x00, 0x46, 0x24, 0x8D, 0x4E, 0x18, 0x8A, 0x54, 0x24, 0x18, 0x8A, 0xC2, 0x24, 0x01, 0x02,
    0xC0, 0x80, 0xE3, 0x01, 0x0A, 0xC3, 0x8A, 0x59, 0x08, 0x02, 0xC0, 0x80, 0xE3, 0xE0, 0x0A, 0xC3,
    0x84, 0xD2, 0x88, 0x41, 0x08, 0x74, 0x13, 0x8A, 0x55, 0x30, 0x02, 0xD2, 0x02, 0xD2, 0x32, 0xD0,
    0x80, 0xE2, 0x20, 0x32, 0xD0, 0x88, 0x51, 0x08, 0xEB, 0x05, 0x24, 0xDF, 0x88, 0x41, 0x08, 0x89,
    0x69, 0x04, 0x8B, 0x45, 0x10, 0xC1, 0xE8, 0x13, 0xF6, 0xD0, 0x24, 0x01, 0x75, 0x14, 0xD9, 0x05,
    0x30, 0x80, 0xE9, 0x00, 0x5F, 0xD9, 0x5C, 0x24, 0x0C, 0x5D, 0xD9, 0x44, 0x24, 0x08, 0x5B, 0xD9,
    0x19, 0xC3, 0x8A, 0x45, 0x28, 0xC0, 0xE8, 0x04, 0x3C, 0x03, 0x72, 0x14, 0xD9, 0x05, 0x50, 0x05,
    0xDD, 0x00, 0x5F, 0xD9, 0x5C, 0x24, 0x0C, 0x5D, 0xD9, 0x44, 0x24, 0x08, 0x5B, 0xD9, 0x19, 0xC3,
    0x0F, 0xB6, 0xD0, 0xD9, 0x04, 0x95, 0xE8, 0xE3, 0x23, 0x02, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x44,
    0x24, 0x10, 0xD9, 0x19, 0x5F, 0x5D, 0x5B, 0xC3,
];
const DETOUR_CREATE_BODY: [u8; 319] = [
    0x57, 0x8B, 0xCA, 0x33, 0xFF, 0x2B, 0xCF, 0x74, 0x56, 0x83, 0xE9, 0x01, 0x74, 0x2F, 0x83, 0xE9,
    0x01, 0x75, 0x6A, 0x3B, 0xC7, 0x74, 0x21, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x01, 0x00, 0x00, 0x81,
    0xF9, 0xFF, 0x01, 0x00, 0x00, 0x77, 0x11, 0x69, 0xC9, 0x08, 0x02, 0x00, 0x00, 0x81, 0xC1, 0xE0,
    0x4D, 0x67, 0x02, 0x39, 0x41, 0x10, 0x74, 0x4A, 0x83, 0xC8, 0xFF, 0x5F, 0xC3, 0x3B, 0xC7, 0x74,
    0x19, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0xFF, 0x00, 0x00, 0x69, 0xC9, 0x34, 0x01, 0x00, 0x00, 0x81,
    0xC1, 0xE0, 0x50, 0xAE, 0x02, 0x39, 0x41, 0x08, 0x74, 0x28, 0x83, 0xC8, 0xFF, 0x5F, 0xC3, 0x3B,
    0xC7, 0x74, 0x1A, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x89,
    0x64, 0x9C, 0x14, 0x02, 0x3B, 0xCF, 0x74, 0x05, 0x39, 0x41, 0x44, 0x74, 0x05, 0x83, 0xC8, 0xFF,
    0x5F, 0xC3, 0x8B, 0x0D, 0x04, 0x2C, 0x1F, 0x02, 0x3B, 0xCF, 0x74, 0xF1, 0x89, 0x51, 0x10, 0x66,
    0x8B, 0x54, 0x24, 0x08, 0x89, 0x41, 0x0C, 0x66, 0x89, 0x51, 0x14, 0x8B, 0xD1, 0x81, 0xEA, 0xB8,
    0xE0, 0xDE, 0x02, 0xB8, 0x67, 0x66, 0x66, 0x66, 0xF7, 0xEA, 0xC1, 0xFA, 0x04, 0x8B, 0xC2, 0xC1,
    0xE8, 0x1F, 0x03, 0xD0, 0xA1, 0x00, 0x2C, 0x1F, 0x02, 0x53, 0x8B, 0xD8, 0x83, 0xC0, 0x01, 0x25,
    0xFF, 0xFF, 0x07, 0x00, 0xC1, 0xE3, 0x0C, 0xA3, 0x00, 0x2C, 0x1F, 0x02, 0x8B, 0x01, 0x56, 0x0B,
    0xD3, 0x3B, 0xC1, 0x8D, 0x71, 0x08, 0x89, 0x79, 0x18, 0x89, 0x16, 0x75, 0x0B, 0x8D, 0x41, 0x04,
    0x89, 0x3D, 0x04, 0x2C, 0x1F, 0x02, 0xEB, 0x1E, 0x8B, 0x51, 0x04, 0x3B, 0xD7, 0xA3, 0x04, 0x2C,
    0x1F, 0x02, 0x8D, 0x41, 0x04, 0x74, 0x04, 0x8B, 0x19, 0x89, 0x1A, 0x8B, 0x11, 0x3B, 0xD7, 0x74,
    0x05, 0x8B, 0x18, 0x89, 0x5A, 0x04, 0x8B, 0x15, 0x08, 0x2C, 0x1F, 0x02, 0x3B, 0xD7, 0x89, 0x38,
    0x89, 0x39, 0x75, 0x10, 0x89, 0x08, 0x8B, 0x06, 0x5E, 0x5B, 0x89, 0x09, 0x89, 0x0D, 0x08, 0x2C,
    0x1F, 0x02, 0x5F, 0xC3, 0x8B, 0x7A, 0x04, 0x89, 0x0F, 0x8B, 0x7A, 0x04, 0x89, 0x38, 0x89, 0x4A,
    0x04, 0x8B, 0x06, 0x5E, 0x5B, 0x89, 0x11, 0x89, 0x0D, 0x08, 0x2C, 0x1F, 0x02, 0x5F, 0xC3,
];
const DETOUR_DESTROY_BODY: [u8; 164] = [
    0x83, 0xCA, 0xFF, 0x3B, 0xCA, 0x57, 0x0F, 0x84, 0x96, 0x00, 0x00, 0x00, 0x8B, 0xC1, 0x25, 0xFF,
    0x0F, 0x00, 0x00, 0x8D, 0x3C, 0x80, 0x39, 0x0C, 0xFD, 0xC0, 0xE0, 0xDE, 0x02, 0x8D, 0x3C, 0xFD,
    0xB8, 0xE0, 0xDE, 0x02, 0x75, 0x7C, 0x39, 0x57, 0x08, 0x74, 0x77, 0x83, 0x7F, 0x18, 0x00, 0x89,
    0x57, 0x08, 0x74, 0x05, 0xE8, 0xD7, 0xCD, 0xFF, 0xFF, 0x39, 0x3D, 0x08, 0x2C, 0x1F, 0x02, 0x75,
    0x17, 0x8B, 0x07, 0x3B, 0xC7, 0x75, 0x0C, 0xC7, 0x05, 0x08, 0x2C, 0x1F, 0x02, 0x00, 0x00, 0x00,
    0x00, 0xEB, 0x1C, 0xA3, 0x08, 0x2C, 0x1F, 0x02, 0x8B, 0x47, 0x04, 0x85, 0xC0, 0x74, 0x04, 0x8B,
    0x0F, 0x89, 0x08, 0x8B, 0x07, 0x85, 0xC0, 0x74, 0x06, 0x8B, 0x57, 0x04, 0x89, 0x50, 0x04, 0xA1,
    0x04, 0x2C, 0x1F, 0x02, 0x85, 0xC0, 0xC7, 0x47, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x07, 0x00,
    0x00, 0x00, 0x00, 0x75, 0x0D, 0x89, 0x3F, 0x89, 0x7F, 0x04, 0x89, 0x3D, 0x04, 0x2C, 0x1F, 0x02,
    0x5F, 0xC3, 0x8B, 0x48, 0x04, 0x89, 0x4F, 0x04, 0x89, 0x07, 0x8B, 0x50, 0x04, 0x89, 0x3A, 0x89,
    0x78, 0x04, 0x5F, 0xC3,
];

const RECORD_ALLOC_BODY: [u8; 116] = [
    0xA1, 0xB8, 0x3A, 0x20, 0x02, 0x85, 0xC0, 0x75, 0x01, 0xC3, 0x83, 0xC9, 0xFF, 0x56, 0x89, 0x48,
    0x28, 0x89, 0x48, 0x24, 0x89, 0x48, 0x2C, 0x8D, 0x70, 0x30, 0x8D, 0x48, 0x1C, 0xC6, 0x06, 0x00,
    0x8B, 0x11, 0x3B, 0xD0, 0x75, 0x1C, 0x80, 0x0E, 0x20, 0xC7, 0x05, 0xB8, 0x3A, 0x20, 0x02, 0x00,
    0x00, 0x00, 0x00, 0xC7, 0x40, 0x20, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x01, 0x00, 0x00, 0x00, 0x00,
    0x5E, 0xC3, 0x89, 0x15, 0xB8, 0x3A, 0x20, 0x02, 0x8B, 0x50, 0x20, 0x85, 0xD2, 0x57, 0x74, 0x05,
    0x8B, 0x39, 0x89, 0x7A, 0x1C, 0x8B, 0x11, 0x85, 0xD2, 0x74, 0x06, 0x8B, 0x78, 0x20, 0x89, 0x7A,
    0x20, 0x80, 0x0E, 0x20, 0x5F, 0xC7, 0x40, 0x20, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x01, 0x00, 0x00,
    0x00, 0x00, 0x5E, 0xC3,
];
const OBSTACLE_ALLOC_BODY: [u8; 143] = [
    0xA1, 0x58, 0x14, 0x2B, 0x02, 0x85, 0xC0, 0x0F, 0x84, 0x81, 0x00, 0x00, 0x00, 0x8B, 0x48, 0x14,
    0x3B, 0xC8, 0x8D, 0x50, 0x14, 0x56, 0x75, 0x0F, 0x8D, 0x48, 0x18, 0xC7, 0x05, 0x58, 0x14, 0x2B,
    0x02, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x22, 0x8B, 0x70, 0x18, 0x85, 0xF6, 0x89, 0x0D, 0x58, 0x14,
    0x2B, 0x02, 0x8D, 0x48, 0x18, 0x53, 0x74, 0x05, 0x8B, 0x1A, 0x89, 0x5E, 0x14, 0x8B, 0x32, 0x85,
    0xF6, 0x74, 0x05, 0x8B, 0x19, 0x89, 0x5E, 0x18, 0x5B, 0xC7, 0x01, 0x00, 0x00, 0x00, 0x00, 0xC7,
    0x02, 0x00, 0x00, 0x00, 0x00, 0x8B, 0xB7, 0x6C, 0x85, 0x00, 0x00, 0x85, 0xF6, 0x75, 0x0C, 0x89,
    0x02, 0x89, 0x01, 0x89, 0x87, 0x6C, 0x85, 0x00, 0x00, 0x5E, 0xC3, 0x8B, 0x76, 0x18, 0x89, 0x31,
    0x8B, 0x8F, 0x6C, 0x85, 0x00, 0x00, 0x89, 0x0A, 0x8B, 0x97, 0x6C, 0x85, 0x00, 0x00, 0x8B, 0x4A,
    0x18, 0x89, 0x41, 0x14, 0x8B, 0x97, 0x6C, 0x85, 0x00, 0x00, 0x89, 0x42, 0x18, 0x5E, 0xC3,
];
const OBSTACLE_RELEASE_BODY: [u8; 76] = [
    0x8B, 0x0D, 0x58, 0x14, 0x2B, 0x02, 0x85, 0xC9, 0x75, 0x11, 0x8B, 0x48, 0x18, 0x89, 0x0D, 0x58,
    0x14, 0x2B, 0x02, 0xC7, 0x40, 0x18, 0x00, 0x00, 0x00, 0x00, 0xC3, 0x8B, 0x50, 0x18, 0x85, 0xD2,
    0x74, 0x22, 0x53, 0x56, 0x8B, 0x71, 0x18, 0x89, 0x56, 0x14, 0x8B, 0x50, 0x18, 0x8B, 0x52, 0x18,
    0x89, 0x4A, 0x14, 0x8B, 0x50, 0x18, 0x8B, 0x59, 0x18, 0x8B, 0x72, 0x18, 0x89, 0x5A, 0x18, 0x89,
    0x71, 0x18, 0x5E, 0x5B, 0xC7, 0x40, 0x18, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const CHUNK_CELL_INDEX_BODY: [u8; 145] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x55, 0x8B, 0x6C, 0x24, 0x10, 0x56, 0x8B, 0xF0, 0x66, 0x8B, 0x46,
    0x2C, 0x66, 0x3B, 0x43, 0x08, 0x74, 0x06, 0x5E, 0x5D, 0x33, 0xC0, 0x5B, 0xC3, 0x57, 0x8B, 0xBB,
    0x40, 0x02, 0x00, 0x00, 0x85, 0xFF, 0x74, 0x2D, 0x8B, 0xCE, 0x2B, 0x4F, 0x14, 0xB8, 0xAB, 0xAA,
    0xAA, 0x2A, 0xF7, 0xE9, 0xC1, 0xFA, 0x03, 0x8B, 0xC2, 0xC1, 0xE8, 0x1F, 0x03, 0xC2, 0x78, 0x15,
    0x0F, 0xB7, 0x4F, 0x10, 0x3B, 0xC1, 0x7D, 0x0D, 0x5F, 0x5E, 0x89, 0x45, 0x00, 0x5D, 0xB8, 0x01,
    0x00, 0x00, 0x00, 0x5B, 0xC3, 0x8B, 0x8B, 0x48, 0x02, 0x00, 0x00, 0x85, 0xC9, 0x74, 0x2B, 0x2B,
    0x71, 0x14, 0xB8, 0xAB, 0xAA, 0xAA, 0x2A, 0xF7, 0xEE, 0xC1, 0xFA, 0x03, 0x8B, 0xC2, 0xC1, 0xE8,
    0x1F, 0x03, 0xC2, 0x78, 0x15, 0x0F, 0xB7, 0x51, 0x10, 0x3B, 0xC2, 0x7D, 0x0D, 0x5F, 0x5E, 0x89,
    0x45, 0x00, 0x5D, 0xB8, 0x02, 0x00, 0x00, 0x00, 0x5B, 0xC3, 0x5F, 0x5E, 0x5D, 0x33, 0xC0, 0x5B,
    0xC3,
];
const CELL_LOCATE_BODY: [u8; 131] = [
    0x51, 0x0F, 0xB7, 0x48, 0x2C, 0x81, 0xF9, 0xFF, 0xFF, 0x00, 0x00, 0x53, 0x8B, 0x5C, 0x24, 0x10,
    0x55, 0x8B, 0x6C, 0x24, 0x10, 0x56, 0x74, 0x64, 0x81, 0xF9, 0x66, 0x08, 0x00, 0x00, 0x73, 0x5C,
    0x8B, 0x34, 0x8D, 0xE8, 0xAA, 0xF7, 0x00, 0x85, 0xF6, 0x74, 0x51, 0x8D, 0x4C, 0x24, 0x0C, 0x51,
    0x56, 0xE8, 0x8A, 0x26, 0x00, 0x00, 0x83, 0xC4, 0x08, 0x83, 0xE8, 0x00, 0x74, 0x3E, 0x83, 0xE8,
    0x01, 0x74, 0x1F, 0x83, 0xE8, 0x01, 0x75, 0x34, 0x0F, 0xB7, 0x56, 0x08, 0x8B, 0x44, 0x24, 0x0C,
    0x5E, 0x89, 0x55, 0x00, 0x5D, 0xC7, 0x03, 0x01, 0x00, 0x00, 0x00, 0x89, 0x07, 0xB0, 0x01, 0x5B,
    0x59, 0xC3, 0x0F, 0xB7, 0x4E, 0x08, 0x8B, 0x54, 0x24, 0x0C, 0x5E, 0x89, 0x4D, 0x00, 0x5D, 0xC7,
    0x03, 0x00, 0x00, 0x00, 0x00, 0x89, 0x17, 0xB0, 0x01, 0x5B, 0x59, 0xC3, 0x5E, 0x5D, 0x32, 0xC0,
    0x5B, 0x59, 0xC3,
];
const SPLINE_EVAL_POS_BODY: [u8; 468] = [
    0x83, 0xEC, 0x08, 0x57, 0x8B, 0xF8, 0x8B, 0x47, 0x0C, 0x83, 0xF8, 0x02, 0x7D, 0x20, 0xA1, 0xD0,
    0xA6, 0x52, 0x02, 0x89, 0x06, 0x8B, 0x0D, 0xD4, 0xA6, 0x52, 0x02, 0x89, 0x4E, 0x04, 0x8B, 0x15,
    0xD8, 0xA6, 0x52, 0x02, 0x89, 0x56, 0x08, 0x5F, 0x83, 0xC4, 0x08, 0xC2, 0x04, 0x00, 0xF3, 0x0F,
    0x2C, 0x54, 0x24, 0x10, 0x85, 0xD2, 0x89, 0x54, 0x24, 0x04, 0x7D, 0x19, 0x8B, 0x3F, 0x8B, 0x07,
    0x89, 0x06, 0x8B, 0x4F, 0x04, 0x89, 0x4E, 0x04, 0x8B, 0x57, 0x08, 0x89, 0x56, 0x08, 0x5F, 0x83,
    0xC4, 0x08, 0xC2, 0x04, 0x00, 0x8D, 0x48, 0xFF, 0x3B, 0xD1, 0x7C, 0x22, 0x8D, 0x14, 0x40, 0x8B,
    0x07, 0x8B, 0x4C, 0x90, 0xF4, 0x8D, 0x44, 0x90, 0xF4, 0x89, 0x0E, 0x8B, 0x50, 0x04, 0x89, 0x56,
    0x04, 0x8B, 0x40, 0x08, 0x89, 0x46, 0x08, 0x5F, 0x83, 0xC4, 0x08, 0xC2, 0x04, 0x00, 0xD9, 0x44,
    0x24, 0x10, 0x8B, 0x0F, 0xDA, 0x64, 0x24, 0x04, 0x8D, 0x04, 0x52, 0x03, 0xC0, 0x03, 0xC0, 0x8D,
    0x54, 0x52, 0x03, 0x03, 0xD2, 0x03, 0xD2, 0x53, 0xD9, 0x5C, 0x24, 0x14, 0x8B, 0x5F, 0x08, 0xD9,
    0x44, 0x24, 0x14, 0x8B, 0x7F, 0x04, 0xD9, 0xC0, 0x03, 0xD8, 0xDC, 0xC8, 0x03, 0xFA, 0xD9, 0x5C,
    0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0xC0, 0xD8, 0xCA, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0xC0,
    0xD8, 0xE2, 0xDD, 0x05, 0xF8, 0xBF, 0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0xC0,
    0xDE, 0xEB, 0xD9, 0xCA, 0xDC, 0x05, 0x40, 0xBF, 0xE5, 0x00, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0xC1,
    0xD8, 0xE3, 0xD8, 0xE3, 0xDE, 0xC4, 0xDC, 0xCB, 0xD9, 0xCB, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0xC0,
    0xDE, 0xEA, 0xD9, 0xC9, 0xDE, 0xCA, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x08, 0x04,
    0xD9, 0x44, 0x24, 0x14, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44,
    0x24, 0x14, 0xD9, 0x44, 0x0A, 0x04, 0xD8, 0xCB, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x43,
    0x04, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xC9, 0xD8, 0x44, 0x24, 0x14, 0xD9,
    0x5C, 0x24, 0x14, 0xD9, 0x47, 0x04, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xC9,
    0xD8, 0x44, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x08, 0x08, 0xD8, 0xCB, 0xD9, 0x5C,
    0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x44, 0x0A, 0x08, 0xD8, 0xCD, 0xDE, 0xC1, 0xD9, 0x5C,
    0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x43, 0x08, 0xD8, 0xCB, 0xF3, 0x0F, 0x10, 0x44, 0x24,
    0x0C, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x47, 0x08, 0xD8, 0xCA,
    0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x04, 0x08, 0xDE, 0xCB, 0xD9, 0xCA, 0xD9, 0x5C, 0x24,
    0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x04, 0x0A, 0xDE, 0xCC, 0xDE, 0xC3, 0xD9, 0xCA, 0xD9, 0x5C,
    0x24, 0x14, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x03, 0x5B, 0xDE, 0xCB, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9,
    0x5C, 0x24, 0x10, 0xD9, 0x07, 0xF3, 0x0F, 0x11, 0x46, 0x04, 0xF3, 0x0F, 0x10, 0x44, 0x24, 0x04,
    0xDE, 0xC9, 0xF3, 0x0F, 0x11, 0x46, 0x08, 0x5F, 0xD8, 0x44, 0x24, 0x0C, 0xD9, 0x1E, 0x83, 0xC4,
    0x08, 0xC2, 0x04, 0x00,
];
