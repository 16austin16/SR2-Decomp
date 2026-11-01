//! Port module: sixteen small leaves of the simulation subtree (doc 41, "Port candidates"),
//! checked against the disassembly.
//!
//! These have tens to hundreds of LTCG callers each, so every port goes through a `pushad`
//! thunk: all registers come back as the caller left them except EAX, which carries the result
//! (`vec3_distance_sq` preserves every register, EAX too; its result is on ST0). That is a
//! superset of what each original preserves. Where an original returns AL only, the upper bytes
//! of EAX are reproduced as the original leaves them (the caller's, a handle, a node pointer, the
//! x87 status word...), so the whole of EAX is the same.
//!
//! Shadow check: for the first `SHADOW_CALLS` calls of each port a relocated copy of the
//! original runs on the same seven registers (and stack argument), all seven are captured, and
//! its EAX / ST0 / memory writes are compared with the Rust version's; on a mismatch the
//! original's are kept. The two that write state (the ragdoll cache, `timer_set_random`) run on
//! a snapshot that is restored in between. `parked_vehicles_do_frame` only decides whether to run
//! five calls: its copy runs with those calls pointed at recording stubs, and the decision is
//! compared. The shadow also logs if an original changes a register it was found to preserve.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

const HANDLES: usize = 0x0214_9C64;
const LOCAL_PLAYER: usize = 0x0217_03D4;
const GAME_NOW: usize = 0x00E8_43AC;
const RAND_INDEX: usize = 0x00EC_3ABC;
const RAND_TABLE: u32 = 0x00EC_3AC0;
const RAGDOLL_CACHE: usize = 0x00ED_7428;
const WRAP: u32 = 0x6B49_D200; // 1,800,000,000 ms
const HALF: i32 = 0x35A4_E900; // 900,000,000 ms

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

#[repr(C)]
struct Block {
    f: u32,
    r: Regs,
    arg: u32,
    esp: u32,
    st0: f64,
    want_st0: u32,
}

/// Call `f` with all seven registers from `r` and one stack argument (harmless for the callees
/// that take none; a `ret 4` callee pops it), then capture all seven registers into `r` and, when
/// `want_st0`, pop ST0 (returned).
unsafe fn call_full(f: usize, r: &mut Regs, arg: u32, want_st0: bool) -> f64 {
    let mut b = Block { f: f as u32, r: *r, arg, esp: 0, st0: 0.0, want_st0: want_st0 as u32 };
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
    b.st0
}

/// A naked thunk: `pushad`, then `f(&mut block, &stack args)`, then `popad`. The Rust body puts
/// its result in the block's EAX; an `f32` return stays on ST0.
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

const EQ_LOCAL: usize = 0;
const DEAD_OR_INVALID: usize = 1;
const NON_SCRIPTED: usize = 2;
const ACTIVE: usize = 3;
const RIDER_FIND: usize = 4;
const ENTERING: usize = 5;
const EXITING: usize = 6;
const SEAT_OCCUPANT: usize = 7;
const VELOCITY: usize = 8;
const STATIC_CONTACT: usize = 9;
const WHEEL_CONTACT: usize = 10;
const RAGDOLL_FIND: usize = 11;
const ELAPSED: usize = 12;
const SET_RANDOM: usize = 13;
const DIST_SQ: usize = 14;
const PARKED: usize = 15;
const COUNT: usize = 16;

static PORTS: [Port; COUNT] = [
    port("human_equals_local_player", 0x0092_6650, ECX | EDX | SAVED),
    port("human_is_dead_or_invalid", 0x0098_55F0, EDX | SAVED),
    port("human_from_handle_non_scripted", 0x009D_06A0, EDX | SAVED),
    port("human_from_handle_active", 0x0043_8710, SAVED),
    port("vehicle_interaction_rider_find", 0x00B0_7A30, SAVED),
    port("human_vehicle_interaction_entering", 0x009B_9320, SAVED),
    port("human_vehicle_interaction_exiting", 0x009B_9350, SAVED),
    port("vehicle_seat_occupant_by_index", 0x00AA_7380, EDX | SAVED),
    port("vehicle_get_velocity", 0x00AA_68E0, SAVED),
    port("object_has_static_contact", 0x00AA_B5E0, EDX | SAVED),
    port("vehicle_any_wheel_in_contact", 0x00AA_B6B0, SAVED),
    port("ragdoll_find_for_object", 0x0071_0DB0, SAVED),
    port("timer_elapsed_ms", 0x00BD_D2A0, SAVED),
    port("timer_set_random", 0x00BD_D1B0, SAVED),
    port("vec3_distance_sq", 0x00BD_EA30, EAX | ECX | EDX | SAVED),
    port("parked_vehicles_do_frame", 0x0094_8D30, ECX | SAVED),
];

/// Count a call; `Some((call number, original copy))` while it should be shadowed.
fn shadowing(id: usize) -> Option<(u32, usize)> {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_sim_leaves: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    (orig != 0 && n < SHADOW_CALLS).then_some((n, orig))
}

fn mismatch(id: usize, n: u32, what: String) {
    let m = PORTS[id].mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!("port_sim_leaves: {} mismatch (call {n}): {what}", PORTS[id].name));
    }
}

static PRESERVE_LOGS: AtomicU32 = AtomicU32::new(0);

