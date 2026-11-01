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
//!
//! Later additions (order table handlers by slot):
//! - the bare `ret` / `xor eax, eax` / `mov eax, 2` handlers are 1, 3 and 6 bytes long, so they are
//!   pinned together with their int3 padding; the thunks are the original's own instructions;
//! - more payload initialisers (`payload_x`): scratch-buffer shadow like `payload`, plus the EAX /
//!   ECX / EDX each original leaves;
//! - net writers (`nw`): the original and Rust write into two scratch buffers positioned so that
//!   `buf + *pos` is their start; the buffers, final `*pos` and the human's payload bytes are
//!   compared and the original's bytes are what the caller gets;
//! - net readers (`nr`): both run on the real buffer with `*pos` in a scratch cell, the human's
//!   payload bytes, final positions and the recorded failure-reporter call are compared;
//! - enter / exit / on_event / update handlers and `memset`-using initialisers (`seq_port`): the copy
//!   runs with its callees pointed at recording stubs (registers and stack arguments recorded) and
//!   its `ai_event_cancel` calls at the unreplaced cancel; the recorded calls, the human fields it
//!   writes, the event nodes and its EAX are compared with a dry run of the Rust side, and the winning
//!   plan is then made for real in the original's order;
//! - `ai_order_interrupt_query`: the table read of the copy is pointed at a table of stubs.
//! The `pushad` thunks keep every register but EAX; EAX (and ECX / EDX for the payload
//! initialisers) is reproduced where the original sets it to something defined (constants, a callee's
//! result, the payload pointer). Not reproduced: the stray EAX the net writers / readers and the
//! event-cancelling exits leave (the table dispatchers `ai_order_net_write` / `ai_order_net_read` do
//! not read it after `call eax`), and ECX / EDX after calls (clobbered in the originals).

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
const RET_NOP: usize = 24;
const RET_ZERO: usize = 25;
const RET_TWO: usize = 26;
const PX_K20: usize = 27;
const PX_K15: usize = 28;
const PX_K17: usize = 29;
const PX_K1C: usize = 30;
const PX_K0F: usize = 31;
const PX_K1D: usize = 32;
const PX_K09: usize = 33;
const PX_K0E: usize = 34;
const PX_K10: usize = 35;
const PX_K0C: usize = 36;
const PX_K1B: usize = 37;
const PX_K0B: usize = 38;
const PX_K22: usize = 39;
const PX_K0A: usize = 40;
const PX_K0D: usize = 41;
const PX_K13: usize = 42;
const PX_K18: usize = 43;
const PX_K04: usize = 44;
const PX_K1A: usize = 45;
const PX_K02: usize = 46;
const NW_K20: usize = 47;
const NW_K14: usize = 48;
const NW_K08: usize = 49;
const NW_K19: usize = 50;
const NW_K01: usize = 51;
const NW_K15: usize = 52;
const NW_K05: usize = 53;
const NW_K06: usize = 54;
const NW_K1C: usize = 55;
const NW_K0C: usize = 56;
const NW_K17: usize = 57;
const EN_K19: usize = 58;
const EN_K23: usize = 59;
const EN_K20: usize = 60;
const EN_K16: usize = 61;
const EX_K15: usize = 62;
const EX_K0F: usize = 63;
const EX_K10: usize = 64;
const EX_K1E: usize = 65;
const OE_K03: usize = 66;
const OE_K0C: usize = 67;
const OE_K17: usize = 68;
const OE_K13: usize = 69;
const OE_K20: usize = 70;
const OE_K10: usize = 71;
const OE_K1D: usize = 72;
const UP_K09: usize = 73;
const IQ: usize = 74;
const NR_K19: usize = 75;
const NR_K01: usize = 76;
const NR_K05: usize = 77;
const NR_K14: usize = 78;
const NR_K06: usize = 79;
const NR_K11: usize = 80;
const NR_K1F: usize = 81;
const NR_K15: usize = 82;
const NR_K21: usize = 83;
const PX_K00: usize = 84;
const COUNT: usize = 85;

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
    port("ai_order_noop_ret", 0x0040_4330, EAX | ECX | EDX | SAVED),
    port("ai_order_ret_zero", 0x0040_29C0, ECX | EDX | SAVED),
    port("ai_order_ret_two", 0x0045_AD70, ECX | EDX | SAVED),
    port("ai_order_k20_payload_init", 0x0046_2A50, ECX | EDX | SAVED),
    port("ai_order_k15_payload_init", 0x0045_7E70, ECX | EDX | SAVED),
    port("ai_order_k17_payload_init", 0x0045_9590, ECX | EDX | SAVED),
    port("ai_order_k1c_payload_init", 0x0045_DC20, ECX | EDX | SAVED),
    port("ai_order_k0f_payload_init", 0x0043_0A80, ECX | EDX | SAVED),
    port("ai_order_k1d_payload_init", 0x0046_01E0, EDX | SAVED),
    port("ai_order_k09_payload_init", 0x0041_7970, EDX | SAVED),
    port("ai_order_k0e_payload_init", 0x0042_F940, ECX | EDX | SAVED),
    port("ai_order_k10_payload_init", 0x0043_1BE0, ECX | EDX | SAVED),
    port("ai_order_k0c_payload_init", 0x0042_7EE0, ECX | EDX | SAVED),
    port("ai_order_k1b_payload_init", 0x0045_CA10, ECX | EDX | SAVED),
    port("ai_order_k0b_payload_init", 0x0041_ED60, ECX | EDX | SAVED),
    port("ai_order_k22_payload_init", 0x0042_4930, SAVED),
    port("ai_order_k0a_payload_init", 0x0041_B5A0, EDX | SAVED),
    port("ai_order_k0d_payload_init", 0x0042_C420, SAVED),
    port("ai_order_k13_payload_init", 0x0045_43A0, EDX | SAVED),
    port("ai_order_k18_payload_init", 0x0045_9FB0, EDX | SAVED),
    port("ai_order_k04_payload_init", 0x0040_97F0, ECX | EDX | SAVED),
    port("ai_order_k1a_payload_init", 0x0045_BC20, SAVED),
    port("ai_order_k02_payload_init", 0x0040_4020, SAVED),
    port("ai_order_k20_net_write", 0x0046_2A60, SAVED),
    port("ai_order_k14_net_write", 0x0045_5FE0, SAVED),
    port("ai_order_k08_net_write", 0x0041_5E80, SAVED),
    port("ai_order_k19_net_write", 0x0045_AB00, SAVED),
    port("ai_order_k01_net_write", 0x0040_25E0, SAVED),
    port("ai_order_k15_net_write", 0x0045_7E80, SAVED),
    port("ai_order_k05_net_write", 0x0040_D310, SAVED),
    port("ai_order_k06_net_write", 0x0041_0B20, SAVED),
    port("ai_order_k1c_net_write", 0x0045_DC40, SAVED),
    port("ai_order_k0c_net_write", 0x0042_7F00, SAVED),
    port("ai_order_k17_net_write", 0x0045_95B0, SAVED),
    port("ai_order_k19_enter", 0x0045_AC50, SAVED),
    port("ai_order_k23_enter", 0x0045_8690, SAVED),
    port("ai_order_k20_enter", 0x0046_2B70, SAVED),
    port("ai_order_k16_enter", 0x0045_9340, SAVED),
    port("ai_order_k15_exit", 0x0045_8240, ECX | EDX | SAVED),
    port("ai_order_k0f_exit", 0x0043_1010, SAVED),
    port("ai_order_k10_exit", 0x0043_2200, SAVED),
    port("ai_order_k1e_exit", 0x0046_1750, SAVED),
    port("ai_order_k03_on_event", 0x0040_6650, SAVED),
    port("ai_order_k0c_on_event", 0x0042_8C60, SAVED),
    port("ai_order_k17_on_event", 0x0045_9930, SAVED),
    port("ai_order_k13_on_event", 0x0045_5060, SAVED),
    port("ai_order_k20_on_event", 0x0046_2CD0, SAVED),
    port("ai_order_k10_on_event", 0x0043_2360, SAVED),
    port("ai_order_k1d_on_event", 0x0046_0D00, SAVED),
    port("ai_order_k09_update", 0x0041_7BC0, SAVED),
    port("ai_order_interrupt_query", 0x0043_7890, SAVED),
    port("ai_order_k19_net_read", 0x0045_AB50, SAVED),
    port("ai_order_k01_net_read", 0x0040_2640, SAVED),
    port("ai_order_k05_net_read", 0x0040_D3A0, SAVED),
    port("ai_order_k14_net_read", 0x0045_6020, SAVED),
    port("ai_order_k06_net_read", 0x0041_0BC0, SAVED),
    port("ai_order_k11_net_read", 0x0043_2870, SAVED),
    port("ai_order_k1f_net_read", 0x0046_2320, SAVED),
    port("ai_order_k15_net_read", 0x0045_7F00, SAVED),
    port("ai_order_k21_net_read", 0x0046_3380, SAVED),
    port("ai_order_k00_payload_init", 0x0044_69A0, SAVED),
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
unsafe fn replay(tape: &[Vec<u32>], esi: u32, edi: u32) -> Option<u32> {
    let mut last = None;
    for e in tape {
        unsafe {
            match e[0] {
                T_GAZE => {
                    last = Some(call_regs(GAZE_SET_TARGET, e[1], e[2], e[3], esi, edi, &[e[4], e[5]]));
                }
                T_STOP => {
                    last = Some(call_regs(STOP_HELPER, e[1], 0, 0, esi, edi, &[e[1], e[2]]));
                }
                T_IDLE_CLEAR => {
                    last = Some(call_regs(IDLE_CLEAR, e[1], 0, 0, esi, edi, &[]));
                }
                T_ANIM => {
                    wd(a(e[1], 0x32C), e[6]);
                    last = Some(call_regs(ANIM_SET_UPDATE, 0, 0, 0, esi, edi, &[e[1], e[2], e[3], e[4], e[5]]));
                }
                T_GAZE_CLEAR => {
                    last = Some(call_regs(GAZE_CLEAR, e[1], 0, 0, esi, edi, &[]));
                }
                // [T_GEN, callee, eax, ecx, edx, esi, edi, args...]: the registers as the original had them.
                T_GEN => {
                    last = Some(call_regs(GEN_CALLEES[e[1] as usize].0, e[2], e[3], e[4], e[5], e[6], &e[7..]));
                }
                // [T_CANCEL, type, arg0, arg1]: ai_event_cancel (Rust), whose registers nobody reads.
                T_CANCEL => event_cancel(e[1], e[2], e[3]),
                _ => {}
            }
        }
    }
    last
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

// --- constant handlers: bare `ret` stubs, `xor eax, eax`, `mov eax, 2` -------------------------------------
//
// The originals are 1, 3 and 6 bytes long. A detour needs 5, so each is pinned together with the
// int3 padding that follows it (never executed), and the thunks do exactly what the original does,
// without a `pushad` (no register but EAX is touched, and EAX only where the original does).

#[unsafe(naked)]
extern "C" fn nop_thunk() {
    std::arch::naked_asm!("ret")
}

#[unsafe(naked)]
extern "C" fn zero_thunk() {
    std::arch::naked_asm!("xor eax, eax", "ret")
}

#[unsafe(naked)]
extern "C" fn two_thunk() {
    std::arch::naked_asm!("mov eax, 2", "ret")
}

// --- more payload initialisers (cdecl (payload*) -> EAX = payload) ---------------------------------------------

/// The value a payload initialiser leaves in (ECX, EDX) when it sets one.
type Outs = (Option<u32>, Option<u32>);
type FillX = unsafe fn(usize) -> Outs;

const N1: u32 = u32::MAX;
const CLOCK: usize = 0x00E8_43AC;
const F_E87998: usize = 0x00E8_7998;
const SCRATCH: usize = 0x80;

unsafe fn ww(p: usize, v: u16) {
    unsafe { std::ptr::write_unaligned(p as *mut u16, v) }
}

unsafe fn zeros(p: usize, offs: &[usize]) {
    for &o in offs {
        unsafe { wd(p + o, 0) };
    }
}

/// `fld dword / fstp dword`: a signalling NaN comes back quiet, everything else unchanged.
fn x87_rt(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

/// `cmp v, 0x6B49D200 / jle / add v, 0x94B62E00`: the clock wrap (signed compare).
fn clock_wrap(v: u32) -> u32 {
    if (v as i32) > 0x6B49_D200 { v.wrapping_add(0x94B6_2E00) } else { v }
}

/// EAX = the payload. Shadow: the original and Rust fill scratch buffers, EAX / ECX / EDX are
/// compared (a register the fill does not set must come back unchanged) and the bytes the original
/// wrote go to the payload.
fn payload_x(r: &mut Pushad, args: *const u32, id: usize, fill: FillX) {
    let pre = *r;
    let p = unsafe { *args };
    r.eax = p;
    let Some((n, orig)) = shadowing(id) else {
        let (c, d) = unsafe { fill(p as usize) };
        if let Some(c) = c {
            r.ecx = c;
        }
        if let Some(d) = d {
            r.edx = d;
        }
        return;
    };
    let mut theirs = [FILL_MARK; SCRATCH];
    let mut ours = [FILL_MARK; SCRATCH];
    let inp = Regs::of(&pre);
    let mut out = inp;
    let (c, d) = unsafe {
        call_full(orig, &mut out, theirs.as_mut_ptr() as u32, false);
        fill(ours.as_mut_ptr() as usize)
    };
    check_preserved(id, &inp, &out);
    let (ec, ed) = (c.unwrap_or(pre.ecx), d.unwrap_or(pre.edx));
    if theirs != ours || out.eax != theirs.as_ptr() as u32 || out.ecx != ec || out.edx != ed {
        mismatch(id, n, format!("original {theirs:x?} eax {:#x} ecx {:#x} edx {:#x}, Rust {ours:x?} ecx {ec:#x} edx {ed:#x}", out.eax, out.ecx, out.edx));
    }
    r.ecx = out.ecx;
    r.edx = out.edx;
    for (i, &b) in theirs.iter().enumerate() {
        if b != FILL_MARK {
            unsafe { wb(p as usize + i, b) };
        }
    }
}

macro_rules! payload_x_port {
    ($entry:ident, $thunk:ident, $id:expr, $fill:ident) => {
        extern "C" fn $entry(r: &mut Pushad, args: *const u32) {
            payload_x(r, args, $id, $fill);
        }
        regs_thunk!($thunk, $entry);
    };
}

/// Kind 0x20 (0x462A50): byte 0 = 0.
unsafe fn fx_k20(p: usize) -> Outs {
    unsafe { wb(p, 0) };
    (None, None)
}

/// Kinds 0x15 and 0x21 (0x457E70, 0x4632F0): dword 0 = 0.
unsafe fn fx_k15(p: usize) -> Outs {
    unsafe { wd(p, 0) };
    (None, None)
}

/// Kind 0x17 (0x459590): dword 0 = -1, dword +4 = 0.
unsafe fn fx_k17(p: usize) -> Outs {
    unsafe {
        wd(p, N1);
        wd(p + 4, 0);
    }
    (None, None)
}

/// Kind 0x1C (0x45DC20): byte 0 = 0, dword +8 = -1, dword +4 = 0.
unsafe fn fx_k1c(p: usize) -> Outs {
    unsafe {
        wb(p, 0);
        wd(p + 8, N1);
        wd(p + 4, 0);
    }
    (None, None)
}

/// Kind 0x0F (0x430A80): byte 0 = 0, zeros at +4 .. +0x10.
unsafe fn fx_k0f(p: usize) -> Outs {
    unsafe {
        wb(p, 0);
        zeros(p, &[4, 8, 0xC, 0x10]);
    }
    (None, None)
}

/// Kind 0x1D (0x4601E0): byte 0 = 0, zeros at +4, +8, +0x10, +0x14, +0x18; ECX = 0.
unsafe fn fx_k1d(p: usize) -> Outs {
    unsafe {
        wb(p, 0);
        zeros(p, &[8, 4, 0x10, 0x14, 0x18]);
    }
    (Some(0), None)
}

/// Kind 0x09 (0x417970): zeros at 0, +4, +8, +0xC, dword +0x10 = -1; ECX = 0.
unsafe fn fx_k09(p: usize) -> Outs {
    unsafe {
        zeros(p, &[4, 0, 8, 0xC]);
        wd(p + 0x10, N1);
    }
    (Some(0), None)
}

/// Kind 0x0E (0x42F940): dword 0 = 0, dword +4 = -1, zeros at +8, +0xC, +0x10.
unsafe fn fx_k0e(p: usize) -> Outs {
    unsafe {
        wd(p, 0);
        wd(p + 4, N1);
        zeros(p, &[8, 0xC, 0x10]);
    }
    (None, None)
}

/// Kind 0x10 (0x431BE0): zeros at 0, +4, +8, +0xC, +0x10.
unsafe fn fx_k10(p: usize) -> Outs {
    unsafe { zeros(p, &[4, 0, 8, 0xC, 0x10]) };
    (None, None)
}

/// Kind 0x0C (0x427EE0): byte 0 = 0, zeros at +4 .. +0x10 and +0x14, word +0x18 = 0.
unsafe fn fx_k0c(p: usize) -> Outs {
    unsafe {
        wb(p, 0);
        zeros(p, &[4, 8, 0xC, 0x10]);
        ww(p + 0x18, 0);
        wd(p + 0x14, 0);
    }
    (None, None)
}

/// Kind 0x1B (0x45CA10): dword +0x1C = -1, zeros at 0, +4 .. +0x18.
unsafe fn fx_k1b(p: usize) -> Outs {
    unsafe {
        wd(p + 0x1C, N1);
        zeros(p, &[0x10, 0x14, 0x18, 4, 8, 0xC, 0]);
    }
    (None, None)
}

/// Kind 0x0B (0x41ED60): byte 0 = 0, zeros at +4 .. +0x28 (all but +0x1C are f32 zeros).
unsafe fn fx_k0b(p: usize) -> Outs {
    unsafe {
        wb(p, 0);
        zeros(p, &[4, 8, 0xC, 0x20, 0x24, 0x28, 0x10, 0x14, 0x18, 0x1C]);
    }
    (None, None)
}

/// Kind 0x22 (0x424930): ECX = -1, EDX = 0.
unsafe fn fx_k22(p: usize) -> Outs {
    unsafe {
        wb(p, 0);
        wd(p + 0x14, 0);
        zeros(p, &[4, 8]);
        wb(p + 0x18, 0);
        for o in [0xC, 0x10, 0x1C, 0x28, 0x2C] {
            wd(p + o, N1);
        }
        wd(p + 0x30, 0);
    }
    (Some(N1), Some(0))
}

/// Kind 0x0A (0x41B5A0): word 0 = 0, byte +2 = 0, zeros at +4, +8, +0xC .. +0x20, dword +0x24 = -1;
/// ECX = 0.
unsafe fn fx_k0a(p: usize) -> Outs {
    unsafe {
        ww(p, 0);
        wb(p + 2, 0);
        zeros(p, &[4, 8, 0xC, 0x10, 0x14, 0x18, 0x1C, 0x20]);
        wd(p + 0x24, N1);
    }
    (Some(0), None)
}

/// Kind 0x0D (0x42C420): ECX = 0, EDX = -1.
unsafe fn fx_k0d(p: usize) -> Outs {
    unsafe {
        ww(p, 0);
        wb(p + 2, 0);
        zeros(p, &[0x10, 0x28, 0x14, 0x18, 0x1C, 4, 8, 0xC]);
        for o in [0x20, 0x24, 0x2C] {
            wd(p + o, N1);
        }
        ww(p + 0x30, 0);
    }
    (Some(0), Some(N1))
}

/// Kind 0x13 (0x4543A0): dword 0 = dword +0x14 = -1, zeros at +4, +8, +0x10, byte +0x18 = 0, dword
/// +0xC = the wrapped clock (also left in ECX).
unsafe fn fx_k13(p: usize) -> Outs {
    unsafe {
        zeros(p, &[4, 0x10, 8]);
        wb(p + 0x18, 0);
        let v = clock_wrap(rd(CLOCK));
        wd(p, N1);
        wd(p + 0x14, N1);
        wd(p + 0xC, v);
        (Some(v), None)
    }
}

/// Kind 0x18 (0x459FB0): byte 0 = 0, zeros at +4 .. +0x10, dword +0x14 = the wrapped clock + 1500
/// (also left in ECX).
unsafe fn fx_k18(p: usize) -> Outs {
    unsafe {
        let v = clock_wrap(rd(CLOCK).wrapping_add(0x5DC));
        wb(p, 0);
        zeros(p, &[4, 8, 0xC, 0x10]);
        wd(p + 0x14, v);
        (Some(v), None)
    }
}

/// Kind 0x04 (0x4097F0): bytes 0 / 1 = 1 / 0, zeros at +4 .. +0xC and +0x14 .. +0x28, dword +0x10 =
/// -1, dword +0x2C = 0, +0x30 = the f32 at 0xE87998.
unsafe fn fx_k04(p: usize) -> Outs {
    unsafe {
        ww(p, 0);
        wb(p, 1);
        zeros(p, &[4, 8, 0xC, 0x14, 0x18, 0x1C, 0x20, 0x24, 0x28]);
        wd(p + 0x10, N1);
        wd(p + 0x2C, 0);
        wd(p + 0x30, x87_rt(rd(F_E87998)));
    }
    (None, None)
}

/// Kind 0x1A (0x45BC20): the three globals at 0x252A6D0 are copied to +0x10 and +0x20; ECX / EDX end as
/// the second / third.
unsafe fn fx_k1a(p: usize) -> Outs {
    unsafe {
        let (d0, d4, d8) = (rd(0x0252_A6D0), rd(0x0252_A6D4), rd(0x0252_A6D8));
        wd(p + 4, 0);
        wd(p + 8, 0);
        wd(p, N1);
        wd(p + 0x1C, N1);
        wd(p + 0x10, d0);
        wd(p + 0x14, d4);
        wd(p + 0x18, d8);
        wd(p + 0x20, d0);
        wd(p + 0x24, d4);
        wb(p + 0xC, 8);
        wd(p + 0x28, d8);
        (Some(d4), Some(d8))
    }
}

/// Kind 0x02 (0x404020): ECX = the wrapped clock, EDX = 0.
unsafe fn fx_k02(p: usize) -> Outs {
    unsafe {
        let v = clock_wrap(rd(CLOCK));
        wb(p, 0);
        wd(p + 4, 0);
        wd(p + 0x10, 0xFFFF);
        wd(p + 8, v);
        wb(p + 0xC, 0);
        wb(p + 0xD, 0);
        wd(p + 0x18, x87_rt(rd(F_E87998)));
        wd(p + 0x14, 0);
        (Some(v), Some(0))
    }
}

payload_x_port!(px_k20_entry, px_k20_thunk, PX_K20, fx_k20);
payload_x_port!(px_k15_entry, px_k15_thunk, PX_K15, fx_k15);
payload_x_port!(px_k17_entry, px_k17_thunk, PX_K17, fx_k17);
payload_x_port!(px_k1c_entry, px_k1c_thunk, PX_K1C, fx_k1c);
payload_x_port!(px_k0f_entry, px_k0f_thunk, PX_K0F, fx_k0f);
payload_x_port!(px_k1d_entry, px_k1d_thunk, PX_K1D, fx_k1d);
payload_x_port!(px_k09_entry, px_k09_thunk, PX_K09, fx_k09);
payload_x_port!(px_k0e_entry, px_k0e_thunk, PX_K0E, fx_k0e);
payload_x_port!(px_k10_entry, px_k10_thunk, PX_K10, fx_k10);
payload_x_port!(px_k0c_entry, px_k0c_thunk, PX_K0C, fx_k0c);
payload_x_port!(px_k1b_entry, px_k1b_thunk, PX_K1B, fx_k1b);
payload_x_port!(px_k0b_entry, px_k0b_thunk, PX_K0B, fx_k0b);
payload_x_port!(px_k22_entry, px_k22_thunk, PX_K22, fx_k22);
payload_x_port!(px_k0a_entry, px_k0a_thunk, PX_K0A, fx_k0a);
payload_x_port!(px_k0d_entry, px_k0d_thunk, PX_K0D, fx_k0d);
payload_x_port!(px_k13_entry, px_k13_thunk, PX_K13, fx_k13);
payload_x_port!(px_k18_entry, px_k18_thunk, PX_K18, fx_k18);
payload_x_port!(px_k04_entry, px_k04_thunk, PX_K04, fx_k04);
payload_x_port!(px_k1a_entry, px_k1a_thunk, PX_K1A, fx_k1a);
payload_x_port!(px_k02_entry, px_k02_thunk, PX_K02, fx_k02);

// --- network writers (cdecl (human, buf, u16 *pos)) -------------------------------------------------------------
//
// Each handler appends bytes to `buf + *pos` and advances the u16 `*pos`. The callers
// (`ai_order_net_write`) ignore EAX and keep nothing in ECX / EDX, so the thunks keep every register.

/// The output cursor: `buf` and the address of the u16 position.
struct Wr {
    buf: usize,
    pos: usize,
}

impl Wr {
    unsafe fn cur(&self) -> usize {
        self.buf.wrapping_add(unsafe { std::ptr::read_unaligned(self.pos as *const u16) } as usize)
    }
    unsafe fn adv(&self, n: u16) {
        unsafe {
            let v = std::ptr::read_unaligned(self.pos as *const u16);
            std::ptr::write_unaligned(self.pos as *mut u16, v.wrapping_add(n));
        }
    }
    unsafe fn b(&self, v: u8) {
        unsafe {
            wb(self.cur(), v);
            self.adv(1);
        }
    }
    unsafe fn w(&self, v: u16) {
        unsafe {
            std::ptr::write_unaligned(self.cur() as *mut u16, v);
            self.adv(2);
        }
    }
    unsafe fn d(&self, v: u32) {
        unsafe {
            std::ptr::write_unaligned(self.cur() as *mut u32, v);
            self.adv(4);
        }
    }
}

/// The 16-bit network id of an entity handle: 0xFFFF for a null handle; otherwise the word at +2
/// of the record `[entity + 0xB0]` of the slot `[0x2149C64 + (handle & 0xFFF) * 16]` when that
/// slot's handle (+0x44) matches, falling back to the handle's low word when that is absent or
/// 0xFFFF.
unsafe fn net_id16(handle: u32) -> u16 {
    unsafe {
        if handle == 0 {
            return 0xFFFF;
        }
        let ent = rd(0x0214_9C64 + (((handle & 0xFFF) << 4) as usize)) as usize;
        let mut v = 0xFFFFu16;
        if ent != 0 && rd(ent + 0x44) == handle {
            let q = rd(ent + 0xB0) as usize;
            if q != 0 {
                v = rw(q + 2);
            }
        }
        if v == 0xFFFF { handle as u16 } else { v }
    }
}

const NWS: usize = 0x400;
/// The human bytes a net writer might touch (the order payload).
const NW_SNAP: usize = 0x60;

unsafe fn snap(h: usize) -> [u8; NW_SNAP] {
    unsafe { std::ptr::read_unaligned((h + 0x150) as *const [u8; NW_SNAP]) }
}

unsafe fn unsnap(h: usize, s: &[u8; NW_SNAP]) {
    unsafe { std::ptr::write_unaligned((h + 0x150) as *mut [u8; NW_SNAP], *s) }
}

/// Shadow: the original (writing into a scratch buffer placed so that `buf + pos` is its start) and
/// Rust (into another) both run, the two scratch buffers, the final positions and the payload bytes
/// of the human are compared, and the original's output is what ends up in the caller's buffer.
fn nw(id: usize, r: &mut Pushad, args: *const u32, body: unsafe fn(usize, &Wr)) {
    let p = *r;
    let (h, buf, pos) = unsafe { (*args as usize, *args.add(1) as usize, *args.add(2) as usize) };
    let real = Wr { buf, pos };
    let Some((n, orig)) = shadowing(id) else { return unsafe { body(h, &real) } };
    unsafe {
        let pos0 = std::ptr::read_unaligned(pos as *const u16);
        if pos0 > 0xF000 {
            return body(h, &real);
        }
        let before = snap(h);
        let mut theirs = [FILL_MARK; NWS];
        let mut ours = [FILL_MARK; NWS];
        let (mut tp, mut op) = (pos0, pos0);
        let tb = (theirs.as_mut_ptr() as usize).wrapping_sub(pos0 as usize);
        let ob = (ours.as_mut_ptr() as usize).wrapping_sub(pos0 as usize);
        call_regs(orig, p.eax, p.ecx, p.edx, p.esi, p.edi, &[h as u32, tb as u32, &mut tp as *mut u16 as u32]);
        let h_theirs = snap(h);
        unsnap(h, &before);
        body(h, &Wr { buf: ob, pos: &mut op as *mut u16 as usize });
        let h_ours = snap(h);
        if theirs != ours || tp != op || h_theirs != h_ours {
            mismatch(id, n, format!("human {h:#x} pos {pos0}: original end {tp} {:x?}, Rust end {op} {:x?}", &theirs[..48], &ours[..48]));
            unsnap(h, &h_theirs);
        }
        let len = tp.wrapping_sub(pos0) as usize;
        if len <= NWS {
            std::ptr::copy_nonoverlapping(theirs.as_ptr(), buf.wrapping_add(pos0 as usize) as *mut u8, len);
        }
        std::ptr::write_unaligned(pos as *mut u16, tp);
    }
}

macro_rules! nw_port {
    ($entry:ident, $thunk:ident, $id:expr, $body:ident) => {
        extern "C" fn $entry(r: &mut Pushad, args: *const u32) {
            nw($id, r, args, $body);
        }
        regs_thunk!($thunk, $entry);
    };
}

/// Kind 0x20 (0x462A60): byte +0x150.
unsafe fn nw_body_k20(h: usize, w: &Wr) {
    unsafe { w.b(rb(h + 0x150)) }
}

/// Kind 0x14 (0x455FE0): byte +0x150; when dword +0xAF0 is non-zero also dword +0x154.
unsafe fn nw_body_k14(h: usize, w: &Wr) {
    unsafe {
        w.b(rb(h + 0x150));
        if rd(h + 0xAF0) != 0 {
            w.d(rd(h + 0x154));
        }
    }
}

/// Kind 0x08 (0x415E80): with a record at +0x330 whose dword +8 is not -1: its bytes +0x34 and +8.
unsafe fn nw_body_k08(h: usize, w: &Wr) {
    unsafe {
        let c = rd(h + 0x330) as usize;
        if c == 0 || rd(c + 8) == N1 {
            return;
        }
        w.b(rb(c + 0x34));
        w.b(rb(c + 8));
    }
}

/// Kind 0x19 (0x45AB00): two f32 through the x87 (+0x150, +0x154).
unsafe fn nw_body_k19(h: usize, w: &Wr) {
    unsafe {
        w.d(x87_rt(rd(h + 0x150)));
        w.d(x87_rt(rd(h + 0x154)));
    }
}

/// Kind 0x01 (0x4025E0): three f32 through the x87 (+0x150, +0x154, +0x158).
unsafe fn nw_body_k01(h: usize, w: &Wr) {
    unsafe {
        for o in [0x150, 0x154, 0x158] {
            w.d(x87_rt(rd(h + o)));
        }
    }
}

/// Kinds 0x15, 0x1B, 0x21 (0x457E80, 0x45CA40, 0x463300): the network id of the handle at +0x150.
unsafe fn nw_body_k15(h: usize, w: &Wr) {
    unsafe { w.w(net_id16(rd(h + 0x150))) }
}

/// Kind 0x05 (0x40D310): bytes +0x150 .. +0x155.
unsafe fn nw_body_k05(h: usize, w: &Wr) {
    unsafe {
        for o in 0..6 {
            w.b(rb(h + 0x150 + o));
        }
    }
}

/// Kinds 0x06, 0x07, 0x11, 0x18, 0x1F (0x410B20 and four copies): byte +0x150, the network id of the
/// handle at +0x154.
unsafe fn nw_body_k06(h: usize, w: &Wr) {
    unsafe {
        w.b(rb(h + 0x150));
        w.w(net_id16(rd(h + 0x154)));
    }
}

/// Kind 0x1C (0x45DC40): bytes +0x150 and +0x158, the network id of the handle at +0x154.
unsafe fn nw_body_k1c(h: usize, w: &Wr) {
    unsafe {
        w.b(rb(h + 0x150));
        w.b(rb(h + 0x158));
        w.w(net_id16(rd(h + 0x154)));
    }
}

/// Kind 0x0C (0x427F00): byte +0x150, five f32 through the x87 (+0x154 .. +0x164), then the word at
/// +0x168, which is cleared afterwards (a write to the human).
unsafe fn nw_body_k0c(h: usize, w: &Wr) {
    unsafe {
        w.b(rb(h + 0x150));
        for o in [0x154, 0x158, 0x15C, 0x160, 0x164] {
            w.d(x87_rt(rd(h + o)));
        }
        w.w(rw(h + 0x168));
        ww(h + 0x168, 0);
    }
}

/// Kind 0x17 (0x4595B0): dword +0x150, the network id of the handle at +0x154, then (handle non-zero)
/// bytes +0x158 and +0x15C.
unsafe fn nw_body_k17(h: usize, w: &Wr) {
    unsafe {
        w.d(rd(h + 0x150));
        let hd = rd(h + 0x154);
        w.w(net_id16(hd));
        if hd != 0 {
            w.b(rb(h + 0x158));
            w.b(rb(h + 0x15C));
        }
    }
}

nw_port!(nw_k20_entry, nw_k20_thunk, NW_K20, nw_body_k20);
nw_port!(nw_k14_entry, nw_k14_thunk, NW_K14, nw_body_k14);
nw_port!(nw_k08_entry, nw_k08_thunk, NW_K08, nw_body_k08);
nw_port!(nw_k19_entry, nw_k19_thunk, NW_K19, nw_body_k19);
nw_port!(nw_k01_entry, nw_k01_thunk, NW_K01, nw_body_k01);
nw_port!(nw_k15_entry, nw_k15_thunk, NW_K15, nw_body_k15);
nw_port!(nw_k05_entry, nw_k05_thunk, NW_K05, nw_body_k05);
nw_port!(nw_k06_entry, nw_k06_thunk, NW_K06, nw_body_k06);
nw_port!(nw_k1c_entry, nw_k1c_thunk, NW_K1C, nw_body_k1c);
nw_port!(nw_k0c_entry, nw_k0c_thunk, NW_K0C, nw_body_k0c);
nw_port!(nw_k17_entry, nw_k17_thunk, NW_K17, nw_body_k17);

// --- call-sequence handlers with generic callee stubs ------------------------------------------------------------
//
// enter / exit / on_event / update handlers that make a few calls and write a few fields. The shadow copy
// of the original runs with every callee call pointed at a recording stub (the five registers and
// the stack arguments are recorded; `ai_event_cancel` calls go to the unreplaced copy of the
// cancel, and the event list is snapshotted), the recorded calls are compared with Rust's plan, the human
// fields the handler writes (`watch`) and the event nodes are compared after a dry run of the Rust
// side, and the winning plan is then made for real, in the original's order.

const GEN_CALLEES: [(usize, usize); 13] = [
    (0x0043_75B0, 3), // ai_order_set_action(human, expected kind, new action)
    (0x0040_51F0, 0), // EAX = human
    (0x0040_4A90, 0), // EDI = human
    (0x0045_CBD0, 1), // (value)
    (0x0045_9400, 0), // EAX = human, ECX = event, EDX = 0x59
    (0x0045_3F40, 1), // (handle)
    (0x0043_1820, 0), // EAX = human
    (0x0043_E2C0, 0), // EAX = event, ECX = human
    (0x0044_7CD0, 3), // (human, 0, 0)
    (0x0041_7450, 3), // (human, payload, entity)
    (0x0096_FCA0, 8), // ECX = human, 8 stack arguments
    (0x0045_6070, 4), // ECX = human, (human, ?, buffer, position pointer)
    (0x00CA_0BC0, 3), // memset(dst, value, size)
];
const G_SETACT: u32 = 0;
const G_4051F0: u32 = 1;
const G_404A90: u32 = 2;
const G_45CBD0: u32 = 3;
const G_459400: u32 = 4;
const G_453F40: u32 = 5;
const G_431820: u32 = 6;
const G_43E2C0: u32 = 7;
const G_447CD0: u32 = 8;
const G_417450: u32 = 9;
const G_96FCA0: u32 = 10;

const T_GEN: u32 = 7;
const T_CANCEL: u32 = 8;
const T_EV: u32 = 9; // [id, human, event]

fn gen_rec(idx: u32, r: &mut Pushad, args: *const u32) {
    let n = GEN_CALLEES[idx as usize].1;
    let mut v = vec![T_GEN, idx, r.eax, r.ecx, r.edx, r.esi, r.edi];
    for i in 0..n {
        v.push(unsafe { *args.add(i) });
    }
    tape_push(v);
}

macro_rules! gen_stub {
    ($entry:ident, $thunk:ident, $idx:expr) => {
        extern "C" fn $entry(r: &mut Pushad, args: *const u32) {
            gen_rec($idx, r, args);
        }
        regs_thunk!($thunk, $entry);
    };
}

gen_stub!(gs_setact_entry, gs_setact, G_SETACT);
gen_stub!(gs_4051f0_entry, gs_4051f0, G_4051F0);
gen_stub!(gs_404a90_entry, gs_404a90, G_404A90);
gen_stub!(gs_45cbd0_entry, gs_45cbd0, G_45CBD0);
gen_stub!(gs_459400_entry, gs_459400, G_459400);
gen_stub!(gs_453f40_entry, gs_453f40, G_453F40);
gen_stub!(gs_431820_entry, gs_431820, G_431820);
gen_stub!(gs_43e2c0_entry, gs_43e2c0, G_43E2C0);
gen_stub!(gs_447cd0_entry, gs_447cd0, G_447CD0);
gen_stub!(gs_417450_entry, gs_417450, G_417450);
gen_stub!(gs_96fca0_entry, gs_96fca0, G_96FCA0);
gen_stub!(gs_456070_entry, gs_456070, G_456070);
gen_stub!(gs_memset_entry, gs_memset, G_MEMSET);

unsafe fn rdu(a: usize) -> u32 {
    unsafe { std::ptr::read_unaligned(a as *const u32) }
}

unsafe fn wdu(a: usize, v: u32) {
    unsafe { std::ptr::write_unaligned(a as *mut u32, v) }
}

/// A `T_GEN` plan entry.
fn gcall(idx: u32, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> Vec<u32> {
    let mut v = vec![T_GEN, idx, eax, ecx, edx, esi, edi];
    v.extend_from_slice(args);
    v
}

/// Run a call-sequence port. `plan` is the original's ordered list of callee calls (`T_*` entries;
/// `T_CANCEL` entries are the event cancels the copy performs for real), `pre` / `post` the field
/// writes before / after them, `watch` the dwords those writes (and the callee-free flow) can touch,
/// `ret` the EAX the original ends with when it is not a callee's (checked against the copy). Returns
/// the EAX to leave (`ret`, else the last made call's).
unsafe fn seq_port(
    id: usize,
    p: &Pushad,
    args: *const u32,
    nargs: usize,
    watch: &[usize],
    pre: &dyn Fn(),
    plan: Vec<Vec<u32>>,
    post: &dyn Fn(),
    ret: Option<u32>,
) -> Option<u32> {
    unsafe {
        let mut tape = plan.clone();
        let mut eax = ret;
        let mut use_theirs = false;
        if let Some((n, orig)) = shadowing(id) {
            let nodes = event_nodes();
            let n0 = node_types(&nodes);
            let w0: Vec<u32> = watch.iter().map(|&a| rdu(a)).collect();
            tape_take();
            let av: Vec<u32> = (0..nargs).map(|i| *args.add(i)).collect();
            let e = call_regs(orig, p.eax, p.ecx, p.edx, p.esi, p.edi, &av);
            let rec = tape_take();
            let n_theirs = node_types(&nodes);
            let w_theirs: Vec<u32> = watch.iter().map(|&a| rdu(a)).collect();
            restore_types(&nodes, &n0);
            for (&a, &v) in watch.iter().zip(&w0) {
                wdu(a, v);
            }
            // The Rust side as a dry run: the writes and the cancels, none of the other calls.
            pre();
            for t in plan.iter().filter(|t| t[0] == T_CANCEL) {
                event_cancel(t[1], t[2], t[3]);
            }
            post();
            let n_ours = node_types(&nodes);
            let w_ours: Vec<u32> = watch.iter().map(|&a| rdu(a)).collect();
            let plan_calls: Vec<Vec<u32>> = plan.iter().filter(|t| t[0] != T_CANCEL).cloned().collect();
            let mut bad = Vec::new();
            if rec != plan_calls {
                bad.push(format!("calls: original {rec:x?}, Rust {plan_calls:x?}"));
            }
            if n_theirs != n_ours {
                bad.push("event nodes differ".to_string());
            }
            if w_theirs != w_ours {
                bad.push(format!("fields: original {w_theirs:x?}, Rust {w_ours:x?}"));
            }
            if ret.is_some_and(|r| r != e) {
                bad.push(format!("eax: original {e:#x}, Rust {:#x}", ret.unwrap_or(0)));
            }
            if bad.is_empty() {
                // Undo the dry run; the real run below repeats the writes in the original's order.
                restore_types(&nodes, &n0);
                for (&a, &v) in watch.iter().zip(&w0) {
                    wdu(a, v);
                }
            } else {
                mismatch(id, n, bad.join("; "));
                restore_types(&nodes, &n_theirs);
                for (&a, &v) in watch.iter().zip(&w_theirs) {
                    wdu(a, v);
                }
                tape = rec;
                eax = Some(e);
                use_theirs = true;
            }
        }
        // The copy already did its own writes and cancels when it won; only its calls are left.
        if !use_theirs {
            pre();
        }
        let last = replay(&tape, p.esi, p.edi);
        if !use_theirs {
            post();
        }
        eax.or(last)
    }
}

// --- enters that set the sub-action ---------------------------------------------------------------------------

fn enter_setact(id: usize, r: &mut Pushad, args: *const u32, kind: u32, act: u32) {
    let p = *r;
    unsafe {
        let h = *args;
        let plan = vec![gcall(G_SETACT, p.eax, p.ecx, p.edx, p.esi, p.edi, &[h, kind, act])];
        // ai_order_set_action(human, kind, action) cdecl with EAX = the caller's.
        if let Some(e) = seq_port(id, &p, args, 1, &[], &|| {}, plan, &|| {}, None) {
            r.eax = e;
        }
    }
}

/// Kind 0x19 enter (0x45AC50): ai_order_set_action(human, 0x19, 0xAB).
extern "C" fn en_k19_entry(r: &mut Pushad, args: *const u32) {
    enter_setact(EN_K19, r, args, 0x19, 0xAB);
}
/// Kind 0x23 enter (0x458690): ai_order_set_action(human, 0x23, 0xA3).
extern "C" fn en_k23_entry(r: &mut Pushad, args: *const u32) {
    enter_setact(EN_K23, r, args, 0x23, 0xA3);
}
regs_thunk!(en_k19_thunk, en_k19_entry);
regs_thunk!(en_k23_thunk, en_k23_entry);

/// Kind 0x20 enter (0x462B70): ESI = human; action 0xBE when bit 0 of byte +0x150 is set (then byte
/// +0xE80 |= 2 after the call), else 0xBF; ai_order_set_action(human, 0x20, action).
extern "C" fn en_k20_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let h = *args;
        let bit = rb(a(h, 0x150)) & 1 != 0;
        let plan = vec![gcall(G_SETACT, p.eax, p.ecx, p.edx, h, p.edi, &[h, 0x20, if bit { 0xBE } else { 0xBF }])];
        let post = || {
            if bit {
                wb(a(h, 0xE80), rb(a(h, 0xE80)) | 2);
            }
        };
        if let Some(e) = seq_port(EN_K20, &p, args, 1, &[a(h, 0xE80)], &|| {}, plan, &post, None) {
            r.eax = e;
        }
    }
}
regs_thunk!(en_k20_thunk, en_k20_entry);

/// Kind 0x16 enter (0x459340): 0x96FCA0 (ECX = ESI = human; 0x340, 0, 0, 0.0, 1.0, the f32 at 0xE87998,
/// 0, -1), then ai_order_set_action(human, 0x16, 0xA6).
extern "C" fn en_k16_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let h = *args;
        let f = x87_rt(rd(F_E87998));
        let plan = vec![
            gcall(G_96FCA0, p.eax, h, p.edx, h, p.edi, &[0x340, 0, 0, 0, 0x3F80_0000, f, 0, N1]),
            gcall(G_SETACT, p.eax, h, p.edx, h, p.edi, &[h, 0x16, 0xA6]),
        ];
        if let Some(e) = seq_port(EN_K16, &p, args, 1, &[], &|| {}, plan, &|| {}, None) {
            r.eax = e;
        }
    }
}
regs_thunk!(en_k16_thunk, en_k16_entry);

