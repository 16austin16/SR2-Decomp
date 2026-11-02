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
const SPLINE_EVAL_TANGENT: usize = 36;
const SPLINE_EVAL_2ND: usize = 37;
const HULL_RELEASE: usize = 38;
const ROAD_ROUTE_LEN: usize = 39;
const TRI_EDGE_SIDE: usize = 40;
const RECORD_VALID: usize = 41;
const PATH_REQ_INIT: usize = 42;
const LANES_ADD: usize = 43;
const BIND_CHUNK: usize = 44;
const HULL_BY_HANDLE: usize = 45;
const LINK_CLOSEST: usize = 46;
const SEGMENT_INTERSECT: usize = 47;
const QUERY_TREE_CB: usize = 48;
const QUERY_FILTER: usize = 49;
const QUERY_FIRST_HIT: usize = 50;
const ROAD_PROBE: usize = 51;
const FREE_PATH_LISTS: usize = 52;
const ROUTE_REFILL: usize = 53;
const PATH_MARK_ENDS: usize = 54;
const GOAL_INSTALL: usize = 55;
const COUNT: usize = 56;

const ROAD_PATH_RECYCLE_ADDR: usize = 0x00B1_CCD0;
const NAV_ROUTE_CLEAR_ADDR: usize = 0x00A1_F730;
const NAV_ROUTE_PUSH_ADDR: usize = 0x00A1_F770;
const NAV_PATH_SETUP_ADDR: usize = 0x00A1_21B0;
const NAV_PATH_FINISH_ADDR: usize = 0x00A1_1520;
const NAV_GOAL_HOOK_ADDR: usize = 0x0068_3FB0;
const NAV_GOAL_COMMIT_ADDR: usize = 0x00B2_C220;
/// Ports without a shadow copy (their callees rewrite rings / pools that cannot be snapshotted).
const NO_SHADOW: [usize; 8] = [ROUTE_CLEAR, HULL_RELEASE, HULL_BY_HANDLE, ROAD_PROBE, FREE_PATH_LISTS, ROUTE_REFILL, PATH_MARK_ENDS, GOAL_INSTALL];

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
    port("spline_eval_tangent", 0x00C0_3AF0, SAVED),
    port("spline_eval_second_derivative", 0x00C0_3DC0, SAVED),
    port("nav_detour_release_hull", 0x00A0_AED0, SAVED),
    port("road_route_total_length", 0x00B1_EA90, SAVED),
    port("nav_tri_edge_side_xz", 0x00A0_F3D0, SAVED),
    port("nav_route_record_valid", 0x00A1_7F60, SAVED),
    port("nav_path_request_init", 0x00A1_3100, ECX | SAVED),
    port("road_lanes_add_neighbours", 0x00B1_E570, SAVED),
    port("road_network_bind_chunk_entries", 0x00A8_D970, SAVED),
    port("nav_detour_release_hull_by_handle", 0x00A0_E1D0, SAVED),
    port("nav_link_closest_point", 0x00A0_2CA0, ECX | SAVED),
    port("nav_segment_intersect_xz", 0x00A0_3C30, ECX | EDX | SAVED),
    port("road_query_tree_cb", 0x00A8_9A20, SAVED),
    port("road_query_filter", 0x00A8_9A50, SAVED),
    port("road_query_cb_first_hit", 0x00A8_9AB0, SAVED),
    port("road_network_probe", 0x00A8_A5D0, SAVED),
    port("vehicle_free_path_lists", 0x00B2_6130, SAVED),
    port("nav_route_refill_from_list", 0x00A1_8570, SAVED),
    port("nav_path_mark_ends", 0x00A1_3660, ECX | SAVED),
    port("vehicle_ai_goal_install", 0x00B2_EE60, ECX | SAVED),
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
unsafe fn shadow_run_m(
    id: usize,
    p: &Pushad,
    argp: *mut u32,
    nargs: usize,
    eax_mask: u32,
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
        let oargs = if RET4.contains(&id) {
            call_full4_ret4(orig, &mut o, args)
        } else if id == PATH_REQ_INIT {
            call_full4_ret12(orig, &mut o, args)
        } else {
            call_full4(orig, &mut o, args)
        };
        check_preserved(id, &inp, &o);
        let after_orig = snap(&reg);
        restore(&reg, &pre);
        let e = ours();
        let after_ours = snap(&reg);
        let args_ok = (0..nargs).all(|i| *argp.add(i) == oargs[i]);
        if after_ours != after_orig || !args_ok || ((e ^ o.eax) & eax_mask != 0) {
            let diff = after_ours.iter().zip(&after_orig).position(|(a, b)| a != b);
            mismatch(id, n, format!("in {inp:x?} args {args:x?}: eax original {:#x}, Rust {e:#x}; first differing region {diff:?}, args ok {args_ok}", o.eax));
            restore(&reg, &after_orig);
            for (i, &a) in oargs.iter().enumerate().take(nargs) {
                *argp.add(i) = a;
            }
            return o.eax;
        }
        // a partial EAX compare: the caller gets the original's whole EAX while it is being shadowed
        if eax_mask != 0 && eax_mask != u32::MAX { o.eax } else { e }
    }
}

/// [`shadow_run_m`] comparing all of EAX (`cmp_eax`) or none of it.
unsafe fn shadow_run(
    id: usize,
    p: &Pushad,
    argp: *mut u32,
    nargs: usize,
    cmp_eax: bool,
    regions: impl FnOnce() -> Option<Vec<(usize, usize)>>,
    ours: impl FnOnce() -> u32,
) -> u32 {
    unsafe { shadow_run_m(id, p, argp, nargs, if cmp_eax { u32::MAX } else { 0 }, regions, ours) }
}

/// Ports whose original pops its stack arguments (`ret 4`).
const RET4: [usize; 3] = [SPLINE_EVAL_POS, SPLINE_EVAL_TANGENT, SPLINE_EVAL_2ND];

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

// Round 3 (ports 36..55): all finished and checked offline against the original in an emulator (about 30000
// random cases, 0 differences; only a signalling-NaN quieting case differs in the emulator). Nothing is in
// progress. Not done: spline_param_at_distance*, lane walks, road_network_query, nav_tri_* with callees.

// --- 0xC03AF0 spline_eval_tangent (EAX = spline, ECX = &out vec3, stack f32 t, `ret 4`) --------------------

const SPLINE_K6: usize = 0x00E5_C058; // 6.0
const SPLINE_KM3: usize = 0x00E5_C858; // -3.0
const SPLINE_K9: usize = 0x00E5_C208; // 9.0
const SPLINE_KM12: usize = 0x00E5_C528; // -12.0
const SPLINE_KM9: usize = 0x00E5_C850; // -9.0
const SPLINE_K2: usize = 0x00E5_BFE8; // 2.0
const SPLINE_KM6: usize = 0x00E5_C500; // -6.0
const SPLINE_K18: usize = 0x00E5_C2F8; // 18.0
const SPLINE_K12: usize = 0x00E5_C530; // 12.0
const SPLINE_KM18: usize = 0x00E5_C848; // -18.0

/// Bezier tangent of the spline (control points: point i, tangent-B[i], tangent-A[i+1], point i+1; the
/// constants are read from the exe, 3.0 = SPLINE_C1). Count < 2 (signed): the default vec3. Then on
/// `comiss 0, t` / `jb` (taken for 0 < t and NaN): t <= 0 gives `f32(C1 * f32(tb[0] - p[0]))` per axis.
/// Otherwise the arg slot is overwritten with count - 1 and `t < count - 1` (x87 fcomi, jb: also NaN)
/// goes interior; the rest gives `f32(C1 * f32(p[last] - ta[last]))`. Interior: i = cvttss2si(t),
/// u = f32(t - i), F2 = f32(u*u), V1 = f32(K4*F2 + K3*u - C1) (K3 = 6, K4 = -3), V2 = f32(F2*9 + u*-12 + C1),
/// V3 = f32(K3*u + F2*-9), V4 = f32(F2 + (F2 + F2)); per axis I = f32(p[i]*V1), B = f32(tb[i]*V2),
/// P = f32(p[i+1]*V4), T = f32(ta[i+1]*V3), out = f32(f32(f32(I + P) + B) + T). Every `fstp dword` rounds
/// to f32, the rest stays f64 (53-bit precision). The dead stack-argument slot ends up holding V3's bits
/// (interior) or count - 1 (t >= count - 1); it is untouched for the first two cases.
unsafe fn spline_eval_tangent(spline: u32, out: u32, args: *mut u32) {
    unsafe {
        let (s, o) = (spline as usize, out as usize);
        let count = rd(s + 0xC) as i32;
        if count < 2 {
            copy_vec3(o, SPLINE_DEFAULT);
            return;
        }
        let t = rf(args as usize);
        let c1 = rd_f64(SPLINE_C1);
        if !(0.0 < t || t.is_nan()) {
            let (pts, tb) = (rd(s) as usize, rd(s + 8) as usize);
            for k in 0..3 {
                let d = (rf(tb + 4 * k) as f64 - rf(pts + 4 * k) as f64) as f32;
                wf(o + 4 * k, (d as f64 * c1) as f32);
            }
            return;
        }
        let cm1 = count.wrapping_sub(1);
        *args = cm1 as u32;
        if !(t.is_nan() || (t as f64) < cm1 as f64) {
            let (pts, ta) = (rd(s) as usize, rd(s + 4) as usize);
            let off = (count as usize).wrapping_mul(12).wrapping_sub(12);
            for k in 0..3 {
                let d = (rf(pts.wrapping_add(off) + 4 * k) as f64 - rf(ta.wrapping_add(off) + 4 * k) as f64) as f32;
                wf(o + 4 * k, (d as f64 * c1) as f32);
            }
            return;
        }
        let i = cvtt(t);
        let (k3, k4, k5, k6, k7) = (rd_f64(SPLINE_K6), rd_f64(SPLINE_KM3), rd_f64(SPLINE_K9), rd_f64(SPLINE_KM12), rd_f64(SPLINE_KM9));
        let u = ((t as f64) - (i as f64)) as f32;
        let ud = u as f64;
        let f2 = (ud * ud) as f32;
        let f2d = f2 as f64;
        let k3u = k3 * ud;
        let v1 = ((k4 * f2d + k3u) - c1) as f32 as f64;
        let v2 = ((f2d * k5 + ud * k6) + c1) as f32 as f64;
        let v3 = (k3u + f2d * k7) as f32;
        let v3d = v3 as f64;
        let v4 = (f2d + (f2d + f2d)) as f32 as f64;
        let (pts, ta0, tb0) = (rd(s) as usize, rd(s + 4) as usize, rd(s + 8) as usize);
        let iu = i as usize;
        let pi = pts.wrapping_add(iu.wrapping_mul(12));
        let pj = pts.wrapping_add(iu.wrapping_add(1).wrapping_mul(12));
        let ta = ta0.wrapping_add(iu.wrapping_add(1).wrapping_mul(12));
        let tb = tb0.wrapping_add(iu.wrapping_mul(12));
        let mut res = [0f32; 3];
        for (k, r) in res.iter_mut().enumerate() {
            let ii = (rf(pi + 4 * k) as f64 * v1) as f32;
            let bb = (rf(tb + 4 * k) as f64 * v2) as f32;
            let pp = (rf(pj + 4 * k) as f64 * v4) as f32;
            let tt = (rf(ta + 4 * k) as f64 * v3d) as f32;
            let q = (ii as f64 + pp as f64) as f32;
            let rr = (q as f64 + bb as f64) as f32;
            *r = (rr as f64 + tt as f64) as f32;
        }
        for (k, r) in res.iter().enumerate() {
            wf(o + 4 * k, *r);
        }
        *args = v3.to_bits();
    }
}

extern "C" fn spline_eval_tangent_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        shadow_run(
            SPLINE_EVAL_TANGENT,
            &p,
            args as *mut u32,
            1,
            false,
            || Some(vec![(p.ecx as usize, 12)]),
            || {
                spline_eval_tangent(p.eax, p.ecx, args as *mut u32);
                0
            },
        )
    };
}
regs_thunk_ret4!(spline_eval_tangent_thunk, spline_eval_tangent_entry);

// --- 0xC03DC0 spline_eval_second_derivative (ECX = spline, EDX = &out vec3, stack f32 t, `ret 4`) -----------

/// Second derivative of the same Bezier. Count < 2 gives the default vec3. `comiss 0, t` / `jbe` is taken for
/// t >= 0 and NaN, so only t < 0 takes the first path: `6 * f32(ta[1] + f32(p[0] - f32(2 * tb[0])))` per
/// axis (f32 rounding after each step; K2 = 2, K6 = 6). Then the arg slot is overwritten with count - 1 and
/// `t < count - 1` (or NaN) goes interior; else `6 * f32(p[last] + f32(tb[count-2] - f32(2 * ta[last])))`.
/// Interior: i = cvttss2si(t), u = f32(t - i), W1 = f32(-6u + 6), W2 = f32(18u - 12), W3 = f32(-18u + 6),
/// W4 = f32(6u); per axis I = f32(p[i]*W1), B = f32(tb[i]*W2), T = f32(ta[i+1]*W3), P = f32(p[i+1]*W4),
/// out = f32(f32(f32(I + B) + T) + P). The dead stack-argument slot ends up holding W4's bits (interior),
/// count - 1 (t >= count - 1) or the untouched t (t < 0).
unsafe fn spline_eval_second_derivative(spline: u32, out: u32, args: *mut u32) {
    unsafe {
        let (s, o) = (spline as usize, out as usize);
        let count = rd(s + 0xC) as i32;
        if count < 2 {
            copy_vec3(o, SPLINE_DEFAULT);
            return;
        }
        let t = rf(args as usize);
        let (k2, k6) = (rd_f64(SPLINE_K2), rd_f64(SPLINE_K6));
        if t < 0.0 {
            let (pts, tb, ta1) = (rd(s) as usize, rd(s + 8) as usize, (rd(s + 4) as usize).wrapping_add(12));
            for k in 0..3 {
                let w = (rf(tb + 4 * k) as f64 * k2) as f32;
                let d = (rf(pts + 4 * k) as f64 - w as f64) as f32;
                let e = (rf(ta1 + 4 * k) as f64 + d as f64) as f32;
                wf(o + 4 * k, (e as f64 * k6) as f32);
            }
            return;
        }
        let cm1 = count.wrapping_sub(1);
        *args = cm1 as u32;
        if !(t.is_nan() || (t as f64) < cm1 as f64) {
            let (pts, ta, tb) = (rd(s) as usize, rd(s + 4) as usize, rd(s + 8) as usize);
            let off = (count as usize).wrapping_mul(12);
            let a_last = ta.wrapping_add(off).wrapping_sub(12);
            let b2 = tb.wrapping_add(off).wrapping_sub(24);
            let p_last = pts.wrapping_add(off).wrapping_sub(12);
            for k in 0..3 {
                let w = (rf(a_last + 4 * k) as f64 * k2) as f32;
                let d = (rf(b2 + 4 * k) as f64 - w as f64) as f32;
                let e = (rf(p_last + 4 * k) as f64 + d as f64) as f32;
                wf(o + 4 * k, (e as f64 * k6) as f32);
            }
            return;
        }
        let i = cvtt(t);
        let (kq, kb, kc, kd) = (rd_f64(SPLINE_KM6), rd_f64(SPLINE_K18), rd_f64(SPLINE_K12), rd_f64(SPLINE_KM18));
        let u = ((t as f64) - (i as f64)) as f32;
        let ud = u as f64;
        let w1 = (kq * ud + k6) as f32 as f64;
        let w2 = (ud * kb - kc) as f32 as f64;
        let w3 = (ud * kd + k6) as f32 as f64;
        let w4 = (k6 * ud) as f32;
        let w4d = w4 as f64;
        let (pts, ta0, tb0) = (rd(s) as usize, rd(s + 4) as usize, rd(s + 8) as usize);
        let iu = i as usize;
        let pi = pts.wrapping_add(iu.wrapping_mul(12));
        let pj = pts.wrapping_add(iu.wrapping_add(1).wrapping_mul(12));
        let ta = ta0.wrapping_add(iu.wrapping_add(1).wrapping_mul(12));
        let tb = tb0.wrapping_add(iu.wrapping_mul(12));
        let mut res = [0f32; 3];
        for (k, r) in res.iter_mut().enumerate() {
            let pp = (rf(pj + 4 * k) as f64 * w4d) as f32;
            let tt = (rf(ta + 4 * k) as f64 * w3) as f32;
            let bb = (rf(tb + 4 * k) as f64 * w2) as f32;
            let ii = (rf(pi + 4 * k) as f64 * w1) as f32;
            let q = (ii as f64 + bb as f64) as f32;
            let rr = (q as f64 + tt as f64) as f32;
            *r = (rr as f64 + pp as f64) as f32;
        }
        for (k, r) in res.iter().enumerate() {
            wf(o + 4 * k, *r);
        }
        *args = w4.to_bits();
    }
}