/// Log when the original changed a register it was found to preserve (the thunks preserve all).
fn check_preserved(id: usize, inp: &Regs, out: &Regs) {
    for i in 0..7 {
        if PORTS[id].preserved & (1 << i) != 0 && inp.get(i) != out.get(i) && PRESERVE_LOGS.fetch_add(1, Ordering::Relaxed) < 10 {
            log(&format!(
                "port_sim_leaves: {}: the original changed {} ({:#x} -> {:#x})",
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
    unsafe { call_full(orig, &mut r, arg, false) };
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

// --- shared helpers -------------------------------------------------------------------------------

/// The object of a handle (slot table 0x2149C64, 16-byte entries), or 0 when 0 / stale.
unsafe fn resolve(h: u32) -> u32 {
    if h == 0 {
        return 0;
    }
    let o = unsafe { rd(HANDLES + (h as usize & 0xFFF) * 16) };
    if o != 0 && unsafe { rd(o as usize + 0x44) } == h { o } else { 0 }
}

/// The type-1 (human) object of a handle, or 0.
unsafe fn resolve_human(h: u32) -> u32 {
    let o = unsafe { resolve(h) };
    if o != 0 && unsafe { rd(o as usize + 0x4C) } == 1 { o } else { 0 }
}

/// A `fld dword` / `fstp dword` round trip: the bits, except that a signalling NaN is quieted.
fn q32(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

/// EAX with AL replaced by a bool.
fn lo(eax: u32, al: bool) -> u32 {
    (eax & !0xFF) | al as u32
}

// --- 0x926650 human_equals_local_player (EAX = human -> AL) --------------------------------------

/// AL = human != 0 && human == the local player (0x21703D4); the rest of EAX is the caller's.
unsafe fn eq_local(eax: u32) -> u32 {
    lo(eax, eax != 0 && eax == unsafe { rd(LOCAL_PLAYER) })
}

extern "C" fn eq_local_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(EQ_LOCAL, &p, 0, unsafe { eq_local(p.eax) });
}
regs_thunk!(eq_local_thunk, eq_local_entry);

// --- 0x9855F0 human_is_dead_or_invalid (EDX = human -> AL) ---------------------------------------

/// `fldz; fcomp dword [p]; fnstsw ax`, exactly as the original (it also leaves the same x87
/// exception flags, e.g. invalid for a NaN).
unsafe fn fcomp_zero(p: usize) -> u16 {
    let sw: u16;
    unsafe {
        std::arch::asm!("fldz", "fcomp dword ptr [{p}]", "fnstsw ax", p = in(reg) p, out("ax") sw, clobber_abi("C"));
    }
    sw
}

/// 1 for a null human or one whose handle (+0x44) does not resolve to it; otherwise 0 when
/// 0.0 < hit points (+0x1124) or unordered (a NaN counts as alive), else 1. EAX: the caller's
/// upper bytes for a null human; the handle's for a stale one; else the handle's upper half and
/// the x87 status word's high byte in AH.
unsafe fn dead_or_invalid(eax: u32, edx: u32) -> u32 {
    unsafe {
        if edx == 0 {
            return lo(eax, true);
        }
        let h = rd(edx as usize + 0x44);
        if resolve(h) != edx {
            return lo(h, true);
        }
        let sw = fcomp_zero(edx as usize + 0x1124) as u32;
        (h & 0xFFFF_0000) | (sw & 0xFF00) | (sw & 0x100 == 0) as u32
    }
}

extern "C" fn dead_or_invalid_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(DEAD_OR_INVALID, &p, 0, unsafe { dead_or_invalid(p.eax, p.edx) });
}
regs_thunk!(dead_or_invalid_thunk, dead_or_invalid_entry);

// --- 0x9D06A0 human_from_handle_non_scripted (ECX = handle -> EAX) -------------------------------

/// The live human of the handle only when its kind byte +0x3CC is 2 (`setne cl; sub ecx, 1;
/// and eax, ecx` keeps EAX for 2 and clears it otherwise).
unsafe fn non_scripted(h: u32) -> u32 {
    unsafe {
        let o = resolve_human(h);
        if o != 0 && rb(o as usize + 0x3CC) == 2 { o } else { 0 }
    }
}

extern "C" fn non_scripted_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(NON_SCRIPTED, &p, 0, unsafe { non_scripted(p.ecx) });
}
regs_thunk!(non_scripted_thunk, non_scripted_entry);

// --- 0x438710 human_from_handle_active (EAX = handle, cdecl (u8 require_alive) -> EAX) ----------

const HUMAN_IS_DEAD: usize = 0x0098_5630; // cdecl (human) -> AL

/// The live human of the handle when +0x80 has none of 0xB, (if asked) human_is_dead says no,
/// +0x80 bit 4 is clear (the +0x80 read from before that call) and +0xC8 has 0x400000.
unsafe fn active(h: u32, require_alive: u32) -> u32 {
    unsafe {
        let o = resolve_human(h);
        if o == 0 {
            return 0;
        }
        let f = rd(o as usize + 0x80);
        if f & 0xB != 0 {
            return 0;
        }
        if require_alive as u8 != 0 && call_regs(HUMAN_IS_DEAD, 0, 0, 0, 0, 0, &[o]) as u8 != 0 {
            return 0;
        }
        if f & 0x10 != 0 || rd(o as usize + 0xC8) & 0x40_0000 == 0 {
            return 0;
        }
        o
    }
}

extern "C" fn active_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let arg = unsafe { *args };
    r.eax = checked_eax(ACTIVE, &p, arg, unsafe { active(p.eax, arg) });
}
regs_thunk!(active_thunk, active_entry);

// --- 0xB07A30 vehicle_interaction_rider_find (ESI = human handle -> EAX) -------------------------

/// The rider slot (interaction record +0x10E0, slots at +0x34C, 0xA8 apart, 8 of them) whose
/// first dword is the handle, or 0.
unsafe fn rider_find(h: u32) -> u32 {
    unsafe {
        let o = resolve_human(h);
        if o == 0 {
            return 0;
        }
        let rec = rd(o as usize + 0x10E0);
        if rec == 0 {
            return 0;
        }
        for i in 0..8u32 {
            let s = rec.wrapping_add(0x34C + i * 0xA8);
            if rd(s as usize) == h {
                return s;
            }
        }
        0
    }
}

extern "C" fn rider_find_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(RIDER_FIND, &p, 0, unsafe { rider_find(p.esi) });
}
regs_thunk!(rider_find_thunk, rider_find_entry);

// --- 0x9B9320 / 0x9B9350 human_vehicle_interaction_entering / _exiting (cdecl human -> AL) ------

/// AL = the human's rider slot (rider_find(+0x44)) is in `state` (+4); EAX is 0 without a slot,
/// else the state dword with AL replaced.
unsafe fn interaction_state_is(h: u32, state: u32) -> u32 {
    unsafe {
        let s = rider_find(rd(h as usize + 0x44));
        if s == 0 {
            return 0;
        }
        let v = rd(s as usize + 4);
        lo(v, v == state)
    }
}

extern "C" fn entering_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let h = unsafe { *args };
    r.eax = checked_eax(ENTERING, &p, h, unsafe { interaction_state_is(h, 1) });
}
regs_thunk!(entering_thunk, entering_entry);

extern "C" fn exiting_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let h = unsafe { *args };
    r.eax = checked_eax(EXITING, &p, h, unsafe { interaction_state_is(h, 2) });
}
regs_thunk!(exiting_thunk, exiting_entry);

// --- 0xAA7380 vehicle_seat_occupant_by_index (ECX = vehicle, EDX = seat -> EAX) -----------------

const VEHICLE_SEAT_OCCUPANT: usize = 0x0096_0D60; // EAX = handle -> EAX human (the tail jump)