// --- exits -------------------------------------------------------------------------------------------------

/// Kind 0x15 exit (0x458240): dword +0xC0 &= ~0x80; EAX = human.
extern "C" fn ex_k15_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let h = *args;
        let pre = || wd(a(h, 0xC0), rd(a(h, 0xC0)) & 0xFFFF_FF7F);
        if let Some(e) = seq_port(EX_K15, &p, args, 1, &[a(h, 0xC0)], &pre, vec![], &|| {}, Some(h)) {
            r.eax = e;
        }
    }
}
regs_thunk!(ex_k15_thunk, ex_k15_entry);

/// Kinds 0x0F and 0x10 exits (0x431010, 0x432200): ai_gaze_clear(human) unless byte +0x140 has bit 2,
/// then ai_event_cancel(type 0x59, handle +0x44, handle +0x44). EAX is left as the cancel leaves it.
fn exit_gaze_cancel(id: usize, r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let h = *args;
        let mut plan = Vec::new();
        if rb(a(h, 0x140)) & 4 == 0 {
            plan.push(vec![T_GAZE_CLEAR, h]);
        }
        let hd = rd(a(h, 0x44));
        plan.push(vec![T_CANCEL, 0x59, hd, hd]);
        seq_port(id, &p, args, 1, &[], &|| {}, plan, &|| {}, None);
    }
}
extern "C" fn ex_k0f_entry(r: &mut Pushad, args: *const u32) {
    exit_gaze_cancel(EX_K0F, r, args);
}
extern "C" fn ex_k10_entry(r: &mut Pushad, args: *const u32) {
    exit_gaze_cancel(EX_K10, r, args);
}
regs_thunk!(ex_k0f_thunk, ex_k0f_entry);
regs_thunk!(ex_k10_thunk, ex_k10_entry);

