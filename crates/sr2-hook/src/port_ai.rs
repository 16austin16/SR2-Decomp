//! Port module: the AI / navigation leaves of doc 45 ("Port candidates"), each checked against
//! the disassembly (not just the doc's recipe).
//!
//! Every port goes through a `pushad` thunk, so all registers come back as the caller left them
//! except EAX (and the few registers an original deliberately sets: ECX in `ai_event_find`,
//! `ai_order_payload_init` and `ai_order_k05_payload_init`, EDX in `ai_order_payload_init`). That
//! is a superset of what each original preserves. Where an original returns AL only, the upper
//! bytes of EAX are reproduced as the original leaves them.
//!
//! Shadow check, for the first `SHADOW_CALLS` calls of each address (a relocated copy of the
//! original runs and the results are compared; on a mismatch the original's result is kept):
//! - pure ports (`ai_event_find`, `ai_human_is_active_alive`, `ai_human_leader_flag4`,
//!   `ai_human_has_idle_anim_override`, `ai_human_accumulator_mean`): EAX (or ST0) compared;
//! - payload initialisers: the original and the Rust version each fill a scratch buffer, the
//!   buffers and EAX / ECX are compared, and the chosen bytes (those that are not the fill
//!   pattern) are copied to the caller's payload;
//! - `ai_event_cancel` and the `ai_order_k17_exit` family: the `+8` dword of every node of both
//!   event lists is snapshotted, the original runs, its result is saved and the snapshot restored,
//!   then Rust runs and the two are compared (the original's writes win on a mismatch);
//! - the call sequences (`gaze_at_unless_self`, `stop_if_unflagged`, `set_idle_anim_override`,
//!   `ai_order_payload_init`, the slot-1 exits `k01` / `k0e`): the copy runs with its callee
//!   calls pointed at recording stubs (for `ai_order_payload_init` the handler table read is
//!   pointed at a copy of the table full of stubs), the recorded calls and arguments are compared
//!   with the Rust plan, and the calls are then made for real from the winning list.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

/// The two circular event lists (nodes: +0 next, +8 type, +0xC arg0, +0x10 arg1).
const EVENTS_A: usize = 0x0252_6A00;
const EVENTS_B: usize = 0x0252_6A04;
/// Slot 4 (payload init) of kind 0 in the 36 x 28-byte AI order table at 0xDD5568.
const ORDER_PAYLOAD_SLOT: u32 = 0x00DD_5578;
const ORDER_TABLE: usize = 0x00DD_5568;
const ORDER_KINDS: u32 = 36;

const HUMAN_IS_DEAD: usize = 0x0098_5630; // cdecl (human) -> AL
const EFFECTIVE_LEADER: usize = 0x0098_E7C0; // EAX / EDI = human -> EAX
const GAZE_SET_TARGET: usize = 0x009B_5C80; // cdecl (human, handle); EDX = duration, CL = 1
const GAZE_CLEAR: usize = 0x009B_5FF0; // EAX = human
const STOP_HELPER: usize = 0x0099_3BD0; // cdecl (human, 0)
const ANIM_SET_UPDATE: usize = 0x0096_CB70; // cdecl (human, -1, 1, 1, 0)
const IDLE_CLEAR: usize = 0x0043_FA90; // EAX = human

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
/// that take none), then capture all seven registers into `r` and, when `want_st0`, pop ST0
/// (returned).
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

const FIND: usize = 0;
const CANCEL: usize = 1;
const MEAN: usize = 2;
const ALIVE: usize = 3;
const LEADER: usize = 4;
const IDLE_HAS: usize = 5;
const GAZE: usize = 6;
const STOP: usize = 7;
const IDLE_SET: usize = 8;
const PINIT: usize = 9;
const K01_EXIT: usize = 10;
const K0E_EXIT: usize = 11;
const K01_PI: usize = 12;
const K19_PI: usize = 13;
const K1E_PI: usize = 14;
const K05_PI: usize = 15;
const K06_PI: usize = 16;
const K07_PI: usize = 17;
const K11_PI: usize = 18;
const K14_PI: usize = 19;
const K1F_PI: usize = 20;
const K17_EXIT: usize = 21;
const K1C_EXIT: usize = 22;
const K21_EXIT: usize = 23;
const COUNT: usize = 24;

static PORTS: [Port; COUNT] = [
    port("ai_event_find", 0x0040_20F0, EDX | SAVED),
    port("ai_event_cancel", 0x0040_2160, SAVED),
    port("ai_human_accumulator_mean", 0x0040_67B0, EAX | ECX | EDX | SAVED),
    port("ai_human_is_active_alive", 0x0043_9C00, SAVED),
    port("ai_human_leader_flag4", 0x0043_9660, SAVED),
    port("ai_human_has_idle_anim_override", 0x0043_F9C0, EDX | SAVED),
    port("ai_human_gaze_at_unless_self", 0x0043_F5C0, SAVED),
    port("ai_human_stop_if_unflagged", 0x0043_9430, SAVED),
    port("ai_human_set_idle_anim_override", 0x0043_FA30, SAVED),
    port("ai_order_payload_init", 0x0043_F960, SAVED),
    port("ai_order_k01_exit", 0x0040_28E0, SAVED),
    port("ai_order_k0e_exit", 0x0043_0100, SAVED),
    port("ai_order_k01_payload_init", 0x0040_25D0, ECX | EDX | SAVED),
    port("ai_order_k19_payload_init", 0x0045_AAF0, ECX | EDX | SAVED),
    port("ai_order_k1e_payload_init", 0x0046_1270, ECX | EDX | SAVED),
    port("ai_order_k05_payload_init", 0x0040_D2F0, EDX | SAVED),
    port("ai_order_k06_payload_init", 0x0041_0B10, ECX | EDX | SAVED),
    port("ai_order_k07_payload_init", 0x0041_25B0, ECX | EDX | SAVED),
    port("ai_order_k11_payload_init", 0x0043_27C0, ECX | EDX | SAVED),
    port("ai_order_k14_payload_init", 0x0045_5FD0, ECX | EDX | SAVED),
    port("ai_order_k1f_payload_init", 0x0046_2270, ECX | EDX | SAVED),
    port("ai_order_k17_exit", 0x0045_9850, SAVED),
    port("ai_order_k1c_exit", 0x0045_E600, SAVED),
    port("ai_order_k21_exit", 0x0046_36C0, SAVED),
];