extern "C" fn spline_eval_2nd_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        shadow_run(
            SPLINE_EVAL_2ND,
            &p,
            args as *mut u32,
            1,
            false,
            || Some(vec![(p.edx as usize, 12)]),
            || {
                spline_eval_second_derivative(p.ecx, p.edx, args as *mut u32);
                0
            },
        )
    };
}
regs_thunk_ret4!(spline_eval_2nd_thunk, spline_eval_2nd_entry);

// --- 0xA0AED0 nav_detour_release_hull (EDI = hull object; EAX = the original's leftover) -------------------------

const HULL_LIST: usize = 0x021F_B8C0; // list walked while the hull's ring node has flag bit 0
const HULL_FREE_RING: usize = 0x021F_C194;
const HULL_RING_FLAG: usize = 0x021F_C190; // ring head used with the flag set
const HULL_RING_PLAIN: usize = 0x021F_C198; // ring head used with the flag clear
const NAV_HULL_RELEASE: usize = 0x00A0_AED0;
const NAV_LIST_UNLINK: usize = 0x00A0_7DB0;
const NAV_HULL_FREE_REFS: usize = 0x00A0_7E20;

/// `node = [hull + 0x18]` (a ring node; 0 = nothing to do, EAX 0). Flag byte `[node + 8] & 1` set: for every
/// entry E of the circular list at [0x21FB8C0] (stops when the next link is the head or 0) with a sub-list
/// `[E + 0x2C]`, the first element whose `[+8]` equals `[hull + 8]` goes to 0xA07DB0 (EAX = element,
/// ECX = sub-list head, EDX = ESI = E, EDI = hull) and the sub-list scan stops; then the node leaves the ring
/// whose head slot is 0x21FC190. Flag clear: if `[node + 0x54]` is non-null 0xA07E20 runs (ESI = that, stack
/// arg 0) and the node is re-read; the head slot is 0x21FC198. A head slot equal to the node either clears a
/// single-node ring (`[node] == node`: [node] = 0, [[hull+0x18]+4] = 0, head = 0, straight to the free-ring
/// step) or moves to `[node]`. Then generic unlink (prev [+4], next [+0]), both links cleared, and the node
/// goes into the free ring 0x21FC194 (empty: self-linked and it becomes the head; else inserted before the
/// head), `[hull + 0x18] = 0`. Every field is re-read from memory exactly where the original re-reads it.
/// EAX leaves 0 (no node), the node (empty free ring) or the old free-ring head. ECX / EDX are passed through
/// to 0xA07E20 as the caller left them (nothing touches them before that call). No shadow: the ring writes are
/// unbounded; checked offline against the original in an emulator instead.
unsafe fn detour_release_hull(hull: u32, ecx_in: u32, edx_in: u32) -> u32 {
    unsafe {
        let h = hull as usize;
        let mut eax = rd(h + 0x18) as usize;
        if eax == 0 {
            return 0;
        }
        let single_clear;
        if rb(eax + 8) & 1 != 0 {
            let ebx = rd(HULL_LIST) as usize;
            if ebx != 0 {
                let mut esi = ebx;
                loop {
                    let ecx = rd(esi + 0x2C) as usize;
                    let mut e = ecx;
                    if e != 0 {
                        let edx = rd(h + 8);
                        loop {
                            if rd(e + 8) == edx {
                                call_regs(NAV_LIST_UNLINK, e as u32, ecx as u32, esi as u32, esi as u32, hull, &[]);
                                break;
                            }
                            e = rd(e) as usize;
                            if e == ecx || e == 0 {
                                break;
                            }
                        }
                    }
                    let next = rd(esi) as usize;
                    if next == ebx {
                        break;
                    }
                    esi = next;
                    if esi == 0 {
                        break;
                    }
                }
            }
            eax = rd(h + 0x18) as usize;
            if rd(HULL_RING_FLAG) as usize != eax {
                single_clear = false;
            } else if rd(eax) as usize == eax {
                wd(eax, 0);
                eax = rd(h + 0x18) as usize;
                wd(eax + 4, 0);
                wd(HULL_RING_FLAG, 0);
                single_clear = true;
            } else {
                wd(HULL_RING_FLAG, rd(eax));
                single_clear = false;
            }
        } else {
            let esi = rd(eax + 0x54);
            if esi != 0 {
                call_regs(NAV_HULL_FREE_REFS, eax as u32, ecx_in, edx_in, esi, hull, &[0]);
                eax = rd(h + 0x18) as usize;
            }
            if rd(HULL_RING_PLAIN) as usize != eax {
                single_clear = false;
            } else if rd(eax) as usize == eax {
                wd(eax, 0);
                let edx = rd(h + 0x18) as usize;
                wd(HULL_RING_PLAIN, 0);
                wd(edx + 4, 0);
                single_clear = true;
            } else {
                wd(HULL_RING_PLAIN, rd(eax));
                single_clear = false;
            }
        }
        if !single_clear {
            let prev = rd(eax + 4) as usize;
            if prev != 0 {
                wd(prev, rd(eax));
            }
            eax = rd(h + 0x18) as usize;
            if rd(eax) != 0 {
                let next = rd(eax) as usize;
                wd(next + 4, rd(eax + 4));
            }
            eax = rd(h + 0x18) as usize;
            wd(eax, 0);
            let ecx = rd(h + 0x18) as usize;
            wd(ecx + 4, 0);
        }
        eax = rd(HULL_FREE_RING) as usize;
        if eax == 0 {
            let n = rd(h + 0x18) as usize;
            wd(n, n as u32);
            let n = rd(h + 0x18) as usize;
            wd(n + 4, n as u32);
            wd(HULL_FREE_RING, rd(h + 0x18));
            wd(h + 0x18, 0);
            return n as u32;
        }
        let ecx = rd(h + 0x18) as usize;
        wd(ecx + 4, rd(eax + 4));
        let ecx = rd(h + 0x18) as usize;
        wd(ecx, eax as u32);
        let edx = rd(eax + 4) as usize;
        let ecx = rd(h + 0x18);
        wd(edx, ecx);
        let edx = rd(h + 0x18);
        wd(eax + 4, edx);
        wd(h + 0x18, 0);
        eax as u32
    }
}

extern "C" fn hull_release_entry(r: &mut Pushad, _: *const u32) {
    PORTS[HULL_RELEASE].calls.fetch_add(1, Ordering::Relaxed);
    r.eax = unsafe { detour_release_hull(r.edi, r.ecx, r.edx) };
}
regs_thunk!(hull_release_thunk, hull_release_entry);

// --- 0xB1EA90 road_route_total_length (ESI = ring head, stack u8 flag -> ST0, EAX = last link read) ---------

const ROAD_LEN_SCALE: usize = 0x00E5_BF70;

/// `sum` starts at 0.0f. Head 0: sum 0, EAX 0. Otherwise walks the ring from the head (EAX = ESI, link
/// [+0], stop when it equals the head or is 0): `ctl = [[p + 0xC] + 0x10]`, `rec = [p + 0xC]`; ctl bit 29
/// set skips the step; else x = f32 [rec + 0x14] (the load quiets a signalling NaN), when the flag byte == 1
/// and ctl bit 4 is set x = f32(x * [0xE5BF70]); `sum = f32(x + sum)`. Returns the sum in ST0 and
/// EAX = the final link (the head or 0).
unsafe fn road_route_total_length(head: u32, flag: u8) -> (f32, u32) {
    unsafe {
        let mut sum = 0.0f32;
        if head == 0 {
            return (sum, 0);
        }
        let scale = rd_f64(ROAD_LEN_SCALE);
        let mut p = head;
        loop {
            let rec = rd(p as usize + 0xC) as usize;
            let ctl = rd(rec + 0x10);
            if ctl & 0x2000_0000 == 0 {
                let mut x = f32::from_bits(quiet32(rd(rec + 0x14)));
                if flag == 1 && ctl & 0x10 != 0 {
                    x = (x as f64 * scale) as f32;
                }
                sum = (x as f64 + sum as f64) as f32;
            }
            p = rd(p as usize);
            if p == head || p == 0 {
                return (sum, p);
            }
        }
    }
}

/// `fld dword` quiets a signalling NaN (bit 22 set) and keeps everything else.
fn quiet32(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x7F_FFFF != 0 { bits | 0x40_0000 } else { bits }
}

extern "C" fn road_route_len_entry(r: &mut Pushad, args: *const u32) -> f32 {
    let p = *r;
    let arg0 = unsafe { *args };
    let flag = arg0 as u8;
    let (mut ours, eax) = unsafe { road_route_total_length(p.esi, flag) };
    r.eax = eax;
    let Some((n, orig)) = shadowing(ROAD_ROUTE_LEN) else { return ours };
    let inp = Regs::of(&p);
    let mut out = inp;
    let theirs = unsafe { call_full(orig, &mut out, arg0, true) } as f32;
    check_preserved(ROAD_ROUTE_LEN, &inp, &out);
    if theirs.to_bits() != ours.to_bits() || out.eax != eax {
        mismatch(ROAD_ROUTE_LEN, n, format!("in {inp:x?} flag {flag}: original {:#x} eax {:#x}, Rust {:#x} eax {eax:#x}", theirs.to_bits(), out.eax, ours.to_bits()));
        r.eax = out.eax;
        ours = theirs;
    }
    ours
}
regs_thunk!(road_route_len_thunk, road_route_len_entry);

// --- 0xA0F3D0 nav_tri_edge_side_xz (ESI = tri (3 vertex ptrs), EAX = edge, EDI = point (x, z) -> EAX 0 / 1 / 2) ---

const TRI_EPS: usize = 0x00E5_C1A0; // 1e-4
const TRI_NEG_EPS: usize = 0x00E5_C430; // -1e-4

/// a = tri[edge], b = tri[(edge + 1) % 3] (signed idiv remainder); when a > b (unsigned pointers) they
/// swap and the swapped flag is set. d = b - a on z (`dz`) and a - b on x (`dx`), p = point - a
/// (`px = pt.x - a.x`, `pz = pt.z - a.z`), each stored as f32; `cross = f32(pz*dx + px*dz)`. Then
/// (`fcom` against +-1e-4, f64 constants): not swapped gives 2 for cross > eps, 1 for cross < -eps;
/// swapped gives 1 for cross > eps, 2 for cross < -eps; anything else (including NaN) 0.
unsafe fn tri_edge_side(tri: u32, edge: u32, pt: u32) -> u32 {
    unsafe {
        let t = tri as usize;
        let mut a = rd(t.wrapping_add((edge as usize).wrapping_mul(4))) as usize;
        let rem = (edge as i32).wrapping_add(1) % 3;
        let mut b = rd(t.wrapping_add((rem as u32 as usize).wrapping_mul(4))) as usize;
        let mut swapped = false;
        if a > b {
            std::mem::swap(&mut a, &mut b);
            swapped = true;
        }
        let dz = (rf(b + 8) as f64 - rf(a + 8) as f64) as f32;
        let dx = (rf(a) as f64 - rf(b) as f64) as f32;
        let px = (rf(pt as usize) as f64 - rf(a) as f64) as f32;
        let pz = (rf(pt as usize + 4) as f64 - rf(a + 8) as f64) as f32;
        let cross = ((pz as f64 * dx as f64) + (px as f64 * dz as f64)) as f32 as f64;
        let (eps, neg_eps) = (rd_f64(TRI_EPS), rd_f64(TRI_NEG_EPS));
        if cross > eps {
            if swapped { 1 } else { 2 }
        } else if cross < neg_eps {
            if swapped { 2 } else { 1 }
        } else {
            0
        }
    }
}

extern "C" fn tri_edge_side_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(TRI_EDGE_SIDE, &p, unsafe { tri_edge_side(p.esi, p.eax, p.edi) });
}
regs_thunk!(tri_edge_side_thunk, tri_edge_side_entry);

// --- 0xA17F60 nav_route_record_valid (ECX = pointer -> AL: inside the route pool on a 0x34 boundary) --------

/// `off = ecx - 0x2E400D0`; negative (signed) or >= 0x6800 gives AL = 0. Else AL = `off % 0x34 == 0` via
/// the reciprocal multiply (`hi32(off * 0x4EC4EC4F) >> 4`, times 0x34, subtracted). Registers as the
/// original leaves them: EAX = the low half of the product with AL replaced (valid range) or the caller's
/// EAX with AL cleared; ECX = off (invalid) or the remainder; EDX = the quotient times 0x34 (valid range).
fn route_record_valid(ecx: u32, eax: u32, edx: u32) -> (u32, u32, u32) {
    let off = ecx.wrapping_sub(0x02E4_00D0);
    if (off as i32) < 0 || off >= 0x6800 {
        return (eax & !0xFF, off, edx);
    }
    let prod = off as u64 * 0x4EC4_EC4F;
    let q = ((prod >> 32) as u32) >> 4;
    let m = q.wrapping_mul(0x34);
    let rem = off.wrapping_sub(m);
    ((prod as u32 & !0xFF) | (rem == 0) as u32, rem, m)
}

