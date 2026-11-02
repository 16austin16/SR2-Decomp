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
const POOL_BY_ID: usize = 47;
const POOL_STATE2: usize = 48;
const POOL_STATE1: usize = 49;
const SEND_JOIN_NEW_SESSION: usize = 50;
const SEND_JOIN_SUCCESS: usize = 51;
const SEND_JOIN_FAILED: usize = 52;
const SEND_JOIN_COMPLETE: usize = 53;
const SEND_MP_INFO: usize = 54;
const SEND_UPD_STATE_SUB3: usize = 55;
const SET_PHASE_BROADCAST: usize = 56;
const SEND_JOIN_DENY: usize = 57;
const SEND_UPD_CONN: usize = 58;
const SEND_TO_LIST_OR_REMOTES: usize = 59;
const RECENT_IDS_PUSH: usize = 60;
const ACK_WINDOW_PUSH: usize = 61;
const ACK_RECORD: usize = 62;
const CHECK_TIMEOUT: usize = 63;
const TIMEOUT_EXPIRED: usize = 64;
const TIMEOUT_ARM_10S: usize = 65;
const RATE_METER_INIT: usize = 66;
const RATE_METER_REMAINING: usize = 67;
const CONN_NOTE_RECEIVED: usize = 68;
const ACK_WINDOW_MARK_ALL: usize = 69;
const RESET_ACK_WINDOW: usize = 70;
const CLOCK_PREDICTED_MS: usize = 71;
const CLOCK_ADVANCE: usize = 72;
const OUT_OF_PHASE_MASK: usize = 73;
const PEERS_PHASE_EQUAL: usize = 74;
const ALL_CONNS_STATE_EQ: usize = 75;
const ALL_CONNS_STATE_GE: usize = 76;
const LOBBY_ANY_CONN_STATE_4: usize = 77;
const STATE_IN_SET_B: usize = 78;
const STATE_IS_8: usize = 79;
const LOCAL_CONN_ID: usize = 80;
const ID_TO_HUMAN: usize = 81;
const MAP_SET: usize = 82;
const FIND_RESERVATION: usize = 83;
const REMOVE_RESERVATION: usize = 84;
const GUID_IN_ARRAY: usize = 85;
const FIND_BEST_CONN: usize = 86;
const SESSION_INFO_BASE_INIT: usize = 87;
const BYTE_NC_INIT: usize = 88;
const BYTE_NC_INIT_BITS: usize = 89;
const BIT_NC_INIT: usize = 90;
const BIT_NC_INIT_BITS: usize = 91;
const FLOAT_COMP_INIT: usize = 92;
const FLOAT_COMP_INIT_COPY: usize = 93;
const FLOAT_COMP_COPY: usize = 94;
const VECTOR3_COMP_CTOR: usize = 95;
const VECTOR2_COMP_CTOR: usize = 96;
const VECTOR3_UNCOMP_CTOR: usize = 97;
const HEADING_COMP_CTOR: usize = 98;
const OWNERSHIP_PENDING: usize = 99;
const CONN_OBJECT_LIST_ADD: usize = 100;
const CONN_OBJECT_LIST_REMOVE: usize = 101;
const GUID_ID_EQUAL: usize = 102;
const GUID_ENTRY_READ: usize = 103;
const ID_TO_OBJECT_AC: usize = 104;
const CAPTURE_SNAPSHOT: usize = 105;
const READ_STATE_DELTA: usize = 106;
const GET_SEND_PRIORITY: usize = 107;
const GET_DIRTY_FIELDS: usize = 108;
const ON_STATE_SENT: usize = 109;
const CAN_MIGRATE: usize = 110;
const REQUEST_OWNERSHIP: usize = 111;
const COUNT: usize = 112;

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
    port("net_pool_find_by_id", 0x0081_7D80, SAVED),
    port("net_pool_find_by_id_b", 0x0081_7F40, SAVED),
    port("net_pool_find_by_id_c", 0x0081_8020, SAVED),
    port("net_send_join_new_session", 0x0081_6460, SAVED),
    port("net_send_join_success", 0x0081_65F0, SAVED),
    port("net_send_join_failed", 0x0081_6740, SAVED),
    port("net_send_join_complete", 0x0081_68A0, SAVED),
    port("net_send_mp_info", 0x007F_8DF0, SAVED),
    port("net_send_upd_state_sub3", 0x007F_8D70, SAVED),
    port("net_conn_set_phase_broadcast", 0x007F_8C50, SAVED),
    port("net_send_join_deny", 0x0081_6F60, SAVED),
    port("net_send_upd_conn", 0x0081_6B50, SAVED),
    port("net_send_packet_to_list_or_remotes", 0x0081_48C0, SAVED),
    port("net_conn_recent_ids_push", 0x007F_AA60, EAX | SAVED),
    port("net_conn_ack_window_push", 0x007F_81D0, ECX | SAVED),
    port("net_conn_ack_record", 0x007F_8270, EAX | SAVED),
    port("net_conn_check_timeout", 0x007F_A9E0, SAVED),
    port("net_timeout_expired", 0x007F_C8B0, SAVED),
    port("net_timeout_arm_10s", 0x007F_C8F0, SAVED),
    port("net_rate_meter_init", 0x007F_B1C0, SAVED),
    port("net_rate_meter_remaining", 0x007F_B2B0, SAVED),
    port("net_conn_note_received", 0x007F_B100, EAX | SAVED),
    port("net_conn_ack_window_mark_all", 0x007F_B190, EDX | SAVED),
    port("net_conn_reset_ack_window", 0x007F_8160, EAX | SAVED),
    port("net_clock_predicted_ms", 0x007F_CAB0, SAVED),
    port("net_clock_advance", 0x007F_C930, SAVED),
    port("session_out_of_phase_mask", 0x0080_FF70, SAVED),
    port("session_peers_phase_equal", 0x0080_FF00, SAVED),
    port("session_all_conns_state_eq", 0x0081_0040, SAVED),
    port("session_all_conns_state_ge", 0x0081_00F0, SAVED),
    port("lobby_any_conn_state_4", 0x0081_0080, SAVED),
    port("session_state_in_set_b", 0x0080_CEB0, SAVED),
    port("session_state_is_8", 0x0080_CF10, EDX | SAVED),
    port("net_local_conn_id", 0x007F_7A00, ECX | EDX | SAVED),
    port("net_id_to_human", 0x007F_8020, SAVED),
    port("net_remote_handle_map_set", 0x0080_7A20, SAVED),
    port("session_find_reservation", 0x0080_F2E0, SAVED),
    port("session_remove_reservation", 0x0080_F340, SAVED),
    port("session_guid_in_array", 0x0080_C350, SAVED),
    port("session_find_best_conn", 0x0080_C4E0, SAVED),
    port("session_info_base_init", 0x0080_C560, EAX | EDX | SAVED),
    port("net_byte_noncompressor_init", 0x007F_DBB0, EAX | ECX | EDX | SAVED),
    port("net_byte_noncompressor_init_bits", 0x007F_DBC0, EAX | EDX | SAVED),
    port("net_bit_noncompressor_init", 0x007F_DC90, EAX | EDX | SAVED),
    port("net_bit_noncompressor_init_bits", 0x007F_DCB0, EAX | SAVED),
    port("net_float_compressor_init", 0x007F_DE30, EAX | ECX | SAVED),
    port("net_float_compressor_init_copy", 0x007F_DE60, EAX | EDX | SAVED),
    port("net_float_compressor_copy", 0x007F_E110, EAX | EDX | SAVED),
    port("net_vector3_compressor_ctor", 0x007F_E0B0, EAX | EDX | SAVED),
    port("net_vector2_compressor_ctor", 0x007F_E3A0, EAX | EDX | SAVED),
    port("net_vector3_uncompressed_ctor", 0x007F_E5A0, EAX | ECX | EDX | SAVED),
    port("net_orientation_heading_compressor_ctor", 0x007F_E6A0, EAX | EDX | SAVED),
    port("net_ownership_request_pending", 0x0080_6CE0, SAVED),
    port("net_conn_object_list_add", 0x0080_4940, EAX | EDX | SAVED),
    port("net_conn_object_list_remove", 0x0080_4960, SAVED),
    port("net_guid_id_equal", 0x0081_76C0, SAVED),
    port("net_guid_entry_read", 0x0081_7810, EAX | EDX | SAVED),
    port("net_id_to_object_ac", 0x007F_FDF0, SAVED),
    port("net_object_capture_snapshot", 0x0080_5CD0, SAVED),
    port("net_object_read_state_delta", 0x0080_5D60, SAVED),
    port("net_object_get_send_priority", 0x0080_5E40, SAVED),
    port("net_object_get_dirty_fields", 0x0080_5E90, SAVED),
    port("net_object_on_state_sent", 0x0080_5EC0, SAVED),
    port("net_object_can_migrate", 0x0080_6D50, SAVED),
    port("net_request_ownership", 0x0080_6B90, SAVED),
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

// ===== round 2: pool lookups, packet senders ==============================================================================
// Senders build a packet in a local buffer and call the original send helpers by address
// (0x814670 net_send_packet_to_conn, 0x814740 net_send_packet_to_remotes, ...) with the registers
// and stack arguments the original sets up. Their shadow runs the original copy with the send
// call pointed at a recording stub, then the Rust version with its sends recorded as well; the
// two lists (target, ESI / EDI, stack arguments, packet bytes) are compared, and the winner's
// sends are then issued once for real. All of this runs on the game thread.

const SEND_CONN: usize = 0x0081_4670;
const SEND_REMOTES: usize = 0x0081_4740;
const SEND_RAW: usize = 0x0081_4790;
const IAT_ENTER_CS: usize = 0x00DB_80CC;
const IAT_LEAVE_CS: usize = 0x00DB_80D0;
const IAT_SET_EVENT: usize = 0x00DB_81DC;
const POOL_ITER_NEXT: usize = 0x0069_B9D0;
const POOL_CS: usize = 0x02A1_8944;
const POOL_USERS: usize = 0x02A1_8968;
const POOL_WAITERS: usize = 0x02A1_896C;
const POOL_EVENT: usize = 0x02A1_8960;
const POOL_COUNT: usize = 0x02A1_8940;
const POOL_HEAD: usize = 0x02A1_893C;
const POOL_BASE: usize = 0x02A1_8938;

/// Whether a call now would still be shadowed (no counting).
fn shadowing_active(id: usize) -> bool {
    let p = &PORTS[id];
    p.original.load(Ordering::Relaxed) != 0 && p.calls.load(Ordering::Relaxed) < SHADOW_CALLS
}

#[derive(Clone, PartialEq, Debug)]
struct Sent {
    target: usize,
    esi: u32,
    edi: u32,
    /// Stack arguments; [0] is the buffer pointer (never compared).
    args: Vec<u32>,
    data: Vec<u8>,
}

thread_local! {
    static RECORDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SENT: std::cell::RefCell<Vec<Sent>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Longest packet copy kept per recorded send (the send helpers reject anything over 0x1FF).
const MAX_SENT: usize = 0x1000;

unsafe fn record_send(target: usize, esi: u32, edi: u32, args: &[u32]) {
    unsafe {
        let len = (args[1] as usize).min(MAX_SENT);
        let data = snap_bytes(args[0] as usize, len);
        SENT.with(|s| s.borrow_mut().push(Sent { target, esi, edi, args: args.to_vec(), data }));
    }
}

fn take_sent() -> Vec<Sent> {
    SENT.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

extern "C" fn rec_conn_entry(r: &mut Pushad, a: *const u32) {
    unsafe {
        record_send(SEND_CONN, r.esi, 0, &[*a, *a.add(1), *a.add(2), *a.add(3)]);
        r.eax = 0;
    }
}
regs_thunk!(rec_conn_thunk, rec_conn_entry, "ret");

extern "C" fn rec_remotes_entry(r: &mut Pushad, a: *const u32) {
    unsafe {
        record_send(SEND_REMOTES, 0, r.edi, &[*a, *a.add(1)]);
        r.eax = 0;
    }
}
regs_thunk!(rec_remotes_thunk, rec_remotes_entry, "ret");

extern "C" fn rec_raw_entry(r: &mut Pushad, a: *const u32) {
    unsafe {
        record_send(SEND_RAW, 0, 0, &[r.ecx, *a.add(1), *a]);
        r.eax = 0;
    }
}
regs_thunk!(rec_raw_thunk, rec_raw_entry, "ret");

/// 0x814670: ESI = connection; (buffer, size, a, b) on the stack, `ret` (cdecl).
unsafe fn send_conn_raw(esi: u32, buf: u32, size: u32, a: u32, b: u32) -> u32 {
    unsafe {
        let args = [buf, size, a, b];
        if RECORDING.get() {
            record_send(SEND_CONN, esi, 0, &args);
            return 0;
        }
        call_regs(SEND_CONN, 0, 0, 0, esi, 0, &args)
    }
}

unsafe fn send_conn(esi: u32, buf: &mut [u8], size: u32, a: u32, b: u32) -> u32 {
    unsafe { send_conn_raw(esi, buf.as_mut_ptr() as u32, size, a, b) }
}

/// 0x814740: EDI = session; (buffer, size) on the stack, `ret` (cdecl).
unsafe fn send_remotes_raw(edi: u32, buf: u32, size: u32) -> u32 {
    unsafe {
        let args = [buf, size];
        if RECORDING.get() {
            record_send(SEND_REMOTES, 0, edi, &args);
            return 0;
        }
        call_regs(SEND_REMOTES, 0, 0, 0, 0, edi, &args)
    }
}

unsafe fn send_remotes(edi: u32, buf: &mut [u8], size: u32) -> u32 {
    unsafe { send_remotes_raw(edi, buf.as_mut_ptr() as u32, size) }
}

/// 0x814790: ECX = buffer; stack: an address-like value (the caller's ECX), size; cdecl. The
/// record keeps [buffer, size, that value].
unsafe fn send_raw(buf: &mut [u8], ecx_in: u32, size: u32) -> u32 {
    unsafe {
        let bp = buf.as_mut_ptr() as u32;
        if RECORDING.get() {
            record_send(SEND_RAW, 0, 0, &[bp, size, ecx_in]);
            return 0;
        }
        call_regs(SEND_RAW, 0, bp, 0, 0, 0, &[ecx_in, size])
    }
}

fn sent_equal(a: &[Sent], b: &[Sent], skip3: bool) -> Result<(), String> {
    if a.len() != b.len() {
        return Err(format!("{} sends in the original, {} in Rust", a.len(), b.len()));
    }
    for (k, (x, y)) in a.iter().zip(b).enumerate() {
        if x.target != y.target || x.esi != y.esi || x.edi != y.edi || x.args[1..] != y.args[1..] {
            return Err(format!(
                "send {k}: original {:x?}/{:#x}/{:#x}/{:x?}, Rust {:x?}/{:#x}/{:#x}/{:x?}",
                x.target,
                x.esi,
                x.edi,
                &x.args[1..],
                y.target,
                y.esi,
                y.edi,
                &y.args[1..]
            ));
        }
        let mut dx = x.data.clone();
        let mut dy = y.data.clone();
        if skip3 && dx.len() > 3 && dy.len() > 3 {
            // The 4th header byte is the sequence number the send helper writes: stack garbage here.
            dx[3] = 0;
            dy[3] = 0;
        }
        if dx != dy {
            let i = (0..dx.len().min(dy.len())).find(|&i| dx[i] != dy[i]).unwrap_or(0);
            return Err(format!("send {k}: packet byte {i}: original {:x?}, Rust {:x?}", &x.data, &y.data));
        }
    }
    Ok(())
}

/// Issue the recorded sends for real; the last send's EAX, or None when there were none. With
/// `live` the buffer is the caller's own (kept), else a copy of the recorded bytes.
unsafe fn replay(sent: &[Sent], live: bool) -> Option<u32> {
    unsafe {
        let mut last = None;
        for s in sent {
            let mut buf = vec![0u8; MAX_SENT + 16];
            buf[..s.data.len()].copy_from_slice(&s.data);
            let mut args = s.args.clone();
            if !live {
                args[0] = buf.as_mut_ptr() as u32;
            }
            last = Some(if s.target == SEND_RAW {
                call_regs(s.target, 0, args[0], 0, 0, 0, &[args[2], args[1]])
            } else {
                call_regs(s.target, 0, 0, 0, s.esi, s.edi, &args)
            });
        }
        last
    }
}

/// Sender shadow (see above). `ours` runs with its sends recorded and returns (EAX, whether EAX
/// is the last send's result: then the real send's EAX is used). `regions` are the memory areas
/// the sender writes besides the packet: the original runs on the real state, the state is put
/// back, Rust runs, the two end states are compared and the original's is kept. `live`: the
/// buffer belongs to the caller (the real send writes its 4th byte).
unsafe fn shadow_send_ex(id: usize, p: &Pushad, args: [u32; 3], skip3: bool, live: bool, regions: &[(usize, usize)], ours: impl FnOnce() -> (u32, bool)) -> u32 {
    unsafe {
        let Some((n, orig)) = shadowing(id) else { return ours().0 };
        take_sent();
        RECORDING.set(true);
        let pre: Vec<Vec<u8>> = regions.iter().map(|&(a, l)| snap_bytes(a, l)).collect();
        let inp = Regs::of(p);
        let mut o = inp;
        call_orig(orig, &mut o, args, false);
        let theirs = take_sent();
        let post_o: Vec<Vec<u8>> = regions.iter().map(|&(a, l)| snap_bytes(a, l)).collect();
        for (&(a, l), b) in regions.iter().zip(&pre) {
            if l > 0 {
                std::ptr::copy_nonoverlapping(b.as_ptr(), a as *mut u8, l);
            }
        }
        let (v, is_send) = ours();
        let mine = take_sent();
        RECORDING.set(false);
        let post_r: Vec<Vec<u8>> = regions.iter().map(|&(a, l)| snap_bytes(a, l)).collect();
        check_preserved(id, &inp, &o);
        let mut bad = String::new();
        if post_r != post_o {
            bad = format!("memory: original {post_o:x?}, Rust {post_r:x?}");
            for (&(a, l), b) in regions.iter().zip(&post_o) {
                if l > 0 {
                    std::ptr::copy_nonoverlapping(b.as_ptr(), a as *mut u8, l);
                }
            }
        }
        let win = match sent_equal(&theirs, &mine, skip3) {
            Ok(()) if bad.is_empty() => &mine,
            Ok(()) => &theirs,
            Err(e) => {
                bad = format!("{bad} {e}");
                &theirs
            }
        };
        if !bad.is_empty() {
            mismatch(id, n, format!("in {inp:x?} args {args:x?}: {bad}"));
        }
        match replay(win, live) {
            Some(last) if is_send => last,
            _ => v,
        }
    }
}

unsafe fn shadow_send(id: usize, p: &Pushad, args: [u32; 3], skip3: bool, ours: impl FnOnce() -> (u32, bool)) -> u32 {
    unsafe { shadow_send_ex(id, p, args, skip3, false, &[], ours) }
}

// --- the pool lookups 0x817D80 / 0x817F40 / 0x818020 ----------------------------------------------------------
// Each: bump the reader count under the pool's critical section (0x2A18944), walk the pool from
// its head entry (0x1044-byte entries, iterator 0x69B9D0: EAX = entry, EDI = 0x2A18938 -> next or
// 0) while the lock is NOT held, drop the reader count under the lock (when it reaches 0 and the
// waiter count is positive, SetEvent(0x2A18960)); EAX = the matching entry or 0. They have no
// register inputs; 0x817D80 takes the id byte on the stack (`ret`).

type Cs = extern "system" fn(usize);
type Ev = extern "system" fn(u32) -> i32;

unsafe fn pool_lock(slot: usize) {
    unsafe {
        let f: Cs = std::mem::transmute::<usize, Cs>(import(slot));
        f(POOL_CS);
    }
}

unsafe fn pool_find(mut pred: impl FnMut(u32) -> bool) -> u32 {
    unsafe {
        if rd(POOL_COUNT) == 0 {
            return 0;
        }
        let mut e = rd(POOL_HEAD).wrapping_mul(0x1044).wrapping_add(rd(rd(POOL_BASE) as usize));
        while e != 0 {
            if pred(e) {
                return e;
            }
            e = call_regs(POOL_ITER_NEXT, e, 0, 0, 0, POOL_BASE as u32, &[]);
        }
        0
    }
}

unsafe fn pool_lookup(pred: impl FnMut(u32) -> bool) -> u32 {
    unsafe {
        pool_lock(IAT_ENTER_CS);
        wd(POOL_USERS, rd(POOL_USERS).wrapping_add(1));
        pool_lock(IAT_LEAVE_CS);
        let found = pool_find(pred);
        pool_lock(IAT_ENTER_CS);
        let users = rd(POOL_USERS).wrapping_add(0xFFFF_FFFF);
        wd(POOL_USERS, users);
        if users == 0 && (rd(POOL_WAITERS) as i32) > 0 {
            let f: Ev = std::mem::transmute::<usize, Ev>(import(IAT_SET_EVENT));
            f(rd(POOL_EVENT));
        }
        pool_lock(IAT_LEAVE_CS);
        found
    }
}

/// Shadow: the original (locks and all) runs first; the Rust search (no locks) must find the same entry.
unsafe fn pool_shadow(id: usize, p: &Pushad, args: [u32; 3], pred: impl FnMut(u32) -> bool) -> u32 {
    unsafe {
        let Some((n, orig)) = shadowing(id) else { return pool_lookup(pred) };
        let inp = Regs::of(p);
        let mut o = inp;
        call_orig(orig, &mut o, args, false);
        check_preserved(id, &inp, &o);
        let ours = pool_find(pred);
        if ours != o.eax {
            mismatch(id, n, format!("args {args:x?}: original {:#x}, Rust {ours:#x}", o.eax));
        }
        o.eax
    }
}

unsafe fn h_pool_by_id(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let want = *a as u8;
        r.eax = pool_shadow(id, &p, [*a, 0, 0], |e| rb(e as usize + 4) == want);
    }
}
port_thunk!(pool_by_id_thunk, pool_by_id_entry, POOL_BY_ID, h_pool_by_id);

unsafe fn h_pool_state2(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = pool_shadow(id, &p, [0; 3], |e| rd(rd(e as usize + 0xC) as usize + 0x30) == 2);
    }
}
port_thunk!(pool_state2_thunk, pool_state2_entry, POOL_STATE2, h_pool_state2);

unsafe fn h_pool_state1(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = pool_shadow(id, &p, [0; 3], |e| rd(rd(e as usize + 0xC) as usize + 0x30) == 1);
    }
}
port_thunk!(pool_state1_thunk, pool_state1_entry, POOL_STATE1, h_pool_state1);

// --- the join-flow senders 0x816460 / 0x8165F0 / 0x816740 / 0x8168A0 ------------------------------------------------
// ECX = connection (-> ESI of the send), [esp+4] = object whose [+0xC] is the payload writer; `ret`.
// Header: type byte, u16 (size - 3), and a 4th (sequence) byte when the packet's channel byte
// (record + 0x14) is non-zero. The writer is virtual method `slot` of [obj + 0xC]:
// thiscall(ECX = writer, buffer, &size) with EAX = the method, EDX = buffer, ESI = connection and
// EDI the caller's. The send is 0x814670(buffer, size & 0xFFFF, 0, 0) with ESI = connection.

unsafe fn join_body(p: &Pushad, obj: u32, ty: u8, flag: usize, slot: usize) -> (u32, bool) {
    unsafe {
        let mut buf = [0u8; 0x51C];
        buf[0] = ty;
        let mut size: u32 = if rb(flag) == 0 { 3 } else { 4 };
        let w = rd(obj as usize + 0xC);
        let f = vslot(w, slot);
        let bp = buf.as_mut_ptr() as u32;
        call_regs(f, f as u32, w, bp, p.ecx, p.edi, &[bp, &mut size as *mut u32 as u32]);
        let sz = size & 0xFFFF;
        let len = (size.wrapping_sub(3) & 0xFFFF) as u16;
        buf[1..3].copy_from_slice(&len.to_le_bytes());
        (send_conn(p.ecx, &mut buf, sz, 0, 0), true)
    }
}

unsafe fn h_join(id: usize, r: &mut Pushad, a: *const u32, ty: u8, flag: usize, slot: usize) {
    unsafe {
        let p = *r;
        let obj = *a;
        r.eax = shadow_send(id, &p, [obj, 0, 0], rb(flag) != 0, || join_body(&p, obj, ty, flag, slot));
    }
}
unsafe fn h_join_new_session(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe { h_join(id, r, a, 5, 0x00E8_BF94, 8) }
}
unsafe fn h_join_success(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe { h_join(id, r, a, 6, 0x00E8_BFAC, 10) }
}
unsafe fn h_join_failed(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe { h_join(id, r, a, 7, 0x00E8_BFC4, 10) }
}
unsafe fn h_join_complete(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe { h_join(id, r, a, 8, 0x00E8_BFDC, 10) }
}
port_thunk!(join_new_session_thunk, join_new_session_entry, SEND_JOIN_NEW_SESSION, h_join_new_session);
port_thunk!(join_success_thunk, join_success_entry, SEND_JOIN_SUCCESS, h_join_success);
port_thunk!(join_failed_thunk, join_failed_entry, SEND_JOIN_FAILED, h_join_failed);
port_thunk!(join_complete_thunk, join_complete_entry, SEND_JOIN_COMPLETE, h_join_complete);

// --- the connection-state senders --------------------------------------------------------------------------------------
// Common frame of the originals: type byte at buf[0], u16 (size - 3) at buf[1], payload from the
// header size (3, or 4 when the packet's channel byte at record + 0x14 is non-zero); a local
// buffer of 0x51C bytes (the original's frame has the same room before its cookie).

/// Header size from the packet's channel byte: 3 or 4.
unsafe fn hdr_size(flag: usize) -> usize {
    unsafe { if rb(flag) == 0 { 3 } else { 4 } }
}

fn set_len(buf: &mut [u8], size: usize) {
    buf[1..3].copy_from_slice(&(size.wrapping_sub(3) as u16).to_le_bytes());
}

/// 0x7F8DF0 net_send_mp_info (ECX = connection): packet 0x5B {u8 [conn+0x44], u8 [conn+0x45]}
/// to the connection's session's remotes (EDI = [conn+0x1AC]). EAX = the send's.
unsafe fn h_send_mp_info(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = shadow_send(id, &p, [0; 3], rb(0x00E8_BA94) != 0, || {
            let mut buf = [0u8; 0x51C];
            buf[0] = 0x5B;
            let h = hdr_size(0x00E8_BA94);
            buf[h] = rb(p.ecx as usize + 0x44);
            buf[h + 1] = rb(p.ecx as usize + 0x45);
            let size = h + 2;
            set_len(&mut buf, size);
            (send_remotes(rd(p.ecx as usize + 0x1AC), &mut buf, size as u32), true)
        });
    }
}
port_thunk!(send_mp_info_thunk, send_mp_info_entry, SEND_MP_INFO, h_send_mp_info);

/// 0x7F8D70 net_send_upd_state_sub3 (ECX = connection): only for a connection whose byte [+0] is 0:
/// packet 0x20 {u8 3} to that connection. When it is not sent the original's EAX is the cookie
/// xor ESP (no meaning); the input EAX is returned then.
unsafe fn h_send_upd_state_sub3(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = shadow_send(id, &p, [0; 3], rb(0x00E8_C21C) != 0, || {
            if rb(p.ecx as usize) != 0 {
                return (p.eax, false);
            }
            let mut buf = [0u8; 0x51C];
            buf[0] = 0x20;
            let h = hdr_size(0x00E8_C21C);
            buf[h] = 3;
            let size = h + 1;
            set_len(&mut buf, size);
            (send_conn(p.ecx, &mut buf, size as u32, 0, 0), true)
        });
    }
}
port_thunk!(send_upd_state_sub3_thunk, send_upd_state_sub3_entry, SEND_UPD_STATE_SUB3, h_send_upd_state_sub3);

/// 0x7F8C50 net_conn_set_phase_broadcast (ECX = connection, EDX = phase): stores the phase at
/// [conn+0x40] when it changed; for a connection whose byte [+0] is non-zero sends packet 0x20
/// {u8 1, u32 phase} to the session's remotes (EDI = [conn+0x1AC]). Early exits leave the
/// cookie xor ESP in EAX (no meaning): the input EAX is returned then.
unsafe fn h_set_phase_broadcast(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let c = p.ecx as usize;
        r.eax = shadow_send_ex(id, &p, [0; 3], rb(0x00E8_C21C) != 0, false, &[(c + 0x40, 4)], || {
            if p.edx == rd(c + 0x40) {
                return (p.eax, false);
            }
            wd(c + 0x40, p.edx);
            if rb(c) == 0 {
                return (p.eax, false);
            }
            let mut buf = [0u8; 0x51C];
            buf[0] = 0x20;
            let h = hdr_size(0x00E8_C21C);
            buf[h] = 1;
            buf[h + 1..h + 5].copy_from_slice(&p.edx.to_le_bytes());
            let size = h + 5;
            set_len(&mut buf, size);
            (send_remotes(rd(c + 0x1AC), &mut buf, size as u32), true)
        });
    }
}
port_thunk!(set_phase_broadcast_thunk, set_phase_broadcast_entry, SET_PHASE_BROADCAST, h_set_phase_broadcast);

/// 0x816F60 net_send_join_deny (ECX passed through, [esp+4] = reason byte): packet 4 {u32 0xC9,
/// u8 reason} through 0x814790 (ECX = buffer, stack: the incoming ECX, size; cdecl). EAX = its.
unsafe fn h_send_join_deny(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let arg = *a;
        r.eax = shadow_send(id, &p, [arg, 0, 0], rb(0x00E8_BF7C) != 0, || {
            let mut buf = [0u8; 0x51C];
            buf[0] = 4;
            let h = hdr_size(0x00E8_BF7C);
            buf[h..h + 4].copy_from_slice(&0xC9u32.to_le_bytes());
            buf[h + 4] = arg as u8;
            let size = h + 5;
            set_len(&mut buf, size);
            (send_raw(&mut buf, p.ecx, size as u32), true)
        });
    }
}
port_thunk!(send_join_deny_thunk, send_join_deny_entry, SEND_JOIN_DENY, h_send_join_deny);

/// 0x816B50 net_send_upd_conn (EDI = connection, no stack arguments): packet 0x0B {u8 [conn+0x1B0],
/// then the connection description from 0x8157C0 (EAX = conn, ECX = buffer, stack: &size)} to
/// every other, ready (byte [+0x1B8] == 1), non-local connection of the session's circular list
/// (head [session+0x400], link +0x22CC). EAX at the end: the session pointer after a pass over
/// the list, else 0x8157C0's.
unsafe fn h_send_upd_conn(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = shadow_send(id, &p, [0; 3], rb(0x00E8_C024) != 0, || {
            let edi = p.edi as usize;
            let mut buf = [0u8; 0x51C];
            buf[0] = 0x0B;
            let h = hdr_size(0x00E8_C024);
            buf[h] = rb(edi + 0x1B0);
            let mut size: u32 = h as u32 + 1;
            let w = call_regs(0x0081_57C0, p.edi, buf.as_mut_ptr() as u32, 0, 0, p.edi, &[&mut size as *mut u32 as u32]);
            let mut esi = rd(rd(edi + 0x1AC) as usize + 0x400);
            set_len(&mut buf, size as usize);
            if esi == 0 {
                return (w, false);
            }
            loop {
                if rb(esi as usize + 0x1B8) == 1 && esi != p.edi && esi != rd(rd(esi as usize + 0x1AC) as usize + 0x408) {
                    send_conn(esi, &mut buf, size & 0xFFFF, 0, 0);
                }
                esi = rd(esi as usize + 0x22CC);
                if esi == rd(rd(edi + 0x1AC) as usize + 0x400) || esi == 0 {
                    break;
                }
            }
            (rd(edi + 0x1AC), false)
        });
    }
}
port_thunk!(send_upd_conn_thunk, send_upd_conn_entry, SEND_UPD_CONN, h_send_upd_conn);

/// 0x8148C0 net_send_packet_to_list_or_remotes (stack: buffer, size; `ret`): with no explicit
/// recipient list ([0xEB1FE4] == 0) it is 0x814740 on the session [0x2528C14]; otherwise 0x814670
/// to every entry of the list [0xEB1FDC] (count [0xEB1FE4]) except the session's local connection
/// [session+0x408]. The caller's buffer is used as it is (the send helper writes its 4th byte).
/// EAX: no list -> the send's; empty / negative list -> the count; else the list pointer.
unsafe fn h_send_to_list_or_remotes(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let (buf, size) = (*a, *a.add(1));
        r.eax = shadow_send_ex(id, &p, [buf, size, 0], false, true, &[], || {
            let count = rd(0x00EB_1FE4);
            let session = rd(0x0252_8C14);
            let local = rd(session as usize + 0x408);
            if count == 0 {
                return (send_remotes_raw(session, buf, size), true);
            }
            if (count as i32) <= 0 {
                return (count, false);
            }
            let mut i = 0usize;
            loop {
                let list = rd(0x00EB_1FDC) as usize;
                let e = rd(list + i * 4);
                if e != local {
                    send_conn_raw(e, buf, size, 0, 0);
                }
                i += 1;
                if (i as i32) >= count as i32 {
                    break;
                }
            }
            (rd(0x00EB_1FDC), false)
        });
    }
}
port_thunk!(send_to_list_or_remotes_thunk, send_to_list_or_remotes_entry, SEND_TO_LIST_OR_REMOTES, h_send_to_list_or_remotes);

// --- connection transport leaves: recent ids, ack window, timeouts, rate meter, network clock -----------------------
// Shadow: the original copy runs on the real state, the state it touched is put back, Rust runs,
// and the touched areas and EAX are compared (the original's end state is kept on a mismatch).
// Stack and registers: ECX / EDX are scratch in all of them (the thunk restores them, a superset).

unsafe fn rw(p: usize) -> u16 {
    unsafe { std::ptr::read_unaligned(p as *const u16) }
}
unsafe fn ww(p: usize, v: u16) {
    unsafe { std::ptr::write_unaligned(p as *mut u16, v) }
}

/// `fistp qword` with the rounding mode forced to truncate (control word | 0xC00): the low dword.
/// Out of range / NaN gives the integer indefinite 0x8000000000000000.
fn fist_trunc_lo(v: f64) -> u32 {
    if v.is_nan() || v >= 9.223372036854775808e18 || v < -9.223372036854775808e18 { 0 } else { (v.trunc() as i64) as u32 }
}

unsafe fn shadow_mem(id: usize, p: &Pushad, args: [u32; 3], regions: &[(usize, usize)], ours: impl FnOnce() -> u32) -> u32 {
    unsafe {
        let Some((n, orig)) = shadowing(id) else { return ours() };
        let snap = || regions.iter().map(|&(a, l)| snap_bytes(a, l)).collect::<Vec<_>>();
        let put = |s: &Vec<Vec<u8>>| {
            for (&(a, l), b) in regions.iter().zip(s) {
                if l > 0 {
                    std::ptr::copy_nonoverlapping(b.as_ptr(), a as *mut u8, l);
                }
            }
        };
        let pre = snap();
        let inp = Regs::of(p);
        let mut o = inp;
        call_orig(orig, &mut o, args, false);
        check_preserved(id, &inp, &o);
        let post_o = snap();
        put(&pre);
        let v = ours();
        let post_r = snap();
        if post_r != post_o || v != o.eax {
            mismatch(id, n, format!("in {inp:x?} args {args:x?}: original eax {:#x} mem {post_o:x?}, Rust eax {v:#x} mem {post_r:x?}", o.eax));
            put(&post_o);
            return o.eax;
        }
        v
    }
}

// --- 0x7FAA60 net_conn_recent_ids_push (EAX = conn, SI = id) --------------------------------------------------------
// Ring of recent u16 ids at +0x1CBA (count byte +0x1DBA, at most 0x80): drop an existing equal id
// (shifting the rest down), cap a full ring at 0x7F by dropping the oldest, append. Then a second
// list of up to 0x20 dword entries at +0x1C38 (count byte +0x1CB8) gets {id, 0}. EAX unchanged.

unsafe fn recent_ids_push(conn: usize, id: u16) {
    unsafe {
        let arr = conn + 0x1CBA;
        let cnt = conn + 0x1DBA;
        let mut found = false;
        if rb(cnt) != 0 {
            let mut dl: u8 = 0;
            loop {
                let i = dl as usize;
                let mut shift = found;
                if !found && rw(arr + i * 2) == id {
                    found = true;
                    shift = true;
                }
                if shift && ((i + 1) as i32) < rb(cnt) as i32 {
                    ww(arr + i * 2, rw(arr + i * 2 + 2));
                }
                dl = dl.wrapping_add(1);
                if dl >= rb(cnt) {
                    break;
                }
            }
            if found {
                wb(cnt, rb(cnt).wrapping_add(0xFF));
            }
        }
        if rb(cnt) == 0x80 {
            wb(cnt, 0x7F);
            let mut cl: u8 = 0;
            loop {
                let i = cl as usize;
                ww(arr + i * 2, rw(arr + i * 2 + 2));
                cl = cl.wrapping_add(1);
                if cl >= rb(cnt) {
                    break;
                }
            }
        }
        ww(arr + rb(cnt) as usize * 2, id);
        let c2 = rb(conn + 0x1CB8);
        wb(cnt, rb(cnt).wrapping_add(1));
        if c2 != 0x20 {
            ww(conn + 0x1C38 + c2 as usize * 4, id);
            wb(conn + 0x1C3A + rb(conn + 0x1CB8) as usize * 4, 0);
            wb(conn + 0x1CB8, rb(conn + 0x1CB8).wrapping_add(1));
        }
    }
}

unsafe fn h_recent_ids_push(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let c = p.eax as usize;
        r.eax = shadow_mem(id, &p, [0; 3], &[(c + 0x1C38, 0x184)], || {
            recent_ids_push(c, p.esi as u16);
            p.eax
        });
    }
}
port_thunk!(recent_ids_push_thunk, recent_ids_push_entry, RECENT_IDS_PUSH, h_recent_ids_push);

// --- 0x7F81D0 net_conn_ack_window_push (ECX = conn) --------------------------------------------------------------------
// Entries of 0x10 bytes from conn+0x1C0 (count dword +0x15C0, sequence word +0x15C4). When the
// connection's [+0x1DEC] is a time within 0x35A4E900 of the wall clock [0xE843AC] (either
// direction, signed), the count is bumped and the entry at index `count` gets {word seq, 0 ...};
// the sequence word then +1 (0xFFFF is never reached from 1, but the check is kept).
// EAX: the input (negative stamp), the stamp, stamp - clock, or the new sequence word.

unsafe fn h_ack_window_push_inner(p: &Pushad) -> u32 {
    unsafe {
        let c = p.ecx as usize;
        let v = rd(c + 0x1DEC);
        if (v as i32) < 0 {
            return p.eax;
        }
        let mut eax = v;
        let now = rd(0x00E8_43AC);
        if (eax as i32) > (now as i32) {
            eax = eax.wrapping_sub(now);
            if (eax as i32) < 0x35A4_E900 {
                return eax;
            }
        } else if (now.wrapping_sub(eax) as i32) >= 0x35A4_E900 {
            return eax;
        }
        wd(c + 0x15C0, rd(c + 0x15C0).wrapping_add(1));
        let e = c + (rd(c + 0x15C0).wrapping_add(0x1C) << 4) as usize;
        let seq = rw(c + 0x15C4);
        wd(e + 4, 0);
        ww(e, seq);
        wb(e + 8, 0);
        wb(e + 9, 0);
        wb(e + 0xA, 0);
        wd(e + 0xC, 0);
        ww(c + 0x15C4, rw(c + 0x15C4).wrapping_add(1));
        let n = rw(c + 0x15C4);
        if n == 0xFFFF {
            ww(c + 0x15C4, 0);
        }
        n as u32
    }
}