/// Seat count +0x6CE8 for a full vehicle (+0x84D8 == 1; 0 seats gives 0), else 8; a seat at or
/// past it (signed) gives 0; else the human of the handle at +0x8ABC + seat * 0x24.
unsafe fn seat_occupant(v: u32, seat: u32) -> u32 {
    unsafe {
        if v == 0 {
            return 0;
        }
        let n = if rb(v as usize + 0x84D8) == 1 {
            let n = rd(v as usize + 0x6CE8);
            if n == 0 {
                return 0;
            }
            n
        } else {
            8
        };
        if seat as i32 >= n as i32 {
            return 0;
        }
        resolve_human(rd(v.wrapping_add(seat.wrapping_mul(0x24)).wrapping_add(0x8ABC) as usize))
    }
}

extern "C" fn seat_occupant_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(SEAT_OCCUPANT, &p, 0, unsafe { seat_occupant(p.ecx, p.edx) });
}
regs_thunk!(seat_occupant_thunk, seat_occupant_entry);

// --- 0xAA68E0 vehicle_get_velocity (ECX = vehicle, cdecl (out) -> EAX = out) --------------------

const ZERO_VECTOR: usize = 0x0252_A6D0;

/// A full vehicle: the three f32 at physics body (+0x554) +0x1A0, through x87 (a signalling NaN
/// is quieted); otherwise the zero vector 0x252A6D0 copied as dwords. EAX = out.
unsafe fn velocity(v: u32, out: u32) -> u32 {
    unsafe {
        let o = out as usize;
        if rb(v as usize + 0x84D8) == 1 {
            let b = rd(v as usize + 0x554) as usize + 0x1A0;
            let s = [rd(b), rd(b + 4), rd(b + 8)];
            for (i, &x) in s.iter().enumerate() {
                wd(o + i * 4, q32(x));
            }
        } else {
            for i in 0..3 {
                wd(o + i * 4, rd(ZERO_VECTOR + i * 4));
            }
        }
        out
    }
}

/// Shadow: the original and the Rust version each write into a scratch vector; the chosen one is
/// then copied to the caller's.
extern "C" fn velocity_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let out = unsafe { *args };
    let Some((n, orig)) = shadowing(VELOCITY) else {
        r.eax = unsafe { velocity(p.ecx, out) };
        return;
    };
    let mut theirs = [0xDEAD_BEEFu32; 4];
    let mut ours = [0xDEAD_BEEFu32; 4];
    let t = run_original(VELOCITY, orig, &p, theirs.as_mut_ptr() as u32);
    let e = unsafe { velocity(p.ecx, ours.as_mut_ptr() as u32) };
    if theirs != ours || t.eax != theirs.as_ptr() as u32 || e != ours.as_ptr() as u32 {
        mismatch(VELOCITY, n, format!("vehicle {:#x}: original {theirs:x?}, Rust {ours:x?}", p.ecx));
        ours = theirs;
    }
    unsafe { copy_dwords(out as usize, ours.as_ptr() as usize, 3) };
    r.eax = out;
}
regs_thunk!(velocity_thunk, velocity_entry);

// --- 0xAAB5E0 object_has_static_contact (EAX = object -> AL) -------------------------------------

/// Walk the contact list 0x175CD84[handle & 0xFFF] (8-byte entries; next +0x10, circular): true
/// at the first node whose other object (+0) is 0. EAX: that node (AL = 1), the head when the
/// walk came round (AL = 0), or 0.
unsafe fn static_contact(obj: u32) -> u32 {
    unsafe {
        let h = rd(obj as usize + 0x44) & 0xFFF;
        let first = rd(0x0175_CD84 + h as usize * 8);
        let mut n = first;
        while n != 0 {
            if rd(n as usize) == 0 {
                return lo(n, true);
            }
            n = rd(n as usize + 0x10);
            if n == first {
                return lo(n, false);
            }
        }
        0
    }
}

extern "C" fn static_contact_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(STATIC_CONTACT, &p, 0, unsafe { static_contact(p.eax) });
}
regs_thunk!(static_contact_thunk, static_contact_entry);

// --- 0xAAB6B0 vehicle_any_wheel_in_contact (EDI = vehicle -> AL) ---------------------------------

/// hk_custom_vehicle_instance vtable +0x40 (and the motorcycle / airplane ones): `+0xE0[i]`.
const WHEEL_QUERY: u32 = 0x00AF_B4C0;

/// False for a null vehicle, class 3 / 4, a non-full one or no part list (+0x6CE0). Otherwise
/// the parts i (count at +0, pointers from +4; list and count re-read every step) not flagged
/// 0x600000 at +0x11C are asked `[+0x53C]->vtbl[0x40](i)` (thiscall) until one is not -1; AL =
/// the walk stopped early. EAX keeps the original's upper bytes (the caller's, the class, the
/// list or the last query result).
unsafe fn wheel_contact(eax: u32, v: u32) -> u32 {
    unsafe {
        if v == 0 {
            return lo(eax, false);
        }
        let class = rd(rd(v as usize + 0x84E4) as usize + 0x9C);
        if class == 3 || class == 4 || rb(v as usize + 0x84D8) != 1 {
            return lo(class, false);
        }
        let list = rd(v as usize + 0x6CE0);
        if list == 0 {
            return 0;
        }
        let mut a = list;
        let mut i = 0u32;
        if rd(list as usize) != 0 {
            loop {
                let part = rd(a.wrapping_add(i.wrapping_mul(4)).wrapping_add(4) as usize);
                if rd(part as usize + 0x11C) & 0x60_0000 == 0 {
                    let obj = rd(v as usize + 0x53C);
                    let vt = rd(obj as usize);
                    let f = rd(vt as usize + 0x40);
                    a = call_regs(f as usize, f, obj, vt, i, v, &[i]);
                    if a != u32::MAX {
                        break;
                    }
                }
                a = rd(v as usize + 0x6CE0);
                i += 1;
                if i >= rd(a as usize) {
                    break;
                }
            }
        }
        lo(a, i != rd(rd(v as usize + 0x6CE0) as usize))
    }
}

/// The shadow calls the virtual query twice, so only when it is the known pure getter (or is
/// never reached).
unsafe fn wheel_shadow_safe(v: u32) -> bool {
    unsafe {
        if v == 0 {
            return true;
        }
        let class = rd(rd(v as usize + 0x84E4) as usize + 0x9C);
        if class == 3 || class == 4 || rb(v as usize + 0x84D8) != 1 {
            return true;
        }
        let list = rd(v as usize + 0x6CE0);
        if list == 0 || rd(list as usize) == 0 {
            return true;
        }
        let obj = rd(v as usize + 0x53C);
        obj != 0 && rd(rd(obj as usize) as usize + 0x40) == WHEEL_QUERY
    }
}