/// Count a call; `Some((call number, original copy))` while it should be shadowed.
fn shadowing(id: usize) -> Option<(u32, usize)> {
    let p = &PORTS[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_ai: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    (orig != 0 && n < SHADOW_CALLS).then_some((n, orig))
}

fn mismatch(id: usize, n: u32, what: String) {
    let m = PORTS[id].mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!("port_ai: {} mismatch (call {n}): {what}", PORTS[id].name));
    }
}

static PRESERVE_LOGS: AtomicU32 = AtomicU32::new(0);

/// Log when the original changed a register it was found to preserve (the thunks preserve all).
fn check_preserved(id: usize, inp: &Regs, out: &Regs) {
    for i in 0..7 {
        if PORTS[id].preserved & (1 << i) != 0 && inp.get(i) != out.get(i) && PRESERVE_LOGS.fetch_add(1, Ordering::Relaxed) < 10 {
            log(&format!(
                "port_ai: {}: the original changed {} ({:#x} -> {:#x})",
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

/// `p + o` as an address.
fn a(p: u32, o: u32) -> usize {
    p.wrapping_add(o) as usize
}

/// EAX with AL replaced by a bool.
fn lo(eax: u32, al: bool) -> u32 {
    (eax & !0xFF) | al as u32
}

unsafe fn rw(p: usize) -> u16 {
    unsafe { *(p as *const u16) }
}

// --- 0x4020F0 ai_event_find (EDI = type, ESI = arg0, EDX = arg1 -> AL) ---------------------------

/// (EAX, ECX): AL = 1 for the first node with `+8 == type`, `arg0 == 0 || +0xC == arg0` and
/// `arg1 == 0 || +0x10 == arg1`, searching list A then list B (each stops on a null link or on
/// coming back to its head); EAX upper bytes are the node (found) or the final link (not found),
/// ECX the head of the list the search ended in.
unsafe fn event_find(ty: u32, arg0: u32, arg1: u32) -> (u32, u32) {
    unsafe {
        let mut ecx = 0;
        let mut eax = 0;
        for list in [EVENTS_A, EVENTS_B] {
            let head = rd(list);
            ecx = head;
            eax = head;
            let mut n = head;
            while n != 0 {
                let p = n as usize;
                if rd(p + 8) == ty && (arg0 == 0 || rd(p + 0xC) == arg0) && (arg1 == 0 || rd(p + 0x10) == arg1) {
                    return (lo(n, true), ecx);
                }
                n = rd(p);
                eax = n;
                if n == head {
                    break;
                }
            }
        }
        (eax & !0xFF, ecx)
    }
}

extern "C" fn find_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let (eax, ecx) = unsafe { event_find(p.edi, p.esi, p.edx) };
    r.eax = checked_eax(FIND, &p, 0, eax);
    r.ecx = ecx;
}
regs_thunk!(find_thunk, find_entry);

// --- 0x402160 ai_event_cancel (ESI = type, cdecl (arg0, arg1)) ---------------------------------------

/// Every node of both lists whose type matches (`-1` matches all), `arg0 == 0 || +0xC == arg0` and
/// `arg1 == 0 || +0x10 == arg1` gets `+8 = -1`. The next link is read before the test and is
/// 0 when it equals the head (the `neg / sbb / and` mask), the head is read once per list.
unsafe fn event_cancel(ty: u32, arg0: u32, arg1: u32) {
    unsafe {
        for list in [EVENTS_A, EVENTS_B] {
            let head = rd(list);
            let mut n = head;
            while n != 0 {
                let p = n as usize;
                let l = rd(p);
                let next = if l != head { l } else { 0 };
                if (ty == u32::MAX || rd(p + 8) == ty) && (arg0 == 0 || rd(p + 0xC) == arg0) && (arg1 == 0 || rd(p + 0x10) == arg1) {
                    wd(p + 8, u32::MAX);
                }
                n = next;
            }
        }
    }
}

/// Every node either list can reach (what `event_cancel` can write to).
unsafe fn event_nodes() -> Vec<usize> {
    let mut v = Vec::new();
    unsafe {
        for list in [EVENTS_A, EVENTS_B] {
            let head = rd(list);
            let mut n = head;
            while n != 0 && v.len() < 100_000 {
                v.push(n as usize);
                let l = rd(n as usize);
                n = if l != head { l } else { 0 };
            }
        }
    }
    v
}

unsafe fn node_types(nodes: &[usize]) -> Vec<u32> {
    nodes.iter().map(|&n| unsafe { rd(n + 8) }).collect()
}

unsafe fn restore_types(nodes: &[usize], vals: &[u32]) {
    for (&n, &v) in nodes.iter().zip(vals) {
        unsafe { wd(n + 8, v) };
    }
}

/// Run a cancel-like operation. While shadowed: the original runs (`run_orig` gets the copy's
/// address), its node writes are saved and undone, then Rust runs, and on a difference the
/// original's writes are put back.
unsafe fn cancel_shadowed(id: usize, run_orig: impl FnOnce(usize), run_rust: impl FnOnce()) {
    let Some((n, orig)) = shadowing(id) else { return run_rust() };
    unsafe {
        let nodes = event_nodes();
        let before = node_types(&nodes);
        run_orig(orig);
        let theirs = node_types(&nodes);
        restore_types(&nodes, &before);
        run_rust();
        let ours = node_types(&nodes);
        if theirs != ours {
            let first = theirs.iter().zip(&ours).position(|(t, o)| t != o).unwrap_or(0);
            mismatch(
                id,
                n,
                format!(
                    "{} nodes; first difference at node {first}: original {:#x}, Rust {:#x}",
                    nodes.len(),
                    theirs.get(first).copied().unwrap_or(0),
                    ours.get(first).copied().unwrap_or(0)
                ),
            );
            restore_types(&nodes, &theirs);
        }
    }
}

extern "C" fn cancel_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let (a0, a1) = unsafe { (*args, *args.add(1)) };
    unsafe {
        cancel_shadowed(
            CANCEL,
            |orig| {
                call_regs(orig, 0, p.ecx, p.edx, p.esi, p.edi, &[a0, a1]);
            },
            || event_cancel(p.esi, a0, a1),
        );
    }
}
regs_thunk!(cancel_thunk, cancel_entry);

// --- 0x4067B0 ai_human_accumulator_mean (ECX = human -> ST0) -----------------------------------------

/// n = int [+0x1120]; n <= 0 gives 0.0, else f32(float [+0x1124] / n) (the divide at 53 bits, one
/// rounding to f32 through the stack slot).
unsafe fn mean(h: u32) -> f32 {
    unsafe {
        let n = rd(a(h, 0x1120)) as i32;
        if n <= 0 {
            return 0.0;
        }
        (f32::from_bits(rd(a(h, 0x1124))) as f64 / n as f64) as f32
    }
}

extern "C" fn mean_entry(r: &mut Pushad, _: *const u32) -> f32 {
    let p = *r;
    let ours = unsafe { mean(p.ecx) };
    let Some((n, orig)) = shadowing(MEAN) else { return ours };
    let inp = Regs::of(&p);
    let mut out = inp;
    let theirs = unsafe { call_full(orig, &mut out, 0, true) } as f32;
    check_preserved(MEAN, &inp, &out);
    if theirs.to_bits() != ours.to_bits() {
        mismatch(MEAN, n, format!("human {:#x}: original {:#x}, Rust {:#x}", p.ecx, theirs.to_bits(), ours.to_bits()));
        return theirs;
    }
    ours
}
regs_thunk!(mean_thunk, mean_entry);

// --- 0x439C00 ai_human_is_active_alive (ESI = human -> AL) -------------------------------------------

/// AL = 1 only for a live, active human: not null, `+0x80` has none of 0xB, bit 4 clear, bit 5
/// set, `+0xC8` has 0x400000, `+0x103` bit 3 clear, `human_is_dead` says no, and `byte +0x595 == 0`.
/// EAX upper bytes: the caller's (null), `+0x80` (early fails), `+0x80 >> 5` (the later fails
/// before the call) or the call's EAX.
unsafe fn alive(eax: u32, h: u32) -> u32 {
    unsafe {
        if h == 0 {
            return eax & !0xFF;
        }
        let f = rd(a(h, 0x80));
        if f & 0xB != 0 || (f >> 4) & 1 != 0 {
            return f & !0xFF;
        }
        let g = f >> 5;
        if g & 1 == 0 || rd(a(h, 0xC8)) & 0x40_0000 == 0 || rb(a(h, 0x103)) & 8 != 0 {
            return g & !0xFF;
        }
        let e = call_regs(HUMAN_IS_DEAD, 0, 0, 0, 0, 0, &[h]);
        if e as u8 != 0 {
            return e & !0xFF;
        }
        lo(e, rb(a(h, 0x595)) == 0)
    }
}

extern "C" fn alive_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(ALIVE, &p, 0, unsafe { alive(p.eax, p.esi) });
}
regs_thunk!(alive_thunk, alive_entry);

// --- 0x439660 ai_human_leader_flag4 (EAX = human -> AL) -------------------------------------------------

/// leader = human_effective_leader(EAX = EDI = human); AL = 0 when leader == human or its kind byte
/// `+0x3CC != 2`, else bit 4 of byte `+0x470`; EAX upper bytes are the leader's.
unsafe fn leader_flag(h: u32) -> u32 {
    unsafe {
        let l = call_regs(EFFECTIVE_LEADER, h, 0, 0, 0, h, &[]);
        if l == h || rb(a(l, 0x3CC)) != 2 {
            return l & !0xFF;
        }
        (l & !0xFF) | ((rb(a(l, 0x470)) >> 4) & 1) as u32
    }
}

extern "C" fn leader_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(LEADER, &p, 0, unsafe { leader_flag(p.eax) });
}
regs_thunk!(leader_thunk, leader_entry);

// --- 0x43F9C0 ai_human_has_idle_anim_override (EAX = human -> AL) ---------------------------------------

/// AL = byte `+0x149 == 0` and dword `+0x32C != -1` (`setne`); the rest of EAX is the human's.
unsafe fn idle_has(h: u32) -> u32 {
    unsafe {
        if rb(a(h, 0x149)) != 0 {
            return h & !0xFF;
        }
        lo(h, rd(a(h, 0x32C)) != u32::MAX)
    }
}

extern "C" fn idle_has_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    r.eax = checked_eax(IDLE_HAS, &p, 0, unsafe { idle_has(p.eax) });
}
regs_thunk!(idle_has_thunk, idle_has_entry);