unsafe fn h_ack_window_push(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let c = p.ecx as usize;
        let ent = c + (rd(c + 0x15C0).wrapping_add(1).wrapping_add(0x1C) << 4) as usize;
        r.eax = shadow_mem(id, &p, [0; 3], &[(c + 0x15C0, 8), (ent, 0x10)], || h_ack_window_push_inner(&p));
    }
}
port_thunk!(ack_window_push_thunk, ack_window_push_entry, ACK_WINDOW_PUSH, h_ack_window_push);

// --- 0x7F8270 net_conn_ack_record (EAX = conn, SI = sequence word) ---------------------------------------------------
// For a connection (id byte +0x1B0 < 0xC): find the ack-window entry (from +0x1C0, 0x10 bytes,
// count +0x15C0) whose word equals the sequence; if its flag byte (+8) is clear: set it, and push
// (clock [0x252A3CC] - the entry's stamp (+4)) >> 1 into the ring of 10 dwords at +0x220C (index
// byte +0x2234, wraps at 10), bumping the total at +0x2238. EAX unchanged.

unsafe fn ack_record(p: &Pushad) {
    unsafe {
        let c = p.eax as usize;
        if c == 0 || rb(c + 0x1B0) >= 0xC {
            return;
        }
        let mut k = 0usize;
        if rd(c + 0x15C0) == 0 {
            return;
        }
        let mut e = c + 0x1C0;
        loop {
            if rw(e) == p.esi as u16 {
                break;
            }
            k += 1;
            e += 0x10;
            if k as u32 >= rd(c + 0x15C0) {
                return;
            }
        }
        if rb(e + 8) != 0 {
            return;
        }
        let d = rd(0x0252_A3CC).wrapping_sub(rd(e + 4));
        wb(e + 8, 1);
        let idx = rb(c + 0x2234) as usize;
        wd(c + 0x2238, rd(c + 0x2238).wrapping_add(1));
        wd(c + 0x220C + idx * 4, d >> 1);
        wb(c + 0x2234, rb(c + 0x2234).wrapping_add(1));
        if rb(c + 0x2234) >= 0xA {
            wb(c + 0x2234, 0);
        }
    }
}

unsafe fn h_ack_record(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let c = p.eax as usize;
        let mut regions = vec![];
        if c != 0 && rb(c + 0x1B0) < 0xC {
            let n = (rd(c + 0x15C0) as usize).min(0x140);
            regions = vec![(c + 0x1C0, n * 0x10), (c + 0x2234, 1), (c + 0x2238, 4), (c + 0x220C, 0x28)];
        }
        r.eax = shadow_mem(id, &p, [0; 3], &regions, || {
            ack_record(&p);
            p.eax
        });
    }
}
port_thunk!(ack_record_thunk, ack_record_entry, ACK_RECORD, h_ack_record);

// --- 0x7FA9E0 net_conn_check_timeout (EDI = conn -> AL) ----------------------------------------------------------------
// Always alive (1) on the hosting path ([0x2528B56] set). Otherwise: a connection whose ready byte
// (+0x1B8) is 0 is dead; one silent for more than 0xEA60 ms (0x3A98 when the session state
// [session+0x1C] is 0xA) has its ready byte cleared and is dead; one silent for more than 0x3A98 ms
// is dead without the clear. The silence is [0x252A3CC] - [conn+0x2240] minus 1 when the clock
// is below the stamp (a wrapped difference). The rest of EAX is untouched.

unsafe fn check_timeout(p: &Pushad) -> u32 {
    unsafe {
        if rb(0x0252_8B56) != 0 {
            return lo(p.eax, 1);
        }
        let c = p.edi as usize;
        if rb(c + 0x1B8) == 0 {
            return lo(p.eax, 0);
        }
        let limit = if rd(rd(c + 0x1AC) as usize + 0x1C) == 0xA { 0x3A98u32 } else { 0xEA60 };
        let now = rd(0x0252_A3CC);
        let stamp = rd(c + 0x2240);
        let mut d = now.wrapping_sub(stamp);
        if stamp > now {
            d = d.wrapping_sub(1);
        }
        if d > limit {
            wb(c + 0x1B8, 0);
            return lo(p.eax, 0);
        }
        lo(p.eax, (d <= 0x3A98) as u32)
    }
}

unsafe fn h_check_timeout(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = shadow_mem(id, &p, [0; 3], &[(p.edi as usize + 0x1B8, 1)], || check_timeout(&p));
    }
}
port_thunk!(check_timeout_thunk, check_timeout_entry, CHECK_TIMEOUT, h_check_timeout);

// --- 0x7FC8B0 net_timeout_expired (-> AL) -----------------------------------------------------------------------------------
// Armed ([0x2528B89]) with a non-negative deadline [0xEB1614]: expired when the wall clock
// [0xE843AC] is at or past it (difference under 0x35A4E900, signed), or the deadline is more than
// that ahead (the clock wrapped). EAX: the deadline (or the difference) with AL replaced.

unsafe fn h_timeout_expired(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let ours = if rb(0x0252_8B89) == 0 {
            lo(p.eax, 0)
        } else {
            let mut eax = rd(0x00EB_1614);
            if (eax as i32) < 0 {
                lo(eax, 0)
            } else {
                let ecx = rd(0x00E8_43AC);
                if (eax as i32) > (ecx as i32) {
                    eax = eax.wrapping_sub(ecx);
                    lo(eax, ((eax as i32) >= 0x35A4_E900) as u32)
                } else {
                    lo(eax, ((ecx.wrapping_sub(eax) as i32) < 0x35A4_E900) as u32)
                }
            }
        };
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(timeout_expired_thunk, timeout_expired_entry, TIMEOUT_EXPIRED, h_timeout_expired);

// --- 0x7FC8F0 net_timeout_arm_10s: deadline = clock + 10000 (wrapped at 0x6B49D200), armed; then a tail jump
// to 0x80B8D0 (which stores another wrapped deadline in [0xEB1FA0]); EAX = its.

unsafe fn h_timeout_arm_10s(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = shadow_mem(id, &p, [0; 3], &[(0x0252_8B89, 1), (0x00EB_1614, 4), (0x00EB_1FA0, 4)], || {
            let mut eax = rd(0x00E8_43AC).wrapping_add(0x2710);
            wb(0x0252_8B89, 1);
            wd(0x00EB_1614, eax);
            if (eax as i32) > 0x6B49_D200 {
                eax = eax.wrapping_sub(0x6B49_D200u32);
                wd(0x00EB_1614, eax);
            }
            call_regs(0x0080_B8D0, eax, p.ecx, p.edx, p.esi, p.edi, &[])
        });
    }
}
port_thunk!(timeout_arm_10s_thunk, timeout_arm_10s_entry, TIMEOUT_ARM_10S, h_timeout_arm_10s);

// --- 0x7FB1C0 net_rate_meter_init (EDX = byte window, ECX = capacity; stack: meter, word; `ret 8`) -------------------
// +0x00 capacity (<= 0xE000), +0x04..+0x78 the 30 slots (cleared), +0x7C head (0), +0x7D window
// (<= 0x1E), +0x7E the word. EAX = meter.

unsafe fn h_rate_meter_init(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let (m, w) = (*a, *a.add(1));
        r.eax = shadow_mem(id, &p, [m, w, 0], &[(m as usize, 0x80)], || {
            let dl = p.edx as u8;
            let win = if dl < 0x1E { dl as u32 } else { 0x1E };
            wb(m as usize + 0x7D, win as u8);
            ww(m as usize + 0x7E, w as u16);
            let cap = if p.ecx > 0xE000 { 0xE000 } else { p.ecx };
            wd(m as usize, cap);
            for i in 1..=0x1E {
                wd(m as usize + i * 4, 0);
            }
            wb(m as usize + 0x7C, 0);
            m
        });
    }
}
port_thunk!(rate_meter_init_thunk, rate_meter_init_entry, RATE_METER_INIT, h_rate_meter_init, "ret 8");

// --- 0x7FB2B0 net_rate_meter_remaining (EDX = meter -> EAX) ------------------------------------------------------------
// capacity - (the sum of the last `window` slots, walking back from the head with wrap at 0x1D), or 0
// when the sum exceeds the capacity.

unsafe fn h_rate_meter_remaining(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let m = p.edx as usize;
        let mut head = rb(m + 0x7C) as u32;
        let mut n = rb(m + 0x7D) as u32;
        let mut sum = 0u32;
        while n > 0 {
            sum = sum.wrapping_add(rd(m + head as usize * 4 + 4));
            head = if head == 0 { 0x1D } else { head - 1 };
            n -= 1;
        }
        let cap = rd(m);
        let ours = if sum > cap { 0 } else { cap - sum };
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(rate_meter_remaining_thunk, rate_meter_remaining_entry, RATE_METER_REMAINING, h_rate_meter_remaining);

// --- 0x7FB100 net_conn_note_received (EAX = conn, DX = id) --------------------------------------------------------------
// Pushes the id (unless 0xFFFF) on the list of up to 100 words at +0x15C6 (count word +0x168E) for a
// ready connection, and stamps +0x2240 with the clock [0x252A3CC]. EAX unchanged.

unsafe fn h_conn_note_received(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let c = p.eax as usize;
        let regions = if c != 0 { vec![(c + 0x168E, 2), (c + 0x15C6, 0xC8), (c + 0x2240, 4)] } else { vec![] };
        r.eax = shadow_mem(id, &p, [0; 3], &regions, || {
            if c != 0 {
                if rb(c + 0x1B8) != 0 && p.edx as u16 != 0xFFFF {
                    let n = rw(c + 0x168E);
                    if n < 0x64 {
                        ww(c + n as usize * 2 + 0x15C6, p.edx as u16);
                        ww(c + 0x168E, rw(c + 0x168E).wrapping_add(1));
                    }
                }
                wd(c + 0x2240, rd(0x0252_A3CC));
            }
            p.eax
        });
    }
}
port_thunk!(conn_note_received_thunk, conn_note_received_entry, CONN_NOTE_RECEIVED, h_conn_note_received);

// --- 0x7FB190 net_conn_ack_window_mark_all (EDX = conn) ------------------------------------------------------------------
// Sets byte +9 of every ack-window entry. EAX = the count (0 when empty); unchanged for no connection.

unsafe fn h_ack_window_mark_all(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let c = p.edx as usize;
        let n = if c != 0 { rd(c + 0x15C0) } else { 0 };
        let regions = if c != 0 { vec![(c + 0x1C0, (n as usize).min(0x140) * 0x10)] } else { vec![] };
        r.eax = shadow_mem(id, &p, [0; 3], &regions, || {
            if c == 0 {
                return p.eax;
            }
            let mut i = 0u32;
            while i < rd(c + 0x15C0) {
                wb(c + 0x1C9 + i as usize * 0x10, 1);
                i += 1;
            }
            i
        });
    }
}
port_thunk!(ack_window_mark_all_thunk, ack_window_mark_all_entry, ACK_WINDOW_MARK_ALL, h_ack_window_mark_all);

// --- 0x7F8160 net_conn_reset_ack_window (EAX = conn) --------------------------------------------------------------------
// Clears the count, the received-list count and +0x1690, sets the sequence word to 1 and clears the first
// entry. EAX unchanged.

unsafe fn h_reset_ack_window(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let c = p.eax as usize;
        let regions = if c != 0 { vec![(c + 0x15C0, 8), (c + 0x168E, 4), (c + 0x1C0, 0x10)] } else { vec![] };
        r.eax = shadow_mem(id, &p, [0; 3], &regions, || {
            if c != 0 {
                wd(c + 0x15C0, 0);
                ww(c + 0x168E, 0);
                ww(c + 0x1690, 0);
                ww(c + 0x15C4, 0);
                wd(c + 0x1C4, 0);
                ww(c + 0x1C0, 0);
                wb(c + 0x1C8, 0);
                wb(c + 0x1C9, 0);
                wb(c + 0x1CA, 0);
                wd(c + 0x1CC, 0);
                ww(c + 0x15C4, 1);
            }
            p.eax
        });
    }
}
port_thunk!(reset_ack_window_thunk, reset_ack_window_entry, RESET_ACK_WINDOW, h_reset_ack_window);

// --- 0x7FCAB0 net_clock_predicted_ms (-> EAX) ----------------------------------------------------------------------------
// With the hosting path ([0x2528B56] set and [0x2528B2B] clear) or an unset session byte (+0x3F8): the
// session clock ([session+8]) plus 1000 * the local connection's [+0x1DE8] (0 -> just the clock).
// Otherwise the clock minus trunc(f32[0x213002C] * f64[0xE5C200]) plus 100.

unsafe fn h_clock_predicted_ms(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let s = rd(0x0252_8C14) as usize;
        let local_path = if rb(0x0252_8B56) != 0 && rb(0x0252_8B2B) == 0 { true } else { rb(s + 0x3F8) == 0 };
        let ours = if !local_path {
            let t = fist_trunc_lo(rf(0x0213_002C) as f64 * f64::from_bits(((rd(0x00E5_C204) as u64) << 32) | rd(0x00E5_C200) as u64));
            rd(s + 8).wrapping_sub(t).wrapping_add(0x64)
        } else {
            let n = rd(rd(s + 0x408) as usize + 0x1DE8);
            if n == 0 { rd(s + 8) } else { n.wrapping_mul(0x3E8).wrapping_add(rd(s + 8)) }
        };
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(clock_predicted_ms_thunk, clock_predicted_ms_entry, CLOCK_PREDICTED_MS, h_clock_predicted_ms);

// --- 0x7FC930 net_clock_advance -----------------------------------------------------------------------------------------------
// Unless the session is the hosting kind ([0x2528B56] set, a session, [0x2528B2B] clear or +0x3F8 clear
// -> straight out with EAX = session): add the frame delta [0x212B15C] to field +8 of every pooled
// entry (same reader-count / lock pattern as the pool lookups), add it to the offline clock
// [0x212B160], then recompute the delta as trunc(f32[0xE8437C or 0xE84380] * f64[0xE5C0D8]) clamped
// to 0xFA (unsigned). EAX = the unclamped value.

unsafe fn clock_advance(p: &Pushad) -> u32 {
    unsafe {
        let _ = p;
        if rb(0x0252_8B56) != 0 {
            let s = rd(0x0252_8C14);
            if s != 0 && (rb(0x0252_8B2B) == 0 || rb(s as usize + 0x3F8) == 0) {
                return s;
            }
        }
        let delta = rd(0x0212_B15C);
        pool_lookup(|e| {
            wd(e as usize + 8, rd(e as usize + 8).wrapping_add(delta));
            false
        });
        wd(0x0212_B160, rd(0x0212_B160).wrapping_add(rd(0x0212_B15C)));
        let f = if rb(0x0252_8B56) != 0 { rf(0x00E8_437C) } else { rf(0x00E8_4380) };
        let c = f64::from_bits(((rd(0x00E5_C0DC) as u64) << 32) | rd(0x00E5_C0D8) as u64);
        let eax = fist_trunc_lo(f as f64 * c);
        wd(0x0212_B15C, eax);
        if eax > 0xFA {
            wd(0x0212_B15C, 0xFA);
        }
        eax
    }
}

unsafe fn h_clock_advance(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let mut regions = vec![(0x0212_B15Cusize, 4usize), (0x0212_B160, 4)];
        if shadowing_active(id) {
            pool_find(|e| {
                regions.push((e as usize + 8, 4));
                false
            });
        }
        r.eax = shadow_mem(id, &p, [0; 3], &regions, || clock_advance(&p));
    }
}
port_thunk!(clock_advance_thunk, clock_advance_entry, CLOCK_ADVANCE, h_clock_advance);

// --- session predicates and list walkers --------------------------------------------------------------------------------
// Circular connection list: head [session+0x400], link [conn+0x22CC]; the walks stop on the head or 0.

/// 0x80FF70 net_session_out_of_phase_mask (ECX = session -> EAX): bit [conn+0x1B0] for every
/// connection whose phase [+0x40] differs from the designated peer's ([session+0x404]) phase
/// (the peer itself and the empty list give 0).
unsafe fn h_out_of_phase_mask(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let s = p.ecx as usize;
        let head = rd(s + 0x400);
        let mut mask = 0u32;
        if head != 0 {
            let peer = rd(s + 0x404);
            let mut n = head;
            loop {
                if n != peer && rd(peer as usize + 0x40) != rd(n as usize + 0x40) {
                    mask |= 1u32.wrapping_shl(rb(n as usize + 0x1B0) as u32);
                }
                n = rd(n as usize + 0x22CC);
                if n == head || n == 0 {
                    break;
                }
            }
        }
        r.eax = checked_eax(id, &p, [0; 3], mask);
    }
}
port_thunk!(out_of_phase_mask_thunk, out_of_phase_mask_entry, OUT_OF_PHASE_MASK, h_out_of_phase_mask);

/// 0x80FF00 session_peers_phase_equal (EDX = session -> AL): true when this is not the hosting
/// path ([0x2528B56] set, [0x2528B2B] clear) or the session byte +0x3F8 is clear; false when
/// 0x8CD8D0 (EAX = session) says so; else whether every connection but the peer shares the peer's phase.
/// The upper EAX bytes are the original's: the node it stopped on, or 0x8CD8D0's.
unsafe fn peers_phase_equal(p: &Pushad) -> u32 {
    unsafe {
        let s = p.edx as usize;
        if (rb(0x0252_8B56) != 0 && rb(0x0252_8B2B) == 0) || rb(s + 0x3F8) == 0 {
            return lo(p.eax, 1);
        }
        let r = call_regs(0x008C_D8D0, p.edx, 0, 0, 0, 0, &[]);
        if r as u8 != 0 {
            return lo(r, 0);
        }
        let head = rd(s + 0x400);
        let mut n = head;
        if n == 0 {
            return lo(n, 1);
        }
        let peer = rd(s + 0x404);
        loop {
            if n != peer && rd(peer as usize + 0x40) != rd(n as usize + 0x40) {
                return lo(n, 0);
            }
            n = rd(n as usize + 0x22CC);
            if n == head || n == 0 {
                return lo(n, 1);
            }
        }
    }
}

unsafe fn h_peers_phase_equal(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], peers_phase_equal(&p));
    }
}
port_thunk!(peers_phase_equal_thunk, peers_phase_equal_entry, PEERS_PHASE_EQUAL, h_peers_phase_equal);

/// 0x810040 session_all_conns_state_eq / 0x8100F0 session_all_conns_state_ge (ESI = session,
/// EDI = state -> AL): needs 0x80FF00 (EDX = session) to say yes, then every connection's state
/// [+0x3C] must equal / be >= (signed) EDI. The upper EAX bytes: 0x80FF00's, or the node that failed / ended the walk.
unsafe fn all_conns_state(p: &Pushad, ge: bool) -> u32 {
    unsafe {
        let s = p.esi as usize;
        let r = call_regs(0x0080_FF00, p.eax, p.ecx, p.esi, p.esi, p.edi, &[]);
        if r as u8 == 0 {
            return lo(r, 0);
        }
        let head = rd(s + 0x400);
        let mut n = head;
        if n == 0 {
            return lo(0, 1);
        }
        loop {
            let st = rd(n as usize + 0x3C);
            let bad = if ge { (st as i32) < (p.edi as i32) } else { st != p.edi };
            if bad {
                return lo(n, 0);
            }
            n = rd(n as usize + 0x22CC);
            if n == head || n == 0 {
                return lo(n, 1);
            }
        }
    }
}

unsafe fn h_all_conns_state_eq(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], all_conns_state(&p, false));
    }
}
port_thunk!(all_conns_state_eq_thunk, all_conns_state_eq_entry, ALL_CONNS_STATE_EQ, h_all_conns_state_eq);

unsafe fn h_all_conns_state_ge(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], all_conns_state(&p, true));
    }
}
port_thunk!(all_conns_state_ge_thunk, all_conns_state_ge_entry, ALL_CONNS_STATE_GE, h_all_conns_state_ge);

/// 0x810080 lobby_any_conn_state_4 (-> AL): some connection of the list of the session at [0x2528CE8] is in state 4.
unsafe fn h_lobby_any_conn_state_4(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let head = rd(rd(0x0252_8CE8) as usize + 0x400);
        let mut n = head;
        let ours = if n == 0 {
            lo(n, 0)
        } else {
            loop {
                if rd(n as usize + 0x3C) == 4 {
                    break lo(n, 1);
                }
                n = rd(n as usize + 0x22CC);
                if n == head || n == 0 {
                    break lo(n, 0);
                }
            }
        };
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(lobby_any_conn_state_4_thunk, lobby_any_conn_state_4_entry, LOBBY_ANY_CONN_STATE_4, h_lobby_any_conn_state_4);

/// 0x80CEB0 session_state_in_set_b (EAX = session -> AL): state ([+0x1C]) in {2, 4, 7, 9, 10, 11, 12}.
/// EAX keeps state - 2 above the low byte.
unsafe fn h_state_in_set_b(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let e = rd(p.eax as usize + 0x1C).wrapping_sub(2);
        let yes = e <= 0xA && [0u8, 1, 0, 1, 1, 0, 1, 0, 0, 0, 0][e as usize] == 0;
        r.eax = checked_eax(id, &p, [0; 3], lo(e, yes as u32));
    }
}
port_thunk!(state_in_set_b_thunk, state_in_set_b_entry, STATE_IN_SET_B, h_state_in_set_b);

/// 0x80CF10 session_state_is_8 (-> EAX 0 / 1): the state of the session at [0x2528CB4] is 8.
unsafe fn h_state_is_8(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let ours = (rd(rd(0x0252_8CB4) as usize + 0x1C) == 8) as u32;
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(state_is_8_thunk, state_is_8_entry, STATE_IS_8, h_state_is_8);

/// 0x7F7A00 net_local_conn_id (-> AL): the id byte [+0x1B0] of the session's local connection ([session+0x408]), 0 without a session.
/// EAX keeps the connection pointer above AL.
unsafe fn h_local_conn_id(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let s = rd(0x0252_8C14);
        let ours = if s == 0 {
            0
        } else {
            let c = rd(s as usize + 0x408);
            lo(c, rb(c as usize + 0x1B0) as u32)
        };
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(local_conn_id_thunk, local_conn_id_entry, LOCAL_CONN_ID, h_local_conn_id);

/// 0x7F8020 net_id_to_human (AX = net id -> EAX): the object a net id maps to (map 0x212E258, find
/// 0x808E30 (ESI = map; id, 0)), through the handle slot table, when its flag byte +0x3CC is exactly 1
/// (the original builds an all-ones / zero mask with `neg` / `sbb` / `not`); else 0.
unsafe fn id_to_human(p: &Pushad) -> u32 {
    unsafe {
        let key = p.eax & 0xFFFF;
        if p.eax as u16 == 0xFFFF {
            return 0;
        }
        let node = call_regs(0x0080_8E30, key, p.ecx, p.edx, rd(NET_ID_MAP), p.edi, &[key, 0]);
        if node == 0 {
            return 0;
        }
        let rec = rd(node as usize);
        if rec == 0 {
            return 0;
        }
        let handle = rd(rec as usize + 4);
        if handle == 0 {
            return 0;
        }
        let o = rd(HANDLES + ((handle & 0xFFF) as usize) * 16);
        if o == 0 || rd(o as usize + 0x44) != handle || rd(o as usize + 0x4C) != 1 {
            return 0;
        }
        if rb(o as usize + 0x3CC).wrapping_sub(1) != 0 { 0 } else { o }
    }
}

unsafe fn h_id_to_human(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], id_to_human(&p));
    }
}
port_thunk!(id_to_human_thunk, id_to_human_entry, ID_TO_HUMAN, h_id_to_human);

/// 0x807A20 net_remote_handle_map_set (EAX = key, [esp+4] = value, `ret 4`; ESI saved): find-or-create
/// (0x808E30 with create = 1) the node of the key in map 0x212E258 and store the value in its first
/// dword. AL = 1 (EAX = the node pointer above it), or 0 (EAX = 0) without a node.
/// Shadow: Rust first (it creates the node), then the original copy finds the same node and stores the same value.
unsafe fn h_map_set(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let v = *a;
        let node = call_regs(0x0080_8E30, p.eax, p.ecx, p.edx, rd(NET_ID_MAP), p.edi, &[p.eax, 1]);
        let ours = if node == 0 {
            0
        } else {
            wd(node as usize, v);
            lo(node, 1)
        };
        let Some((n, orig)) = shadowing(id) else {
            r.eax = ours;
            return;
        };
        let inp = Regs::of(&p);
        let mut o = inp;
        call_orig(orig, &mut o, [v, 0, 0], false);
        check_preserved(id, &inp, &o);
        if o.eax != ours {
            mismatch(id, n, format!("key {:#x} value {v:#x}: original {:#x}, Rust {ours:#x}", p.eax, o.eax));
            r.eax = o.eax;
            return;
        }
        r.eax = ours;
    }
}
port_thunk!(map_set_thunk, map_set_entry, MAP_SET, h_map_set, "ret 4");

/// 0x80F2E0 session_find_reservation (EAX = session, stack: guid*, out index*; `ret 8`): the last-to-first
/// scan of the session's reservations (count +0x3F4, 0x50-byte entries from +0x5C, GUID at +0 compared with
/// 0x8176E0 (ECX = wanted, EDX = entry), id dword at +0x24 compared with the wanted one's). Found: store
/// the index through the out pointer (when non-null), AL = 1; else AL = 0. The upper EAX bytes are 0x8176E0's.
unsafe fn find_reservation(p: &Pushad, guid: u32, out: u32) -> u32 {
    unsafe {
        let s = p.eax as usize;
        let mut i = (rd(s + 0x3F4) as i32).wrapping_sub(1);
        if i < 0 {
            return lo(p.eax, 0);
        }
        let mut e = (s + 0x5C + (i as usize).wrapping_mul(0x50)) as u32;
        let mut r;
        loop {
            r = call_regs(0x0081_76E0, p.eax, guid, e, 0, 0, &[]);
            if r as u8 != 0 && rd(e as usize + 0x24) == rd(guid as usize + 0x24) {
                if out != 0 {
                    wd(out as usize, i as u32);
                }
                return lo(r, 1);
            }
            i -= 1;
            e = e.wrapping_sub(0x50);
            if i < 0 {
                return lo(r, 0);
            }
        }
    }
}

unsafe fn h_find_reservation(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let (guid, out) = (*a, *a.add(1));
        let regions = if out != 0 { vec![(out as usize, 4usize)] } else { vec![] };
        r.eax = shadow_mem(id, &p, [guid, out, 0], &regions, || find_reservation(&p, guid, out));
    }
}
port_thunk!(find_reservation_thunk, find_reservation_entry, FIND_RESERVATION, h_find_reservation, "ret 8");

/// 0x80F340 session_remove_reservation (EAX = session, EDX = index): count -= 1 and, unless the removed index
/// is the new count, the last entry's two 0x28-byte halves (+0x34 and +0x5C of the 0x50-byte entry slot)
/// overwrite the removed one's. EAX = the session, or the removed slot's +0x5C half.
unsafe fn remove_reservation(p: &Pushad) -> u32 {
    unsafe {
        let s = p.eax as usize;
        wd(s + 0x3F4, rd(s + 0x3F4).wrapping_add(0xFFFF_FFFF));
        let last = rd(s + 0x3F4);
        if last == p.edx {
            return p.eax;
        }
        let l = (last as usize).wrapping_mul(0x50);
        let d = (p.edx as usize).wrapping_mul(0x50).wrapping_add(s);
        let tmp1 = snap_bytes(l.wrapping_add(s).wrapping_add(0x34), 0x28);
        std::ptr::copy_nonoverlapping(tmp1.as_ptr(), d.wrapping_add(0x34) as *mut u8, 0x28);
        let tmp2 = snap_bytes(l.wrapping_add(s).wrapping_add(0x5C), 0x28);
        std::ptr::copy_nonoverlapping(tmp2.as_ptr(), d.wrapping_add(0x5C) as *mut u8, 0x28);
        d.wrapping_add(0x5C) as u32
    }
}

unsafe fn h_remove_reservation(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let s = p.eax as usize;
        let d = (p.edx as usize).wrapping_mul(0x50).wrapping_add(s);
        r.eax = shadow_mem(id, &p, [0; 3], &[(s + 0x3F4, 4), (d.wrapping_add(0x34), 0x50)], || remove_reservation(&p));
    }
}
port_thunk!(remove_reservation_thunk, remove_reservation_entry, REMOVE_RESERVATION, h_remove_reservation);

/// 0x80C350 session_guid_in_array (EAX = array of 0x28-byte GUID entries; stack: count, base; cdecl): AL = 1 when some
/// entry matches the GUID at base + 0x14C (0x8176E0: ECX = entry, EDX = GUID) with an equal id dword (+0x24).
/// EAX: the id dword with AL = 1 when found; else the last value EAX held with AL = 0.
unsafe fn guid_in_array(p: &Pushad, count: u32, base: u32) -> u32 {
    unsafe {
        let g = base.wrapping_add(0x14C);
        let mut eax = p.eax;
        let mut e = p.eax;
        let mut i = 0i32;
        if (count as i32) > 0 {
            loop {
                eax = call_regs(0x0081_76E0, eax, e, g, 0, 0, &[]);
                if eax as u8 != 0 {
                    eax = rd(g as usize + 0x24);
                    if eax == rd(e as usize + 0x24) {
                        return lo(eax, 1);
                    }
                }
                i += 1;
                e = e.wrapping_add(0x28);
                if i >= count as i32 {
                    break;
                }
            }
        }
        lo(eax, 0)
    }
}

unsafe fn h_guid_in_array(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let (count, base) = (*a, *a.add(1));
        r.eax = checked_eax(id, &p, [count, base, 0], guid_in_array(&p, count, base));
    }
}
port_thunk!(guid_in_array_thunk, guid_in_array_entry, GUID_IN_ARRAY, h_guid_in_array);

/// 0x80C4E0 session_find_best_conn (EDI = list head, [esp+4] = connection to skip -> EAX): among the ready (byte +0x1B8),
/// not-flagged (byte +0x1DCC clear) connections but the skipped one, the one with the largest dword +0x1DBC
/// (the first wins ties; any beats nothing); 0 for none.
unsafe fn h_find_best_conn(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let skip = *a;
        let (mut best, mut score) = (0u32, 0u32);
        let head = p.edi;
        if head != 0 {
            let mut c = head;
            loop {
                if c != skip && rb(c as usize + 0x1DCC) == 0 && rb(c as usize + 0x1B8) != 0 {
                    let v = rd(c as usize + 0x1DBC);
                    if v > score || best == 0 {
                        best = c;
                        score = v;
                    }
                }
                c = rd(c as usize + 0x22CC);
                if c == head || c == 0 {
                    break;
                }
            }
        }
        r.eax = checked_eax(id, &p, [skip, 0, 0], best);
    }
}
port_thunk!(find_best_conn_thunk, find_best_conn_entry, FIND_BEST_CONN, h_find_best_conn);

/// 0x80C560 session_info_base_init (EAX = object): vtable 0xDFFD54 and the base fields cleared / set.
unsafe fn h_session_info_base_init(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        r.eax = shadow_mem(id, &p, [0; 3], &[(o, 0x4A)], || {
            wd(o, 0x00DF_FD54);
            wd(o + 0x30, 0);
            wb(o + 0x34, 0);
            wd(o + 0x38, 0);
            wd(o + 0x3C, 1);
            for off in [0x41, 0x42, 0x43, 0x44, 0x46, 0x47, 0x48, 0x49] {
                wb(o + off, 0);
            }
            p.eax
        });
    }
}
port_thunk!(session_info_base_init_thunk, session_info_base_init_entry, SESSION_INFO_BASE_INIT, h_session_info_base_init);

// --- serialiser helper-object constructors (EAX = object) ----------------------------------------------------------------
// Stores only. Floats pass through `fld dword` / `fstp dword` (a signalling NaN is quieted: q32).
// Several take many stack arguments, so their shadow calls the original copy with `call_regs`.

/// `shadow_mem` for originals with many stack arguments (`call_regs` restores ESP after a `ret N`).
unsafe fn shadow_mem_regs(id: usize, p: &Pushad, args: &[u32], regions: &[(usize, usize)], ours: impl FnOnce() -> u32) -> u32 {
    unsafe {
        let Some((n, orig)) = shadowing(id) else { return ours() };
        let snap = || regions.iter().map(|&(a, l)| snap_bytes(a, l)).collect::<Vec<_>>();
        let put = |s: &Vec<Vec<u8>>| {
            for (&(a, l), b) in regions.iter().zip(s) {
                if l > 0 {
                    std::ptr::copy_nonoverlapping(b.as_ptr(), a as *mut u8, l);
                }
            }
        };
        let pre = snap();
        let eax = call_regs(orig, p.eax, p.ecx, p.edx, p.esi, p.edi, args);
        let post_o = snap();
        put(&pre);
        let v = ours();
        let post_r = snap();
        if post_r != post_o || v != eax {
            mismatch(id, n, format!("in {:x?} args {args:x?}: original eax {eax:#x} mem {post_o:x?}, Rust eax {v:#x} mem {post_r:x?}", Regs::of(p)));
            put(&post_o);
            return eax;
        }
        v
    }
}

unsafe fn args_of(a: *const u32, n: usize) -> Vec<u32> {
    (0..n).map(|i| unsafe { *a.add(i) }).collect()
}

/// 0x7FDBB0 / 0x7FDBC0 byte noncompressor init (EAX = object [, ECX = bit count]): vtable 0xE264C4, byte count (0 / ECX >> 3) at +4.
unsafe fn h_byte_nc_init(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        r.eax = shadow_mem(id, &p, [0; 3], &[(o, 8)], || {
            wd(o, 0x00E2_64C4);
            wd(o + 4, 0);
            p.eax
        });
    }
}
unsafe fn h_byte_nc_init_bits(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        r.eax = shadow_mem(id, &p, [0; 3], &[(o, 8)], || {
            wd(o, 0x00E2_64C4);
            wd(o + 4, p.ecx >> 3);
            p.eax
        });
    }
}
port_thunk!(byte_nc_init_thunk, byte_nc_init_entry, BYTE_NC_INIT, h_byte_nc_init);
port_thunk!(byte_nc_init_bits_thunk, byte_nc_init_bits_entry, BYTE_NC_INIT_BITS, h_byte_nc_init_bits);

/// 0x7FDC90 / 0x7FDCB0 bit noncompressor init (EAX = object [, ECX = bit count]): vtable 0xE264D8, {bits, bytes, rest} at +4 / +8 / +0xC.
unsafe fn h_bit_nc_init(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        r.eax = shadow_mem(id, &p, [0; 3], &[(o, 0x10)], || {
            wd(o, 0x00E2_64D8);
            wd(o + 4, 0);
            wd(o + 8, 0);
            wd(o + 0xC, 0);
            p.eax
        });
    }
}
unsafe fn h_bit_nc_init_bits(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        r.eax = shadow_mem(id, &p, [0; 3], &[(o, 0x10)], || {
            wd(o + 4, p.ecx);
            wd(o, 0x00E2_64D8);
            wd(o + 8, p.ecx >> 3);
            wd(o + 0xC, p.ecx & 7);
            p.eax
        });
    }
}
port_thunk!(bit_nc_init_thunk, bit_nc_init_entry, BIT_NC_INIT, h_bit_nc_init);
port_thunk!(bit_nc_init_bits_thunk, bit_nc_init_bits_entry, BIT_NC_INIT_BITS, h_bit_nc_init_bits);

/// 0x7FDE30 net_float_compressor_init (EAX = object, ECX = +0; stack: 5 floats, a byte; `ret 0x18`).
unsafe fn h_float_comp_init(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        let v = args_of(a, 6);
        r.eax = shadow_mem_regs(id, &p, &v, &[(o, 0x1C)], || {
            wd(o + 4, q32(v[0]));
            wd(o, p.ecx);
            wb(o + 0x18, v[5] as u8);
            wd(o + 8, q32(v[1]));
            wd(o + 0xC, q32(v[2]));
            wd(o + 0x10, q32(v[3]));
            wd(o + 0x14, q32(v[4]));
            p.eax
        });
    }
}
port_thunk!(float_comp_init_thunk, float_comp_init_entry, FLOAT_COMP_INIT, h_float_comp_init, "ret 24");

/// 0x7FDE60 net_float_compressor_init_copy (EAX = object; stack: 7 dwords copied to +4; vtable 0xE264EC; `ret 0x1C`).
unsafe fn h_float_comp_init_copy(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        let v = args_of(a, 7);
        r.eax = shadow_mem_regs(id, &p, &v, &[(o, 0x20)], || {
            for (i, x) in v.iter().enumerate() {
                wd(o + 4 + i * 4, *x);
            }
            wd(o, 0x00E2_64EC);
            p.eax
        });
    }
}
port_thunk!(float_comp_init_copy_thunk, float_comp_init_copy_entry, FLOAT_COMP_INIT_COPY, h_float_comp_init_copy, "ret 28");

/// 0x7FE110 net_float_compressor_copy (EAX = destination, ECX = source): 7 dwords from source + 4 to destination + 4.
unsafe fn h_float_comp_copy(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        r.eax = shadow_mem(id, &p, [0; 3], &[(o + 4, 0x1C)], || {
            let tmp = snap_bytes(p.ecx as usize + 4, 0x1C);
            std::ptr::copy_nonoverlapping(tmp.as_ptr(), (o + 4) as *mut u8, 0x1C);
            p.eax
        });
    }
}
port_thunk!(float_comp_copy_thunk, float_comp_copy_entry, FLOAT_COMP_COPY, h_float_comp_copy);

/// 0x7FE0B0 net_vector3_compressor_ctor (EAX = object; stack: three 7-dword blocks and a float; `ret 0x58`):
/// vtable 0xE26500, 0xE264EC at +4 / +0x24 / +0x44, the blocks at +8 / +0x28 / +0x48, the float at +0x64.
unsafe fn h_vector3_comp_ctor(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        let v = args_of(a, 22);
        r.eax = shadow_mem_regs(id, &p, &v, &[(o, 0x68)], || {
            wd(o, 0x00E2_6500);
            wd(o + 4, 0x00E2_64EC);
            wd(o + 0x24, 0x00E2_64EC);
            wd(o + 0x44, 0x00E2_64EC);
            for i in 0..7 {
                wd(o + 8 + i * 4, v[i]);
            }
            for i in 0..7 {
                wd(o + 0x28 + i * 4, v[7 + i]);
            }
            for i in 0..7 {
                wd(o + 0x48 + i * 4, v[14 + i]);
            }
            wd(o + 0x64, q32(v[21]));
            p.eax
        });
    }
}
port_thunk!(vector3_comp_ctor_thunk, vector3_comp_ctor_entry, VECTOR3_COMP_CTOR, h_vector3_comp_ctor, "ret 88");

/// 0x7FE3A0 net_vector2_compressor_ctor (EAX = object; stack: two 7-dword blocks and a float; `ret 0x3C`).
unsafe fn h_vector2_comp_ctor(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        let v = args_of(a, 15);
        r.eax = shadow_mem_regs(id, &p, &v, &[(o, 0x48)], || {
            wd(o, 0x00E2_6514);
            wd(o + 4, 0x00E2_64EC);
            wd(o + 0x24, 0x00E2_64EC);
            for i in 0..7 {
                wd(o + 8 + i * 4, v[i]);
            }
            for i in 0..7 {
                wd(o + 0x28 + i * 4, v[7 + i]);
            }
            wd(o + 0x44, q32(v[14]));
            p.eax
        });
    }
}
port_thunk!(vector2_comp_ctor_thunk, vector2_comp_ctor_entry, VECTOR2_COMP_CTOR, h_vector2_comp_ctor, "ret 60");