extern "C" fn wheel_contact_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    if unsafe { wheel_shadow_safe(p.edi) } {
        r.eax = checked_eax(WHEEL_CONTACT, &p, 0, unsafe { wheel_contact(p.eax, p.edi) });
    } else {
        PORTS[WHEEL_CONTACT].calls.fetch_add(1, Ordering::Relaxed);
        r.eax = unsafe { wheel_contact(p.eax, p.edi) };
    }
}
regs_thunk!(wheel_contact_thunk, wheel_contact_entry);

// --- 0x710DB0 ragdoll_find_for_object (ECX = object -> EAX) --------------------------------------

const RAGDOLLS: u32 = 0x0296_00F0; // 12 records of 0x12A0, handle at +0xF70
const RAGDOLL_SIZE: u32 = 0x12A0;

/// The cached index (0xED7428, used only when > 0) if its record's handle matches; else a scan
/// of the 12 records, caching the hit (or -1 on a miss).
unsafe fn ragdoll_find(obj: u32) -> u32 {
    unsafe {
        let c = rd(RAGDOLL_CACHE);
        if c as i32 > 0 {
            let a = c.wrapping_mul(RAGDOLL_SIZE);
            if rd(a.wrapping_add(RAGDOLLS + 0xF70) as usize) == rd(obj as usize + 0x44) {
                return a.wrapping_add(RAGDOLLS);
            }
        }
        let h = rd(obj as usize + 0x44);
        for i in 0..12u32 {
            if rd((RAGDOLLS + 0xF70 + i * RAGDOLL_SIZE) as usize) == h {
                wd(RAGDOLL_CACHE, i);
                return RAGDOLLS + i * RAGDOLL_SIZE;
            }
        }
        wd(RAGDOLL_CACHE, u32::MAX);
        0
    }
}

extern "C" fn ragdoll_find_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let Some((n, orig)) = shadowing(RAGDOLL_FIND) else {
        r.eax = unsafe { ragdoll_find(p.ecx) };
        return;
    };
    unsafe {
        let before = rd(RAGDOLL_CACHE);
        let t = run_original(RAGDOLL_FIND, orig, &p, 0);
        let theirs = (t.eax, rd(RAGDOLL_CACHE));
        wd(RAGDOLL_CACHE, before);
        let ours = (ragdoll_find(p.ecx), rd(RAGDOLL_CACHE));
        if ours != theirs {
            mismatch(RAGDOLL_FIND, n, format!("object {:#x} cache {before:#x}: original {theirs:x?}, Rust {ours:x?}", p.ecx));
            wd(RAGDOLL_CACHE, theirs.1);
            r.eax = theirs.0;
        } else {
            r.eax = ours.0;
        }
    }
}
regs_thunk!(ragdoll_find_thunk, ragdoll_find_entry);

// --- 0xBDD2A0 timer_elapsed_ms (EAX = &timestamp -> EAX) -----------------------------------------

/// Signed ms from the stamp to now (0xE843AC) on the 1,800,000,000 ms clock, a gap over
/// 900,000,000 taken the other way round; an unset stamp (< 0) gives -1,800,000,000.
unsafe fn elapsed(p: u32) -> u32 {
    let t = unsafe { rd(p as usize) } as i32;
    if t < 0 {
        return WRAP.wrapping_neg();
    }
    let n = unsafe { rd(GAME_NOW) } as i32;
    let w = WRAP as i32;
    let r = if t > n {
        let d = t.wrapping_sub(n);
        if d <= HALF { d.wrapping_neg() } else { d.wrapping_sub(w).wrapping_neg() }
    } else {
        let d = n.wrapping_sub(t);
        if d <= HALF { d } else { t.wrapping_sub(n).wrapping_add(w).wrapping_neg() }
    };
    r as u32
}

extern "C" fn elapsed_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(ELAPSED, &p, 0, unsafe { elapsed(p.eax) });
}
regs_thunk!(elapsed_thunk, elapsed_entry);

// --- 0xBDD1B0 timer_set_random (EDI = &timestamp, EAX = max, stdcall (min) -> EAX) --------------

/// `v % d` with the original's unsigned `div` (a zero divisor faults the same way).
fn udiv_rem(v: u32, d: u32) -> u32 {
    let rem: u32;
    unsafe {
        std::arch::asm!("div {d}", d = in(reg) d, inout("eax") v => _, inout("edx") 0u32 => rem, options(nomem, nostack));
    }
    rem
}

/// The shared random table (0xEC3AC0, index 0xEC3ABC wrapping at 0x800) gives v; r = min + v %
/// (max - min + 1); ts = now + r, wrapped down past 1,800,000,000 (r >= 0) or up below 0 (r <
/// 0). EAX = the stored stamp.
unsafe fn set_random(max: u32, ts: u32, min: u32) -> u32 {
    unsafe {
        let k = rd(RAND_INDEX);
        let v = rd(RAND_TABLE.wrapping_add(k.wrapping_mul(4)) as usize);
        let k1 = k.wrapping_add(1);
        wd(RAND_INDEX, if (k1 as i32) < 0x800 { k1 } else { 0 });
        let r = udiv_rem(v, max.wrapping_sub(min).wrapping_add(1)).wrapping_add(min);
        let mut t = rd(GAME_NOW).wrapping_add(r);
        if r as i32 >= 0 {
            if t as i32 > WRAP as i32 {
                t = t.wrapping_sub(WRAP);
            }
        } else if (t as i32) < 0 {
            t = t.wrapping_add(WRAP);
        }
        wd(ts as usize, t);
        t
    }
}

/// Shadow on a snapshot of the stamp and the table index.
extern "C" fn set_random_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let min = unsafe { *args };
    let Some((n, orig)) = shadowing(SET_RANDOM) else {
        r.eax = unsafe { set_random(p.eax, p.edi, min) };
        return;
    };
    unsafe {
        let ts = p.edi as usize;
        let before = (rd(RAND_INDEX), rd(ts));
        let t = run_original(SET_RANDOM, orig, &p, min);
        let theirs = (t.eax, rd(RAND_INDEX), rd(ts));
        wd(RAND_INDEX, before.0);
        wd(ts, before.1);
        let e = set_random(p.eax, p.edi, min);
        let ours = (e, rd(RAND_INDEX), rd(ts));
        if ours != theirs {
            mismatch(SET_RANDOM, n, format!("max {:#x} min {min:#x} before {before:x?}: original {theirs:x?}, Rust {ours:x?}", p.eax));
            wd(RAND_INDEX, theirs.1);
            wd(ts, theirs.2);
            r.eax = theirs.0;
        } else {
            r.eax = e;
        }
    }
}
regs_thunk!(set_random_thunk, set_random_entry, "ret 4");