// --- the call tape (call-sequence ports) -------------------------------------------------------------

thread_local! {
    static TAPE: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
}

fn tape_push(v: Vec<u32>) {
    TAPE.with(|t| t.borrow_mut().push(v));
}

fn tape_take() -> Vec<Vec<u32>> {
    TAPE.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

const T_GAZE: u32 = 1; // [id, eax, ecx, edx, human, handle]
const T_STOP: u32 = 2; // [id, human, 0]
const T_IDLE_CLEAR: u32 = 3; // [id, human]
const T_ANIM: u32 = 4; // [id, human, -1, 1, 1, 0, value of +0x32C at the call]
const T_GAZE_CLEAR: u32 = 5; // [id, human]
const T_PAYLOAD: u32 = 6; // [id, payload]

extern "C" fn rec_gaze(r: &mut Pushad, a: *const u32) {
    unsafe { tape_push(vec![T_GAZE, r.eax, r.ecx, r.edx, *a, *a.add(1)]) };
}
extern "C" fn rec_stop(_: &mut Pushad, a: *const u32) {
    unsafe { tape_push(vec![T_STOP, *a, *a.add(1)]) };
}
extern "C" fn rec_idle_clear(r: &mut Pushad, _: *const u32) {
    tape_push(vec![T_IDLE_CLEAR, r.eax]);
}
extern "C" fn rec_anim(_: &mut Pushad, args: *const u32) {
    unsafe {
        let v: Vec<u32> = (0..5).map(|i| *args.add(i)).collect();
        tape_push([vec![T_ANIM], v.clone(), vec![rd(a(v[0], 0x32C))]].concat());
    }
}
extern "C" fn rec_gaze_clear(r: &mut Pushad, _: *const u32) {
    tape_push(vec![T_GAZE_CLEAR, r.eax]);
}
extern "C" fn rec_payload(_: &mut Pushad, a: *const u32) {
    unsafe { tape_push(vec![T_PAYLOAD, *a]) };
}
regs_thunk!(gaze_stub, rec_gaze);
regs_thunk!(stop_stub, rec_stop);
regs_thunk!(idle_clear_stub, rec_idle_clear);
regs_thunk!(anim_stub, rec_anim);
regs_thunk!(gaze_clear_stub, rec_gaze_clear);
regs_thunk!(payload_stub, rec_payload);

/// Make the calls of a tape for real (the order the originals make them in).
unsafe fn replay(tape: &[Vec<u32>], esi: u32, edi: u32) {
    for e in tape {
        unsafe {
            match e[0] {
                T_GAZE => {
                    call_regs(GAZE_SET_TARGET, e[1], e[2], e[3], esi, edi, &[e[4], e[5]]);
                }
                T_STOP => {
                    call_regs(STOP_HELPER, e[1], 0, 0, esi, edi, &[e[1], e[2]]);
                }
                T_IDLE_CLEAR => {
                    call_regs(IDLE_CLEAR, e[1], 0, 0, esi, edi, &[]);
                }
                T_ANIM => {
                    wd(a(e[1], 0x32C), e[6]);
                    call_regs(ANIM_SET_UPDATE, 0, 0, 0, esi, edi, &[e[1], e[2], e[3], e[4], e[5]]);
                }
                T_GAZE_CLEAR => {
                    call_regs(GAZE_CLEAR, e[1], 0, 0, esi, edi, &[]);
                }
                _ => {}
            }
        }
    }
}

/// Shadow a call-sequence port: the copy (callees recorded, not run) decides first; the winning
/// tape is then made for real. `run_orig` runs the copy.
unsafe fn tape_port(id: usize, p: &Pushad, plan: Vec<Vec<u32>>, run_orig: impl FnOnce(usize)) {
    let tape = match shadowing(id) {
        Some((n, orig)) => {
            tape_take();
            run_orig(orig);
            let t = tape_take();
            if t != plan {
                mismatch(id, n, format!("original {t:x?}, Rust {plan:x?}"));
                t
            } else {
                plan
            }
        }
        None => plan,
    };
    unsafe { replay(&tape, p.esi, p.edi) };
}

// --- 0x43F5C0 ai_human_gaze_at_unless_self (EAX = human, ECX = handle, cdecl (duration)) ---------

extern "C" fn gaze_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let dur = unsafe { *args };
    let h = p.eax;
    unsafe {
        // Skip when byte +0x140 has bit 2 or the human's handle +0x44 equals ECX; else
        // ai_gaze_set_target(human, handle) with EDX = duration and CL = 1 (the callee reads
        // bit 0 of ECX).
        let plan = if rb(a(h, 0x140)) & 4 == 0 && rd(a(h, 0x44)) != p.ecx { vec![vec![T_GAZE, h, (p.ecx & !0xFF) | 1, dur, h, p.ecx]] } else { vec![] };
        tape_port(GAZE, &p, plan, |orig| {
            let inp = Regs::of(&p);
            let mut out = inp;
            call_full(orig, &mut out, dur, false);
            check_preserved(GAZE, &inp, &out);
        });
    }
}
regs_thunk!(gaze_thunk, gaze_entry);