/// Kind 0x1E exit (0x461750): ai_event_cancel(type, handle, handle) for the types 0x48, 0x6F, 0x6E
/// and 0x58 in that order, handle = dword +0x44 of the human.
extern "C" fn ex_k1e_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let h = *args;
        let hd = rd(a(h, 0x44));
        let plan = [0x48, 0x6F, 0x6E, 0x58].iter().map(|&t| vec![T_CANCEL, t, hd, hd]).collect();
        seq_port(EX_K1E, &p, args, 1, &[], &|| {}, plan, &|| {}, None);
    }
}
regs_thunk!(ex_k1e_thunk, ex_k1e_entry);

// --- on_event handlers (cdecl (human, event*) -> EAX) --------------------------------------------------------

/// Kind 0x03 (0x406650): event type 0x12 -> 0x404A90 (EAX = 0x12, EDI = human); 0x59 -> 0x4051F0
/// (EAX = human); EAX = 0.
extern "C" fn oe_k03_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (h, ev) = (*args, *args.add(1));
        let plan = match rd(a(ev, 8)) {
            0x12 => vec![gcall(G_404A90, 0x12, p.ecx, p.edx, p.esi, h, &[])],
            0x59 => vec![gcall(G_4051F0, h, p.ecx, p.edx, p.esi, p.edi, &[])],
            _ => vec![],
        };
        r.eax = seq_port(OE_K03, &p, args, 2, &[], &|| {}, plan, &|| {}, Some(0)).unwrap_or(0);
    }
}
regs_thunk!(oe_k03_thunk, oe_k03_entry);

/// Kind 0x0C (0x428C60): event type 0x1C -> EAX = 2 when byte +0x150 has bits 0 / 1 or byte +0x148 is
/// 0x76, else 0x45CBD0(event dword +0x24) and EAX = 0; other types EAX = 0.
extern "C" fn oe_k0c_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (h, ev) = (*args, *args.add(1));
        let mut plan = vec![];
        let mut ret = 0;
        if rd(a(ev, 8)) == 0x1C {
            if rb(a(h, 0x150)) & 3 != 0 || rb(a(h, 0x148)) == 0x76 {
                ret = 2;
            } else {
                let v = rd(a(ev, 0x24));
                plan.push(gcall(G_45CBD0, v, p.ecx, p.edx, h, p.edi, &[v]));
            }
        }
        r.eax = seq_port(OE_K0C, &p, args, 2, &[], &|| {}, plan, &|| {}, Some(ret)).unwrap_or(ret);
    }
}
regs_thunk!(oe_k0c_thunk, oe_k0c_entry);

/// Kind 0x17 (0x459930): type 5 -> when event dword +0xC differs from the human's handle +0x44:
/// +0x154 = it, +0x158 = event +0x24, byte +0x15C = event byte +0x28; type 0x59 -> 0x459400
/// (EAX = human, ECX = event, EDX = 0x59); EAX = 0.
extern "C" fn oe_k17_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (h, ev) = (*args, *args.add(1));
        let t = rd(a(ev, 8));
        let mut plan = vec![];
        if t == 0x59 {
            plan.push(gcall(G_459400, h, ev, 0x59, p.esi, p.edi, &[]));
        }
        let pre = || {
            if t == 5 && rd(a(ev, 0xC)) != rd(a(h, 0x44)) {
                wd(a(h, 0x154), rd(a(ev, 0xC)));
                wd(a(h, 0x158), rd(a(ev, 0x24)));
                wb(a(h, 0x15C), rb(a(ev, 0x28)));
            }
        };
        let watch = [a(h, 0x154), a(h, 0x158), a(h, 0x15C)];
        r.eax = seq_port(OE_K17, &p, args, 2, &watch, &pre, plan, &|| {}, Some(0)).unwrap_or(0);
    }
}
regs_thunk!(oe_k17_thunk, oe_k17_entry);

/// Kind 0x13 (0x455060): type 5 -> when event dword +0xC differs from handle +0x44: +0x160 = it, +0x164
/// = event +0x24, byte +0x168 = event byte +0x28; type 0x53 -> 0x453F40(dword +0x150); EAX = 0.
extern "C" fn oe_k13_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (h, ev) = (*args, *args.add(1));
        let t = rd(a(ev, 8));
        let mut plan = vec![];
        if t == 0x53 {
            let v = rd(a(h, 0x150));
            plan.push(gcall(G_453F40, h, v, 0x53, p.esi, p.edi, &[v]));
        }
        let pre = || {
            if t == 5 && rd(a(ev, 0xC)) != rd(a(h, 0x44)) {
                wd(a(h, 0x160), rd(a(ev, 0xC)));
                wd(a(h, 0x164), rd(a(ev, 0x24)));
                wb(a(h, 0x168), rb(a(ev, 0x28)));
            }
        };
        let watch = [a(h, 0x160), a(h, 0x164), a(h, 0x168)];
        r.eax = seq_port(OE_K13, &p, args, 2, &watch, &pre, plan, &|| {}, Some(0)).unwrap_or(0);
    }
}
regs_thunk!(oe_k13_thunk, oe_k13_entry);

/// Kind 0x20 (0x462CD0): type 0x4F -> ai_event_cancel(0x50, handle, handle) and dword +0xAF0 = 0,
/// EAX = 0; type 0x53 -> byte +0xE80 |= 2, EAX = 0; other types EAX = 3.
extern "C" fn oe_k20_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (h, ev) = (*args, *args.add(1));
        let t = rd(a(ev, 8));
        let mut plan = vec![];
        let ret = if t == 0x4F || t == 0x53 { 0 } else { 3 };
        if t == 0x4F {
            let hd = rd(a(h, 0x44));
            plan.push(vec![T_CANCEL, 0x50, hd, hd]);
        }
        let pre = || {
            if t == 0x53 {
                wb(a(h, 0xE80), rb(a(h, 0xE80)) | 2);
            }
        };
        let post = || {
            if t == 0x4F {
                wd(a(h, 0xAF0), 0);
            }
        };
        let watch = [a(h, 0xAF0), a(h, 0xE80)];
        r.eax = seq_port(OE_K20, &p, args, 2, &watch, &pre, plan, &post, Some(ret)).unwrap_or(ret);
    }
}
regs_thunk!(oe_k20_thunk, oe_k20_entry);

/// Kind 0x10 (0x432360): type 0x59 -> 0x431820 (EAX = human); EAX = 0.
extern "C" fn oe_k10_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (h, ev) = (*args, *args.add(1));
        let plan = if rd(a(ev, 8)) == 0x59 { vec![gcall(G_431820, h, p.ecx, p.edx, p.esi, p.edi, &[])] } else { vec![] };
        r.eax = seq_port(OE_K10, &p, args, 2, &[], &|| {}, plan, &|| {}, Some(0)).unwrap_or(0);
    }
}
regs_thunk!(oe_k10_thunk, oe_k10_entry);

/// Kind 0x1D (0x460D00): only for a human with dword +0x4C == 1 and byte +0x3CC == 2: type 0x37 ->
/// byte +0x150 |= 8, then 0x43E2C0 (EAX = event, ECX = human, EDX = 0x37); type 0x53 -> 0x447CD0(human,
/// 0, 0) (EAX = event, ECX = human, EDX = 0x53); EAX = 0.
extern "C" fn oe_k1d_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let (h, ev) = (*args, *args.add(1));
        let mut plan = vec![];
        let mut or8 = false;
        if h != 0 && rd(a(h, 0x4C)) == 1 && rb(a(h, 0x3CC)) == 2 {
            match rd(a(ev, 8)) {
                0x37 => {
                    or8 = true;
                    plan.push(gcall(G_43E2C0, ev, h, 0x37, p.esi, p.edi, &[]));
                }
                0x53 => plan.push(gcall(G_447CD0, ev, h, 0x53, p.esi, p.edi, &[h, 0, 0])),
                _ => {}
            }
        }
        let pre = || {
            if or8 {
                wb(a(h, 0x150), rb(a(h, 0x150)) | 8);
            }
        };
        let watch = if h != 0 { vec![a(h, 0x150)] } else { vec![] };
        r.eax = seq_port(OE_K1D, &p, args, 2, &watch, &pre, plan, &|| {}, Some(0)).unwrap_or(0);
    }
}
regs_thunk!(oe_k1d_thunk, oe_k1d_entry);