// --- 0xBDEA30 vec3_distance_sq (ECX = &a, EAX = &b -> ST0) ---------------------------------------

const M52: u64 = (1 << 52) - 1;

fn snan32(b: u32) -> bool {
    b & 0x7FC0_0000 == 0x7F80_0000 && b & 0x003F_FFFF != 0
}

/// Two NaN operands on the x87: the larger significand; a tie goes to the positive one.
fn nan_pick(x: f64, y: f64) -> f64 {
    let (mx, my) = (x.to_bits() & M52, y.to_bits() & M52);
    if mx > my || (mx == my && x.is_sign_positive()) { x } else { y }
}

/// `fsub` / `fadd dword` with ST0 already loaded (so quiet): both NaN gives ST0 for a signalling
/// m32, else the x87 pick; otherwise the f64 result (one NaN propagates, quieted).
fn x87_m32(st: f64, m: u32, sub: bool) -> f64 {
    let y = f32::from_bits(m) as f64;
    if st.is_nan() && y.is_nan() {
        return if snan32(m) { st } else { nan_pick(st, y) };
    }
    if sub { st - y } else { st + y }
}

/// Per axis d = f32(a - b); acc = f32(d0 * d0), then f32(d * d + acc), at 53-bit precision.
unsafe fn dist_sq(a: u32, b: u32) -> f32 {
    unsafe {
        let mut acc = 0f32;
        for i in 0..3 {
            let ai = f32::from_bits(rd(a as usize + i * 4)) as f64;
            let d = x87_m32(ai, rd(b as usize + i * 4), true) as f32;
            let sq = d as f64 * d as f64;
            acc = if i == 0 { sq as f32 } else { x87_m32(sq, acc.to_bits(), false) as f32 };
        }
        acc
    }
}

extern "C" fn dist_sq_entry(r: &mut Pushad, _: *const u32) -> f32 {
    let p = *r;
    let ours = unsafe { dist_sq(p.ecx, p.eax) };
    let Some((n, orig)) = shadowing(DIST_SQ) else { return ours };
    let inp = Regs::of(&p);
    let mut out = inp;
    let theirs = unsafe { call_full(orig, &mut out, 0, true) } as f32;
    check_preserved(DIST_SQ, &inp, &out);
    if theirs.to_bits() != ours.to_bits() {
        mismatch(DIST_SQ, n, format!("a {:#x} b {:#x}: original {:#x}, Rust {:#x}", p.ecx, p.eax, theirs.to_bits(), ours.to_bits()));
        return theirs;
    }
    ours
}
regs_thunk!(dist_sq_thunk, dist_sq_entry);

// --- 0x948D30 parked_vehicles_do_frame (no args) -------------------------------------------------

const PLAYER_IN_SPECIAL_VEHICLE: usize = 0x009E_1EA0; // EAX = human -> AL
const PARKED_FLAGS: usize = 0x0215_DA14; // six bytes
const PARKED_STEPS: [usize; 4] = [0x0094_84A0, 0x0094_7F40, 0x0094_80A0, 0x0094_8120];
const PARKED_SPAWN_UPDATE: usize = 0x0094_8770; // cdecl (0)
/// The copy's five step calls (offsets in the body), pointed at the recording stubs.
const PARKED_STEP_CALLS: [usize; 5] = [0x22, 0x27, 0x2C, 0x31, 0x38];

/// Not while 0x9E1EA0(EAX = local player) says so, nor while any of the six flags is clear.
unsafe fn parked_should_run() -> bool {
    unsafe {
        if call_regs(PLAYER_IN_SPECIAL_VEHICLE, rd(LOCAL_PLAYER), 0, 0, 0, 0, &[]) as u8 != 0 {
            return false;
        }
        (0..6).all(|i| rb(PARKED_FLAGS + i) != 0)
    }
}

/// The steps take no register inputs (checked); the `lea ecx, [ecx]` at 0x94812D is padding.
unsafe fn parked_run() {
    unsafe {
        calls(&PARKED_STEPS);
        call_regs(PARKED_SPAWN_UPDATE, 0, 0, 0, 0, 0, &[0]);
    }
}

static PARKED_TAPE: AtomicU32 = AtomicU32::new(0);

extern "C" fn parked_record(i: u32) {
    PARKED_TAPE.fetch_or(1 << i, Ordering::Relaxed);
}

macro_rules! tape_stub {
    ($name:ident, $i:literal) => {
        #[unsafe(naked)]
        extern "C" fn $name() {
            std::arch::naked_asm!(
                "push eax", "push ecx", "push edx", "push {i}", "call {f}", "add esp, 4", "pop edx", "pop ecx", "pop eax", "ret",
                i = const $i,
                f = sym parked_record,
            )
        }
    };
}
tape_stub!(parked_stub0, 0);
tape_stub!(parked_stub1, 1);
tape_stub!(parked_stub2, 2);
tape_stub!(parked_stub3, 3);
tape_stub!(parked_stub4, 4);
const PARKED_STUBS: [extern "C" fn(); 5] = [parked_stub0, parked_stub1, parked_stub2, parked_stub3, parked_stub4];

/// Shadow: the copy (steps recorded, not run) decides first; the steps then run once, as the
/// original decided on a mismatch.
extern "C" fn parked_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let ours = unsafe { parked_should_run() };
    let mut run = ours;
    if let Some((n, orig)) = shadowing(PARKED) {
        PARKED_TAPE.store(0, Ordering::Relaxed);
        run_original(PARKED, orig, &p, 0);
        let tape = PARKED_TAPE.load(Ordering::Relaxed);
        if tape != if ours { 0x1F } else { 0 } {
            mismatch(PARKED, n, format!("original steps {tape:#x}, Rust runs {ours}"));
            run = tape == 0x1F;
        }
    }
    if run {
        unsafe { parked_run() };
    }
}
regs_thunk!(parked_thunk, parked_entry);

// --- install ------------------------------------------------------------------------------------