/// 0x7FE5A0 net_vector3_uncompressed_ctor (EAX = object; stack: a float; `ret 4`): vtable 0xE26528, the float at +4.
unsafe fn h_vector3_uncomp_ctor(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        let v = args_of(a, 1);
        r.eax = shadow_mem_regs(id, &p, &v, &[(o, 8)], || {
            wd(o, 0x00E2_6528);
            wd(o + 4, q32(v[0]));
            p.eax
        });
    }
}
port_thunk!(vector3_uncomp_ctor_thunk, vector3_uncomp_ctor_entry, VECTOR3_UNCOMP_CTOR, h_vector3_uncomp_ctor, "ret 4");

/// 0x7FE6A0 net_orientation_heading_compressor_ctor (EAX = object; stack: a 7-dword block; `ret 0x1C`): vtable
/// 0xE2653C, 0xE264EC at +4, the block at +8, then +0x24 = the block's SECOND dword as a float (the original
/// loads it with `fld [esp+8]` before its pushes).
unsafe fn h_heading_comp_ctor(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        let v = args_of(a, 7);
        r.eax = shadow_mem_regs(id, &p, &v, &[(o, 0x28)], || {
            wd(o, 0x00E2_653C);
            wd(o + 4, 0x00E2_64EC);
            for i in 0..7 {
                wd(o + 8 + i * 4, v[i]);
            }
            wd(o + 0x24, q32(v[1]));
            p.eax
        });
    }
}
port_thunk!(heading_comp_ctor_thunk, heading_comp_ctor_entry, HEADING_COMP_CTOR, h_heading_comp_ctor, "ret 28");

// --- replicated-object helpers --------------------------------------------------------------------------------------------
// Handle slots: HANDLES + (handle & 0xFFF) * 16 -> object, valid when [obj + 0x44] == handle; [obj + 0xB0] is its
// replication record ([rec + 8] picks the manager in the table at 0x212E25C, [rec + 2] is the net id).

const MANAGERS: usize = 0x0212_E25C;

/// The object of a handle, or 0 (handle 0, empty slot, stale handle).
unsafe fn handle_obj(h: u32) -> u32 {
    unsafe {
        if h == 0 {
            return 0;
        }
        let o = rd(HANDLES + ((h & 0xFFF) as usize) * 16);
        if o != 0 && rd(o as usize + 0x44) == h { o } else { 0 }
    }
}

/// 0x806CE0 net_ownership_request_pending (EAX = object -> AL): is a request for the object's net id
/// (0xFFFF without a record) queued in the pending table (count [0x2528BAC], words at 0x212BB38 + 8 * i)?
/// EAX = the index reached, AL replaced.
unsafe fn h_ownership_pending(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let o = p.eax as usize;
        let h = rd(o + 0x44);
        let nid: u16 = if h == 0 {
            0xFFFF
        } else {
            let obj = handle_obj(h);
            if obj == 0 {
                0xFFFF
            } else {
                let rec = rd(obj as usize + 0xB0);
                if rec == 0 { 0xFFFF } else { rw(rec as usize + 2) }
            }
        };
        let count = rd(0x0252_8BAC) as i32;
        let mut ours = 0;
        if count > 0 {
            let mut i = 0i32;
            ours = loop {
                if rw(0x0212_BB38 + i as usize * 8) == nid {
                    break lo(i as u32, 1);
                }
                i += 1;
                if i >= count {
                    break lo(i as u32, 0);
                }
            };
        }
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(ownership_pending_thunk, ownership_pending_entry, OWNERSHIP_PENDING, h_ownership_pending);

/// 0x804940 net_conn_object_list_add (EAX = connection, EDX = handle): append to the connection's handle list
/// (dwords at +0x1DF0, count +0x21F0).
unsafe fn h_conn_object_list_add(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let c = p.eax as usize;
        r.eax = shadow_mem(id, &p, [0; 3], &[(c + 0x1DF0, 0x404)], || {
            let n = rd(c + 0x21F0) as usize;
            wd(c + n.wrapping_mul(4).wrapping_add(0x1DF0), p.edx);
            wd(c + 0x21F0, rd(c + 0x21F0).wrapping_add(1));
            p.eax
        });
    }
}
port_thunk!(conn_object_list_add_thunk, conn_object_list_add_entry, CONN_OBJECT_LIST_ADD, h_conn_object_list_add);

/// 0x804960 net_conn_object_list_remove (EAX = connection, EDI = handle -> AL): swap-remove the handle
/// from that list; AL = 1 when it was there. EAX = the connection with AL replaced.
unsafe fn h_conn_object_list_remove(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let c = p.eax as usize;
        r.eax = shadow_mem(id, &p, [0; 3], &[(c + 0x1DF0, 0x404)], || {
            let n = rd(c + 0x21F0);
            let mut i = 0u32;
            if n != 0 {
                loop {
                    if rd(c + 0x1DF0 + i as usize * 4) == p.edi {
                        let last = rd(c + (n as usize).wrapping_mul(4).wrapping_add(0x1DEC));
                        wd(c + 0x1DF0 + i as usize * 4, last);
                        wd(c + 0x21F0, rd(c + 0x21F0).wrapping_add(0xFFFF_FFFF));
                        return lo(p.eax, 1);
                    }
                    i += 1;
                    if i >= n {
                        break;
                    }
                }
            }
            lo(p.eax, 0)
        });
    }
}
port_thunk!(conn_object_list_remove_thunk, conn_object_list_remove_entry, CONN_OBJECT_LIST_REMOVE, h_conn_object_list_remove);

/// 0x8176C0 net_guid_id_equal (ESI, EDI = two GUID entries -> EAX 0 / 1): 0x8176E0 (ECX = ESI, EDX = EDI) and
/// equal id dwords (+0x24).
unsafe fn h_guid_id_equal(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let g = call_regs(0x0081_76E0, p.eax, p.esi, p.edi, p.esi, p.edi, &[]);
        let ours = (g as u8 != 0 && rd(p.edi as usize + 0x24) == rd(p.esi as usize + 0x24)) as u32;
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(guid_id_equal_thunk, guid_id_equal_entry, GUID_ID_EQUAL, h_guid_id_equal);

/// 0x817810 net_guid_entry_read (EAX = entry (0x28 bytes), EDX = cursor (byte offset) pointer, ESI = buffer):
/// clear the entry (id dword +0x24 = -1), read {8 bytes, 8 bytes, 4 bytes} from buffer + cursor into
/// +0 / +8 / +0x24, advancing the cursor by 8 / 8 / 4. EAX unchanged.
unsafe fn h_guid_entry_read(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let (e, cur, buf) = (p.eax as usize, p.edx as usize, p.esi);
        r.eax = shadow_mem(id, &p, [0; 3], &[(e, 0x28), (cur, 4)], || {
            for i in 0..9 {
                wd(e + i * 4, 0);
            }
            wd(e + 0x24, 0xFFFF_FFFF);
            let at = |c: u32, o: u32| buf.wrapping_add(c).wrapping_add(o) as usize;
            let c = rd(cur);
            wd(e, rd(at(c, 0)));
            wd(e + 4, rd(at(c, 4)));
            wd(cur, rd(cur).wrapping_add(8));
            let c = rd(cur);
            wd(e + 8, rd(at(c, 0)));
            wd(e + 0xC, rd(at(c, 4)));
            wd(cur, rd(cur).wrapping_add(8));
            let c = rd(cur);
            wd(e + 0x24, rd(at(c, 0)));
            wd(cur, rd(cur).wrapping_add(4));
            p.eax
        });
    }
}
port_thunk!(guid_entry_read_thunk, guid_entry_read_entry, GUID_ENTRY_READ, h_guid_entry_read);

/// 0x7FFDF0 net_id_to_object_ac (AX = net id -> EAX): like net_id_to_human without the flag test: the field
/// +0xAC of the object the net id's handle names, 0 when anything is missing.
unsafe fn h_id_to_object_ac(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        let ours = if p.eax as u16 == 0xFFFF {
            0
        } else {
            let key = p.eax & 0xFFFF;
            let node = call_regs(0x0080_8E30, key, p.ecx, p.edx, rd(NET_ID_MAP), p.edi, &[key, 0]);
            let rec = if node != 0 { rd(node as usize) } else { 0 };
            let handle = if rec != 0 { rd(rec as usize + 4) } else { 0 };
            if handle == 0 {
                0
            } else {
                let o = handle_obj(handle);
                if o == 0 { 0 } else { rd(o as usize + 0xAC) }
            }
        };
        r.eax = checked_eax(id, &p, [0; 3], ours);
    }
}
port_thunk!(id_to_object_ac_thunk, id_to_object_ac_entry, ID_TO_OBJECT_AC, h_id_to_object_ac);

// --- virtual-dispatch wrappers (no shadow: a second run would repeat the virtual call) --------------------------------------
// Each looks up the handle's record manager (table 0x212E25C, index [rec + 8]) and calls one of its virtual methods
// with the handle (and the caller's own arguments) pushed, ECX = the manager. Counted, not shadowed.

/// 0x805CD0 net_object_capture_snapshot (EDX = handle, ESI passed on): vtable +0x24 (handle, ESI), `ret 8`.
/// EAX when nothing is called: the input, the slot (stale), 0, or the record.
unsafe fn h_capture_snapshot(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        PORTS[id].calls.fetch_add(1, Ordering::Relaxed);
        if p.edx == 0 {
            return;
        }
        let slot = rd(HANDLES + ((p.edx & 0xFFF) as usize) * 16);
        if slot == 0 {
            r.eax = 0;
            return;
        }
        if rd(slot as usize + 0x44) != p.edx {
            r.eax = slot;
            return;
        }
        let rec = rd(slot as usize + 0xB0);
        if rec == 0 {
            r.eax = 0;
            return;
        }
        let m = rd(MANAGERS + rd(rec as usize + 8) as usize * 4);
        if m == 0 {
            r.eax = rec;
            return;
        }
        let vt = rd(m as usize);
        let f = rd(vt as usize + 0x24);
        r.eax = call_regs(f as usize, vt, m, f, p.esi, p.edi, &[p.edx, p.esi]);
    }
}
port_thunk!(capture_snapshot_thunk, capture_snapshot_entry, CAPTURE_SNAPSHOT, h_capture_snapshot);

/// 0x805D60 net_object_read_state_delta (EAX = handle; stack a0..a3; `ret`): vtable +0x30 (handle, a0..a3), `ret 0x14`;
/// EAX = 2 for a stale / recordless handle.
unsafe fn h_read_state_delta(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        PORTS[id].calls.fetch_add(1, Ordering::Relaxed);
        let rec = if p.eax != 0 {
            let o = handle_obj(p.eax);
            if o == 0 { 0 } else { rd(o as usize + 0xB0) }
        } else {
            0
        };
        if rec == 0 {
            r.eax = 2;
            return;
        }
        let m = rd(MANAGERS + rd(rec as usize + 8) as usize * 4);
        let vt = rd(m as usize);
        let f = rd(vt as usize + 0x30);
        r.eax = call_regs(f as usize, f, m, vt, *a, p.edi, &[p.eax, *a, *a.add(1), *a.add(2), *a.add(3)]);
    }
}
port_thunk!(read_state_delta_thunk, read_state_delta_entry, READ_STATE_DELTA, h_read_state_delta);

/// 0x805E40 net_object_get_send_priority (EAX = record; stack a0..a6; `ret`): vtable +0x38 ([rec + 4], a0..a6)
/// with ECX = the manager of [rec + 8]; `ret 0x20`. EAX = AX 0 (upper half kept) without a record. Returns AX.
unsafe fn h_get_send_priority(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        PORTS[id].calls.fetch_add(1, Ordering::Relaxed);
        if p.eax == 0 {
            r.eax = lo(p.eax, 0) & 0xFFFF_0000;
            return;
        }
        let m = rd(MANAGERS + rd(p.eax as usize + 8) as usize * 4);
        let vt = rd(m as usize);
        let h = rd(p.eax as usize + 4);
        let f = rd(vt as usize + 0x38);
        let args = [h, *a, *a.add(1), *a.add(2), *a.add(3), *a.add(4), *a.add(5), *a.add(6)];
        r.eax = call_regs(f as usize, h, m, vt, *a, p.edi, &args);
    }
}
port_thunk!(get_send_priority_thunk, get_send_priority_entry, GET_SEND_PRIORITY, h_get_send_priority);

/// 0x805E90 net_object_get_dirty_fields (EAX = handle, ECX = record; stack a0, a1; `ret`): vtable +0x34 (handle, a0, a1),
/// `ret 0xC`; EAX = AX 0 (upper half kept) for handle 0.
unsafe fn h_get_dirty_fields(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        PORTS[id].calls.fetch_add(1, Ordering::Relaxed);
        if p.eax == 0 {
            r.eax = p.eax & 0xFFFF_0000;
            return;
        }
        let m = rd(MANAGERS + rd(p.ecx as usize + 8) as usize * 4);
        let vt = rd(m as usize);
        let f = rd(vt as usize + 0x34);
        r.eax = call_regs(f as usize, f, m, vt, *a, p.edi, &[p.eax, *a, *a.add(1)]);
    }
}
port_thunk!(get_dirty_fields_thunk, get_dirty_fields_entry, GET_DIRTY_FIELDS, h_get_dirty_fields);

/// 0x805EC0 net_object_on_state_sent (EAX = handle; stack a0..a2; `ret`): vtable +0x3C (handle, a0..a2), `ret 0x10`.
/// EAX stays the handle when it is stale / recordless.
unsafe fn h_on_state_sent(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        PORTS[id].calls.fetch_add(1, Ordering::Relaxed);
        let rec = if p.eax != 0 {
            let o = handle_obj(p.eax);
            if o == 0 { 0 } else { rd(o as usize + 0xB0) }
        } else {
            0
        };
        if rec == 0 {
            return;
        }
        let m = rd(MANAGERS + rd(rec as usize + 8) as usize * 4);
        let vt = rd(m as usize);
        let f = rd(vt as usize + 0x3C);
        r.eax = call_regs(f as usize, f, m, vt, *a, p.edi, &[p.eax, *a, *a.add(1), *a.add(2)]);
    }
}
port_thunk!(on_state_sent_thunk, on_state_sent_entry, ON_STATE_SENT, h_on_state_sent);

/// 0x806D50 net_object_can_migrate (EAX = object -> AL/EAX): no for a null object, flag bits 0x0B (+0x80), no record,
/// or a record whose word +0x67E is not 0xFFFF (EAX = 0, or the object with AL = 0 for the first three);
/// else the manager's vtable +0x44 (handle), `ret 4`, decides (EAX = 1 / 0).
unsafe fn can_migrate(p: &Pushad) -> u32 {
    unsafe {
        let o = p.eax as usize;
        if p.eax == 0 {
            return 0;
        }
        if rb(o + 0x80) & 0xB != 0 {
            return lo(p.eax, 0);
        }
        let rec = rd(o + 0xB0);
        if rec == 0 {
            return lo(p.eax, 0);
        }
        if rw(rec as usize + 0x67E) != 0xFFFF {
            return 0;
        }
        let m = rd(MANAGERS + rd(rec as usize + 8) as usize * 4);
        let vt = rd(m as usize);
        let f = rd(vt as usize + 0x44);
        let h = rd(o + 0x44);
        let r = call_regs(f as usize, h, m, f, p.esi, p.edi, &[h]);
        (r as u8 != 0) as u32
    }
}

unsafe fn h_can_migrate(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        let p = *r;
        r.eax = checked_eax(id, &p, [0; 3], can_migrate(&p));
    }
}
port_thunk!(can_migrate_thunk, can_migrate_entry, CAN_MIGRATE, h_can_migrate);

// --- 0x806B90 net_request_ownership (stack: object, byte, callback; `ret`) -------------------------------------------------
// Object -> record (as above). With no session or no record: AL = 0 (EAX = the object's handle). A record that
// is flagged (+0x67C bit 1) or owned locally (0x806760: EAX = handle, ECX = record, ESI = session) needs no request:
// the callback (when non-null) is called with ([rec + 4], 1). Otherwise, unless the pending table [0x2528BAC] is full
// (0x20: AL = 0, EAX = 0x806760's), packet 0x28 {u16 [rec + 2], u8 arg2} goes to the connection 0x80FE80 finds for
// the record's owner byte [rec], and {word [rec + 2], callback} is appended to the pending table (0x212BB38, 8
// bytes each). AL = 1 at the end; EAX = the callback's / the count + 1 / the handle or 0x806760's.

unsafe fn request_ownership(p: &Pushad, obj: u32, arg2: u32, cb: u32) -> (u32, bool) {
    unsafe {
        let h = rd(obj as usize + 0x44);
        let rec = {
            let o = handle_obj(h);
            if o == 0 { 0 } else { rd(o as usize + 0xB0) }
        };
        let sess = rd(SESSION);
        if sess == 0 || rec == 0 {
            return (lo(h, 0), false);
        }
        let mut eax = h;
        let local = if rb(rec as usize + 0x67C) & 2 != 0 {
            true
        } else {
            eax = call_regs(0x0080_6760, h, rec, p.edx, sess, rec, &[]);
            eax as u8 != 0
        };
        if local {
            if cb != 0 {
                eax = call_regs(cb as usize, rd(rec as usize + 4), 0, 0, 0, 0, &[rd(rec as usize + 4), 1]);
            }
            return (lo(eax, 1), false);
        }
        let n = rd(0x0252_8BAC);
        if n == 0x20 {
            return (lo(eax, 0), false);
        }
        let mut buf = [0u8; 0x51C];
        buf[0] = 0x28;
        let hd = hdr_size(0x00E8_C2DC);
        buf[hd..hd + 2].copy_from_slice(&rw(rec as usize + 2).to_le_bytes());
        buf[hd + 2] = arg2 as u8;
        let size = hd + 3;
        set_len(&mut buf, size);
        let conn = call_regs(0x0080_FE80, sess, rec, rb(rec as usize) as u32, sess, rec, &[0, 0, size as u32, buf.as_ptr() as u32]);
        send_conn(conn, &mut buf, size as u32, 0, 0);
        let n = rd(0x0252_8BAC);
        ww(0x0212_BB38 + n as usize * 8, rw(rec as usize + 2));
        wd(0x0212_BB3C + n as usize * 8, cb);
        wd(0x0252_8BAC, n + 1);
        (lo(n + 1, 1), false)
    }
}

unsafe fn h_request_ownership(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        let p = *r;
        let (obj, arg2, cb) = (*a, *a.add(1), *a.add(2));
        // A callback call cannot be shadowed (it would run twice): that path is only counted.
        if shadowing_active(id) && cb != 0 {
            let h = rd(obj as usize + 0x44);
            let o = handle_obj(h);
            let rec = if o == 0 { 0 } else { rd(o as usize + 0xB0) };
            if rec != 0 && rd(SESSION) != 0 && (rb(rec as usize + 0x67C) & 2 != 0 || call_regs(0x0080_6760, h, rec, p.edx, rd(SESSION), rec, &[]) as u8 != 0) {
                PORTS[id].calls.fetch_add(1, Ordering::Relaxed);
                r.eax = request_ownership(&p, obj, arg2, cb).0;
                return;
            }
        }
        let n = rd(0x0252_8BAC) as usize;
        let regions = [(0x0252_8BACusize, 4usize), (0x0212_BB38 + n.min(0x20) * 8, 8)];
        r.eax = shadow_send_ex(id, &p, [obj, arg2, cb], rb(0x00E8_C2DC) != 0, false, &regions, || request_ownership(&p, obj, arg2, cb));
    }
}
port_thunk!(request_ownership_thunk, request_ownership_entry, REQUEST_OWNERSHIP, h_request_ownership);

/// The shadow copies' sends (port, offset of the `call`, recording stub): the copy of a sender
/// calls the stub instead of the send helper, so the original never really sends in a shadow run.
fn stub_table() -> Vec<(usize, usize, usize)> {
    let conn = rec_conn_thunk as *const () as usize;
    let remotes = rec_remotes_thunk as *const () as usize;
    let raw = rec_raw_thunk as *const () as usize;
    vec![
        (TIMEOUT_ARM_10S, 0x27, 0x0080_B8D0),
        (REQUEST_OWNERSHIP, 0xF5, conn),
        (SEND_MP_INFO, 0x61, remotes),
        (SEND_UPD_STATE_SUB3, 0x54, conn),
        (SET_PHASE_BROADCAST, 0x67, remotes),
        (SEND_JOIN_DENY, 0x5A, raw),
        (SEND_UPD_CONN, 0x99, conn),
        (SEND_TO_LIST_OR_REMOTES, 0x2E, remotes),
        (SEND_TO_LIST_OR_REMOTES, 0x5B, conn),
        (SEND_JOIN_NEW_SESSION, 0x69, conn),
        (SEND_JOIN_SUCCESS, 0x69, conn),
        (SEND_JOIN_FAILED, 0x69, conn),
        (SEND_JOIN_COMPLETE, 0x69, conn),
    ]
}

/// Calls of a copy into another ported function: (port, offset of the call, callee port); the copy
/// then calls the callee's original copy instead of the (Rust) replacement.
const DEPS2: [(usize, usize, usize); 10] = [
    (ID_TO_OBJECT_AC, 0x13, MAP_FIND),
    (GUID_ID_EQUAL, 0x4, GUID_EQUAL),
    (REQUEST_OWNERSHIP, 0x87, RECORD_OWNED),
    (REQUEST_OWNERSHIP, 0xEE, FIND_CONN),
    (ALL_CONNS_STATE_EQ, 0x2, PEERS_PHASE_EQUAL),
    (ALL_CONNS_STATE_GE, 0x2, PEERS_PHASE_EQUAL),
    (FIND_RESERVATION, 0x25, GUID_EQUAL),
    (GUID_IN_ARRAY, 0x24, GUID_EQUAL),
    (ID_TO_HUMAN, 0x13, MAP_FIND),
    (MAP_SET, 0xB, MAP_FIND),
];

fn round2_patch_copies() {
    for &(id, off, dep) in &DEPS2 {
        let (copy, callee) = (PORTS[id].original.load(Ordering::Relaxed), PORTS[dep].original.load(Ordering::Relaxed));
        if copy != 0 && callee != 0 {
            unsafe { set_rel32(copy, off, callee) };
        } else if copy != 0 {
            PORTS[id].original.store(0, Ordering::Relaxed);
        }
    }
    for (id, off, stub) in stub_table() {
        let copy = PORTS[id].original.load(Ordering::Relaxed);
        if copy != 0 {
            unsafe { set_rel32(copy, off, stub) };
        }
    }
}


// ===== round 3: the object core, join_req and the session action dispatcher =========================================
// These functions allocate pool records, call virtual methods of the per-type managers (table 0x212E25C), and send
// packets, so a shadow would double their side effects. They are checked with per-branch counters only (the summary
// line lists, per port, the calls and then the hit count of every numbered branch), and offline against the
// original bytes with the callees stubbed (see the `tests3` module). Callees are called by address with the
// registers and stack arguments the original sets up. Thread: every one of them runs on the game thread
// (sim_frame / packet receive); the network thread (0x814320) calls none of them.
// EBX / EBP are never inputs of a callee here: `call_regs` leaves them to the Rust frame.

/// The address a callee is called at: the game's own, except in the offline tests, which put stubs there.
#[cfg(not(test))]
#[inline(always)]
fn cal(a: usize) -> usize {
    a
}
#[cfg(test)]
fn cal(a: usize) -> usize {
    tests3::redirect(a)
}

const OBJ_HEAD: usize = 0x0212_E2AC;
const TYPE_LISTS: usize = 0x0212_E284;
const FREE_LIST: usize = 0x0212_C254;
const FREE_COUNT: usize = 0x0212_BA34;
const SNAP_FLAG: usize = 0x0144_E9B6;
const LOCAL_FLAG_B: usize = 0x0252_8B9D;

struct Port3 {
    name: &'static str,
    addr: usize,
    calls: AtomicU32,
    br: [AtomicU32; 24],
}

const fn p3(name: &'static str, addr: usize) -> Port3 {
    Port3 { name, addr, calls: AtomicU32::new(0), br: [const { AtomicU32::new(0) }; 24] }
}

const DISPATCH_ACTION: usize = 0;
const ADD_FIELD: usize = 1;
const REGS_RESET: usize = 2;
const DESTROY_FROM_PACKET: usize = 3;
const SEND_NEW_OWNER: usize = 4;
const OBJ_UNREGISTER: usize = 5;
const DEACTIVATE_SEND: usize = 6;
const REGISTER_LOCAL: usize = 7;
const REGISTER_FROM_CREATE: usize = 8;
const SEND_JOIN_REQ: usize = 9;
const COUNT3: usize = 10;

static P3S: [Port3; COUNT3] = [
    p3("session_dispatch_action", 0x0080_ED70),
    p3("net_registration_add_field", 0x0080_7780),
    p3("net_registrations_reset", 0x0080_6E30),
    p3("net_object_destroy_from_packet", 0x0080_4550),
    p3("net_object_send_new_owner", 0x0080_67D0),
    p3("net_object_unregister", 0x0080_4140),
    p3("net_object_deactivate_send", 0x0080_53B0),
    p3("net_object_register_local", 0x0080_5000),
    p3("net_object_register_from_create", 0x0080_4320),
    p3("net_send_join_req", 0x0081_5930),
];

fn enter(id: usize) {
    P3S[id].calls.fetch_add(1, Ordering::Relaxed);
}

/// Count a branch (numbered per port, see the comments of each port).
fn br(id: usize, k: usize) {
    P3S[id].br[k].fetch_add(1, Ordering::Relaxed);
}

/// The handle slot of `h` (the object, or 0 when the slot is empty); no staleness check.
unsafe fn slot_of(h: u32) -> u32 {
    unsafe { rd(HANDLES + ((h & 0xFFF) as usize) * 16) }
}

/// Append `rec` at the tail of the circular list of `ty` (0x212E284 + ty * 4), as the registration code does.
unsafe fn type_list_append(rec: u32, ty: u32) {
    unsafe {
        let head_slot = TYPE_LISTS + ty as usize * 4;
        let r = rec as usize;
        let head = rd(head_slot);
        if head == 0 {
            wd(r + 0x674, rec);
            wd(r + 0x678, rec);
        } else {
            let tail = rd(head as usize + 0x678);
            wd(tail as usize + 0x674, rec);
            let head = rd(head_slot);
            let tail = rd(head as usize + 0x678);
            wd(r + 0x678, tail);
            let head = rd(head_slot);
            wd(head as usize + 0x678, rec);
            let head = rd(head_slot);
            wd(r + 0x674, head);
        }
        wd(head_slot, rec);
    }
}

// --- 0x80ED70 session_dispatch_action (EAX = action, ESI = session) -----------------------------------------------------
// Branches: 1 = action 0xFF or > 5 (EAX = 2); 2 = actions 0 / 3 / 5 (EAX = 0); 3 / 4 / 5 = actions 1 / 2 / 4, which call the
// session vtable slot +0xC / +0x10 / +0x14 (ECX = this = ESI, EDX = the method, EAX = the vtable) and set byte +0xC00;
// EAX is the method's.
unsafe fn h_dispatch_action(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        enter(id);
        let act = r.eax;
        let s = r.esi;
        if act == 0xFF || act > 5 {
            br(id, 1);
            r.eax = 2;
            return;
        }
        let (slot, b) = match act {
            1 => (0xC, 3),
            2 => (0x10, 4),
            4 => (0x14, 5),
            _ => {
                br(id, 2);
                r.eax = 0;
                return;
            }
        };
        br(id, b);
        let vt = rd(s as usize);
        let f = rd(vt as usize + slot);
        r.eax = call_regs(f as usize, vt, s, f, s, r.edi, &[]);
        wb(s as usize + 0xC00, 1);
    }
}
port_thunk!(dispatch_action_thunk, dispatch_action_entry, DISPATCH_ACTION, h_dispatch_action);

// --- 0x807780 net_registration_add_field (EDX = registration, stack: a, b, c, d; `ret 0x10`) ----------------------------------
// Entry n = [reg + 0x18] at reg + 0x20 + n * 0x44; its index is stored at +8, the arguments at +0 (a), +4 (b), +0x40 (c),
// +0x3C (d). The bit width [reg + 0x1C] grows until count >> width is 0. Branches: 1 = full (EAX = 0); 2 = the index equals the
// all-ones mask of the new width (EAX = 0, the entry stays half written); 3 = added (EAX = entry + 8, not 1).
unsafe fn h_add_field(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        enter(id);
        let reg = r.edx as usize;
        let n = rd(reg + 0x18);
        if n >= 0x10 {
            br(id, 1);
            r.eax = 0;
            return;
        }
        let f = reg + 0x20 + n as usize * 0x44;
        let e = f + 8;
        wd(e + 0x10, 0);
        wd(e + 8, 0);
        ww(e + 0xC, 0);
        wd(e + 4, 0);
        wd(e, 0xFFFF_FFFF);
        wd(e, rd(reg + 0x18));
        wd(f + 0x3C, *a.add(3));
        wd(reg + 0x18, rd(reg + 0x18).wrapping_add(1));
        let bits = rd(reg + 0x1C);
        if rd(reg + 0x18) >> (bits & 31) != 0 {
            loop {
                wd(reg + 0x1C, rd(reg + 0x1C).wrapping_add(1));
                if rd(reg + 0x18) >> (rd(reg + 0x1C) & 31) == 0 {
                    break;
                }
            }
        }
        let mask = (1u32 << (rd(reg + 0x1C) & 31)).wrapping_sub(1);
        if rd(e) == mask {
            br(id, 2);
            r.eax = 0;
            return;
        }
        br(id, 3);
        wd(f, *a);
        wd(f + 4, *a.add(1));
        wd(f + 0x40, *a.add(2));
        r.eax = e as u32;
    }
}
port_thunk!(add_field_thunk, add_field_entry, ADD_FIELD, h_add_field, "ret 16");

// --- 0x804140 net_object_unregister (ESI = record) ------------------------------------------------------------------------
// Unlinks the record from its type list, drops its snapshot data (0x7FFB70, when byte 0x144E9B6 is set), detaches the
// object (the manager's vtable +0x10 with the handle, then clears bit 7 of [obj + 0x80] and [obj + 0xB0]), frees the
// net-id mapping (0x807AA0), unlinks it from the global list (0x212E2AC) and pushes it on the free list (0x212C254).
// Branches: 1 = null record (EAX unchanged); 2 = type out of 0..9 (EAX = the type); 3 = it headed its type list alone;
// 4 = it headed a longer list; 5 = not the list head; 6 = snapshot data freed; 7 = object found and detached;
// 8 = it headed the global list alone; 9 = it headed a longer global list; 10 = free list empty; 11 = free list non-empty.
// EAX at the end: the old free-list head (0 or the head), the type, or the entry EAX.
unsafe fn unregister(id: usize, esi: u32, edi_in: u32, eax_in: u32) -> u32 {
    unsafe {
        if esi == 0 {
            br(id, 1);
            return eax_in;
        }
        let s = esi as usize;
        let ty = rd(s + 8);
        if (ty as i32) < 0 || (ty as i32) >= 10 {
            br(id, 2);
            return ty;
        }
        let head_slot = TYPE_LISTS + ty as usize * 4;
        let mut skip_unlink = false;
        if rd(head_slot) == esi {
            let next = rd(s + 0x674);
            if next == esi {
                br(id, 3);
                wd(s + 0x674, 0);
                wd(s + 0x678, 0);
                wd(head_slot, 0);
                skip_unlink = true;
            } else {
                br(id, 4);
                wd(head_slot, next);
            }
        } else {
            br(id, 5);
        }
        let mut eax_cur = ty;
        if !skip_unlink {
            let prev = rd(s + 0x678);
            eax_cur = prev;
            let next = rd(s + 0x674);
            wd(prev as usize + 0x674, next);
            let next = rd(s + 0x674);
            let prev = rd(s + 0x678);
            wd(next as usize + 0x678, prev);
            wd(s + 0x674, 0);
            wd(s + 0x678, 0);
        }
        if rb(SNAP_FLAG) != 0 {
            br(id, 6);
            call_regs(cal(0x007F_FB70), (eax_cur & 0xFFFF_0000) | rw(s + 2) as u32, 0, 0, esi, edi_in, &[]);
        }
        let h = rd(s + 4);
        if h != 0 {
            let obj = slot_of(h);
            if obj != 0 && rd(obj as usize + 0x44) == h {
                br(id, 7);
                let mgr = rd(MANAGERS + rd(s + 8) as usize * 4);
                let vt = rd(mgr as usize);
                let f = rd(vt as usize + 0x10);
                call_regs(f as usize, f, mgr, vt, esi, obj, &[h]);
                wd(obj as usize + 0x80, rd(obj as usize + 0x80) & 0xFFFF_FF7F);
                wd(obj as usize + 0xB0, 0);
            }
        }
        call_regs(cal(0x0080_7AA0), rw(s + 2) as u32, 0, 0, esi, edi_in, &[]);
        if rd(OBJ_HEAD) == esi {
            let next = rd(s + 0x66C);
            if next == esi {
                br(id, 8);
                wd(OBJ_HEAD, 0);
                return unregister_free_list(id, s, esi);
            }
            br(id, 9);
            wd(OBJ_HEAD, next);
        }
        let prev = rd(s + 0x670);
        if prev != 0 {
            wd(prev as usize + 0x66C, rd(s + 0x66C));
        }
        let next = rd(s + 0x66C);
        if next != 0 {
            wd(next as usize + 0x670, rd(s + 0x670));
        }
        unregister_free_list(id, s, esi)
    }
}

/// The tail of 0x804140: the record goes on the free list; EAX = the old head.
unsafe fn unregister_free_list(id: usize, s: usize, esi: u32) -> u32 {
    unsafe {
        let head = rd(FREE_LIST);
        wd(s + 0x670, 0);
        wd(s + 0x66C, 0);
        if head == 0 {
            br(id, 10);
            wd(FREE_COUNT, rd(FREE_COUNT).wrapping_sub(1));
            wd(s + 0x66C, esi);
            wd(s + 0x670, esi);
            wd(FREE_LIST, esi);
            return 0;
        }
        br(id, 11);
        let tail = rd(head as usize + 0x670);
        wd(FREE_COUNT, rd(FREE_COUNT).wrapping_sub(1));
        wd(tail as usize + 0x66C, esi);
        wd(s + 0x670, rd(head as usize + 0x670));
        wd(head as usize + 0x670, esi);
        wd(s + 0x66C, head);
        wd(FREE_LIST, esi);
        head
    }
}

unsafe fn h_obj_unregister(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        enter(id);
        r.eax = unregister(id, r.esi, r.edi, r.eax);
    }
}
port_thunk!(obj_unregister_thunk, obj_unregister_entry, OBJ_UNREGISTER, h_obj_unregister);

// --- 0x806E30 net_registrations_reset (no arguments) ----------------------------------------------------------------------
// Walks the global record list (head 0x212E2AC) unregistering each record with 0x804140 (ESI = record; the next link is read
// before the call and is 0 when it points back at the head), then clears the per-type heads and tails and sets byte 0xE8860A.
// Branches: 1 = list empty; 2 = at least one record unregistered. EAX = the last 0x804140's.
unsafe fn h_regs_reset(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        enter(id);
        let mut eax = r.eax;
        let mut esi = rd(OBJ_HEAD);
        br(id, if esi == 0 { 1 } else { 2 });
        while esi != 0 {
            let next = rd(esi as usize + 0x66C);
            let edi = if next != rd(OBJ_HEAD) { next } else { 0 };
            eax = call_regs(cal(0x0080_4140), next, r.ecx, r.edx, esi, edi, &[]);
            esi = edi;
        }
        wd(OBJ_HEAD, 0);
        wd(0x0212_B518, 0);
        wd(0x0212_E284, 0);
        wd(0x0212_B4F0, 0);
        wd(0x0212_E288, 0);
        wd(0x0212_B4F4, 0);
        wd(0x0212_E28C, 0);
        wd(0x0212_B4F8, 0);
        wd(0x0212_E290, 0);
        wd(0x0212_B4FC, 0);
        wd(0x0212_E294, 0);
        wd(0x0212_B500, 0);
        wd(0x0212_E298, 0);
        wd(0x0212_B504, 0);
        wd(0x0212_E29C, 0);
        wd(0x0212_B508, 0);
        wd(0x0212_E2A0, 0);
        wd(0x0212_B50C, 0);
        wd(0x0212_E2A4, 0);
        wd(0x0212_B510, 0);
        wd(0x0212_E2A8, 0);
        wd(0x0212_B514, 0);
        wd(FREE_COUNT, 0);
        wb(0x00E8_860A, 1);
        r.eax = eax;
    }
}
port_thunk!(regs_reset_thunk, regs_reset_entry, REGS_RESET, h_regs_reset);

// --- 0x804550 net_object_destroy_from_packet (stack: packet {i32 type, u16 id at +4, ...}; AL = 1 / the 0x836DD0 flag) -------
// Branches: 1 = type outside 0..10 or no manager (AL = 1); 2 = id not in the map (0x836DD0 decides: AL = (AL == 1));
// 3 = map node with a null record (the same); 4 = record found: 0x8044C0, the manager's vtable +0x18 (handle, &0, packet + 6),
// 0x804140, AL = 1.
unsafe fn h_destroy_from_packet(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        enter(id);
        let pkt = *a;
        let ty = rd(pkt as usize);
        if (ty as i32) < 0 || (ty as i32) > 10 || rd(MANAGERS + ty as usize * 4) == 0 {
            br(id, 1);
            r.eax = lo(ty, 1);
            return;
        }
        let key = rw(pkt as usize + 4) as u32;
        let map = rd(NET_ID_MAP);
        let node = call_regs(cal(0x0080_8E30), key, r.ecx, r.edx, map, pkt + 4, &[key, 0]);
        let mut esi = map;
        let mut have = false;
        if node != 0 {
            esi = rd(node as usize);
            have = esi != 0;
        }
        if !have {
            br(id, if node == 0 { 2 } else { 3 });
            let v = call_regs(cal(0x0083_6DD0), node, pkt + 4, r.edx, esi, pkt + 4, &[]);
            r.eax = lo(v, ((v as u8) == 1) as u32);
            return;
        }
        br(id, 4);
        let s = esi as usize;
        let h = rd(s + 4);
        let cx = rw(s + 2) as u32;
        call_regs(cal(0x0080_44C0), node, cx, h, esi, pkt + 4, &[0]);
        let mgr = rd(MANAGERS + rd(s + 8) as usize * 4);
        let vt = rd(mgr as usize);
        let f = rd(vt as usize + 0x18);
        let mut out = 0u32;
        call_regs(f as usize, rd(s + 4), mgr, vt, esi, pkt + 4, &[rd(s + 4), &mut out as *mut u32 as u32, pkt + 6]);
        let e = call_regs(cal(0x0080_4140), f, mgr, vt, esi, pkt + 4, &[]);
        r.eax = lo(e, 1);
    }
}
port_thunk!(destroy_from_packet_thunk, destroy_from_packet_entry, DESTROY_FROM_PACKET, h_destroy_from_packet);