// --- update k09 (0x417BC0) ----------------------------------------------------------------------------------

/// With a live entity (handle dword +0x158 matches, entity dword +0x4C == 3): 0x417450(human, human
/// + 0x150, entity) (EAX = handle, ECX = entity, EDX = human + 0x150, ESI = human). EAX is the call's,
/// else the handle.
extern "C" fn up_k09_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let h = *args;
        let hd = rd(a(h, 0x158));
        let mut plan = vec![];
        let mut ret = Some(hd);
        if hd != 0 {
            let ent = rd(0x0214_9C64 + (((hd & 0xFFF) << 4) as usize));
            if ent != 0 && rd(a(ent, 0x44)) == hd && rd(a(ent, 0x4C)) == 3 {
                plan.push(gcall(G_417450, hd, ent, a(h, 0x150) as u32, h, p.edi, &[h, a(h, 0x150) as u32, ent]));
                ret = None;
            }
        }
        if let Some(e) = seq_port(UP_K09, &p, args, 1, &[], &|| {}, plan, &|| {}, ret) {
            r.eax = e;
        }
    }
}
regs_thunk!(up_k09_thunk, up_k09_entry);

// --- 0x437890 ai_order_interrupt_query (ECX = human, cdecl (event*) -> EAX) ---------------------------------

/// A copy of the on_event column of the order table (slot 3) with a stub in every non-null entry.
static FAKE_EV_TABLE: [AtomicU32; 252] = [const { AtomicU32::new(0) }; 252];
/// Offset of the table displacement in the body (`mov eax, [edx*4 + 0xDD5574]`).
const IQ_DISP: usize = 0x63;
const ORDER_EVENT_SLOT: u32 = 0x00DD_5574;

extern "C" fn rec_ev(r: &mut Pushad, a: *const u32) {
    unsafe { tape_push(vec![T_EV, *a, *a.add(1)]) };
    r.eax = 1;
}
regs_thunk!(ev_stub, rec_ev);

fn fill_fake_ev_table() {
    for (i, e) in FAKE_EV_TABLE.iter().enumerate() {
        let real = unsafe { rd(ORDER_TABLE + i * 4) };
        e.store(if real != 0 { addr(ev_stub) as u32 } else { 0 }, Ordering::Relaxed);
    }
}

/// The handler the original would call, if any: ECX = human (null: nothing), the player-controlled
/// shortcut (`[human + 0xB0]` has a record whose byte 0 is not the byte at `[[0x2528C14] + 0x408] +
/// 0x1B0`, or whose dword +0x67C has bit 0), the order kind `byte +0x149` (0xFF: nothing), byte +0x80
/// & 0xB, dword +0xC8 & 0x400000 (must be set) and `[kind * 28 + 0xDD5574]`.
unsafe fn iq_handler(h: u32) -> Option<usize> {
    unsafe {
        if h == 0 {
            return None;
        }
        let rec = rd(a(h, 0xB0)) as usize;
        if rec != 0 {
            let g = rd(0x0252_8C14) as usize;
            if g != 0 {
                let o = rd(g + 0x408) as usize;
                if o != 0 && (rb(rec) != rb(o + 0x1B0) || rb(rec + 0x67C) & 1 != 0) {
                    return None;
                }
            }
        }
        let k = rb(a(h, 0x149));
        if k == 0xFF || rb(a(h, 0x80)) & 0x0B != 0 || rd(a(h, 0xC8)) & 0x0040_0000 == 0 {
            return None;
        }
        let idx = (k as i8 as i32 as u32).wrapping_mul(7);
        let f = rd(ORDER_EVENT_SLOT.wrapping_add(idx.wrapping_mul(4)) as usize) as usize;
        (f != 0).then_some(f)
    }
}

extern "C" fn iq_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let h = p.ecx;
        let ev = *args;
        let f = iq_handler(h);
        let mut call_it = f.is_some();
        if let Some((n, orig)) = shadowing(IQ) {
            fill_fake_ev_table();
            tape_take();
            let e = call_regs(orig, p.eax, p.ecx, p.edx, p.esi, p.edi, &[ev]);
            let t = tape_take();
            let plan = if f.is_some() { vec![vec![T_EV, h, ev]] } else { vec![] };
            if t != plan || e != 1 {
                mismatch(IQ, n, format!("human {h:#x}: original {t:x?} -> {e:#x}, Rust {plan:x?}"));
                call_it = !t.is_empty();
            }
        }
        r.eax = 1;
        if call_it {
            // ECX = human, EDX = event, ESI = the human's [+0xB0] record; EAX = the handler.
            let f = f.unwrap_or_else(|| rd(ORDER_EVENT_SLOT.wrapping_add((rb(a(h, 0x149)) as i8 as i32 as u32).wrapping_mul(28)) as usize) as usize);
            r.eax = call_regs(f, f as u32, h, ev, rd(a(h, 0xB0)), p.edi, &[h, ev]);
        }
    }
}
regs_thunk!(iq_thunk, iq_entry);

// --- network readers (cdecl (human, ?, buf, u16 *pos); results unused by the caller) ----------------------------
//
// Each handler reads from `buf + *pos`, advances `*pos` and fills fields of the human's order
// payload (+0x150 ..); with no buffer it calls the failure reporter 0x447CD0(human, 0, 0) instead.
// Shadow: the copy (the reporter call recorded) and Rust both run on the real buffer with the
// position in a scratch cell, the human's payload bytes, the final positions and the recorded
// calls are compared, and the original's results win on a mismatch.

const G_456070: u32 = 11;
/// In a recorded / planned `0x456070` call: the position pointer (a scratch cell during the shadow).
const POS_MARK: u32 = 0xFFFF_FFFC;

/// `idiv` remainder (faults on a zero divisor exactly as the original does).
unsafe fn idiv_rem(num: i32, div: i32) -> i32 {
    let r: i32;
    unsafe {
        std::arch::asm!("cdq", "idiv {d:e}", d = in(reg) div, inout("eax") num => _, out("edx") r);
    }
    r
}

/// The entity handle for a 16-bit network id: the value hash of the id picks a bucket of the table at
/// `[0x212E258]` (bucket array +0x5C, bucket count +0x60); the chain (+4 next, key at +0xC) gives a
/// node whose first dword points at a record whose dword +4 is the handle; 0 when absent.
unsafe fn handle_of_net_id(id: u16) -> u32 {
    unsafe {
        let s = id as i32;
        let mut a = (!(s.wrapping_shl(15))).wrapping_add(s);
        let mut c = (a >> 10) ^ a;
        c = c.wrapping_mul(9);
        c ^= c >> 6;
        c = c.wrapping_add(!(c.wrapping_shl(11)));
        a = (c >> 16) ^ c;
        let t = rd(0x0212_E258) as usize;
        let rem = idiv_rem(a, rd(t + 0x60) as i32);
        let mut n = rd((rd(t + 0x5C) as usize).wrapping_add((rem as u32 as usize).wrapping_mul(4))) as usize;
        while n != 0 {
            if rd(n + 0xC) == id as u32 {
                let rec = rd(n) as usize;
                return if rec == 0 { 0 } else { rd(rec + 4) };
            }
            n = rd(n + 4) as usize;
        }
        0
    }
}

/// u16 position cell helpers.
unsafe fn pos_get(pos: usize) -> u16 {
    unsafe { std::ptr::read_unaligned(pos as *const u16) }
}
unsafe fn pos_add(pos: usize, n: u16) {
    unsafe { std::ptr::write_unaligned(pos as *mut u16, pos_get(pos).wrapping_add(n)) }
}
unsafe fn rd_b(buf: usize, pos: usize) -> u8 {
    unsafe {
        let v = rb(buf.wrapping_add(pos_get(pos) as usize));
        pos_add(pos, 1);
        v
    }
}
unsafe fn rd_w(buf: usize, pos: usize) -> u16 {
    unsafe {
        let v = std::ptr::read_unaligned(buf.wrapping_add(pos_get(pos) as usize) as *const u16);
        pos_add(pos, 2);
        v
    }
}
unsafe fn rd_d(buf: usize, pos: usize) -> u32 {
    unsafe {
        let v = rdu(buf.wrapping_add(pos_get(pos) as usize));
        pos_add(pos, 4);
        v
    }
}

/// The reporter call as the originals set it up: `(human, 0, 0)` in the given registers.
fn fail_call(eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, h: u32) -> Vec<Vec<u32>> {
    vec![gcall(G_447CD0, eax, ecx, edx, esi, edi, &[h, 0, 0])]
}

type NrBody = unsafe fn(p: &Pushad, h: usize, a1: u32, buf: usize, pos: usize) -> Vec<Vec<u32>>;

fn nr(id: usize, r: &mut Pushad, args: *const u32, body: NrBody) {
    let p = *r;
    let (h, a1, buf, pos) = unsafe { (*args as usize, *args.add(1), *args.add(2) as usize, *args.add(3) as usize) };
    unsafe {
        let fix = |plan: Vec<Vec<u32>>| -> Vec<Vec<u32>> {
            plan.into_iter().map(|e| e.into_iter().map(|v| if v == POS_MARK { pos as u32 } else { v }).collect()).collect()
        };
        let Some((n, orig)) = shadowing(id) else {
            let plan = body(&p, h, a1, buf, pos);
            if let Some(e) = replay(&fix(plan), p.esi, p.edi) {
                r.eax = e;
            }
            return;
        };
        let pos0 = pos_get(pos);
        let before = snap(h);
        let mut cell = pos0;
        let cell_a = &mut cell as *mut u16 as usize;
        tape_take();
        let e = call_regs(orig, p.eax, p.ecx, p.edx, p.esi, p.edi, &[h as u32, a1, buf as u32, cell_a as u32]);
        let rec: Vec<Vec<u32>> = tape_take().into_iter().map(|t| t.into_iter().map(|v| if v == cell_a as u32 { POS_MARK } else { v }).collect()).collect();
        let (end_theirs, h_theirs) = (cell, snap(h));
        unsnap(h, &before);
        cell = pos0;
        let plan = body(&p, h, a1, buf, cell_a);
        let (end_ours, h_ours) = (cell, snap(h));
        let _ = e;
        let (tape, end) = if rec != plan || end_theirs != end_ours || h_theirs != h_ours {
            mismatch(id, n, format!("human {h:#x} pos {pos0}: original end {end_theirs} {rec:x?}, Rust end {end_ours} {plan:x?}"));
            unsnap(h, &h_theirs);
            (rec, end_theirs)
        } else {
            (plan, end_ours)
        };
        std::ptr::write_unaligned(pos as *mut u16, end);
        if let Some(e) = replay(&fix(tape), p.esi, p.edi) {
            r.eax = e;
        }
    }
}

macro_rules! nr_port {
    ($entry:ident, $thunk:ident, $id:expr, $body:ident) => {
        extern "C" fn $entry(r: &mut Pushad, args: *const u32) {
            nr($id, r, args, $body);
        }
        regs_thunk!($thunk, $entry);
    };
}

/// Kind 0x19 (0x45AB50): no buffer -> reporter (EAX = human, ECX = 0); else dwords +0x150, +0x154.
unsafe fn nr_body_k19(p: &Pushad, h: usize, _a1: u32, buf: usize, pos: usize) -> Vec<Vec<u32>> {
    unsafe {
        if buf == 0 {
            return fail_call(h as u32, 0, p.edx, p.esi, p.edi, h as u32);
        }
        wdu(h + 0x150, rd_d(buf, pos));
        wdu(h + 0x154, rd_d(buf, pos));
        vec![]
    }
}

/// Kind 0x01 (0x402640): no buffer -> reporter (EAX = human, EDX = 0); else three f32 through the x87.
unsafe fn nr_body_k01(p: &Pushad, h: usize, _a1: u32, buf: usize, pos: usize) -> Vec<Vec<u32>> {
    unsafe {
        if buf == 0 {
            return fail_call(h as u32, p.ecx, 0, p.esi, p.edi, h as u32);
        }
        for o in [0x150, 0x154, 0x158] {
            wdu(h + o, x87_rt(rd_d(buf, pos)));
        }
        vec![]
    }
}

/// Kind 0x05 (0x40D3A0): dword +0xAF0 == 0 -> reporter (ECX = human, ESI unchanged); no buffer ->
/// reporter (ESI = 0); else bytes +0x150 .. +0x155.
unsafe fn nr_body_k05(p: &Pushad, h: usize, _a1: u32, buf: usize, pos: usize) -> Vec<Vec<u32>> {
    unsafe {
        if rd(h + 0xAF0) == 0 {
            return fail_call(p.eax, h as u32, p.edx, p.esi, p.edi, h as u32);
        }
        if buf == 0 {
            return fail_call(p.eax, h as u32, p.edx, 0, p.edi, h as u32);
        }
        for o in 0..6 {
            wb(h + 0x150 + o, rd_b(buf, pos));
        }
        vec![]
    }
}

/// Kind 0x14 (0x456020): no buffer -> tail call of 0x456070 (ECX = human, the four arguments); else byte
/// +0x150, and when dword +0xAF0 is non-zero dword +0x154.
unsafe fn nr_body_k14(p: &Pushad, h: usize, a1: u32, buf: usize, pos: usize) -> Vec<Vec<u32>> {
    unsafe {
        if buf == 0 {
            return vec![gcall(G_456070, p.eax, h as u32, p.edx, p.esi, p.edi, &[h as u32, a1, 0, POS_MARK])];
        }
        wb(h + 0x150, rd_b(buf, pos));
        if rd(h + 0xAF0) != 0 {
            wdu(h + 0x154, rd_d(buf, pos));
        }
        vec![]
    }
}

/// Kinds 0x06, 0x11, 0x1F (0x410BC0, 0x432870, 0x462320): no buffer -> reporter (EAX = human, ECX = 0);
/// else byte +0x150, then the handle of the 16-bit network id (0xFFFF: 0) into dword +0x154.
unsafe fn nr_body_k06(p: &Pushad, h: usize, _a1: u32, buf: usize, pos: usize) -> Vec<Vec<u32>> {
    unsafe {
        if buf == 0 {
            return fail_call(h as u32, 0, p.edx, p.esi, p.edi, h as u32);
        }
        wb(h + 0x150, rd_b(buf, pos));
        let id = rd_w(buf, pos);
        wdu(h + 0x154, if id == 0xFFFF { 0 } else { handle_of_net_id(id) });
        vec![]
    }
}

/// Kinds 0x15 and 0x21 (0x457F00, 0x463380): no buffer -> reporter (EAX = human); else the handle of the
/// 16-bit network id (0xFFFF: 0) into dword +0x150.
unsafe fn nr_body_k15(p: &Pushad, h: usize, _a1: u32, buf: usize, pos: usize) -> Vec<Vec<u32>> {
    unsafe {
        if buf == 0 {
            return fail_call(h as u32, p.ecx, p.edx, p.esi, p.edi, h as u32);
        }
        let id = rd_w(buf, pos);
        wdu(h + 0x150, if id == 0xFFFF { 0 } else { handle_of_net_id(id) });
        vec![]
    }
}

nr_port!(nr_k19_entry, nr_k19_thunk, NR_K19, nr_body_k19);
nr_port!(nr_k01_entry, nr_k01_thunk, NR_K01, nr_body_k01);
nr_port!(nr_k05_entry, nr_k05_thunk, NR_K05, nr_body_k05);
nr_port!(nr_k14_entry, nr_k14_thunk, NR_K14, nr_body_k14);
nr_port!(nr_k06_entry, nr_k06_thunk, NR_K06, nr_body_k06);
nr_port!(nr_k11_entry, nr_k11_thunk, NR_K11, nr_body_k06);
nr_port!(nr_k1f_entry, nr_k1f_thunk, NR_K1F, nr_body_k06);
nr_port!(nr_k15_entry, nr_k15_thunk, NR_K15, nr_body_k15);
nr_port!(nr_k21_entry, nr_k21_thunk, NR_K21, nr_body_k15);

// --- kind 0x00 payload init (0x4469A0) -------------------------------------------------------------------------

const G_MEMSET: u32 = 12;