// --- 0x439430 ai_human_stop_if_unflagged (EAX = human) -------------------------------------------------

extern "C" fn stop_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let h = p.eax;
    unsafe {
        // byte +0x470 bit 4 clear: 0x993BD0(human, 0).
        let plan = if rb(a(h, 0x470)) & 0x10 == 0 { vec![vec![T_STOP, h, 0]] } else { vec![] };
        tape_port(STOP, &p, plan, |orig| {
            let inp = Regs::of(&p);
            let mut out = inp;
            call_full(orig, &mut out, 0, false);
            check_preserved(STOP, &inp, &out);
        });
    }
}
regs_thunk!(stop_thunk, stop_entry);

// --- 0x43FA30 ai_human_set_idle_anim_override (ESI = human, cdecl (anim id)) -------------------------

extern "C" fn idle_set_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let h = p.esi;
    let arg = unsafe { *args };
    unsafe {
        // Only with byte +0x149 == 0, byte +0x148 not 0x11 / 0x1C and no 0x400 in the word at
        // [[+0xD4] + 0x3C]. An old override (+0x32C != -1) is cleared first (0x43FA90: stores -1
        // and updates); then +0x32C = id and human_anim_set_update(human, -1, 1, 1, 0).
        let k = rb(a(h, 0x148));
        let guard = rb(a(h, 0x149)) == 0 && k != 0x11 && k != 0x1C && rw(a(rd(a(h, 0xD4)), 0x3C)) & 0x400 == 0;
        let mut plan = Vec::new();
        if guard {
            if rd(a(h, 0x32C)) != u32::MAX {
                plan.push(vec![T_IDLE_CLEAR, h]);
            }
            plan.push(vec![T_ANIM, h, u32::MAX, 1, 1, 0, arg]);
        }
        tape_port(IDLE_SET, &p, plan, |orig| {
            // The copy stores the new id itself; undo that.
            let old = rd(a(h, 0x32C));
            let inp = Regs::of(&p);
            let mut out = inp;
            call_full(orig, &mut out, arg, false);
            check_preserved(IDLE_SET, &inp, &out);
            wd(a(h, 0x32C), old);
        });
    }
}
regs_thunk!(idle_set_thunk, idle_set_entry);

// --- 0x43F960 ai_order_payload_init (EAX = kind, ECX = human) ----------------------------------------

/// A copy of the 36 x 28-byte handler table with a stub in every non-null entry; the shadow copy
/// of the function reads this instead (its table displacement is patched).
static FAKE_TABLE: [AtomicU32; 252] = [const { AtomicU32::new(0) }; 252];
/// Offset of the table displacement in the body (`mov eax, [edx*4 + disp32]`).
const PINIT_DISP: usize = 16;

fn fill_fake_table() {
    for (i, e) in FAKE_TABLE.iter().enumerate() {
        let real = unsafe { rd(ORDER_TABLE + i * 4) };
        e.store(if real != 0 { addr(payload_stub) as u32 } else { 0 }, Ordering::Relaxed);
    }
}

extern "C" fn pinit_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let (kind, human) = (p.eax, p.ecx);
    if human == 0 {
        // Nothing but the shadow count: the original returns at once with every register kept.
        shadowing(PINIT);
        return;
    }
    unsafe {
        // fn = [0xDD5578 + kind * 28]; EDX = kind * 7 either way, EAX = fn, ECX += 0x150 (kept
        // that way) when a handler runs: handler(payload = human + 0x150), cdecl.
        let f = rd(ORDER_PAYLOAD_SLOT.wrapping_add(kind.wrapping_mul(28)) as usize);
        let arg = human.wrapping_add(0x150);
        let mut call_it = f != 0;
        if let Some((n, orig)) = shadowing(PINIT) {
            if kind < ORDER_KINDS {
                fill_fake_table();
                tape_take();
                let mut out = Regs::of(&p);
                call_full(orig, &mut out, 0, false);
                check_preserved(PINIT, &Regs::of(&p), &out);
                let t = tape_take();
                let plan = if f != 0 { vec![vec![T_PAYLOAD, arg]] } else { vec![] };
                if t != plan {
                    mismatch(PINIT, n, format!("kind {kind:#x}: original {t:x?}, Rust {plan:x?}"));
                    call_it = !t.is_empty();
                }
            }
        }
        r.edx = kind.wrapping_mul(7);
        r.eax = f;
        if call_it {
            r.ecx = arg;
            r.eax = call_regs(f as usize, f, arg, r.edx, p.esi, p.edi, &[arg]);
        }
    }
}
regs_thunk!(pinit_thunk, pinit_entry);