// --- 0x8067D0 net_object_send_new_owner (stack: packet {u16 id, u8 owner at +2, u8 flag at +3, ... +4 payload, u16 size at +0x204}) ---
// Branches: 1 = id not mapped, no record or no session (AL = 1); 2 = no connection and no fallback: packet is passed to 0x804710 (ECX = &6)
// (AL = 1); 3 = connection found; then 4 = the object cannot migrate (EAX = the check's), 5 = migrates: owner byte stored, the payload
// sent to the connection (0x814670) and the manager's vtable +0x50 (handle, connection) called (AL = 1).
unsafe fn h_send_new_owner(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        enter(id);
        let pkt = *a;
        let p = pkt as usize;
        let key = rw(p) as u32;
        let map = rd(NET_ID_MAP);
        let node = call_regs(cal(0x0080_8E30), key, r.ecx, r.edx, map, r.edi, &[key, 0]);
        if node == 0 {
            br(id, 1);
            r.eax = lo(node, 1);
            return;
        }
        let rec = rd(node as usize);
        if rec == 0 {
            br(id, 1);
            r.eax = lo(node, 1);
            return;
        }
        let sess = rd(SESSION);
        if sess == 0 {
            br(id, 1);
            r.eax = lo(node, 1);
            return;
        }
        let edx = (pkt & !0xFF) | rb(p + 2) as u32;
        let local = rd(sess as usize + 0x408);
        let mut esi = call_regs(cal(0x0080_FE80), sess, r.ecx, edx, map, rec, &[]);
        let found;
        if esi != 0 {
            found = true;
        } else {
            let mut walk = false;
            if rb(p + 3) != 0 {
                let ecx = rd(local as usize + 0x1AC);
                if local == rd(ecx as usize + 0x404) {
                    walk = true;
                } else {
                    esi = rd(sess as usize + 0x404);
                }
            }
            if walk {
                // the connection list of the session, starting at its head: the first entry other than the local one
                let head = rd(sess as usize + 0x400);
                esi = head;
                if esi == 0 {
                    found = false;
                } else {
                    loop {
                        if esi != local {
                            found = true;
                            break;
                        }
                        esi = rd(esi as usize + 0x22CC);
                        if esi == head || esi == 0 {
                            found = false;
                            break;
                        }
                    }
                }
            } else {
                found = esi != 0;
            }
        }
        if !found {
            br(id, 2);
            let mut six = 6u32;
            let e = call_regs(cal(0x0080_4710), node, &mut six as *mut u32 as u32, edx, esi, rec, &[pkt + 4]);
            r.eax = lo(e, 1);
            return;
        }
        br(id, 3);
        let h = rd(rec as usize + 4);
        let obj = if h == 0 {
            0
        } else {
            let o = slot_of(h);
            if o != 0 && rd(o as usize + 0x44) == h { o } else { 0 }
        };
        let can = call_regs(cal(0x0080_6D50), obj, r.ecx, edx, esi, rec, &[]);
        if can as u8 == 0 {
            br(id, 4);
            r.eax = can;
            return;
        }
        br(id, 5);
        wb(rec as usize, rb(esi as usize + 0x1B0));
        let size = rw(p + 0x204) as u32;
        call_regs(cal(0x0081_4670), can, r.ecx, edx, esi, rec, &[pkt + 4, size, 0, 0]);
        let mgr = rd(MANAGERS + rd(rec as usize + 8) as usize * 4);
        let vt = rd(mgr as usize);
        let f = rd(vt as usize + 0x50);
        let hh = rd(rec as usize + 4);
        let e = call_regs(f as usize, f, mgr, hh, esi, rec, &[hh, esi]);
        r.eax = lo(e, 1);
    }
}
port_thunk!(send_new_owner_thunk, send_new_owner_entry, SEND_NEW_OWNER, h_send_new_owner);

// --- 0x8053B0 net_object_deactivate_send (ECX = handle) ----------------------------------------------------------------------
// If the object's record belongs to the local connection, packet 0x0F {u32 [session + 8], u8 [rec + 8], u16 [rec + 2], manager payload
// (vtable +0x14: handle, &buffer, &cursor)} goes to every connection of the session whose state is > 7, whose byte +0 is 0 and that
// does not hold the handle in its owner list (0x804960 removes it there; EAX = connection, EDI = handle); finally the record is
// unregistered (0x804140, ESI = record). Branches: 1 = no object / record / session, or the record is not the local connection's;
// 2 = the connection list is empty; 3 = sent to a connection (0x814670, ESI = connection); 4 = a connection skipped (state or byte
// test); 5 = skipped because 0x804960 found the handle. EAX = the final 0x804140's, or what the original leaves on the early exits.
unsafe fn h_deactivate_send(id: usize, r: &mut Pushad, _a: *const u32) {
    unsafe {
        enter(id);
        let h = r.ecx;
        if h == 0 {
            br(id, 1);
            return;
        }
        let obj = slot_of(h);
        if obj == 0 || rd(obj as usize + 0x44) != h {
            br(id, 1);
            r.eax = obj;
            return;
        }
        let rec = rd(obj as usize + 0xB0);
        if rec == 0 {
            br(id, 1);
            r.eax = obj;
            return;
        }
        let sess = rd(SESSION);
        if sess == 0 {
            br(id, 1);
            r.eax = obj;
            return;
        }
        let local = rd(sess as usize + 0x408);
        if rb(rec as usize) != rb(local as usize + 0x1B0) {
            br(id, 1);
            r.eax = lo(obj, rb(rec as usize) as u32);
            return;
        }
        let mgr = rd(MANAGERS + rd(rec as usize + 8) as usize * 4);
        let mut buf = [0u8; 0x520];
        buf[0] = 0x0F;
        let mut cur = if rb(0x00E8_C084) == 0 { 3u32 } else { 4 };
        let stamp = rd(sess as usize + 8);
        buf[(cur as u16) as usize..][..4].copy_from_slice(&stamp.to_le_bytes());
        cur += 4;
        buf[(cur as u16) as usize] = rb(rec as usize + 8);
        cur += 1;
        buf[(cur as u16) as usize..][..2].copy_from_slice(&rw(rec as usize + 2).to_le_bytes());
        cur += 2;
        let vt = rd(mgr as usize);
        let f = rd(vt as usize + 0x14);
        let hh = rd(rec as usize + 4);
        let cur_slot = &mut cur as *mut u32 as u32;
        let e = call_regs(f as usize, vt, mgr, f, mgr, r.edi, &[hh, buf.as_mut_ptr() as u32, cur_slot]);
        let cur = std::ptr::read_volatile(&cur);
        buf[1..3].copy_from_slice(&(cur.wrapping_sub(3) as u16).to_le_bytes());
        let list = rd(sess as usize + 0x400);
        if list == 0 {
            br(id, 2);
        } else {
            let mut c = list;
            loop {
                let cp = c as usize;
                if (rd(cp + 0x3C) as i32) > 7 && rb(cp) == 0 {
                    let a = call_regs(cal(0x0080_4960), c, e, mgr, c, hh, &[]);
                    if a as u8 == 0 {
                        br(id, 3);
                        call_regs(cal(0x0081_4670), a, (cur as u16) as u32, mgr, c, hh, &[buf.as_mut_ptr() as u32, (cur as u16) as u32, 0, 0]);
                    } else {
                        br(id, 5);
                    }
                } else {
                    br(id, 4);
                }
                c = rd(cp + 0x22CC);
                if c == rd(sess as usize + 0x400) || c == 0 {
                    break;
                }
            }
        }
        r.eax = call_regs(cal(0x0080_4140), e, mgr, f, rec, r.edi, &[]);
    }
}
port_thunk!(deactivate_send_thunk, deactivate_send_entry, DEACTIVATE_SEND, h_deactivate_send);

// --- 0x805000 net_object_register_local (EDI = handle, stack: type) ----------------------------------------------------------
// Allocates a record (0x804050), maps its net id (0x807A20), links it into the type list, attaches it to the object ([obj + 0xB0]),
// calls the manager's vtable +4 (handle), clears the connection's bit in [rec + 0x67E] for every connection with state > 7 and byte 0 = 0
// (appending the handle to their owner list) and finally points the object that 0x925030 finds at the record.
// Branches: 1 = the object already has a record; 2 = type != 6 with [0x2528BB4] > 0; 3 = no session; 4 = no object or object flag
// bit 0 (EAX = the virtual call's, or [0x27B3C60]); 5 = allocation failed; 6 = registered; 7 = 0x925030 found no object; 8 = the
// virtual call through [0x27B3C60] (its state is 1, slot +4 of the object at +0x90) was made.
unsafe fn h_register_local(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        enter(id);
        let edi = r.edi;
        let ty = *a;
        let mut eax = r.eax;
        if edi != 0 {
            eax = slot_of(edi);
            if eax != 0 && rd(eax as usize + 0x44) == edi && rd(eax as usize + 0xB0) != 0 {
                br(id, 1);
                r.eax = eax;
                return;
            }
        }
        if ty != 6 && (rd(0x0252_8BB4) as i32) > 0 {
            br(id, 2);
            r.eax = eax;
            return;
        }
        let sess = rd(SESSION);
        if sess == 0 {
            br(id, 3);
            r.eax = 0;
            return;
        }
        let local = rd(sess as usize + 0x408);
        let mut ebp = 0;
        if edi != 0 {
            let o = slot_of(edi);
            if o != 0 && rd(o as usize + 0x44) == edi {
                ebp = o;
            }
        }
        let o = rd(0x027B_3C60);
        eax = o;
        if o != 0 && rd(o as usize + 0x3C) == 1 {
            br(id, 8);
            let c = rd(o as usize + 0x90);
            let vt = rd(c as usize);
            let f = rd(vt as usize + 4);
            eax = call_regs(f as usize, vt, c, f, r.esi, edi, &[]);
        }
        if ebp == 0 || rb(ebp as usize + 0x80) & 1 != 0 {
            br(id, 4);
            r.eax = eax;
            return;
        }
        let rec = call_regs(cal(0x0080_4050), eax, r.ecx, r.edx, r.esi, edi, &[]);
        if rec == 0 {
            br(id, 5);
            r.eax = 0;
            return;
        }
        br(id, 6);
        let rp = rec as usize;
        ww(rp + 2, edi as u16);
        call_regs(cal(0x0080_7A20), edi & 0xFFFF, r.ecx, r.edx, rec, edi, &[rec]);
        wd(rp + 4, edi);
        wd(rp + 8, ty);
        wb(rp, rb(local as usize + 0x1B0));
        wb(rp + 0x67C, 0);
        type_list_append(rec, ty);
        wd(ebp as usize + 0xB0, rec);
        let mgr = rd(MANAGERS + ty as usize * 4);
        let vt = rd(mgr as usize);
        let f = rd(vt as usize + 4);
        call_regs(f as usize, f, mgr, vt, rec, edi, &[edi]);
        ww(rp + 0x67E, 0xFFFF);
        let head = rd(sess as usize + 0x400);
        if head != 0 {
            let mut c = head;
            loop {
                let cp = c as usize;
                if (rd(cp + 0x3C) as i32) > 7 && rb(cp) == 0 {
                    let n = rd(cp + 0x21F0);
                    wd(cp + n as usize * 4 + 0x1DF0, edi);
                    let cl = rb(cp + 0x1B0);
                    wd(cp + 0x21F0, rd(cp + 0x21F0).wrapping_add(1));
                    let bit = !(1u32 << (cl & 31));
                    ww(rp + 0x67E, rw(rp + 0x67E) & bit as u16);
                }
                c = rd(cp + 0x22CC);
                if c == rd(sess as usize + 0x400) || c == 0 {
                    break;
                }
            }
        }
        if rb(SNAP_FLAG) != 0 {
            call_regs(cal(0x007F_FAC0), 0, 0, 0, rec, edi, &[rw(rp + 2) as u32]);
        }
        let obj = call_regs(cal(0x0092_5030), 0, edi, 0, rec, edi, &[]);
        if obj == 0 {
            br(id, 7);
            r.eax = 0;
            return;
        }
        wd(obj as usize + 0xB0, rec);
        r.eax = obj;
    }
}
port_thunk!(register_local_thunk, register_local_entry, REGISTER_LOCAL, h_register_local);

// --- 0x804320 net_object_register_from_create (stack: create packet {u16 id, i32 type at +4, payload at +8, u8 owner at +0x208}) ----
// AL = 1 unless allocation fails (0). Branches: 1 = type out of 0..9, no manager or no connection for the owner byte (AL = 1);
// 2 = the manager's vtable +0xC (5 args, byte 0x2528B9D set around it) produced no handle: AL = (0x804F00(id) != 0); 3 = allocation
// failed (AL = 0); 4 = record built and registered (0x925030, 0x807A20, 0x7FFAC0, manager vtable +4, 0x803FA0; AL = 1).
unsafe fn h_register_from_create(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        enter(id);
        let pkt = *a;
        let p = pkt as usize;
        let ty = rd(p + 4);
        if (ty as i32) < 0 || (ty as i32) >= 10 {
            br(id, 1);
            r.eax = lo(ty, 1);
            return;
        }
        let mgr = rd(MANAGERS + ty as usize * 4);
        if mgr == 0 {
            br(id, 1);
            r.eax = lo(ty, 1);
            return;
        }
        let sess = rd(SESSION);
        let edx = (r.edx & !0xFF) | rb(p + 0x208) as u32;
        let conn = call_regs(cal(0x0080_FE80), sess, r.ecx, edx, r.esi, pkt, &[]);
        if conn == 0 {
            br(id, 1);
            r.eax = 1;
            return;
        }
        let mut l0 = 0u32;
        let mut l1 = 0u32;
        let vt = rd(mgr as usize);
        let f = rd(vt as usize + 0xC);
        wb(LOCAL_FLAG_B, 1);
        let e = call_regs(
            f as usize,
            conn,
            mgr,
            f,
            r.esi,
            pkt,
            &[&mut l1 as *mut u32 as u32, &mut l0 as *mut u32 as u32, pkt + 8, rw(p) as u32, rb(conn as usize + 0x1B0) as u32],
        );
        wb(LOCAL_FLAG_B, 0);
        let l1 = std::ptr::read_volatile(&l1);
        if l1 == 0 {
            br(id, 2);
            let v = call_regs(cal(0x0080_4F00), rw(p) as u32, 0, 0, r.esi, pkt, &[]);
            r.eax = lo(v, (v != 0) as u32);
            return;
        }
        let rec = call_regs(cal(0x0080_4050), e, 0, 0, r.esi, pkt, &[]);
        if rec == 0 {
            br(id, 3);
            r.eax = 0;
            return;
        }
        br(id, 4);
        let rp = rec as usize;
        wd(rp + 4, l1);
        ww(rp + 2, rw(p));
        wd(rp + 8, rd(p + 4));
        wb(rp, rb(p + 0x208));
        wb(rp + 0x67C, 0);
        type_list_append(rec, rd(p + 4));
        let obj = call_regs(cal(0x0092_5030), 0, l1, 0, rec, pkt, &[]);
        if obj != 0 {
            wd(obj as usize + 0xB0, rec);
        }
        call_regs(cal(0x0080_7A20), rw(rp + 2) as u32, l1, 0, rec, pkt, &[rec]);
        if rb(SNAP_FLAG) != 0 {
            call_regs(cal(0x007F_FAC0), obj, rw(p) as u32, 0, rec, pkt, &[rw(p) as u32]);
        }
        let vt = rd(mgr as usize);
        let f2 = rd(vt as usize + 4);
        call_regs(f2 as usize, l1, mgr, f2, rec, pkt, &[l1]);
        let e = call_regs(cal(0x0080_3FA0), f2, rec, vt, rec, pkt, &[]);
        r.eax = lo(e, 1);
    }
}
port_thunk!(register_from_create_thunk, register_from_create_entry, REGISTER_FROM_CREATE, h_register_from_create);

// --- 0x815930 net_send_join_req (stack: connection to the host, requester object, session-like object; `ret`) --------------------
// Packet 0x02 {string 0x212AA48 + NUL, u32 0xC9, u32 [0xE8C3E8], <0x7FB3D0 record of the local player (ESI = player + 0x28)>,
// u64 [player + 0x14C], u64 [+0x154], u32 [+0x170], <requester's vtable +0x28 (&buffer, &cursor)>, u8 [requester + 0xC + 0x48],
// wide string [player + 0x4C] with its NUL, u8 [player + 0x44], u8 [player + 0x45], then either one byte 0 or (when the third
// argument is set, [0x2528B56] is clear or [0x2528B2B] is set, and its byte +0x3F8 is set) a byte [arg + 0x30] - 1 followed by 20 bytes
// (u64, u64, u32 at +0x14C, +0x154, +0x170) for each state-0 connection of its list (+0x400)}; the length word at buffer + 1 is the
// size - 3 and the packet goes to the host with 0x814670 (ESI = host). EAX = the send's.
// Branches: 1 = no third argument or gated off (a byte 0 is appended); 2 = the connection list was walked; 3 = a list entry
// contributed its 20 bytes.
unsafe fn h_send_join_req(id: usize, r: &mut Pushad, a: *const u32) {
    unsafe {
        enter(id);
        let (host, req, third) = (*a, *a.add(1), *a.add(2));
        let mut buf = [0u8; 0x520];
        buf[0] = 2;
        let mut cur: u32 = if rb(0x00E8_BF4C) == 0 { 3 } else { 4 };
        let s0 = 0x0212_AA48usize;
        let mut n = 0usize;
        while rb(s0 + n) != 0 {
            n += 1;
        }
        let n16 = n as u16 as u32;
        let hdr = cur;
        std::ptr::copy_nonoverlapping(s0 as *const u8, buf.as_mut_ptr().add((hdr as u16) as usize), n16 as usize);
        cur += n16;
        let player = rd(req as usize + 0x408);
        buf[(cur as u16) as usize] = 0;
        cur += 1;
        buf[(cur as u16) as usize..][..4].copy_from_slice(&0xC9u32.to_le_bytes());
        cur += 4;
        buf[(cur as u16) as usize..][..4].copy_from_slice(&rd(0x00E8_C3E8).to_le_bytes());
        cur += 4;
        let bp = buf.as_mut_ptr() as u32;
        let cp = &mut cur as *mut u32 as u32;
        call_regs(cal(0x007F_B3D0), cp, bp, 0, player + 0x28, player, &[]);
        cur = std::ptr::read_volatile(&cur);
        let put8 = |buf: &mut [u8; 0x520], cur: &mut u32, p: usize| {
            buf[(*cur as u16) as usize..][..4].copy_from_slice(&rd(p).to_le_bytes());
            buf[((*cur as u16) as usize) + 4..][..4].copy_from_slice(&rd(p + 4).to_le_bytes());
            *cur += 8;
        };
        put8(&mut buf, &mut cur, player as usize + 0x14C);
        put8(&mut buf, &mut cur, player as usize + 0x154);
        buf[(cur as u16) as usize..][..4].copy_from_slice(&rd(player as usize + 0x170).to_le_bytes());
        cur += 4;
        let obj = rd(req as usize + 0xC);
        let vt = rd(obj as usize);
        let f = rd(vt as usize + 0x28);
        call_regs(f as usize, vt, obj, f, rd(player as usize + 0x158), player, &[bp, cp]);
        cur = std::ptr::read_volatile(&cur);
        buf[(cur as u16) as usize] = rb(rd(req as usize + 0xC) as usize + 0x48);
        cur += 1;
        let ws = player as usize + 0x4C;
        let mut wn = 0usize;
        while rw(ws + wn * 2) != 0 {
            wn += 1;
        }
        let wl = ((wn * 2 + 2) as u16) as u32;
        std::ptr::copy_nonoverlapping(ws as *const u8, buf.as_mut_ptr().add((cur as u16) as usize), wl as usize);
        cur += wl;
        buf[(cur as u16) as usize] = rb(player as usize + 0x44);
        cur += 1;
        buf[(cur as u16) as usize] = rb(player as usize + 0x45);
        cur += 1;
        let gated = third != 0 && (rb(0x0252_8B56) == 0 || rb(0x0252_8B2B) != 0) && rb(third as usize + 0x3F8) != 0;
        if !gated {
            br(id, 1);
            buf[(cur as u16) as usize] = 0;
            cur += 1;
        } else {
            br(id, 2);
            buf[(cur as u16) as usize] = rb(third as usize + 0x30).wrapping_sub(1);
            cur += 1;
            let head = rd(third as usize + 0x400);
            if head != 0 {
                let mut c = head;
                loop {
                    let cp2 = c as usize;
                    if rb(cp2) == 0 {
                        br(id, 3);
                        put8(&mut buf, &mut cur, cp2 + 0x14C);
                        put8(&mut buf, &mut cur, cp2 + 0x154);
                        buf[(cur as u16) as usize..][..4].copy_from_slice(&rd(cp2 + 0x170).to_le_bytes());
                        cur += 4;
                    }
                    c = rd(cp2 + 0x22CC);
                    if c == head || c == 0 {
                        break;
                    }
                }
            }
        }
        buf[1..3].copy_from_slice(&(cur.wrapping_sub(3) as u16).to_le_bytes());
        r.eax = call_regs(cal(0x0081_4670), 0, 0, 0, host, player, &[bp, (cur as u16) as u32, 0, 0]);
    }
}
port_thunk!(send_join_req_thunk, send_join_req_entry, SEND_JOIN_REQ, h_send_join_req);

fn install3() -> usize {
    let list: [(usize, &[u8], extern "C" fn()); COUNT3] = [
        (DISPATCH_ACTION, &B_DISPATCH_ACTION, dispatch_action_thunk),
        (ADD_FIELD, &B_ADD_FIELD, add_field_thunk),
        (REGS_RESET, &B_REGS_RESET, regs_reset_thunk),
        (DESTROY_FROM_PACKET, &B_DESTROY_FROM_PACKET, destroy_from_packet_thunk),
        (SEND_NEW_OWNER, &B_SEND_NEW_OWNER, send_new_owner_thunk),
        (OBJ_UNREGISTER, &B_OBJ_UNREGISTER, obj_unregister_thunk),
        (DEACTIVATE_SEND, &B_DEACTIVATE_SEND, deactivate_send_thunk),
        (REGISTER_LOCAL, &B_REGISTER_LOCAL, register_local_thunk),
        (REGISTER_FROM_CREATE, &B_REGISTER_FROM_CREATE, register_from_create_thunk),
        (SEND_JOIN_REQ, &B_SEND_JOIN_REQ, send_join_req_thunk),
    ];
    let mut ok = 0;
    for &(id, body, thunk) in &list {
        if replace_pinned(P3S[id].name, P3S[id].addr, body, thunk as usize) {
            ok += 1;
        }
    }
    ok
}

fn summary3() -> Vec<String> {
    P3S.iter()
        .filter_map(|p| {
            let c = p.calls.load(Ordering::Relaxed);
            (c > 0).then(|| {
                let b: Vec<String> = p.br.iter().enumerate().filter_map(|(i, x)| {
                    let v = x.load(Ordering::Relaxed);
                    (v > 0).then(|| format!("b{i}={v}"))
                }).collect();
                format!("{} {c} [{}]", p.name, b.join(" "))
            })
        })
        .collect()
}


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
        (POOL_BY_ID, &B_POOL_BY_ID, &B_POOL_BY_ID_CALLS, pool_by_id_thunk),
        (POOL_STATE2, &B_POOL_STATE2, &B_POOL_STATE2_CALLS, pool_state2_thunk),
        (POOL_STATE1, &B_POOL_STATE1, &B_POOL_STATE1_CALLS, pool_state1_thunk),
        (SEND_JOIN_NEW_SESSION, &B_SEND_JOIN_NEW_SESSION, &B_SEND_JOIN_NEW_SESSION_CALLS, join_new_session_thunk),
        (SEND_JOIN_SUCCESS, &B_SEND_JOIN_SUCCESS, &B_SEND_JOIN_SUCCESS_CALLS, join_success_thunk),
        (SEND_JOIN_FAILED, &B_SEND_JOIN_FAILED, &B_SEND_JOIN_FAILED_CALLS, join_failed_thunk),
        (SEND_JOIN_COMPLETE, &B_SEND_JOIN_COMPLETE, &B_SEND_JOIN_COMPLETE_CALLS, join_complete_thunk),
        (SEND_MP_INFO, &B_SEND_MP_INFO, &B_SEND_MP_INFO_CALLS, send_mp_info_thunk),
        (SEND_UPD_STATE_SUB3, &B_SEND_UPD_STATE_SUB3, &B_SEND_UPD_STATE_SUB3_CALLS, send_upd_state_sub3_thunk),
        (SET_PHASE_BROADCAST, &B_SET_PHASE_BROADCAST, &B_SET_PHASE_BROADCAST_CALLS, set_phase_broadcast_thunk),
        (SEND_JOIN_DENY, &B_SEND_JOIN_DENY, &B_SEND_JOIN_DENY_CALLS, send_join_deny_thunk),
        (SEND_UPD_CONN, &B_SEND_UPD_CONN, &B_SEND_UPD_CONN_CALLS, send_upd_conn_thunk),
        (SEND_TO_LIST_OR_REMOTES, &B_SEND_TO_LIST_OR_REMOTES, &B_SEND_TO_LIST_OR_REMOTES_CALLS, send_to_list_or_remotes_thunk),
        (RECENT_IDS_PUSH, &B_RECENT_IDS_PUSH, &B_RECENT_IDS_PUSH_CALLS, recent_ids_push_thunk),
        (ACK_WINDOW_PUSH, &B_ACK_WINDOW_PUSH, &B_ACK_WINDOW_PUSH_CALLS, ack_window_push_thunk),
        (ACK_RECORD, &B_ACK_RECORD, &B_ACK_RECORD_CALLS, ack_record_thunk),
        (CHECK_TIMEOUT, &B_CHECK_TIMEOUT, &B_CHECK_TIMEOUT_CALLS, check_timeout_thunk),
        (TIMEOUT_EXPIRED, &B_TIMEOUT_EXPIRED, &B_TIMEOUT_EXPIRED_CALLS, timeout_expired_thunk),
        (TIMEOUT_ARM_10S, &B_TIMEOUT_ARM_10S, &B_TIMEOUT_ARM_10S_CALLS, timeout_arm_10s_thunk),
        (RATE_METER_INIT, &B_RATE_METER_INIT, &B_RATE_METER_INIT_CALLS, rate_meter_init_thunk),
        (RATE_METER_REMAINING, &B_RATE_METER_REMAINING, &B_RATE_METER_REMAINING_CALLS, rate_meter_remaining_thunk),
        (CONN_NOTE_RECEIVED, &B_CONN_NOTE_RECEIVED, &B_CONN_NOTE_RECEIVED_CALLS, conn_note_received_thunk),
        (ACK_WINDOW_MARK_ALL, &B_ACK_WINDOW_MARK_ALL, &B_ACK_WINDOW_MARK_ALL_CALLS, ack_window_mark_all_thunk),
        (RESET_ACK_WINDOW, &B_RESET_ACK_WINDOW, &B_RESET_ACK_WINDOW_CALLS, reset_ack_window_thunk),
        (CLOCK_PREDICTED_MS, &B_CLOCK_PREDICTED_MS, &B_CLOCK_PREDICTED_MS_CALLS, clock_predicted_ms_thunk),
        (CLOCK_ADVANCE, &B_CLOCK_ADVANCE, &B_CLOCK_ADVANCE_CALLS, clock_advance_thunk),
        (OUT_OF_PHASE_MASK, &B_OUT_OF_PHASE_MASK, &B_OUT_OF_PHASE_MASK_CALLS, out_of_phase_mask_thunk),
        (PEERS_PHASE_EQUAL, &B_PEERS_PHASE_EQUAL, &B_PEERS_PHASE_EQUAL_CALLS, peers_phase_equal_thunk),
        (ALL_CONNS_STATE_EQ, &B_ALL_CONNS_STATE_EQ, &B_ALL_CONNS_STATE_EQ_CALLS, all_conns_state_eq_thunk),
        (ALL_CONNS_STATE_GE, &B_ALL_CONNS_STATE_GE, &B_ALL_CONNS_STATE_GE_CALLS, all_conns_state_ge_thunk),
        (LOBBY_ANY_CONN_STATE_4, &B_LOBBY_ANY_CONN_STATE_4, &B_LOBBY_ANY_CONN_STATE_4_CALLS, lobby_any_conn_state_4_thunk),
        (STATE_IN_SET_B, &B_STATE_IN_SET_B, &B_STATE_IN_SET_B_CALLS, state_in_set_b_thunk),
        (STATE_IS_8, &B_STATE_IS_8, &B_STATE_IS_8_CALLS, state_is_8_thunk),
        (LOCAL_CONN_ID, &B_LOCAL_CONN_ID, &B_LOCAL_CONN_ID_CALLS, local_conn_id_thunk),
        (ID_TO_HUMAN, &B_ID_TO_HUMAN, &B_ID_TO_HUMAN_CALLS, id_to_human_thunk),
        (MAP_SET, &B_MAP_SET, &B_MAP_SET_CALLS, map_set_thunk),
        (FIND_RESERVATION, &B_FIND_RESERVATION, &B_FIND_RESERVATION_CALLS, find_reservation_thunk),
        (REMOVE_RESERVATION, &B_REMOVE_RESERVATION, &B_REMOVE_RESERVATION_CALLS, remove_reservation_thunk),
        (GUID_IN_ARRAY, &B_GUID_IN_ARRAY, &B_GUID_IN_ARRAY_CALLS, guid_in_array_thunk),
        (FIND_BEST_CONN, &B_FIND_BEST_CONN, &B_FIND_BEST_CONN_CALLS, find_best_conn_thunk),
        (SESSION_INFO_BASE_INIT, &B_SESSION_INFO_BASE_INIT, &B_SESSION_INFO_BASE_INIT_CALLS, session_info_base_init_thunk),
        (BYTE_NC_INIT, &B_BYTE_NC_INIT, &B_BYTE_NC_INIT_CALLS, byte_nc_init_thunk),
        (BYTE_NC_INIT_BITS, &B_BYTE_NC_INIT_BITS, &B_BYTE_NC_INIT_BITS_CALLS, byte_nc_init_bits_thunk),
        (BIT_NC_INIT, &B_BIT_NC_INIT, &B_BIT_NC_INIT_CALLS, bit_nc_init_thunk),
        (BIT_NC_INIT_BITS, &B_BIT_NC_INIT_BITS, &B_BIT_NC_INIT_BITS_CALLS, bit_nc_init_bits_thunk),
        (FLOAT_COMP_INIT, &B_FLOAT_COMP_INIT, &B_FLOAT_COMP_INIT_CALLS, float_comp_init_thunk),
        (FLOAT_COMP_INIT_COPY, &B_FLOAT_COMP_INIT_COPY, &B_FLOAT_COMP_INIT_COPY_CALLS, float_comp_init_copy_thunk),
        (FLOAT_COMP_COPY, &B_FLOAT_COMP_COPY, &B_FLOAT_COMP_COPY_CALLS, float_comp_copy_thunk),
        (VECTOR3_COMP_CTOR, &B_VECTOR3_COMP_CTOR, &B_VECTOR3_COMP_CTOR_CALLS, vector3_comp_ctor_thunk),
        (VECTOR2_COMP_CTOR, &B_VECTOR2_COMP_CTOR, &B_VECTOR2_COMP_CTOR_CALLS, vector2_comp_ctor_thunk),
        (VECTOR3_UNCOMP_CTOR, &B_VECTOR3_UNCOMP_CTOR, &B_VECTOR3_UNCOMP_CTOR_CALLS, vector3_uncomp_ctor_thunk),
        (HEADING_COMP_CTOR, &B_HEADING_COMP_CTOR, &B_HEADING_COMP_CTOR_CALLS, heading_comp_ctor_thunk),
        (OWNERSHIP_PENDING, &B_OWNERSHIP_PENDING, &B_OWNERSHIP_PENDING_CALLS, ownership_pending_thunk),
        (CONN_OBJECT_LIST_ADD, &B_CONN_OBJECT_LIST_ADD, &B_CONN_OBJECT_LIST_ADD_CALLS, conn_object_list_add_thunk),
        (CONN_OBJECT_LIST_REMOVE, &B_CONN_OBJECT_LIST_REMOVE, &B_CONN_OBJECT_LIST_REMOVE_CALLS, conn_object_list_remove_thunk),
        (GUID_ID_EQUAL, &B_GUID_ID_EQUAL, &B_GUID_ID_EQUAL_CALLS, guid_id_equal_thunk),
        (GUID_ENTRY_READ, &B_GUID_ENTRY_READ, &B_GUID_ENTRY_READ_CALLS, guid_entry_read_thunk),
        (ID_TO_OBJECT_AC, &B_ID_TO_OBJECT_AC, &B_ID_TO_OBJECT_AC_CALLS, id_to_object_ac_thunk),
        (CAPTURE_SNAPSHOT, &B_CAPTURE_SNAPSHOT, &B_CAPTURE_SNAPSHOT_CALLS, capture_snapshot_thunk),
        (READ_STATE_DELTA, &B_READ_STATE_DELTA, &B_READ_STATE_DELTA_CALLS, read_state_delta_thunk),
        (GET_SEND_PRIORITY, &B_GET_SEND_PRIORITY, &B_GET_SEND_PRIORITY_CALLS, get_send_priority_thunk),
        (GET_DIRTY_FIELDS, &B_GET_DIRTY_FIELDS, &B_GET_DIRTY_FIELDS_CALLS, get_dirty_fields_thunk),
        (ON_STATE_SENT, &B_ON_STATE_SENT, &B_ON_STATE_SENT_CALLS, on_state_sent_thunk),
        (CAN_MIGRATE, &B_CAN_MIGRATE, &B_CAN_MIGRATE_CALLS, can_migrate_thunk),
        (REQUEST_OWNERSHIP, &B_REQUEST_OWNERSHIP, &B_REQUEST_OWNERSHIP_CALLS, request_ownership_thunk),
    ];
    // The shadow copies first: replacing patches the entry.
    for &(id, body, call_offsets, _) in &list {
        let port = &PORTS[id];
        match unsafe { detour::relocate(port.addr, body, call_offsets) } {
            Ok(copy) => port.original.store(copy, Ordering::Relaxed),
            Err(e) => log(&format!("port_net: {}: no shadow copy: {e:?}", port.name)),
        }
    }
    round2_patch_copies();
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
    let ok3 = install3();
    log(&format!("port_net: replaced {}/{} with Rust", ok + ok3, COUNT + COUNT3));
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
    let mut parts = parts;
    parts.extend(summary3());
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