/// Kind 0x00 payload init (cdecl (payload*), 0x4469A0): memset(payload + 0x28, 0, 0x34) (the CRT's,
/// 0xCA0BC0, with EAX = payload + 0x28 from the `lea`, ESI = payload, EDI = 0), then byte 0 = 0x14, byte 1 = 0, byte 2 = 0, zeros at +4
/// (written last), +0xC, +0x10, +0x14 and -1 at +8, +0x18, +0x1C, +0x20; EAX = -1.
extern "C" fn px_k00_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    unsafe {
        let pl = *args as usize;
        let plan = vec![gcall(G_MEMSET, (pl + 0x28) as u32, p.ecx, p.edx, pl as u32, 0, &[(pl + 0x28) as u32, 0, 0x34])];
        let post = || {
            ww(pl, 0);
            wb(pl + 2, 0);
            wb(pl, rb(pl) | 0x14);
            zeros(pl, &[0xC, 0x10, 0x14, 4]);
            for o in [0x18, 0x1C, 8, 0x20] {
                wd(pl + o, N1);
            }
        };
        let watch: Vec<usize> = (0..9).map(|i| pl + i * 4).collect();
        seq_port(PX_K00, &p, args, 1, &watch, &|| {}, plan, &post, Some(N1));
        r.eax = N1;
    }
}
regs_thunk!(px_k00_thunk, px_k00_entry);

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
            EN_K19 | EN_K23 => set_rel32(copy, 0xC, addr(gs_setact), 0xE8),
            EN_K20 => set_rel32(copy, 0x16, addr(gs_setact), 0xE8) && set_rel32(copy, 0x2F, addr(gs_setact), 0xE8),
            EN_K16 => set_rel32(copy, 0x2C, addr(gs_96fca0), 0xE8) && set_rel32(copy, 0x39, addr(gs_setact), 0xE8),
            EX_K0F | EX_K10 => {
                let cancel = PORTS[CANCEL].original.load(Ordering::Relaxed);
                cancel != 0 && set_rel32(copy, 0xD, addr(gaze_clear_stub), 0xE8) && set_rel32(copy, 0x1D, cancel, 0xE8)
            }
            EX_K1E => {
                let cancel = PORTS[CANCEL].original.load(Ordering::Relaxed);
                cancel != 0 && [0x10, 0x1F, 0x2E, 0x3D].iter().all(|&o| set_rel32(copy, o, cancel, 0xE8))
            }
            OE_K03 => set_rel32(copy, 0x15, addr(gs_4051f0), 0xE8) && set_rel32(copy, 0x22, addr(gs_404a90), 0xE8),
            OE_K0C => set_rel32(copy, 0x25, addr(gs_45cbd0), 0xE8),
            OE_K17 => set_rel32(copy, 0x15, addr(gs_459400), 0xE8),
            OE_K13 => set_rel32(copy, 0x1C, addr(gs_453f40), 0xE8),
            OE_K20 => {
                let cancel = PORTS[CANCEL].original.load(Ordering::Relaxed);
                cancel != 0 && set_rel32(copy, 0x35, cancel, 0xE8)
            }
            OE_K10 => set_rel32(copy, 0xE, addr(gs_431820), 0xE8),
            OE_K1D => set_rel32(copy, 0x2D, addr(gs_447cd0), 0xE8) && set_rel32(copy, 0x3F, addr(gs_43e2c0), 0xE8),
            UP_K09 => set_rel32(copy, 0x38, addr(gs_417450), 0xE8),
            IQ => {
                let at = copy + IQ_DISP;
                let ok = std::ptr::read_unaligned(at as *const u32) == ORDER_EVENT_SLOT
                    && std::ptr::read_unaligned((at - 3) as *const [u8; 3]) == [0x8B, 0x04, 0x95];
                if ok {
                    std::ptr::write_unaligned(at as *mut u32, (FAKE_EV_TABLE.as_ptr() as u32).wrapping_add(0xC));
                }
                ok
            }
            NR_K19 | NR_K06 | NR_K11 | NR_K1F | NR_K15 | NR_K21 => set_rel32(copy, 0xF, addr(gs_447cd0), 0xE8),
            NR_K01 => set_rel32(copy, 0x72, addr(gs_447cd0), 0xE8),
            NR_K05 => set_rel32(copy, 0x9A, addr(gs_447cd0), 0xE8),
            NR_K14 => set_rel32(copy, 0xE, addr(gs_456070), 0xE9),
            PX_K00 => set_rel32(copy, 0xF, addr(gs_memset), 0xE8),
            K17_EXIT | K1C_EXIT | K21_EXIT => {
                let cancel = PORTS[CANCEL].original.load(Ordering::Relaxed);
                cancel != 0 && set_rel32(copy, 0xF, cancel, 0xE8)
            }
            _ => true,
        }
    }
}


/// More addresses with the same body as a port above (id, address): installed with the same thunk.
const COPIES: [(usize, usize); 32] = [
    (RET_NOP, 0x0043_34D0), (RET_NOP, 0x0045_9390), (RET_NOP, 0x0045_AC70), (RET_NOP, 0x0045_CDB0), (RET_NOP, 0x0046_2BB0),
    (RET_NOP, 0x0041_79A0), (RET_NOP, 0x0043_32E0), (RET_NOP, 0x0045_92A0), (RET_NOP, 0x0045_8570),
    (RET_NOP, 0x0040_5C60), (RET_NOP, 0x0041_7990), (RET_NOP, 0x0043_32D0), (RET_NOP, 0x0045_9290), (RET_NOP, 0x0045_8560),
    (RET_NOP, 0x0040_5C50), (RET_NOP, 0x0041_5D90), (RET_NOP, 0x0043_32C0), (RET_NOP, 0x0045_9280), (RET_NOP, 0x0045_8550),
    (RET_ZERO, 0x0041_7C10), (RET_ZERO, 0x0045_93E0), (RET_ZERO, 0x0045_C130), (RET_ZERO, 0x0045_CEE0), (RET_ZERO, 0x0046_1990), (RET_ZERO, 0x0045_9140),
    (PX_K15, 0x0046_32F0),
    (NW_K15, 0x0045_CA40), (NW_K15, 0x0046_3300),
    (NW_K06, 0x0041_25C0), (NW_K06, 0x0043_27D0), (NW_K06, 0x0045_9FF0), (NW_K06, 0x0046_2280),
];

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
        (RET_NOP, &NOP_BODY, &NOP_BODY_CALLS, nop_thunk),
        (RET_ZERO, &ZERO_BODY, &ZERO_BODY_CALLS, zero_thunk),
        (RET_TWO, &TWO_BODY, &TWO_BODY_CALLS, two_thunk),
        (PX_K20, &PI_K20_BODY, &PI_K20_BODY_CALLS, px_k20_thunk),
        (PX_K15, &PI_K15_BODY, &PI_K15_BODY_CALLS, px_k15_thunk),
        (PX_K17, &PI_K17_BODY, &PI_K17_BODY_CALLS, px_k17_thunk),
        (PX_K1C, &PI_K1C_BODY, &PI_K1C_BODY_CALLS, px_k1c_thunk),
        (PX_K0F, &PI_K0F_BODY, &PI_K0F_BODY_CALLS, px_k0f_thunk),
        (PX_K1D, &PI_K1D_BODY, &PI_K1D_BODY_CALLS, px_k1d_thunk),
        (PX_K09, &PI_K09_BODY, &PI_K09_BODY_CALLS, px_k09_thunk),
        (PX_K0E, &PI_K0E_BODY, &PI_K0E_BODY_CALLS, px_k0e_thunk),
        (PX_K10, &PI_K10_BODY, &PI_K10_BODY_CALLS, px_k10_thunk),
        (PX_K0C, &PI_K0C_BODY, &PI_K0C_BODY_CALLS, px_k0c_thunk),
        (PX_K1B, &PI_K1B_BODY, &PI_K1B_BODY_CALLS, px_k1b_thunk),
        (PX_K0B, &PI_K0B_BODY, &PI_K0B_BODY_CALLS, px_k0b_thunk),
        (PX_K22, &PI_K22_BODY, &PI_K22_BODY_CALLS, px_k22_thunk),
        (PX_K0A, &PI_K0A_BODY, &PI_K0A_BODY_CALLS, px_k0a_thunk),
        (PX_K0D, &PI_K0D_BODY, &PI_K0D_BODY_CALLS, px_k0d_thunk),
        (PX_K13, &PI_K13_BODY, &PI_K13_BODY_CALLS, px_k13_thunk),
        (PX_K18, &PI_K18_BODY, &PI_K18_BODY_CALLS, px_k18_thunk),
        (PX_K04, &PI_K04_BODY, &PI_K04_BODY_CALLS, px_k04_thunk),
        (PX_K1A, &PI_K1A_BODY, &PI_K1A_BODY_CALLS, px_k1a_thunk),
        (PX_K02, &PI_K02_BODY, &PI_K02_BODY_CALLS, px_k02_thunk),
        (NW_K20, &NW_K20_BODY, &NW_K20_BODY_CALLS, nw_k20_thunk),
        (NW_K14, &NW_K14_BODY, &NW_K14_BODY_CALLS, nw_k14_thunk),
        (NW_K08, &NW_K08_BODY, &NW_K08_BODY_CALLS, nw_k08_thunk),
        (NW_K19, &NW_K19_BODY, &NW_K19_BODY_CALLS, nw_k19_thunk),
        (NW_K01, &NW_K01_BODY, &NW_K01_BODY_CALLS, nw_k01_thunk),
        (NW_K15, &NW_K15_BODY, &NW_K15_BODY_CALLS, nw_k15_thunk),
        (NW_K05, &NW_K05_BODY, &NW_K05_BODY_CALLS, nw_k05_thunk),
        (NW_K06, &NW_K06_BODY, &NW_K06_BODY_CALLS, nw_k06_thunk),
        (NW_K1C, &NW_K1C_BODY, &NW_K1C_BODY_CALLS, nw_k1c_thunk),
        (NW_K0C, &NW_K0C_BODY, &NW_K0C_BODY_CALLS, nw_k0c_thunk),
        (NW_K17, &NW_K17_BODY, &NW_K17_BODY_CALLS, nw_k17_thunk),
        (EN_K19, &EN_K19_BODY, &EN_K19_BODY_CALLS, en_k19_thunk),
        (EN_K23, &EN_K23_BODY, &EN_K23_BODY_CALLS, en_k23_thunk),
        (EN_K20, &EN_K20_BODY, &EN_K20_BODY_CALLS, en_k20_thunk),
        (EN_K16, &EN_K16_BODY, &EN_K16_BODY_CALLS, en_k16_thunk),
        (EX_K15, &EX_K15_BODY, &EX_K15_BODY_CALLS, ex_k15_thunk),
        (EX_K0F, &EX_K0F_BODY, &EX_K0F_BODY_CALLS, ex_k0f_thunk),
        (EX_K10, &EX_K10_BODY, &EX_K10_BODY_CALLS, ex_k10_thunk),
        (EX_K1E, &EX_K1E_BODY, &EX_K1E_BODY_CALLS, ex_k1e_thunk),
        (OE_K03, &OE_K03_BODY, &OE_K03_BODY_CALLS, oe_k03_thunk),
        (OE_K0C, &OE_K0C_BODY, &OE_K0C_BODY_CALLS, oe_k0c_thunk),
        (OE_K17, &OE_K17_BODY, &OE_K17_BODY_CALLS, oe_k17_thunk),
        (OE_K13, &OE_K13_BODY, &OE_K13_BODY_CALLS, oe_k13_thunk),
        (OE_K20, &OE_K20_BODY, &OE_K20_BODY_CALLS, oe_k20_thunk),
        (OE_K10, &OE_K10_BODY, &OE_K10_BODY_CALLS, oe_k10_thunk),
        (OE_K1D, &OE_K1D_BODY, &OE_K1D_BODY_CALLS, oe_k1d_thunk),
        (UP_K09, &UP_K09_BODY, &UP_K09_BODY_CALLS, up_k09_thunk),
        (IQ, &IQ_BODY, &IQ_BODY_CALLS, iq_thunk),
        (NR_K19, &NR_K19_BODY, &NR_K19_BODY_CALLS, nr_k19_thunk),
        (NR_K01, &NR_K01_BODY, &NR_K01_BODY_CALLS, nr_k01_thunk),
        (NR_K05, &NR_K05_BODY, &NR_K05_BODY_CALLS, nr_k05_thunk),
        (NR_K14, &NR_K14_BODY, &NR_K14_BODY_CALLS, nr_k14_thunk),
        (NR_K06, &NR_K06_BODY, &NR_K06_BODY_CALLS, nr_k06_thunk),
        (NR_K11, &NR_K11_BODY, &NR_K11_BODY_CALLS, nr_k11_thunk),
        (NR_K1F, &NR_K1F_BODY, &NR_K1F_BODY_CALLS, nr_k1f_thunk),
        (NR_K15, &NR_K15_BODY, &NR_K15_BODY_CALLS, nr_k15_thunk),
        (NR_K21, &NR_K21_BODY, &NR_K21_BODY_CALLS, nr_k21_thunk),
        (PX_K00, &PI_K00_BODY, &PI_K00_BODY_CALLS, px_k00_thunk),
    ];
    let mut ok = 0;
    for &(id, body, call_offsets, thunk) in &list {
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
    let mut total = COUNT;
    for (id, at) in COPIES {
        total += 1;
        let (_, body, _, thunk) = list.iter().find(|e| e.0 == id).expect("copy of a port in the list");
        if replace_pinned(PORTS[id].name, at, body, *thunk as usize) {
            ok += 1;
        }
    }
    log(&format!("port_ai: replaced {ok}/{total} with Rust"));
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

const NOP_BODY: [u8; 5] = [0xC3, 0xCC, 0xCC, 0xCC, 0xCC];
const NOP_BODY_CALLS: [usize; 0] = [];
const ZERO_BODY: [u8; 5] = [0x33, 0xC0, 0xC3, 0xCC, 0xCC];
const ZERO_BODY_CALLS: [usize; 0] = [];
const TWO_BODY: [u8; 6] = [0xB8, 0x02, 0x00, 0x00, 0x00, 0xC3];
const TWO_BODY_CALLS: [usize; 0] = [];
const PI_K20_BODY: [u8; 8] = [
    0x8B, 0x44, 0x24, 0x04, 0xC6, 0x00, 0x00, 0xC3,
];
const PI_K20_BODY_CALLS: [usize; 0] = [];
const PI_K15_BODY: [u8; 11] = [
    0x8B, 0x44, 0x24, 0x04, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const PI_K15_BODY_CALLS: [usize; 0] = [];
const PI_K17_BODY: [u8; 18] = [
    0x8B, 0x44, 0x24, 0x04, 0xC7, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00,
    0x00, 0xC3,
];
const PI_K17_BODY_CALLS: [usize; 0] = [];
const PI_K1C_BODY: [u8; 22] = [
    0x8B, 0x44, 0x24, 0x04, 0xC6, 0x00, 0x00, 0xC7, 0x40, 0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0xC7, 0x40,
    0x04, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const PI_K1C_BODY_CALLS: [usize; 0] = [];
const PI_K0F_BODY: [u8; 26] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0xC6, 0x00, 0x00, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00,
    0xD9, 0x50, 0x08, 0xD9, 0x50, 0x0C, 0xD9, 0x58, 0x10, 0xC3,
];
const PI_K0F_BODY_CALLS: [usize; 0] = [];
const PI_K1D_BODY: [u8; 26] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0x33, 0xC9, 0x88, 0x08, 0x89, 0x48, 0x08, 0x89, 0x48, 0x04,
    0xD9, 0x50, 0x10, 0xD9, 0x50, 0x14, 0xD9, 0x58, 0x18, 0xC3,
];
const PI_K1D_BODY_CALLS: [usize; 0] = [];
const PI_K09_BODY: [u8; 27] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0x33, 0xC9, 0xD9, 0x50, 0x04, 0xD9, 0x18, 0x89, 0x48, 0x08,
    0x89, 0x48, 0x0C, 0xC7, 0x40, 0x10, 0xFF, 0xFF, 0xFF, 0xFF, 0xC3,
];
const PI_K09_BODY_CALLS: [usize; 0] = [];
const PI_K0E_BODY: [u8; 29] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0xC7, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x40, 0x04, 0xFF,
    0xFF, 0xFF, 0xFF, 0xD9, 0x50, 0x08, 0xD9, 0x50, 0x0C, 0xD9, 0x58, 0x10, 0xC3,
];
const PI_K0E_BODY_CALLS: [usize; 0] = [];
const PI_K10_BODY: [u8; 29] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xD9, 0x50, 0x08, 0xD9, 0x50, 0x0C, 0xD9, 0x58, 0x10, 0xC3,
];
const PI_K10_BODY_CALLS: [usize; 0] = [];
const PI_K0C_BODY: [u8; 31] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0xC6, 0x00, 0x00, 0xD9, 0x50, 0x04, 0xD9, 0x50, 0x08, 0xD9,
    0x50, 0x0C, 0xD9, 0x50, 0x10, 0x66, 0xC7, 0x40, 0x18, 0x00, 0x00, 0xD9, 0x58, 0x14, 0xC3,
];
const PI_K0C_BODY_CALLS: [usize; 0] = [];
const PI_K1B_BODY: [u8; 38] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0xC7, 0x40, 0x1C, 0xFF, 0xFF, 0xFF, 0xFF, 0xD9, 0x50, 0x10,
    0xD9, 0x50, 0x14, 0xD9, 0x50, 0x18, 0xD9, 0x50, 0x04, 0xD9, 0x50, 0x08, 0xD9, 0x58, 0x0C, 0xC7,
    0x00, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const PI_K1B_BODY_CALLS: [usize; 0] = [];
const PI_K0B_BODY: [u8; 44] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0xC6, 0x00, 0x00, 0xD9, 0x50, 0x04, 0xD9, 0x50, 0x08, 0xD9,
    0x50, 0x0C, 0xD9, 0x50, 0x20, 0xD9, 0x50, 0x24, 0xD9, 0x50, 0x28, 0xD9, 0x50, 0x10, 0xD9, 0x50,
    0x14, 0xD9, 0x58, 0x18, 0xC7, 0x40, 0x1C, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const PI_K0B_BODY_CALLS: [usize; 0] = [];