extern "C" fn record_valid_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let (eax, ecx, edx) = route_record_valid(p.ecx, p.eax, p.edx);
    let eax = checked_eax(RECORD_VALID, &p, eax);
    r.eax = eax;
    r.ecx = ecx;
    r.edx = edx;
}
regs_thunk!(record_valid_thunk, record_valid_entry);

// --- 0xA13100 nav_path_request_init (ECX = request, EDX = &{u16 flags, u8 extra}, ESI = start vec3, EDI = goal vec3;
//     stack count, waypoints, f32 radius; `ret 0xC`; EAX = request) ----------------------------------------------

/// Stores: [+0] count, [+4] waypoint ptr, [+8] u16 flags and [+0xA] extra byte from EDX's struct; start
/// position (+0xC..) copied from ESI when flags bit 15 is set, else three 0.0 bit patterns; [+0x24] =
/// radius (`fld`/`fst` dword: a signalling NaN comes out quieted); goal (+0x18..) copied from EDI when
/// the radius is strictly greater than 0 (an x87 `fcomp` against 0.0 with `test ah, 0x41`, so <= 0 and NaN
/// take the zero branch), else three zeros. The leading clears of +8 / +9 / +0xA are overwritten, but done
/// in the original order.
unsafe fn path_request_init(req: u32, flags: u32, start: u32, goal: u32, args: *const u32) {
    unsafe {
        let c = req as usize;
        wb(c + 8, 0);
        wb(c + 9, 0);
        wb(c + 0xA, 0);
        wd(c, *args);
        wd(c + 4, *args.add(1));
        let f = flags as usize;
        let (lo, hi, ex) = (rb(f), rb(f + 1), rb(f + 2));
        wb(c + 8, lo);
        wb(c + 9, hi);
        wb(c + 0xA, ex);
        if rb(c + 9) & 0x80 != 0 {
            copy_vec3(c + 0xC, start as usize);
        } else {
            for k in 0..3 {
                wd(c + 0xC + 4 * k, 0);
            }
        }
        let radius_bits = quiet32(*args.add(2));
        wd(c + 0x24, radius_bits);
        if f32::from_bits(radius_bits) > 0.0 {
            copy_vec3(c + 0x18, goal as usize);
        } else {
            for k in 0..3 {
                wd(c + 0x18 + 4 * k, 0);
            }
        }
    }
}

/// Like [`regs_thunk_ret4`] for `ret 0xC`.
macro_rules! regs_thunk_ret12 {
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
                "ret 12",
                f = sym $f,
            )
        }
    };
}

/// Like [`call_full4`] for a callee that pops 12 bytes of stack arguments (`ret 0xC`).
unsafe fn call_full4_ret12(f: usize, r: &mut Regs, args: [u32; 4]) -> [u32; 4] {
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
            "sub esp, 12",
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

extern "C" fn path_request_init_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    r.eax = unsafe {
        shadow_run(
            PATH_REQ_INIT,
            &p,
            args as *mut u32,
            3,
            true,
            || Some(vec![(p.ecx as usize, 0x28)]),
            || {
                path_request_init(p.ecx, p.edx, p.esi, p.edi, args);
                p.ecx
            },
        )
    };
}
regs_thunk_ret12!(path_request_init_thunk, path_request_init_entry);

// --- 0xB1E570 road_lanes_add_neighbours (EAX = &count, ESI = lane ptr array, stack cap (int), u8 flag; AL = count >= cap) ---


/// The lane record of index `cx` (u16): `[[net] + 4] + cx * 0x3C` (the original multiplies by 15 and uses a
/// `*4` addressing mode). Returns the 32-bit wrapped address.
unsafe fn lane_rec(cx: u16) -> usize {
    unsafe { (rd(rd(ROAD_NET) as usize + 4) as usize).wrapping_add((cx as usize * 15).wrapping_mul(4)) }
}

/// The dedupe-and-append step: `edi` entries are scanned (signed `edi <= 0` scans nothing); a hit leaves
/// the array alone; otherwise `arr[edi] = rec` and `[count] = edi + 1`.
unsafe fn lane_append(cnt: usize, arr: usize, edi: i32, rec: usize) {
    unsafe {
        let mut k = 0i32;
        while k < edi {
            if rd(arr.wrapping_add((k as u32 as usize).wrapping_mul(4))) as usize == rec {
                return;
            }
            k += 1;
        }
        wd(arr.wrapping_add((edi as u32 as usize).wrapping_mul(4)), rec as u32);
        wd(cnt, (edi as u32).wrapping_add(1));
    }
}

/// Pass 1, for ebx = 0 .. count (count re-read): leave when count >= cap; append the lane in u16 [lane + 0x36]
/// (0xFFFF = none, null address = none, already present = skip), then, when lane byte [+0x10] bit 0 is clear
/// and count < cap, the one in u16 [lane + 0x38]. Flag byte 0 stops there. Else pass 2 over the entries
/// that existed when pass 1 ended (n = count): those with [+0x10] bit 0 set append their [+0x38] lane while
/// count < cap; pass 3 over the entries from n to the growing count appends their [+0x36] lanes. AL leaves
/// as `count >= cap` (signed) and the rest of EAX is the pointer's.
unsafe fn lanes_add_neighbours(cnt: u32, arr: u32, cap: i32, flag: u8) -> u32 {
    unsafe {
        let (cnt_p, arr_p) = (cnt as usize, arr as usize);
        let item = |i: i32| rd(arr_p.wrapping_add((i as u32 as usize).wrapping_mul(4))) as usize;
        let count = || rd(cnt_p) as i32;
        let mut ebx = 0i32;
        if count() > 0 {
            loop {
                let edi = count();
                if edi >= cap {
                    break;
                }
                let e = item(ebx);
                let cx = rw(e + 0x36);
                if cx != 0xFFFF {
                    let rec = lane_rec(cx);
                    if rec != 0 {
                        lane_append(cnt_p, arr_p, edi, rec);
                    }
                }
                let e = item(ebx);
                if rb(e + 0x10) & 1 == 0 {
                    let cx = rw(e + 0x38);
                    if cx != 0xFFFF {
                        let rec = lane_rec(cx);
                        if rec != 0 {
                            let edi = count();
                            if edi < cap {
                                lane_append(cnt_p, arr_p, edi, rec);
                            }
                        }
                    }
                }
                ebx += 1;
                if ebx >= count() {
                    break;
                }
            }
        }
        if flag != 0 {
            let n = count();
            let mut ebp = 0i32;
            if n > 0 {
                loop {
                    let edi = count();
                    if edi >= cap {
                        break;
                    }
                    let e = item(ebp);
                    if rb(e + 0x10) & 1 != 0 {
                        let cx = rw(e + 0x38);
                        if cx != 0xFFFF {
                            let rec = lane_rec(cx);
                            if rec != 0 {
                                lane_append(cnt_p, arr_p, edi, rec);
                            }
                        }
                    }
                    ebp += 1;
                    if ebp >= n {
                        break;
                    }
                }
            }
            ebp = n;
            if n < count() {
                loop {
                    let edi = count();
                    if edi >= cap {
                        break;
                    }
                    let e = item(ebp);
                    let cx = rw(e + 0x36);
                    if cx != 0xFFFF {
                        let rec = lane_rec(cx);
                        if rec != 0 {
                            lane_append(cnt_p, arr_p, edi, rec);
                        }
                    }
                    ebp += 1;
                    if ebp >= count() {
                        break;
                    }
                }
            }
        }
        (cnt & !0xFF) | (count() >= cap) as u32
    }
}

extern "C" fn lanes_add_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (cap, flag) = (*args as i32, *args.add(1) as u8);
        r.eax = shadow_run(
            LANES_ADD,
            &p,
            args as *mut u32,
            2,
            true,
            || {
                let count = rd(p.eax as usize) as i32;
                let room = (cap as i64 - count.max(0) as i64).clamp(0, 0x4000) as usize;
                // entries from max(count, 0) up to cap can be written (the count itself is the other region)
                if cap as i64 - count.max(0) as i64 > 0x4000 {
                    return None;
                }
                Some(vec![(p.eax as usize, 4), (p.esi as usize + 4 * count.max(0) as usize, 4 * room)])
            },
            || lanes_add_neighbours(p.eax, p.esi, cap, flag),
        );
    }
}
regs_thunk!(lanes_add_thunk, lanes_add_entry);

// --- 0xA8D970 road_network_bind_chunk_entries (stack table, table count, owner; plain `ret`) ----------------

/// Table = `arg0` (20-byte records: key [+4], handle [+8], two values [+0xC], [+0x10]), `n = arg1` (signed);
/// owner = `arg2` with entry count [+0x144] (signed) and entries pointer [+0x148] (40-byte entries, key [+0]).
/// Nothing happens when the entries pointer is 0, n <= 0 or the count <= 0. Else for each entry j the scan
/// resumes at the last matched table index (`edx`, starting 0; skipped when edx >= n): the first record at
/// or after it whose key equals the entry's key; on a hit, a record with handle -1 gets
/// word [rec] = u16 [owner + 8] and handle = j; then entry[+4] = rec[+0xC], entry[+8] = rec[+0x10] and the
/// resume index becomes the hit. EAX leaves as the entries pointer (nothing done) or the final count.
unsafe fn bind_chunk_entries(table: u32, n: i32, owner: u32) -> u32 {
    unsafe {
        let o = owner as usize;
        let entries = rd(o + 0x148);
        let m = rd(o + 0x144) as i32;
        if entries == 0 || n <= 0 || m <= 0 {
            return entries;
        }
        let tb = table as usize;
        let rec_at = |i: i32| tb.wrapping_add((i as u32 as usize).wrapping_mul(20));
        let mut edx = 0i32;
        let mut edi = entries as usize;
        let mut j = 0i32;
        loop {
            let mut eax = edx;
            if edx < n {
                let key = rd(edi);
                let mut found = false;
                loop {
                    if rd(rec_at(eax) + 4) == key {
                        found = true;
                        break;
                    }
                    eax += 1;
                    if eax >= n {
                        break;
                    }
                }
                if found && eax < n {
                    let rec = rec_at(eax);
                    if rd(rec + 4) == key {
                        if rd(rec + 8) == 0xFFFF_FFFF {
                            let w = rw(o + 8);
                            wb(rec, w as u8);
                            wb(rec + 1, (w >> 8) as u8);
                            wd(rec + 8, j as u32);
                        }
                        wd(edi + 4, rd(rec + 0xC));
                        wd(edi + 8, rd(rec + 0x10));
                        edx = eax;
                    }
                }
            }
            j += 1;
            edi = edi.wrapping_add(0x28);
            if j >= m {
                return j as u32;
            }
        }
    }
}

extern "C" fn bind_chunk_entries_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (table, n, owner) = (*args, *args.add(1) as i32, *args.add(2));
        r.eax = shadow_run(
            BIND_CHUNK,
            &p,
            args as *mut u32,
            3,
            true,
            || {
                let (entries, m) = (rd(owner as usize + 0x148) as usize, rd(owner as usize + 0x144) as i32);
                if entries == 0 || n <= 0 || m <= 0 {
                    return Some(vec![]);
                }
                if m > 0x2000 || n > 0x2000 {
                    return None;
                }
                Some(vec![(entries, 0x28 * m as usize), (table as usize, 20 * n as usize)])
            },
            || bind_chunk_entries(table, n, owner),
        );
    }
}
regs_thunk!(bind_chunk_entries_thunk, bind_chunk_entries_entry);

// --- 0xA0E1D0 nav_detour_release_hull_by_handle (ECX = handle or -1) -------------------------------------------

/// `-1` does nothing. Else slot = handle & 0xFFF in the 40-byte table at 0x2DEE0B8 (handle at [+8]); a
/// matching handle runs nav_detour_release_hull (EDI = the slot, called by address); EAX leaves as the
/// call's result, `handle & 0xFFF` (no match) or unchanged (-1). No shadow: the call changes the rings.
extern "C" fn hull_by_handle_entry(r: &mut Pushad, _: *const u32) {
    PORTS[HULL_BY_HANDLE].calls.fetch_add(1, Ordering::Relaxed);
    let h = r.ecx;
    if h == 0xFFFF_FFFF {
        return;
    }
    unsafe {
        let idx = (h & 0xFFF) as usize;
        let slot = 0x02DE_E0B8 + idx * 40;
        r.eax = h & 0xFFF;
        if rd(slot + 8) == h {
            r.eax = call_regs(NAV_HULL_RELEASE, h & 0xFFF, r.ecx, r.edx, r.esi, slot as u32, &[]);
        }
    }
}
regs_thunk!(hull_by_handle_thunk, hull_by_handle_entry);

// --- 0xA02CA0 nav_link_closest_point (ECX = link, EAX = point P, ESI = point Q -> AL) ----------------------------

const LINK_ZERO: usize = 0x00E5_BFA0; // 0.0

/// `checked_eax` that only compares the bits in `mask` (the rest of EAX is x87 status junk from `fnstsw ax`).
/// While shadowing, the original's whole EAX is what the caller gets.
fn checked_eax_masked(id: usize, p: &Pushad, ours: u32, mask: u32) -> u32 {
    let Some((n, orig)) = shadowing(id) else { return ours };
    let r = run_original(id, orig, p);
    if (r.eax ^ ours) & mask != 0 {
        mismatch(id, n, format!("in {:x?}: original {:#x}, Rust {ours:#x} (mask {mask:#x})", Regs::of(p), r.eax));
    }
    r.eax
}

/// Link record: endpoints A = [+0 .. +8], B = [+0xC .. +0x14] (f32 vec3), flags byte [+0x18]. Bit 1 clear
/// gives AL = 0 (the rest of EAX untouched). Else `d = B - A` (each f32), P' = f32(P - A), Q' = f32(Q - A).
/// Bit 0 set: `dot1 = f32(P'.x*d.z + P'.y*d.y + P'.z*(-d.x))` must not be negative (NaN passes) and
/// `dot2 = f32(Q'.z*(-d.x) + (Q'.x*d.z + Q'.y*d.y))` must be negative or NaN for AL = 1. Bit 0 clear:
/// `dotQ` (the same expression over Q') times `dotP` (over P') must be < 0.0 for EAX = 1 (NaN gives 0).
/// Only AL is meaningful (the original leaves the x87 status word in AH on most paths).
unsafe fn link_closest_point(link: u32, p: u32, q: u32, eax_in: u32) -> u32 {
    unsafe {
        let l = link as usize;
        let flags = rb(l + 0x18);
        let low = eax_in & !0xFF;
        if flags & 2 == 0 {
            return low;
        }
        let (ax, ay, az) = (rf(l) as f64, rf(l + 4) as f64, rf(l + 8) as f64);
        let dx = (rf(l + 0xC) as f64 - ax) as f32;
        let dy = (rf(l + 0x10) as f64 - ay) as f32;
        let dz = (rf(l + 0x14) as f64 - az) as f32;
        let ndx = -dx;
        let (dyd, dzd, ndxd) = (dy as f64, dz as f64, ndx as f64);
        let sub = |v: u32, k: usize, a: f64| (rf(v as usize + 4 * k) as f64 - a) as f32 as f64;
        if flags & 1 != 0 {
            let (px, py, pz) = (sub(p, 0, ax), sub(p, 1, ay), sub(p, 2, az));
            let dot1 = ((px * dzd + py * dyd) + pz * ndxd) as f32;
            if dot1 < 0.0 {
                return low;
            }
            let (qx, qy, qz) = (sub(q, 0, ax), sub(q, 1, ay), sub(q, 2, az));
            let dot2 = ((dyd * qy + dzd * qx) + ndxd * qz) as f32;
            if dot2 < 0.0 || dot2.is_nan() { low | 1 } else { low }
        } else {
            let (px, py, pz) = (sub(p, 0, ax), sub(p, 1, ay), sub(p, 2, az));
            let (qx, qy, qz) = (sub(q, 0, ax), sub(q, 1, ay), sub(q, 2, az));
            let dot_q = ((qx * dzd + qy * dyd) + qz * ndxd) as f32;
            let dot_p = ((dyd * py + dzd * px) + ndxd * pz) as f32;
            let prod = dot_q as f64 * dot_p as f64;
            (prod < rd_f64(LINK_ZERO)) as u32
        }
    }
}