const B_POOL_BY_ID: [u8; 210] = [
    0x53, 0x55, 0x8B, 0x2D, 0xCC, 0x80, 0xDB, 0x00, 0x56, 0x57, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF,
    0xD5, 0x8B, 0x1D, 0xD0, 0x80, 0xDB, 0x00, 0x83, 0x05, 0x68, 0x89, 0xA1, 0x02, 0x01, 0x68, 0x44,
    0x89, 0xA1, 0x02, 0xFF, 0xD3, 0x83, 0x3D, 0x40, 0x89, 0xA1, 0x02, 0x00, 0x74, 0x30, 0x8B, 0x35,
    0x3C, 0x89, 0xA1, 0x02, 0xA1, 0x38, 0x89, 0xA1, 0x02, 0x69, 0xF6, 0x44, 0x10, 0x00, 0x00, 0x03,
    0x30, 0x74, 0x1B, 0x8A, 0x4C, 0x24, 0x14, 0x38, 0x4E, 0x04, 0x74, 0x4C, 0x8B, 0xC6, 0xBF, 0x38,
    0x89, 0xA1, 0x02, 0xE8, 0xF8, 0x3B, 0xE8, 0xFF, 0x8B, 0xF0, 0x85, 0xF6, 0x75, 0xE5, 0x68, 0x44,
    0x89, 0xA1, 0x02, 0xFF, 0xD5, 0x83, 0x05, 0x68, 0x89, 0xA1, 0x02, 0xFF, 0x8B, 0x15, 0x68, 0x89,
    0xA1, 0x02, 0x75, 0x16, 0xA1, 0x6C, 0x89, 0xA1, 0x02, 0x85, 0xC0, 0x7E, 0x0D, 0x8B, 0x0D, 0x60,
    0x89, 0xA1, 0x02, 0x51, 0xFF, 0x15, 0xDC, 0x81, 0xDB, 0x00, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF,
    0xD3, 0x33, 0xC0, 0x5F, 0x5E, 0x5D, 0x5B, 0xC3, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF, 0xD5, 0x83,
    0x05, 0x68, 0x89, 0xA1, 0x02, 0xFF, 0x8B, 0x15, 0x68, 0x89, 0xA1, 0x02, 0x75, 0x16, 0xA1, 0x6C,
    0x89, 0xA1, 0x02, 0x85, 0xC0, 0x7E, 0x0D, 0x8B, 0x0D, 0x60, 0x89, 0xA1, 0x02, 0x51, 0xFF, 0x15,
    0xDC, 0x81, 0xDB, 0x00, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF, 0xD3, 0x5F, 0x8B, 0xC6, 0x5E, 0x5D,
    0x5B, 0xC3,
];
const B_POOL_BY_ID_CALLS: [usize; 1] = [0x53];
const B_POOL_STATE2: [u8; 210] = [
    0x53, 0x8B, 0x1D, 0xCC, 0x80, 0xDB, 0x00, 0x55, 0x56, 0x57, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF,
    0xD3, 0x8B, 0x2D, 0xD0, 0x80, 0xDB, 0x00, 0x83, 0x05, 0x68, 0x89, 0xA1, 0x02, 0x01, 0x68, 0x44,
    0x89, 0xA1, 0x02, 0xFF, 0xD5, 0x83, 0x3D, 0x40, 0x89, 0xA1, 0x02, 0x00, 0x74, 0x30, 0x8B, 0x35,
    0x3C, 0x89, 0xA1, 0x02, 0xA1, 0x38, 0x89, 0xA1, 0x02, 0x69, 0xF6, 0x44, 0x10, 0x00, 0x00, 0x03,
    0x30, 0x74, 0x1B, 0x8B, 0x4E, 0x0C, 0x83, 0x79, 0x30, 0x02, 0x74, 0x4C, 0x8B, 0xC6, 0xBF, 0x38,
    0x89, 0xA1, 0x02, 0xE8, 0x38, 0x3A, 0xE8, 0xFF, 0x8B, 0xF0, 0x85, 0xF6, 0x75, 0xE5, 0x68, 0x44,
    0x89, 0xA1, 0x02, 0xFF, 0xD3, 0x83, 0x05, 0x68, 0x89, 0xA1, 0x02, 0xFF, 0x8B, 0x15, 0x68, 0x89,
    0xA1, 0x02, 0x75, 0x16, 0xA1, 0x6C, 0x89, 0xA1, 0x02, 0x85, 0xC0, 0x7E, 0x0D, 0x8B, 0x0D, 0x60,
    0x89, 0xA1, 0x02, 0x51, 0xFF, 0x15, 0xDC, 0x81, 0xDB, 0x00, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF,
    0xD5, 0x33, 0xC0, 0x5F, 0x5E, 0x5D, 0x5B, 0xC3, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF, 0xD3, 0x83,
    0x05, 0x68, 0x89, 0xA1, 0x02, 0xFF, 0x8B, 0x15, 0x68, 0x89, 0xA1, 0x02, 0x75, 0x16, 0xA1, 0x6C,
    0x89, 0xA1, 0x02, 0x85, 0xC0, 0x7E, 0x0D, 0x8B, 0x0D, 0x60, 0x89, 0xA1, 0x02, 0x51, 0xFF, 0x15,
    0xDC, 0x81, 0xDB, 0x00, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF, 0xD5, 0x5F, 0x8B, 0xC6, 0x5E, 0x5D,
    0x5B, 0xC3,
];
const B_POOL_STATE2_CALLS: [usize; 1] = [0x53];
const B_POOL_STATE1: [u8; 210] = [
    0x53, 0x8B, 0x1D, 0xCC, 0x80, 0xDB, 0x00, 0x55, 0x56, 0x57, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF,
    0xD3, 0x8B, 0x2D, 0xD0, 0x80, 0xDB, 0x00, 0x83, 0x05, 0x68, 0x89, 0xA1, 0x02, 0x01, 0x68, 0x44,
    0x89, 0xA1, 0x02, 0xFF, 0xD5, 0x83, 0x3D, 0x40, 0x89, 0xA1, 0x02, 0x00, 0x74, 0x30, 0x8B, 0x35,
    0x3C, 0x89, 0xA1, 0x02, 0xA1, 0x38, 0x89, 0xA1, 0x02, 0x69, 0xF6, 0x44, 0x10, 0x00, 0x00, 0x03,
    0x30, 0x74, 0x1B, 0x8B, 0x4E, 0x0C, 0x83, 0x79, 0x30, 0x01, 0x74, 0x4C, 0x8B, 0xC6, 0xBF, 0x38,
    0x89, 0xA1, 0x02, 0xE8, 0x58, 0x39, 0xE8, 0xFF, 0x8B, 0xF0, 0x85, 0xF6, 0x75, 0xE5, 0x68, 0x44,
    0x89, 0xA1, 0x02, 0xFF, 0xD3, 0x83, 0x05, 0x68, 0x89, 0xA1, 0x02, 0xFF, 0x8B, 0x15, 0x68, 0x89,
    0xA1, 0x02, 0x75, 0x16, 0xA1, 0x6C, 0x89, 0xA1, 0x02, 0x85, 0xC0, 0x7E, 0x0D, 0x8B, 0x0D, 0x60,
    0x89, 0xA1, 0x02, 0x51, 0xFF, 0x15, 0xDC, 0x81, 0xDB, 0x00, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF,
    0xD5, 0x33, 0xC0, 0x5F, 0x5E, 0x5D, 0x5B, 0xC3, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF, 0xD3, 0x83,
    0x05, 0x68, 0x89, 0xA1, 0x02, 0xFF, 0x8B, 0x15, 0x68, 0x89, 0xA1, 0x02, 0x75, 0x16, 0xA1, 0x6C,
    0x89, 0xA1, 0x02, 0x85, 0xC0, 0x7E, 0x0D, 0x8B, 0x0D, 0x60, 0x89, 0xA1, 0x02, 0x51, 0xFF, 0x15,
    0xDC, 0x81, 0xDB, 0x00, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF, 0xD5, 0x5F, 0x8B, 0xC6, 0x5E, 0x5D,
    0x5B, 0xC3,
];
const B_POOL_STATE1_CALLS: [usize; 1] = [0x53];
const B_SEND_JOIN_NEW_SESSION: [u8; 135] = [
    0x81, 0xEC, 0x24, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x20, 0x05, 0x00, 0x00, 0x80, 0x3D, 0x94, 0xBF, 0xE8, 0x00, 0x00, 0x8B, 0x84, 0x24, 0x28, 0x05,
    0x00, 0x00, 0x56, 0x8B, 0xF1, 0xC6, 0x44, 0x24, 0x08, 0x05, 0xC7, 0x44, 0x24, 0x04, 0x03, 0x00,
    0x00, 0x00, 0x74, 0x08, 0xC7, 0x44, 0x24, 0x04, 0x04, 0x00, 0x00, 0x00, 0x8B, 0x48, 0x0C, 0x8B,
    0x01, 0x8B, 0x40, 0x20, 0x8D, 0x54, 0x24, 0x04, 0x52, 0x8D, 0x54, 0x24, 0x0C, 0x52, 0xFF, 0xD0,
    0x8B, 0x44, 0x24, 0x04, 0x6A, 0x00, 0x0F, 0xB7, 0xD0, 0x8D, 0x48, 0xFD, 0x6A, 0x00, 0x52, 0x8D,
    0x44, 0x24, 0x14, 0x50, 0x66, 0x89, 0x4C, 0x24, 0x19, 0xE8, 0xA2, 0xE1, 0xFF, 0xFF, 0x8B, 0x8C,
    0x24, 0x34, 0x05, 0x00, 0x00, 0x83, 0xC4, 0x10, 0x5E, 0x33, 0xCC, 0xE8, 0x6F, 0xFB, 0x47, 0x00,
    0x81, 0xC4, 0x24, 0x05, 0x00, 0x00, 0xC3,
];
const B_SEND_JOIN_NEW_SESSION_CALLS: [usize; 2] = [0x69, 0x7b];
const B_SEND_JOIN_SUCCESS: [u8; 135] = [
    0x81, 0xEC, 0x24, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x20, 0x05, 0x00, 0x00, 0x80, 0x3D, 0xAC, 0xBF, 0xE8, 0x00, 0x00, 0x8B, 0x84, 0x24, 0x28, 0x05,
    0x00, 0x00, 0x56, 0x8B, 0xF1, 0xC6, 0x44, 0x24, 0x08, 0x06, 0xC7, 0x44, 0x24, 0x04, 0x03, 0x00,
    0x00, 0x00, 0x74, 0x08, 0xC7, 0x44, 0x24, 0x04, 0x04, 0x00, 0x00, 0x00, 0x8B, 0x48, 0x0C, 0x8B,
    0x01, 0x8B, 0x40, 0x28, 0x8D, 0x54, 0x24, 0x04, 0x52, 0x8D, 0x54, 0x24, 0x0C, 0x52, 0xFF, 0xD0,
    0x8B, 0x44, 0x24, 0x04, 0x6A, 0x00, 0x0F, 0xB7, 0xD0, 0x8D, 0x48, 0xFD, 0x6A, 0x00, 0x52, 0x8D,
    0x44, 0x24, 0x14, 0x50, 0x66, 0x89, 0x4C, 0x24, 0x19, 0xE8, 0x12, 0xE0, 0xFF, 0xFF, 0x8B, 0x8C,
    0x24, 0x34, 0x05, 0x00, 0x00, 0x83, 0xC4, 0x10, 0x5E, 0x33, 0xCC, 0xE8, 0xDF, 0xF9, 0x47, 0x00,
    0x81, 0xC4, 0x24, 0x05, 0x00, 0x00, 0xC3,
];
const B_SEND_JOIN_SUCCESS_CALLS: [usize; 2] = [0x69, 0x7b];
const B_SEND_JOIN_FAILED: [u8; 135] = [
    0x81, 0xEC, 0x24, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x20, 0x05, 0x00, 0x00, 0x80, 0x3D, 0xC4, 0xBF, 0xE8, 0x00, 0x00, 0x8B, 0x84, 0x24, 0x28, 0x05,
    0x00, 0x00, 0x56, 0x8B, 0xF1, 0xC6, 0x44, 0x24, 0x08, 0x07, 0xC7, 0x44, 0x24, 0x04, 0x03, 0x00,
    0x00, 0x00, 0x74, 0x08, 0xC7, 0x44, 0x24, 0x04, 0x04, 0x00, 0x00, 0x00, 0x8B, 0x48, 0x0C, 0x8B,
    0x01, 0x8B, 0x40, 0x28, 0x8D, 0x54, 0x24, 0x04, 0x52, 0x8D, 0x54, 0x24, 0x0C, 0x52, 0xFF, 0xD0,
    0x8B, 0x44, 0x24, 0x04, 0x6A, 0x00, 0x0F, 0xB7, 0xD0, 0x8D, 0x48, 0xFD, 0x6A, 0x00, 0x52, 0x8D,
    0x44, 0x24, 0x14, 0x50, 0x66, 0x89, 0x4C, 0x24, 0x19, 0xE8, 0xC2, 0xDE, 0xFF, 0xFF, 0x8B, 0x8C,
    0x24, 0x34, 0x05, 0x00, 0x00, 0x83, 0xC4, 0x10, 0x5E, 0x33, 0xCC, 0xE8, 0x8F, 0xF8, 0x47, 0x00,
    0x81, 0xC4, 0x24, 0x05, 0x00, 0x00, 0xC3,
];
const B_SEND_JOIN_FAILED_CALLS: [usize; 2] = [0x69, 0x7b];
const B_SEND_JOIN_COMPLETE: [u8; 135] = [
    0x81, 0xEC, 0x24, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x20, 0x05, 0x00, 0x00, 0x80, 0x3D, 0xDC, 0xBF, 0xE8, 0x00, 0x00, 0x8B, 0x84, 0x24, 0x28, 0x05,
    0x00, 0x00, 0x56, 0x8B, 0xF1, 0xC6, 0x44, 0x24, 0x08, 0x08, 0xC7, 0x44, 0x24, 0x04, 0x03, 0x00,
    0x00, 0x00, 0x74, 0x08, 0xC7, 0x44, 0x24, 0x04, 0x04, 0x00, 0x00, 0x00, 0x8B, 0x48, 0x0C, 0x8B,
    0x01, 0x8B, 0x40, 0x28, 0x8D, 0x54, 0x24, 0x04, 0x52, 0x8D, 0x54, 0x24, 0x0C, 0x52, 0xFF, 0xD0,
    0x8B, 0x44, 0x24, 0x04, 0x6A, 0x00, 0x0F, 0xB7, 0xD0, 0x8D, 0x48, 0xFD, 0x6A, 0x00, 0x52, 0x8D,
    0x44, 0x24, 0x14, 0x50, 0x66, 0x89, 0x4C, 0x24, 0x19, 0xE8, 0x62, 0xDD, 0xFF, 0xFF, 0x8B, 0x8C,
    0x24, 0x34, 0x05, 0x00, 0x00, 0x83, 0xC4, 0x10, 0x5E, 0x33, 0xCC, 0xE8, 0x2F, 0xF7, 0x47, 0x00,
    0x81, 0xC4, 0x24, 0x05, 0x00, 0x00, 0xC3,
];
const B_SEND_JOIN_COMPLETE_CALLS: [usize; 2] = [0x69, 0x7b];
const B_SEND_MP_INFO: [u8; 156] = [
    0x81, 0xEC, 0x24, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x20, 0x05, 0x00, 0x00, 0x80, 0x3D, 0x94, 0xBA, 0xE8, 0x00, 0x00, 0x53, 0x57, 0xC6, 0x44, 0x24,
    0x0C, 0x5B, 0xB8, 0x03, 0x00, 0x00, 0x00, 0x74, 0x05, 0xB8, 0x04, 0x00, 0x00, 0x00, 0x0F, 0xB6,
    0x59, 0x44, 0x8B, 0xB9, 0xAC, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xD0, 0x88, 0x5C, 0x14, 0x0C, 0x0F,
    0xB6, 0x59, 0x45, 0x83, 0xC0, 0x01, 0x0F, 0xB7, 0xD0, 0x88, 0x5C, 0x14, 0x0C, 0x83, 0xC0, 0x01,
    0x8D, 0x50, 0xFD, 0x0F, 0xB7, 0xC0, 0x66, 0x89, 0x54, 0x24, 0x0D, 0x50, 0x8D, 0x54, 0x24, 0x10,
    0x52, 0xE8, 0xEA, 0xB8, 0x01, 0x00, 0x8B, 0x8C, 0x24, 0x30, 0x05, 0x00, 0x00, 0x83, 0xC4, 0x08,
    0x5F, 0x5B, 0x33, 0xCC, 0xE8, 0xE6, 0xD1, 0x49, 0x00, 0x81, 0xC4, 0x24, 0x05, 0x00, 0x00, 0xC3,
    0x8B, 0x44, 0x24, 0x04, 0x8A, 0x48, 0x01, 0x8B, 0x54, 0x24, 0x08, 0x8A, 0x00, 0x56, 0x8B, 0x72,
    0x04, 0x88, 0x46, 0x44, 0x8B, 0x52, 0x04, 0x88, 0x4A, 0x45, 0x5E, 0xC3,
];
const B_SEND_MP_INFO_CALLS: [usize; 2] = [0x61, 0x74];
const B_SEND_UPD_STATE_SUB3: [u8; 114] = [
    0x81, 0xEC, 0x20, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x1C, 0x05, 0x00, 0x00, 0x56, 0x8B, 0xF1, 0x80, 0x3E, 0x00, 0x75, 0x40, 0x80, 0x3D, 0x1C, 0xC2,
    0xE8, 0x00, 0x00, 0xC6, 0x44, 0x24, 0x04, 0x20, 0xB8, 0x03, 0x00, 0x00, 0x00, 0x74, 0x05, 0xB8,
    0x04, 0x00, 0x00, 0x00, 0x0F, 0xB7, 0xC8, 0x83, 0xC0, 0x01, 0x8D, 0x50, 0xFD, 0x6A, 0x00, 0x0F,
    0xB7, 0xC0, 0xC6, 0x44, 0x0C, 0x08, 0x03, 0x6A, 0x00, 0x50, 0x8D, 0x4C, 0x24, 0x10, 0x51, 0x66,
    0x89, 0x54, 0x24, 0x15, 0xE8, 0xA7, 0xB8, 0x01, 0x00, 0x83, 0xC4, 0x10, 0x8B, 0x8C, 0x24, 0x20,
    0x05, 0x00, 0x00, 0x5E, 0x33, 0xCC, 0xE8, 0x74, 0xD2, 0x49, 0x00, 0x81, 0xC4, 0x20, 0x05, 0x00,
    0x00, 0xC3,
];
const B_SEND_UPD_STATE_SUB3_CALLS: [usize; 2] = [0x54, 0x66];
const B_SET_PHASE_BROADCAST: [u8; 134] = [
    0x81, 0xEC, 0x24, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x20, 0x05, 0x00, 0x00, 0x3B, 0x51, 0x40, 0x56, 0x57, 0x74, 0x54, 0x80, 0x39, 0x00, 0x89, 0x51,
    0x40, 0x74, 0x4C, 0x80, 0x3D, 0x1C, 0xC2, 0xE8, 0x00, 0x00, 0xC6, 0x44, 0x24, 0x0C, 0x20, 0xB8,
    0x03, 0x00, 0x00, 0x00, 0x74, 0x05, 0xB8, 0x04, 0x00, 0x00, 0x00, 0x8B, 0xB9, 0xAC, 0x01, 0x00,
    0x00, 0x0F, 0xB7, 0xF0, 0x83, 0xC0, 0x01, 0xC6, 0x44, 0x34, 0x0C, 0x01, 0x0F, 0xB7, 0xF0, 0x89,
    0x54, 0x34, 0x0C, 0x83, 0xC0, 0x04, 0x8D, 0x50, 0xFD, 0x0F, 0xB7, 0xC0, 0x66, 0x89, 0x54, 0x24,
    0x0D, 0x50, 0x8D, 0x54, 0x24, 0x10, 0x52, 0xE8, 0x84, 0xBA, 0x01, 0x00, 0x83, 0xC4, 0x08, 0x8B,
    0x8C, 0x24, 0x28, 0x05, 0x00, 0x00, 0x5F, 0x5E, 0x33, 0xCC, 0xE8, 0x80, 0xD3, 0x49, 0x00, 0x81,
    0xC4, 0x24, 0x05, 0x00, 0x00, 0xC3,
];
const B_SET_PHASE_BROADCAST_CALLS: [usize; 2] = [0x67, 0x7a];
const B_SEND_JOIN_DENY: [u8; 120] = [
    0x81, 0xEC, 0x24, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x20, 0x05, 0x00, 0x00, 0x80, 0x3D, 0x7C, 0xBF, 0xE8, 0x00, 0x00, 0x53, 0xC6, 0x44, 0x24, 0x08,
    0x04, 0xB8, 0x03, 0x00, 0x00, 0x00, 0x74, 0x05, 0xB8, 0x04, 0x00, 0x00, 0x00, 0x8A, 0x9C, 0x24,
    0x2C, 0x05, 0x00, 0x00, 0x0F, 0xB7, 0xD0, 0xC7, 0x44, 0x14, 0x08, 0xC9, 0x00, 0x00, 0x00, 0x83,
    0xC0, 0x04, 0x0F, 0xB7, 0xD0, 0x83, 0xC0, 0x01, 0x88, 0x5C, 0x14, 0x08, 0x50, 0x8D, 0x50, 0xFD,
    0x51, 0x8D, 0x4C, 0x24, 0x10, 0x66, 0x89, 0x54, 0x24, 0x11, 0xE8, 0xD1, 0xD7, 0xFF, 0xFF, 0x8B,
    0x8C, 0x24, 0x2C, 0x05, 0x00, 0x00, 0x83, 0xC4, 0x08, 0x5B, 0x33, 0xCC, 0xE8, 0x7E, 0xF0, 0x47,
    0x00, 0x81, 0xC4, 0x24, 0x05, 0x00, 0x00, 0xC3,
];
const B_SEND_JOIN_DENY_CALLS: [usize; 2] = [0x5a, 0x6c];
const B_SEND_UPD_CONN: [u8; 208] = [
    0x81, 0xEC, 0x24, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x20, 0x05, 0x00, 0x00, 0x80, 0x3D, 0x24, 0xC0, 0xE8, 0x00, 0x00, 0x55, 0x56, 0xC6, 0x44, 0x24,
    0x0C, 0x0B, 0xB8, 0x03, 0x00, 0x00, 0x00, 0x74, 0x05, 0xB8, 0x04, 0x00, 0x00, 0x00, 0x8A, 0x97,
    0xB0, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xC8, 0x83, 0xC0, 0x01, 0x89, 0x44, 0x24, 0x08, 0x8D, 0x44,
    0x24, 0x08, 0x88, 0x54, 0x0C, 0x0C, 0x50, 0x8B, 0xC7, 0x8D, 0x4C, 0x24, 0x10, 0xE8, 0x1E, 0xEC,
    0xFF, 0xFF, 0x8B, 0x97, 0xAC, 0x01, 0x00, 0x00, 0x8B, 0x6C, 0x24, 0x0C, 0x8B, 0xB2, 0x00, 0x04,
    0x00, 0x00, 0x83, 0xC4, 0x04, 0x85, 0xF6, 0x8D, 0x4D, 0xFD, 0x66, 0x89, 0x4C, 0x24, 0x0D, 0x74,
    0x48, 0x80, 0xBE, 0xB8, 0x01, 0x00, 0x00, 0x01, 0x75, 0x27, 0x3B, 0xF7, 0x74, 0x23, 0x8B, 0x86,
    0xAC, 0x01, 0x00, 0x00, 0x3B, 0xB0, 0x08, 0x04, 0x00, 0x00, 0x74, 0x15, 0x6A, 0x00, 0x0F, 0xB7,
    0xCD, 0x6A, 0x00, 0x51, 0x8D, 0x54, 0x24, 0x18, 0x52, 0xE8, 0x82, 0xDA, 0xFF, 0xFF, 0x83, 0xC4,
    0x10, 0x8B, 0xB6, 0xCC, 0x22, 0x00, 0x00, 0x8B, 0x87, 0xAC, 0x01, 0x00, 0x00, 0x3B, 0xB0, 0x00,
    0x04, 0x00, 0x00, 0x74, 0x04, 0x85, 0xF6, 0x75, 0xB8, 0x8B, 0x8C, 0x24, 0x28, 0x05, 0x00, 0x00,
    0x5E, 0x5D, 0x33, 0xCC, 0xE8, 0x36, 0xF4, 0x47, 0x00, 0x81, 0xC4, 0x24, 0x05, 0x00, 0x00, 0xC3,
];
const B_SEND_UPD_CONN_CALLS: [usize; 3] = [0x4d, 0x99, 0xc4];
const B_SEND_TO_LIST_OR_REMOTES: [u8; 121] = [
    0x83, 0xEC, 0x0C, 0xA1, 0xE4, 0x1F, 0xEB, 0x00, 0x85, 0xC0, 0x53, 0x8B, 0x5C, 0x24, 0x18, 0x55,
    0x8B, 0x6C, 0x24, 0x18, 0x56, 0x57, 0x8B, 0x3D, 0x14, 0x8C, 0x52, 0x02, 0x8B, 0x8F, 0x08, 0x04,
    0x00, 0x00, 0x89, 0x44, 0x24, 0x18, 0x89, 0x4C, 0x24, 0x14, 0x75, 0x12, 0x53, 0x55, 0xE8, 0x4D,
    0xFE, 0xFF, 0xFF, 0x83, 0xC4, 0x08, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C, 0xC3, 0x33, 0xFF,
    0x85, 0xC0, 0x7E, 0x2D, 0xA1, 0xDC, 0x1F, 0xEB, 0x00, 0x8B, 0x54, 0x24, 0x14, 0x39, 0x14, 0xB8,
    0x74, 0x16, 0x8B, 0x34, 0xB8, 0x6A, 0x00, 0x6A, 0x00, 0x53, 0x55, 0xE8, 0x50, 0xFD, 0xFF, 0xFF,
    0xA1, 0xDC, 0x1F, 0xEB, 0x00, 0x83, 0xC4, 0x10, 0x83, 0xC7, 0x01, 0x3B, 0x7C, 0x24, 0x18, 0x7C,
    0xD8, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x0C, 0xC3,
];
const B_SEND_TO_LIST_OR_REMOTES_CALLS: [usize; 2] = [0x2e, 0x5b];
const B_RECENT_IDS_PUSH: [u8; 227] = [
    0x53, 0x32, 0xD2, 0x32, 0xDB, 0x38, 0x90, 0xBA, 0x1D, 0x00, 0x00, 0x57, 0x76, 0x4C, 0x55, 0x90,
    0x80, 0xFB, 0x01, 0x0F, 0xB6, 0xCA, 0x74, 0x0C, 0x66, 0x39, 0xB4, 0x48, 0xBA, 0x1C, 0x00, 0x00,
    0x75, 0x20, 0xB3, 0x01, 0x0F, 0xB6, 0xA8, 0xBA, 0x1D, 0x00, 0x00, 0x8D, 0x79, 0x01, 0x3B, 0xFD,
    0x7D, 0x10, 0x66, 0x8B, 0xBC, 0x48, 0xBC, 0x1C, 0x00, 0x00, 0x66, 0x89, 0xBC, 0x48, 0xBA, 0x1C,
    0x00, 0x00, 0x80, 0xC2, 0x01, 0x3A, 0x90, 0xBA, 0x1D, 0x00, 0x00, 0x72, 0xC3, 0x80, 0xFB, 0x01,
    0x5D, 0x75, 0x07, 0x80, 0x80, 0xBA, 0x1D, 0x00, 0x00, 0xFF, 0x80, 0xB8, 0xBA, 0x1D, 0x00, 0x00,
    0x80, 0x75, 0x3B, 0x32, 0xD2, 0x32, 0xC9, 0x80, 0xFA, 0x7F, 0xC6, 0x80, 0xBA, 0x1D, 0x00, 0x00,
    0x7F, 0x73, 0x2B, 0xEB, 0x0B, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00, 0x8D, 0x64, 0x24, 0x00,
    0x0F, 0xB6, 0xD1, 0x66, 0x8B, 0xBC, 0x50, 0xBC, 0x1C, 0x00, 0x00, 0x80, 0xC1, 0x01, 0x66, 0x89,
    0xBC, 0x50, 0xBA, 0x1C, 0x00, 0x00, 0x3A, 0x88, 0xBA, 0x1D, 0x00, 0x00, 0x72, 0xE2, 0x0F, 0xB6,
    0x88, 0xBA, 0x1D, 0x00, 0x00, 0x66, 0x89, 0xB4, 0x48, 0xBA, 0x1C, 0x00, 0x00, 0x8A, 0x88, 0xB8,
    0x1C, 0x00, 0x00, 0x80, 0x80, 0xBA, 0x1D, 0x00, 0x00, 0x01, 0x80, 0xF9, 0x20, 0x5F, 0x5B, 0x74,
    0x21, 0x0F, 0xB6, 0xD1, 0x66, 0x89, 0xB4, 0x90, 0x38, 0x1C, 0x00, 0x00, 0x0F, 0xB6, 0x88, 0xB8,
    0x1C, 0x00, 0x00, 0xC6, 0x84, 0x88, 0x3A, 0x1C, 0x00, 0x00, 0x00, 0x80, 0x80, 0xB8, 0x1C, 0x00,
    0x00, 0x01, 0xC3,
];
const B_RECENT_IDS_PUSH_CALLS: [usize; 0] = [];
const B_ACK_WINDOW_PUSH: [u8; 128] = [
    0x33, 0xD2, 0x39, 0x91, 0xEC, 0x1D, 0x00, 0x00, 0x7C, 0x75, 0x8B, 0x81, 0xEC, 0x1D, 0x00, 0x00,
    0x3B, 0xC2, 0x7C, 0x6B, 0x56, 0x8B, 0x35, 0xAC, 0x43, 0xE8, 0x00, 0x3B, 0xC6, 0x7E, 0x0B, 0x2B,
    0xC6, 0x3D, 0x00, 0xE9, 0xA4, 0x35, 0x7C, 0x56, 0xEB, 0x0A, 0x2B, 0xF0, 0x81, 0xFE, 0x00, 0xE9,
    0xA4, 0x35, 0x7D, 0x4A, 0x83, 0x81, 0xC0, 0x15, 0x00, 0x00, 0x01, 0x8B, 0x81, 0xC0, 0x15, 0x00,
    0x00, 0x0F, 0xB7, 0xB1, 0xC4, 0x15, 0x00, 0x00, 0x83, 0xC0, 0x1C, 0xC1, 0xE0, 0x04, 0x03, 0xC1,
    0x89, 0x50, 0x04, 0x66, 0x89, 0x30, 0x88, 0x50, 0x08, 0x88, 0x50, 0x09, 0x88, 0x50, 0x0A, 0x89,
    0x50, 0x0C, 0x66, 0x83, 0x81, 0xC4, 0x15, 0x00, 0x00, 0x01, 0x0F, 0xB7, 0x81, 0xC4, 0x15, 0x00,
    0x00, 0x66, 0x3D, 0xFF, 0xFF, 0x75, 0x07, 0x66, 0x89, 0x91, 0xC4, 0x15, 0x00, 0x00, 0x5E, 0xC3,
];
const B_ACK_WINDOW_PUSH_CALLS: [usize; 0] = [];
const B_ACK_RECORD: [u8; 135] = [
    0x85, 0xC0, 0x0F, 0x84, 0x7E, 0x00, 0x00, 0x00, 0x80, 0xB8, 0xB0, 0x01, 0x00, 0x00, 0x0C, 0x73,
    0x75, 0x33, 0xC9, 0x39, 0x88, 0xC0, 0x15, 0x00, 0x00, 0x76, 0x6B, 0x8D, 0x90, 0xC0, 0x01, 0x00,
    0x00, 0x66, 0x39, 0x32, 0x74, 0x0F, 0x83, 0xC1, 0x01, 0x83, 0xC2, 0x10, 0x3B, 0x88, 0xC0, 0x15,
    0x00, 0x00, 0x72, 0xED, 0xC3, 0xC1, 0xE1, 0x04, 0x80, 0xBC, 0x01, 0xC8, 0x01, 0x00, 0x00, 0x00,
    0x8D, 0x14, 0x01, 0x75, 0x41, 0x8B, 0x0D, 0xCC, 0xA3, 0x52, 0x02, 0x2B, 0x8A, 0xC4, 0x01, 0x00,
    0x00, 0xC6, 0x82, 0xC8, 0x01, 0x00, 0x00, 0x01, 0x0F, 0xB6, 0x90, 0x34, 0x22, 0x00, 0x00, 0x83,
    0x80, 0x38, 0x22, 0x00, 0x00, 0x01, 0xD1, 0xE9, 0x89, 0x8C, 0x90, 0x0C, 0x22, 0x00, 0x00, 0x80,
    0x80, 0x34, 0x22, 0x00, 0x00, 0x01, 0x80, 0xB8, 0x34, 0x22, 0x00, 0x00, 0x0A, 0x72, 0x07, 0xC6,
    0x80, 0x34, 0x22, 0x00, 0x00, 0x00, 0xC3,
];
const B_ACK_RECORD_CALLS: [usize; 0] = [];
const B_CHECK_TIMEOUT: [u8; 115] = [
    0x80, 0x3D, 0x56, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x03, 0xB0, 0x01, 0xC3, 0x80, 0xBF, 0xB8, 0x01,
    0x00, 0x00, 0x00, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x8B, 0x8F, 0xAC, 0x01, 0x00, 0x00, 0x83, 0x79,
    0x1C, 0x0A, 0x53, 0x56, 0xB0, 0x01, 0xBB, 0x60, 0xEA, 0x00, 0x00, 0x75, 0x05, 0xBB, 0x98, 0x3A,
    0x00, 0x00, 0x8B, 0x0D, 0xCC, 0xA3, 0x52, 0x02, 0x8B, 0xB7, 0x40, 0x22, 0x00, 0x00, 0x8B, 0xD1,
    0x2B, 0xD6, 0x3B, 0xF1, 0x76, 0x03, 0x83, 0xEA, 0x01, 0x3B, 0xD3, 0x76, 0x0C, 0x5E, 0xC6, 0x87,
    0xB8, 0x01, 0x00, 0x00, 0x00, 0x32, 0xC0, 0x5B, 0xC3, 0x3B, 0xF1, 0x76, 0x07, 0x2B, 0xCE, 0x83,
    0xE9, 0x01, 0xEB, 0x02, 0x2B, 0xCE, 0x81, 0xF9, 0x98, 0x3A, 0x00, 0x00, 0x76, 0x02, 0x32, 0xC0,
    0x5E, 0x5B, 0xC3,
];
const B_CHECK_TIMEOUT_CALLS: [usize; 0] = [];
const B_TIMEOUT_EXPIRED: [u8; 53] = [
    0x80, 0x3D, 0x89, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x1C, 0xA1, 0x14, 0x16, 0xEB, 0x00, 0x85, 0xC0,
    0x7C, 0x13, 0x8B, 0x0D, 0xAC, 0x43, 0xE8, 0x00, 0x3B, 0xC1, 0x7E, 0x0C, 0x2B, 0xC1, 0x3D, 0x00,
    0xE9, 0xA4, 0x35, 0x7D, 0x0D, 0x32, 0xC0, 0xC3, 0x2B, 0xC8, 0x81, 0xF9, 0x00, 0xE9, 0xA4, 0x35,
    0x7D, 0xF3, 0xB0, 0x01, 0xC3,
];
const B_TIMEOUT_EXPIRED_CALLS: [usize; 0] = [];
const B_TIMEOUT_ARM_10S: [u8; 44] = [
    0xA1, 0xAC, 0x43, 0xE8, 0x00, 0x05, 0x10, 0x27, 0x00, 0x00, 0x3D, 0x00, 0xD2, 0x49, 0x6B, 0xC6,
    0x05, 0x89, 0x8B, 0x52, 0x02, 0x01, 0xA3, 0x14, 0x16, 0xEB, 0x00, 0x7E, 0x0A, 0x2D, 0x00, 0xD2,
    0x49, 0x6B, 0xA3, 0x14, 0x16, 0xEB, 0x00, 0xE9, 0xB4, 0xEF, 0x00, 0x00,
];
const B_TIMEOUT_ARM_10S_CALLS: [usize; 0] = [];
const B_RATE_METER_INIT: [u8; 142] = [
    0x8B, 0x44, 0x24, 0x04, 0x80, 0xFA, 0x1E, 0x0F, 0xB6, 0xD2, 0x72, 0x05, 0xBA, 0x1E, 0x00, 0x00,
    0x00, 0x81, 0xF9, 0x00, 0xE0, 0x00, 0x00, 0x88, 0x50, 0x7D, 0x66, 0x8B, 0x54, 0x24, 0x08, 0x66,
    0x89, 0x50, 0x7E, 0x76, 0x05, 0xB9, 0x00, 0xE0, 0x00, 0x00, 0x89, 0x08, 0x33, 0xC9, 0x89, 0x48,
    0x04, 0x89, 0x48, 0x08, 0x89, 0x48, 0x0C, 0x89, 0x48, 0x10, 0x89, 0x48, 0x14, 0x89, 0x48, 0x18,
    0x89, 0x48, 0x1C, 0x89, 0x48, 0x20, 0x89, 0x48, 0x24, 0x89, 0x48, 0x28, 0x89, 0x48, 0x2C, 0x89,
    0x48, 0x30, 0x89, 0x48, 0x34, 0x89, 0x48, 0x38, 0x89, 0x48, 0x3C, 0x89, 0x48, 0x40, 0x89, 0x48,
    0x44, 0x89, 0x48, 0x48, 0x89, 0x48, 0x4C, 0x89, 0x48, 0x50, 0x89, 0x48, 0x54, 0x89, 0x48, 0x58,
    0x89, 0x48, 0x5C, 0x89, 0x48, 0x60, 0x89, 0x48, 0x64, 0x89, 0x48, 0x68, 0x89, 0x48, 0x6C, 0x89,
    0x48, 0x70, 0x89, 0x48, 0x74, 0x89, 0x48, 0x78, 0x88, 0x48, 0x7C, 0xC2, 0x08, 0x00,
];
const B_RATE_METER_INIT_CALLS: [usize; 0] = [];
const B_RATE_METER_REMAINING: [u8; 52] = [
    0x0F, 0xB6, 0x42, 0x7C, 0x56, 0x0F, 0xB6, 0x72, 0x7D, 0x33, 0xC9, 0x85, 0xF6, 0x7E, 0x18, 0x90,
    0x03, 0x4C, 0x82, 0x04, 0x85, 0xC0, 0x75, 0x07, 0xB8, 0x1D, 0x00, 0x00, 0x00, 0xEB, 0x03, 0x83,
    0xE8, 0x01, 0x83, 0xEE, 0x01, 0x75, 0xE9, 0x8B, 0x02, 0x3B, 0xC8, 0x5E, 0x76, 0x03, 0x33, 0xC0,
    0xC3, 0x2B, 0xC1, 0xC3,
];
const B_RATE_METER_REMAINING_CALLS: [usize; 0] = [];
const B_CONN_NOTE_RECEIVED: [u8; 65] = [
    0x85, 0xC0, 0x74, 0x3C, 0x80, 0xB8, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x74, 0x27, 0x66, 0x81, 0xFA,
    0xFF, 0xFF, 0x74, 0x20, 0x0F, 0xB7, 0x88, 0x8E, 0x16, 0x00, 0x00, 0x66, 0x83, 0xF9, 0x64, 0x73,
    0x13, 0x0F, 0xB7, 0xC9, 0x66, 0x89, 0x94, 0x48, 0xC6, 0x15, 0x00, 0x00, 0x66, 0x83, 0x80, 0x8E,
    0x16, 0x00, 0x00, 0x01, 0x8B, 0x15, 0xCC, 0xA3, 0x52, 0x02, 0x89, 0x90, 0x40, 0x22, 0x00, 0x00,
    0xC3,
];
const B_CONN_NOTE_RECEIVED_CALLS: [usize; 0] = [];
const B_ACK_WINDOW_MARK_ALL: [u8; 38] = [
    0x85, 0xD2, 0x74, 0x21, 0x33, 0xC0, 0x39, 0x82, 0xC0, 0x15, 0x00, 0x00, 0x76, 0x17, 0x8D, 0x8A,
    0xC9, 0x01, 0x00, 0x00, 0xC6, 0x01, 0x01, 0x83, 0xC0, 0x01, 0x83, 0xC1, 0x10, 0x3B, 0x82, 0xC0,
    0x15, 0x00, 0x00, 0x72, 0xEF, 0xC3,
];
const B_ACK_WINDOW_MARK_ALL_CALLS: [usize; 0] = [];
const B_RESET_ACK_WINDOW: [u8; 100] = [
    0x33, 0xC9, 0x3B, 0xC1, 0x74, 0x5D, 0x89, 0x88, 0xC0, 0x15, 0x00, 0x00, 0x66, 0x89, 0x88, 0x8E,
    0x16, 0x00, 0x00, 0x66, 0x89, 0x88, 0x90, 0x16, 0x00, 0x00, 0x66, 0x89, 0x88, 0xC4, 0x15, 0x00,
    0x00, 0x89, 0x88, 0xC4, 0x01, 0x00, 0x00, 0x66, 0x89, 0x88, 0xC0, 0x01, 0x00, 0x00, 0x88, 0x88,
    0xC8, 0x01, 0x00, 0x00, 0x88, 0x88, 0xC9, 0x01, 0x00, 0x00, 0x88, 0x88, 0xCA, 0x01, 0x00, 0x00,
    0x89, 0x88, 0xCC, 0x01, 0x00, 0x00, 0x66, 0x83, 0x80, 0xC4, 0x15, 0x00, 0x00, 0x01, 0x0F, 0xB7,
    0x90, 0xC4, 0x15, 0x00, 0x00, 0x66, 0x81, 0xFA, 0xFF, 0xFF, 0x75, 0x07, 0x66, 0x89, 0x88, 0xC4,
    0x15, 0x00, 0x00, 0xC3,
];
const B_RESET_ACK_WINDOW_CALLS: [usize; 0] = [];
const B_CLOCK_PREDICTED_MS: [u8; 130] = [
    0x8B, 0x0D, 0x14, 0x8C, 0x52, 0x02, 0x83, 0xEC, 0x0C, 0x80, 0x3D, 0x56, 0x8B, 0x52, 0x02, 0x00,
    0x74, 0x09, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x43, 0x80, 0xB9, 0xF8, 0x03, 0x00,
    0x00, 0x00, 0x74, 0x3A, 0xD9, 0x05, 0x2C, 0x00, 0x13, 0x02, 0xDC, 0x0D, 0x00, 0xC2, 0xE5, 0x00,
    0xD9, 0x7C, 0x24, 0x02, 0x0F, 0xB7, 0x44, 0x24, 0x02, 0x0D, 0x00, 0x0C, 0x00, 0x00, 0x89, 0x44,
    0x24, 0x04, 0x8B, 0x41, 0x08, 0xD9, 0x6C, 0x24, 0x04, 0xDF, 0x7C, 0x24, 0x04, 0x8B, 0x54, 0x24,
    0x04, 0x2B, 0xC2, 0x83, 0xC0, 0x64, 0xD9, 0x6C, 0x24, 0x02, 0x83, 0xC4, 0x0C, 0xC3, 0x8B, 0x81,
    0x08, 0x04, 0x00, 0x00, 0x8B, 0x80, 0xE8, 0x1D, 0x00, 0x00, 0x85, 0xC0, 0x76, 0x0D, 0x69, 0xC0,
    0xE8, 0x03, 0x00, 0x00, 0x03, 0x41, 0x08, 0x83, 0xC4, 0x0C, 0xC3, 0x8B, 0x41, 0x08, 0x83, 0xC4,
    0x0C, 0xC3,
];
const B_CLOCK_PREDICTED_MS_CALLS: [usize; 0] = [];
const B_CLOCK_ADVANCE: [u8; 341] = [
    0x83, 0xEC, 0x14, 0x80, 0x3D, 0x56, 0x8B, 0x52, 0x02, 0x00, 0x53, 0x55, 0x56, 0x57, 0x74, 0x23,
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x74, 0x1A, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00,
    0x0F, 0x84, 0x27, 0x01, 0x00, 0x00, 0x80, 0xB8, 0xF8, 0x03, 0x00, 0x00, 0x00, 0x0F, 0x84, 0x1A,
    0x01, 0x00, 0x00, 0x8B, 0x2D, 0xCC, 0x80, 0xDB, 0x00, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF, 0xD5,
    0x8B, 0x1D, 0xD0, 0x80, 0xDB, 0x00, 0x83, 0x05, 0x68, 0x89, 0xA1, 0x02, 0x01, 0x68, 0x44, 0x89,
    0xA1, 0x02, 0xFF, 0xD3, 0x83, 0x3D, 0x40, 0x89, 0xA1, 0x02, 0x00, 0x74, 0x34, 0xA1, 0x3C, 0x89,
    0xA1, 0x02, 0x8B, 0x0D, 0x38, 0x89, 0xA1, 0x02, 0x69, 0xC0, 0x44, 0x10, 0x00, 0x00, 0x03, 0x01,
    0x74, 0x1F, 0x8B, 0x35, 0x5C, 0xB1, 0x12, 0x02, 0xEB, 0x06, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x70, 0x08, 0xBF, 0x38, 0x89, 0xA1, 0x02, 0xE8, 0x13, 0xF0, 0xE9, 0xFF, 0x85, 0xC0, 0x75,
    0xEF, 0x68, 0x44, 0x89, 0xA1, 0x02, 0xFF, 0xD5, 0x83, 0x05, 0x68, 0x89, 0xA1, 0x02, 0xFF, 0x8B,
    0x15, 0x68, 0x89, 0xA1, 0x02, 0x75, 0x16, 0xA1, 0x6C, 0x89, 0xA1, 0x02, 0x85, 0xC0, 0x7E, 0x0D,
    0x8B, 0x0D, 0x60, 0x89, 0xA1, 0x02, 0x51, 0xFF, 0x15, 0xDC, 0x81, 0xDB, 0x00, 0x68, 0x44, 0x89,
    0xA1, 0x02, 0xFF, 0xD3, 0x8B, 0x15, 0x5C, 0xB1, 0x12, 0x02, 0x01, 0x15, 0x60, 0xB1, 0x12, 0x02,
    0x80, 0x3D, 0x56, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x30, 0xD9, 0x05, 0x7C, 0x43, 0xE8, 0x00, 0xDC,
    0x0D, 0xD8, 0xC0, 0xE5, 0x00, 0xD9, 0x7C, 0x24, 0x16, 0x0F, 0xB7, 0x44, 0x24, 0x16, 0x0D, 0x00,
    0x0C, 0x00, 0x00, 0x89, 0x44, 0x24, 0x18, 0xD9, 0x6C, 0x24, 0x18, 0xDF, 0x7C, 0x24, 0x18, 0x8B,
    0x44, 0x24, 0x18, 0xD9, 0x6C, 0x24, 0x16, 0xEB, 0x2E, 0xD9, 0x05, 0x80, 0x43, 0xE8, 0x00, 0xDC,
    0x0D, 0xD8, 0xC0, 0xE5, 0x00, 0xD9, 0x7C, 0x24, 0x16, 0x0F, 0xB7, 0x44, 0x24, 0x16, 0x0D, 0x00,
    0x0C, 0x00, 0x00, 0x89, 0x44, 0x24, 0x18, 0xD9, 0x6C, 0x24, 0x18, 0xDF, 0x7C, 0x24, 0x18, 0x8B,
    0x44, 0x24, 0x18, 0xD9, 0x6C, 0x24, 0x16, 0x3D, 0xFA, 0x00, 0x00, 0x00, 0xA3, 0x5C, 0xB1, 0x12,
    0x02, 0x76, 0x0A, 0xC7, 0x05, 0x5C, 0xB1, 0x12, 0x02, 0xFA, 0x00, 0x00, 0x00, 0x5F, 0x5E, 0x5D,
    0x5B, 0x83, 0xC4, 0x14, 0xC3,
];
const B_CLOCK_ADVANCE_CALLS: [usize; 1] = [0x88];
const B_OUT_OF_PHASE_MASK: [u8; 68] = [
    0x56, 0x8B, 0xB1, 0x00, 0x04, 0x00, 0x00, 0x8B, 0xD6, 0x33, 0xC0, 0x85, 0xD2, 0x74, 0x33, 0x57,
    0x8B, 0xB9, 0x04, 0x04, 0x00, 0x00, 0x53, 0x3B, 0xD7, 0x74, 0x17, 0x8B, 0x4F, 0x40, 0x3B, 0x4A,
    0x40, 0x74, 0x0F, 0x8A, 0x8A, 0xB0, 0x01, 0x00, 0x00, 0xBB, 0x01, 0x00, 0x00, 0x00, 0xD3, 0xE3,
    0x0B, 0xC3, 0x8B, 0x92, 0xCC, 0x22, 0x00, 0x00, 0x3B, 0xD6, 0x74, 0x04, 0x85, 0xD2, 0x75, 0xD7,
    0x5B, 0x5F, 0x5E, 0xC3,
];
const B_OUT_OF_PHASE_MASK_CALLS: [usize; 0] = [];
const B_PEERS_PHASE_EQUAL: [u8; 98] = [
    0x80, 0x3D, 0x56, 0x8B, 0x52, 0x02, 0x00, 0x74, 0x09, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00,
    0x74, 0x09, 0x80, 0xBA, 0xF8, 0x03, 0x00, 0x00, 0x00, 0x75, 0x03, 0xB0, 0x01, 0xC3, 0x8B, 0xC2,
    0xE8, 0xAB, 0xD9, 0x0B, 0x00, 0x84, 0xC0, 0x74, 0x03, 0x32, 0xC0, 0xC3, 0x8B, 0x8A, 0x00, 0x04,
    0x00, 0x00, 0x8B, 0xC1, 0x85, 0xC0, 0x56, 0x74, 0x21, 0x8B, 0x92, 0x04, 0x04, 0x00, 0x00, 0x90,
    0x3B, 0xC2, 0x74, 0x08, 0x8B, 0x72, 0x40, 0x3B, 0x70, 0x40, 0x75, 0x12, 0x8B, 0x80, 0xCC, 0x22,
    0x00, 0x00, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0, 0x75, 0xE6, 0xB0, 0x01, 0x5E, 0xC3, 0x32, 0xC0,
    0x5E, 0xC3,
];
const B_PEERS_PHASE_EQUAL_CALLS: [usize; 1] = [0x20];
const B_ALL_CONNS_STATE_EQ: [u8; 54] = [
    0x8B, 0xD6, 0xE8, 0xB9, 0xFE, 0xFF, 0xFF, 0x84, 0xC0, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x8B, 0x8E,
    0x00, 0x04, 0x00, 0x00, 0x8B, 0xC1, 0x85, 0xC0, 0x74, 0x19, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x78, 0x3C, 0x75, 0xE6, 0x8B, 0x80, 0xCC, 0x22, 0x00, 0x00, 0x3B, 0xC1, 0x74, 0x04, 0x85,
    0xC0, 0x75, 0xED, 0xB0, 0x01, 0xC3,
];
const B_ALL_CONNS_STATE_EQ_CALLS: [usize; 1] = [0x2];
const B_ALL_CONNS_STATE_GE: [u8; 54] = [
    0x8B, 0xD6, 0xE8, 0x09, 0xFE, 0xFF, 0xFF, 0x84, 0xC0, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x8B, 0x8E,
    0x00, 0x04, 0x00, 0x00, 0x8B, 0xC1, 0x85, 0xC0, 0x74, 0x19, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x39, 0x78, 0x3C, 0x7C, 0xE6, 0x8B, 0x80, 0xCC, 0x22, 0x00, 0x00, 0x3B, 0xC1, 0x74, 0x04, 0x85,
    0xC0, 0x75, 0xED, 0xB0, 0x01, 0xC3,
];
const B_ALL_CONNS_STATE_GE_CALLS: [usize; 1] = [0x2];
const B_LOBBY_ANY_CONN_STATE_4: [u8; 47] = [
    0xA1, 0xE8, 0x8C, 0x52, 0x02, 0x8B, 0x88, 0x00, 0x04, 0x00, 0x00, 0x8B, 0xC1, 0x85, 0xC0, 0x74,
    0x18, 0xBA, 0x04, 0x00, 0x00, 0x00, 0x39, 0x50, 0x3C, 0x74, 0x11, 0x8B, 0x80, 0xCC, 0x22, 0x00,
    0x00, 0x3B, 0xC1, 0x74, 0x04, 0x85, 0xC0, 0x75, 0xED, 0x32, 0xC0, 0xC3, 0xB0, 0x01, 0xC3,
];
const B_LOBBY_ANY_CONN_STATE_4_CALLS: [usize; 0] = [];
const B_STATE_IN_SET_B: [u8; 51] = [
    0x8B, 0x40, 0x1C, 0x83, 0xC0, 0xFE, 0x83, 0xF8, 0x0A, 0x77, 0x11, 0x0F, 0xB6, 0x88, 0xD8, 0xCE,
    0x80, 0x00, 0xFF, 0x24, 0x8D, 0xD0, 0xCE, 0x80, 0x00, 0xB0, 0x01, 0xC3, 0x32, 0xC0, 0xC3, 0x90,
    0xC9, 0xCE, 0x80, 0x00, 0xCC, 0xCE, 0x80, 0x00, 0x00, 0x01, 0x00, 0x01, 0x01, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00,
];
const B_STATE_IN_SET_B_CALLS: [usize; 0] = [];
const B_STATE_IS_8: [u8; 16] = [
    0x8B, 0x0D, 0xB4, 0x8C, 0x52, 0x02, 0x33, 0xC0, 0x83, 0x79, 0x1C, 0x08, 0x0F, 0x94, 0xC0, 0xC3,
];
const B_STATE_IS_8_CALLS: [usize; 0] = [];
const B_LOCAL_CONN_ID: [u8; 25] = [
    0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0x8B, 0x80, 0x08, 0x04,
    0x00, 0x00, 0x8A, 0x80, 0xB0, 0x01, 0x00, 0x00, 0xC3,
];
const B_LOCAL_CONN_ID_CALLS: [usize; 0] = [];
const B_ID_TO_HUMAN: [u8; 95] = [
    0x66, 0x3D, 0xFF, 0xFF, 0x74, 0x44, 0x56, 0x8B, 0x35, 0x58, 0xE2, 0x12, 0x02, 0x0F, 0xB7, 0xC0,
    0x6A, 0x00, 0x50, 0xE8, 0xF8, 0x0D, 0x01, 0x00, 0x85, 0xC0, 0x5E, 0x74, 0x2D, 0x8B, 0x00, 0x85,
    0xC0, 0x74, 0x27, 0x8B, 0x48, 0x04, 0x85, 0xC9, 0x74, 0x20, 0x8B, 0xD1, 0x81, 0xE2, 0xFF, 0x0F,
    0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x0B, 0x39,
    0x48, 0x44, 0x75, 0x06, 0x83, 0x78, 0x4C, 0x01, 0x74, 0x03, 0x33, 0xC0, 0xC3, 0x8A, 0x88, 0xCC,
    0x03, 0x00, 0x00, 0x80, 0xE9, 0x01, 0xF6, 0xD9, 0x1B, 0xC9, 0xF7, 0xD1, 0x23, 0xC1, 0xC3,
];
const B_ID_TO_HUMAN_CALLS: [usize; 1] = [0x13];
const B_MAP_SET: [u8; 40] = [
    0x51, 0x56, 0x8B, 0x35, 0x58, 0xE2, 0x12, 0x02, 0x6A, 0x01, 0x50, 0xE8, 0x00, 0x14, 0x00, 0x00,
    0x85, 0xC0, 0x75, 0x07, 0x32, 0xC0, 0x5E, 0x59, 0xC2, 0x04, 0x00, 0x8B, 0x4C, 0x24, 0x0C, 0x89,
    0x08, 0xB0, 0x01, 0x5E, 0x59, 0xC2, 0x04, 0x00,
];
const B_MAP_SET_CALLS: [usize; 1] = [0xb];
const B_FIND_RESERVATION: [u8; 89] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x55, 0x8B, 0x6C, 0x24, 0x10, 0x56, 0x8B, 0xB0, 0xF4, 0x03, 0x00,
    0x00, 0x83, 0xEE, 0x01, 0x57, 0x78, 0x29, 0x8D, 0x0C, 0xB6, 0xC1, 0xE1, 0x04, 0x8D, 0x7C, 0x01,
    0x5C, 0x8B, 0xCB, 0x8B, 0xD7, 0xE8, 0xD6, 0x83, 0x00, 0x00, 0x84, 0xC0, 0x74, 0x08, 0x8B, 0x57,
    0x24, 0x3B, 0x53, 0x24, 0x74, 0x13, 0x83, 0xEE, 0x01, 0x83, 0xEF, 0x50, 0x85, 0xF6, 0x7D, 0xE1,
    0x5F, 0x5E, 0x5D, 0x32, 0xC0, 0x5B, 0xC2, 0x08, 0x00, 0x85, 0xED, 0x74, 0x03, 0x89, 0x75, 0x00,
    0x5F, 0x5E, 0x5D, 0xB0, 0x01, 0x5B, 0xC2, 0x08, 0x00,
];
const B_FIND_RESERVATION_CALLS: [usize; 1] = [0x25];
const B_REMOVE_RESERVATION: [u8; 97] = [
    0x83, 0x80, 0xF4, 0x03, 0x00, 0x00, 0xFF, 0x8B, 0x88, 0xF4, 0x03, 0x00, 0x00, 0x3B, 0xCA, 0x74,
    0x4F, 0x53, 0x8D, 0x0C, 0x89, 0x55, 0x56, 0xC1, 0xE1, 0x04, 0x8B, 0x6C, 0x01, 0x58, 0x8D, 0x14,
    0x92, 0x8D, 0x74, 0x01, 0x34, 0x57, 0xC1, 0xE2, 0x04, 0x03, 0xD0, 0x8D, 0x5A, 0x34, 0x8B, 0xFB,
    0xB9, 0x09, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0x89, 0x6B, 0x24, 0x8B, 0x88, 0xF4, 0x03, 0x00, 0x00,
    0x8D, 0x0C, 0x89, 0xC1, 0xE1, 0x04, 0x8D, 0x74, 0x01, 0x5C, 0x8D, 0x42, 0x5C, 0x8B, 0x56, 0x24,
    0xB9, 0x09, 0x00, 0x00, 0x00, 0x8B, 0xF8, 0xF3, 0xA5, 0x5F, 0x5E, 0x5D, 0x89, 0x50, 0x24, 0x5B,
    0xC3,
];
const B_REMOVE_RESERVATION_CALLS: [usize; 0] = [];
const B_GUID_IN_ARRAY: [u8; 77] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x55, 0x56, 0x57, 0x33, 0xFF, 0x85, 0xDB, 0x7E, 0x31, 0x8B, 0x6C,
    0x24, 0x18, 0x81, 0xC5, 0x4C, 0x01, 0x00, 0x00, 0x8B, 0xF0, 0x8D, 0x9B, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0xCE, 0x8B, 0xD5, 0xE8, 0x67, 0xB3, 0x00, 0x00, 0x84, 0xC0, 0x74, 0x08, 0x8B, 0x45, 0x24,
    0x3B, 0x46, 0x24, 0x74, 0x11, 0x83, 0xC7, 0x01, 0x83, 0xC6, 0x28, 0x3B, 0xFB, 0x7C, 0xE1, 0x5F,
    0x5E, 0x5D, 0x32, 0xC0, 0x5B, 0xC3, 0x5F, 0x5E, 0x5D, 0xB0, 0x01, 0x5B, 0xC3,
];
const B_GUID_IN_ARRAY_CALLS: [usize; 1] = [0x24];
const B_FIND_BEST_CONN: [u8; 73] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x56, 0x33, 0xC0, 0x33, 0xF6, 0x85, 0xFF, 0x8B, 0xCF, 0x74, 0x36,
    0x3B, 0xCB, 0x74, 0x24, 0x80, 0xB9, 0xCC, 0x1D, 0x00, 0x00, 0x00, 0x75, 0x1B, 0x80, 0xB9, 0xB8,
    0x01, 0x00, 0x00, 0x00, 0x74, 0x12, 0x8B, 0x91, 0xBC, 0x1D, 0x00, 0x00, 0x3B, 0xD6, 0x77, 0x04,
    0x85, 0xC0, 0x75, 0x04, 0x8B, 0xC1, 0x8B, 0xF2, 0x8B, 0x89, 0xCC, 0x22, 0x00, 0x00, 0x3B, 0xCF,
    0x74, 0x04, 0x85, 0xC9, 0x75, 0xCA, 0x5E, 0x5B, 0xC3,
];
const B_FIND_BEST_CONN_CALLS: [usize; 0] = [];
const B_SESSION_INFO_BASE_INIT: [u8; 49] = [
    0x33, 0xC9, 0xC7, 0x00, 0x54, 0xFD, 0xDF, 0x00, 0x89, 0x48, 0x30, 0x88, 0x48, 0x34, 0x89, 0x48,
    0x38, 0xC7, 0x40, 0x3C, 0x01, 0x00, 0x00, 0x00, 0x88, 0x48, 0x41, 0x88, 0x48, 0x42, 0x88, 0x48,
    0x43, 0x88, 0x48, 0x44, 0x88, 0x48, 0x46, 0x88, 0x48, 0x47, 0x88, 0x48, 0x48, 0x88, 0x48, 0x49,
    0xC3,
];
const B_SESSION_INFO_BASE_INIT_CALLS: [usize; 0] = [];
const B_BYTE_NC_INIT: [u8; 14] = [
    0xC7, 0x00, 0xC4, 0x64, 0xE2, 0x00, 0xC7, 0x40, 0x04, 0x00, 0x00, 0x00, 0x00, 0xC3,
];
const B_BYTE_NC_INIT_CALLS: [usize; 0] = [];
const B_BYTE_NC_INIT_BITS: [u8; 13] = [
    0xC1, 0xE9, 0x03, 0xC7, 0x00, 0xC4, 0x64, 0xE2, 0x00, 0x89, 0x48, 0x04, 0xC3,
];
const B_BYTE_NC_INIT_BITS_CALLS: [usize; 0] = [];
const B_BIT_NC_INIT: [u8; 18] = [
    0x33, 0xC9, 0xC7, 0x00, 0xD8, 0x64, 0xE2, 0x00, 0x89, 0x48, 0x04, 0x89, 0x48, 0x08, 0x89, 0x48,
    0x0C, 0xC3,
];
const B_BIT_NC_INIT_CALLS: [usize; 0] = [];
const B_BIT_NC_INIT_BITS: [u8; 24] = [
    0x8B, 0xD1, 0x89, 0x48, 0x04, 0xC1, 0xEA, 0x03, 0x83, 0xE1, 0x07, 0xC7, 0x00, 0xD8, 0x64, 0xE2,
    0x00, 0x89, 0x50, 0x08, 0x89, 0x48, 0x0C, 0xC3,
];
const B_BIT_NC_INIT_BITS_CALLS: [usize; 0] = [];
const B_FLOAT_COMP_INIT: [u8; 47] = [
    0xD9, 0x44, 0x24, 0x04, 0x8A, 0x54, 0x24, 0x18, 0xD9, 0x58, 0x04, 0x89, 0x08, 0xD9, 0x44, 0x24,
    0x08, 0x88, 0x50, 0x18, 0xD9, 0x58, 0x08, 0xD9, 0x44, 0x24, 0x0C, 0xD9, 0x58, 0x0C, 0xD9, 0x44,
    0x24, 0x10, 0xD9, 0x58, 0x10, 0xD9, 0x44, 0x24, 0x14, 0xD9, 0x58, 0x14, 0xC2, 0x18, 0x00,
];
const B_FLOAT_COMP_INIT_CALLS: [usize; 0] = [];
const B_FLOAT_COMP_INIT_COPY: [u8; 27] = [
    0x56, 0x57, 0x8D, 0x78, 0x04, 0xB9, 0x07, 0x00, 0x00, 0x00, 0x8D, 0x74, 0x24, 0x0C, 0xF3, 0xA5,
    0x5F, 0xC7, 0x00, 0xEC, 0x64, 0xE2, 0x00, 0x5E, 0xC2, 0x1C, 0x00,
];
const B_FLOAT_COMP_INIT_COPY_CALLS: [usize; 0] = [];
const B_FLOAT_COMP_COPY: [u8; 20] = [
    0x56, 0x57, 0x8B, 0xF1, 0x83, 0xC6, 0x04, 0x8D, 0x78, 0x04, 0xB9, 0x07, 0x00, 0x00, 0x00, 0xF3,
    0xA5, 0x5F, 0x5E, 0xC3,
];
const B_FLOAT_COMP_COPY_CALLS: [usize; 0] = [];
const B_VECTOR3_COMP_CTOR: [u8; 81] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0xD9, 0x45, 0x5C, 0x56, 0xC7, 0x00, 0x00, 0x65, 0xE2, 0x00,
    0x57, 0xB9, 0xEC, 0x64, 0xE2, 0x00, 0x89, 0x48, 0x04, 0x89, 0x48, 0x24, 0x89, 0x48, 0x44, 0x8D,
    0x78, 0x08, 0xB9, 0x07, 0x00, 0x00, 0x00, 0x8D, 0x75, 0x08, 0xF3, 0xA5, 0x8D, 0x78, 0x28, 0xB9,
    0x07, 0x00, 0x00, 0x00, 0x8D, 0x75, 0x24, 0xF3, 0xA5, 0x8D, 0x78, 0x48, 0xB9, 0x07, 0x00, 0x00,
    0x00, 0x8D, 0x75, 0x40, 0xF3, 0xA5, 0xD9, 0x58, 0x64, 0x5F, 0x5E, 0x8B, 0xE5, 0x5D, 0xC2, 0x58,
    0x00,
];
const B_VECTOR3_COMP_CTOR_CALLS: [usize; 0] = [];
const B_VECTOR2_COMP_CTOR: [u8; 59] = [
    0xD9, 0x44, 0x24, 0x3C, 0x56, 0x57, 0xC7, 0x00, 0x14, 0x65, 0xE2, 0x00, 0xB9, 0xEC, 0x64, 0xE2,
    0x00, 0x89, 0x48, 0x04, 0x89, 0x48, 0x24, 0x8D, 0x78, 0x08, 0xB9, 0x07, 0x00, 0x00, 0x00, 0x8D,
    0x74, 0x24, 0x0C, 0xF3, 0xA5, 0x8D, 0x78, 0x28, 0xB9, 0x07, 0x00, 0x00, 0x00, 0x8D, 0x74, 0x24,
    0x28, 0xF3, 0xA5, 0xD9, 0x58, 0x44, 0x5F, 0x5E, 0xC2, 0x3C, 0x00,
];
const B_VECTOR2_COMP_CTOR_CALLS: [usize; 0] = [];
const B_VECTOR3_UNCOMP_CTOR: [u8; 14] = [
    0xD9, 0x44, 0x24, 0x04, 0xC7, 0x00, 0x28, 0x65, 0xE2, 0x00, 0xD9, 0x58, 0x04, 0xC2,
];
const B_VECTOR3_UNCOMP_CTOR_CALLS: [usize; 0] = [];
const B_HEADING_COMP_CTOR: [u8; 41] = [
    0xD9, 0x44, 0x24, 0x08, 0x56, 0x57, 0xC7, 0x00, 0x3C, 0x65, 0xE2, 0x00, 0xC7, 0x40, 0x04, 0xEC,
    0x64, 0xE2, 0x00, 0x8D, 0x78, 0x08, 0xB9, 0x07, 0x00, 0x00, 0x00, 0x8D, 0x74, 0x24, 0x0C, 0xF3,
    0xA5, 0xD9, 0x58, 0x24, 0x5F, 0x5E, 0xC2, 0x1C, 0x00,
];
const B_HEADING_COMP_CTOR_CALLS: [usize; 0] = [];
const B_OWNERSHIP_PENDING: [u8; 106] = [
    0x8B, 0x48, 0x44, 0x85, 0xC9, 0x75, 0x07, 0xBA, 0xFF, 0xFF, 0x00, 0x00, 0xEB, 0x39, 0x8B, 0xD1,
    0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x82, 0x64, 0x9C, 0x14, 0x02, 0x85,
    0xC0, 0x74, 0x05, 0x39, 0x48, 0x44, 0x74, 0x07, 0xB8, 0xFF, 0xFF, 0x00, 0x00, 0xEB, 0x15, 0x8B,
    0x80, 0xB0, 0x00, 0x00, 0x00, 0x85, 0xC0, 0x75, 0x07, 0xB8, 0xFF, 0xFF, 0x00, 0x00, 0xEB, 0x04,
    0x0F, 0xB7, 0x40, 0x02, 0x0F, 0xB7, 0xD0, 0x8B, 0x0D, 0xAC, 0x8B, 0x52, 0x02, 0x33, 0xC0, 0x85,
    0xC9, 0x7E, 0x11, 0x66, 0x39, 0x14, 0xC5, 0x38, 0xBB, 0x12, 0x02, 0x74, 0x0A, 0x83, 0xC0, 0x01,
    0x3B, 0xC1, 0x7C, 0xEF, 0x32, 0xC0, 0xC3, 0xB0, 0x01, 0xC3,
];
const B_OWNERSHIP_PENDING_CALLS: [usize; 0] = [];
const B_CONN_OBJECT_LIST_ADD: [u8; 21] = [
    0x8B, 0x88, 0xF0, 0x21, 0x00, 0x00, 0x89, 0x94, 0x88, 0xF0, 0x1D, 0x00, 0x00, 0x83, 0x80, 0xF0,
    0x21, 0x00, 0x00, 0x01, 0xC3,
];
const B_CONN_OBJECT_LIST_ADD_CALLS: [usize; 0] = [];
const B_CONN_OBJECT_LIST_REMOVE: [u8; 62] = [
    0x56, 0x8B, 0xB0, 0xF0, 0x21, 0x00, 0x00, 0x33, 0xC9, 0x85, 0xF6, 0x76, 0x14, 0x8D, 0x90, 0xF0,
    0x1D, 0x00, 0x00, 0x39, 0x3A, 0x74, 0x0E, 0x83, 0xC1, 0x01, 0x83, 0xC2, 0x04, 0x3B, 0xCE, 0x72,
    0xF2, 0x32, 0xC0, 0x5E, 0xC3, 0x8B, 0x94, 0xB0, 0xEC, 0x1D, 0x00, 0x00, 0x89, 0x94, 0x88, 0xF0,
    0x1D, 0x00, 0x00, 0x83, 0x80, 0xF0, 0x21, 0x00, 0x00, 0xFF, 0xB0, 0x01, 0x5E, 0xC3,
];
const B_CONN_OBJECT_LIST_REMOVE_CALLS: [usize; 0] = [];
const B_GUID_ID_EQUAL: [u8; 30] = [
    0x8B, 0xCE, 0x8B, 0xD7, 0xE8, 0x17, 0x00, 0x00, 0x00, 0x84, 0xC0, 0x74, 0x0E, 0x8B, 0x47, 0x24,
    0x3B, 0x46, 0x24, 0x75, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33, 0xC0, 0xC3,
];
const B_GUID_ID_EQUAL_CALLS: [usize; 1] = [0x4];
const B_GUID_ENTRY_READ: [u8; 84] = [
    0x33, 0xC9, 0x89, 0x08, 0x89, 0x48, 0x04, 0x89, 0x48, 0x08, 0x89, 0x48, 0x0C, 0x89, 0x48, 0x10,
    0x89, 0x48, 0x14, 0x89, 0x48, 0x18, 0x89, 0x48, 0x1C, 0x89, 0x48, 0x20, 0xC7, 0x40, 0x24, 0xFF,
    0xFF, 0xFF, 0xFF, 0x8B, 0x0A, 0x57, 0x8B, 0x3C, 0x31, 0x89, 0x38, 0x8B, 0x4C, 0x31, 0x04, 0x89,
    0x48, 0x04, 0x83, 0x02, 0x08, 0x8B, 0x0A, 0x8B, 0x3C, 0x31, 0x89, 0x78, 0x08, 0x8B, 0x4C, 0x31,
    0x04, 0x89, 0x48, 0x0C, 0x83, 0x02, 0x08, 0x8B, 0x0A, 0x8B, 0x0C, 0x31, 0x89, 0x48, 0x24, 0x83,
    0x02, 0x04, 0x5F, 0xC3,
];
const B_GUID_ENTRY_READ_CALLS: [usize; 0] = [];
const B_ID_TO_OBJECT_AC: [u8; 78] = [
    0x66, 0x3D, 0xFF, 0xFF, 0x74, 0x3E, 0x56, 0x8B, 0x35, 0x58, 0xE2, 0x12, 0x02, 0x0F, 0xB7, 0xC0,
    0x6A, 0x00, 0x50, 0xE8, 0x28, 0x90, 0x00, 0x00, 0x85, 0xC0, 0x5E, 0x74, 0x27, 0x8B, 0x00, 0x85,
    0xC0, 0x74, 0x21, 0x8B, 0x40, 0x04, 0x85, 0xC0, 0x74, 0x1A, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F,
    0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x89, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC9, 0x74, 0x05, 0x39,
    0x41, 0x44, 0x74, 0x03, 0x33, 0xC0, 0xC3, 0x8B, 0x81, 0xAC, 0x00, 0x00, 0x00, 0xC3,
];
const B_ID_TO_OBJECT_AC_CALLS: [usize; 1] = [0x13];
const B_CAPTURE_SNAPSHOT: [u8; 63] = [
    0x85, 0xD2, 0x74, 0x3A, 0x8B, 0xC2, 0x25, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80,
    0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x26, 0x39, 0x50, 0x44, 0x75, 0x21, 0x8B, 0x80, 0xB0,
    0x00, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x17, 0x8B, 0x48, 0x08, 0x8B, 0x0C, 0x8D, 0x5C, 0xE2, 0x12,
    0x02, 0x85, 0xC9, 0x74, 0x09, 0x8B, 0x01, 0x56, 0x52, 0x8B, 0x50, 0x24, 0xFF, 0xD2, 0xC3,
];
const B_CAPTURE_SNAPSHOT_CALLS: [usize; 0] = [];
const B_READ_STATE_DELTA: [u8; 87] = [
    0x85, 0xC0, 0x74, 0x24, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B,
    0x89, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC9, 0x74, 0x0F, 0x39, 0x41, 0x44, 0x75, 0x0A, 0x8B, 0x89,
    0xB0, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x75, 0x06, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xC3, 0x8B, 0x51,
    0x08, 0x8B, 0x0C, 0x95, 0x5C, 0xE2, 0x12, 0x02, 0x8B, 0x11, 0x56, 0x8B, 0x74, 0x24, 0x14, 0x56,
    0x8B, 0x74, 0x24, 0x14, 0x56, 0x8B, 0x74, 0x24, 0x14, 0x56, 0x8B, 0x74, 0x24, 0x14, 0x56, 0x50,
    0x8B, 0x42, 0x30, 0xFF, 0xD0, 0x5E, 0xC3,
];
const B_READ_STATE_DELTA_CALLS: [usize; 0] = [];
const B_GET_SEND_PRIORITY: [u8; 67] = [
    0x85, 0xC0, 0x75, 0x04, 0x66, 0x33, 0xC0, 0xC3, 0x8B, 0x48, 0x08, 0x56, 0x8B, 0x74, 0x24, 0x20,
    0x8B, 0x0C, 0x8D, 0x5C, 0xE2, 0x12, 0x02, 0x8B, 0x11, 0x8B, 0x40, 0x04, 0x56, 0x8B, 0x74, 0x24,
    0x20, 0x8B, 0x52, 0x38, 0x56, 0x8B, 0x74, 0x24, 0x20, 0x56, 0x8B, 0x74, 0x24, 0x20, 0x56, 0x8B,
    0x74, 0x24, 0x20, 0x56, 0x8B, 0x74, 0x24, 0x20, 0x56, 0x8B, 0x74, 0x24, 0x20, 0x56, 0x50, 0xFF,
    0xD2, 0x5E, 0xC3,
];
const B_GET_SEND_PRIORITY_CALLS: [usize; 0] = [];
const B_GET_DIRTY_FIELDS: [u8; 39] = [
    0x85, 0xC0, 0x75, 0x04, 0x66, 0x33, 0xC0, 0xC3, 0x8B, 0x51, 0x08, 0x8B, 0x0C, 0x95, 0x5C, 0xE2,
    0x12, 0x02, 0x8B, 0x11, 0x56, 0x8B, 0x74, 0x24, 0x0C, 0x56, 0x8B, 0x74, 0x24, 0x0C, 0x56, 0x50,
    0x8B, 0x42, 0x34, 0xFF, 0xD0, 0x5E, 0xC3,
];
const B_GET_DIRTY_FIELDS_CALLS: [usize; 0] = [];
const B_ON_STATE_SENT: [u8; 76] = [
    0x85, 0xC0, 0x74, 0x47, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B,
    0x89, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC9, 0x74, 0x32, 0x39, 0x41, 0x44, 0x75, 0x2D, 0x8B, 0x89,
    0xB0, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x74, 0x23, 0x8B, 0x51, 0x08, 0x8B, 0x0C, 0x95, 0x5C, 0xE2,
    0x12, 0x02, 0x8B, 0x11, 0x56, 0x8B, 0x74, 0x24, 0x10, 0x56, 0x8B, 0x74, 0x24, 0x10, 0x56, 0x8B,
    0x74, 0x24, 0x10, 0x56, 0x50, 0x8B, 0x42, 0x3C, 0xFF, 0xD0, 0x5E, 0xC3,
];
const B_ON_STATE_SENT_CALLS: [usize; 0] = [];
const B_CAN_MIGRATE: [u8; 82] = [
    0x85, 0xC0, 0x75, 0x03, 0x32, 0xC0, 0xC3, 0xF6, 0x80, 0x80, 0x00, 0x00, 0x00, 0x0B, 0x75, 0xF4,
    0x8B, 0x88, 0xB0, 0x00, 0x00, 0x00, 0x85, 0xC9, 0x74, 0xEA, 0x66, 0x81, 0xB9, 0x7E, 0x06, 0x00,
    0x00, 0xFF, 0xFF, 0x0F, 0x94, 0xC1, 0x84, 0xC9, 0x74, 0x25, 0x8B, 0x88, 0xB0, 0x00, 0x00, 0x00,
    0x8B, 0x51, 0x08, 0x8B, 0x0C, 0x95, 0x5C, 0xE2, 0x12, 0x02, 0x8B, 0x11, 0x8B, 0x40, 0x44, 0x8B,
    0x52, 0x44, 0x50, 0xFF, 0xD2, 0x84, 0xC0, 0x74, 0x06, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0x33,
    0xC0, 0xC3,
];
const B_CAN_MIGRATE_CALLS: [usize; 0] = [];
const B_REQUEST_OWNERSHIP: [u8; 328] = [
    0x81, 0xEC, 0x24, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x20, 0x05, 0x00, 0x00, 0x8B, 0x84, 0x24, 0x28, 0x05, 0x00, 0x00, 0x8B, 0x40, 0x44, 0x85, 0xC0,
    0x53, 0x8B, 0x9C, 0x24, 0x34, 0x05, 0x00, 0x00, 0x56, 0x57, 0x74, 0x1A, 0x8B, 0xC8, 0x81, 0xE1,
    0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE1, 0x04, 0x8B, 0x89, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC9, 0x74,
    0x05, 0x39, 0x41, 0x44, 0x74, 0x04, 0x33, 0xFF, 0xEB, 0x06, 0x8B, 0xB9, 0xB0, 0x00, 0x00, 0x00,
    0x8B, 0x35, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xF6, 0x75, 0x1A, 0x32, 0xC0, 0x5F, 0x5E, 0x5B, 0x8B,
    0x8C, 0x24, 0x20, 0x05, 0x00, 0x00, 0x33, 0xCC, 0xE8, 0x52, 0xF4, 0x48, 0x00, 0x81, 0xC4, 0x24,
    0x05, 0x00, 0x00, 0xC3, 0x85, 0xFF, 0x74, 0xE2, 0xF6, 0x87, 0x7C, 0x06, 0x00, 0x00, 0x02, 0x0F,
    0x85, 0x9A, 0x00, 0x00, 0x00, 0x8B, 0xCF, 0xE8, 0x44, 0xFB, 0xFF, 0xFF, 0x84, 0xC0, 0x0F, 0x85,
    0x8B, 0x00, 0x00, 0x00, 0x83, 0x3D, 0xAC, 0x8B, 0x52, 0x02, 0x20, 0x74, 0xBD, 0x80, 0x3D, 0xDC,
    0xC2, 0xE8, 0x00, 0x00, 0xC6, 0x44, 0x24, 0x10, 0x28, 0xB8, 0x03, 0x00, 0x00, 0x00, 0x74, 0x05,
    0xB8, 0x04, 0x00, 0x00, 0x00, 0x66, 0x8B, 0x4F, 0x02, 0x0F, 0xB7, 0xD0, 0x66, 0x89, 0x4C, 0x14,
    0x10, 0x8A, 0x8C, 0x24, 0x38, 0x05, 0x00, 0x00, 0x83, 0xC0, 0x02, 0x0F, 0xB7, 0xD0, 0x88, 0x4C,
    0x14, 0x10, 0x83, 0xC0, 0x01, 0x8D, 0x50, 0xFD, 0x6A, 0x00, 0x0F, 0xB7, 0xC0, 0x6A, 0x00, 0x50,
    0x8D, 0x4C, 0x24, 0x1C, 0x66, 0x89, 0x54, 0x24, 0x1D, 0x8A, 0x17, 0x51, 0x8B, 0xC6, 0xE8, 0xFD,
    0x91, 0x00, 0x00, 0x8B, 0xF0, 0xE8, 0xE6, 0xD9, 0x00, 0x00, 0xA1, 0xAC, 0x8B, 0x52, 0x02, 0x66,
    0x8B, 0x57, 0x02, 0x66, 0x89, 0x14, 0xC5, 0x38, 0xBB, 0x12, 0x02, 0x83, 0xC4, 0x10, 0x89, 0x1C,
    0xC5, 0x3C, 0xBB, 0x12, 0x02, 0x83, 0xC0, 0x01, 0xA3, 0xAC, 0x8B, 0x52, 0x02, 0xEB, 0x0F, 0x85,
    0xDB, 0x74, 0x0B, 0x8B, 0x47, 0x04, 0x6A, 0x01, 0x50, 0xFF, 0xD3, 0x83, 0xC4, 0x08, 0x8B, 0x8C,
    0x24, 0x2C, 0x05, 0x00, 0x00, 0x5F, 0x5E, 0x5B, 0x33, 0xCC, 0xB0, 0x01, 0xE8, 0x7E, 0xF3, 0x48,
    0x00, 0x81, 0xC4, 0x24, 0x05, 0x00, 0x00, 0xC3,
];
const B_REQUEST_OWNERSHIP_CALLS: [usize; 5] = [0x68, 0x87, 0xee, 0xf5, 0x13c];