const PI_K22_BODY: [u8; 44] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0x33, 0xD2, 0x88, 0x10, 0xD9, 0x58, 0x14, 0x83, 0xC9, 0xFF,
    0x89, 0x50, 0x04, 0x89, 0x50, 0x08, 0x88, 0x50, 0x18, 0x89, 0x48, 0x0C, 0x89, 0x48, 0x10, 0x89,
    0x48, 0x1C, 0x89, 0x50, 0x30, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0xC3,
];
const PI_K22_BODY_CALLS: [usize; 0] = [];
const PI_K0A_BODY: [u8; 46] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0x33, 0xC9, 0x66, 0x89, 0x08, 0x88, 0x48, 0x02, 0x89, 0x48,
    0x04, 0x89, 0x48, 0x08, 0xC7, 0x40, 0x24, 0xFF, 0xFF, 0xFF, 0xFF, 0xD9, 0x50, 0x0C, 0xD9, 0x50,
    0x10, 0xD9, 0x50, 0x14, 0xD9, 0x50, 0x18, 0xD9, 0x50, 0x1C, 0xD9, 0x58, 0x20, 0xC3,
];
const PI_K0A_BODY_CALLS: [usize; 0] = [];
const PI_K0D_BODY: [u8; 55] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0x33, 0xC9, 0x66, 0x89, 0x08, 0x88, 0x48, 0x02, 0xD9, 0x50,
    0x10, 0x89, 0x48, 0x28, 0x83, 0xCA, 0xFF, 0x89, 0x50, 0x20, 0x89, 0x50, 0x24, 0xD9, 0x50, 0x14,
    0xD9, 0x50, 0x18, 0xD9, 0x50, 0x1C, 0xD9, 0x50, 0x04, 0xD9, 0x50, 0x08, 0xD9, 0x58, 0x0C, 0x89,
    0x50, 0x2C, 0x66, 0x89, 0x48, 0x30, 0xC3,
];
const PI_K0D_BODY_CALLS: [usize; 0] = [];
const PI_K13_BODY: [u8; 58] = [
    0x8B, 0x44, 0x24, 0x04, 0x33, 0xC9, 0x89, 0x48, 0x04, 0x89, 0x48, 0x10, 0x88, 0x48, 0x18, 0x89,
    0x48, 0x08, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x81, 0xF9, 0x00, 0xD2, 0x49, 0x6B, 0xC7, 0x00,
    0xFF, 0xFF, 0xFF, 0xFF, 0xC7, 0x40, 0x14, 0xFF, 0xFF, 0xFF, 0xFF, 0x89, 0x48, 0x0C, 0x7E, 0x09,
    0x81, 0xC1, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x48, 0x0C, 0xC3,
];
const PI_K13_BODY_CALLS: [usize; 0] = [];
const PI_K18_BODY: [u8; 58] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0xC6, 0x00, 0x00, 0xC7,
    0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xD9, 0x50, 0x08, 0x81, 0xC1, 0xDC, 0x05, 0x00, 0x00, 0xD9,
    0x50, 0x0C, 0x81, 0xF9, 0x00, 0xD2, 0x49, 0x6B, 0xD9, 0x58, 0x10, 0x89, 0x48, 0x14, 0x7E, 0x09,
    0x81, 0xC1, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x48, 0x14, 0xC3,
];
const PI_K18_BODY_CALLS: [usize; 0] = [];
const PI_K04_BODY: [u8; 65] = [
    0x8B, 0x44, 0x24, 0x04, 0xD9, 0xEE, 0x66, 0xC7, 0x00, 0x00, 0x00, 0x80, 0x08, 0x01, 0xD9, 0x50,
    0x04, 0xD9, 0x50, 0x08, 0xD9, 0x50, 0x0C, 0xD9, 0x50, 0x14, 0xD9, 0x50, 0x18, 0xD9, 0x50, 0x1C,
    0xD9, 0x50, 0x20, 0xD9, 0x50, 0x24, 0xD9, 0x58, 0x28, 0xC7, 0x40, 0x10, 0xFF, 0xFF, 0xFF, 0xFF,
    0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0xC7, 0x40, 0x2C, 0x00, 0x00, 0x00, 0x00, 0xD9, 0x58, 0x30,
    0xC3,
];
const PI_K04_BODY_CALLS: [usize; 0] = [];
const PI_K1A_BODY: [u8; 87] = [
    0x8B, 0x44, 0x24, 0x04, 0x33, 0xC9, 0x88, 0x48, 0x0C, 0x89, 0x48, 0x04, 0x89, 0x48, 0x08, 0xC7,
    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xC7, 0x40, 0x1C, 0xFF, 0xFF, 0xFF, 0xFF, 0x8B, 0x0D, 0xD0, 0xA6,
    0x52, 0x02, 0x89, 0x48, 0x10, 0x8B, 0x15, 0xD4, 0xA6, 0x52, 0x02, 0x89, 0x50, 0x14, 0x8B, 0x0D,
    0xD8, 0xA6, 0x52, 0x02, 0x89, 0x48, 0x18, 0x8B, 0x15, 0xD0, 0xA6, 0x52, 0x02, 0x89, 0x50, 0x20,
    0x8B, 0x0D, 0xD4, 0xA6, 0x52, 0x02, 0x89, 0x48, 0x24, 0x8B, 0x15, 0xD8, 0xA6, 0x52, 0x02, 0x80,
    0x48, 0x0C, 0x08, 0x89, 0x50, 0x28, 0xC3,
];
const PI_K1A_BODY_CALLS: [usize; 0] = [];
const PI_K02_BODY: [u8; 63] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x33, 0xD2, 0x81, 0xF9, 0x00, 0xD2,
    0x49, 0x6B, 0x88, 0x10, 0x89, 0x50, 0x04, 0xC7, 0x40, 0x10, 0xFF, 0xFF, 0x00, 0x00, 0x89, 0x48,
    0x08, 0x7E, 0x09, 0x81, 0xC1, 0x00, 0x2E, 0xB6, 0x94, 0x89, 0x48, 0x08, 0xD9, 0x05, 0x98, 0x79,
    0xE8, 0x00, 0x88, 0x50, 0x0C, 0x88, 0x50, 0x0D, 0xD9, 0x58, 0x18, 0x89, 0x50, 0x14, 0xC3,
];
const PI_K02_BODY_CALLS: [usize; 0] = [];
const NW_K20_BODY: [u8; 31] = [
    0x8B, 0x44, 0x24, 0x0C, 0x0F, 0xB7, 0x08, 0x8B, 0x54, 0x24, 0x04, 0x8A, 0x92, 0x50, 0x01, 0x00,
    0x00, 0x56, 0x8B, 0x74, 0x24, 0x0C, 0x88, 0x14, 0x31, 0x66, 0x83, 0x00, 0x01, 0x5E, 0xC3,
];
const NW_K20_BODY_CALLS: [usize; 0] = [];
const NW_K14_BODY: [u8; 61] = [
    0x8B, 0x44, 0x24, 0x0C, 0x0F, 0xB7, 0x10, 0x8B, 0x4C, 0x24, 0x04, 0x53, 0x8A, 0x99, 0x50, 0x01,
    0x00, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x10, 0x88, 0x1C, 0x32, 0x66, 0x83, 0x00, 0x01, 0x83, 0xB9,
    0xF0, 0x0A, 0x00, 0x00, 0x00, 0x0F, 0xB7, 0x10, 0x74, 0x10, 0x8B, 0x89, 0x54, 0x01, 0x00, 0x00,
    0x0F, 0xB7, 0xD2, 0x89, 0x0C, 0x32, 0x66, 0x83, 0x00, 0x04, 0x5E, 0x5B, 0xC3,
];
const NW_K14_BODY_CALLS: [usize; 0] = [];
const NW_K08_BODY: [u8; 62] = [
    0x8B, 0x44, 0x24, 0x04, 0x8B, 0x88, 0x30, 0x03, 0x00, 0x00, 0x85, 0xC9, 0x74, 0x2F, 0x83, 0x79,
    0x08, 0xFF, 0x74, 0x29, 0x8B, 0x44, 0x24, 0x0C, 0x0F, 0xB7, 0x10, 0x53, 0x8A, 0x59, 0x34, 0x56,
    0x8B, 0x74, 0x24, 0x10, 0x88, 0x1C, 0x32, 0x66, 0x83, 0x00, 0x01, 0x0F, 0xB7, 0x10, 0x8A, 0x49,
    0x08, 0x0F, 0xB7, 0xD2, 0x88, 0x0C, 0x32, 0x66, 0x83, 0x00, 0x01, 0x5E, 0x5B, 0xC3,
];
const NW_K08_BODY_CALLS: [usize; 0] = [];
const NW_K19_BODY: [u8; 68] = [
    0x8B, 0x4C, 0x24, 0x04, 0xD9, 0x81, 0x50, 0x01, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x0C, 0x8B, 0x54,
    0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0x56, 0x0F, 0xB7, 0x30, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x89,
    0x3C, 0x16, 0x66, 0x83, 0x00, 0x04, 0x0F, 0xB7, 0x30, 0xD9, 0x81, 0x54, 0x01, 0x00, 0x00, 0xD9,
    0x5C, 0x24, 0x0C, 0x0F, 0xB7, 0xCE, 0x8B, 0x74, 0x24, 0x0C, 0x89, 0x34, 0x11, 0x66, 0x83, 0x00,
    0x04, 0x5F, 0x5E, 0xC3,
];
const NW_K19_BODY_CALLS: [usize; 0] = [];
const NW_K01_BODY: [u8; 95] = [
    0x8B, 0x4C, 0x24, 0x04, 0xD9, 0x81, 0x50, 0x01, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x0C, 0x8B, 0x54,
    0x24, 0x08, 0xD9, 0x5C, 0x24, 0x04, 0x56, 0x0F, 0xB7, 0x30, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x89,
    0x3C, 0x16, 0x66, 0x83, 0x00, 0x04, 0x0F, 0xB7, 0x30, 0xD9, 0x81, 0x54, 0x01, 0x00, 0x00, 0xD9,
    0x5C, 0x24, 0x0C, 0x8B, 0x7C, 0x24, 0x0C, 0x0F, 0xB7, 0xF6, 0x89, 0x3C, 0x16, 0x66, 0x83, 0x00,
    0x04, 0x0F, 0xB7, 0x30, 0xD9, 0x81, 0x58, 0x01, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x0C, 0x0F, 0xB7,
    0xCE, 0x8B, 0x74, 0x24, 0x0C, 0x89, 0x34, 0x11, 0x66, 0x83, 0x00, 0x04, 0x5F, 0x5E, 0xC3,
];
const NW_K01_BODY_CALLS: [usize; 0] = [];
const NW_K15_BODY: [u8; 127] = [
    0x8B, 0x54, 0x24, 0x04, 0x8B, 0x8A, 0x50, 0x01, 0x00, 0x00, 0x85, 0xC9, 0x75, 0x16, 0x8B, 0x44,
    0x24, 0x0C, 0x0F, 0xB7, 0x08, 0x8B, 0x54, 0x24, 0x08, 0x66, 0xC7, 0x04, 0x11, 0xFF, 0xFF, 0x66,
    0x83, 0x00, 0x02, 0xC3, 0x8B, 0xC1, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80,
    0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x05, 0x39, 0x48, 0x44, 0x74, 0x07, 0xB8, 0xFF, 0xFF,
    0x00, 0x00, 0xEB, 0x15, 0x8B, 0x80, 0xB0, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x75, 0x07, 0xB8, 0xFF,
    0xFF, 0x00, 0x00, 0xEB, 0x04, 0x0F, 0xB7, 0x40, 0x02, 0x66, 0x3D, 0xFF, 0xFF, 0x0F, 0xB7, 0xC8,
    0x75, 0x07, 0x0F, 0xB7, 0x8A, 0x50, 0x01, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x0C, 0x0F, 0xB7, 0x10,
    0x56, 0x8B, 0x74, 0x24, 0x0C, 0x66, 0x89, 0x0C, 0x32, 0x66, 0x83, 0x00, 0x02, 0x5E, 0xC3,
];
const NW_K15_BODY_CALLS: [usize; 0] = [];
const NW_K05_BODY: [u8; 133] = [
    0x8B, 0x44, 0x24, 0x0C, 0x0F, 0xB7, 0x10, 0x8B, 0x4C, 0x24, 0x04, 0x53, 0x0F, 0xB6, 0x99, 0x50,
    0x01, 0x00, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x10, 0x88, 0x1C, 0x32, 0x66, 0x83, 0x00, 0x01, 0x0F,
    0xB7, 0x10, 0x0F, 0xB6, 0x99, 0x51, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xD2, 0x88, 0x1C, 0x32, 0x66,
    0x83, 0x00, 0x01, 0x0F, 0xB7, 0x10, 0x0F, 0xB6, 0x99, 0x52, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xD2,
    0x88, 0x1C, 0x32, 0x66, 0x83, 0x00, 0x01, 0x0F, 0xB7, 0x10, 0x0F, 0xB6, 0x99, 0x53, 0x01, 0x00,
    0x00, 0x0F, 0xB7, 0xD2, 0x88, 0x1C, 0x32, 0x66, 0x83, 0x00, 0x01, 0x0F, 0xB7, 0x10, 0x0F, 0xB6,
    0x99, 0x54, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xD2, 0x88, 0x1C, 0x32, 0x66, 0x83, 0x00, 0x01, 0x0F,
    0xB7, 0x10, 0x8A, 0x89, 0x55, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xD2, 0x88, 0x0C, 0x32, 0x66, 0x83,
    0x00, 0x01, 0x5E, 0x5B, 0xC3,
];
const NW_K05_BODY_CALLS: [usize; 0] = [];
const NW_K06_BODY: [u8; 145] = [
    0x8B, 0x44, 0x24, 0x0C, 0x0F, 0xB7, 0x08, 0x53, 0x56, 0x8B, 0x74, 0x24, 0x0C, 0x8A, 0x96, 0x50,
    0x01, 0x00, 0x00, 0x57, 0x8B, 0x7C, 0x24, 0x14, 0x88, 0x14, 0x39, 0x66, 0x83, 0x00, 0x01, 0x8B,
    0x8E, 0x54, 0x01, 0x00, 0x00, 0x85, 0xC9, 0x0F, 0xB7, 0x18, 0x75, 0x11, 0x0F, 0xB7, 0xCB, 0x66,
    0xC7, 0x04, 0x39, 0xFF, 0xFF, 0x66, 0x83, 0x00, 0x02, 0x5F, 0x5E, 0x5B, 0xC3, 0x8B, 0xD1, 0x81,
    0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x92, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xD2,
    0x74, 0x05, 0x39, 0x4A, 0x44, 0x74, 0x07, 0xB9, 0xFF, 0xFF, 0x00, 0x00, 0xEB, 0x13, 0x8B, 0x92,
    0xB0, 0x00, 0x00, 0x00, 0x85, 0xD2, 0xB9, 0xFF, 0xFF, 0x00, 0x00, 0x74, 0x04, 0x0F, 0xB7, 0x4A,
    0x02, 0x66, 0x81, 0xF9, 0xFF, 0xFF, 0x0F, 0xB7, 0xD1, 0x75, 0x07, 0x0F, 0xB7, 0x96, 0x54, 0x01,
    0x00, 0x00, 0x0F, 0xB7, 0xCB, 0x66, 0x89, 0x14, 0x39, 0x66, 0x83, 0x00, 0x02, 0x5F, 0x5E, 0x5B,
    0xC3,
];
const NW_K06_BODY_CALLS: [usize; 0] = [];
const NW_K1C_BODY: [u8; 165] = [
    0x8B, 0x44, 0x24, 0x0C, 0x8B, 0x54, 0x24, 0x04, 0x0F, 0xB7, 0x08, 0x53, 0x0F, 0xB6, 0x9A, 0x50,
    0x01, 0x00, 0x00, 0x57, 0x8B, 0x7C, 0x24, 0x10, 0x88, 0x1C, 0x39, 0x66, 0x83, 0x00, 0x01, 0x0F,
    0xB7, 0x08, 0x0F, 0xB6, 0x9A, 0x58, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xC9, 0x88, 0x1C, 0x39, 0x66,
    0x83, 0x00, 0x01, 0x8B, 0x8A, 0x54, 0x01, 0x00, 0x00, 0x85, 0xC9, 0x0F, 0xB7, 0x18, 0x75, 0x10,
    0x0F, 0xB7, 0xD3, 0x66, 0xC7, 0x04, 0x3A, 0xFF, 0xFF, 0x66, 0x83, 0x00, 0x02, 0x5F, 0x5B, 0xC3,
    0x56, 0x8B, 0xF1, 0x81, 0xE6, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE6, 0x04, 0x8B, 0xB6, 0x64, 0x9C,
    0x14, 0x02, 0x85, 0xF6, 0x74, 0x05, 0x39, 0x4E, 0x44, 0x74, 0x07, 0xB9, 0xFF, 0xFF, 0x00, 0x00,
    0xEB, 0x13, 0x8B, 0xB6, 0xB0, 0x00, 0x00, 0x00, 0x85, 0xF6, 0xB9, 0xFF, 0xFF, 0x00, 0x00, 0x74,
    0x04, 0x0F, 0xB7, 0x4E, 0x02, 0x66, 0x81, 0xF9, 0xFF, 0xFF, 0x0F, 0xB7, 0xF1, 0x75, 0x07, 0x0F,
    0xB7, 0xB2, 0x54, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xCB, 0x66, 0x89, 0x34, 0x39, 0x66, 0x83, 0x00,
    0x02, 0x5E, 0x5F, 0x5B, 0xC3,
];
const NW_K1C_BODY_CALLS: [usize; 0] = [];
const NW_K0C_BODY: [u8; 200] = [
    0x8B, 0x44, 0x24, 0x0C, 0x0F, 0xB7, 0x10, 0x8B, 0x4C, 0x24, 0x04, 0x53, 0x8A, 0x99, 0x50, 0x01,
    0x00, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x10, 0x88, 0x1C, 0x32, 0x66, 0x83, 0x00, 0x01, 0x0F, 0xB7,
    0x10, 0xD9, 0x81, 0x54, 0x01, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x0C, 0x0F, 0xB7, 0xD2, 0x57, 0x8B,
    0x7C, 0x24, 0x10, 0x89, 0x3C, 0x32, 0x66, 0x83, 0x00, 0x04, 0x0F, 0xB7, 0x10, 0xD9, 0x81, 0x58,
    0x01, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x10, 0x8B, 0x7C, 0x24, 0x10, 0x0F, 0xB7, 0xD2, 0x89, 0x3C,
    0x32, 0x66, 0x83, 0x00, 0x04, 0x0F, 0xB7, 0x10, 0xD9, 0x81, 0x5C, 0x01, 0x00, 0x00, 0xD9, 0x5C,
    0x24, 0x10, 0x8B, 0x7C, 0x24, 0x10, 0x0F, 0xB7, 0xD2, 0x89, 0x3C, 0x32, 0x66, 0x83, 0x00, 0x04,
    0x0F, 0xB7, 0x10, 0xD9, 0x81, 0x60, 0x01, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x10, 0x8B, 0x7C, 0x24,
    0x10, 0x0F, 0xB7, 0xD2, 0x89, 0x3C, 0x32, 0x66, 0x83, 0x00, 0x04, 0x0F, 0xB7, 0x10, 0xD9, 0x81,
    0x64, 0x01, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x10, 0x8B, 0x7C, 0x24, 0x10, 0x0F, 0xB7, 0xD2, 0x89,
    0x3C, 0x32, 0x66, 0x83, 0x00, 0x04, 0x0F, 0xB7, 0x10, 0x66, 0x8B, 0xB9, 0x68, 0x01, 0x00, 0x00,
    0x0F, 0xB7, 0xD2, 0x66, 0x89, 0x3C, 0x32, 0x66, 0x83, 0x00, 0x02, 0x5F, 0x5E, 0x66, 0xC7, 0x81,
    0x68, 0x01, 0x00, 0x00, 0x00, 0x00, 0x5B, 0xC3,
];
const NW_K0C_BODY_CALLS: [usize; 0] = [];
const NW_K17_BODY: [u8; 188] = [
    0x8B, 0x44, 0x24, 0x0C, 0x0F, 0xB7, 0x08, 0x8B, 0x54, 0x24, 0x08, 0x53, 0x56, 0x8B, 0x74, 0x24,
    0x0C, 0x57, 0x8B, 0xBE, 0x50, 0x01, 0x00, 0x00, 0x89, 0x3C, 0x11, 0x66, 0x83, 0x00, 0x04, 0x8B,
    0x8E, 0x54, 0x01, 0x00, 0x00, 0x85, 0xC9, 0x0F, 0xB7, 0x18, 0x75, 0x0B, 0x0F, 0xB7, 0xCB, 0x66,
    0xC7, 0x04, 0x11, 0xFF, 0xFF, 0xEB, 0x4C, 0x8B, 0xF9, 0x81, 0xE7, 0xFF, 0x0F, 0x00, 0x00, 0xC1,
    0xE7, 0x04, 0x8B, 0xBF, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xFF, 0x74, 0x05, 0x39, 0x4F, 0x44, 0x74,
    0x07, 0xB9, 0xFF, 0xFF, 0x00, 0x00, 0xEB, 0x13, 0x8B, 0xBF, 0xB0, 0x00, 0x00, 0x00, 0x85, 0xFF,
    0xB9, 0xFF, 0xFF, 0x00, 0x00, 0x74, 0x04, 0x0F, 0xB7, 0x4F, 0x02, 0x66, 0x81, 0xF9, 0xFF, 0xFF,
    0x0F, 0xB7, 0xF9, 0x75, 0x07, 0x0F, 0xB7, 0xBE, 0x54, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xCB, 0x66,
    0x89, 0x3C, 0x11, 0x66, 0x83, 0x00, 0x02, 0x83, 0xBE, 0x54, 0x01, 0x00, 0x00, 0x00, 0x0F, 0xB7,
    0x08, 0x74, 0x25, 0x0F, 0xB6, 0x9E, 0x58, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xC9, 0x88, 0x1C, 0x11,
    0x66, 0x83, 0x00, 0x01, 0x0F, 0xB7, 0x08, 0x0F, 0xB6, 0x9E, 0x5C, 0x01, 0x00, 0x00, 0x0F, 0xB7,
    0xC9, 0x88, 0x1C, 0x11, 0x66, 0x83, 0x00, 0x01, 0x5F, 0x5E, 0x5B, 0xC3,
];
const NW_K17_BODY_CALLS: [usize; 0] = [];