// --- slot-1 exits k01 / k0e (cdecl (human, new kind)) -----------------------------------------------------

fn exit_slot1(id: usize, r: &mut Pushad, args: *const u32) {
    let p = *r;
    let h = unsafe { *args };
    unsafe {
        // Unless byte +0x140 has bit 2: tail-jump to ai_gaze_clear(EAX = human). EAX = human.
        let plan = if rb(a(h, 0x140)) & 4 == 0 { vec![vec![T_GAZE_CLEAR, h]] } else { vec![] };
        tape_port(id, &p, plan, |orig| {
            let inp = Regs::of(&p);
            let mut out = inp;
            call_full(orig, &mut out, h, false);
            check_preserved(id, &inp, &out);
        });
    }
    r.eax = h;
}

extern "C" fn k01_exit_entry(r: &mut Pushad, args: *const u32) {
    exit_slot1(K01_EXIT, r, args);
}
extern "C" fn k0e_exit_entry(r: &mut Pushad, args: *const u32) {
    exit_slot1(K0E_EXIT, r, args);
}
regs_thunk!(k01_exit_thunk, k01_exit_entry);
regs_thunk!(k0e_exit_thunk, k0e_exit_entry);

// --- exits k17 / k1c / k21 (cdecl (human, new kind)) --------------------------------------------------

/// ai_event_cancel(type 0x59, arg0 = arg1 = the human's handle +0x44); ESI is kept.
fn exit_cancel(id: usize, r: &mut Pushad, args: *const u32) {
    let p = *r;
    let h = unsafe { *args };
    unsafe {
        let handle = rd(a(h, 0x44));
        cancel_shadowed(
            id,
            |orig| {
                let inp = Regs::of(&p);
                let mut out = inp;
                call_full(orig, &mut out, h, false);
                check_preserved(id, &inp, &out);
            },
            || event_cancel(0x59, handle, handle),
        );
    }
}

extern "C" fn k17_exit_entry(r: &mut Pushad, args: *const u32) {
    exit_cancel(K17_EXIT, r, args);
}
extern "C" fn k1c_exit_entry(r: &mut Pushad, args: *const u32) {
    exit_cancel(K1C_EXIT, r, args);
}
extern "C" fn k21_exit_entry(r: &mut Pushad, args: *const u32) {
    exit_cancel(K21_EXIT, r, args);
}
regs_thunk!(k17_exit_thunk, k17_exit_entry);
regs_thunk!(k1c_exit_thunk, k1c_exit_entry);
regs_thunk!(k21_exit_thunk, k21_exit_entry);

// --- payload initialisers (cdecl (payload*) -> EAX = payload) --------------------------------------------

/// Kind 0x01 (0x4025D0): a vec3 of zeros.
unsafe fn fill_k01(p: usize) {
    unsafe {
        wd(p, 0);
        wd(p + 4, 0);
        wd(p + 8, 0);
    }
}

/// Kind 0x19 (0x45AAF0): two f32 zeros.
unsafe fn fill_k19(p: usize) {
    unsafe {
        wd(p, 0);
        wd(p + 4, 0);
    }
}

/// Kind 0x1E (0x461270): dword 0 at +0, f32 zeros at +4 .. +0x10.
unsafe fn fill_k1e(p: usize) {
    unsafe {
        for o in [0, 4, 8, 0xC, 0x10] {
            wd(p + o, 0);
        }
    }
}

/// Kind 0x05 (0x40D2F0): a zero word, `|= 8` on byte 0, then bytes 2..4 = 0 and byte 5 = 0xFF.
unsafe fn fill_k05(p: usize) {
    unsafe {
        wb(p, 8);
        wb(p + 1, 0);
        wb(p + 2, 0);
        wb(p + 3, 0);
        wb(p + 4, 0);
        wb(p + 5, 0xFF);
    }
}

/// Kinds 0x06, 0x07, 0x11, 0x14, 0x1F: byte 0 = 0, dword +4 = 0.
unsafe fn fill_k07(p: usize) {
    unsafe {
        wb(p, 0);
        wd(p + 4, 0);
    }
}

const FILL_MARK: u8 = 0xA5;

/// EAX = the payload; `ecx` is the value the original leaves in ECX when it sets one. Shadow: the
/// original and Rust fill scratch buffers, and the bytes the original wrote go to the payload.
fn payload(r: &mut Pushad, args: *const u32, id: usize, fill: unsafe fn(usize), ecx: Option<u32>) {
    let pre = *r;
    let p = unsafe { *args };
    r.eax = p;
    if let Some(c) = ecx {
        r.ecx = c;
    }
    let Some((n, orig)) = shadowing(id) else { return unsafe { fill(p as usize) } };
    let mut theirs = [FILL_MARK; 32];
    let mut ours = [FILL_MARK; 32];
    let inp = Regs::of(&pre);
    let mut out = inp;
    unsafe {
        call_full(orig, &mut out, theirs.as_mut_ptr() as u32, false);
        fill(ours.as_mut_ptr() as usize);
    }
    check_preserved(id, &inp, &out);
    if theirs != ours || out.eax != theirs.as_ptr() as u32 || out.ecx != r.ecx {
        mismatch(id, n, format!("original {theirs:x?} eax {:#x} ecx {:#x}, Rust {ours:x?} ecx {:#x}", out.eax, out.ecx, r.ecx));
        r.ecx = out.ecx;
    }
    for (i, &b) in theirs.iter().enumerate() {
        if b != FILL_MARK {
            unsafe { wb(p as usize + i, b) };
        }
    }
}

macro_rules! payload_port {
    ($entry:ident, $thunk:ident, $id:expr, $fill:ident, $ecx:expr) => {
        extern "C" fn $entry(r: &mut Pushad, args: *const u32) {
            payload(r, args, $id, $fill, $ecx);
        }
        regs_thunk!($thunk, $entry);
    };
}

payload_port!(k01_pi_entry, k01_pi_thunk, K01_PI, fill_k01, None);
payload_port!(k19_pi_entry, k19_pi_thunk, K19_PI, fill_k19, None);
payload_port!(k1e_pi_entry, k1e_pi_thunk, K1E_PI, fill_k1e, None);
payload_port!(k05_pi_entry, k05_pi_thunk, K05_PI, fill_k05, Some(0));
payload_port!(k06_pi_entry, k06_pi_thunk, K06_PI, fill_k07, None);
payload_port!(k07_pi_entry, k07_pi_thunk, K07_PI, fill_k07, None);
payload_port!(k11_pi_entry, k11_pi_thunk, K11_PI, fill_k07, None);
payload_port!(k14_pi_entry, k14_pi_thunk, K14_PI, fill_k07, None);
payload_port!(k1f_pi_entry, k1f_pi_thunk, K1F_PI, fill_k07, None);