/// Re-aim the rel32 of the `call` / `jmp` at `off` in a relocated copy.
unsafe fn set_rel32(copy: usize, off: usize, dest: usize) {
    let rel = (dest as i32).wrapping_sub((copy + off + 5) as i32);
    unsafe { std::ptr::write_unaligned((copy + off + 1) as *mut i32, rel) };
}

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let list: [(usize, &[u8], &[usize], extern "C" fn()); COUNT] = [
        (EQ_LOCAL, &EQ_LOCAL_BODY, &EQ_LOCAL_BODY_CALLS, eq_local_thunk),
        (DEAD_OR_INVALID, &DEAD_OR_INVALID_BODY, &DEAD_OR_INVALID_BODY_CALLS, dead_or_invalid_thunk),
        (NON_SCRIPTED, &NON_SCRIPTED_BODY, &NON_SCRIPTED_BODY_CALLS, non_scripted_thunk),
        (ACTIVE, &ACTIVE_BODY, &ACTIVE_BODY_CALLS, active_thunk),
        (RIDER_FIND, &RIDER_FIND_BODY, &RIDER_FIND_BODY_CALLS, rider_find_thunk),
        (ENTERING, &ENTERING_BODY, &ENTERING_BODY_CALLS, entering_thunk),
        (EXITING, &EXITING_BODY, &EXITING_BODY_CALLS, exiting_thunk),
        (SEAT_OCCUPANT, &SEAT_OCCUPANT_BODY, &SEAT_OCCUPANT_BODY_CALLS, seat_occupant_thunk),
        (VELOCITY, &VELOCITY_BODY, &VELOCITY_BODY_CALLS, velocity_thunk),
        (STATIC_CONTACT, &STATIC_CONTACT_BODY, &STATIC_CONTACT_BODY_CALLS, static_contact_thunk),
        (WHEEL_CONTACT, &WHEEL_CONTACT_BODY, &WHEEL_CONTACT_BODY_CALLS, wheel_contact_thunk),
        (RAGDOLL_FIND, &RAGDOLL_FIND_BODY, &RAGDOLL_FIND_BODY_CALLS, ragdoll_find_thunk),
        (ELAPSED, &ELAPSED_BODY, &ELAPSED_BODY_CALLS, elapsed_thunk),
        (SET_RANDOM, &SET_RANDOM_BODY, &SET_RANDOM_BODY_CALLS, set_random_thunk),
        (DIST_SQ, &DIST_SQ_BODY, &DIST_SQ_BODY_CALLS, dist_sq_thunk),
        (PARKED, &PARKED_BODY, &PARKED_BODY_CALLS, parked_thunk),
    ];
    let mut ok = 0;
    for (id, body, call_offsets, thunk) in list {
        let port = &PORTS[id];
        // The shadow copy first: replacing patches the entry.
        match unsafe { detour::relocate(port.addr, body, call_offsets) } {
            Ok(copy) => {
                unsafe {
                    if id == SEAT_OCCUPANT {
                        // The tail `jmp vehicle_seat_occupant` at +0x2F.
                        set_rel32(copy, 0x2F, VEHICLE_SEAT_OCCUPANT);
                    } else if id == PARKED {
                        for (&off, stub) in PARKED_STEP_CALLS.iter().zip(PARKED_STUBS) {
                            set_rel32(copy, off, stub as usize);
                        }
                    }
                }
                port.original.store(copy, Ordering::Relaxed);
            }
            Err(e) => log(&format!("port_sim_leaves: {}: no shadow copy: {e:?}", port.name)),
        }
        if replace_pinned(port.name, port.addr, body, thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_sim_leaves: replaced {ok}/{COUNT} with Rust"));
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
    if parts.is_empty() { String::new() } else { format!("port_sim_leaves: {}", parts.join(", ")) }
}

// --- original bodies (tools/body_bytes.py) ---------------------------------------------------------

const EQ_LOCAL_BODY: [u8; 18] = [
    0x85, 0xC0, 0x74, 0x0B, 0x3B, 0x05, 0xD4, 0x03, 0x17, 0x02, 0x75, 0x03, 0xB0, 0x01, 0xC3, 0x32,
    0xC0, 0xC3,
];
const EQ_LOCAL_BODY_CALLS: [usize; 0] = [];
const DEAD_OR_INVALID_BODY: [u8; 64] = [
    0x85, 0xD2, 0x74, 0x39, 0x8B, 0x42, 0x44, 0x85, 0xC0, 0x74, 0x1A, 0x8B, 0xC8, 0x81, 0xE1, 0xFF,
    0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x89, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC9, 0x74, 0x05,
    0x39, 0x41, 0x44, 0x74, 0x02, 0x33, 0xC9, 0x3B, 0xCA, 0x75, 0x12, 0xD9, 0xEE, 0xD8, 0x9A, 0x24,
    0x11, 0x00, 0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x01, 0x74, 0x03, 0x32, 0xC0, 0xC3, 0xB0, 0x01, 0xC3,
];
const DEAD_OR_INVALID_BODY_CALLS: [usize; 0] = [];
const NON_SCRIPTED_BODY: [u8; 56] = [
    0x85, 0xC9, 0x74, 0x1F, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80,
    0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x0B, 0x39, 0x48, 0x44, 0x75, 0x06, 0x83, 0x78, 0x4C,
    0x01, 0x74, 0x03, 0x33, 0xC0, 0xC3, 0x33, 0xC9, 0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00, 0x02, 0x0F,
    0x95, 0xC1, 0x83, 0xE9, 0x01, 0x23, 0xC1, 0xC3,
];
const NON_SCRIPTED_BODY_CALLS: [usize; 0] = [];
const ACTIVE_BODY: [u8; 103] = [
    0x85, 0xC0, 0x56, 0x74, 0x20, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04,
    0x8B, 0xB1, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xF6, 0x74, 0x0B, 0x39, 0x46, 0x44, 0x75, 0x06, 0x83,
    0x7E, 0x4C, 0x01, 0x74, 0x04, 0x33, 0xC0, 0x5E, 0xC3, 0x53, 0x8B, 0x9E, 0x80, 0x00, 0x00, 0x00,
    0xF6, 0xC3, 0x0B, 0x75, 0x2D, 0x80, 0x7C, 0x24, 0x0C, 0x00, 0x74, 0x0D, 0x56, 0xE8, 0xDE, 0xCE,
    0x54, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x19, 0xC1, 0xEB, 0x04, 0xF6, 0xC3, 0x01, 0x75,
    0x11, 0xF7, 0x86, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x74, 0x05, 0x5B, 0x8B, 0xC6,
    0x5E, 0xC3, 0x5B, 0x33, 0xC0, 0x5E, 0xC3,
];
const ACTIVE_BODY_CALLS: [usize; 1] = [0x3d];
const RIDER_FIND_BODY: [u8; 96] = [
    0x85, 0xF6, 0x74, 0x29, 0x8B, 0xC6, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80,
    0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x15, 0x39, 0x70, 0x44, 0x75, 0x10, 0x83, 0x78, 0x4C,
    0x01, 0x75, 0x0A, 0x8B, 0x90, 0xE0, 0x10, 0x00, 0x00, 0x85, 0xD2, 0x75, 0x03, 0x33, 0xC0, 0xC3,
    0x33, 0xC0, 0x8D, 0x8A, 0x4C, 0x03, 0x00, 0x00, 0x39, 0x31, 0x74, 0x11, 0x83, 0xC0, 0x01, 0x81,
    0xC1, 0xA8, 0x00, 0x00, 0x00, 0x83, 0xF8, 0x08, 0x7C, 0xEE, 0x33, 0xC0, 0xC3, 0x83, 0xF8, 0xFF,
    0x74, 0xDB, 0x69, 0xC0, 0xA8, 0x00, 0x00, 0x00, 0x8D, 0x84, 0x10, 0x4C, 0x03, 0x00, 0x00, 0xC3,
];
const RIDER_FIND_BODY_CALLS: [usize; 0] = [];
const ENTERING_BODY: [u8; 43] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x70, 0x44, 0xE8, 0x03, 0xE7, 0x14, 0x00, 0x85, 0xC0, 0x5E,
    0x75, 0x0B, 0x33, 0xC9, 0x83, 0xF8, 0x01, 0x0F, 0x94, 0xC1, 0x8A, 0xC1, 0xC3, 0x8B, 0x40, 0x04,
    0x33, 0xC9, 0x83, 0xF8, 0x01, 0x0F, 0x94, 0xC1, 0x8A, 0xC1, 0xC3,
];
const ENTERING_BODY_CALLS: [usize; 1] = [0x8];
const EXITING_BODY: [u8; 43] = [
    0x8B, 0x44, 0x24, 0x04, 0x56, 0x8B, 0x70, 0x44, 0xE8, 0xD3, 0xE6, 0x14, 0x00, 0x85, 0xC0, 0x5E,
    0x75, 0x0B, 0x33, 0xC9, 0x83, 0xF8, 0x02, 0x0F, 0x94, 0xC1, 0x8A, 0xC1, 0xC3, 0x8B, 0x40, 0x04,
    0x33, 0xC9, 0x83, 0xF8, 0x02, 0x0F, 0x94, 0xC1, 0x8A, 0xC1, 0xC3,
];
const EXITING_BODY_CALLS: [usize; 1] = [0x8];
const SEAT_OCCUPANT_BODY: [u8; 52] = [
    0x85, 0xC9, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x80, 0xB9, 0xD8, 0x84, 0x00, 0x00, 0x01, 0x74, 0x07,
    0xB8, 0x08, 0x00, 0x00, 0x00, 0xEB, 0x0A, 0x8B, 0x81, 0xE8, 0x6C, 0x00, 0x00, 0x85, 0xC0, 0x74,
    0xE3, 0x3B, 0xD0, 0x7D, 0xDF, 0x8D, 0x04, 0xD2, 0x8B, 0x84, 0x81, 0xBC, 0x8A, 0x00, 0x00, 0xE9,
    0xAC, 0x99, 0xEB, 0xFF,
];
const SEAT_OCCUPANT_BODY_CALLS: [usize; 0] = [];
const VELOCITY_BODY: [u8; 124] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF0, 0x83, 0xEC, 0x20, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4,
    0x89, 0x44, 0x24, 0x1C, 0x80, 0xB9, 0xD8, 0x84, 0x00, 0x00, 0x01, 0x8B, 0x45, 0x08, 0x74, 0x29,
    0x8B, 0x0D, 0xD0, 0xA6, 0x52, 0x02, 0x89, 0x08, 0x8B, 0x15, 0xD4, 0xA6, 0x52, 0x02, 0x89, 0x50,
    0x04, 0x8B, 0x0D, 0xD8, 0xA6, 0x52, 0x02, 0x89, 0x48, 0x08, 0x8B, 0x4C, 0x24, 0x1C, 0x33, 0xCC,
    0xE8, 0x2A, 0xF7, 0x1E, 0x00, 0x8B, 0xE5, 0x5D, 0xC3, 0x8B, 0x91, 0x54, 0x05, 0x00, 0x00, 0x0F,
    0x28, 0x82, 0xA0, 0x01, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x1C, 0x0F, 0x29, 0x04, 0x24, 0xD9, 0x04,
    0x24, 0xD9, 0x18, 0x33, 0xCC, 0xD9, 0x44, 0x24, 0x04, 0xD9, 0x58, 0x04, 0xD9, 0x44, 0x24, 0x08,
    0xD9, 0x58, 0x08, 0xE8, 0xF7, 0xF6, 0x1E, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
];
const VELOCITY_BODY_CALLS: [usize; 2] = [0x40, 0x73];
const STATIC_CONTACT_BODY: [u8; 54] = [
    0x8B, 0x40, 0x44, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0x8B, 0x0C, 0xC5, 0x84, 0xCD, 0x75, 0x01, 0x8D,
    0x04, 0xC5, 0x80, 0xCD, 0x75, 0x01, 0x8B, 0xC1, 0x85, 0xC0, 0x74, 0x14, 0x8D, 0x64, 0x24, 0x00,
    0x83, 0x38, 0x00, 0x74, 0x0E, 0x8B, 0x40, 0x10, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0, 0x75, 0xF0,
    0x32, 0xC0, 0xC3, 0xB0, 0x01, 0xC3,
];
const STATIC_CONTACT_BODY_CALLS: [usize; 0] = [];
const WHEEL_CONTACT_BODY: [u8; 141] = [
    0x85, 0xFF, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x8B, 0x87, 0xE4, 0x84, 0x00, 0x00, 0x8B, 0x80, 0x9C,
    0x00, 0x00, 0x00, 0x83, 0xF8, 0x03, 0x0F, 0x94, 0xC1, 0x80, 0xF9, 0x01, 0x74, 0xE6, 0x83, 0xF8,
    0x04, 0x0F, 0x94, 0xC0, 0x3C, 0x01, 0x74, 0xDC, 0x80, 0xBF, 0xD8, 0x84, 0x00, 0x00, 0x01, 0x75,
    0xD3, 0x8B, 0x87, 0xE0, 0x6C, 0x00, 0x00, 0x85, 0xC0, 0x74, 0xC9, 0x56, 0x33, 0xF6, 0x39, 0x30,
    0x76, 0x3E, 0xEB, 0x0C, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x8B, 0x4C, 0xB0, 0x04, 0xF7, 0x81, 0x1C, 0x01, 0x00, 0x00, 0x00, 0x00, 0x60, 0x00, 0x75, 0x13,
    0x8B, 0x8F, 0x3C, 0x05, 0x00, 0x00, 0x8B, 0x11, 0x8B, 0x42, 0x40, 0x56, 0xFF, 0xD0, 0x83, 0xF8,
    0xFF, 0x75, 0x0D, 0x8B, 0x87, 0xE0, 0x6C, 0x00, 0x00, 0x83, 0xC6, 0x01, 0x3B, 0x30, 0x72, 0xD0,
    0x8B, 0x8F, 0xE0, 0x6C, 0x00, 0x00, 0x3B, 0x31, 0x5E, 0x0F, 0x95, 0xC0, 0xC3,
];
const WHEEL_CONTACT_BODY_CALLS: [usize; 0] = [];
const RAGDOLL_FIND_BODY: [u8; 99] = [
    0xA1, 0x28, 0x74, 0xED, 0x00, 0x85, 0xC0, 0x7E, 0x18, 0x69, 0xC0, 0xA0, 0x12, 0x00, 0x00, 0x8B,
    0x90, 0x60, 0x10, 0x96, 0x02, 0x3B, 0x51, 0x44, 0x75, 0x07, 0x8D, 0x80, 0xF0, 0x00, 0x96, 0x02,
    0xC3, 0x8B, 0x51, 0x44, 0x33, 0xC0, 0xB9, 0x60, 0x10, 0x96, 0x02, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x39, 0x11, 0x74, 0x1E, 0x81, 0xC1, 0xA0, 0x12, 0x00, 0x00, 0x83, 0xC0, 0x01, 0x81, 0xF9, 0xE0,
    0xEF, 0x96, 0x02, 0x7C, 0xEB, 0xC7, 0x05, 0x28, 0x74, 0xED, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x33,
    0xC0, 0xC3, 0xA3, 0x28, 0x74, 0xED, 0x00, 0x69, 0xC0, 0xA0, 0x12, 0x00, 0x00, 0x05, 0xF0, 0x00,
    0x96, 0x02, 0xC3,
];
const RAGDOLL_FIND_BODY_CALLS: [usize; 0] = [];
const ELAPSED_BODY: [u8; 70] = [
    0x8B, 0x00, 0x85, 0xC0, 0x7D, 0x08, 0xB8, 0x00, 0xD2, 0x49, 0x6B, 0xF7, 0xD8, 0xC3, 0x8B, 0x15,
    0xAC, 0x43, 0xE8, 0x00, 0x3B, 0xC2, 0x7E, 0x11, 0x2B, 0xC2, 0x3D, 0x00, 0xE9, 0xA4, 0x35, 0x7E,
    0x22, 0x05, 0x00, 0x2E, 0xB6, 0x94, 0xF7, 0xD8, 0xC3, 0x8B, 0xCA, 0x2B, 0xC8, 0x81, 0xF9, 0x00,
    0xE9, 0xA4, 0x35, 0x7E, 0x0A, 0x2B, 0xC2, 0x05, 0x00, 0xD2, 0x49, 0x6B, 0xF7, 0xD8, 0xC3, 0xF7,
    0xD9, 0x8B, 0xC1, 0xF7, 0xD8, 0xC3,
];
const ELAPSED_BODY_CALLS: [usize; 0] = [];
const SET_RANDOM_BODY: [u8; 114] = [
    0x8B, 0x0D, 0xBC, 0x3A, 0xEC, 0x00, 0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x8B, 0xF0, 0x8B, 0x04,
    0x8D, 0xC0, 0x3A, 0xEC, 0x00, 0x83, 0xC1, 0x01, 0x81, 0xF9, 0x00, 0x08, 0x00, 0x00, 0x89, 0x0D,
    0xBC, 0x3A, 0xEC, 0x00, 0x7C, 0x0A, 0xC7, 0x05, 0xBC, 0x3A, 0xEC, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x2B, 0xF3, 0x83, 0xC6, 0x01, 0x33, 0xD2, 0xF7, 0xF6, 0x5E, 0x03, 0xD3, 0x5B, 0x78, 0x1A, 0xA1,
    0xAC, 0x43, 0xE8, 0x00, 0x03, 0xC2, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x07, 0x7E, 0x20, 0x05,
    0x00, 0x2E, 0xB6, 0x94, 0x89, 0x07, 0xC2, 0x04, 0x00, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x8D,
    0x04, 0x11, 0x85, 0xC0, 0x89, 0x07, 0x7D, 0x07, 0x05, 0x00, 0xD2, 0x49, 0x6B, 0x89, 0x07, 0xC2,
    0x04, 0x00,
];
const SET_RANDOM_BODY_CALLS: [usize; 0] = [];
const DIST_SQ_BODY: [u8; 71] = [
    0x83, 0xEC, 0x08, 0xD9, 0x01, 0xD8, 0x20, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xDC, 0xC8, 0xD9,
    0x5C, 0x24, 0x04, 0xD9, 0x41, 0x04, 0xD8, 0x60, 0x04, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0xDC,
    0xC8, 0xD8, 0x44, 0x24, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0x41, 0x08, 0xD8, 0x60, 0x08, 0xD9,
    0x1C, 0x24, 0xD9, 0x04, 0x24, 0xDC, 0xC8, 0xD8, 0x44, 0x24, 0x04, 0xD9, 0x5C, 0x24, 0x04, 0xD9,
    0x44, 0x24, 0x04, 0x83, 0xC4, 0x08, 0xC3,
];
const DIST_SQ_BODY_CALLS: [usize; 0] = [];
const PARKED_BODY: [u8; 66] = [
    0x51, 0xA1, 0xD4, 0x03, 0x17, 0x02, 0xE8, 0x65, 0x91, 0x09, 0x00, 0x84, 0xC0, 0x75, 0x31, 0x33,
    0xC0, 0x80, 0xB8, 0x14, 0xDA, 0x15, 0x02, 0x00, 0x74, 0x26, 0x83, 0xC0, 0x01, 0x83, 0xF8, 0x06,
    0x72, 0xEF, 0xE8, 0x49, 0xF7, 0xFF, 0xFF, 0xE8, 0xE4, 0xF1, 0xFF, 0xFF, 0xE8, 0x3F, 0xF3, 0xFF,
    0xFF, 0xE8, 0xBA, 0xF3, 0xFF, 0xFF, 0x6A, 0x00, 0xE8, 0x03, 0xFA, 0xFF, 0xFF, 0x83, 0xC4, 0x04,
    0x59, 0xC3,
];
const PARKED_BODY_CALLS: [usize; 6] = [0x6, 0x22, 0x27, 0x2c, 0x31, 0x38];