const EN_K19_BODY: [u8; 21] = [
    0x8B, 0x44, 0x24, 0x04, 0x68, 0xAB, 0x00, 0x00, 0x00, 0x6A, 0x19, 0x50, 0xE8, 0x4F, 0xC9, 0xFD,
    0xFF, 0x83, 0xC4, 0x0C, 0xC3,
];
const EN_K19_BODY_CALLS: [usize; 1] = [0xc];
const EN_K23_BODY: [u8; 21] = [
    0x8B, 0x44, 0x24, 0x04, 0x68, 0xA3, 0x00, 0x00, 0x00, 0x6A, 0x23, 0x50, 0xE8, 0x0F, 0xEF, 0xFD,
    0xFF, 0x83, 0xC4, 0x0C, 0xC3,
];
const EN_K23_BODY_CALLS: [usize; 1] = [0xc];
const EN_K20_BODY: [u8; 57] = [
    0x56, 0x8B, 0x74, 0x24, 0x08, 0xF6, 0x86, 0x50, 0x01, 0x00, 0x00, 0x01, 0x74, 0x19, 0x68, 0xBE,
    0x00, 0x00, 0x00, 0x6A, 0x20, 0x56, 0xE8, 0x25, 0x4A, 0xFD, 0xFF, 0x83, 0xC4, 0x0C, 0x80, 0x8E,
    0x80, 0x0E, 0x00, 0x00, 0x02, 0x5E, 0xC3, 0x68, 0xBF, 0x00, 0x00, 0x00, 0x6A, 0x20, 0x56, 0xE8,
    0x0C, 0x4A, 0xFD, 0xFF, 0x83, 0xC4, 0x0C, 0x5E, 0xC3,
];
const EN_K20_BODY_CALLS: [usize; 2] = [0x16, 0x2f];
const EN_K16_BODY: [u8; 67] = [
    0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0x56, 0x8B, 0x74, 0x24, 0x08, 0x6A, 0xFF, 0x6A, 0x00, 0x83,
    0xEC, 0x0C, 0xD9, 0x5C, 0x24, 0x08, 0x8B, 0xCE, 0xD9, 0xE8, 0xD9, 0x5C, 0x24, 0x04, 0xD9, 0xEE,
    0xD9, 0x1C, 0x24, 0x6A, 0x00, 0x6A, 0x00, 0x68, 0x40, 0x03, 0x00, 0x00, 0xE8, 0x2F, 0x69, 0x51,
    0x00, 0x68, 0xA6, 0x00, 0x00, 0x00, 0x6A, 0x16, 0x56, 0xE8, 0x32, 0xE2, 0xFD, 0xFF, 0x83, 0xC4,
    0x2C, 0x5E, 0xC3,
];
const EN_K16_BODY_CALLS: [usize; 2] = [0x2c, 0x39];
const EX_K15_BODY: [u8; 15] = [
    0x8B, 0x44, 0x24, 0x04, 0x81, 0xA0, 0xC0, 0x00, 0x00, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0xC3,
];
const EX_K15_BODY_CALLS: [usize; 0] = [];
const EX_K0F_BODY: [u8; 39] = [
    0x8B, 0x44, 0x24, 0x04, 0xF6, 0x80, 0x40, 0x01, 0x00, 0x00, 0x04, 0x75, 0x05, 0xE8, 0xCE, 0x4F,
    0x58, 0x00, 0x8B, 0x40, 0x44, 0x56, 0x50, 0x50, 0xBE, 0x59, 0x00, 0x00, 0x00, 0xE8, 0x2E, 0x11,
    0xFD, 0xFF, 0x83, 0xC4, 0x08, 0x5E, 0xC3,
];
const EX_K0F_BODY_CALLS: [usize; 2] = [0xd, 0x1d];
const EX_K10_BODY: [u8; 39] = [
    0x8B, 0x44, 0x24, 0x04, 0xF6, 0x80, 0x40, 0x01, 0x00, 0x00, 0x04, 0x75, 0x05, 0xE8, 0xDE, 0x3D,
    0x58, 0x00, 0x8B, 0x40, 0x44, 0x56, 0x50, 0x50, 0xBE, 0x59, 0x00, 0x00, 0x00, 0xE8, 0x3E, 0xFF,
    0xFC, 0xFF, 0x83, 0xC4, 0x08, 0x5E, 0xC3,
];
const EX_K10_BODY_CALLS: [usize; 2] = [0xd, 0x1d];
const EX_K1E_BODY: [u8; 72] = [
    0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0x47, 0x44, 0x50, 0x50, 0xBE, 0x48, 0x00, 0x00, 0x00,
    0xE8, 0xFB, 0x09, 0xFA, 0xFF, 0x8B, 0x47, 0x44, 0x50, 0x50, 0xBE, 0x6F, 0x00, 0x00, 0x00, 0xE8,
    0xEC, 0x09, 0xFA, 0xFF, 0x8B, 0x47, 0x44, 0x50, 0x50, 0xBE, 0x6E, 0x00, 0x00, 0x00, 0xE8, 0xDD,
    0x09, 0xFA, 0xFF, 0x8B, 0x7F, 0x44, 0x57, 0x57, 0xBE, 0x58, 0x00, 0x00, 0x00, 0xE8, 0xCE, 0x09,
    0xFA, 0xFF, 0x83, 0xC4, 0x20, 0x5F, 0x5E, 0xC3,
];
const EX_K1E_BODY_CALLS: [usize; 4] = [0x10, 0x1f, 0x2e, 0x3d];
const OE_K03_BODY: [u8; 43] = [
    0x8B, 0x44, 0x24, 0x08, 0x8B, 0x40, 0x08, 0x83, 0xF8, 0x12, 0x74, 0x11, 0x83, 0xF8, 0x59, 0x75,
    0x17, 0x8B, 0x44, 0x24, 0x04, 0xE8, 0x86, 0xEB, 0xFF, 0xFF, 0x33, 0xC0, 0xC3, 0x57, 0x8B, 0x7C,
    0x24, 0x08, 0xE8, 0x19, 0xE4, 0xFF, 0xFF, 0x5F, 0x33, 0xC0, 0xC3,
];
const OE_K03_BODY_CALLS: [usize; 2] = [0x15, 0x22];
const OE_K0C_BODY: [u8; 56] = [
    0x8B, 0x44, 0x24, 0x08, 0x83, 0x78, 0x08, 0x1C, 0x56, 0x75, 0x22, 0x8B, 0x74, 0x24, 0x08, 0xF6,
    0x86, 0x50, 0x01, 0x00, 0x00, 0x03, 0x75, 0x19, 0x80, 0xBE, 0x48, 0x01, 0x00, 0x00, 0x76, 0x74,
    0x10, 0x8B, 0x40, 0x24, 0x50, 0xE8, 0x46, 0x3F, 0x03, 0x00, 0x83, 0xC4, 0x04, 0x33, 0xC0, 0x5E,
    0xC3, 0xB8, 0x02, 0x00, 0x00, 0x00, 0x5E, 0xC3,
];
const OE_K0C_BODY_CALLS: [usize; 1] = [0x25];
const OE_K17_BODY: [u8; 64] = [
    0x8B, 0x4C, 0x24, 0x08, 0x8B, 0x51, 0x08, 0x83, 0xFA, 0x05, 0x8B, 0x44, 0x24, 0x04, 0x74, 0x0D,
    0x83, 0xFA, 0x59, 0x75, 0x28, 0xE8, 0xB6, 0xFA, 0xFF, 0xFF, 0x33, 0xC0, 0xC3, 0x8B, 0x51, 0x0C,
    0x3B, 0x50, 0x44, 0x74, 0x18, 0x89, 0x90, 0x54, 0x01, 0x00, 0x00, 0x8B, 0x51, 0x24, 0x89, 0x90,
    0x58, 0x01, 0x00, 0x00, 0x8A, 0x49, 0x28, 0x88, 0x88, 0x5C, 0x01, 0x00, 0x00, 0x33, 0xC0, 0xC3,
];
const OE_K17_BODY_CALLS: [usize; 1] = [0x15];
const OE_K13_BODY: [u8; 74] = [
    0x8B, 0x4C, 0x24, 0x08, 0x8B, 0x51, 0x08, 0x83, 0xFA, 0x05, 0x8B, 0x44, 0x24, 0x04, 0x74, 0x17,
    0x83, 0xFA, 0x53, 0x75, 0x32, 0x8B, 0x88, 0x50, 0x01, 0x00, 0x00, 0x51, 0xE8, 0xBF, 0xEE, 0xFF,
    0xFF, 0x83, 0xC4, 0x04, 0x33, 0xC0, 0xC3, 0x8B, 0x51, 0x0C, 0x3B, 0x50, 0x44, 0x74, 0x18, 0x89,
    0x90, 0x60, 0x01, 0x00, 0x00, 0x8B, 0x51, 0x24, 0x89, 0x90, 0x64, 0x01, 0x00, 0x00, 0x8A, 0x49,
    0x28, 0x88, 0x88, 0x68, 0x01, 0x00, 0x00, 0x33, 0xC0, 0xC3,
];
const OE_K13_BODY_CALLS: [usize; 1] = [0x1c];
const OE_K20_BODY: [u8; 76] = [
    0x8B, 0x44, 0x24, 0x08, 0x8B, 0x40, 0x08, 0x83, 0xF8, 0x4F, 0x74, 0x19, 0x83, 0xF8, 0x53, 0x74,
    0x06, 0xB8, 0x03, 0x00, 0x00, 0x00, 0xC3, 0x8B, 0x44, 0x24, 0x04, 0x80, 0x88, 0x80, 0x0E, 0x00,
    0x00, 0x02, 0x33, 0xC0, 0xC3, 0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x8B, 0x47, 0x44, 0x50, 0x50,
    0xBE, 0x50, 0x00, 0x00, 0x00, 0xE8, 0x56, 0xF4, 0xF9, 0xFF, 0x83, 0xC4, 0x08, 0xC7, 0x87, 0xF0,
    0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5F, 0x5E, 0x33, 0xC0, 0xC3,
];
const OE_K20_BODY_CALLS: [usize; 1] = [0x35];
const OE_K10_BODY: [u8; 22] = [
    0x8B, 0x44, 0x24, 0x08, 0x83, 0x78, 0x08, 0x59, 0x75, 0x09, 0x8B, 0x44, 0x24, 0x04, 0xE8, 0xAD,
    0xF4, 0xFF, 0xFF, 0x33, 0xC0, 0xC3,
];
const OE_K10_BODY_CALLS: [usize; 1] = [0xe];
const OE_K1D_BODY: [u8; 71] = [
    0x8B, 0x4C, 0x24, 0x04, 0x85, 0xC9, 0x74, 0x3C, 0x83, 0x79, 0x4C, 0x01, 0x75, 0x36, 0x80, 0xB9,
    0xCC, 0x03, 0x00, 0x00, 0x02, 0x75, 0x2D, 0x8B, 0x44, 0x24, 0x08, 0x8B, 0x50, 0x08, 0x83, 0xFA,
    0x37, 0x74, 0x15, 0x83, 0xFA, 0x53, 0x75, 0x1C, 0x6A, 0x00, 0x6A, 0x00, 0x51, 0xE8, 0x9E, 0x6F,
    0xFE, 0xFF, 0x83, 0xC4, 0x0C, 0x33, 0xC0, 0xC3, 0x80, 0x89, 0x50, 0x01, 0x00, 0x00, 0x08, 0xE8,
    0x7C, 0xD5, 0xFD, 0xFF, 0x33, 0xC0, 0xC3,
];
const OE_K1D_BODY_CALLS: [usize; 2] = [0x2d, 0x3f];
const UP_K09_BODY: [u8; 66] = [
    0x56, 0x8B, 0x74, 0x24, 0x08, 0x8B, 0x86, 0x58, 0x01, 0x00, 0x00, 0x85, 0xC0, 0x8D, 0x96, 0x50,
    0x01, 0x00, 0x00, 0x74, 0x2B, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04,
    0x8B, 0x89, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC9, 0x74, 0x16, 0x39, 0x41, 0x44, 0x75, 0x11, 0x83,
    0x79, 0x4C, 0x03, 0x75, 0x0B, 0x51, 0x52, 0x56, 0xE8, 0x53, 0xF8, 0xFF, 0xFF, 0x83, 0xC4, 0x0C,
    0x5E, 0xC3,
];
const UP_K09_BODY_CALLS: [usize; 1] = [0x38];
const IQ_BODY: [u8; 130] = [
    0x85, 0xC9, 0x56, 0x74, 0x76, 0x8B, 0xB1, 0xB0, 0x00, 0x00, 0x00, 0x85, 0xF6, 0x74, 0x26, 0xA1,
    0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x1D, 0x8B, 0x80, 0x08, 0x04, 0x00, 0x00, 0x85, 0xC0,
    0x74, 0x13, 0x8A, 0x16, 0x3A, 0x90, 0xB0, 0x01, 0x00, 0x00, 0x75, 0x4F, 0xF6, 0x86, 0x7C, 0x06,
    0x00, 0x00, 0x01, 0x75, 0x46, 0x8A, 0x81, 0x49, 0x01, 0x00, 0x00, 0x3C, 0xFF, 0x74, 0x3C, 0xF6,
    0x81, 0x80, 0x00, 0x00, 0x00, 0x0B, 0x75, 0x33, 0xF7, 0x81, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x40, 0x00, 0x74, 0x27, 0x0F, 0xBE, 0xC0, 0x8D, 0x14, 0xC5, 0x00, 0x00, 0x00, 0x00, 0x2B, 0xD0,
    0x8B, 0x04, 0x95, 0x74, 0x55, 0xDD, 0x00, 0x85, 0xC0, 0x74, 0x10, 0x8B, 0x54, 0x24, 0x08, 0x52,
    0x51, 0xFF, 0xD0, 0x83, 0xC4, 0x08, 0x83, 0xF8, 0x01, 0x75, 0x05, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0x5E, 0xC3,
];
const IQ_BODY_CALLS: [usize; 0] = [];