// --- install ------------------------------------------------------------------------------------

/// Re-aim the rel32 of the `call` / `jmp` at `off` in a relocated copy (checking the opcode).
unsafe fn set_rel32(copy: usize, off: usize, dest: usize, op: u8) -> bool {
    unsafe {
        if *((copy + off) as *const u8) != op {
            return false;
        }
        let rel = (dest as i32).wrapping_sub((copy + off + 5) as i32);
        std::ptr::write_unaligned((copy + off + 1) as *mut i32, rel);
    }
    true
}

/// Point the copy's callee calls at the recording stubs / the cancel original. False if the body
/// is not shaped as expected (then the port runs without a shadow).
unsafe fn rewire(id: usize, copy: usize) -> bool {
    unsafe {
        match id {
            GAZE => set_rel32(copy, 0x16, addr(gaze_stub), 0xE8),
            STOP => set_rel32(copy, 0x12, addr(stop_stub), 0xE8),
            IDLE_SET => set_rel32(copy, 0x30, addr(idle_clear_stub), 0xE8) && set_rel32(copy, 0x48, addr(anim_stub), 0xE8),
            K01_EXIT | K0E_EXIT => set_rel32(copy, 0xD, addr(gaze_clear_stub), 0xE9),
            PINIT => {
                let at = copy + PINIT_DISP;
                let ok = std::ptr::read_unaligned(at as *const u32) == ORDER_PAYLOAD_SLOT
                    && std::ptr::read_unaligned((at - 3) as *const [u8; 3]) == [0x8B, 0x04, 0x95];
                if ok {
                    std::ptr::write_unaligned(at as *mut u32, (FAKE_TABLE.as_ptr() as u32).wrapping_add(0x10));
                }
                ok
            }
            K17_EXIT | K1C_EXIT | K21_EXIT => {
                let cancel = PORTS[CANCEL].original.load(Ordering::Relaxed);
                cancel != 0 && set_rel32(copy, 0xF, cancel, 0xE8)
            }
            _ => true,
        }
    }
}

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let list: [(usize, &[u8], &[usize], extern "C" fn()); COUNT] = [
        (FIND, &FIND_BODY, &FIND_BODY_CALLS, find_thunk),
        (CANCEL, &CANCEL_BODY, &CANCEL_BODY_CALLS, cancel_thunk),
        (MEAN, &MEAN_BODY, &MEAN_BODY_CALLS, mean_thunk),
        (ALIVE, &ALIVE_BODY, &ALIVE_BODY_CALLS, alive_thunk),
        (LEADER, &LEADER_BODY, &LEADER_BODY_CALLS, leader_thunk),
        (IDLE_HAS, &IDLE_HAS_BODY, &IDLE_HAS_BODY_CALLS, idle_has_thunk),
        (GAZE, &GAZE_BODY, &GAZE_BODY_CALLS, gaze_thunk),
        (STOP, &STOP_BODY, &STOP_BODY_CALLS, stop_thunk),
        (IDLE_SET, &IDLE_SET_BODY, &IDLE_SET_BODY_CALLS, idle_set_thunk),
        (PINIT, &PINIT_BODY, &PINIT_BODY_CALLS, pinit_thunk),
        (K01_EXIT, &K01_EXIT_BODY, &K01_EXIT_BODY_CALLS, k01_exit_thunk),
        (K0E_EXIT, &K0E_EXIT_BODY, &K0E_EXIT_BODY_CALLS, k0e_exit_thunk),
        (K01_PI, &K01_PI_BODY, &K01_PI_BODY_CALLS, k01_pi_thunk),
        (K19_PI, &K19_PI_BODY, &K19_PI_BODY_CALLS, k19_pi_thunk),
        (K1E_PI, &K1E_PI_BODY, &K1E_PI_BODY_CALLS, k1e_pi_thunk),
        (K05_PI, &K05_PI_BODY, &K05_PI_BODY_CALLS, k05_pi_thunk),
        (K06_PI, &K06_PI_BODY, &K06_PI_BODY_CALLS, k06_pi_thunk),
        (K07_PI, &K07_PI_BODY, &K07_PI_BODY_CALLS, k07_pi_thunk),
        (K11_PI, &K11_PI_BODY, &K11_PI_BODY_CALLS, k11_pi_thunk),
        (K14_PI, &K14_PI_BODY, &K14_PI_BODY_CALLS, k14_pi_thunk),
        (K1F_PI, &K1F_PI_BODY, &K1F_PI_BODY_CALLS, k1f_pi_thunk),
        (K17_EXIT, &K17_EXIT_BODY, &K17_EXIT_BODY_CALLS, k17_exit_thunk),
        (K1C_EXIT, &K1C_EXIT_BODY, &K1C_EXIT_BODY_CALLS, k1c_exit_thunk),
        (K21_EXIT, &K21_EXIT_BODY, &K21_EXIT_BODY_CALLS, k21_exit_thunk),
    ];
    let mut ok = 0;
    for (id, body, call_offsets, thunk) in list {
        let port = &PORTS[id];
        // The shadow copy first: replacing patches the entry.
        match unsafe { detour::relocate(port.addr, body, call_offsets) } {
            Ok(copy) => {
                if unsafe { rewire(id, copy) } {
                    port.original.store(copy, Ordering::Relaxed);
                } else {
                    log(&format!("port_ai: {}: copy not shaped as expected, no shadow", port.name));
                }
            }
            Err(e) => log(&format!("port_ai: {}: no shadow copy: {e:?}", port.name)),
        }
        if replace_pinned(port.name, port.addr, body, thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_ai: replaced {ok}/{COUNT} with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let parts: Vec<String> = PORTS
        .iter()
        .filter_map(|p| {
            let c = p.calls.load(Ordering::Relaxed);
            (c > 0).then(|| format!("{} {c} ({})", p.name, p.mismatches.load(Ordering::Relaxed)))
        })
        .collect();
    if parts.is_empty() { String::new() } else { format!("port_ai: {}", parts.join(", ")) }
}

// --- original bodies (tools/body_bytes.py) ---------------------------------------------------------

const FIND_BODY: [u8; 103] = [
    0x8B, 0x0D, 0x00, 0x6A, 0x52, 0x02, 0x85, 0xC9, 0x8B, 0xC1, 0x74, 0x25, 0x8D, 0x64, 0x24, 0x00,
    0x39, 0x78, 0x08, 0x75, 0x12, 0x85, 0xF6, 0x74, 0x05, 0x39, 0x70, 0x0C, 0x75, 0x09, 0x85, 0xD2,
    0x74, 0x42, 0x39, 0x50, 0x10, 0x74, 0x3D, 0x8B, 0x00, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0, 0x75,
    0xDF, 0x8B, 0x0D, 0x04, 0x6A, 0x52, 0x02, 0x85, 0xC9, 0x8B, 0xC1, 0x74, 0x24, 0x8D, 0x49, 0x00,
    0x39, 0x78, 0x08, 0x75, 0x12, 0x85, 0xF6, 0x74, 0x05, 0x39, 0x70, 0x0C, 0x75, 0x09, 0x85, 0xD2,
    0x74, 0x12, 0x39, 0x50, 0x10, 0x74, 0x0D, 0x8B, 0x00, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0, 0x75,
    0xDF, 0x32, 0xC0, 0xC3, 0xB0, 0x01, 0xC3,
];
const FIND_BODY_CALLS: [usize; 0] = [];
const CANCEL_BODY: [u8; 141] = [
    0x53, 0x8B, 0x5C, 0x24, 0x0C, 0x55, 0x8B, 0x6C, 0x24, 0x0C, 0x57, 0x8B, 0x3D, 0x00, 0x6A, 0x52,
    0x02, 0x8B, 0xCF, 0x85, 0xC9, 0x74, 0x33, 0x8B, 0x11, 0x8B, 0xC2, 0x2B, 0xC7, 0xF7, 0xD8, 0x1B,
    0xC0, 0x23, 0xC2, 0x83, 0xFE, 0xFF, 0x74, 0x05, 0x39, 0x71, 0x08, 0x75, 0x19, 0x85, 0xED, 0x74,
    0x05, 0x39, 0x69, 0x0C, 0x75, 0x10, 0x85, 0xDB, 0x74, 0x05, 0x39, 0x59, 0x10, 0x75, 0x07, 0xC7,
    0x41, 0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0x8B, 0xC8, 0xEB, 0xC9, 0x8B, 0x3D, 0x04, 0x6A, 0x52, 0x02,
    0x8B, 0xD7, 0x85, 0xD2, 0x74, 0x33, 0x8B, 0x0A, 0x8B, 0xC1, 0x2B, 0xC7, 0xF7, 0xD8, 0x1B, 0xC0,
    0x23, 0xC1, 0x83, 0xFE, 0xFF, 0x74, 0x05, 0x39, 0x72, 0x08, 0x75, 0x19, 0x85, 0xED, 0x74, 0x05,
    0x39, 0x6A, 0x0C, 0x75, 0x10, 0x85, 0xDB, 0x74, 0x05, 0x39, 0x5A, 0x10, 0x75, 0x07, 0xC7, 0x42,
    0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0x8B, 0xD0, 0xEB, 0xC9, 0x5F, 0x5D, 0x5B, 0xC3,
];
const CANCEL_BODY_CALLS: [usize; 0] = [];
const MEAN_BODY: [u8; 41] = [
    0x51, 0x8B, 0x81, 0x20, 0x11, 0x00, 0x00, 0x85, 0xC0, 0x89, 0x04, 0x24, 0x7E, 0x11, 0xD9, 0x81,
    0x24, 0x11, 0x00, 0x00, 0xDA, 0x34, 0x24, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC3, 0xD9,
    0xEE, 0xD9, 0x1C, 0x24, 0xD9, 0x04, 0x24, 0x59, 0xC3,
];
const MEAN_BODY_CALLS: [usize; 0] = [];
const ALIVE_BODY: [u8; 78] = [
    0x85, 0xF6, 0x74, 0x47, 0x8B, 0x86, 0x80, 0x00, 0x00, 0x00, 0xA8, 0x0B, 0x75, 0x3D, 0x8B, 0xC8,
    0xC1, 0xE9, 0x04, 0xF6, 0xC1, 0x01, 0x75, 0x33, 0xC1, 0xE8, 0x05, 0xA8, 0x01, 0x74, 0x2C, 0xF7,
    0x86, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x74, 0x20, 0xF6, 0x86, 0x03, 0x01, 0x00,
    0x00, 0x08, 0x75, 0x17, 0x56, 0xE8, 0xF6, 0xB9, 0x54, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75,
    0x0A, 0x38, 0x86, 0x95, 0x05, 0x00, 0x00, 0x0F, 0x94, 0xC0, 0xC3, 0x32, 0xC0, 0xC3,
];
const ALIVE_BODY_CALLS: [usize; 1] = [0x35];
const LEADER_BODY: [u8; 37] = [
    0x57, 0x8B, 0xF8, 0xE8, 0x58, 0x51, 0x55, 0x00, 0x3B, 0xC7, 0x5F, 0x75, 0x03, 0x32, 0xC0, 0xC3,
    0x80, 0xB8, 0xCC, 0x03, 0x00, 0x00, 0x02, 0x75, 0xF4, 0x8A, 0x80, 0x70, 0x04, 0x00, 0x00, 0xC0,
    0xE8, 0x04, 0x24, 0x01, 0xC3,
];
const LEADER_BODY_CALLS: [usize; 1] = [0x3];
const IDLE_HAS_BODY: [u8; 27] = [
    0x80, 0xB8, 0x49, 0x01, 0x00, 0x00, 0x00, 0x74, 0x03, 0x32, 0xC0, 0xC3, 0x33, 0xC9, 0x83, 0xB8,
    0x2C, 0x03, 0x00, 0x00, 0xFF, 0x0F, 0x95, 0xC1, 0x8A, 0xC1, 0xC3,
];
const IDLE_HAS_BODY_CALLS: [usize; 0] = [];
const GAZE_BODY: [u8; 31] = [
    0xF6, 0x80, 0x40, 0x01, 0x00, 0x00, 0x04, 0x75, 0x15, 0x39, 0x48, 0x44, 0x74, 0x10, 0x8B, 0x54,
    0x24, 0x04, 0x51, 0x50, 0xB1, 0x01, 0xE8, 0xA5, 0x66, 0x57, 0x00, 0x83, 0xC4, 0x08, 0xC3,
];
const GAZE_BODY_CALLS: [usize; 1] = [0x16];
const STOP_BODY: [u8; 30] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xF6, 0x80, 0x70, 0x04, 0x00, 0x00, 0x10, 0x75, 0x0B, 0x6A,
    0x00, 0x50, 0xE8, 0x89, 0xA7, 0x55, 0x00, 0x83, 0xC4, 0x08, 0x8B, 0xE5, 0x5D, 0xC3,
];
const STOP_BODY_CALLS: [usize; 1] = [0x12];
const IDLE_SET_BODY: [u8; 81] = [
    0x80, 0xBE, 0x49, 0x01, 0x00, 0x00, 0x00, 0x75, 0x47, 0x8A, 0x86, 0x48, 0x01, 0x00, 0x00, 0x3C,
    0x11, 0x74, 0x3D, 0x3C, 0x1C, 0x74, 0x39, 0x8B, 0x86, 0xD4, 0x00, 0x00, 0x00, 0x66, 0xF7, 0x40,
    0x3C, 0x00, 0x04, 0x75, 0x2B, 0x83, 0xBE, 0x2C, 0x03, 0x00, 0x00, 0xFF, 0x74, 0x07, 0x8B, 0xC6,
    0xE8, 0x2B, 0x00, 0x00, 0x00, 0x8B, 0x4C, 0x24, 0x04, 0x6A, 0x00, 0x6A, 0x01, 0x6A, 0x01, 0x6A,
    0xFF, 0x56, 0x89, 0x8E, 0x2C, 0x03, 0x00, 0x00, 0xE8, 0xF3, 0xD0, 0x52, 0x00, 0x83, 0xC4, 0x14,
    0xC3,
];
const IDLE_SET_BODY_CALLS: [usize; 2] = [0x30, 0x48];
const PINIT_BODY: [u8; 35] = [
    0x85, 0xC9, 0x74, 0x1E, 0x8D, 0x14, 0xC5, 0x00, 0x00, 0x00, 0x00, 0x2B, 0xD0, 0x8B, 0x04, 0x95,
    0x78, 0x55, 0xDD, 0x00, 0x85, 0xC0, 0x74, 0x0A, 0x81, 0xC1, 0x50, 0x01, 0x00, 0x00, 0x51, 0xFF,
    0xD0, 0x59, 0xC3,
];
const PINIT_BODY_CALLS: [usize; 0] = [];
const K01_EXIT_BODY: [u8; 19] = [
    0x8B, 0x44, 0x24, 0x04, 0xF6, 0x80, 0x40, 0x01, 0x00, 0x00, 0x04, 0x75, 0x05, 0xE9, 0xFE, 0x36,
    0x5B, 0x00, 0xC3,
];
const K01_EXIT_BODY_CALLS: [usize; 0] = [];
const K0E_EXIT_BODY: [u8; 19] = [
    0x8B, 0x44, 0x24, 0x04, 0xF6, 0x80, 0x40, 0x01, 0x00, 0x00, 0x04, 0x75, 0x05, 0xE9, 0xDE, 0x5E,
    0x58, 0x00, 0xC3,
];
const K0E_EXIT_BODY_CALLS: [usize; 0] = [];
const K01_PI_BODY: [u8; 15] = [
    0xD9, 0xEE, 0x8B, 0x44, 0x24, 0x04, 0xD9, 0x10, 0xD9, 0x50, 0x04, 0xD9, 0x58, 0x08, 0xC3,
];
const K01_PI_BODY_CALLS: [usize; 0] = [];
const K19_PI_BODY: [u8; 12] = [
    0xD9, 0xEE, 0x8B, 0x44, 0x24, 0x04, 0xD9, 0x10, 0xD9, 0x58, 0x04, 0xC3,
];
const K19_PI_BODY_CALLS: [usize; 0] = [];
const K1E_PI_BODY: [u8; 25] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xD9, 0x50, 0x04, 0xD9,
    0x50, 0x08, 0xD9, 0x50, 0x0C, 0xD9, 0x58, 0x10, 0xC3,
];
const K1E_PI_BODY_CALLS: [usize; 0] = [];
const K05_PI_BODY: [u8; 26] = [
    0x8B, 0x44, 0x24, 0x04, 0x33, 0xC9, 0x66, 0x89, 0x08, 0x80, 0x08, 0x08, 0x88, 0x48, 0x02, 0x88,
    0x48, 0x03, 0x88, 0x48, 0x04, 0xC6, 0x40, 0x05, 0xFF, 0xC3,
];
const K05_PI_BODY_CALLS: [usize; 0] = [];
const K06_PI_BODY: [u8; 15] = [
    0x8B, 0x44, 0x24, 0x04, 0xC6, 0x00, 0x00, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const K06_PI_BODY_CALLS: [usize; 0] = [];
const K07_PI_BODY: [u8; 15] = [
    0x8B, 0x44, 0x24, 0x04, 0xC6, 0x00, 0x00, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const K07_PI_BODY_CALLS: [usize; 0] = [];
const K11_PI_BODY: [u8; 15] = [
    0x8B, 0x44, 0x24, 0x04, 0xC6, 0x00, 0x00, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const K11_PI_BODY_CALLS: [usize; 0] = [];
const K14_PI_BODY: [u8; 15] = [
    0x8B, 0x44, 0x24, 0x04, 0xC6, 0x00, 0x00, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const K14_PI_BODY_CALLS: [usize; 0] = [];
const K1F_PI_BODY: [u8; 15] = [
    0x8B, 0x44, 0x24, 0x04, 0xC6, 0x00, 0x00, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const K1F_PI_BODY_CALLS: [usize; 0] = [];
const K17_EXIT_BODY: [u8; 25] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x44, 0x56, 0x50, 0x50, 0xBE, 0x59, 0x00, 0x00, 0x00, 0xE8,
    0xFC, 0x88, 0xFA, 0xFF, 0x83, 0xC4, 0x08, 0x5E, 0xC3,
];
const K17_EXIT_BODY_CALLS: [usize; 1] = [0xf];
const K1C_EXIT_BODY: [u8; 25] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x44, 0x56, 0x50, 0x50, 0xBE, 0x59, 0x00, 0x00, 0x00, 0xE8,
    0x4C, 0x3B, 0xFA, 0xFF, 0x83, 0xC4, 0x08, 0x5E, 0xC3,
];
const K1C_EXIT_BODY_CALLS: [usize; 1] = [0xf];
const K21_EXIT_BODY: [u8; 25] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x40, 0x44, 0x56, 0x50, 0x50, 0xBE, 0x59, 0x00, 0x00, 0x00, 0xE8,
    0x8C, 0xEA, 0xF9, 0xFF, 0x83, 0xC4, 0x08, 0x5E, 0xC3,
];
const K21_EXIT_BODY_CALLS: [usize; 1] = [0xf];

/// A stub's address.
fn addr(f: extern "C" fn()) -> usize {
    f as usize
}