extern "C" fn link_closest_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax_masked(LINK_CLOSEST, &p, unsafe { link_closest_point(p.ecx, p.eax, p.esi, p.eax) }, 0xFF);
}
regs_thunk!(link_closest_thunk, link_closest_entry);

// --- 0xA03C30 nav_segment_intersect_xz (ESI = A, ECX = B, EDX = C, EDI = D (vec3, x and z used), stack out (x, z);
//     AL = 1 and out written when segment A-B crosses C-D) -------------------------------------------------------

/// With `s0 = A.x - B.x`, `s1 = D.x - C.x`, `s2 = D.z - C.z` (53-bit, no f32 rounding), `det = f32(s2*s0 -
/// (A.z - B.z)*s1)`; det == 0 (ordered) gives AL = 0. `r = f32(f32(s1*(B.z - C.z) - s2*(B.x - C.x)) / det)`,
/// `q = f32(f32((B.z - C.z)*s0 - (B.x - C.x)*(A.z - B.z)) / det)`; r or q below 0 or above 1 gives AL = 0
/// (NaN passes both tests). Else out.x = f32(B.x + f32(f32(s0)*r)), out.z = f32(f32(r*f32(A.z - B.z)) + B.z),
/// EAX = (bits of out.x & ~0xFF) | 1. On failure only AL = 0 is defined (AH holds the x87 status word).
unsafe fn segment_intersect_xz(a: u32, b: u32, c: u32, d: u32, out: u32, eax_in: u32) -> u32 {
    unsafe {
        let (a, b, c, d) = (a as usize, b as usize, c as usize, d as usize);
        let low = eax_in & !0xFF;
        let f = |p: usize, k: usize| rf(p + 4 * k) as f64;
        let (ax, az, bx, bz, cx, cz, dx, dz) = (f(a, 0), f(a, 1), f(b, 0), f(b, 1), f(c, 0), f(c, 1), f(d, 0), f(d, 1));
        let s0 = ax - bx;
        let s1 = dx - cx;
        let s2 = dz - cz;
        let m1 = s2 * s0;
        let s3 = az - bz;
        let m2 = s3 * s1;
        let det = (m1 - m2) as f32;
        if det == 0.0 {
            return low;
        }
        let detd = det as f64;
        let t0 = bx - cx;
        let t1 = bz - cz;
        let n1 = (s1 * t1 - s2 * t0) as f32;
        let r = (n1 as f64 / detd) as f32;
        if r < 0.0 || 1.0 < r {
            return low;
        }
        let v = az - bz;
        let m1b = (t1 * s0 - t0 * v) as f32;
        let q = (m1b as f64 / detd) as f32;
        if q < 0.0 || 1.0 < q {
            return low;
        }
        let s0f = s0 as f32;
        let vf = v as f32;
        let w1 = (s0f as f64 * r as f64) as f32;
        let w2 = (r as f64 * vf as f64) as f32;
        let ox = (bx + w1 as f64) as f32;
        let oz = (w2 as f64 + bz) as f32;
        wf(out as usize, ox);
        wf(out as usize + 4, oz);
        (ox.to_bits() & !0xFF) | 1
    }
}

extern "C" fn segment_intersect_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let out = *args;
        r.eax = shadow_run_m(
            SEGMENT_INTERSECT,
            &p,
            args as *mut u32,
            1,
            0xFF,
            || Some(vec![(out as usize, 8)]),
            || segment_intersect_xz(p.esi, p.ecx, p.edx, p.edi, out, p.eax),
        );
    }
}
regs_thunk!(segment_intersect_thunk, segment_intersect_entry);

// --- road query callbacks and the probe (globals of the current query: 0x2FA8CC0..0x2FA8D2C) --------------------

const RQ_MASK_BITS: usize = 0x02FA_8CC0; // u32: records with any of these ctl bits are skipped
const RQ_WANT_BITS: usize = 0x02FA_8CC4; // u32: wanted ctl bits (all of them in mode 1, any of them in mode 0)
const RQ_MAX: usize = 0x02FA_8CC8; // i32: capacity of the result buffer
const RQ_MODE: usize = 0x02FA_8CCC; // u8: 1 = all wanted bits, 0 = any
const RQ_KIND: usize = 0x02FA_8CD0; // u32: required (ctl >> 26) & 3, or -1 for any
const RQ_EXTRA: usize = 0x02FA_8CD4; // u32 (an object for the 0xA8D010 test), 0 = none
const RQ_BUF: usize = 0x02FA_8D24; // u32*: result buffer
const RQ_COUNT: usize = 0x02FA_8D28; // i32: results stored
const RQ_TREE_ADD: usize = 0x0063_2C60;
const RQ_EXTRA_TEST: usize = 0x00A8_D010;
const RQ_FILTER: usize = 0x00A8_9A50;
const RQ_TREE_QUERY: usize = 0x00BF_73B0;
const RQ_TREE_CB: usize = 0x00A8_9A20;
const RQ_FIRST_HIT_CB: usize = 0x00A8_9AB0;

// --- 0xA89A20 road_query_tree_cb (stack: a, b, c vec3 pointers; AL = 1) ----------------------------------

/// `s = [a]`; 0x632C60(EAX = [s], ECX = s + 0x1C, EDX = ESI = s, stack b) then 0x632C60(EAX = [s], ECX = s + 0x22,
/// stack c) (EDX is the first call's leftover = b); AL = 1 on top of the second call's EAX.
unsafe fn query_tree_cb(a: u32, b: u32, c: u32, edi: u32) -> u32 {
    unsafe {
        let s = rd(a as usize);
        call_regs(RQ_TREE_ADD, rd(s as usize), s + 0x1C, b, s, edi, &[b]);
        let e = call_regs(RQ_TREE_ADD, rd(s as usize), s + 0x22, b, s, edi, &[c]);
        (e & !0xFF) | 1
    }
}

extern "C" fn query_tree_cb_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (a, b, c) = (*args, *args.add(1), *args.add(2));
        r.eax = shadow_run(
            QUERY_TREE_CB,
            &p,
            args as *mut u32,
            3,
            true,
            || Some(vec![(b as usize, 12), (c as usize, 12)]),
            || query_tree_cb(a, b, c, p.edi),
        );
    }
}
regs_thunk!(query_tree_cb_thunk, query_tree_cb_entry);

// --- 0xA89A50 road_query_filter (EDX = lane record; AL = 1 filtered out, 0 accepted) ------------------------------

/// `ctl = [rec + 0x10]`. Mode byte 1: ctl must contain all the wanted bits ([0x2FA8CC4]); mode 0: any of them.
/// Failing that gives AL = 1. Then ctl & [0x2FA8CC0] != 0 gives 1; the kind [0x2FA8CD0] (unless -1) must equal
/// `(ctl >> 26) & 3` (EAX holds that value from then on) else 1; with [0x2FA8CD4] non-null 0xA8D010
/// (ECX = it, stack = record) must return 0 for 1, else the result is 0. `mov al, 1` / `xor al, al`
/// keep the rest of EAX as it stands (ctl, the shifted kind, or 0xA8D010's EAX).
unsafe fn query_filter(rec: u32, esi: u32, edi: u32) -> u32 {
    unsafe {
        let mut eax = rd(rec as usize + 0x10);
        let want = rd(RQ_WANT_BITS);
        let hit = if rb(RQ_MODE) != 0 { eax & want == want } else { want & eax != 0 };
        if !hit {
            return (eax & !0xFF) | 1;
        }
        if rd(RQ_MASK_BITS) & eax != 0 {
            return (eax & !0xFF) | 1;
        }
        let kind = rd(RQ_KIND);
        if kind != 0xFFFF_FFFF {
            eax = (eax >> 26) & 3;
            if eax != kind {
                return (eax & !0xFF) | 1;
            }
        }
        let extra = rd(RQ_EXTRA);
        if extra == 0 {
            return eax & !0xFF;
        }
        let e = call_regs(RQ_EXTRA_TEST, eax, extra, rec, esi, edi, &[rec]);
        if e & 0xFF == 0 { (e & !0xFF) | 1 } else { e & !0xFF }
    }
}

extern "C" fn query_filter_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(QUERY_FILTER, &p, unsafe { query_filter(p.edx, p.esi, p.edi) });
}
regs_thunk!(query_filter_thunk, query_filter_entry);

// --- 0xA89AB0 road_query_cb_first_hit (stack: node**; AL = continue) -------------------------------------------

/// `rec = [a]`; the filter (0xA89A50, EDX = rec) saying 1 gives AL = 1 (EAX = the filter's with AL = 1).
/// Else `buf[count] = rec`, count += 1, AL = (count < max, signed) with the rest of EAX = the new count.
unsafe fn query_first_hit(a: u32, ecx: u32, esi: u32, edi: u32) -> u32 {
    unsafe {
        let rec = rd(a as usize);
        let e = call_regs(RQ_FILTER, a, ecx, rec, rec, edi, &[]);
        let _ = esi;
        if e & 0xFF != 0 {
            return (e & !0xFF) | 1;
        }
        let count = rd(RQ_COUNT);
        let buf = rd(RQ_BUF);
        wd(buf as usize + (count as usize).wrapping_mul(4), rec);
        let n = rd(RQ_COUNT).wrapping_add(1);
        let lt = (n as i32) < (rd(RQ_MAX) as i32);
        wd(RQ_COUNT, n);
        (n & !0xFF) | lt as u32
    }
}

extern "C" fn query_first_hit_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let a = *args;
        r.eax = shadow_run(
            QUERY_FIRST_HIT,
            &p,
            args as *mut u32,
            1,
            true,
            || {
                let (count, buf) = (rd(RQ_COUNT), rd(RQ_BUF));
                if buf == 0 {
                    return None;
                }
                Some(vec![(RQ_COUNT, 4), (buf as usize + (count as usize).wrapping_mul(4), 4)])
            },
            || query_first_hit(a, p.ecx, p.esi, p.edi),
        );
    }
}
regs_thunk!(query_first_hit_entry_thunk, query_first_hit_entry);

// --- 0xA8A5D0 road_network_probe (EDX = result buffer, ESI = &count out; stack: a, b, wanted bits) ----------------

/// No network ([0x2225B64] == 0) or no tree ([net + 0x84] == 0): `[ESI] = 0`, EAX = net. Else the query globals
/// are set (buffer = EDX, count = 0, wanted bits = arg2, extra = 0, max = 0x20, mask = 0x100, mode = 1,
/// kind = -1) and the tree query 0xBF73B0 runs with (EAX = tree, ECX = a, EDX = [tree + 8]; stack [tree + 8], a,
/// b, [tree], tree_cb 0xA89A20, first_hit_cb 0xA89AB0; it pops 0x18); `[ESI] = count`, EAX = count.
/// No shadow: the callbacks write the buffer and whatever the tree query touches.
unsafe fn road_network_probe(buf: u32, out: u32, args: *const u32, ecx: u32, edi: u32) -> u32 {
    unsafe {
        let net = rd(ROAD_NET);
        if net == 0 || rd(net as usize + 0x84) == 0 {
            wd(out as usize, 0);
            return net;
        }
        wd(RQ_BUF, buf);
        wd(RQ_COUNT, 0);
        wd(RQ_WANT_BITS, *args.add(2));
        wd(RQ_EXTRA, 0);
        wd(RQ_MAX, 0x20);
        wd(RQ_MASK_BITS, 0x100);
        wb(RQ_MODE, 1);
        wd(RQ_KIND, 0xFFFF_FFFF);
        let tree = rd(net as usize + 0x84);
        let root = rd(tree as usize);
        let t8 = rd(tree as usize + 8);
        let _ = ecx;
        call_regs(
            RQ_TREE_QUERY,
            tree,
            *args,
            t8,
            out,
            edi,
            &[t8, *args, *args.add(1), root, RQ_TREE_CB as u32, RQ_FIRST_HIT_CB as u32],
        );
        let c = rd(RQ_COUNT);
        wd(out as usize, c);
        c
    }
}

extern "C" fn road_network_probe_entry(r: &mut Pushad, args: *const u32) {
    PORTS[ROAD_PROBE].calls.fetch_add(1, Ordering::Relaxed);
    r.eax = unsafe { road_network_probe(r.edx, r.esi, args, r.ecx, r.edi) };
}
regs_thunk!(road_probe_thunk, road_network_probe_entry);

// --- 0xB26130 vehicle_free_path_lists (stack: vehicle) ---------------------------------------------------------

/// Walks the ring whose head slot is `vehicle + 0x458`: for each node (`next = [node]`, 0 when it equals the
/// head slot's current value, read before the recycle) road_path_node_recycle (0xB1CCD0: EAX = node, ECX = the
/// raw next, EDX = EDI = the slot, ESI = next). Then tail-jumps to nav_route_list_free (0xA18500) with
/// EAX = `vehicle + 0x45C`, ESI / EDI as the caller had them. No shadow: the recycles rewrite rings.
extern "C" fn vehicle_free_path_lists_entry(r: &mut Pushad, args: *const u32) {
    PORTS[FREE_PATH_LISTS].calls.fetch_add(1, Ordering::Relaxed);
    let p = *r;
    unsafe {
        let veh = *args;
        let slot = veh + 0x458;
        let mut eax = rd(slot as usize);
        let (mut ecx, mut edx) = (p.ecx, p.edx);
        while eax != 0 {
            ecx = rd(eax as usize);
            let esi = if ecx != rd(slot as usize) { ecx } else { 0 };
            edx = slot;
            call_regs(ROAD_PATH_RECYCLE_ADDR, eax, ecx, edx, esi, slot, &[]);
            eax = esi;
        }
        r.eax = call_regs(NAV_ROUTE_LIST_FREE, veh + 0x45C, ecx, edx, p.esi, p.edi, &[]);
    }
}
regs_thunk!(free_path_lists_thunk, vehicle_free_path_lists_entry);

// --- 0xA18570 nav_route_refill_from_list (EAX = route, EDI = &list head slot) -----------------------------------