// --- original bodies, round 3 (tools/body_bytes.py) ---------------------------------------------------------------------------

const B_DISPATCH_ACTION: [u8; 104] = [
    0x3D, 0xFF, 0x00, 0x00, 0x00, 0x74, 0x42, 0x83, 0xF8, 0x05, 0x77, 0x3D, 0xFF, 0x24, 0x85, 0xC0,
    0xED, 0x80, 0x00, 0x8B, 0x06, 0x8B, 0x50, 0x0C, 0x8B, 0xCE, 0xFF, 0xD2, 0xC6, 0x86, 0x00, 0x0C,
    0x00, 0x00, 0x01, 0xC3, 0x8B, 0x06, 0x8B, 0x50, 0x10, 0x8B, 0xCE, 0xFF, 0xD2, 0xC6, 0x86, 0x00,
    0x0C, 0x00, 0x00, 0x01, 0xC3, 0x8B, 0x06, 0x8B, 0x50, 0x14, 0x8B, 0xCE, 0xFF, 0xD2, 0xC6, 0x86,
    0x00, 0x0C, 0x00, 0x00, 0x01, 0xC3, 0x33, 0xC0, 0xC3, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xC3, 0x90,
    0xB6, 0xED, 0x80, 0x00, 0x83, 0xED, 0x80, 0x00, 0x94, 0xED, 0x80, 0x00, 0xB6, 0xED, 0x80, 0x00,
    0xA5, 0xED, 0x80, 0x00, 0xB6, 0xED, 0x80, 0x00,
];
const B_ADD_FIELD: [u8; 145] = [
    0x8B, 0x42, 0x18, 0x83, 0xF8, 0x10, 0x72, 0x05, 0x33, 0xC0, 0xC2, 0x10, 0x00, 0x8B, 0xC8, 0xC1,
    0xE1, 0x04, 0x03, 0xC8, 0x56, 0x8D, 0x74, 0x8A, 0x20, 0x33, 0xC9, 0x8D, 0x46, 0x08, 0x89, 0x48,
    0x10, 0x89, 0x48, 0x08, 0x66, 0x89, 0x48, 0x0C, 0x89, 0x48, 0x04, 0xC7, 0x00, 0xFF, 0xFF, 0xFF,
    0xFF, 0x8B, 0x4A, 0x18, 0x89, 0x08, 0x8B, 0x4C, 0x24, 0x14, 0x89, 0x4E, 0x3C, 0x83, 0x42, 0x18,
    0x01, 0x8B, 0x4A, 0x1C, 0x57, 0x8B, 0x7A, 0x18, 0xD3, 0xEF, 0x85, 0xFF, 0x76, 0x12, 0x8B, 0xFF,
    0x83, 0x42, 0x1C, 0x01, 0x8B, 0x7A, 0x18, 0x8B, 0x4A, 0x1C, 0xD3, 0xEF, 0x85, 0xFF, 0x77, 0xF0,
    0x8B, 0x4A, 0x1C, 0xBF, 0x01, 0x00, 0x00, 0x00, 0xD3, 0xE7, 0x83, 0xEF, 0x01, 0x39, 0x38, 0x75,
    0x07, 0x5F, 0x33, 0xC0, 0x5E, 0xC2, 0x10, 0x00, 0x8B, 0x54, 0x24, 0x0C, 0x8B, 0x4C, 0x24, 0x10,
    0x89, 0x16, 0x8B, 0x54, 0x24, 0x14, 0x5F, 0x89, 0x4E, 0x04, 0x89, 0x56, 0x40, 0x5E, 0xC2, 0x10,
    0x00,
];
const B_REGS_RESET: [u8; 198] = [
    0x53, 0x56, 0x8B, 0x35, 0xAC, 0xE2, 0x12, 0x02, 0x57, 0x33, 0xDB, 0xEB, 0x03, 0x8D, 0x49, 0x00,
    0x3B, 0xF3, 0x74, 0x1D, 0x8B, 0x86, 0x6C, 0x06, 0x00, 0x00, 0x8B, 0xF8, 0x2B, 0x3D, 0xAC, 0xE2,
    0x12, 0x02, 0xF7, 0xDF, 0x1B, 0xFF, 0x23, 0xF8, 0xE8, 0xE3, 0xD2, 0xFF, 0xFF, 0x8B, 0xF7, 0xEB,
    0xDF, 0x5F, 0x5E, 0x89, 0x1D, 0xAC, 0xE2, 0x12, 0x02, 0x89, 0x1D, 0x18, 0xB5, 0x12, 0x02, 0x89,
    0x1D, 0x84, 0xE2, 0x12, 0x02, 0x89, 0x1D, 0xF0, 0xB4, 0x12, 0x02, 0x89, 0x1D, 0x88, 0xE2, 0x12,
    0x02, 0x89, 0x1D, 0xF4, 0xB4, 0x12, 0x02, 0x89, 0x1D, 0x8C, 0xE2, 0x12, 0x02, 0x89, 0x1D, 0xF8,
    0xB4, 0x12, 0x02, 0x89, 0x1D, 0x90, 0xE2, 0x12, 0x02, 0x89, 0x1D, 0xFC, 0xB4, 0x12, 0x02, 0x89,
    0x1D, 0x94, 0xE2, 0x12, 0x02, 0x89, 0x1D, 0x00, 0xB5, 0x12, 0x02, 0x89, 0x1D, 0x98, 0xE2, 0x12,
    0x02, 0x89, 0x1D, 0x04, 0xB5, 0x12, 0x02, 0x89, 0x1D, 0x9C, 0xE2, 0x12, 0x02, 0x89, 0x1D, 0x08,
    0xB5, 0x12, 0x02, 0x89, 0x1D, 0xA0, 0xE2, 0x12, 0x02, 0x89, 0x1D, 0x0C, 0xB5, 0x12, 0x02, 0x89,
    0x1D, 0xA4, 0xE2, 0x12, 0x02, 0x89, 0x1D, 0x10, 0xB5, 0x12, 0x02, 0x89, 0x1D, 0xA8, 0xE2, 0x12,
    0x02, 0x89, 0x1D, 0x14, 0xB5, 0x12, 0x02, 0x89, 0x1D, 0x34, 0xBA, 0x12, 0x02, 0xC6, 0x05, 0x0A,
    0x86, 0xE8, 0x00, 0x01, 0x5B, 0xC3,
];
const B_DESTROY_FROM_PACKET: [u8; 156] = [
    0x53, 0x8B, 0x5C, 0x24, 0x08, 0x8B, 0x03, 0x85, 0xC0, 0x0F, 0x8C, 0x89, 0x00, 0x00, 0x00, 0x83,
    0xF8, 0x0A, 0x0F, 0x8F, 0x80, 0x00, 0x00, 0x00, 0x83, 0x3C, 0x85, 0x5C, 0xE2, 0x12, 0x02, 0x00,
    0x74, 0x76, 0x0F, 0xB7, 0x43, 0x04, 0x56, 0x8B, 0x35, 0x58, 0xE2, 0x12, 0x02, 0x57, 0x8D, 0x7B,
    0x04, 0x0F, 0xB7, 0xC0, 0x6A, 0x00, 0x50, 0xE8, 0xA4, 0x48, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x06,
    0x8B, 0x30, 0x85, 0xF6, 0x75, 0x10, 0x8B, 0xCF, 0xE8, 0x33, 0x28, 0x03, 0x00, 0x5F, 0x3C, 0x01,
    0x5E, 0x0F, 0x94, 0xC0, 0x5B, 0xC3, 0x8B, 0x56, 0x04, 0x66, 0x8B, 0x4E, 0x02, 0x6A, 0x00, 0xE8,
    0x0C, 0xFF, 0xFF, 0xFF, 0x8B, 0x4E, 0x08, 0x8B, 0x0C, 0x8D, 0x5C, 0xE2, 0x12, 0x02, 0x83, 0xC4,
    0x04, 0x83, 0xC3, 0x06, 0x53, 0x8D, 0x44, 0x24, 0x14, 0xC7, 0x44, 0x24, 0x14, 0x00, 0x00, 0x00,
    0x00, 0x8B, 0x11, 0x8B, 0x52, 0x18, 0x50, 0x8B, 0x46, 0x04, 0x50, 0xFF, 0xD2, 0xE8, 0x5E, 0xFB,
    0xFF, 0xFF, 0x5F, 0x5E, 0xB0, 0x01, 0x5B, 0xC3, 0xB0, 0x01, 0x5B, 0xC3,
];
const B_SEND_NEW_OWNER: [u8; 301] = [
    0x8B, 0x44, 0x24, 0x04, 0x0F, 0xB7, 0x00, 0x53, 0x55, 0x56, 0x8B, 0x35, 0x58, 0xE2, 0x12, 0x02,
    0x57, 0x0F, 0xB7, 0xC8, 0x6A, 0x00, 0x51, 0xE8, 0x44, 0x26, 0x00, 0x00, 0x85, 0xC0, 0x0F, 0x84,
    0x02, 0x01, 0x00, 0x00, 0x8B, 0x38, 0x85, 0xFF, 0x0F, 0x84, 0xF8, 0x00, 0x00, 0x00, 0x8B, 0x1D,
    0x14, 0x8C, 0x52, 0x02, 0x85, 0xDB, 0x0F, 0x84, 0xEA, 0x00, 0x00, 0x00, 0x8B, 0x54, 0x24, 0x14,
    0x8A, 0x52, 0x02, 0x8B, 0xAB, 0x08, 0x04, 0x00, 0x00, 0x8B, 0xC3, 0xE8, 0x60, 0x96, 0x00, 0x00,
    0x8B, 0xF0, 0x85, 0xF6, 0x75, 0x61, 0x8B, 0x44, 0x24, 0x14, 0x80, 0x78, 0x03, 0x00, 0x74, 0x14,
    0x8B, 0x8D, 0xAC, 0x01, 0x00, 0x00, 0x3B, 0xA9, 0x04, 0x04, 0x00, 0x00, 0x74, 0x29, 0x8B, 0xB3,
    0x04, 0x04, 0x00, 0x00, 0x85, 0xF6, 0x75, 0x3F, 0x83, 0xC0, 0x04, 0x50, 0x8D, 0x4C, 0x24, 0x18,
    0xC7, 0x44, 0x24, 0x18, 0x06, 0x00, 0x00, 0x00, 0xE8, 0xB3, 0xDE, 0xFF, 0xFF, 0x83, 0xC4, 0x04,
    0xB0, 0x01, 0x5F, 0x5E, 0x5D, 0x5B, 0xC3, 0x8B, 0x9B, 0x00, 0x04, 0x00, 0x00, 0x8B, 0xF3, 0x85,
    0xF6, 0x74, 0xD5, 0x3B, 0xF5, 0x75, 0xCD, 0x8B, 0xB6, 0xCC, 0x22, 0x00, 0x00, 0x3B, 0xF3, 0x74,
    0xC7, 0x85, 0xF6, 0x75, 0xEE, 0xEB, 0xC1, 0x8B, 0x47, 0x04, 0x85, 0xC0, 0x74, 0x1E, 0x8B, 0xD0,
    0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B, 0x8A, 0x64, 0x9C, 0x14, 0x02, 0x85,
    0xC9, 0x74, 0x09, 0x39, 0x41, 0x44, 0x75, 0x04, 0x8B, 0xC1, 0xEB, 0x02, 0x33, 0xC0, 0xE8, 0x9D,
    0x04, 0x00, 0x00, 0x84, 0xC0, 0x75, 0x05, 0x5F, 0x5E, 0x5D, 0x5B, 0xC3, 0x8A, 0x86, 0xB0, 0x01,
    0x00, 0x00, 0x88, 0x07, 0x8B, 0x44, 0x24, 0x14, 0x0F, 0xB7, 0x88, 0x04, 0x02, 0x00, 0x00, 0x6A,
    0x00, 0x6A, 0x00, 0x51, 0x83, 0xC0, 0x04, 0x50, 0xE8, 0x93, 0xDD, 0x00, 0x00, 0x8B, 0x57, 0x08,
    0x8B, 0x0C, 0x95, 0x5C, 0xE2, 0x12, 0x02, 0x8B, 0x01, 0x8B, 0x57, 0x04, 0x8B, 0x40, 0x50, 0x83,
    0xC4, 0x10, 0x56, 0x52, 0xFF, 0xD0, 0x5F, 0x5E, 0x5D, 0xB0, 0x01, 0x5B, 0xC3,
];
const B_OBJ_UNREGISTER: [u8; 399] = [
    0x51, 0x53, 0x33, 0xDB, 0x3B, 0xF3, 0x57, 0x0F, 0x84, 0x7E, 0x01, 0x00, 0x00, 0x8B, 0x46, 0x08,
    0x3B, 0xC3, 0x0F, 0x8C, 0x73, 0x01, 0x00, 0x00, 0x83, 0xF8, 0x0A, 0x0F, 0x8D, 0x6A, 0x01, 0x00,
    0x00, 0x39, 0x34, 0x85, 0x84, 0xE2, 0x12, 0x02, 0x75, 0x26, 0x8B, 0x8E, 0x74, 0x06, 0x00, 0x00,
    0x3B, 0xCE, 0x75, 0x15, 0x89, 0x9E, 0x74, 0x06, 0x00, 0x00, 0x89, 0x9E, 0x78, 0x06, 0x00, 0x00,
    0x89, 0x1C, 0x85, 0x84, 0xE2, 0x12, 0x02, 0xEB, 0x37, 0x89, 0x0C, 0x85, 0x84, 0xE2, 0x12, 0x02,
    0x8B, 0x86, 0x78, 0x06, 0x00, 0x00, 0x8B, 0x8E, 0x74, 0x06, 0x00, 0x00, 0x89, 0x88, 0x74, 0x06,
    0x00, 0x00, 0x8B, 0x96, 0x74, 0x06, 0x00, 0x00, 0x8B, 0x86, 0x78, 0x06, 0x00, 0x00, 0x89, 0x82,
    0x78, 0x06, 0x00, 0x00, 0x89, 0x9E, 0x74, 0x06, 0x00, 0x00, 0x89, 0x9E, 0x78, 0x06, 0x00, 0x00,
    0x38, 0x1D, 0xB6, 0xE9, 0x44, 0x01, 0x74, 0x09, 0x66, 0x8B, 0x46, 0x02, 0xE8, 0x9F, 0xB9, 0xFF,
    0xFF, 0x8B, 0x46, 0x04, 0x3B, 0xC3, 0x74, 0x3C, 0x8B, 0xC8, 0x81, 0xE1, 0xFF, 0x0F, 0x00, 0x00,
    0xC1, 0xE1, 0x04, 0x8B, 0xB9, 0x64, 0x9C, 0x14, 0x02, 0x3B, 0xFB, 0x74, 0x27, 0x39, 0x47, 0x44,
    0x75, 0x22, 0x8B, 0x56, 0x08, 0x8B, 0x0C, 0x95, 0x5C, 0xE2, 0x12, 0x02, 0x8B, 0x11, 0x50, 0x8B,
    0x42, 0x10, 0xFF, 0xD0, 0x81, 0xA7, 0x80, 0x00, 0x00, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0x89, 0x9F,
    0xB0, 0x00, 0x00, 0x00, 0x0F, 0xB7, 0x46, 0x02, 0xE8, 0x83, 0x38, 0x00, 0x00, 0x39, 0x35, 0xAC,
    0xE2, 0x12, 0x02, 0x75, 0x17, 0x8B, 0x86, 0x6C, 0x06, 0x00, 0x00, 0x3B, 0xC6, 0x75, 0x08, 0x89,
    0x1D, 0xAC, 0xE2, 0x12, 0x02, 0xEB, 0x31, 0xA3, 0xAC, 0xE2, 0x12, 0x02, 0x8B, 0x86, 0x70, 0x06,
    0x00, 0x00, 0x3B, 0xC3, 0x74, 0x0C, 0x8B, 0x8E, 0x6C, 0x06, 0x00, 0x00, 0x89, 0x88, 0x6C, 0x06,
    0x00, 0x00, 0x8B, 0x86, 0x6C, 0x06, 0x00, 0x00, 0x3B, 0xC3, 0x74, 0x0C, 0x8B, 0x96, 0x70, 0x06,
    0x00, 0x00, 0x89, 0x90, 0x70, 0x06, 0x00, 0x00, 0xA1, 0x54, 0xC2, 0x12, 0x02, 0x3B, 0xC3, 0x89,
    0x9E, 0x70, 0x06, 0x00, 0x00, 0x89, 0x9E, 0x6C, 0x06, 0x00, 0x00, 0x75, 0x1D, 0x83, 0x2D, 0x34,
    0xBA, 0x12, 0x02, 0x01, 0x89, 0xB6, 0x6C, 0x06, 0x00, 0x00, 0x89, 0xB6, 0x70, 0x06, 0x00, 0x00,
    0x89, 0x35, 0x54, 0xC2, 0x12, 0x02, 0x5F, 0x5B, 0x59, 0xC3, 0x8B, 0x88, 0x70, 0x06, 0x00, 0x00,
    0x83, 0x2D, 0x34, 0xBA, 0x12, 0x02, 0x01, 0x89, 0xB1, 0x6C, 0x06, 0x00, 0x00, 0x8B, 0x90, 0x70,
    0x06, 0x00, 0x00, 0x89, 0x96, 0x70, 0x06, 0x00, 0x00, 0x89, 0xB0, 0x70, 0x06, 0x00, 0x00, 0x89,
    0x86, 0x6C, 0x06, 0x00, 0x00, 0x89, 0x35, 0x54, 0xC2, 0x12, 0x02, 0x5F, 0x5B, 0x59, 0xC3,
];
const B_DEACTIVATE_SEND: [u8; 345] = [
    0x81, 0xEC, 0x1C, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x18, 0x05, 0x00, 0x00, 0x85, 0xC9, 0x0F, 0x84, 0x28, 0x01, 0x00, 0x00, 0x8B, 0xC1, 0x25, 0xFF,
    0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x0F, 0x84,
    0x10, 0x01, 0x00, 0x00, 0x39, 0x48, 0x44, 0x0F, 0x85, 0x07, 0x01, 0x00, 0x00, 0x53, 0x8B, 0x98,
    0xB0, 0x00, 0x00, 0x00, 0x85, 0xDB, 0x0F, 0x84, 0xF7, 0x00, 0x00, 0x00, 0x8B, 0x0D, 0x14, 0x8C,
    0x52, 0x02, 0x85, 0xC9, 0x55, 0x8B, 0xE9, 0x0F, 0x84, 0xE5, 0x00, 0x00, 0x00, 0x8B, 0x91, 0x08,
    0x04, 0x00, 0x00, 0x8A, 0x03, 0x3A, 0x82, 0xB0, 0x01, 0x00, 0x00, 0x0F, 0x85, 0xD1, 0x00, 0x00,
    0x00, 0x80, 0x3D, 0x84, 0xC0, 0xE8, 0x00, 0x00, 0x8B, 0x53, 0x08, 0x56, 0x8B, 0x34, 0x95, 0x5C,
    0xE2, 0x12, 0x02, 0xC6, 0x44, 0x24, 0x10, 0x0F, 0xB8, 0x03, 0x00, 0x00, 0x00, 0x74, 0x05, 0xB8,
    0x04, 0x00, 0x00, 0x00, 0x8B, 0x49, 0x08, 0x0F, 0xB7, 0xD0, 0x89, 0x4C, 0x14, 0x10, 0x83, 0xC0,
    0x04, 0x89, 0x44, 0x24, 0x0C, 0x8A, 0x53, 0x08, 0x0F, 0xB7, 0xC8, 0x88, 0x54, 0x0C, 0x10, 0x83,
    0xC0, 0x01, 0x89, 0x44, 0x24, 0x0C, 0x66, 0x8B, 0x53, 0x02, 0x0F, 0xB7, 0xC8, 0x66, 0x89, 0x54,
    0x0C, 0x10, 0x83, 0xC0, 0x02, 0x8D, 0x4C, 0x24, 0x0C, 0x51, 0x89, 0x44, 0x24, 0x10, 0x8B, 0x4B,
    0x04, 0x8B, 0x06, 0x8D, 0x54, 0x24, 0x14, 0x52, 0x8B, 0x50, 0x14, 0x51, 0x8B, 0xCE, 0xFF, 0xD2,
    0x8B, 0x44, 0x24, 0x0C, 0x8B, 0xB5, 0x00, 0x04, 0x00, 0x00, 0x83, 0xC0, 0xFD, 0x85, 0xF6, 0x66,
    0x89, 0x44, 0x24, 0x11, 0x74, 0x44, 0x57, 0x83, 0x7E, 0x3C, 0x07, 0x7E, 0x2A, 0x80, 0x3E, 0x00,
    0x75, 0x25, 0x8B, 0x7B, 0x04, 0x8B, 0xC6, 0xE8, 0xA4, 0xF4, 0xFF, 0xFF, 0x84, 0xC0, 0x75, 0x17,
    0x0F, 0xB7, 0x4C, 0x24, 0x10, 0x6A, 0x00, 0x6A, 0x00, 0x51, 0x8D, 0x54, 0x24, 0x20, 0x52, 0xE8,
    0x9C, 0xF1, 0x00, 0x00, 0x83, 0xC4, 0x10, 0x8B, 0xB6, 0xCC, 0x22, 0x00, 0x00, 0x3B, 0xB5, 0x00,
    0x04, 0x00, 0x00, 0x74, 0x04, 0x85, 0xF6, 0x75, 0xBE, 0x5F, 0x8B, 0xF3, 0xE8, 0x4F, 0xEC, 0xFF,
    0xFF, 0x5E, 0x5D, 0x5B, 0x8B, 0x8C, 0x24, 0x18, 0x05, 0x00, 0x00, 0x33, 0xCC, 0xE8, 0x4D, 0x0B,
    0x49, 0x00, 0x81, 0xC4, 0x1C, 0x05, 0x00, 0x00, 0xC3,
];
const B_REGISTER_LOCAL: [u8; 489] = [
    0x83, 0xEC, 0x08, 0x85, 0xFF, 0x53, 0x8B, 0x5C, 0x24, 0x10, 0x74, 0x26, 0x8B, 0xC7, 0x25, 0xFF,
    0x0F, 0x00, 0x00, 0xC1, 0xE0, 0x04, 0x8B, 0x80, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x12,
    0x39, 0x78, 0x44, 0x75, 0x0D, 0x83, 0xB8, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x85, 0xB2, 0x01,
    0x00, 0x00, 0x83, 0xFB, 0x06, 0x74, 0x0D, 0x83, 0x3D, 0xB4, 0x8B, 0x52, 0x02, 0x00, 0x0F, 0x8F,
    0xA0, 0x01, 0x00, 0x00, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0x85, 0xC0, 0x89, 0x44, 0x24, 0x08, 0x0F,
    0x84, 0x8F, 0x01, 0x00, 0x00, 0x85, 0xFF, 0x8B, 0x88, 0x08, 0x04, 0x00, 0x00, 0x55, 0x89, 0x4C,
    0x24, 0x08, 0x74, 0x1E, 0x8B, 0xD7, 0x81, 0xE2, 0xFF, 0x0F, 0x00, 0x00, 0xC1, 0xE2, 0x04, 0x8B,
    0x82, 0x64, 0x9C, 0x14, 0x02, 0x85, 0xC0, 0x74, 0x09, 0x39, 0x78, 0x44, 0x75, 0x04, 0x8B, 0xE8,
    0xEB, 0x02, 0x33, 0xED, 0xA1, 0x60, 0x3C, 0x7B, 0x02, 0x85, 0xC0, 0x74, 0x13, 0x83, 0x78, 0x3C,
    0x01, 0x75, 0x0D, 0x8B, 0x88, 0x90, 0x00, 0x00, 0x00, 0x8B, 0x01, 0x8B, 0x50, 0x04, 0xFF, 0xD2,
    0x85, 0xED, 0x0F, 0x84, 0x3B, 0x01, 0x00, 0x00, 0xF6, 0x85, 0x80, 0x00, 0x00, 0x00, 0x01, 0x0F,
    0x85, 0x2E, 0x01, 0x00, 0x00, 0x56, 0xE8, 0x95, 0xEF, 0xFF, 0xFF, 0x8B, 0xF0, 0x85, 0xF6, 0x0F,
    0x84, 0x1D, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xC7, 0x56, 0x66, 0x89, 0x7E, 0x02, 0xE8, 0x4E, 0x29,
    0x00, 0x00, 0x8B, 0x44, 0x24, 0x0C, 0x89, 0x7E, 0x04, 0x89, 0x5E, 0x08, 0x8A, 0x88, 0xB0, 0x01,
    0x00, 0x00, 0x88, 0x0E, 0xC6, 0x86, 0x7C, 0x06, 0x00, 0x00, 0x00, 0x8B, 0x04, 0x9D, 0x84, 0xE2,
    0x12, 0x02, 0x85, 0xC0, 0x75, 0x0E, 0x89, 0xB6, 0x74, 0x06, 0x00, 0x00, 0x89, 0xB6, 0x78, 0x06,
    0x00, 0x00, 0xEB, 0x39, 0x8B, 0x90, 0x78, 0x06, 0x00, 0x00, 0x89, 0xB2, 0x74, 0x06, 0x00, 0x00,
    0x8B, 0x04, 0x9D, 0x84, 0xE2, 0x12, 0x02, 0x8B, 0x88, 0x78, 0x06, 0x00, 0x00, 0x89, 0x8E, 0x78,
    0x06, 0x00, 0x00, 0x8B, 0x14, 0x9D, 0x84, 0xE2, 0x12, 0x02, 0x89, 0xB2, 0x78, 0x06, 0x00, 0x00,
    0x8B, 0x04, 0x9D, 0x84, 0xE2, 0x12, 0x02, 0x89, 0x86, 0x74, 0x06, 0x00, 0x00, 0x89, 0x34, 0x9D,
    0x84, 0xE2, 0x12, 0x02, 0x89, 0xB5, 0xB0, 0x00, 0x00, 0x00, 0x8B, 0x0C, 0x9D, 0x5C, 0xE2, 0x12,
    0x02, 0x8B, 0x11, 0x8B, 0x42, 0x04, 0x57, 0xFF, 0xD0, 0x8B, 0x54, 0x24, 0x10, 0x66, 0xC7, 0x86,
    0x7E, 0x06, 0x00, 0x00, 0xFF, 0xFF, 0x8B, 0x82, 0x00, 0x04, 0x00, 0x00, 0x85, 0xC0, 0x74, 0x4B,
    0xBB, 0x07, 0x00, 0x00, 0x00, 0x39, 0x58, 0x3C, 0x7E, 0x2F, 0x80, 0x38, 0x00, 0x75, 0x2A, 0x8B,
    0x88, 0xF0, 0x21, 0x00, 0x00, 0x89, 0xBC, 0x88, 0xF0, 0x1D, 0x00, 0x00, 0x8A, 0x88, 0xB0, 0x01,
    0x00, 0x00, 0x83, 0x80, 0xF0, 0x21, 0x00, 0x00, 0x01, 0xBD, 0x01, 0x00, 0x00, 0x00, 0xD3, 0xE5,
    0xF7, 0xD5, 0x66, 0x21, 0xAE, 0x7E, 0x06, 0x00, 0x00, 0x8B, 0x80, 0xCC, 0x22, 0x00, 0x00, 0x3B,
    0x82, 0x00, 0x04, 0x00, 0x00, 0x74, 0x04, 0x85, 0xC0, 0x75, 0xBA, 0x80, 0x3D, 0xB6, 0xE9, 0x44,
    0x01, 0x00, 0x74, 0x0D, 0x0F, 0xB7, 0x56, 0x02, 0x52, 0xE8, 0xF2, 0xA8, 0xFF, 0xFF, 0x83, 0xC4,
    0x04, 0x8B, 0xCF, 0xE8, 0x58, 0xFE, 0x11, 0x00, 0x85, 0xC0, 0x74, 0x06, 0x89, 0xB0, 0xB0, 0x00,
    0x00, 0x00, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x08, 0xC3,
];
const B_REGISTER_FROM_CREATE: [u8; 385] = [
    0x51, 0x53, 0x55, 0x57, 0x8B, 0x7C, 0x24, 0x14, 0x8B, 0x47, 0x04, 0x33, 0xDB, 0x3B, 0xC3, 0x7C,
    0x24, 0x83, 0xF8, 0x0A, 0x7D, 0x1F, 0x8B, 0x2C, 0x85, 0x5C, 0xE2, 0x12, 0x02, 0x3B, 0xEB, 0x74,
    0x14, 0x8A, 0x97, 0x08, 0x02, 0x00, 0x00, 0xA1, 0x14, 0x8C, 0x52, 0x02, 0xE8, 0x2F, 0xBB, 0x00,
    0x00, 0x3B, 0xC3, 0x75, 0x07, 0x5F, 0x5D, 0xB0, 0x01, 0x5B, 0x59, 0xC3, 0x0F, 0xB7, 0x0F, 0x89,
    0x5C, 0x24, 0x14, 0x89, 0x5C, 0x24, 0x0C, 0x0F, 0xB6, 0x80, 0xB0, 0x01, 0x00, 0x00, 0x8B, 0x55,
    0x00, 0x8B, 0x52, 0x0C, 0x50, 0x51, 0x8D, 0x47, 0x08, 0x50, 0x8D, 0x4C, 0x24, 0x18, 0x51, 0x8D,
    0x44, 0x24, 0x24, 0x50, 0x8B, 0xCD, 0xC6, 0x05, 0x9D, 0x8B, 0x52, 0x02, 0x01, 0xFF, 0xD2, 0x39,
    0x5C, 0x24, 0x14, 0x88, 0x1D, 0x9D, 0x8B, 0x52, 0x02, 0x75, 0x12, 0x66, 0x8B, 0x07, 0xE8, 0x5D,
    0x0B, 0x00, 0x00, 0x5F, 0x85, 0xC0, 0x5D, 0x0F, 0x95, 0xC0, 0x5B, 0x59, 0xC3, 0x56, 0xE8, 0x9D,
    0xFC, 0xFF, 0xFF, 0x8B, 0xF0, 0x3B, 0xF3, 0x75, 0x08, 0x5E, 0x5F, 0x5D, 0x32, 0xC0, 0x5B, 0x59,
    0xC3, 0x8B, 0x44, 0x24, 0x18, 0x89, 0x46, 0x04, 0x66, 0x8B, 0x0F, 0x66, 0x89, 0x4E, 0x02, 0x8B,
    0x57, 0x04, 0x89, 0x56, 0x08, 0x8A, 0x87, 0x08, 0x02, 0x00, 0x00, 0x88, 0x06, 0x88, 0x9E, 0x7C,
    0x06, 0x00, 0x00, 0x8B, 0x4F, 0x04, 0x39, 0x1C, 0x8D, 0x84, 0xE2, 0x12, 0x02, 0x75, 0x0E, 0x89,
    0xB6, 0x74, 0x06, 0x00, 0x00, 0x89, 0xB6, 0x78, 0x06, 0x00, 0x00, 0xEB, 0x4B, 0x8B, 0xD1, 0x8B,
    0x04, 0x95, 0x84, 0xE2, 0x12, 0x02, 0x8B, 0x88, 0x78, 0x06, 0x00, 0x00, 0x89, 0xB1, 0x74, 0x06,
    0x00, 0x00, 0x8B, 0x57, 0x04, 0x8B, 0x04, 0x95, 0x84, 0xE2, 0x12, 0x02, 0x8B, 0x88, 0x78, 0x06,
    0x00, 0x00, 0x89, 0x8E, 0x78, 0x06, 0x00, 0x00, 0x8B, 0x57, 0x04, 0x8B, 0x04, 0x95, 0x84, 0xE2,
    0x12, 0x02, 0x89, 0xB0, 0x78, 0x06, 0x00, 0x00, 0x8B, 0x4F, 0x04, 0x8B, 0x14, 0x8D, 0x84, 0xE2,
    0x12, 0x02, 0x89, 0x96, 0x74, 0x06, 0x00, 0x00, 0x8B, 0x47, 0x04, 0x8B, 0x4C, 0x24, 0x18, 0x89,
    0x34, 0x85, 0x84, 0xE2, 0x12, 0x02, 0xE8, 0xD5, 0x0B, 0x12, 0x00, 0x3B, 0xC3, 0x74, 0x06, 0x89,
    0xB0, 0xB0, 0x00, 0x00, 0x00, 0x0F, 0xB7, 0x46, 0x02, 0x56, 0xE8, 0xB1, 0x35, 0x00, 0x00, 0x38,
    0x1D, 0xB6, 0xE9, 0x44, 0x01, 0x74, 0x0C, 0x0F, 0xB7, 0x0F, 0x51, 0xE8, 0x40, 0xB6, 0xFF, 0xFF,
    0x83, 0xC4, 0x04, 0x8B, 0x55, 0x00, 0x8B, 0x44, 0x24, 0x18, 0x8B, 0x52, 0x04, 0x50, 0x8B, 0xCD,
    0xFF, 0xD2, 0x8B, 0xCE, 0xE8, 0x07, 0xFB, 0xFF, 0xFF, 0x5E, 0x5F, 0x5D, 0xB0, 0x01, 0x5B, 0x59,
    0xC3,
];
const B_SEND_JOIN_REQ: [u8; 654] = [
    0x81, 0xEC, 0x28, 0x05, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8, 0x00, 0x33, 0xC4, 0x89, 0x84, 0x24,
    0x24, 0x05, 0x00, 0x00, 0x80, 0x3D, 0x4C, 0xBF, 0xE8, 0x00, 0x00, 0x8B, 0x84, 0x24, 0x2C, 0x05,
    0x00, 0x00, 0x53, 0x8B, 0x9C, 0x24, 0x38, 0x05, 0x00, 0x00, 0x55, 0x8B, 0xAC, 0x24, 0x38, 0x05,
    0x00, 0x00, 0x56, 0x57, 0x89, 0x44, 0x24, 0x14, 0xC6, 0x44, 0x24, 0x18, 0x02, 0xBE, 0x03, 0x00,
    0x00, 0x00, 0x74, 0x05, 0xBE, 0x04, 0x00, 0x00, 0x00, 0xB8, 0x48, 0xAA, 0x12, 0x02, 0x8D, 0x48,
    0x01, 0x8A, 0x10, 0x83, 0xC0, 0x01, 0x84, 0xD2, 0x75, 0xF7, 0x2B, 0xC1, 0x0F, 0xB7, 0xF8, 0x0F,
    0xB7, 0xCF, 0x0F, 0xB7, 0xD6, 0x51, 0x8D, 0x44, 0x14, 0x1C, 0x68, 0x48, 0xAA, 0x12, 0x02, 0x50,
    0xE8, 0xAB, 0xAE, 0x48, 0x00, 0x03, 0xF7, 0x8B, 0xBD, 0x08, 0x04, 0x00, 0x00, 0x0F, 0xB7, 0xCE,
    0x83, 0xC6, 0x01, 0x0F, 0xB7, 0xD6, 0x83, 0xC6, 0x04, 0xC6, 0x44, 0x0C, 0x24, 0x00, 0x8B, 0x0D,
    0xE8, 0xC3, 0xE8, 0x00, 0x0F, 0xB7, 0xC6, 0x83, 0xC6, 0x04, 0xC7, 0x44, 0x14, 0x24, 0xC9, 0x00,
    0x00, 0x00, 0x89, 0x4C, 0x04, 0x24, 0x89, 0x74, 0x24, 0x1C, 0x83, 0xC4, 0x0C, 0x8D, 0x77, 0x28,
    0x8D, 0x44, 0x24, 0x10, 0x8D, 0x4C, 0x24, 0x18, 0xE8, 0xE3, 0x59, 0xFE, 0xFF, 0x8B, 0x97, 0x4C,
    0x01, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x10, 0x8B, 0xB7, 0x50, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xC8,
    0x89, 0x54, 0x0C, 0x18, 0x89, 0x74, 0x0C, 0x1C, 0x83, 0xC0, 0x08, 0x89, 0x44, 0x24, 0x10, 0x8B,
    0x97, 0x54, 0x01, 0x00, 0x00, 0x8B, 0xB7, 0x58, 0x01, 0x00, 0x00, 0x0F, 0xB7, 0xC8, 0x89, 0x54,
    0x0C, 0x18, 0x83, 0xC0, 0x08, 0x89, 0x44, 0x24, 0x10, 0x0F, 0xB7, 0xD0, 0x89, 0x74, 0x0C, 0x1C,
    0x8B, 0x8F, 0x70, 0x01, 0x00, 0x00, 0x83, 0xC0, 0x04, 0x89, 0x44, 0x24, 0x10, 0x89, 0x4C, 0x14,
    0x18, 0x8B, 0x4D, 0x0C, 0x8B, 0x11, 0x8B, 0x52, 0x28, 0x8D, 0x44, 0x24, 0x10, 0x50, 0x8D, 0x44,
    0x24, 0x1C, 0x50, 0xFF, 0xD2, 0x8B, 0x74, 0x24, 0x10, 0x8B, 0x45, 0x0C, 0x8A, 0x50, 0x48, 0x0F,
    0xB7, 0xCE, 0x88, 0x54, 0x0C, 0x18, 0x8D, 0x4F, 0x4C, 0x83, 0xC6, 0x01, 0x8B, 0xC1, 0x89, 0x74,
    0x24, 0x10, 0x8D, 0x50, 0x02, 0x66, 0x8B, 0x28, 0x83, 0xC0, 0x02, 0x66, 0x85, 0xED, 0x75, 0xF5,
    0x2B, 0xC2, 0xD1, 0xF8, 0x8D, 0x44, 0x00, 0x02, 0x0F, 0xB7, 0xE8, 0x0F, 0xB7, 0xD5, 0x52, 0x0F,
    0xB7, 0xC6, 0x51, 0x8D, 0x4C, 0x04, 0x20, 0x51, 0xE8, 0xB3, 0xAD, 0x48, 0x00, 0x03, 0xF5, 0x89,
    0x74, 0x24, 0x1C, 0x8A, 0x47, 0x44, 0x0F, 0xB7, 0xD6, 0x83, 0xC6, 0x01, 0x0F, 0xB7, 0xCE, 0x88,
    0x44, 0x14, 0x24, 0x89, 0x74, 0x24, 0x1C, 0x8A, 0x57, 0x45, 0x83, 0xC4, 0x0C, 0x83, 0xC6, 0x01,
    0x85, 0xDB, 0x88, 0x54, 0x0C, 0x18, 0x0F, 0x84, 0xA9, 0x00, 0x00, 0x00, 0x80, 0x3D, 0x56, 0x8B,
    0x52, 0x02, 0x00, 0x74, 0x0D, 0x80, 0x3D, 0x2B, 0x8B, 0x52, 0x02, 0x00, 0x0F, 0x84, 0x93, 0x00,
    0x00, 0x00, 0x80, 0xBB, 0xF8, 0x03, 0x00, 0x00, 0x00, 0x0F, 0x84, 0x86, 0x00, 0x00, 0x00, 0x8A,
    0x43, 0x30, 0x8B, 0xAB, 0x00, 0x04, 0x00, 0x00, 0x0F, 0xB7, 0xCE, 0x2C, 0x01, 0x88, 0x44, 0x0C,
    0x18, 0x83, 0xC6, 0x01, 0x8B, 0xC5, 0x85, 0xC0, 0x89, 0x74, 0x24, 0x10, 0x74, 0x76, 0x8B, 0xFF,
    0x80, 0x38, 0x00, 0x75, 0x50, 0x8B, 0x90, 0x4C, 0x01, 0x00, 0x00, 0x8B, 0xB8, 0x50, 0x01, 0x00,
    0x00, 0x0F, 0xB7, 0xCE, 0x89, 0x54, 0x0C, 0x18, 0x89, 0x7C, 0x0C, 0x1C, 0x83, 0xC6, 0x08, 0x89,
    0x74, 0x24, 0x10, 0x8B, 0x90, 0x54, 0x01, 0x00, 0x00, 0x8B, 0xB8, 0x58, 0x01, 0x00, 0x00, 0x0F,
    0xB7, 0xCE, 0x89, 0x54, 0x0C, 0x18, 0x83, 0xC6, 0x08, 0x89, 0x74, 0x24, 0x10, 0x0F, 0xB7, 0xD6,
    0x89, 0x7C, 0x0C, 0x1C, 0x8B, 0x88, 0x70, 0x01, 0x00, 0x00, 0x83, 0xC6, 0x04, 0x89, 0x4C, 0x14,
    0x18, 0x89, 0x74, 0x24, 0x10, 0x8B, 0x80, 0xCC, 0x22, 0x00, 0x00, 0x3B, 0xC5, 0x74, 0x15, 0x85,
    0xC0, 0x75, 0x9D, 0xEB, 0x0F, 0x0F, 0xB7, 0xD6, 0x83, 0xC6, 0x01, 0xC6, 0x44, 0x14, 0x18, 0x00,
    0x89, 0x74, 0x24, 0x10, 0x6A, 0x00, 0x0F, 0xB7, 0xCE, 0x6A, 0x00, 0x8D, 0x46, 0xFD, 0x8B, 0x74,
    0x24, 0x1C, 0x51, 0x8D, 0x54, 0x24, 0x24, 0x52, 0x66, 0x89, 0x44, 0x24, 0x29, 0xE8, 0xCE, 0xEA,
    0xFF, 0xFF, 0x8B, 0x8C, 0x24, 0x44, 0x05, 0x00, 0x00, 0x83, 0xC4, 0x10, 0x5F, 0x5E, 0x5D, 0x5B,
    0x33, 0xCC, 0xE8, 0x98, 0x04, 0x48, 0x00, 0x81, 0xC4, 0x28, 0x05, 0x00, 0x00, 0xC3,
];

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

