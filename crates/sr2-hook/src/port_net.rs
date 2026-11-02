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