/// Empty list (`[EDI] == 0`): nothing. Else nav_route_clear (0xA1F730, ESI = route), then up to 3 times: the
/// head node of the circular list (links +0x1C next, +0x20 prev; a single node empties the slot) is unlinked
/// and made a one-node ring, and handed to nav_route_push (0xA1F770: ESI = route, stack node, 0, 1).
/// EAX leaves 0 when the list ran dry, else the last push's.
extern "C" fn route_refill_entry(r: &mut Pushad, _: *const u32) {
    PORTS[ROUTE_REFILL].calls.fetch_add(1, Ordering::Relaxed);
    let p = *r;
    unsafe {
        let (route, slot) = (p.eax, p.edi);
        if rd(slot as usize) == 0 {
            return;
        }
        call_regs(NAV_ROUTE_CLEAR_ADDR, route, p.ecx, p.edx, route, slot, &[]);
        let mut ecx: u32;
        let mut edx = p.edx;
        let mut eax;
        let mut n = 0;
        loop {
            eax = rd(slot as usize);
            if eax == 0 {
                break;
            }
            let e = eax as usize;
            ecx = rd(e + 0x1C);
            if ecx == eax {
                wd(e + 0x1C, 0);
                wd(e + 0x20, 0);
                wd(slot as usize, 0);
            } else {
                wd(slot as usize, ecx);
                ecx = rd(e + 0x20);
                if ecx != 0 {
                    edx = rd(e + 0x1C);
                    wd(ecx as usize + 0x1C, edx);
                }
                ecx = rd(e + 0x1C);
                if ecx != 0 {
                    edx = rd(e + 0x20);
                    wd(ecx as usize + 0x20, edx);
                }
                wd(e + 0x1C, 0);
                wd(e + 0x20, 0);
            }
            wd(e + 0x1C, eax);
            wd(e + 0x20, eax);
            eax = call_regs(NAV_ROUTE_PUSH_ADDR, eax, ecx, edx, route, slot, &[eax, 0, 1]);
            n += 1;
            if n >= 3 {
                break;
            }
        }
        r.eax = eax;
    }
}
regs_thunk!(route_refill_thunk, route_refill_entry);

// --- 0xA13660 nav_path_mark_ends (ESI = &path head, EDI = request) -------------------------------------------------

/// 0xA121B0(EAX = ESI, stack EDI) runs first. With a head node: byte `[[head + 4] + 0x37]` |= 1, then the same
/// node again |= 2; when `request[+9] & 0x10` and the head is still non-null 0xA11520(EAX = request + 8, stack
/// ESI, 0, 0) runs. EAX leaves as the original's last value (0 / the marked node / the callee's).
extern "C" fn path_mark_ends_entry(r: &mut Pushad, _: *const u32) {
    PORTS[PATH_MARK_ENDS].calls.fetch_add(1, Ordering::Relaxed);
    let p = *r;
    unsafe {
        let (head_slot, req) = (p.esi, p.edi);
        call_regs(NAV_PATH_SETUP_ADDR, head_slot, p.ecx, p.edx, head_slot, req, &[req]);
        let mut eax = rd(head_slot as usize);
        if eax == 0 {
            r.eax = 0;
            return;
        }
        let n = rd(eax as usize + 4) as usize;
        wb(n + 0x37, rb(n + 0x37) | 1);
        let h = rd(head_slot as usize);
        let n2 = if h != 0 { rd(h as usize + 4) as usize } else { 0 };
        wb(n2 + 0x37, rb(n2 + 0x37) | 2);
        eax = n2 as u32;
        if rb(req as usize + 9) & 0x10 != 0 && rd(head_slot as usize) != 0 {
            eax = call_regs(NAV_PATH_FINISH_ADDR, req + 8, p.ecx, p.edx, head_slot, req, &[head_slot, 0, 0]);
        }
        r.eax = eax;
    }
}
regs_thunk!(path_mark_ends_thunk, path_mark_ends_entry);

// --- 0xB2EE60 vehicle_ai_goal_install (EAX = 13-dword block or 0; stack: object, value, u16) -------------------------