const NR_K19_BODY: [u8; 70] = [
    0x8B, 0x4C, 0x24, 0x0C, 0x85, 0xC9, 0x75, 0x10, 0x8B, 0x44, 0x24, 0x04, 0x51, 0x51, 0x50, 0xE8,
    0x6C, 0xD1, 0xFE, 0xFF, 0x83, 0xC4, 0x0C, 0xC3, 0x8B, 0x44, 0x24, 0x10, 0x8B, 0x54, 0x24, 0x04,
    0x56, 0x0F, 0xB7, 0x30, 0x8B, 0x34, 0x0E, 0x89, 0xB2, 0x50, 0x01, 0x00, 0x00, 0x66, 0x83, 0x00,
    0x04, 0x0F, 0xB7, 0x30, 0x0F, 0xB7, 0xF6, 0x8B, 0x0C, 0x0E, 0x89, 0x8A, 0x54, 0x01, 0x00, 0x00,
    0x66, 0x83, 0x00, 0x04, 0x5E, 0xC3,
];
const NR_K19_BODY_CALLS: [usize; 1] = [0xf];
const NR_K01_BODY: [u8; 126] = [
    0x8B, 0x54, 0x24, 0x0C, 0x83, 0xEC, 0x08, 0x85, 0xD2, 0x74, 0x5E, 0x8B, 0x44, 0x24, 0x18, 0x0F,
    0xB7, 0x08, 0x56, 0x0F, 0xB7, 0xF1, 0x8B, 0x34, 0x16, 0x89, 0x74, 0x24, 0x18, 0x83, 0xC1, 0x04,
    0xD9, 0x44, 0x24, 0x18, 0x66, 0x89, 0x08, 0x0F, 0xB7, 0xF1, 0x8B, 0x34, 0x16, 0x83, 0xC1, 0x04,
    0x66, 0x89, 0x08, 0x89, 0x74, 0x24, 0x04, 0x0F, 0xB7, 0xF1, 0x8B, 0x14, 0x16, 0x83, 0xC1, 0x04,
    0x66, 0x89, 0x08, 0x8B, 0x44, 0x24, 0x10, 0x89, 0x54, 0x24, 0x08, 0x5E, 0xD9, 0x98, 0x50, 0x01,
    0x00, 0x00, 0xD9, 0x04, 0x24, 0xD9, 0x98, 0x54, 0x01, 0x00, 0x00, 0xD9, 0x44, 0x24, 0x04, 0xD9,
    0x98, 0x58, 0x01, 0x00, 0x00, 0x83, 0xC4, 0x08, 0xC3, 0x8B, 0x44, 0x24, 0x0C, 0x6A, 0x00, 0x6A,
    0x00, 0x50, 0xE8, 0x19, 0x56, 0x04, 0x00, 0x83, 0xC4, 0x0C, 0x83, 0xC4, 0x08, 0xC3,
];
const NR_K01_BODY_CALLS: [usize; 1] = [0x72];
const NR_K05_BODY: [u8; 164] = [
    0x8B, 0x4C, 0x24, 0x04, 0x83, 0xB9, 0xF0, 0x0A, 0x00, 0x00, 0x00, 0x56, 0x0F, 0x84, 0x83, 0x00,
    0x00, 0x00, 0x8B, 0x74, 0x24, 0x10, 0x85, 0xF6, 0x74, 0x7B, 0x8B, 0x44, 0x24, 0x14, 0x0F, 0xB7,
    0x10, 0x0F, 0xB6, 0x14, 0x32, 0x88, 0x91, 0x50, 0x01, 0x00, 0x00, 0x66, 0x83, 0x00, 0x01, 0x0F,
    0xB7, 0x10, 0x0F, 0xB7, 0xD2, 0x0F, 0xB6, 0x14, 0x32, 0x88, 0x91, 0x51, 0x01, 0x00, 0x00, 0x66,
    0x83, 0x00, 0x01, 0x0F, 0xB7, 0x10, 0x0F, 0xB7, 0xD2, 0x0F, 0xB6, 0x14, 0x32, 0x88, 0x91, 0x52,
    0x01, 0x00, 0x00, 0x66, 0x83, 0x00, 0x01, 0x0F, 0xB7, 0x10, 0x0F, 0xB7, 0xD2, 0x0F, 0xB6, 0x14,
    0x32, 0x88, 0x91, 0x53, 0x01, 0x00, 0x00, 0x66, 0x83, 0x00, 0x01, 0x0F, 0xB7, 0x10, 0x0F, 0xB7,
    0xD2, 0x0F, 0xB6, 0x14, 0x32, 0x88, 0x91, 0x54, 0x01, 0x00, 0x00, 0x66, 0x83, 0x00, 0x01, 0x0F,
    0xB7, 0x10, 0x0F, 0xB7, 0xD2, 0x0F, 0xB6, 0x14, 0x32, 0x88, 0x91, 0x55, 0x01, 0x00, 0x00, 0x66,
    0x83, 0x00, 0x01, 0x5E, 0xC3, 0x6A, 0x00, 0x6A, 0x00, 0x51, 0xE8, 0x91, 0xA8, 0x03, 0x00, 0x83,
    0xC4, 0x0C, 0x5E, 0xC3,
];
const NR_K05_BODY_CALLS: [usize; 1] = [0x9a];
const NR_K14_BODY: [u8; 69] = [
    0x8B, 0x4C, 0x24, 0x04, 0x56, 0x8B, 0x74, 0x24, 0x10, 0x85, 0xF6, 0x75, 0x06, 0x5E, 0xE9, 0x3D,
    0x00, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x14, 0x0F, 0xB7, 0x10, 0x8A, 0x14, 0x32, 0x88, 0x91, 0x50,
    0x01, 0x00, 0x00, 0x66, 0x83, 0x00, 0x01, 0x83, 0xB9, 0xF0, 0x0A, 0x00, 0x00, 0x00, 0x0F, 0xB7,
    0x10, 0x74, 0x10, 0x0F, 0xB7, 0xD2, 0x8B, 0x14, 0x32, 0x89, 0x91, 0x54, 0x01, 0x00, 0x00, 0x66,
    0x83, 0x00, 0x04, 0x5E, 0xC3,
];
const NR_K14_BODY_CALLS: [usize; 0] = [];
const NR_K06_BODY: [u8; 190] = [
    0x8B, 0x4C, 0x24, 0x0C, 0x85, 0xC9, 0x75, 0x10, 0x8B, 0x44, 0x24, 0x04, 0x51, 0x51, 0x50, 0xE8,
    0xFC, 0x70, 0x03, 0x00, 0x83, 0xC4, 0x0C, 0xC3, 0x8B, 0x44, 0x24, 0x10, 0x0F, 0xB7, 0x10, 0x8A,
    0x14, 0x0A, 0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x88, 0x97, 0x50, 0x01, 0x00, 0x00, 0x66, 0x83,
    0x00, 0x01, 0x0F, 0xB7, 0x10, 0x0F, 0xB7, 0xF2, 0x0F, 0xB7, 0x0C, 0x0E, 0x83, 0xC2, 0x02, 0x66,
    0x81, 0xF9, 0xFF, 0xFF, 0x66, 0x89, 0x10, 0x75, 0x0B, 0x33, 0xC0, 0x89, 0x87, 0x54, 0x01, 0x00,
    0x00, 0x5F, 0x5E, 0xC3, 0x0F, 0xB7, 0xF1, 0x8B, 0xC6, 0xC1, 0xE0, 0x0F, 0xF7, 0xD0, 0x03, 0xC6,
    0x8B, 0xC8, 0xC1, 0xF9, 0x0A, 0x33, 0xC8, 0x8D, 0x0C, 0xC9, 0x8B, 0xC1, 0xC1, 0xF8, 0x06, 0x33,
    0xC8, 0x8B, 0xD1, 0xC1, 0xE2, 0x0B, 0xF7, 0xD2, 0x03, 0xCA, 0x8B, 0xC1, 0xC1, 0xF8, 0x10, 0x33,
    0xC1, 0x8B, 0x0D, 0x58, 0xE2, 0x12, 0x02, 0x99, 0xF7, 0x79, 0x60, 0x8B, 0x41, 0x5C, 0x8B, 0x14,
    0x90, 0x85, 0xD2, 0x74, 0xB4, 0x3B, 0x72, 0x0C, 0x74, 0x12, 0x8B, 0x52, 0x04, 0x85, 0xD2, 0x75,
    0xF4, 0x33, 0xC0, 0x89, 0x87, 0x54, 0x01, 0x00, 0x00, 0x5F, 0x5E, 0xC3, 0x8B, 0x12, 0x85, 0xD2,
    0x74, 0x97, 0x8B, 0x42, 0x04, 0x89, 0x87, 0x54, 0x01, 0x00, 0x00, 0x5F, 0x5E, 0xC3,
];
const NR_K06_BODY_CALLS: [usize; 1] = [0xf];
const NR_K11_BODY: [u8; 190] = [
    0x8B, 0x4C, 0x24, 0x0C, 0x85, 0xC9, 0x75, 0x10, 0x8B, 0x44, 0x24, 0x04, 0x51, 0x51, 0x50, 0xE8,
    0x4C, 0x54, 0x01, 0x00, 0x83, 0xC4, 0x0C, 0xC3, 0x8B, 0x44, 0x24, 0x10, 0x0F, 0xB7, 0x10, 0x8A,
    0x14, 0x0A, 0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x88, 0x97, 0x50, 0x01, 0x00, 0x00, 0x66, 0x83,
    0x00, 0x01, 0x0F, 0xB7, 0x10, 0x0F, 0xB7, 0xF2, 0x0F, 0xB7, 0x0C, 0x0E, 0x83, 0xC2, 0x02, 0x66,
    0x81, 0xF9, 0xFF, 0xFF, 0x66, 0x89, 0x10, 0x75, 0x0B, 0x33, 0xC0, 0x89, 0x87, 0x54, 0x01, 0x00,
    0x00, 0x5F, 0x5E, 0xC3, 0x0F, 0xB7, 0xF1, 0x8B, 0xC6, 0xC1, 0xE0, 0x0F, 0xF7, 0xD0, 0x03, 0xC6,
    0x8B, 0xC8, 0xC1, 0xF9, 0x0A, 0x33, 0xC8, 0x8D, 0x0C, 0xC9, 0x8B, 0xC1, 0xC1, 0xF8, 0x06, 0x33,
    0xC8, 0x8B, 0xD1, 0xC1, 0xE2, 0x0B, 0xF7, 0xD2, 0x03, 0xCA, 0x8B, 0xC1, 0xC1, 0xF8, 0x10, 0x33,
    0xC1, 0x8B, 0x0D, 0x58, 0xE2, 0x12, 0x02, 0x99, 0xF7, 0x79, 0x60, 0x8B, 0x41, 0x5C, 0x8B, 0x14,
    0x90, 0x85, 0xD2, 0x74, 0xB4, 0x3B, 0x72, 0x0C, 0x74, 0x12, 0x8B, 0x52, 0x04, 0x85, 0xD2, 0x75,
    0xF4, 0x33, 0xC0, 0x89, 0x87, 0x54, 0x01, 0x00, 0x00, 0x5F, 0x5E, 0xC3, 0x8B, 0x12, 0x85, 0xD2,
    0x74, 0x97, 0x8B, 0x42, 0x04, 0x89, 0x87, 0x54, 0x01, 0x00, 0x00, 0x5F, 0x5E, 0xC3,
];
const NR_K11_BODY_CALLS: [usize; 1] = [0xf];
const NR_K1F_BODY: [u8; 190] = [
    0x8B, 0x4C, 0x24, 0x0C, 0x85, 0xC9, 0x75, 0x10, 0x8B, 0x44, 0x24, 0x04, 0x51, 0x51, 0x50, 0xE8,
    0x9C, 0x59, 0xFE, 0xFF, 0x83, 0xC4, 0x0C, 0xC3, 0x8B, 0x44, 0x24, 0x10, 0x0F, 0xB7, 0x10, 0x8A,
    0x14, 0x0A, 0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x88, 0x97, 0x50, 0x01, 0x00, 0x00, 0x66, 0x83,
    0x00, 0x01, 0x0F, 0xB7, 0x10, 0x0F, 0xB7, 0xF2, 0x0F, 0xB7, 0x0C, 0x0E, 0x83, 0xC2, 0x02, 0x66,
    0x81, 0xF9, 0xFF, 0xFF, 0x66, 0x89, 0x10, 0x75, 0x0B, 0x33, 0xC0, 0x89, 0x87, 0x54, 0x01, 0x00,
    0x00, 0x5F, 0x5E, 0xC3, 0x0F, 0xB7, 0xF1, 0x8B, 0xC6, 0xC1, 0xE0, 0x0F, 0xF7, 0xD0, 0x03, 0xC6,
    0x8B, 0xC8, 0xC1, 0xF9, 0x0A, 0x33, 0xC8, 0x8D, 0x0C, 0xC9, 0x8B, 0xC1, 0xC1, 0xF8, 0x06, 0x33,
    0xC8, 0x8B, 0xD1, 0xC1, 0xE2, 0x0B, 0xF7, 0xD2, 0x03, 0xCA, 0x8B, 0xC1, 0xC1, 0xF8, 0x10, 0x33,
    0xC1, 0x8B, 0x0D, 0x58, 0xE2, 0x12, 0x02, 0x99, 0xF7, 0x79, 0x60, 0x8B, 0x41, 0x5C, 0x8B, 0x14,
    0x90, 0x85, 0xD2, 0x74, 0xB4, 0x3B, 0x72, 0x0C, 0x74, 0x12, 0x8B, 0x52, 0x04, 0x85, 0xD2, 0x75,
    0xF4, 0x33, 0xC0, 0x89, 0x87, 0x54, 0x01, 0x00, 0x00, 0x5F, 0x5E, 0xC3, 0x8B, 0x12, 0x85, 0xD2,
    0x74, 0x97, 0x8B, 0x42, 0x04, 0x89, 0x87, 0x54, 0x01, 0x00, 0x00, 0x5F, 0x5E, 0xC3,
];
const NR_K1F_BODY_CALLS: [usize; 1] = [0xf];
const NR_K15_BODY: [u8; 177] = [
    0x8B, 0x44, 0x24, 0x0C, 0x85, 0xC0, 0x75, 0x10, 0x50, 0x50, 0x8B, 0x44, 0x24, 0x0C, 0x50, 0xE8,
    0xBC, 0xFD, 0xFE, 0xFF, 0x83, 0xC4, 0x0C, 0xC3, 0x8B, 0x54, 0x24, 0x10, 0x0F, 0xB7, 0x0A, 0x56,
    0x0F, 0xB7, 0xF1, 0x0F, 0xB7, 0x04, 0x06, 0x83, 0xC1, 0x02, 0x66, 0x3D, 0xFF, 0xFF, 0x66, 0x89,
    0x0A, 0x75, 0x0E, 0x8B, 0x54, 0x24, 0x08, 0x33, 0xC0, 0x89, 0x82, 0x50, 0x01, 0x00, 0x00, 0x5E,
    0xC3, 0x0F, 0xB7, 0xF0, 0x8B, 0xC6, 0xC1, 0xE0, 0x0F, 0xF7, 0xD0, 0x03, 0xC6, 0x8B, 0xC8, 0xC1,
    0xF9, 0x0A, 0x33, 0xC8, 0x8D, 0x0C, 0xC9, 0x8B, 0xD1, 0xC1, 0xFA, 0x06, 0x33, 0xCA, 0x8B, 0xC1,
    0xC1, 0xE0, 0x0B, 0xF7, 0xD0, 0x03, 0xC8, 0x8B, 0xC1, 0xC1, 0xF8, 0x10, 0x33, 0xC1, 0x8B, 0x0D,
    0x58, 0xE2, 0x12, 0x02, 0x99, 0xF7, 0x79, 0x60, 0x8B, 0x49, 0x5C, 0x8B, 0x14, 0x91, 0x85, 0xD2,
    0x74, 0xB1, 0x3B, 0x72, 0x0C, 0x74, 0x15, 0x8B, 0x52, 0x04, 0x85, 0xD2, 0x75, 0xF4, 0x8B, 0x54,
    0x24, 0x08, 0x33, 0xC0, 0x89, 0x82, 0x50, 0x01, 0x00, 0x00, 0x5E, 0xC3, 0x8B, 0x12, 0x85, 0xD2,
    0x74, 0x91, 0x8B, 0x42, 0x04, 0x8B, 0x54, 0x24, 0x08, 0x89, 0x82, 0x50, 0x01, 0x00, 0x00, 0x5E,
    0xC3,
];
const NR_K15_BODY_CALLS: [usize; 1] = [0xf];
const NR_K21_BODY: [u8; 177] = [
    0x8B, 0x44, 0x24, 0x0C, 0x85, 0xC0, 0x75, 0x10, 0x50, 0x50, 0x8B, 0x44, 0x24, 0x0C, 0x50, 0xE8,
    0x3C, 0x49, 0xFE, 0xFF, 0x83, 0xC4, 0x0C, 0xC3, 0x8B, 0x54, 0x24, 0x10, 0x0F, 0xB7, 0x0A, 0x56,
    0x0F, 0xB7, 0xF1, 0x0F, 0xB7, 0x04, 0x06, 0x83, 0xC1, 0x02, 0x66, 0x3D, 0xFF, 0xFF, 0x66, 0x89,
    0x0A, 0x75, 0x0E, 0x8B, 0x54, 0x24, 0x08, 0x33, 0xC0, 0x89, 0x82, 0x50, 0x01, 0x00, 0x00, 0x5E,
    0xC3, 0x0F, 0xB7, 0xF0, 0x8B, 0xC6, 0xC1, 0xE0, 0x0F, 0xF7, 0xD0, 0x03, 0xC6, 0x8B, 0xC8, 0xC1,
    0xF9, 0x0A, 0x33, 0xC8, 0x8D, 0x0C, 0xC9, 0x8B, 0xD1, 0xC1, 0xFA, 0x06, 0x33, 0xCA, 0x8B, 0xC1,
    0xC1, 0xE0, 0x0B, 0xF7, 0xD0, 0x03, 0xC8, 0x8B, 0xC1, 0xC1, 0xF8, 0x10, 0x33, 0xC1, 0x8B, 0x0D,
    0x58, 0xE2, 0x12, 0x02, 0x99, 0xF7, 0x79, 0x60, 0x8B, 0x49, 0x5C, 0x8B, 0x14, 0x91, 0x85, 0xD2,
    0x74, 0xB1, 0x3B, 0x72, 0x0C, 0x74, 0x15, 0x8B, 0x52, 0x04, 0x85, 0xD2, 0x75, 0xF4, 0x8B, 0x54,
    0x24, 0x08, 0x33, 0xC0, 0x89, 0x82, 0x50, 0x01, 0x00, 0x00, 0x5E, 0xC3, 0x8B, 0x12, 0x85, 0xD2,
    0x74, 0x91, 0x8B, 0x42, 0x04, 0x8B, 0x54, 0x24, 0x08, 0x89, 0x82, 0x50, 0x01, 0x00, 0x00, 0x5E,
    0xC3,
];
const NR_K21_BODY_CALLS: [usize; 1] = [0xf];

const PI_K00_BODY: [u8; 64] = [
    0x56, 0x8B, 0x74, 0x24, 0x08, 0x57, 0x6A, 0x34, 0x33, 0xFF, 0x8D, 0x46, 0x28, 0x57, 0x50, 0xE8,
    0x0C, 0xA2, 0x85, 0x00, 0x33, 0xC0, 0x66, 0x89, 0x06, 0x88, 0x46, 0x02, 0x80, 0x0E, 0x14, 0x83,
    0xC8, 0xFF, 0x83, 0xC4, 0x0C, 0x89, 0x7E, 0x0C, 0x89, 0x7E, 0x10, 0x89, 0x7E, 0x14, 0x89, 0x7E,
    0x04, 0x5F, 0x89, 0x46, 0x18, 0x89, 0x46, 0x1C, 0x89, 0x46, 0x08, 0x89, 0x46, 0x20, 0x5E, 0xC3,
];
const PI_K00_BODY_CALLS: [usize; 1] = [0xf];

/// A stub's address.
fn addr(f: extern "C" fn()) -> usize {
    f as usize
}