// --- round 3: the object core against the original bytes, callees stubbed -----------------------------------------------------
// The whole exe image is mapped at its real addresses, so the original functions run where they are and the Rust ports run
// beside them. Every callee (and every virtual method of the managers / connections) is replaced by a stub that logs its
// registers and arguments and returns a deterministic value; the original and the Rust version run on the same memory (a world of
// sessions, connections, records, objects and managers rebuilt at random for each call) and must produce the same EAX, the same
// memory and the same call log.

#[cfg(test)]
mod tests3 {
    use super::*;
    use std::cell::RefCell;
    use std::ffi::c_void;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

    const HB: usize = 0x2A0_0000;
    const WINDOWS: [(usize, usize); 7] = [
        (0x214_9000, 0x1_1000),
        (0x212_A000, 0x6000),
        (0x252_8000, 0x1000),
        (0x27B_3000, 0x1000),
        (0xE8_8000, 0x5000),
        (0x144_E000, 0x1000),
        (HB, 0x4_0000),
    ];
    const SESS: usize = HB;
    const CONN0: usize = HB + 0x3000;
    const REC0: usize = HB + 0xC000;
    const OBJ0: usize = HB + 0x10000;
    const MGR0: usize = HB + 0x11000;
    const VTM: usize = HB + 0x11400;
    const VTS: usize = HB + 0x11500;
    const VTR: usize = HB + 0x11600;
    const VTO: usize = HB + 0x11700;
    const NODE0: usize = HB + 0x12000;
    const PKT: usize = HB + 0x13000;
    const REQ: usize = HB + 0x14000;
    const REQOBJ: usize = HB + 0x14400;
    const SPECIAL: usize = HB + 0x14800;
    const SPECIAL90: usize = HB + 0x14900;
    const POOLNEXT: usize = HB + 0x15000;

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
        fn chance(&mut self, pct: u32) -> bool {
            self.below(100) < pct
        }
    }

    fn h32(vals: &[u32]) -> u32 {
        let mut h = 0x811C_9DC5u32;
        for v in vals {
            for b in v.to_le_bytes() {
                h = (h ^ b as u32).wrapping_mul(0x0100_0193);
            }
        }
        h
    }

    // ---- the data windows --------------------------------------------------------------------------------------------
    // The game's globals are mapped at their real addresses (the test exe sits at 0x10000000); the code is not: the original
    // functions run from relocated copies whose calls are re-aimed at the stubs, and the Rust ports reach the same stubs through
    // `cal` (the identity outside the tests).
    fn map_windows() {
        for &(a, l) in WINDOWS.iter().chain(&[(0xE8_0000usize, 0x1000usize)]) {
            let base = a & !0xFFFF;
            let len = (a + l - base).div_ceil(0x1_0000) * 0x1_0000;
            unsafe {
                let mut p = VirtualAlloc(base as *const c_void, len, MEM_RESERVE, PAGE_EXECUTE_READWRITE);
                if p.is_null() {
                    p = base as *mut c_void; // reserved by an earlier window of the same 64 KB block
                }
                assert_eq!(p as usize, base, "cannot reserve {base:#x}: error {}", windows_sys::Win32::Foundation::GetLastError());
                let c = VirtualAlloc(a as *const c_void, l, MEM_COMMIT, PAGE_EXECUTE_READWRITE);
                assert_eq!(c as usize, a, "cannot map {a:#x}: error {}", windows_sys::Win32::Foundation::GetLastError());
            }
        }
    }

    pub(super) fn redirect(a: usize) -> usize {
        STUB_ADDR.with(|m| *m.borrow().get(&a).unwrap_or_else(|| panic!("no stub for {a:#x}")))
    }

    /// A relocated copy of an original body: its `call rel32`s (at `calls`) go to the stubs.
    unsafe fn copy_body(body: &[u8], orig: usize, calls: &[usize]) -> usize {
        unsafe {
            let p = VirtualAlloc(std::ptr::null(), body.len() + 16, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) as usize;
            std::ptr::copy_nonoverlapping(body.as_ptr(), p as *mut u8, body.len());
            for &off in calls {
                let rel = i32::from_le_bytes(body[off + 1..off + 5].try_into().unwrap());
                let target = (orig + off + 5).wrapping_add(rel as usize);
                let to = redirect(target);
                let r2 = (to as i32).wrapping_sub((p + off + 5) as i32);
                std::ptr::write_unaligned((p + off + 1) as *mut i32, r2);
            }
            p
        }
    }

    // ---- stubs ---------------------------------------------------------------------------------------------------------
    struct StubInfo {
        addr: usize,
        name: &'static str,
        slot: bool,
    }

    thread_local! {
        static STUBS: RefCell<Vec<StubInfo>> = const { RefCell::new(Vec::new()) };
        static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static STUB_ADDR: RefCell<std::collections::HashMap<usize, usize>> = RefCell::new(std::collections::HashMap::new());
    }

    /// The length word (and the 4th header byte of a 4-byte header) is written after the payload: stack garbage until then.
    fn mask_hdr(data: &mut [u8]) {
        for b in data.iter_mut().skip(1).take(2) {
            *b = 0;
        }
        if MASK3.get() != 0 && unsafe { rb(MASK3.get()) } != 0 && data.len() > 3 {
            data[3] = 0;
        }
    }

    fn log(s: String) {
        LOG.with(|l| l.borrow_mut().push(s));
    }

    fn take_log() -> Vec<String> {
        LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
    }

    extern "C" fn stub_dispatch(id: u32, r: &mut Pushad, a: *const u32) {
        unsafe {
            let (addr, name, slot) = STUBS.with(|s| {
                let s = s.borrow();
                (s[id as usize].addr, s[id as usize].name, s[id as usize].slot)
            });
            let ar = |i: usize| *a.add(i);
            let all = format!("eax={:x} ecx={:x} edx={:x} esi={:x} edi={:x}", r.eax, r.ecx, r.edx, r.esi, r.edi);
            let _ = slot;
            match addr {
                0x00C9_604F => {}
                0x00CA_0850 => {
                    std::ptr::copy_nonoverlapping(ar(1) as *const u8, ar(0) as *mut u8, ar(2) as usize);
                    r.eax = ar(0);
                }
                // net_remote_handle_map_find (EAX = key, ESI = map; key, create)
                0x0080_8E30 => {
                    let key = ar(0);
                    log(format!("{name} eax={:x} esi={:x} key={:x} create={:x}", r.eax, r.esi, key, ar(1)));
                    let i = (key & 15) as usize;
                    r.eax = if rb(NODE0 + 0x100 + i) & 3 != 0 { (NODE0 + i * 8) as u32 } else { 0 };
                }
                // session_find_conn_by_id (EAX = session, DL = owner)
                0x0080_FE80 => {
                    log(format!("{name} eax={:x} dl={:x}", r.eax, r.edx as u8));
                    let i = (r.edx & 3) as usize;
                    r.eax = if rb(SESS + 0x1000 + i) & 3 != 0 { (CONN0 + i * 0x2400) as u32 } else { 0 };
                }
                // net_pool_alloc 0x804050: the next record of the pool
                0x0080_4050 => {
                    log(format!("{name}"));
                    let n = rd(POOLNEXT);
                    wd(POOLNEXT, n + 1);
                    r.eax = if n < 2 && rb(POOLNEXT + 0x10 + n as usize) & 1 != 0 { (REC0 + (7 - n as usize) * 0x700) as u32 } else { 0 };
                }
                // 0x925030: the object of a handle (ECX), or 0
                0x0092_5030 => {
                    log(format!("{name} ecx={:x}", r.ecx));
                    let h = r.ecx;
                    r.eax = if h == 0 {
                        0
                    } else {
                        let o = rd(0x0214_9C64 + ((h & 0xFFF) as usize) * 16);
                        if o != 0 && rd(o as usize + 0x44) == h { o } else { 0 }
                    };
                }
                // 0x806D50 can_migrate: EAX = object
                0x0080_6D50 => {
                    log(format!("{name} eax={:x}", r.eax));
                    r.eax = h32(&[r.eax, 1]) & 0x0100_01FF;
                }
                0x0080_4F00 => {
                    log(format!("{name} ax={:x}", r.eax as u16));
                    r.eax = h32(&[r.eax & 0xFFFF, 2]) & 0xFF00_0001;
                }
                0x0083_6DD0 => {
                    log(format!("{name} ecx={:x}", r.ecx));
                    r.eax = h32(&[r.ecx, 3]) & 0x0101_0102;
                }
                0x0080_44C0 => {
                    log(format!("{name} cx={:x} edx={:x} arg0={:x}", r.ecx as u16, r.edx, ar(0)));
                    r.eax = h32(&[r.ecx & 0xFFFF, 4]);
                }
                0x0080_4710 => {
                    log(format!("{name} ecx*={:x} edi={:x} arg0={:x}", rd(r.ecx as usize), r.edi, ar(0)));
                    r.eax = h32(&[ar(0), 5]);
                }
                0x0080_4140 => {
                    log(format!("{name} esi={:x}", r.esi));
                    r.eax = h32(&[r.esi, 6]);
                }
                0x0080_4960 => {
                    log(format!("{name} eax={:x} edi={:x}", r.eax, r.edi));
                    r.eax = h32(&[r.eax, r.edi, 7]) & 0xFFFF_FF01;
                }
                0x0080_7A20 => {
                    log(format!("{name} ax={:x} arg0={:x}", r.eax as u16, ar(0)));
                    r.eax = h32(&[r.eax, 8]);
                }
                0x0080_7AA0 => {
                    log(format!("{name} eax={:x}", r.eax));
                    r.eax = h32(&[r.eax, 9]);
                }
                0x007F_FB70 | 0x007F_FAC0 => {
                    let nargs = if addr == 0x007F_FAC0 { format!(" arg0={:x}", ar(0)) } else { format!(" eax={:x}", r.eax) };
                    log(format!("{name}{nargs}"));
                    r.eax = h32(&[addr as u32, 10]);
                }
                0x0080_3FA0 => {
                    log(format!("{name} ecx={:x}", r.ecx));
                    r.eax = h32(&[r.ecx, 11]);
                }
                // 0x814670: ESI = connection; (buf, size, 0, 0)
                0x0081_4670 => {
                    let size = ar(1) as usize;
                    let mut data: Vec<u8> = (0..size.min(0x520)).map(|i| rb(ar(0) as usize + i)).collect();
                    if MASK3.get() != 0 && unsafe { rb(MASK3.get()) } != 0 && data.len() > 3 {
                        data[3] = 0; // the sequence byte of a 4-byte header is written by the send helper
                    }
                    log(format!("{name} esi={:x} size={:x} a2={:x} a3={:x} data={:02x?}", r.esi, size, ar(2), ar(3), data));
                    r.eax = h32(&[size as u32, 12]);
                }
                // 0x7FB3D0: EAX = &cursor (word), ECX = buf, ESI = source; writes 12 bytes
                0x007F_B3D0 => {
                    let cur = rw(r.eax as usize) as usize;
                    log(format!("{name} esi={:x} cur={:x}", r.esi, cur));
                    for i in 0..12 {
                        wb(r.ecx as usize + cur + i, (h32(&[r.esi, i as u32]) & 0xFF) as u8);
                    }
                    ww(r.eax as usize, (cur + 12) as u16);
                }
                // ---- virtual methods: fake addresses 0xF0000000 + 0x100 * table + slot ----
                _ if addr >= 0xF000_0000 => {
                    let table = (addr >> 8) & 0xFF;
                    let slot = addr & 0xFF;
                    match (table, slot) {
                        // manager slot 3 (+0xC): (&L1, &L0, pkt + 8, id, owner)
                        (1, 3) => {
                            log(format!("{name} ecx={:x} edx={:x} esi={:x} edi={:x} l1={:x} l0={:x} a2={:x} a3={:x} a4={:x}", r.ecx, r.edx, r.esi, r.edi, rd(ar(0) as usize), rd(ar(1) as usize), ar(2), ar(3), ar(4)));
                            wd(ar(0) as usize, if h32(&[ar(3), 20]) & 1 != 0 { h32(&[ar(3), 21]) | 0x1000 } else { 0 });
                            wd(ar(1) as usize, h32(&[ar(3), 22]));
                            r.eax = h32(&[ar(3), 23]);
                        }
                        // manager slot 5 (+0x14): (handle, &buf, &cursor): writes 6 bytes
                        (1, 5) => {
                            let cur = rd(ar(2) as usize) as usize;
                            let mut data: Vec<u8> = (0..cur).map(|i| rb(ar(1) as usize + i)).collect();
                            mask_hdr(&mut data);
                            log(format!("{name} {all} h={:x} cur={:x} data={:02x?}", ar(0), cur, data));
                            for i in 0..6 {
                                wb(ar(1) as usize + cur + i, (h32(&[ar(0), i as u32]) & 0xFF) as u8);
                            }
                            wd(ar(2) as usize, (cur + 6) as u32);
                            r.eax = h32(&[ar(0), 24]);
                        }
                        // manager slot 6 (+0x18): (handle, &out, pkt + 6)
                        (1, 6) => {
                            log(format!("{name} {all} h={:x} out={:x} a2={:x}", ar(0), rd(ar(1) as usize), ar(2)));
                            wd(ar(1) as usize, 0x77);
                            r.eax = h32(&[ar(0), 25]);
                        }
                        // manager slots 1 / 4 (+4 / +0x10): (handle)
                        (1, 1) | (1, 4) => {
                            log(format!("{name} {all} h={:x}", ar(0)));
                            r.eax = h32(&[ar(0), slot as u32]);
                        }
                        // manager slot 0x14 (+0x50): (handle, conn)
                        (1, 0x14) => {
                            log(format!("{name} {all} h={:x} conn={:x}", ar(0), ar(1)));
                            r.eax = h32(&[ar(0), ar(1), 26]);
                        }
                        // session slots 3 / 4 / 5: no arguments
                        (2, _) | (4, _) => {
                            log(format!("{name} {all}"));
                            r.eax = h32(&[addr as u32, r.ecx, 27]);
                        }
                        // requester slot 0xA (+0x28): (&buf, &cursor): writes 5 bytes
                        (3, 0xA) => {
                            let cur = rd(ar(1) as usize) as usize;
                            let mut data: Vec<u8> = (0..cur).map(|i| rb(ar(0) as usize + i)).collect();
                            mask_hdr(&mut data);
                            log(format!("{name} ecx={:x} edx={:x} esi={:x} edi={:x} cur={:x} data={:02x?}", r.ecx, r.edx, r.esi, r.edi, cur, data));
                            for i in 0..5 {
                                wb(ar(0) as usize + cur + i, (h32(&[r.ecx, i as u32]) & 0xFF) as u8);
                            }
                            wd(ar(1) as usize, (cur + 5) as u32);
                            r.eax = h32(&[r.ecx, 28]);
                        }
                        _ => panic!("unexpected virtual stub {name}"),
                    }
                }
                _ => panic!("unexpected stub {name} ({addr:#x})"),
            }
        }
    }

    unsafe fn make_stub(id: usize, ret: u16) -> usize {
        unsafe {
            let p = VirtualAlloc(std::ptr::null(), 64, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) as usize;
            let mut c: Vec<u8> = vec![0x60, 0x89, 0xE0, 0x8D, 0x4C, 0x24, 0x24, 0x51, 0x50, 0x68];
            c.extend((id as u32).to_le_bytes());
            let call_at = p + c.len();
            c.push(0xE8);
            c.extend(((stub_dispatch as *const () as usize).wrapping_sub(call_at + 5) as u32).to_le_bytes());
            c.extend([0x83, 0xC4, 0x0C, 0x61]);
            if ret == 0 {
                c.push(0xC3);
            } else {
                c.push(0xC2);
                c.extend(ret.to_le_bytes());
            }
            std::ptr::copy_nonoverlapping(c.as_ptr(), p as *mut u8, c.len());
            p
        }
    }

    /// Register a stub for `addr` (a real function: its first bytes become a jump) or a fake slot address.
    unsafe fn stub(name: &'static str, addr: usize, ret: u16) -> usize {
        unsafe {
            let id = STUBS.with(|s| {
                let mut s = s.borrow_mut();
                s.push(StubInfo { addr, name, slot: addr >= 0xF000_0000 });
                s.len() - 1
            });
            let code = make_stub(id, ret);
            STUB_ADDR.with(|m| m.borrow_mut().insert(addr, code));
            code
        }
    }

    fn setup_stubs() {
        thread_local! { static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
        if DONE.get() {
            return;
        }
        DONE.set(true);
        map_windows();
        unsafe {
            for (n, a, r) in [
                ("map_find", 0x0080_8E30usize, 8u16),
                ("find_conn", 0x0080_FE80, 0),
                ("pool_alloc", 0x0080_4050, 0),
                ("obj_of_handle", 0x0092_5030, 0),
                ("can_migrate", 0x0080_6D50, 0),
                ("lookup_id", 0x0080_4F00, 0),
                ("destroy_check", 0x0083_6DD0, 0),
                ("pending_del", 0x0080_44C0, 0),
                ("fallback_send", 0x0080_4710, 0),
                ("unregister", 0x0080_4140, 0),
                ("owner_list_remove", 0x0080_4960, 0),
                ("map_set", 0x0080_7A20, 4),
                ("map_free", 0x0080_7AA0, 0),
                ("snapshot_free", 0x007F_FB70, 0),
                ("snapshot_add", 0x007F_FAC0, 0),
                ("create_ack", 0x0080_3FA0, 0),
                ("send_conn", 0x0081_4670, 0),
                ("write_player", 0x007F_B3D0, 0),
                ("cookie", 0x00C9_604F, 0),
                ("memcpy", 0x00CA_0850, 0),
            ] {
                stub(n, a, r);
            }
            let set = |table: usize, slot: usize, ret: u16, name: &'static str, vt: usize| {
                let fake = 0xF000_0000 + table * 0x100 + slot;
                let code = stub(name, fake, ret);
                wd(vt + slot * 4, code as u32);
            };
            // all the vtables are rewritten with the stub entry points once; the world builder only points objects at them
            for (vt, tab) in [(VTM, 1usize), (VTS, 2), (VTR, 3), (VTO, 4)] {
                match tab {
                    1 => {
                        set(1, 1, 4, "mgr.create", vt);
                        set(1, 3, 0x14, "mgr.read_create", vt);
                        set(1, 4, 4, "mgr.detach", vt);
                        set(1, 5, 0xC, "mgr.write_payload", vt);
                        set(1, 6, 0xC, "mgr.destroy", vt);
                        set(1, 0x14, 8, "mgr.new_owner", vt);
                    }
                    2 => {
                        set(2, 3, 0, "sess.slot3", vt);
                        set(2, 4, 0, "sess.slot4", vt);
                        set(2, 5, 0, "sess.slot5", vt);
                    }
                    3 => set(3, 0xA, 8, "req.write", vt),
                    _ => set(4, 1, 0, "special.slot1", vt),
                }
            }
            // the vtables sit in the world window, which every iteration refills: keep a copy to restore
            VT_COPY.with(|v| {
                let mut v = v.borrow_mut();
                for vt in [VTM, VTS, VTR, VTO] {
                    v.push((vt, (0..0x80).map(|i| rb(vt + i)).collect()));
                }
            });
        }
    }

    thread_local! { static IGNORE_EAX: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
    thread_local! { static MASK3: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
    thread_local! { static VT_COPY: RefCell<Vec<(usize, Vec<u8>)>> = const { RefCell::new(Vec::new()) }; }

    // ---- the world ------------------------------------------------------------------------------------------------------
    fn snapshot() -> Vec<Vec<u8>> {
        WINDOWS.iter().map(|&(a, l)| unsafe { std::slice::from_raw_parts(a as *const u8, l) }.to_vec()).collect()
    }

    fn restore(s: &[Vec<u8>]) {
        for (&(a, _), b) in WINDOWS.iter().zip(s) {
            unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), a as *mut u8, b.len()) };
        }
    }

    fn fill_random(rng: &mut Rng) {
        for &(a, l) in &WINDOWS {
            for i in (0..l).step_by(4) {
                unsafe { wd(a + i, rng.u32()) };
            }
        }
        VT_COPY.with(|v| {
            for (vt, b) in v.borrow().iter() {
                unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), *vt as *mut u8, b.len()) };
            }
        });
    }

    struct World {
        /// handles of the 8 objects
        handles: [u32; 8],
        /// indices of the registered records
        reg: Vec<usize>,
    }

    fn rec(i: usize) -> usize {
        REC0 + i * 0x700
    }
    fn obj(i: usize) -> usize {
        OBJ0 + i * 0x100
    }
    fn conn(i: usize) -> usize {
        CONN0 + i * 0x2400
    }

    /// A consistent world: session, 4 connections (circular list), 8 records (global list / free list / loose), 8 objects with handles,
    /// the 10 managers, the id nodes.
    unsafe fn build_world(rng: &mut Rng) -> World {
        unsafe {
            fill_random(rng);
            // handle table: every slot starts empty
            for i in 0..0x1000usize {
                wd(0x0214_9C64 + i * 16, 0);
            }
            let mut handles = [0u32; 8];
            for j in 0..8 {
                let h = (j as u32 + 1) | ((rng.below(0x200) + 1) << 12);
                handles[j] = h;
                let o = obj(j);
                wd(o + 0x44, h);
                wd(0x0214_9C64 + ((h & 0xFFF) as usize) * 16, o as u32);
                wd(o + 0x80, if rng.chance(30) { rng.u32() } else { rng.u32() & !1 });
                wd(o + 0xB0, 0);
            }
            // session + connections
            wd(0x0252_8C14, if rng.chance(90) { SESS as u32 } else { 0 });
            wd(SESS + 8, rng.u32());
            let order: Vec<usize> = {
                let mut v = vec![0usize, 1, 2, 3];
                for i in (1..4).rev() {
                    v.swap(i, rng.below(i as u32 + 1) as usize);
                }
                v.truncate(1 + rng.below(4) as usize);
                v
            };
            for (k, &c) in order.iter().enumerate() {
                let next = order[(k + 1) % order.len()];
                wd(conn(c) + 0x22CC, conn(next) as u32);
            }
            for c in 0..4 {
                if !order.contains(&c) {
                    wd(conn(c) + 0x22CC, conn(order[rng.below(order.len() as u32) as usize]) as u32);
                }
            }
            wd(SESS + 0x400, if rng.chance(90) { conn(order[0]) as u32 } else { 0 });
            wd(SESS + 0x404, if rng.chance(70) { conn(rng.below(4) as usize) as u32 } else { 0 });
            let local = conn(rng.below(4) as usize);
            wd(SESS + 0x408, local as u32);
            for c in 0..4 {
                let cp = conn(c);
                wd(cp + 0x1AC, SESS as u32);
                wd(cp + 0x3C, if rng.chance(70) { 8 + rng.below(3) } else { rng.below(8) });
                wb(cp, if rng.chance(70) { 0 } else { 1 });
                wb(cp + 0x1B0, rng.below(32) as u8);
                wd(cp + 0x21F0, rng.below(4));
                for k in 0..4 {
                    wd(cp + 0x1DF0 + k * 4, if rng.chance(40) { handles[rng.below(8) as usize] } else { rng.u32() });
                }
                wd(cp + 0x404, if rng.chance(50) { local as u32 } else { conn(rng.below(4) as usize) as u32 });
            }
            // the local connection: its session pointer's +0x404 decides some paths
            for i in 0..4 {
                wb(SESS + 0x1000 + i, rng.u32() as u8);
            }
            // managers: all types, one shared vtable
            for t in 0..11usize {
                let m = MGR0 + t * 0x20;
                wd(m, VTM as u32);
                wd(0x0212_E25C + t * 4, m as u32);
            }
            wd(0x0212_E258, rng.u32());
            // records: state 0 = registered (global + type lists), 1 = free list, 2 = loose
            let mut state = [2u8; 8];
            for s in state.iter_mut() {
                *s = rng.below(3) as u8;
            }
            let reg: Vec<usize> = (0..8).filter(|&i| state[i] == 0).collect();
            let free: Vec<usize> = (0..8).filter(|&i| state[i] == 1).collect();
            for i in 0..8usize {
                let r = rec(i);
                wb(r, rng.u32() as u8);
                ww(r + 2, rng.u32() as u16);
                // a record usually refers to the handle of one of the objects
                wd(r + 4, if rng.chance(75) { handles[rng.below(8) as usize] } else if rng.chance(50) { 0 } else { rng.u32() });
                wd(r + 8, if state[i] != 0 || rng.chance(92) { rng.below(10) } else { rng.u32() });
                wd(r + 0x66C, 0);
                wd(r + 0x670, 0);
                wd(r + 0x674, 0);
                wd(r + 0x678, 0);
                wb(r + 0x67C, rng.u32() as u8);
            }
            // objects point at some records
            for j in 0..8 {
                let k = rng.below(8) as usize;
                if rng.chance(50) && rd(rec(k) + 8) < 10 {
                    wd(obj(j) + 0xB0, rec(k) as u32);
                }
            }
            // nodes: [node] = a record (or 0)
            for i in 0..16usize {
                let n = NODE0 + i * 8;
                let k = rng.below(8) as usize;
                wd(n, if rng.chance(85) && rd(rec(k) + 8) < 10 { rec(k) as u32 } else { 0 });
            }
            let link = |list: &[usize], next: usize, prev: usize| {
                for (k, &i) in list.iter().enumerate() {
                    wd(rec(i) + next, rec(list[(k + 1) % list.len()]) as u32);
                    wd(rec(i) + prev, rec(list[(k + list.len() - 1) % list.len()]) as u32);
                }
            };
            link(&reg, 0x66C, 0x670);
            wd(0x0212_E2AC, if reg.is_empty() { 0 } else { rec(reg[rng.below(reg.len() as u32) as usize]) as u32 });
            link(&free, 0x66C, 0x670);
            wd(0x0212_C254, if free.is_empty() { 0 } else { rec(free[rng.below(free.len() as u32) as usize]) as u32 });
            wd(0x0212_BA34, rng.below(100));
            for t in 0..10usize {
                let mem: Vec<usize> = reg.iter().copied().filter(|&i| rd(rec(i) + 8) == t as u32).collect();
                link(&mem, 0x674, 0x678);
                wd(0x0212_E284 + t * 4, if mem.is_empty() { 0 } else { rec(mem[rng.below(mem.len() as u32) as usize]) as u32 });
            }
            for &i in &reg {
                let t = rd(rec(i) + 8) as usize;
                if t < 11 {
                    wd(0x0212_E25C + t * 4, (MGR0 + t * 0x20) as u32);
                }
            }
            // the pool allocator
            wd(POOLNEXT, 0);
            // flags the functions read
            wb(0x0144_E9B6, rng.u32() as u8);
            wd(0x0252_8BB4, rng.below(3).wrapping_sub(1));
            wd(0x0252_8BAC, rng.below(5));
            wd(0x027B_3C60, if rng.chance(50) { SPECIAL as u32 } else { 0 });
            wd(SPECIAL + 0x3C, rng.below(3));
            wd(SPECIAL + 0x90, SPECIAL90 as u32);
            wd(SPECIAL90, VTO as u32);
            // the packet and the requester
            for i in 0..0x300 {
                wb(PKT + i, rng.u32() as u8);
            }
            wd(PKT + 4, if rng.chance(85) { rng.below(11) } else { rng.u32() });
            wd(REQ + 0x408, SESS as u32);
            wd(REQ + 0xC, REQOBJ as u32);
            wd(REQOBJ, VTR as u32);
            World { handles, reg }
        }
    }

    struct Out {
        eax: u32,
        mem: Vec<Vec<u8>>,
        log: Vec<String>,
    }

    fn compare(what: &str, iter: u32, o: &Out, r: &Out) {
        let skip_eax = IGNORE_EAX.get();
        if (o.eax != r.eax && !skip_eax) || o.log != r.log || o.mem != r.mem {
            let mut why = String::new();
            if o.eax != r.eax && !skip_eax {
                why += &format!("eax: original {:#x}, Rust {:#x}\n", o.eax, r.eax);
            }
            for (k, (x, y)) in o.log.iter().zip(&r.log).enumerate() {
                if x != y {
                    why += &format!("call {k}:\n  original {x}\n  Rust     {y}\n");
                    break;
                }
            }
            if o.log.len() != r.log.len() {
                why += &format!("{} calls in the original, {} in Rust\n", o.log.len(), r.log.len());
            }
            for (w, (x, y)) in WINDOWS.iter().zip(o.mem.iter().zip(&r.mem)) {
                if let Some(i) = (0..x.len()).find(|&i| x[i] != y[i]) {
                    why += &format!("memory differs at {:#x}: original {:#x}, Rust {:#x}\n", w.0 + i, x[i], y[i]);
                }
            }
            panic!("{what} iter {iter}: {why}");
        }
    }

    type Handler = unsafe fn(usize, &mut Pushad, *const u32);

    /// Run the original at `addr` and the Rust handler on the same memory and compare everything.
    unsafe fn differential(what: &str, iter: u32, addr: usize, h: Handler, id: usize, regs: Regs, args: &[u32]) {
        unsafe {
            if iter < 2 || std::env::var("NET3_TRACE").is_ok() {
                eprintln!("{what} iter {iter}");
            }
            let s0 = snapshot();
            take_log();
            let oeax = call_regs(addr, regs.eax, regs.ecx, regs.edx, regs.esi, regs.edi, args);
            if iter < 2 || std::env::var("NET3_TRACE").is_ok() {
                eprintln!("  original done");
            }
            let o = Out { eax: oeax, log: take_log(), mem: snapshot() };
            restore(&s0);
            let mut p = Pushad { edi: regs.edi, esi: regs.esi, ebp: regs.ebp, esp: 0, ebx: regs.ebx, edx: regs.edx, ecx: regs.ecx, eax: regs.eax };
            h(id, &mut p, args.as_ptr());
            let ru = Out { eax: p.eax, log: take_log(), mem: snapshot() };
            restore(&s0);
            compare(what, iter, &o, &ru);
        }
    }

    /// Sometimes no manager for the packet's type.
    unsafe fn null_manager(rng: &mut Rng) {
        unsafe {
            let t = rd(PKT + 4);
            if t < 11 && rng.chance(15) {
                wd(0x0212_E25C + t as usize * 4, 0);
            }
        }
    }

    fn regs(rng: &mut Rng) -> Regs {
        Regs { eax: rng.u32(), ecx: rng.u32(), edx: rng.u32(), ebx: 0xB1B1_B1B1, esi: rng.u32(), edi: rng.u32(), ebp: 0xE9E9_E9E9 }
    }

    /// Makes the test exe link at a free base (see `rust_matches_original`), then:
    /// RUSTFLAGS="-C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release -p sr2-hook --target i686-pc-windows-msvc
    /// object_core_matches_original -- --ignored --nocapture
    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the doc comment)"]
    fn object_core_matches_original() {
        unsafe extern "system" fn on_crash(info: *const windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
            unsafe {
                let rec = &*(*info).ExceptionRecord;
                let c = &*(*info).ContextRecord;
                eprintln!(
                    "CRASH code {:#x} at {:#x} (fault address {:#x}); eax={:x} ecx={:x} edx={:x} esi={:x} edi={:x} ebx={:x} ebp={:x} esp={:x}",
                    rec.ExceptionCode, rec.ExceptionAddress as usize, rec.ExceptionInformation[1], c.Eax, c.Ecx, c.Edx, c.Esi, c.Edi, c.Ebx, c.Ebp, c.Esp
                );
            }
            0
        }
        unsafe { windows_sys::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(Some(on_crash)) };
        setup_stubs();
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        struct Orig {
            dispatch: usize,
            add_field: usize,
            unreg: usize,
            reset: usize,
            destroy: usize,
            new_owner: usize,
            deact: usize,
            reg_local: usize,
            reg_create: usize,
            join_req: usize,
        }
        let orig = unsafe {
            let j = |a: usize, l: &mut [u8]| {
                let _ = (a, &l);
            };
            let _ = j;
            // the jump table of 0x80ED70 holds absolute addresses: re-aim the table and the `jmp [eax*4 + table]`
            let d = copy_body(&B_DISPATCH_ACTION, 0x0080_ED70, &[]);
            let delta = d.wrapping_sub(0x0080_ED70);
            std::ptr::write_unaligned((d + 0xF) as *mut u32, (d + 0x50) as u32);
            for k in 0..6 {
                let e = (d + 0x50 + k * 4) as *mut u32;
                std::ptr::write_unaligned(e, std::ptr::read_unaligned(e).wrapping_add(delta as u32));
            }
            Orig {
                dispatch: d,
                add_field: copy_body(&B_ADD_FIELD, 0x0080_7780, &[]),
                unreg: copy_body(&B_OBJ_UNREGISTER, 0x0080_4140, &[0x8c, 0xd8]),
                reset: copy_body(&B_REGS_RESET, 0x0080_6E30, &[0x28]),
                destroy: copy_body(&B_DESTROY_FROM_PACKET, 0x0080_4550, &[0x37, 0x48, 0x5f, 0x8d]),
                new_owner: copy_body(&B_SEND_NEW_OWNER, 0x0080_67D0, &[0x17, 0x4b, 0x88, 0xde, 0x108]),
                deact: copy_body(&B_DEACTIVATE_SEND, 0x0080_53B0, &[0x107, 0x11f, 0x13c, 0x14d]),
                reg_local: copy_body(&B_REGISTER_LOCAL, 0x0080_5000, &[0xb6, 0xcd, 0x1c9, 0x1d3]),
                reg_create: copy_body(&B_REGISTER_FROM_CREATE, 0x0080_4320, &[0x2c, 0x7e, 0x8e, 0x136, 0x14a, 0x15b, 0x174]),
                join_req: copy_body(&B_SEND_JOIN_REQ, 0x0081_5930, &[0x70, 0xb8, 0x168, 0x26d, 0x282]),
            }
        };
        let iters: u32 = std::env::var("NET3_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(4000);
        unsafe {
            for iter in 0..iters {
                let w = build_world(&mut rng);
                let _ = &w.handles;
                let o = &orig;
                // session_dispatch_action: ESI = a session object whose vtable is VTS
                {
                    let mut g = regs(&mut rng);
                    g.esi = (HB + 0x16000) as u32;
                    wd(HB + 0x16000, VTS as u32);
                    wb(HB + 0x16000 + 0xC00, 0);
                    g.eax = if rng.chance(15) { 0xFF } else { rng.below(8) };
                    differential("dispatch_action", iter, o.dispatch, h_dispatch_action, DISPATCH_ACTION, g, &[]);
                }
                // net_registration_add_field: EDX = registration
                {
                    let reg = HB + 0x18000;
                    let mut g = regs(&mut rng);
                    g.edx = reg as u32;
                    wd(reg + 0x18, if rng.chance(10) { 0x10 + rng.below(3) } else { rng.below(17) });
                    wd(reg + 0x1C, if rng.chance(10) { rng.u32() } else { rng.below(6) });
                    differential("add_field", iter, o.add_field, h_add_field, ADD_FIELD, g, &[rng.u32(), rng.u32(), rng.u32(), rng.u32()]);
                }
                // net_object_unregister: ESI = a record (or null)
                {
                    let mut g = regs(&mut rng);
                    g.esi = if rng.chance(8) || w.reg.is_empty() { 0 } else { rec(w.reg[rng.below(w.reg.len() as u32) as usize]) as u32 };
                    differential("unregister", iter, o.unreg, h_obj_unregister, OBJ_UNREGISTER, g, &[]);
                }
                // net_registrations_reset
                differential("regs_reset", iter, o.reset, h_regs_reset, REGS_RESET, regs(&mut rng), &[]);
                // net_object_destroy_from_packet
                null_manager(&mut rng);
                differential("destroy_from_packet", iter, o.destroy, h_destroy_from_packet, DESTROY_FROM_PACKET, regs(&mut rng), &[PKT as u32]);
                // net_object_send_new_owner
                {
                    wb(PKT + 3, rng.u32() as u8);
                    differential("send_new_owner", iter, o.new_owner, h_send_new_owner, SEND_NEW_OWNER, regs(&mut rng), &[PKT as u32]);
                }
                // net_object_deactivate_send: ECX = handle
                {
                    let mut g = regs(&mut rng);
                    g.ecx = if rng.chance(10) { 0 } else if rng.chance(10) { rng.u32() } else { w.handles[rng.below(8) as usize] };
                    MASK3.set(0x00E8_C084);
                    // handle 0: the original leaves cookie ^ ESP in EAX (no meaning); the port returns the input EAX
                    IGNORE_EAX.set(g.ecx == 0);
                    differential("deactivate_send", iter, o.deact, h_deactivate_send, DEACTIVATE_SEND, g, &[]);
                }
                IGNORE_EAX.set(false);
                // net_object_register_local: EDI = handle, stack: type
                {
                    let mut g = regs(&mut rng);
                    g.edi = if rng.chance(8) { 0 } else if rng.chance(8) { rng.u32() } else { w.handles[rng.below(8) as usize] };
                    for k in 0..2 {
                        wb(POOLNEXT + 0x10 + k, rng.u32() as u8);
                    }
                    let ty = if rng.chance(25) { 6 } else { rng.below(10) };
                    differential("register_local", iter, o.reg_local, h_register_local, REGISTER_LOCAL, g, &[ty]);
                }
                // net_object_register_from_create: stack: packet
                {
                    for k in 0..2 {
                        wb(POOLNEXT + 0x10 + k, rng.u32() as u8);
                    }
                    wb(PKT + 0x208, rng.u32() as u8);
                null_manager(&mut rng);
                    differential("register_from_create", iter, o.reg_create, h_register_from_create, REGISTER_FROM_CREATE, regs(&mut rng), &[PKT as u32]);
                }
                // net_send_join_req: stack: host connection, requester, third
                {
                    let third = if rng.chance(25) { 0 } else { SESS as u32 };
                    for i in 0..0x40 {
                        wb(0x0212_AA48 + i, if i < 12 { b'a' + (rng.u32() % 26) as u8 } else { 0 });
                    }
                    wb(REQOBJ + 0x48, rng.u32() as u8);
                    let pl = HB + 0x2C000;
                    wd(REQ + 0x408, pl as u32);
                    for i in 0..0x30 {
                        ww(pl + 0x4C + i * 2, if i < 5 { 0x41 + rng.below(20) as u16 } else { 0 });
                    }
                    MASK3.set(0x00E8_BF4C);
                    differential("send_join_req", iter, o.join_req, h_send_join_req, SEND_JOIN_REQ, regs(&mut rng), &[conn(0) as u32, REQ as u32, third]);
                }
            }
        }
    }
}