/// If `u16 [obj + 0x5C]` is non-zero 0x683FB0 runs (EDX = that value, ESI = EAX). Then `[obj + 0x24] = value`,
/// a non-null block is copied (13 dwords) to `obj + 0x28`, `u16 [obj + 0x5C] = arg2`, and 0xB2C220 runs
/// (stack obj, 0; it pops 8) with ESI / EDI as left by the copy. AL = 1 on top of its EAX.
extern "C" fn goal_install_entry(r: &mut Pushad, args: *const u32) {
    PORTS[GOAL_INSTALL].calls.fetch_add(1, Ordering::Relaxed);
    let p = *r;
    unsafe {
        let (obj, val, half) = (*args, *args.add(1), *args.add(2));
        let src = p.eax;
        let o = obj as usize;
        let old = rb(o + 0x5C) as u32 | (rb(o + 0x5D) as u32) << 8;
        if old != 0 {
            call_regs(NAV_GOAL_HOOK_ADDR, src, p.ecx, old, src, p.edi, &[]);
        }
        wd(o + 0x24, val);
        let (mut esi, mut edi, mut ecx) = (src, p.edi, p.ecx);
        if src != 0 {
            edi = obj + 0x28;
            ecx = 0xD;
            while ecx != 0 {
                wd(edi as usize, rd(esi as usize));
                esi += 4;
                edi += 4;
                ecx -= 1;
            }
        }
        ecx = (ecx & 0xFFFF_0000) | (half & 0xFFFF);
        wb(o + 0x5C, half as u8);
        wb(o + 0x5D, (half >> 8) as u8);
        let e = call_regs(NAV_GOAL_COMMIT_ADDR, val, ecx, old, esi, edi, &[obj, 0]);
        r.eax = (e & !0xFF) | 1;
    }
}
regs_thunk!(goal_install_thunk, goal_install_entry);

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
        (SPLINE_EVAL_TANGENT, &SPLINE_EVAL_TANGENT_BODY, &[], spline_eval_tangent_thunk),
        (SPLINE_EVAL_2ND, &SPLINE_EVAL_2ND_BODY, &[], spline_eval_2nd_thunk),
        (HULL_RELEASE, &HULL_RELEASE_BODY, &HULL_RELEASE_BODY_CALLS, hull_release_thunk),
        (ROAD_ROUTE_LEN, &ROAD_ROUTE_LEN_BODY, &[], road_route_len_thunk),
        (TRI_EDGE_SIDE, &TRI_EDGE_SIDE_BODY, &[], tri_edge_side_thunk),
        (RECORD_VALID, &RECORD_VALID_BODY, &[], record_valid_thunk),
        (PATH_REQ_INIT, &PATH_REQ_INIT_BODY, &[], path_request_init_thunk),
        (LANES_ADD, &LANES_ADD_BODY, &[], lanes_add_thunk),
        (BIND_CHUNK, &BIND_CHUNK_BODY, &[], bind_chunk_entries_thunk),
        (HULL_BY_HANDLE, &HULL_BY_HANDLE_BODY, &HULL_BY_HANDLE_BODY_CALLS, hull_by_handle_thunk),
        (LINK_CLOSEST, &LINK_CLOSEST_BODY, &[], link_closest_thunk),
        (SEGMENT_INTERSECT, &SEGMENT_INTERSECT_BODY, &[], segment_intersect_thunk),
        (QUERY_TREE_CB, &QUERY_TREE_CB_BODY, &QUERY_TREE_CB_BODY_CALLS, query_tree_cb_thunk),
        (QUERY_FILTER, &QUERY_FILTER_BODY, &QUERY_FILTER_BODY_CALLS, query_filter_thunk),
        (QUERY_FIRST_HIT, &QUERY_FIRST_HIT_BODY, &QUERY_FIRST_HIT_BODY_CALLS, query_first_hit_entry_thunk),
        (ROAD_PROBE, &ROAD_PROBE_BODY, &ROAD_PROBE_BODY_CALLS, road_probe_thunk),
        (FREE_PATH_LISTS, &FREE_PATH_LISTS_BODY, &FREE_PATH_LISTS_BODY_CALLS, free_path_lists_thunk),
        (ROUTE_REFILL, &ROUTE_REFILL_BODY, &ROUTE_REFILL_BODY_CALLS, route_refill_thunk),
        (PATH_MARK_ENDS, &PATH_MARK_ENDS_BODY, &PATH_MARK_ENDS_BODY_CALLS, path_mark_ends_thunk),
        (GOAL_INSTALL, &GOAL_INSTALL_BODY, &GOAL_INSTALL_BODY_CALLS, goal_install_thunk),
    ];
    let mut ok = 0;
    for (id, body, call_offsets, thunk) in list {
        let port = &PORTS[id];
        match unsafe { detour::relocate(port.addr, body, call_offsets) } {
            // nav_route_clear has no shadow. The two speed-override shadows stop before their call:
            // `add esp, 4` (undoing the push) replaces `call 0xB2B8A0`.
            Ok(_) if NO_SHADOW.contains(&id) => {}
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
const SPLINE_EVAL_TANGENT_BODY: [u8; 705] = [
    0x83, 0xEC, 0x48, 0x56, 0x8B, 0xF0, 0x8B, 0x46, 0x0C, 0x83, 0xF8, 0x02, 0x7D, 0x1F, 0xA1, 0xD0,
    0xA6, 0x52, 0x02, 0x89, 0x01, 0x8B, 0x15, 0xD4, 0xA6, 0x52, 0x02, 0x89, 0x51, 0x04, 0xA1, 0xD8,
    0xA6, 0x52, 0x02, 0x89, 0x41, 0x08, 0x5E, 0x83, 0xC4, 0x48, 0xC2, 0x04, 0x00, 0x0F, 0x57, 0xC9,
    0xF3, 0x0F, 0x10, 0x44, 0x24, 0x50, 0x0F, 0x2F, 0xC8, 0x72, 0x60, 0x8B, 0x06, 0x8B, 0x76, 0x08,
    0xD9, 0x06, 0xD8, 0x20, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x46, 0x04, 0xD8, 0x60, 0x04, 0xD9, 0x5C,
    0x24, 0x14, 0xD9, 0x46, 0x08, 0x5E, 0xD8, 0x60, 0x08, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24,
    0x0C, 0xDD, 0x05, 0xF8, 0xBF, 0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x18, 0x8B,
    0x54, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x10, 0x89, 0x11, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x1C, 0x8B,
    0x44, 0x24, 0x1C, 0x89, 0x41, 0x04, 0xD8, 0x4C, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x20, 0x8B, 0x54,
    0x24, 0x20, 0x89, 0x51, 0x08, 0x83, 0xC4, 0x48, 0xC2, 0x04, 0x00, 0xD9, 0x44, 0x24, 0x50, 0x83,
    0xC0, 0xFF, 0x89, 0x44, 0x24, 0x50, 0xDB, 0x44, 0x24, 0x50, 0xD9, 0xC9, 0xDB, 0xF1, 0xDD, 0xD9,
    0x72, 0x76, 0x8B, 0x46, 0x0C, 0xDD, 0xD8, 0x8B, 0x56, 0x04, 0x8B, 0x36, 0x8D, 0x04, 0x40, 0x03,
    0xC0, 0x03, 0xC0, 0xD9, 0x44, 0x30, 0xF4, 0x8D, 0x54, 0x02, 0xF4, 0xD8, 0x22, 0x8D, 0x44, 0x30,
    0xF4, 0x5E, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x40, 0x04, 0xD8, 0x62, 0x04, 0xD9, 0x5C, 0x24, 0x1C,
    0xD9, 0x40, 0x08, 0xD8, 0x62, 0x08, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x18, 0xDD, 0x05,
    0xF8, 0xBF, 0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x0C, 0x8B, 0x44, 0x24, 0x0C,
    0xD9, 0x44, 0x24, 0x1C, 0x89, 0x01, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x10, 0x8B, 0x54, 0x24, 0x10,
    0x89, 0x51, 0x04, 0xD8, 0x4C, 0x24, 0x20, 0xD9, 0x5C, 0x24, 0x14, 0x8B, 0x44, 0x24, 0x14, 0x89,
    0x41, 0x08, 0x83, 0xC4, 0x48, 0xC2, 0x04, 0x00, 0x57, 0xF3, 0x0F, 0x2C, 0xF8, 0x89, 0x7C, 0x24,
    0x54, 0xDA, 0x64, 0x24, 0x54, 0x8B, 0x56, 0x04, 0x8D, 0x44, 0x7F, 0x03, 0x03, 0xC0, 0x03, 0xC0,
    0x03, 0xD0, 0xD9, 0x5C, 0x24, 0x54, 0xD9, 0x44, 0x24, 0x54, 0xD9, 0xC0, 0xDC, 0xC8, 0xD9, 0x5C,
    0x24, 0x54, 0xDD, 0x05, 0x58, 0xC0, 0xE5, 0x00, 0xD8, 0xC9, 0xD9, 0x44, 0x24, 0x54, 0xDD, 0x05,
    0x58, 0xC8, 0xE5, 0x00, 0xD8, 0xC9, 0xD8, 0xC2, 0xDD, 0x05, 0xF8, 0xBF, 0xE5, 0x00, 0xDC, 0xE9,
    0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0xC1, 0xDC, 0x0D, 0x08, 0xC2, 0xE5, 0x00, 0xD9, 0xCC,
    0xDC, 0x0D, 0x28, 0xC5, 0xE5, 0x00, 0xDE, 0xC4, 0xDE, 0xC3, 0xD9, 0xCA, 0xD9, 0x5C, 0x24, 0x08,
    0xD9, 0xC1, 0xDC, 0x0D, 0x50, 0xC8, 0xE5, 0x00, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x54, 0xD9, 0xC0,
    0xDC, 0xC0, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x02, 0xD9, 0x44, 0x24, 0x54, 0xD9, 0xC0,
    0xDE, 0xCA, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x44, 0xD9, 0x42, 0x04, 0xD8, 0xC9, 0xD9, 0x5C, 0x24,
    0x48, 0xD8, 0x4A, 0x08, 0x8D, 0x14, 0x7F, 0x8B, 0x7E, 0x08, 0x8B, 0x36, 0xD9, 0x5C, 0x24, 0x4C,
    0x03, 0xD2, 0x03, 0xD2, 0xD9, 0x04, 0x17, 0x03, 0xFA, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0xC0, 0xDE,
    0xCA, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x38, 0xD9, 0x47, 0x04, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x3C,
    0xD8, 0x4F, 0x08, 0xD9, 0x5C, 0x24, 0x40, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0xC0, 0xD8, 0x0C, 0x30,
    0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x30, 0x04, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x18, 0xD8, 0x4C,
    0x30, 0x08, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x04, 0x32, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0xC0, 0xDE,
    0xCA, 0xD9, 0xC9, 0x5F, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x32, 0x04, 0xD8, 0xC9, 0xD9, 0x5C,
    0x24, 0x20, 0xD8, 0x4C, 0x32, 0x08, 0x5E, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x18, 0xD8,
    0x44, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x24, 0xD9, 0x44, 0x24, 0x1C, 0xD8, 0x44, 0x24, 0x10, 0xD9,
    0x5C, 0x24, 0x28, 0xD9, 0x44, 0x24, 0x20, 0xD8, 0x44, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x2C, 0xD9,
    0x44, 0x24, 0x24, 0xD8, 0x44, 0x24, 0x30, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x28, 0xD8,
    0x44, 0x24, 0x34, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x2C, 0xD8, 0x44, 0x24, 0x38, 0xD9,
    0x5C, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x18, 0xD8, 0x44, 0x24, 0x3C, 0xD9, 0x5C, 0x24, 0x30, 0x8B,
    0x54, 0x24, 0x30, 0xD9, 0x44, 0x24, 0x1C, 0x89, 0x11, 0xD8, 0x44, 0x24, 0x40, 0xD9, 0x5C, 0x24,
    0x34, 0x8B, 0x44, 0x24, 0x34, 0xD9, 0x44, 0x24, 0x20, 0x89, 0x41, 0x04, 0xD8, 0x44, 0x24, 0x44,
    0xD9, 0x5C, 0x24, 0x38, 0x8B, 0x54, 0x24, 0x38, 0x89, 0x51, 0x08, 0x83, 0xC4, 0x48, 0xC2, 0x04,
    0x00,
];
const SPLINE_EVAL_2ND_BODY: [u8; 833] = [
    0x8B, 0x41, 0x0C, 0x83, 0xEC, 0x48, 0x83, 0xF8, 0x02, 0x7D, 0x1E, 0xA1, 0xD0, 0xA6, 0x52, 0x02,
    0x89, 0x02, 0x8B, 0x0D, 0xD4, 0xA6, 0x52, 0x02, 0x89, 0x4A, 0x04, 0xA1, 0xD8, 0xA6, 0x52, 0x02,
    0x89, 0x42, 0x08, 0x83, 0xC4, 0x48, 0xC2, 0x04, 0x00, 0x0F, 0x57, 0xC9, 0xF3, 0x0F, 0x10, 0x44,
    0x24, 0x4C, 0x0F, 0x2F, 0xC8, 0x0F, 0x86, 0xA9, 0x00, 0x00, 0x00, 0x8B, 0x41, 0x08, 0xD9, 0x00,
    0xDD, 0x05, 0xE8, 0xBF, 0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x40,
    0x04, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x10, 0xD8, 0x48, 0x08, 0x8B, 0x01, 0x8B, 0x49, 0x04, 0x83,
    0xC1, 0x0C, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x00, 0xD8, 0x64, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x18,
    0xD9, 0x40, 0x04, 0xD8, 0x64, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x40, 0x08, 0xD8, 0x64,
    0x24, 0x14, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x01, 0xD8, 0x44, 0x24, 0x18, 0xD9, 0x5C, 0x24, 0x0C,
    0xD9, 0x41, 0x04, 0xD8, 0x44, 0x24, 0x1C, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x41, 0x08, 0xD8, 0x44,
    0x24, 0x20, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x44, 0x24, 0x0C, 0xDD, 0x05, 0x58, 0xC0, 0xE5, 0x00,
    0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x18, 0x8B, 0x4C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x10,
    0x89, 0x0A, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x1C, 0x8B, 0x44, 0x24, 0x1C, 0x89, 0x42, 0x04, 0xD8,
    0x4C, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x20, 0x8B, 0x4C, 0x24, 0x20, 0x89, 0x4A, 0x08, 0x83, 0xC4,
    0x48, 0xC2, 0x04, 0x00, 0xD9, 0x44, 0x24, 0x4C, 0x83, 0xC0, 0xFF, 0x89, 0x44, 0x24, 0x4C, 0xDB,
    0x44, 0x24, 0x4C, 0x56, 0xD9, 0xC9, 0x57, 0xDB, 0xF1, 0xDD, 0xD9, 0x0F, 0x82, 0xC5, 0x00, 0x00,
    0x00, 0x8B, 0x41, 0x0C, 0xDD, 0xD8, 0x8B, 0x79, 0x04, 0x8D, 0x34, 0x40, 0x03, 0xF6, 0x03, 0xF6,
    0xD9, 0x44, 0x37, 0xF4, 0x8D, 0x7C, 0x37, 0xF4, 0xDD, 0x05, 0xE8, 0xBF, 0xE5, 0x00, 0x8D, 0x44,
    0x40, 0xFA, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x47, 0x04, 0xD8, 0xC9, 0xD9,
    0x5C, 0x24, 0x24, 0xD8, 0x4F, 0x08, 0x8B, 0x79, 0x08, 0x8B, 0x09, 0x8D, 0x04, 0x87, 0xD9, 0x5C,
    0x24, 0x28, 0x5F, 0xD9, 0x00, 0xD8, 0x64, 0x24, 0x1C, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x40, 0x04,
    0xD8, 0x64, 0x24, 0x20, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x40, 0x08, 0x8D, 0x44, 0x0E, 0xF4, 0xD8,
    0x64, 0x24, 0x24, 0x5E, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x00, 0xD8, 0x44, 0x24, 0x0C, 0xD9, 0x5C,
    0x24, 0x18, 0xD9, 0x40, 0x04, 0xD8, 0x44, 0x24, 0x10, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x40, 0x08,
    0xD8, 0x44, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x18, 0xDD, 0x05, 0x58, 0xC0,
    0xE5, 0x00, 0xDC, 0xC9, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x0C, 0x8B, 0x44, 0x24, 0x0C, 0xD9, 0x44,
    0x24, 0x1C, 0x89, 0x02, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x10, 0x8B, 0x4C, 0x24, 0x10, 0x89, 0x4A,
    0x04, 0xD8, 0x4C, 0x24, 0x20, 0xD9, 0x5C, 0x24, 0x14, 0x8B, 0x44, 0x24, 0x14, 0x89, 0x42, 0x08,
    0x83, 0xC4, 0x48, 0xC2, 0x04, 0x00, 0x53, 0xF3, 0x0F, 0x2C, 0xD8, 0x89, 0x5C, 0x24, 0x58, 0xDA,
    0x64, 0x24, 0x58, 0x8B, 0x31, 0x8B, 0x79, 0x04, 0x8D, 0x44, 0x5B, 0x03, 0x03, 0xC0, 0x03, 0xC0,
    0x8B, 0x49, 0x08, 0x03, 0xF8, 0xD9, 0x5C, 0x24, 0x58, 0xD9, 0x44, 0x24, 0x58, 0xDD, 0x05, 0x00,
    0xC5, 0xE5, 0x00, 0xD8, 0xC9, 0xDD, 0x05, 0x58, 0xC0, 0xE5, 0x00, 0xDC, 0xC1, 0xD9, 0xC9, 0xD9,
    0x5C, 0x24, 0x14, 0xD9, 0xC1, 0xDC, 0x0D, 0xF8, 0xC2, 0xE5, 0x00, 0xDC, 0x25, 0x30, 0xC5, 0xE5,
    0x00, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0xC1, 0xDC, 0x0D, 0x48, 0xC8, 0xE5, 0x00, 0xD8, 0xC1, 0xD9,
    0x5C, 0x24, 0x0C, 0xDE, 0xC9, 0xD9, 0x5C, 0x24, 0x58, 0xD9, 0x04, 0x30, 0xD9, 0x44, 0x24, 0x58,
    0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x48, 0xD9, 0x44, 0x30, 0x04, 0xD8, 0xC9,
    0xD9, 0x5C, 0x24, 0x4C, 0xD8, 0x4C, 0x30, 0x08, 0x8D, 0x04, 0x5B, 0x03, 0xC0, 0x03, 0xC0, 0xD9,
    0x5C, 0x24, 0x50, 0x03, 0xC8, 0xD9, 0x07, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9,
    0xC9, 0xD9, 0x5C, 0x24, 0x3C, 0xD9, 0x47, 0x04, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x40, 0xD8, 0x4F,
    0x08, 0xD9, 0x5C, 0x24, 0x44, 0xD9, 0x01, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9,
    0xC9, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x41, 0x04, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x1C, 0xD8, 0x49,
    0x08, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x04, 0x30, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0xC0, 0xDE, 0xCA,
    0xD9, 0xC9, 0xD9, 0x5C, 0x24, 0x24, 0xD9, 0x44, 0x30, 0x04, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x28,
    0xD8, 0x4C, 0x30, 0x08, 0xD9, 0x5C, 0x24, 0x2C, 0xD9, 0x44, 0x24, 0x24, 0xD8, 0x44, 0x24, 0x18,
    0xD9, 0x5C, 0x24, 0x30, 0xD9, 0x44, 0x24, 0x28, 0xD8, 0x44, 0x24, 0x1C, 0x5B, 0x5F, 0x5E, 0xD9,
    0x5C, 0x24, 0x28, 0xD9, 0x44, 0x24, 0x20, 0xD8, 0x44, 0x24, 0x14, 0xD9, 0x5C, 0x24, 0x2C, 0xD9,
    0x44, 0x24, 0x24, 0xD8, 0x44, 0x24, 0x30, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x44, 0x24, 0x28, 0xD8,
    0x44, 0x24, 0x34, 0xD9, 0x5C, 0x24, 0x1C, 0xD9, 0x44, 0x24, 0x2C, 0xD8, 0x44, 0x24, 0x38, 0xD9,
    0x5C, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x18, 0xD8, 0x44, 0x24, 0x3C, 0xD9, 0x5C, 0x24, 0x30, 0x8B,
    0x4C, 0x24, 0x30, 0xD9, 0x44, 0x24, 0x1C, 0x89, 0x0A, 0xD8, 0x44, 0x24, 0x40, 0xD9, 0x5C, 0x24,
    0x34, 0x8B, 0x44, 0x24, 0x34, 0xD9, 0x44, 0x24, 0x20, 0x89, 0x42, 0x04, 0xD8, 0x44, 0x24, 0x44,
    0xD9, 0x5C, 0x24, 0x38, 0x8B, 0x4C, 0x24, 0x38, 0x89, 0x4A, 0x08, 0x83, 0xC4, 0x48, 0xC2, 0x04,
    0x00,
];
const HULL_RELEASE_BODY: [u8; 288] = [
    0x8B, 0x47, 0x18, 0x53, 0x55, 0x33, 0xED, 0x3B, 0xC5, 0x56, 0x0F, 0x84, 0x0C, 0x01, 0x00, 0x00,
    0xF6, 0x40, 0x08, 0x01, 0x74, 0x67, 0x8B, 0x1D, 0xC0, 0xB8, 0x1F, 0x02, 0x3B, 0xDD, 0x8B, 0xF3,
    0x74, 0x32, 0x8B, 0x4E, 0x2C, 0x8B, 0xC1, 0x3B, 0xC5, 0x74, 0x1D, 0x8B, 0x57, 0x08, 0x8B, 0xFF,
    0x39, 0x50, 0x08, 0x74, 0x0C, 0x8B, 0x00, 0x3B, 0xC1, 0x74, 0x0D, 0x3B, 0xC5, 0x75, 0xF1, 0xEB,
    0x07, 0x8B, 0xD6, 0xE8, 0x98, 0xCE, 0xFF, 0xFF, 0x8B, 0x06, 0x3B, 0xC3, 0x74, 0x06, 0x8B, 0xF0,
    0x3B, 0xF5, 0x75, 0xCE, 0x8B, 0x47, 0x18, 0x39, 0x05, 0x90, 0xC1, 0x1F, 0x02, 0x75, 0x55, 0x39,
    0x00, 0x75, 0x10, 0x89, 0x28, 0x8B, 0x47, 0x18, 0x89, 0x68, 0x04, 0x89, 0x2D, 0x90, 0xC1, 0x1F,
    0x02, 0xEB, 0x66, 0x8B, 0x08, 0x89, 0x0D, 0x90, 0xC1, 0x1F, 0x02, 0xEB, 0x37, 0x8B, 0x70, 0x54,
    0x3B, 0xF5, 0x74, 0x0C, 0x55, 0xE8, 0xC6, 0xCE, 0xFF, 0xFF, 0x8B, 0x47, 0x18, 0x83, 0xC4, 0x04,
    0x39, 0x05, 0x98, 0xC1, 0x1F, 0x02, 0x75, 0x1C, 0x39, 0x00, 0x75, 0x10, 0x89, 0x28, 0x8B, 0x57,
    0x18, 0x89, 0x2D, 0x98, 0xC1, 0x1F, 0x02, 0x89, 0x6A, 0x04, 0xEB, 0x2D, 0x8B, 0x08, 0x89, 0x0D,
    0x98, 0xC1, 0x1F, 0x02, 0x8B, 0x48, 0x04, 0x3B, 0xCD, 0x74, 0x04, 0x8B, 0x10, 0x89, 0x11, 0x8B,
    0x47, 0x18, 0x39, 0x28, 0x74, 0x08, 0x8B, 0x08, 0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0x8B, 0x47,
    0x18, 0x89, 0x28, 0x8B, 0x4F, 0x18, 0x89, 0x69, 0x04, 0xA1, 0x94, 0xC1, 0x1F, 0x02, 0x3B, 0xC5,
    0x75, 0x1B, 0x8B, 0x47, 0x18, 0x89, 0x00, 0x8B, 0x47, 0x18, 0x89, 0x40, 0x04, 0x8B, 0x57, 0x18,
    0x89, 0x15, 0x94, 0xC1, 0x1F, 0x02, 0x89, 0x6F, 0x18, 0x5E, 0x5D, 0x5B, 0xC3, 0x8B, 0x4F, 0x18,
    0x8B, 0x50, 0x04, 0x89, 0x51, 0x04, 0x8B, 0x4F, 0x18, 0x89, 0x01, 0x8B, 0x50, 0x04, 0x8B, 0x4F,
    0x18, 0x89, 0x0A, 0x8B, 0x57, 0x18, 0x89, 0x50, 0x04, 0x89, 0x6F, 0x18, 0x5E, 0x5D, 0x5B, 0xC3,
];
const HULL_RELEASE_BODY_CALLS: [usize; 2] = [0x43, 0x85];
const ROAD_ROUTE_LEN_BODY: [u8; 100] = [
    0x83, 0xEC, 0x08, 0x85, 0xF6, 0xD9, 0xEE, 0x53, 0xD9, 0x5C, 0x24, 0x04, 0x8A, 0x5C, 0x24, 0x10,
    0x8B, 0xC6, 0x74, 0x47, 0xDD, 0x05, 0x70, 0xBF, 0xE5, 0x00, 0x8B, 0x48, 0x0C, 0x8B, 0x51, 0x10,
    0xF7, 0xC2, 0x00, 0x00, 0x00, 0x20, 0x75, 0x27, 0x80, 0xFB, 0x01, 0xD9, 0x41, 0x14, 0xD9, 0x5C,
    0x24, 0x08, 0x75, 0x0F, 0xF6, 0xC2, 0x10, 0x74, 0x0A, 0xD9, 0x44, 0x24, 0x08, 0xD8, 0xC9, 0xD9,
    0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xD8, 0x44, 0x24, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0x8B,
    0x00, 0x3B, 0xC6, 0x74, 0x04, 0x85, 0xC0, 0x75, 0xC1, 0xDD, 0xD8, 0xD9, 0x44, 0x24, 0x04, 0x5B,
    0x83, 0xC4, 0x08, 0xC3,
];
const TRI_EDGE_SIDE_BODY: [u8; 186] = [
    0x8B, 0x0C, 0x86, 0x83, 0xC0, 0x01, 0x83, 0xEC, 0x10, 0x99, 0x53, 0xBB, 0x03, 0x00, 0x00, 0x00,
    0xF7, 0xFB, 0x32, 0xC0, 0x8B, 0x14, 0x96, 0x3B, 0xCA, 0x76, 0x08, 0x8B, 0xC1, 0x8B, 0xCA, 0x8B,
    0xD0, 0xB0, 0x01, 0xD9, 0x42, 0x08, 0x84, 0xC0, 0xD8, 0x61, 0x08, 0xD9, 0x5C, 0x24, 0x0C, 0xD9,
    0x01, 0xD8, 0x22, 0xD9, 0x5C, 0x24, 0x10, 0xD9, 0x07, 0xD8, 0x21, 0xD9, 0x5C, 0x24, 0x04, 0xD9,
    0x47, 0x04, 0xD8, 0x61, 0x08, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x08, 0xD8, 0x4C, 0x24,
    0x10, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0x4C, 0x24, 0x0C, 0xDE, 0xC1, 0xD9, 0x5C, 0x24, 0x04, 0xD9,
    0x44, 0x24, 0x04, 0xDC, 0x15, 0xA0, 0xC1, 0xE5, 0x00, 0xDF, 0xE0, 0x74, 0x28, 0xF6, 0xC4, 0x41,
    0x75, 0x0C, 0xDD, 0xD8, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x5B, 0x83, 0xC4, 0x10, 0xC3, 0xDC, 0x1D,
    0x30, 0xC4, 0xE5, 0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x28, 0xB8, 0x02, 0x00, 0x00, 0x00,
    0x5B, 0x83, 0xC4, 0x10, 0xC3, 0xF6, 0xC4, 0x41, 0x75, 0x0C, 0xDD, 0xD8, 0xB8, 0x02, 0x00, 0x00,
    0x00, 0x5B, 0x83, 0xC4, 0x10, 0xC3, 0xDC, 0x1D, 0x30, 0xC4, 0xE5, 0x00, 0xDF, 0xE0, 0xF6, 0xC4,
    0x05, 0x7B, 0xC1, 0x33, 0xC0, 0x5B, 0x83, 0xC4, 0x10, 0xC3,
];
const RECORD_VALID_BODY: [u8; 38] = [
    0x81, 0xE9, 0xD0, 0x00, 0xE4, 0x02, 0x78, 0x1B, 0x81, 0xF9, 0x00, 0x68, 0x00, 0x00, 0x73, 0x13,
    0xB8, 0x4F, 0xEC, 0xC4, 0x4E, 0xF7, 0xE1, 0xC1, 0xEA, 0x04, 0x6B, 0xD2, 0x34, 0x2B, 0xCA, 0x0F,
    0x94, 0xC0, 0xC3, 0x32, 0xC0, 0xC3,
];
const PATH_REQ_INIT_BODY: [u8; 129] = [
    0xD9, 0xEE, 0x33, 0xC0, 0x66, 0x89, 0x41, 0x08, 0x88, 0x41, 0x0A, 0x80, 0x61, 0x09, 0xBF, 0x8B,
    0x44, 0x24, 0x04, 0x89, 0x01, 0x8B, 0x44, 0x24, 0x08, 0x89, 0x41, 0x04, 0x66, 0x8B, 0x02, 0x66,
    0x89, 0x41, 0x08, 0x8A, 0x52, 0x02, 0x88, 0x51, 0x0A, 0xF6, 0x41, 0x09, 0x80, 0x74, 0x13, 0x8B,
    0x06, 0x89, 0x41, 0x0C, 0x8B, 0x56, 0x04, 0x89, 0x51, 0x10, 0x8B, 0x46, 0x08, 0x89, 0x41, 0x14,
    0xEB, 0x09, 0xD9, 0x51, 0x0C, 0xD9, 0x51, 0x10, 0xD9, 0x51, 0x14, 0xD9, 0x44, 0x24, 0x0C, 0xD9,
    0x51, 0x24, 0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x75, 0x18, 0x8B, 0x17, 0xDD, 0xD8, 0x89,
    0x51, 0x18, 0x8B, 0x47, 0x04, 0x89, 0x41, 0x1C, 0x8B, 0x57, 0x08, 0x89, 0x51, 0x20, 0x8B, 0xC1,
    0xC2, 0x0C, 0x00, 0xD9, 0x51, 0x18, 0x8B, 0xC1, 0xD9, 0x51, 0x1C, 0xD9, 0x59, 0x20, 0xC2, 0x0C,
    0x00,
];
const LANES_ADD_BODY: [u8; 403] = [
    0x53, 0x33, 0xDB, 0x39, 0x18, 0x57, 0x0F, 0x8E, 0xB3, 0x00, 0x00, 0x00, 0x8D, 0x64, 0x24, 0x00,
    0x8B, 0x38, 0x3B, 0x7C, 0x24, 0x0C, 0x0F, 0x8D, 0xA3, 0x00, 0x00, 0x00, 0x8B, 0x0C, 0x9E, 0x0F,
    0xB7, 0x49, 0x36, 0x66, 0x81, 0xF9, 0xFF, 0xFF, 0x74, 0x3A, 0x0F, 0xB7, 0xC9, 0x8B, 0xD1, 0xC1,
    0xE2, 0x04, 0x2B, 0xD1, 0x8B, 0x0D, 0x64, 0x5B, 0x22, 0x02, 0x8B, 0x49, 0x04, 0x8D, 0x14, 0x91,
    0x85, 0xD2, 0x74, 0x20, 0x33, 0xC9, 0x85, 0xFF, 0x7E, 0x12, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x14, 0x8E, 0x74, 0x0F, 0x83, 0xC1, 0x01, 0x3B, 0xCF, 0x7C, 0xF4, 0x89, 0x14, 0xBE, 0x83,
    0xC7, 0x01, 0x89, 0x38, 0x8B, 0x0C, 0x9E, 0xF6, 0x41, 0x10, 0x01, 0x75, 0x47, 0x0F, 0xB7, 0x49,
    0x38, 0x66, 0x81, 0xF9, 0xFF, 0xFF, 0x74, 0x3C, 0x0F, 0xB7, 0xC9, 0x8B, 0xD1, 0xC1, 0xE2, 0x04,
    0x2B, 0xD1, 0x8B, 0x0D, 0x64, 0x5B, 0x22, 0x02, 0x8B, 0x49, 0x04, 0x8D, 0x14, 0x91, 0x85, 0xD2,
    0x74, 0x22, 0x8B, 0x38, 0x3B, 0x7C, 0x24, 0x0C, 0x7D, 0x1A, 0x33, 0xC9, 0x85, 0xFF, 0x7E, 0x0C,
    0x39, 0x14, 0x8E, 0x74, 0x0F, 0x83, 0xC1, 0x01, 0x3B, 0xCF, 0x7C, 0xF4, 0x89, 0x14, 0xBE, 0x83,
    0xC7, 0x01, 0x89, 0x38, 0x83, 0xC3, 0x01, 0x3B, 0x18, 0x0F, 0x8C, 0x51, 0xFF, 0xFF, 0xFF, 0x80,
    0x7C, 0x24, 0x10, 0x00, 0x0F, 0x84, 0xB9, 0x00, 0x00, 0x00, 0x8B, 0x18, 0x55, 0x33, 0xED, 0x85,
    0xDB, 0x7E, 0x58, 0x8B, 0x38, 0x3B, 0x7C, 0x24, 0x10, 0x7D, 0x50, 0x8B, 0x0C, 0xAE, 0xF6, 0x41,
    0x10, 0x01, 0x74, 0x40, 0x0F, 0xB7, 0x49, 0x38, 0x66, 0x81, 0xF9, 0xFF, 0xFF, 0x74, 0x35, 0x0F,
    0xB7, 0xC9, 0x8B, 0xD1, 0xC1, 0xE2, 0x04, 0x2B, 0xD1, 0x8B, 0x0D, 0x64, 0x5B, 0x22, 0x02, 0x8B,
    0x49, 0x04, 0x8D, 0x14, 0x91, 0x85, 0xD2, 0x74, 0x1B, 0x33, 0xC9, 0x85, 0xFF, 0x7E, 0x0D, 0x90,
    0x39, 0x14, 0x8E, 0x74, 0x0F, 0x83, 0xC1, 0x01, 0x3B, 0xCF, 0x7C, 0xF4, 0x89, 0x14, 0xBE, 0x83,
    0xC7, 0x01, 0x89, 0x38, 0x83, 0xC5, 0x01, 0x3B, 0xEB, 0x7C, 0xA8, 0x3B, 0x18, 0x8B, 0xEB, 0x7D,
    0x51, 0x8B, 0x38, 0x3B, 0x7C, 0x24, 0x10, 0x7D, 0x49, 0x8B, 0x0C, 0xAE, 0x0F, 0xB7, 0x49, 0x36,
    0x66, 0x81, 0xF9, 0xFF, 0xFF, 0x74, 0x34, 0x0F, 0xB7, 0xC9, 0x8B, 0xD1, 0xC1, 0xE2, 0x04, 0x2B,
    0xD1, 0x8B, 0x0D, 0x64, 0x5B, 0x22, 0x02, 0x8B, 0x49, 0x04, 0x8D, 0x14, 0x91, 0x85, 0xD2, 0x74,
    0x1A, 0x33, 0xC9, 0x85, 0xFF, 0x7E, 0x0C, 0x39, 0x14, 0x8E, 0x74, 0x0F, 0x83, 0xC1, 0x01, 0x3B,
    0xCF, 0x7C, 0xF4, 0x89, 0x14, 0xBE, 0x83, 0xC7, 0x01, 0x89, 0x38, 0x83, 0xC5, 0x01, 0x3B, 0x28,
    0x7C, 0xAF, 0x5D, 0x8B, 0x4C, 0x24, 0x0C, 0x33, 0xD2, 0x39, 0x08, 0x5F, 0x0F, 0x9D, 0xC2, 0x8A,
    0xC2, 0x5B, 0xC3,
];
const BIND_CHUNK_BODY: [u8; 169] = [
    0x83, 0xEC, 0x08, 0x8B, 0x4C, 0x24, 0x14, 0x8B, 0x81, 0x48, 0x01, 0x00, 0x00, 0x8B, 0x89, 0x44,
    0x01, 0x00, 0x00, 0x53, 0x8B, 0x5C, 0x24, 0x14, 0x33, 0xD2, 0x3B, 0xC2, 0x55, 0x8B, 0x6C, 0x24,
    0x14, 0x89, 0x4C, 0x24, 0x0C, 0x76, 0x7C, 0x3B, 0xDA, 0x7E, 0x78, 0x3B, 0xCA, 0x89, 0x54, 0x24,
    0x08, 0x7E, 0x70, 0x56, 0x57, 0x8B, 0xF8, 0x3B, 0xD3, 0x8B, 0xC2, 0x7D, 0x50, 0x8B, 0x37, 0x8D,
    0x0C, 0x92, 0x8D, 0x4C, 0x8D, 0x04, 0x39, 0x31, 0x74, 0x0C, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x14,
    0x3B, 0xC3, 0x7C, 0xF2, 0xEB, 0x37, 0x3B, 0xC3, 0x7D, 0x33, 0x8D, 0x0C, 0x80, 0x39, 0x74, 0x8D,
    0x04, 0x8D, 0x4C, 0x8D, 0x00, 0x75, 0x26, 0x83, 0x79, 0x08, 0xFF, 0x75, 0x12, 0x8B, 0x54, 0x24,
    0x24, 0x66, 0x8B, 0x52, 0x08, 0x66, 0x89, 0x11, 0x8B, 0x54, 0x24, 0x10, 0x89, 0x51, 0x08, 0x8B,
    0x51, 0x0C, 0x89, 0x57, 0x04, 0x8B, 0x49, 0x10, 0x89, 0x4F, 0x08, 0x8B, 0xD0, 0x8B, 0x44, 0x24,
    0x10, 0x83, 0xC0, 0x01, 0x83, 0xC7, 0x28, 0x3B, 0x44, 0x24, 0x14, 0x89, 0x44, 0x24, 0x10, 0x7C,
    0x96, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x08, 0xC3,
];
const HULL_BY_HANDLE_BODY: [u8; 39] = [
    0x83, 0xF9, 0xFF, 0x57, 0x74, 0x1F, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0x8D, 0x3C, 0x80,
    0x39, 0x0C, 0xFD, 0xC0, 0xE0, 0xDE, 0x02, 0x8D, 0x3C, 0xFD, 0xB8, 0xE0, 0xDE, 0x02, 0x75, 0x05,
    0xE8, 0xDB, 0xCC, 0xFF, 0xFF, 0x5F, 0xC3,
];
const HULL_BY_HANDLE_BODY_CALLS: [usize; 1] = [0x20];
const LINK_CLOSEST_BODY: [u8; 399] = [
    0x8A, 0x51, 0x18, 0x83, 0xEC, 0x34, 0xF6, 0xC2, 0x02, 0x0F, 0x84, 0x93, 0x00, 0x00, 0x00, 0xF6,
    0xC2, 0x01, 0xD9, 0x41, 0x0C, 0xD8, 0x21, 0xD9, 0x5C, 0x24, 0x28, 0xD9, 0x41, 0x10, 0xD8, 0x61,
    0x04, 0xD9, 0x5C, 0x24, 0x2C, 0xD9, 0x41, 0x14, 0xD8, 0x61, 0x08, 0xD9, 0x5C, 0x24, 0x10, 0xD9,
    0x44, 0x24, 0x28, 0xD9, 0xE0, 0xD9, 0x5C, 0x24, 0x18, 0xD9, 0x00, 0xD8, 0x21, 0x0F, 0x84, 0xAC,
    0x00, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x40, 0x04, 0xD8, 0x61, 0x04, 0xD9, 0x5C, 0x24,
    0x08, 0xD9, 0x40, 0x08, 0xD8, 0x61, 0x08, 0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x04, 0xD9,
    0x44, 0x24, 0x10, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0x44, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x2C, 0xD9,
    0xC0, 0xDE, 0xCA, 0xD9, 0xCB, 0xDE, 0xC1, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x18, 0xD9,
    0xC0, 0xDE, 0xCA, 0xD9, 0xCA, 0xDE, 0xC1, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD9, 0xEE, 0xD8,
    0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x41, 0x75, 0x0E, 0xDD, 0xDA, 0xDD, 0xDA, 0xDD, 0xD9,
    0xDD, 0xD8, 0x32, 0xC0, 0x83, 0xC4, 0x34, 0xC3, 0xD9, 0x06, 0xD8, 0x21, 0xD9, 0x5C, 0x24, 0x04,
    0xD9, 0x46, 0x04, 0xD8, 0x61, 0x04, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x46, 0x08, 0xD8, 0x61, 0x08,
    0xD9, 0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x04, 0xDE, 0xCB, 0xD9, 0x44, 0x24, 0x08, 0xDE, 0xCC,
    0xD9, 0xCA, 0xDE, 0xC3, 0xD8, 0x4C, 0x24, 0x0C, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9, 0x1C, 0x24, 0xD8,
    0x1C, 0x24, 0xDF, 0xE0, 0xF6, 0xC4, 0x41, 0x7B, 0xB9, 0xB0, 0x01, 0x83, 0xC4, 0x34, 0xC3, 0xD9,
    0x5C, 0x24, 0x1C, 0xD9, 0x40, 0x04, 0xD8, 0x61, 0x04, 0xD9, 0x5C, 0x24, 0x20, 0xD9, 0x40, 0x08,
    0xD8, 0x61, 0x08, 0xD9, 0x5C, 0x24, 0x24, 0xD9, 0x06, 0xD8, 0x21, 0xD9, 0x5C, 0x24, 0x04, 0xD9,
    0x46, 0x04, 0xD8, 0x61, 0x04, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x46, 0x08, 0xD8, 0x61, 0x08, 0xD9,
    0x5C, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x10, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9,
    0x44, 0x24, 0x08, 0xD9, 0x44, 0x24, 0x2C, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xCB, 0xDE, 0xC1, 0xD9,
    0x44, 0x24, 0x0C, 0xD9, 0x44, 0x24, 0x18, 0xD9, 0xC0, 0xDE, 0xCA, 0xD9, 0xCA, 0xDE, 0xC1, 0xD9,
    0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD9, 0x44, 0x24, 0x1C, 0xDE, 0xCB, 0xD9, 0x44, 0x24, 0x20, 0xDE,
    0xCC, 0xD9, 0xCA, 0xDE, 0xC3, 0xD8, 0x4C, 0x24, 0x24, 0xDE, 0xC2, 0xD9, 0xC9, 0xD9, 0x1C, 0x24,
    0xD8, 0x0C, 0x24, 0xDC, 0x1D, 0xA0, 0xBF, 0xE5, 0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7A, 0x09,
    0xB8, 0x01, 0x00, 0x00, 0x00, 0x83, 0xC4, 0x34, 0xC3, 0x33, 0xC0, 0x83, 0xC4, 0x34, 0xC3,
];
const SEGMENT_INTERSECT_BODY: [u8; 281] = [
    0x83, 0xEC, 0x10, 0xD9, 0x06, 0x53, 0xD8, 0x21, 0x8B, 0x5C, 0x24, 0x18, 0xD9, 0x07, 0xD8, 0x22,
    0xD9, 0x47, 0x04, 0xD8, 0x62, 0x04, 0xD8, 0xCA, 0xD9, 0x46, 0x04, 0xD8, 0x61, 0x04, 0xD8, 0xCA,
    0xDE, 0xE9, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0xEE, 0xD9, 0xC0, 0xD9, 0x44, 0x24, 0x04, 0xDD, 0xE1,
    0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x44, 0x0F, 0x8B, 0xCD, 0x00, 0x00, 0x00, 0xD9, 0x01, 0xD8,
    0x22, 0xD9, 0x41, 0x04, 0xD8, 0x62, 0x04, 0xDE, 0xCC, 0xD9, 0x47, 0x04, 0xD8, 0x62, 0x04, 0xD8,
    0xC9, 0xDE, 0xEC, 0xD9, 0xCB, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0xF1, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0xD2, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x0F, 0x8B,
    0x94, 0x00, 0x00, 0x00, 0xD9, 0xE8, 0xD8, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x0F, 0x8B, 0x85,
    0x00, 0x00, 0x00, 0xD9, 0x41, 0x04, 0xD8, 0x62, 0x04, 0xD8, 0xCD, 0xD9, 0x46, 0x04, 0xD8, 0x61,
    0x04, 0xDE, 0xCD, 0xDE, 0xE4, 0xD9, 0xCB, 0xD9, 0x5C, 0x24, 0x04, 0xD8, 0x7C, 0x24, 0x04, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x44, 0x24, 0x04, 0xD8, 0xD1, 0xDF, 0xE0, 0xDD, 0xD9, 0xF6, 0xC4, 0x05,
    0x7B, 0x5A, 0xD9, 0xE8, 0xDE, 0xD9, 0xDF, 0xE0, 0xF6, 0xC4, 0x05, 0x7B, 0x51, 0xD9, 0xC9, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x46, 0x04, 0xD8, 0x61, 0x04, 0xD9, 0x5C, 0x24, 0x08, 0xD9, 0x44, 0x24,
    0x04, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x0C, 0xD8, 0x4C, 0x24, 0x08, 0xD9, 0x5C, 0x24, 0x10, 0xD9,
    0x01, 0xD8, 0x44, 0x24, 0x0C, 0xD9, 0x5C, 0x24, 0x04, 0x8B, 0x44, 0x24, 0x04, 0xD9, 0x44, 0x24,
    0x10, 0xD8, 0x41, 0x04, 0x89, 0x03, 0xB0, 0x01, 0xD9, 0x5C, 0x24, 0x08, 0x8B, 0x4C, 0x24, 0x08,
    0x89, 0x4B, 0x04, 0x5B, 0x83, 0xC4, 0x10, 0xC3, 0xDD, 0xDB, 0xDD, 0xDA, 0xDD, 0xD8, 0xDD, 0xD8,
    0x32, 0xC0, 0xDD, 0xD8, 0x5B, 0x83, 0xC4, 0x10, 0xC3,
];
const QUERY_TREE_CB_BODY: [u8; 44] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x54, 0x24, 0x08, 0x56, 0x8B, 0x30, 0x8B, 0x06, 0x8D, 0x4E, 0x1C,
    0x52, 0xE8, 0x2A, 0x92, 0xBA, 0xFF, 0x8B, 0x44, 0x24, 0x14, 0x50, 0x8B, 0x06, 0x8D, 0x4E, 0x22,
    0xE8, 0x1B, 0x92, 0xBA, 0xFF, 0x83, 0xC4, 0x08, 0xB0, 0x01, 0x5E, 0xC3,
];
const QUERY_TREE_CB_BODY_CALLS: [usize; 2] = [0x11, 0x20];
const QUERY_FILTER_BODY: [u8; 94] = [
    0x80, 0x3D, 0xCC, 0x8C, 0xFA, 0x02, 0x00, 0x8B, 0x42, 0x10, 0x74, 0x13, 0x8B, 0x0D, 0xC4, 0x8C,
    0xFA, 0x02, 0x56, 0x8B, 0xF0, 0x23, 0xF1, 0x3B, 0xF1, 0x5E, 0x74, 0x0B, 0xB0, 0x01, 0xC3, 0x85,
    0x05, 0xC4, 0x8C, 0xFA, 0x02, 0x74, 0xF5, 0x85, 0x05, 0xC0, 0x8C, 0xFA, 0x02, 0x75, 0xED, 0x8B,
    0x0D, 0xD0, 0x8C, 0xFA, 0x02, 0x83, 0xF9, 0xFF, 0x74, 0x0A, 0xC1, 0xE8, 0x1A, 0x83, 0xE0, 0x03,
    0x3B, 0xC1, 0x75, 0xD8, 0x8B, 0x0D, 0xD4, 0x8C, 0xFA, 0x02, 0x85, 0xC9, 0x74, 0x0D, 0x52, 0xE8,
    0x6C, 0x35, 0x00, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x74, 0xC1, 0x32, 0xC0, 0xC3,
];
const QUERY_FILTER_BODY_CALLS: [usize; 1] = [0x4f];
const QUERY_FIRST_HIT_BODY: [u8; 65] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x30, 0x8B, 0xD6, 0xE8, 0x92, 0xFF, 0xFF, 0xFF, 0x84, 0xC0,
    0x74, 0x04, 0xB0, 0x01, 0x5E, 0xC3, 0x8B, 0x0D, 0x28, 0x8D, 0xFA, 0x02, 0x8B, 0x15, 0x24, 0x8D,
    0xFA, 0x02, 0x89, 0x34, 0x8A, 0xA1, 0x28, 0x8D, 0xFA, 0x02, 0x83, 0xC0, 0x01, 0x33, 0xC9, 0x3B,
    0x05, 0xC8, 0x8C, 0xFA, 0x02, 0xA3, 0x28, 0x8D, 0xFA, 0x02, 0x0F, 0x9C, 0xC1, 0x8A, 0xC1, 0x5E,
    0xC3,
];
const QUERY_FIRST_HIT_BODY_CALLS: [usize; 1] = [0x9];
const ROAD_PROBE_BODY: [u8; 133] = [
    0xA1, 0x64, 0x5B, 0x22, 0x02, 0x33, 0xC9, 0x3B, 0xC1, 0x74, 0x77, 0x39, 0x88, 0x84, 0x00, 0x00,
    0x00, 0x74, 0x6F, 0x89, 0x15, 0x24, 0x8D, 0xFA, 0x02, 0x8B, 0x54, 0x24, 0x0C, 0x89, 0x0D, 0x28,
    0x8D, 0xFA, 0x02, 0x89, 0x15, 0xC4, 0x8C, 0xFA, 0x02, 0x8B, 0x54, 0x24, 0x08, 0x89, 0x0D, 0xD4,
    0x8C, 0xFA, 0x02, 0x68, 0xB0, 0x9A, 0xA8, 0x00, 0xC7, 0x05, 0xC8, 0x8C, 0xFA, 0x02, 0x20, 0x00,
    0x00, 0x00, 0xC7, 0x05, 0xC0, 0x8C, 0xFA, 0x02, 0x00, 0x01, 0x00, 0x00, 0xC6, 0x05, 0xCC, 0x8C,
    0xFA, 0x02, 0x01, 0xC7, 0x05, 0xD0, 0x8C, 0xFA, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x8B, 0x80, 0x84,
    0x00, 0x00, 0x00, 0x8B, 0x08, 0x68, 0x20, 0x9A, 0xA8, 0x00, 0x51, 0x8B, 0x4C, 0x24, 0x10, 0x52,
    0x8B, 0x50, 0x08, 0x51, 0x52, 0xE8, 0x66, 0xCD, 0x16, 0x00, 0xA1, 0x28, 0x8D, 0xFA, 0x02, 0x89,
    0x06, 0xC3, 0x89, 0x0E, 0xC3,
];
const ROAD_PROBE_BODY_CALLS: [usize; 1] = [0x75];
const FREE_PATH_LISTS_BODY: [u8; 60] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x8B, 0x83, 0x58, 0x04, 0x00, 0x00, 0x56, 0x57, 0x8D, 0xBB, 0x58,
    0x04, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x17, 0x8B, 0x08, 0x8B, 0xF1, 0x2B, 0x37, 0x8B, 0xD7, 0xF7,
    0xDE, 0x1B, 0xF6, 0x23, 0xF1, 0xE8, 0x76, 0x6B, 0xFF, 0xFF, 0x8B, 0xC6, 0xEB, 0xE5, 0x5F, 0x5E,
    0x8D, 0x83, 0x5C, 0x04, 0x00, 0x00, 0x5B, 0xE9, 0x94, 0x23, 0xEF, 0xFF,
];
const FREE_PATH_LISTS_BODY_CALLS: [usize; 1] = [0x25];
const ROUTE_REFILL_BODY: [u8; 105] = [
    0x55, 0x33, 0xED, 0x39, 0x2F, 0x56, 0x8B, 0xF0, 0x74, 0x5C, 0x53, 0xE8, 0xB0, 0x71, 0x00, 0x00,
    0x33, 0xDB, 0x8B, 0x07, 0x3B, 0xC5, 0x74, 0x4D, 0x8B, 0x48, 0x1C, 0x3B, 0xC8, 0x75, 0x0A, 0x89,
    0x68, 0x1C, 0x89, 0x68, 0x20, 0x89, 0x2F, 0xEB, 0x22, 0x89, 0x0F, 0x8B, 0x48, 0x20, 0x3B, 0xCD,
    0x74, 0x06, 0x8B, 0x50, 0x1C, 0x89, 0x51, 0x1C, 0x8B, 0x48, 0x1C, 0x3B, 0xCD, 0x74, 0x06, 0x8B,
    0x50, 0x20, 0x89, 0x51, 0x20, 0x89, 0x68, 0x1C, 0x89, 0x68, 0x20, 0x6A, 0x01, 0x55, 0x50, 0x89,
    0x40, 0x1C, 0x89, 0x40, 0x20, 0xE8, 0xA6, 0x71, 0x00, 0x00, 0x83, 0xC3, 0x01, 0x83, 0xC4, 0x0C,
    0x83, 0xFB, 0x03, 0x7C, 0xAD, 0x5B, 0x5E, 0x5D, 0xC3,
];
const ROUTE_REFILL_BODY_CALLS: [usize; 2] = [0xb, 0x55];
const PATH_MARK_ENDS_BODY: [u8; 71] = [
    0x51, 0x57, 0x8B, 0xC6, 0xE8, 0x47, 0xEB, 0xFF, 0xFF, 0x8B, 0x06, 0x83, 0xC4, 0x04, 0x85, 0xC0,
    0x74, 0x33, 0x8B, 0x40, 0x04, 0x80, 0x48, 0x37, 0x01, 0x8B, 0x06, 0x85, 0xC0, 0x74, 0x05, 0x8B,
    0x40, 0x04, 0xEB, 0x02, 0x33, 0xC0, 0x80, 0x48, 0x37, 0x02, 0xF6, 0x47, 0x09, 0x10, 0x74, 0x15,
    0x83, 0x3E, 0x00, 0x74, 0x10, 0x6A, 0x00, 0x6A, 0x00, 0x8D, 0x47, 0x08, 0x56, 0xE8, 0x7E, 0xDE,
    0xFF, 0xFF, 0x83, 0xC4, 0x0C, 0x59, 0xC3,
];
const PATH_MARK_ENDS_BODY_CALLS: [usize; 2] = [0x4, 0x3d];
const GOAL_INSTALL_BODY: [u8; 69] = [
    0x51, 0x53, 0x8B, 0x5C, 0x24, 0x0C, 0x0F, 0xB7, 0x53, 0x5C, 0x66, 0x85, 0xD2, 0x56, 0x57, 0x8B,
    0xF0, 0x74, 0x05, 0xE8, 0x38, 0x51, 0xB5, 0xFF, 0x85, 0xF6, 0x8B, 0x44, 0x24, 0x18, 0x89, 0x43,
    0x24, 0x74, 0x0A, 0x8D, 0x7B, 0x28, 0xB9, 0x0D, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0x66, 0x8B, 0x4C,
    0x24, 0x1C, 0x6A, 0x00, 0x53, 0x66, 0x89, 0x4B, 0x5C, 0xE8, 0x82, 0xD3, 0xFF, 0xFF, 0x5F, 0x5E,
    0xB0, 0x01, 0x5B, 0x59, 0xC3,
];
const GOAL_INSTALL_BODY_CALLS: [usize; 2] = [0x13, 0x39];
