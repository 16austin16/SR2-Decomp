//! Detours into the original game, installed once `.text` is decrypted.
//!
//! Addresses are for SR2_pc.exe sha256 e20156a0â€¦cfe8 (Steam, AppID 9480);
//! see docs/functions.csv.

use std::arch::{asm, naked_asm};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use sr2_formats::hash::filename_hash_ptr;

use crate::{detour, log};

// --- 0x520450 game_frame_tick (observe only) -------------------------------

/// cdecl, no args, returns nonzero to quit. Called in a loop by
/// `game_main_thread_proc` (0x520790).
const GAME_FRAME_TICK: usize = 0x0052_0450;
const GAME_FRAME_TICK_PROLOGUE: [u8; 9] = [0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x83, 0xEC, 0x10];

static FRAME_TICK_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Run the frame body in Rust (`frame.rs`) instead of the original.
const RUST_FRAME: bool = true;
static FRAMES: AtomicU32 = AtomicU32::new(0);

extern "C" fn game_frame_tick_hook() -> i32 {
    let original: extern "C" fn() -> i32 =
        unsafe { std::mem::transmute(FRAME_TICK_ORIGINAL.load(Ordering::Relaxed)) };
    let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    if TELEPORT_TEST {
        crate::teleport::tick(n);
    }
    crate::missionlog::tick();
    crate::menu::tick();
    if n == 1 || n.is_multiple_of(3600) {
        log(&format!("game_frame_tick: frame {n}{}", if RUST_FRAME { " (Rust body)" } else { "" }));
    }
    if n > 3600 && n.is_multiple_of(900) {
        log(&crate::actions::summary());
        log(&crate::lua_bindings::summary());
    }
    if n == 900 || n == 4300 {
        crate::actions::stick_self_test();
    }
    if n == FIND_RECORD_VERIFY_FRAME {
        find_record_verify();
        find_entry_verify();
    }
    if n == 1 || n == FIND_RECORD_VERIFY_FRAME || n.is_multiple_of(3600) {
        log(&file_resolve_summary());
        log(&crate::mempool::summary());
        log(&crate::streamman::summary());
        log(&crate::hddio::summary());
        log(&crate::rsched::summary());
        log(&crate::input::summary());
        log(&crate::render::summary());
        log(&crate::gfx_cmd::summary());
        log(&crate::texguard::summary());
        log(&crate::vint_lua::summary());
        log(&crate::controls::summary());
        log(&crate::actions::summary());
        if SURVEY {
            crate::survey::report(&format!("frame {n}"));
        }
    }
    if n.is_multiple_of(60) {
        file_read_log_flush();
        call_logs_flush();
    }
    if n.is_multiple_of(3600) && FIND_ENTRY_SHADOW {
        log(&format!(
            "find_entry live: {} calls, {} mismatches",
            FIND_ENTRY_CALLS.load(Ordering::Relaxed),
            FIND_ENTRY_MISMATCHES.load(Ordering::Relaxed)
        ));
    }
    if RUST_FRAME { crate::frame::game_frame_tick() } else { original() }
}

// --- 0xBF2BD0 hash_filename_ci (replaced) ----------------------------------

/// Custom LTCG convention: EDX = `const char*`, result in EAX.
/// On exit the original leaves ECX = 0 and EDX one past the NUL (or, for a
/// NULL string, EDX = 0 and ECX untouched). The thunk reproduces that, in case
/// a caller relies on it, and preserves every other register.
const HASH_FILENAME_CI: usize = 0x00BF_2BD0;
const HASH_FILENAME_CI_BODY: [u8; 48] = [
    0x85, 0xD2, 0x75, 0x04, 0x83, 0xC8, 0xFF, 0xC3, 0x0F, 0xBE, 0x0A, 0x33, 0xC0, 0x83, 0xC2, 0x01,
    0x85, 0xC9, 0x74, 0x1C, 0x56, 0x8D, 0x71, 0xBF, 0x83, 0xFE, 0x19, 0x77, 0x03, 0x83, 0xC1, 0x20,
    0xC1, 0xC0, 0x06, 0x33, 0xC1, 0x0F, 0xBE, 0x0A, 0x83, 0xC2, 0x01, 0x85, 0xC9, 0x75, 0xE6, 0x5E,
];

extern "C" fn hash_filename_ci_impl(name: *const u8) -> u32 {
    unsafe { filename_hash_ptr(name) }
}

#[unsafe(naked)]
extern "C" fn hash_filename_ci_thunk() {
    naked_asm!(
        "push ecx",
        "push edx",
        "push edx",
        "call {imp}",
        "add esp, 4",
        "pop edx",
        "pop ecx",
        "test edx, edx",
        "jz 3f",
        "2:",
        "cmp byte ptr [edx], 0",
        "lea edx, [edx + 1]",
        "jne 2b",
        "xor ecx, ecx",
        "3:",
        "ret",
        imp = sym hash_filename_ci_impl,
    )
}

/// Call a function with the original's convention (EDX = str).
unsafe fn call_edx_str(f: usize, s: *const u8) -> u32 {
    let r: u32;
    unsafe {
        asm!("call {f}", f = in(reg) f, inout("edx") s => _, out("eax") r, out("ecx") _);
    }
    r
}

/// Run the original and the Rust thunk on the same inputs. Only replace if they agree.
fn hash_self_test() -> bool {
    let cases: [&[u8]; 6] =
        [b"\0", b"patch.vpp_pc\0", b"CUSTOMIZATION_ITEMS.XTBL\0", b"Sr2_City.Chunk_PC\0", b"\xe9t\xffZ[@`{\0", b"a\0"];
    let thunk = hash_filename_ci_thunk as extern "C" fn() as usize;
    for c in cases {
        let (a, b) = unsafe { (call_edx_str(HASH_FILENAME_CI, c.as_ptr()), call_edx_str(thunk, c.as_ptr())) };
        if a != b {
            log(&format!("hash self-test mismatch for {c:?}: original {a:#x}, rust {b:#x}"));
            return false;
        }
    }
    let (a, b) = unsafe { (call_edx_str(HASH_FILENAME_CI, std::ptr::null()), call_edx_str(thunk, std::ptr::null())) };
    a == b
}

// --- 0xC0A3B0 packfile_find_record (replaced, original kept for verify) ----

/// cdecl `(const char* name_ext) -> file_record*`. Whole body pinned below; only
/// the first 7 bytes (`sub esp,10; mov edx,[esp+14]`) are stolen for the trampoline.
const PACKFILE_FIND_RECORD: usize = 0x00C0_A3B0;
const PACKFILE_FIND_RECORD_PROLOGUE_LEN: usize = 7;
const PACKFILE_FIND_RECORD_BODY: [u8; 254] = [
    0x83, 0xEC, 0x10, 0x8B, 0x54, 0x24, 0x14, 0x53, 0x55, 0x56, 0x33, 0xDB, 0x57, 0x89, 0x5C, 0x24,
    0x14, 0xE8, 0x0A, 0x88, 0xFE, 0xFF, 0x8B, 0x0D, 0x4C, 0x89, 0x34, 0x02, 0x8D, 0x79, 0x99, 0x33,
    0xD2, 0x8B, 0xF0, 0xF7, 0xF7, 0x89, 0x74, 0x24, 0x18, 0x8B, 0xEA, 0x2B, 0xCD, 0x85, 0xC9, 0x0F,
    0x8E, 0xB3, 0x00, 0x00, 0x00, 0x8D, 0x04, 0xAD, 0x00, 0x00, 0x00, 0x00, 0x89, 0x44, 0x24, 0x10,
    0xEB, 0x04, 0x8B, 0x74, 0x24, 0x18, 0x8B, 0x0D, 0x48, 0x89, 0x34, 0x02, 0x8B, 0x54, 0x24, 0x10,
    0x8B, 0x3C, 0x0A, 0x85, 0xFF, 0x0F, 0x84, 0x97, 0x00, 0x00, 0x00, 0x3B, 0x37, 0x75, 0x71, 0x64,
    0x8B, 0x0D, 0x2C, 0x00, 0x00, 0x00, 0x8B, 0x09, 0x8B, 0x47, 0x04, 0x8D, 0x71, 0x18, 0x8B, 0x48,
    0x04, 0x85, 0xC9, 0x74, 0x19, 0x80, 0x39, 0x00, 0x74, 0x14, 0x8B, 0x10, 0x51, 0x52, 0x68, 0x3C,
    0x7E, 0xE3, 0x00, 0x56, 0xE8, 0xBD, 0xC4, 0x08, 0x00, 0x83, 0xC4, 0x10, 0xEB, 0x12, 0x8B, 0x08,
    0x8B, 0xD6, 0x8A, 0x01, 0x88, 0x02, 0x83, 0xC1, 0x01, 0x83, 0xC2, 0x01, 0x84, 0xC0, 0x75, 0xF2,
    0x8B, 0xC6, 0x8D, 0x50, 0x01, 0x8A, 0x08, 0x83, 0xC0, 0x01, 0x84, 0xC9, 0x75, 0xF7, 0x2B, 0xC2,
    0x74, 0x36, 0x8B, 0x4F, 0x04, 0xE8, 0xA6, 0x08, 0x00, 0x00, 0x50, 0x8B, 0x44, 0x24, 0x28, 0x50,
    0xE8, 0xBB, 0xEB, 0x08, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x75, 0x04, 0x89, 0x7C, 0x24, 0x14,
    0x8B, 0x0D, 0x4C, 0x89, 0x34, 0x02, 0x83, 0x44, 0x24, 0x10, 0x04, 0x83, 0xC3, 0x01, 0x2B, 0xCD,
    0x3B, 0xD9, 0x0F, 0x8C, 0x5A, 0xFF, 0xFF, 0xFF, 0x5F, 0x5E, 0x5D, 0x33, 0xC0, 0x5B, 0x83, 0xC4,
    0x10, 0xC3, 0x8B, 0x44, 0x24, 0x14, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x10, 0xC3,
];

/// Globals (docs/analysis/01_packfile.md Â§4).
const G_HASH_SLOTS: usize = 0x0234_8948; // *const *const FileRecord
const G_HASH_SLOT_COUNT: usize = 0x0234_894C; // i32
const G_FILE_RECORDS: usize = 0x0234_8958; // *const FileRecord
const G_FILE_RECORD_COUNT: usize = 0x0234_8954; // i32

#[repr(C)]
struct FileRecord {
    hash: u32,
    entry: *const VppEntry,
}

/// Only the two string pointers are needed here (28-byte `vpp_entry`).
#[repr(C)]
struct VppEntry {
    name: *const u8,
    ext: *const u8,
}

/// Count calls per memory-pool vtable slot (crate::survey) and log them with
/// the frame summaries. For finding which methods matter; off normally.
const SURVEY: bool = false;
/// Teleport the player around a loop after a save loads (teleport.rs), to exercise
/// streaming. Test runs only (tools/ingame/run_teleport.ps1): it moves the player.
const TELEPORT_TEST: bool = false;

/// Run the verify pass once the main menu is up (packfiles mounted at boot).
const FIND_RECORD_VERIFY_FRAME: u32 = 300;

static FIND_RECORD_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Set if the verify pass finds a mismatch: the hook then defers to the original.
static FIND_RECORD_FALLBACK: AtomicBool = AtomicBool::new(false);

/// `_stricmp(query, entry_full_name) == 0`, where the full name is `"name.ext"`,
/// or just `name` when ext is NULL or empty. `_stricmp` in the "C" locale folds
/// only ASCII Aâ€“Z, as here.
unsafe fn query_matches_entry(query: *const u8, e: &VppEntry) -> bool {
    let name = unsafe { std::ffi::CStr::from_ptr(e.name.cast()) }.to_bytes();
    let ext: &[u8] = if e.ext.is_null() { b"" } else { unsafe { std::ffi::CStr::from_ptr(e.ext.cast()) }.to_bytes() };
    let mut q = unsafe { std::ffi::CStr::from_ptr(query.cast()) }.to_bytes().iter();
    let mut eat = |part: &[u8]| part.iter().all(|&b| q.next().is_some_and(|&c| c.eq_ignore_ascii_case(&b)));
    let matched = if ext.is_empty() { eat(name) } else { eat(name) && eat(b".") && eat(ext) };
    matched && q.next().is_none()
}

/// Mirrors the original exactly, including two quirks:
/// - the probe is linear from the bucket to the end of the slot array, with no
///   wrap-around, and runs until an empty slot;
/// - if it runs off the end without hitting an empty slot, it returns NULL even
///   when it found a match on the way.
///
/// Entries whose full name is empty are skipped. The last match wins.
unsafe fn find_record(query: *const u8) -> *const FileRecord {
    unsafe {
        let slots = *(G_HASH_SLOTS as *const *const *const FileRecord);
        let hash = filename_hash_ptr(query);
        let count = *(G_HASH_SLOT_COUNT as *const i32);
        let bucket = (hash % (count as u32).wrapping_sub(0x67)) as i32;
        let mut found = std::ptr::null();
        let mut i = 0i32;
        // `count` is re-read each iteration in the original; it does not change mid-call.
        while i < *(G_HASH_SLOT_COUNT as *const i32) - bucket {
            let rec = *slots.add((bucket + i) as usize);
            if rec.is_null() {
                return found;
            }
            if (*rec).hash == hash && !query.is_null() {
                let e = &*(*rec).entry;
                let nonempty = *e.name != 0 || (!e.ext.is_null() && *e.ext != 0);
                if nonempty && query_matches_entry(query, e) {
                    found = rec;
                }
            }
            i += 1;
        }
        std::ptr::null()
    }
}

extern "C" fn packfile_find_record_hook(query: *const u8) -> *const FileRecord {
    if FIND_RECORD_FALLBACK.load(Ordering::Relaxed) {
        let original: extern "C" fn(*const u8) -> *const FileRecord =
            unsafe { std::mem::transmute(FIND_RECORD_ORIGINAL.load(Ordering::Relaxed)) };
        return original(query);
    }
    unsafe { find_record(query) }
}

/// Compare Rust and the original for every registered file (as stored, upper-cased,
/// and with a miss suffix). Runs on the game thread from `game_frame_tick`.
fn find_record_verify() {
    let tramp = FIND_RECORD_ORIGINAL.load(Ordering::Relaxed);
    if tramp == 0 {
        return;
    }
    let original: extern "C" fn(*const u8) -> *const FileRecord = unsafe { std::mem::transmute(tramp) };
    let (records, count) =
        unsafe { (*(G_FILE_RECORDS as *const *const FileRecord), *(G_FILE_RECORD_COUNT as *const i32)) };
    let (mut checked, mut hits, mut mismatches) = (0u32, 0u32, 0u32);
    let mut check = |q: &[u8]| {
        let (a, b) = unsafe { (original(q.as_ptr()), find_record(q.as_ptr())) };
        checked += 1;
        hits += (!a.is_null()) as u32;
        if a != b {
            mismatches += 1;
            if mismatches <= 10 {
                log(&format!(
                    "find_record mismatch for {:?}: original {a:p}, rust {b:p}",
                    String::from_utf8_lossy(&q[..q.len() - 1])
                ));
            }
        }
    };
    for k in 0..count.max(0) as usize {
        let e = unsafe { &*(*records.add(k)).entry };
        let mut full = unsafe { std::ffi::CStr::from_ptr(e.name.cast()) }.to_bytes().to_vec();
        if !e.ext.is_null() && unsafe { *e.ext } != 0 {
            full.push(b'.');
            full.extend_from_slice(unsafe { std::ffi::CStr::from_ptr(e.ext.cast()) }.to_bytes());
        }
        for variant in [full.clone(), full.to_ascii_uppercase(), [&full[..], b"_nope"].concat()] {
            let mut z = variant;
            z.push(0);
            check(&z);
        }
    }
    for q in [&b"\0"[..], b"no_such_file.xtbl\0", b".\0"] {
        check(q);
    }
    if mismatches == 0 {
        log(&format!("find_record verify: {checked} queries over {count} records, {hits} hits, all match"));
    } else {
        FIND_RECORD_FALLBACK.store(true, Ordering::Relaxed);
        log(&format!("find_record verify: {mismatches}/{checked} MISMATCH, falling back to original"));
    }
}

// --- 0xC0B410 packfile_mount (replaced; every call logged) -----------------

/// LTCG: ECX = `const char* path`, returns bool in AL, no stack args.
/// Prologue `sub esp,50; mov eax,[__security_cookie]`: 8 bytes, absolute operands only.
const PACKFILE_MOUNT: usize = 0x00C0_B410;
const PACKFILE_MOUNT_PROLOGUE: [u8; 8] = [0x83, 0xEC, 0x50, 0xA1, 0x50, 0x0B, 0xE8, 0x00];

const G_PACKFILES: usize = 0x0234_8978; // *mut [u8; 0x180]
const G_PACKFILE_COUNT: usize = 0x0234_8970; // i32
const PACKFILE_SIZE: usize = 0x180;

const G_PACKFILE_MAX: usize = 0x0234_896C; // i32
const FILE_OPEN: usize = 0x00BF_DC20; // ECX = path, stack (mode, flag), caller-clean -> file_handle*
const FILE_CLOSE: usize = 0x00BF_DF30; // cdecl (file_handle*)
const PACKFILE_LOAD: usize = 0x00C0_A7E0; // ESI = slot, stack (h, permanent, register), caller-clean -> AL
const STR_RB: usize = 0x00DD_88E4; // the game's "rb" literal

/// Trampoline to the original; the thunk below calls [`MOUNT_IMPL`], which is
/// either this or the Rust port.
static MOUNT_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static MOUNT_IMPL: AtomicUsize = AtomicUsize::new(0);
/// Use the Rust port. The original stays on the trampoline either way.
const MOUNT_USE_RUST: bool = true;

/// `0xBFF700` `path_basename(EDI=path, dst, size)`: copy what follows the last
/// `\` or `/` with `strncpy` semantics, then force `dst[size-1] = 0`.
unsafe fn path_basename(path: *const u8, dst: &mut [u8]) {
    let s = unsafe { cstr(path) };
    let name = match s.iter().rposition(|&c| c == b'\\' || c == b'/') {
        Some(i) => &s[i + 1..],
        None => s,
    };
    let n = name.len().min(dst.len());
    dst[..n].copy_from_slice(&name[..n]);
    dst[n..].fill(0);
    let last = dst.len() - 1;
    dst[last] = 0;
}

/// `0xC0AE90`: the mounted packfile whose name (+0x8) `_stricmp`-matches `name`.
unsafe fn packfile_find_mounted(name: &[u8]) -> Option<*mut u8> {
    unsafe {
        let base = *(G_PACKFILES as *const *mut u8);
        (0..*(G_PACKFILE_COUNT as *const i32)).map(|i| base.add(i as usize * PACKFILE_SIZE)).find(|&pf| {
            cstr(pf.add(8)).eq_ignore_ascii_case(name)
        })
    }
}

// LTCG-compiled callees need not preserve EBX/ESI/EDI/EBP, so these shims save
// all four. ESI and EBP can't be asm operands on x86-32 anyway.

unsafe fn file_open(path: *const u8, mode: *const u8, flag: u32) -> *mut u8 {
    let h: *mut u8;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "push {flag}",
            "push {mode}",
            "call {f}",
            "add esp, 8",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) FILE_OPEN,
            flag = in(reg) flag,
            mode = in(reg) mode,
            in("ecx") path,
            lateout("eax") h,
            clobber_abi("C"),
        );
    }
    h
}

unsafe fn packfile_load(slot: *mut u8, h: *mut u8, permanent: bool, register: bool) -> bool {
    let r: u32;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "push {reg}",
            "push {perm}",
            "push {h}",
            "mov esi, {slot}",
            "call {f}",
            "add esp, 12",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) PACKFILE_LOAD,
            slot = in(reg) slot,
            h = in(reg) h,
            perm = in(reg) permanent as u32,
            reg = in(reg) register as u32,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    r as u8 != 0
}

/// Rust port of `packfile_mount` (0xC0B410), docs/analysis/01_packfile.md Â§5a.
extern "C" fn packfile_mount_rust(path: *const u8) -> u32 {
    unsafe {
        // 0xC0B120: rejects only NULL (its basename length check cannot fail).
        if path.is_null() {
            return 0;
        }
        let count = G_PACKFILE_COUNT as *mut i32;
        if *count >= *(G_PACKFILE_MAX as *const i32) {
            return 0;
        }
        let mut name = [0u8; 0x41];
        path_basename(path, &mut name);
        if packfile_find_mounted(cstr(name.as_ptr())).is_some() {
            return 1;
        }
        let h = file_open(path, STR_RB as *const u8, 0);
        if h.is_null() {
            return 0;
        }
        let slot = (*(G_PACKFILES as *const *mut u8)).add(*count as usize * PACKFILE_SIZE);
        *count += 1;
        // On failure packfile_load closes the handle itself.
        if !packfile_load(slot, h, true, true) {
            *count -= 1;
            return 0;
        }
        let flags = slot.add(0x14C) as *mut u32;
        match *(h.add(0x150) as *const u32) {
            0 => *flags |= 0x8,
            1 => *flags |= 0x10,
            2 => *flags |= 0x4,
            _ => {}
        }
        let close: extern "C" fn(*mut u8) = std::mem::transmute(FILE_CLOSE);
        close(h);
        1
    }
}

/// Our override archive (game folder), mounted right after `patch.vpp_pc` so its
/// files win the last-match lookup. Built by `sr2-mod`.
const OVERRIDE_PACKFILE: &[u8] = b"decomp.vpp_pc\0";

/// [`packfile_mount_rust`], plus the override archive after the patch archive.
extern "C" fn packfile_mount_with_overrides(path: *const u8) -> u32 {
    let r = packfile_mount_rust(path);
    if r != 0 && unsafe { cstr(path) }.eq_ignore_ascii_case(b"patch.vpp_pc") {
        let p = OVERRIDE_PACKFILE.as_ptr();
        if std::path::Path::new("decomp.vpp_pc").exists() {
            let ok = packfile_mount_rust(p);
            packfile_mount_log(p, ok, 0);
        } else {
            log("no decomp.vpp_pc in the game folder; skipping override archive");
        }
    }
    r
}

/// ECX-convention entry for [`packfile_mount_with_overrides`].
#[unsafe(naked)]
extern "C" fn packfile_mount_rust_ecx() {
    naked_asm!("push ecx", "call {f}", "add esp, 4", "ret", f = sym packfile_mount_with_overrides)
}

#[unsafe(naked)]
extern "C" fn packfile_mount_thunk() {
    naked_asm!(
        "push ecx",
        "call dword ptr [{imp}]", // ECX still = path
        "push eax",
        "push dword ptr [esp + 8]", // caller's return address
        "push eax",
        "push dword ptr [esp + 12]", // path
        "call {log}",
        "add esp, 12",
        "pop eax",
        "add esp, 4",
        "ret",
        imp = sym MOUNT_IMPL,
        log = sym packfile_mount_log,
    )
}

// --- 0xC0A970 packfile_mount_temp (every call logged) ----------------------

/// LTCG: ECX = `const char* path`, EAX = `packfile** out_pf`, stack
/// `file_handle** out_handle`, caller-clean, returns bool in AL. Restores ECX.
/// Prologue `push ecx; push ebx; mov ebx,[esp+c]; push esi; push edi`: 8 bytes,
/// stack-relative only, so it runs unchanged on the trampoline.
const PACKFILE_MOUNT_TEMP: usize = 0x00C0_A970;
const PACKFILE_MOUNT_TEMP_PROLOGUE: [u8; 8] = [0x51, 0x53, 0x8B, 0x5C, 0x24, 0x0C, 0x56, 0x57];

const G_TEMP_PACKFILES: usize = 0x0234_8968; // *mut [u8; 0x180]
const G_TEMP_PACKFILE_COUNT: usize = 0x0234_8960; // i32
const G_TEMP_PACKFILE_MAX: usize = 0x0234_895C; // i32
/// `i32[max_temp]`: the file-record count when each temp packfile was mounted
/// (its records are appended after that, and dropped again on unmount).
const G_TEMP_RECORD_MARKS: usize = 0x0234_8964;

static MOUNT_TEMP_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Called by the thunk with the original's convention: the trampoline, or
/// [`packfile_mount_temp_rust_conv`].
static MOUNT_TEMP_IMPL: AtomicUsize = AtomicUsize::new(0);
const MOUNT_TEMP_USE_RUST: bool = true;

/// `0xC0B5E0`: the temp packfile whose name (+0x8) `_stricmp`-matches `path`
/// (the whole argument, not its basename).
unsafe fn packfile_find_temp(path: &[u8]) -> Option<*mut u8> {
    unsafe {
        let base = *(G_TEMP_PACKFILES as *const *mut u8);
        (0..*(G_TEMP_PACKFILE_COUNT as *const i32))
            .map(|i| base.add(i as usize * PACKFILE_SIZE))
            .find(|&pf| cstr(pf.add(8)).eq_ignore_ascii_case(path))
    }
}

/// Rust port of `packfile_mount_temp` (0xC0A970). Unlike `packfile_mount`, it
/// keeps the handle open (returned through `out_handle`) and registers the
/// records without making the packfile permanent.
extern "C" fn packfile_mount_temp_rust(path: *const u8, out_pf: *mut *mut u8, out_handle: *mut *mut u8) -> u32 {
    unsafe {
        // Already mounted: success, and neither output is written.
        if packfile_find_temp(cstr(path)).is_some() {
            return 1;
        }
        let count = G_TEMP_PACKFILE_COUNT as *mut i32;
        if *count >= *(G_TEMP_PACKFILE_MAX as *const i32) {
            return 0;
        }
        let h = file_open(path, STR_RB as *const u8, 1);
        *out_handle = h;
        if h.is_null() {
            return 0;
        }
        let c = *count;
        *(*(G_TEMP_RECORD_MARKS as *const *mut i32)).add(c as usize) = *(G_FILE_RECORD_COUNT as *const i32);
        let slot = (*(G_TEMP_PACKFILES as *const *mut u8)).add(c as usize * PACKFILE_SIZE);
        *out_pf = slot;
        *count = c + 1;
        if !packfile_load(slot, h, false, true) {
            *count -= 1;
            return 0;
        }
        let flags = slot.add(0x14C) as *mut u32;
        match *(h.add(0x150) as *const u32) {
            0 => *flags |= 0x8,
            1 => *flags |= 0x10,
            2 => *flags |= 0x4,
            _ => {}
        }
        1
    }
}

/// Original-convention entry for [`packfile_mount_temp_rust`].
#[unsafe(naked)]
extern "C" fn packfile_mount_temp_rust_conv() {
    naked_asm!(
        "push dword ptr [esp + 4]", // out_handle
        "push eax",                 // out_pf
        "push ecx",                 // path
        "call {f}",
        "add esp, 12",
        "ret",
        f = sym packfile_mount_temp_rust,
    )
}

#[unsafe(naked)]
extern "C" fn packfile_mount_temp_thunk() {
    naked_asm!(
        "push ecx",                  // path
        "push eax",                  // out_pf
        "push dword ptr [esp + 12]", // out_handle
        "call dword ptr [{imp}]",    // EAX/ECX unchanged since entry
        "add esp, 4",
        // [esp] = out_pf, [esp+4] = path, [esp+8] = ret, [esp+12] = out_handle
        "push eax",
        "push dword ptr [esp + 12]", // caller
        "push eax",                  // result
        "push dword ptr [esp + 24]", // out_handle
        "push dword ptr [esp + 16]", // out_pf
        "push dword ptr [esp + 24]", // path
        "call {log}",
        "add esp, 20",
        "pop eax",
        "pop edx",
        "pop ecx", // the original returns with ECX = path
        "ret",
        imp = sym MOUNT_TEMP_IMPL,
        log = sym packfile_mount_temp_log,
    )
}

extern "C" fn packfile_mount_temp_log(
    path: *const u8,
    out_pf: *const *const u8,
    out_handle: *const *const u8,
    result: u32,
    caller: usize,
) {
    unsafe {
        let count = *(G_TEMP_PACKFILE_COUNT as *const i32);
        let mut line = format!(
            "mount_temp {:?} from {caller:#x} -> {} | temp {count}",
            String::from_utf8_lossy(cstr(path)),
            result as u8
        );
        // The outputs are only written on a fresh mount; on "already mounted"
        // they still hold whatever the caller had there, so range-check first.
        let base = *(G_TEMP_PACKFILES as *const *const u8) as usize;
        let pf = *out_pf;
        let in_array = (pf as usize).wrapping_sub(base) < count.max(0) as usize * PACKFILE_SIZE;
        if result as u8 != 0 && in_array {
            let h = *out_handle;
            let source = if h.is_null() { u32::MAX } else { *(h.add(0x150) as *const u32) };
            let mark = *(*(G_TEMP_RECORD_MARKS as *const *const i32)).add((pf as usize - base) / PACKFILE_SIZE);
            line += &format!(" | source {source} mark {mark}{}", packfile_fingerprint(pf));
        }
        log(&line);
    }
}

// --- 0xC0B8E0 packfile_unmount_temp_all (every call logged) ----------------

/// cdecl, no args, no result. Prologue `mov eax,[temp_count]; test eax,eax`:
/// 7 bytes, absolute operand only.
const PACKFILE_UNMOUNT_TEMP_ALL: usize = 0x00C0_B8E0;
const PACKFILE_UNMOUNT_TEMP_ALL_PROLOGUE: [u8; 7] = [0xA1, 0x60, 0x89, 0x34, 0x02, 0x85, 0xC0];

/// The "temp packfile" allocator object (`0xEC5E58`): vtable +0x34 is
/// `bool(int)`, +0x58 `void(int)`, both thiscall; `0xEC5E94` is the mark
/// passed to +0x58.
const TEMP_PACKFILE_ALLOCATOR: usize = 0x00EC_5E58;
const TEMP_PACKFILE_ALLOCATOR_MARK: usize = 0x00EC_5E94;

static UNMOUNT_TEMP_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static UNMOUNT_TEMP_IMPL: AtomicUsize = AtomicUsize::new(0);
const UNMOUNT_TEMP_USE_RUST: bool = true;

/// `0xC0B2A0`: drop a packfile's records from the hash. Its `n` records are
/// the last `n` in the table (temp packfiles are unmounted newest first), and
/// for each one the first slot from its bucket that points at it is cleared.
/// The scan does not stop at empty slots and the table is not compacted, so
/// probe chains through a cleared slot are cut, as in the original.
unsafe fn packfile_unregister_records(pf: *const u8) {
    unsafe {
        let n = *(pf.add(0x154) as *const u32);
        let records = *(G_FILE_RECORDS as *const *const FileRecord);
        let count = *(G_FILE_RECORD_COUNT as *const i32) as usize;
        let slot_count = *(G_HASH_SLOT_COUNT as *const u32);
        let slots = *(G_HASH_SLOTS as *const *mut *const FileRecord);
        for k in 0..n as usize {
            let rec = records.add(count - 1 - k);
            let bucket = (*rec).hash % slot_count.wrapping_sub(0x67);
            for i in 0..(slot_count - bucket) as usize {
                let s = slots.add(bucket as usize + i);
                if *s == rec {
                    *s = std::ptr::null();
                    break;
                }
            }
        }
    }
}

/// Rust port of `0xC0B8E0`: unmount every temp packfile, then reset the temp
/// allocator and clear the temp array. Handles are not closed here (the
/// mount_temp caller owns them).
extern "C" fn packfile_unmount_temp_all_rust() {
    unsafe {
        let count = G_TEMP_PACKFILE_COUNT as *mut i32;
        let base = *(G_TEMP_PACKFILES as *const *mut u8);
        for i in (0..*count).rev() {
            let pf = base.add(i as usize * PACKFILE_SIZE);
            packfile_unregister_records(pf);
            *(G_FILE_RECORD_COUNT as *mut u32) -= *(pf.add(0x154) as *const u32);
        }
        *count = 0;
        let vtable = *(TEMP_PACKFILE_ALLOCATOR as *const *const usize);
        let check: extern "thiscall" fn(usize, u32) -> u8 = std::mem::transmute(*vtable.add(0x34 / 4));
        if check(TEMP_PACKFILE_ALLOCATOR, 0) != 0 {
            let vtable = *(TEMP_PACKFILE_ALLOCATOR as *const *const usize);
            let reset: extern "thiscall" fn(usize, u32) = std::mem::transmute(*vtable.add(0x58 / 4));
            reset(TEMP_PACKFILE_ALLOCATOR, *(TEMP_PACKFILE_ALLOCATOR_MARK as *const u32));
        }
        let max = *(G_TEMP_PACKFILE_MAX as *const i32);
        std::ptr::write_bytes(*(G_TEMP_PACKFILES as *const *mut u8), 0, max as usize * PACKFILE_SIZE);
    }
}

extern "C" fn packfile_unmount_temp_all_hook() {
    let before = unsafe { temp_state() };
    let imp: extern "C" fn() = unsafe { std::mem::transmute(UNMOUNT_TEMP_IMPL.load(Ordering::Relaxed)) };
    imp();
    let after = unsafe { temp_state() };
    log(&format!("unmount_temp_all: before {before} | after {after}"));
}

/// Temp count and names, record count, and an address-independent hash of the
/// hash slots (record index or empty) and of the temp array bytes.
unsafe fn temp_state() -> String {
    unsafe {
        let count = *(G_TEMP_PACKFILE_COUNT as *const i32);
        let base = *(G_TEMP_PACKFILES as *const *const u8);
        let names: Vec<String> = (0..count.max(0) as usize)
            .map(|i| String::from_utf8_lossy(cstr(base.add(i * PACKFILE_SIZE + 8))).into_owned())
            .collect();
        let records = *(G_FILE_RECORDS as *const *const FileRecord) as usize;
        let slots = *(G_HASH_SLOTS as *const *const usize);
        let mut sh = 0x811C_9DC5u32;
        let mut used = 0u32;
        for i in 0..*(G_HASH_SLOT_COUNT as *const u32) as usize {
            let s = *slots.add(i);
            let idx = if s == 0 { u32::MAX } else { ((s - records) / 8) as u32 };
            used += (s != 0) as u32;
            fnv(&mut sh, &idx.to_le_bytes());
        }
        let max = (*(G_TEMP_PACKFILE_MAX as *const i32)).max(0) as usize;
        let mut th = 0x811C_9DC5u32;
        fnv(&mut th, std::slice::from_raw_parts(base, max * PACKFILE_SIZE));
        format!(
            "temp {count} {names:?} records {} slots used {used} #{sh:08x} temp array #{th:08x}",
            *(G_FILE_RECORD_COUNT as *const i32)
        )
    }
}

// --- 0xC0AD80 packfile_find_entry (replaced, original shadow-checked) ------

/// LTCG: ECX = `const char* path`, returns `vpp_entry*` in EAX, no stack args.
/// Prologue `sub esp,48; mov eax,[__security_cookie]`: 8 bytes, absolute operands only.
const PACKFILE_FIND_ENTRY: usize = 0x00C0_AD80;
const PACKFILE_FIND_ENTRY_PROLOGUE: [u8; 8] = [0x83, 0xEC, 0x48, 0xA1, 0x50, 0x0B, 0xE8, 0x00];

/// `packfile*` searched linearly before the hash table (NULL when unset).
const G_PRIORITY_PACKFILE: usize = 0x0234_8944;
const VPP_ENTRY_SIZE: usize = 0x1C;

static FIND_ENTRY_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Also run the original on every call and return its answer if they differ.
/// The function only reads state, so running both is harmless.
const FIND_ENTRY_SHADOW: bool = true;
static FIND_ENTRY_CALLS: AtomicU32 = AtomicU32::new(0);
static FIND_ENTRY_MISMATCHES: AtomicU32 = AtomicU32::new(0);

/// Rust port of `packfile_find_entry`. The original also leaves the last
/// candidate's full name in the thread's scratch string (TLS slot 0 + 0x18), as
/// `packfile_find_record` does; nothing reads it back, so that is not reproduced.
unsafe fn find_entry(path: *const u8) -> *const VppEntry {
    unsafe {
        let mut name = [0u8; 0x40];
        path_basename(path, &mut name);
        let q = name.as_ptr();
        let pf = *(G_PRIORITY_PACKFILE as *const *const u8);
        if !pf.is_null() {
            let entries = *(pf.add(0x170) as *const *const u8);
            for i in 0..*(pf.add(0x154) as *const i32) {
                let e = entries.add(i as usize * VPP_ENTRY_SIZE) as *const VppEntry;
                if query_matches_entry(q, &*e) {
                    return e;
                }
            }
        }
        // Through the hook, so find_record's fallback switch still applies.
        let rec = packfile_find_record_hook(q);
        if rec.is_null() { std::ptr::null() } else { (*rec).entry }
    }
}

/// Call an `ECX = arg -> EAX` function such as the find_entry trampoline.
unsafe fn call_ecx(f: usize, arg: *const u8) -> usize {
    let r: usize;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "call {f}",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) f,
            in("ecx") arg,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    r
}

extern "C" fn packfile_find_entry_hook(path: *const u8) -> *const VppEntry {
    if path.is_null() {
        // The original would fault in path_basename; don't pretend otherwise.
        return std::ptr::null();
    }
    let rust = unsafe { find_entry(path) };
    let original = FIND_ENTRY_ORIGINAL.load(Ordering::Relaxed);
    if FIND_ENTRY_SHADOW && original != 0 {
        FIND_ENTRY_CALLS.fetch_add(1, Ordering::Relaxed);
        let o = unsafe { call_ecx(original, path) } as *const VppEntry;
        if o != rust {
            if FIND_ENTRY_MISMATCHES.fetch_add(1, Ordering::Relaxed) < 10 {
                log(&format!(
                    "find_entry mismatch for {:?}: original {o:p}, rust {rust:p}",
                    String::from_utf8_lossy(unsafe { cstr(path) })
                ));
            }
            return o;
        }
    }
    rust
}

#[unsafe(naked)]
extern "C" fn packfile_find_entry_thunk() {
    naked_asm!("push ecx", "call {f}", "add esp, 4", "ret", f = sym packfile_find_entry_hook)
}

/// Sweep every registered file (bare, under a directory, upper-cased, missing)
/// through the original and the port.
fn find_entry_verify() {
    let tramp = FIND_ENTRY_ORIGINAL.load(Ordering::Relaxed);
    if tramp == 0 {
        return;
    }
    let (records, count) =
        unsafe { (*(G_FILE_RECORDS as *const *const FileRecord), *(G_FILE_RECORD_COUNT as *const i32)) };
    let (mut checked, mut hits, mut mismatches) = (0u32, 0u32, 0u32);
    let mut check = |q: &[u8]| {
        let (a, b) = unsafe { (call_ecx(tramp, q.as_ptr()) as *const VppEntry, find_entry(q.as_ptr())) };
        checked += 1;
        hits += (!a.is_null()) as u32;
        if a != b {
            mismatches += 1;
            if mismatches <= 10 {
                log(&format!(
                    "find_entry verify mismatch for {:?}: original {a:p}, rust {b:p}",
                    String::from_utf8_lossy(&q[..q.len() - 1])
                ));
            }
        }
    };
    for k in 0..count.max(0) as usize {
        let e = unsafe { &*(*records.add(k)).entry };
        let mut full = unsafe { cstr(e.name) }.to_vec();
        if !e.ext.is_null() && unsafe { *e.ext } != 0 {
            full.push(b'.');
            full.extend_from_slice(unsafe { cstr(e.ext) });
        }
        for variant in [
            full.clone(),
            [&b"data\\packfiles/"[..], &full].concat(),
            full.to_ascii_uppercase(),
            [&full[..], b"_nope"].concat(),
        ] {
            let mut z = variant;
            z.push(0);
            check(&z);
        }
    }
    for q in [&b"\0"[..], b"dir\\\0", b"no_such_file.xtbl\0"] {
        check(q);
    }
    let priority = unsafe { *(G_PRIORITY_PACKFILE as *const *const u8) };
    let priority = if priority.is_null() {
        "none".to_string()
    } else {
        format!("{:?}", String::from_utf8_lossy(unsafe { cstr(priority.add(8)) }))
    };
    if mismatches == 0 {
        log(&format!(
            "find_entry verify: {checked} queries, {hits} hits, priority packfile {priority}, all match"
        ));
    } else {
        log(&format!("find_entry verify: {mismatches}/{checked} MISMATCH (live calls return the original's answer)"));
    }
}

// --- 0xBFD880 file_resolve (replaced, original shadow-checked) -------------

/// LTCG: EAX = `location*`, stack `const char* name`, caller-clean, AL = found.
/// Walks the search-path table and fills the location from the first source
/// that has the file. Prologue `sub esp,8; push ebx; mov ebx,[esp+10]`: 8
/// bytes, stack-relative only.
const FILE_RESOLVE: usize = 0x00BF_D880;
const FILE_RESOLVE_PROLOGUE: [u8; 8] = [0x83, 0xEC, 0x08, 0x53, 0x8B, 0x5C, 0x24, 0x10];

/// `{kind: u32, arg: u32}[count]`. Kind 0 = loose file on disk, 1 = packfile,
/// 2 = the indexed lookup at 0xBFD800; any other kind stops the walk.
const G_SEARCH_PATHS: usize = 0x0234_7678;
const G_SEARCH_PATH_COUNT: usize = 0x0252_A540; // u32

/// `0xBFD800`: kind-2 lookup (ESI = location, ECX = name, stack arg). Goes
/// through 0xC1DA80 / 0xC20CA0 and `_stat`; not ported yet.
const FILE_RESOLVE_INDEXED: usize = 0x00BF_D800;

// The game's CRT (cdecl). Kind 0 keeps using it so the FILE* never crosses CRTs.
const CRT_FOPEN: usize = 0x00C9_9705;
const CRT_FSEEK: usize = 0x00C9_8287;
const CRT_FTELLI64: usize = 0x00C9_FC60;
const CRT_FCLOSE: usize = 0x00C9_78DE;

/// `location` (0x110 bytes): kind +0, arg +4, path `char[0x100]` +8, size
/// +0x108, `vpp_entry*` +0x10C.
const LOCATION_SIZE: usize = 0x110;

static FILE_RESOLVE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Run the original on a copy of the location for every call, compare all
/// 0x110 bytes and the result, and keep the original's answer on a mismatch.
/// Kind 0 opens and closes the file twice as a result; nothing else repeats.
const FILE_RESOLVE_SHADOW: bool = true;
static FILE_RESOLVE_CALLS: AtomicU32 = AtomicU32::new(0);
static FILE_RESOLVE_MISMATCHES: AtomicU32 = AtomicU32::new(0);
/// Hits per kind (0, 1, 2) and misses, for the periodic summary.
static FILE_RESOLVE_HITS: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];

/// `strncpy(dst, src, n)`: copy up to `n` bytes, zero-pad the rest of `n`.
unsafe fn strncpy(dst: *mut u8, src: *const u8, n: usize) {
    unsafe {
        let s = cstr(src);
        let k = s.len().min(n);
        std::ptr::copy_nonoverlapping(s.as_ptr(), dst, k);
        std::ptr::write_bytes(dst.add(k), 0, n - k);
    }
}

/// `0xC18130` (EDI = location, stack name, arg): a loose file, if `arg` is 0
/// and `fopen(name, "rb")` works. Kind/arg/path are written before the size
/// probe, and a failed seek or a size of 4 GiB or more leaks the FILE*, as in
/// the original.
unsafe fn resolve_on_disk(loc: *mut u8, name: *const u8, arg: u32) -> bool {
    unsafe {
        if arg != 0 {
            return false;
        }
        let fopen: extern "C" fn(*const u8, *const u8) -> *mut u8 = std::mem::transmute(CRT_FOPEN);
        let fseek: extern "C" fn(*mut u8, i32, i32) -> i32 = std::mem::transmute(CRT_FSEEK);
        let ftelli64: extern "C" fn(*mut u8) -> i64 = std::mem::transmute(CRT_FTELLI64);
        let fclose: extern "C" fn(*mut u8) -> i32 = std::mem::transmute(CRT_FCLOSE);
        let f = fopen(name, STR_RB as *const u8);
        if f.is_null() {
            return false;
        }
        strncpy(loc.add(8), name, 0xFF);
        *loc.add(0x107) = 0;
        *(loc as *mut u32) = 0;
        *(loc.add(4) as *mut u32) = 0;
        if fseek(f, 0, 2) != 0 {
            return false;
        }
        let size = ftelli64(f);
        if size as u64 > 0xFFFF_FFFE {
            return false;
        }
        *(loc.add(0x108) as *mut u32) = size as u32;
        fclose(f);
        true
    }
}

/// `0xBFD7A0` (ESI = location, stack name, arg): the file in a packfile.
unsafe fn resolve_in_packfile(loc: *mut u8, name: *const u8, arg: u32) -> bool {
    unsafe {
        let e = packfile_find_entry_hook(name);
        if e.is_null() {
            return false;
        }
        *(loc.add(0x10C) as *mut *const VppEntry) = e;
        *(loc.add(4) as *mut u32) = arg;
        *(loc as *mut u32) = 1;
        strncpy(loc.add(8), name, 0x100);
        *loc.add(0x107) = 0;
        *(loc.add(0x108) as *mut u32) = *(e.cast::<u8>().add(0x10) as *const u32);
        true
    }
}

unsafe fn resolve_indexed(loc: *mut u8, name: *const u8, arg: u32) -> bool {
    let r: u32;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "mov esi, {loc}",
            "push {arg}",
            "call {f}",
            "add esp, 4",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) FILE_RESOLVE_INDEXED,
            loc = in(reg) loc,
            arg = in(reg) arg,
            in("ecx") name,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    r as u8 != 0
}

/// Rust port of `file_resolve` (0xBFD880). Returns the kind that matched.
unsafe fn file_resolve(loc: *mut u8, name: *const u8) -> Option<u32> {
    unsafe {
        *(loc as *mut i32) = -1;
        *(loc.add(4) as *mut i32) = -1;
        *loc.add(8) = 0;
        *(loc.add(0x108) as *mut u32) = 0;
        *(loc.add(0x10C) as *mut u32) = 0;
        let table = G_SEARCH_PATHS as *const [u32; 2];
        // The count is re-read every iteration, as in the original.
        let mut i = 0;
        while i < *(G_SEARCH_PATH_COUNT as *const u32) {
            let [kind, arg] = *table.add(i as usize);
            let found = match kind {
                0 => resolve_on_disk(loc, name, arg),
                1 => resolve_in_packfile(loc, name, arg),
                2 => resolve_indexed(loc, name, arg),
                _ => return None,
            };
            if found {
                return Some(kind);
            }
            i += 1;
        }
        None
    }
}

/// Call the original's convention: EAX = location, stack name.
unsafe fn call_file_resolve(f: usize, loc: *mut u8, name: *const u8) -> bool {
    let r: u32;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "push {name}",
            "call {f}",
            "add esp, 4",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) f,
            name = in(reg) name,
            inout("eax") loc => r,
            clobber_abi("C"),
        );
    }
    r as u8 != 0
}

extern "C" fn file_resolve_hook(loc: *mut u8, name: *const u8) -> u32 {
    let original = FILE_RESOLVE_ORIGINAL.load(Ordering::Relaxed);
    let mut shadow = [0u8; LOCATION_SIZE];
    let check = FILE_RESOLVE_SHADOW && original != 0;
    if check {
        // Same starting bytes, so identical writes give identical structs.
        unsafe { std::ptr::copy_nonoverlapping(loc, shadow.as_mut_ptr(), LOCATION_SIZE) };
    }
    let kind = unsafe { file_resolve(loc, name) };
    FILE_RESOLVE_HITS[kind.map_or(3, |k| k as usize)].fetch_add(1, Ordering::Relaxed);
    if check {
        FILE_RESOLVE_CALLS.fetch_add(1, Ordering::Relaxed);
        let o = unsafe { call_file_resolve(original, shadow.as_mut_ptr(), name) };
        let same = o == kind.is_some() && unsafe { std::slice::from_raw_parts(loc, LOCATION_SIZE) } == shadow;
        if !same {
            if FILE_RESOLVE_MISMATCHES.fetch_add(1, Ordering::Relaxed) < 10 {
                let rd = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
                let ours = unsafe { std::slice::from_raw_parts(loc, LOCATION_SIZE) };
                log(&format!(
                    "file_resolve mismatch for {:?}: original {o} kind {} size {:#x}, rust {:?} kind {} size {:#x}",
                    String::from_utf8_lossy(unsafe { cstr(name) }),
                    rd(&shadow, 0) as i32,
                    rd(&shadow, 0x108),
                    kind,
                    rd(ours, 0) as i32,
                    rd(ours, 0x108)
                ));
            }
            unsafe { std::ptr::copy_nonoverlapping(shadow.as_ptr(), loc, LOCATION_SIZE) };
            return o as u32;
        }
    }
    kind.is_some() as u32
}

#[unsafe(naked)]
extern "C" fn file_resolve_thunk() {
    naked_asm!(
        "push dword ptr [esp + 4]", // name
        "push eax",                 // location
        "call {f}",
        "add esp, 8",
        "ret",
        f = sym file_resolve_hook,
    )
}

// --- 0xBFDC20 file_open (every call logged to sr2-fileopen.log) -----------

/// LTCG: ECX = `const char* path`, stack `(const char* mode, bool flag)`,
/// caller-clean, returns `file_handle*`. Prologue `push ebp; mov ebp,esp;
/// and esp,-8; sub esp,224`: 12 bytes, no absolute or relative operands.
const FILE_OPEN_PROLOGUE: [u8; 12] = [0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x81, 0xEC, 0x24, 0x02, 0x00, 0x00];

const FILE_HANDLE_ALLOC: usize = 0x00BF_D710; // no args -> file_handle* (pool 0x33D9710, under a lock)
const FILE_HANDLE_FREE: usize = 0x00BF_FEB0; // ECX = handle, stack pool, callee-clean
const FILE_HANDLE_POOL: usize = 0x033D_9710;
const FILE_SOURCE_OPEN: usize = 0x00BF_D930; // ECX = location*, EDX = mode, ESI = handle -> AL

static FILE_OPEN_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Called by the thunk with the original's convention.
static FILE_OPEN_IMPL: AtomicUsize = AtomicUsize::new(0);
const FILE_OPEN_USE_RUST: bool = true;
/// Port write/append opens too (see [`file_open_rust`]).
const FILE_OPEN_WRITE_IN_RUST: bool = true;

/// Call the original's convention: ECX = path, stack (mode, flag), caller-clean.
unsafe fn call_file_open(f: usize, path: *const u8, mode: *const u8, flag: u32) -> *mut u8 {
    let h: *mut u8;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "push {flag}",
            "push {mode}",
            "call {f}",
            "add esp, 8",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) f,
            flag = in(reg) flag,
            mode = in(reg) mode,
            in("ecx") path,
            lateout("eax") h,
            clobber_abi("C"),
        );
    }
    h
}

unsafe fn file_handle_alloc() -> *mut u8 {
    let h: *mut u8;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "call {f}",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) FILE_HANDLE_ALLOC,
            lateout("eax") h,
            clobber_abi("C"),
        );
    }
    h
}

unsafe fn file_handle_free(h: *mut u8) {
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "push {pool}",
            "call {f}", // ret 4
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) FILE_HANDLE_FREE,
            pool = in(reg) FILE_HANDLE_POOL,
            in("ecx") h,
            clobber_abi("C"),
        );
    }
}

unsafe fn file_source_open(loc: *mut u8, mode: *const u8, h: *mut u8) -> bool {
    let r: u32;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "mov esi, {h}",
            "call {f}",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) FILE_SOURCE_OPEN,
            h = in(reg) h,
            in("ecx") loc,
            in("edx") mode,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    r as u8 != 0
}

/// Rust port of `file_open` (0xBFDC20), docs/analysis/01_packfile.md Â§5d.
///
/// `file_handle` (0x16C bytes, pooled): name `char[0x41]` +0, path +0x41,
/// location kind +0x144, type +0x148 (0 OS file, 1 packfile, -1 dead),
/// stream +0x14C, arg +0x150, access +0x154 (1 read, 2 write/append),
/// position +0x15C, size +0x160, flag +0x168.
extern "C" fn file_open_rust(path: *const u8, mode: *const u8, flag: u32) -> *mut u8 {
    unsafe {
        if *(G_SEARCH_PATH_COUNT as *const u32) == 0 {
            return std::ptr::null_mut();
        }
        let m = cstr(mode);
        if m.len() != 2 || !b"rwa".contains(&m[0]) || !b"tb".contains(&m[1]) {
            return std::ptr::null_mut();
        }
        // Writes (saves, settings) have not been exercised in-game yet, so they
        // stay on the original until they are; a bad port there would corrupt saves.
        if !FILE_OPEN_WRITE_IN_RUST && !m.contains(&b'r') {
            return call_file_open(FILE_OPEN_ORIGINAL.load(Ordering::Relaxed), path, mode, flag);
        }
        let mut name = [0u8; 0x101];
        strncpy(name.as_mut_ptr(), path, 0x100);
        let h = file_handle_alloc();
        if h.is_null() {
            return std::ptr::null_mut();
        }
        let mut loc = [0u8; LOCATION_SIZE];
        let l = loc.as_mut_ptr();
        let fail = |h: *mut u8| {
            *(h.add(0x148) as *mut i32) = -1;
            file_handle_free(h);
            std::ptr::null_mut()
        };
        if m.contains(&b'r') {
            *(h.add(0x154) as *mut u32) = 1;
            if file_resolve_hook(l, name.as_ptr()) == 0 {
                return fail(h);
            }
        } else {
            *(h.add(0x154) as *mut u32) = 2;
            // Append keeps an existing file's size; otherwise (or if there is
            // none) it is a fresh disk file.
            if !(m.contains(&b'a') && resolve_on_disk(l, name.as_ptr(), 0)) {
                strncpy(l.add(8), name.as_ptr(), 0xFF);
                *l.add(0x107) = 0;
                *(l as *mut u32) = 0;
                *(l.add(0x108) as *mut u32) = 0;
            }
            *(l.add(4) as *mut u32) = 0;
        }
        *(h.add(0x168) as *mut u32) = (flag as u8 == 1) as u32;
        if !file_source_open(l, mode, h) {
            return fail(h);
        }
        let rd = |o: usize| *(l.add(o) as *const u32);
        let kind = rd(0);
        *(h.add(0x144) as *mut u32) = kind;
        *(h.add(0x148) as *mut u32) = match kind {
            0 | 2 => 0,
            1 => 1,
            // Unreachable (resolve only yields 0-2); the original leaks the handle here.
            _ => return std::ptr::null_mut(),
        };
        *(h.add(0x150) as *mut u32) = rd(4);
        *(h.add(0x160) as *mut u32) = rd(0x108);
        *(h.add(0x15C) as *mut u32) = if *(h.add(0x154) as *const u32) == 2 { rd(0x108) } else { 0 };
        path_basename(name.as_ptr(), std::slice::from_raw_parts_mut(h, 0x41));
        let p = cstr(l.add(8));
        std::ptr::copy_nonoverlapping(p.as_ptr(), h.add(0x41), p.len());
        *h.add(0x41 + p.len()) = 0;
        h
    }
}

/// Original-convention entry for [`file_open_rust`].
#[unsafe(naked)]
extern "C" fn file_open_rust_conv() {
    naked_asm!(
        "push dword ptr [esp + 8]", // flag
        "push dword ptr [esp + 8]", // mode
        "push ecx",                 // path
        "call {f}",
        "add esp, 12",
        "ret",
        f = sym file_open_rust,
    )
}

#[unsafe(naked)]
extern "C" fn file_open_thunk() {
    naked_asm!(
        "push ecx",                 // path, kept for the log
        "push dword ptr [esp + 12]", // flag
        "push dword ptr [esp + 12]", // mode
        "call dword ptr [{imp}]",    // ECX unchanged since entry
        "add esp, 8",
        // [esp] = path, [esp+4] = ret, [esp+8] = mode, [esp+12] = flag
        "push eax",
        "push dword ptr [esp + 8]",  // caller
        "push eax",                  // handle
        "push dword ptr [esp + 24]", // flag
        "push dword ptr [esp + 24]", // mode
        "push dword ptr [esp + 20]", // path
        "call {log}",
        "add esp, 20",
        "pop eax",
        "add esp, 4",
        "ret",
        imp = sym FILE_OPEN_IMPL,
        log = sym file_open_log,
    )
}

static FILE_OPEN_LOG: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);

/// Log every file_open / file_read call to sr2-fileopen.log / sr2-fileread.log
/// for comparing a run of the original against the port. Off once verified:
/// a save load is ~20k opens and ~2.2M reads (about 200 MB of log).
const LOG_FILE_CALLS: bool = false;

/// One line per call to `sr2-fileopen.log`: the inputs and every handle field
/// except the stream pointer (only whether it is set).
extern "C" fn file_open_log(path: *const u8, mode: *const u8, flag: u32, h: *const u8, caller: usize) {
    use std::io::Write as _;
    // Writes (saves, settings) are rare, so they always go to the main log.
    let write = !unsafe { cstr(mode) }.contains(&b'r');
    if !LOG_FILE_CALLS && !write {
        return;
    }
    let mut line = unsafe {
        format!(
            "{caller:#x} {:?} {:?} {} ->",
            String::from_utf8_lossy(cstr(path)),
            String::from_utf8_lossy(cstr(mode)),
            flag as u8
        )
    };
    if h.is_null() {
        line += " NULL";
    } else {
        unsafe {
            let rd = |o: usize| *(h.add(o) as *const u32);
            line += &format!(
                " {:?} {:?} kind {} type {} stream {} arg {} access {} pos {:#x} size {:#x} flag {}",
                String::from_utf8_lossy(cstr(h)),
                String::from_utf8_lossy(cstr(h.add(0x41))),
                rd(0x144) as i32,
                rd(0x148) as i32,
                (rd(0x14C) != 0) as u8,
                rd(0x150),
                rd(0x154),
                rd(0x15C),
                rd(0x160),
                rd(0x168)
            );
        }
    }
    if write {
        log(&format!("file_open (write) {line}"));
        if !LOG_FILE_CALLS {
            return;
        }
    }
    let mut guard = FILE_OPEN_LOG.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = std::fs::OpenOptions::new().create(true).append(true).open("sr2-fileopen.log").ok();
    }
    if let Some(f) = guard.as_mut() {
        let _ = writeln!(f, "{line}");
    }
}

// --- 0xBFE170 file_read (every call logged to sr2-fileread.log) -----------

/// LTCG: EAX = size, stack `(void* dst, file_handle* h)`, caller-clean,
/// returns bytes read. Prologue `mov ecx,[esp+4]; push ebp; mov ebp,[esp+c]`:
/// 9 bytes, stack-relative only.
const FILE_READ: usize = 0x00BF_E170;
const FILE_READ_PROLOGUE: [u8; 9] = [0x8B, 0x4C, 0x24, 0x04, 0x55, 0x8B, 0x6C, 0x24, 0x0C];

const OS_FILE_READ: usize = 0x00C1_8500; // ESI = os_file, ECX = dst, EDI = n, stack (arg, pos), caller-clean
const PACKFILE_STREAM_READ: usize = 0x00C1_8450; // EAX = stream, ECX = arg, stack (dst, n, pos), caller-clean
const CRT_MEMCPY: usize = 0x00CA_0850;

static FILE_READ_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static FILE_READ_IMPL: AtomicUsize = AtomicUsize::new(0);
const FILE_READ_USE_RUST: bool = true;

unsafe fn os_file_read(stream: *mut u8, dst: *mut u8, n: u32, arg: u32, pos: u32) -> u32 {
    let r: u32;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "mov esi, {stream}",
            "mov edi, {n}",
            "push {pos}",
            "push {arg}",
            "call {f}",
            "add esp, 8",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) OS_FILE_READ,
            stream = in(reg) stream,
            n = in(reg) n,
            pos = in(reg) pos,
            arg = in(reg) arg,
            in("ecx") dst,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    r
}

unsafe fn packfile_stream_read(stream: *mut u8, arg: u32, dst: *mut u8, n: u32, pos: u32) -> u32 {
    let r: u32;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "push {pos}",
            "push {n}",
            "push {dst}",
            "call {f}",
            "add esp, 12",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) PACKFILE_STREAM_READ,
            pos = in(reg) pos,
            n = in(reg) n,
            dst = in(reg) dst,
            in("ecx") arg,
            inout("eax") stream => r,
            clobber_abi("C"),
        );
    }
    r
}

/// Rust port of `file_read` (0xBFE170), docs/analysis/01_packfile.md Â§5e.
/// State +0x158: 0 ok, 1 EOF/short read, 3 one-shot read done (flag bit 8).
extern "C" fn file_read_rust(n: u32, dst: *mut u8, h: *mut u8) -> u32 {
    unsafe {
        let field = |o: usize| h.add(o) as *mut u32;
        if *field(0x158) != 0 {
            return 0;
        }
        if *field(0x154) != 1 {
            *field(0x158) = 1;
            return 0;
        }
        let (pos, size) = (*field(0x15C), *field(0x160));
        if pos >= size {
            *field(0x158) = 1;
            return 0;
        }
        if n == 0 {
            return 0;
        }
        let mut n = n;
        if pos.wrapping_add(n) > size {
            n = size - pos;
            *field(0x158) = 1;
        }
        let stream = *field(0x14C) as *mut u8;
        let got = match *field(0x148) {
            0 => os_file_read(stream, dst, n, *field(0x150), pos),
            1 => {
                // stream = {.., +4 vpp_entry*}; entry +0x18 = its packfile. A
                // packfile with flag 1 (compressed) can only be read whole.
                let entry = *(stream.add(4) as *const *const u8);
                let pf = *(entry.add(0x18) as *const *const u8);
                if *(pf.add(0x14C) as *const u32) & 1 != 0 {
                    if n != size {
                        0
                    } else {
                        let packed = *(entry.add(0x14) as *const u32);
                        packfile_stream_read(stream, *field(0x150), dst, packed, pos)
                    }
                } else {
                    packfile_stream_read(stream, *field(0x150), dst, n, pos)
                }
            }
            2 => {
                let memcpy: extern "C" fn(*mut u8, *const u8, u32) -> *mut u8 = std::mem::transmute(CRT_MEMCPY);
                memcpy(dst, stream.add(pos as usize), n);
                n
            }
            _ => return 0,
        };
        *field(0x15C) = pos.wrapping_add(got);
        if *field(0x168) & 8 != 0 {
            // 0xBFDB20: clear the one-shot bit on the handle and its stream.
            *field(0x168) &= !8;
            match *field(0x148) {
                0 => *(stream.add(8) as *mut u32) &= !8,
                1 => *((*(stream as *const *mut u8)).add(8) as *mut u32) &= !8,
                _ => {}
            }
            *field(0x158) = 3;
        } else if got != n {
            *field(0x158) = 1;
        }
        got
    }
}

#[unsafe(naked)]
extern "C" fn file_read_rust_conv() {
    naked_asm!(
        "push dword ptr [esp + 8]", // h
        "push dword ptr [esp + 8]", // dst
        "push eax",                 // n
        "call {f}",
        "add esp, 12",
        "ret",
        f = sym file_read_rust,
    )
}

#[unsafe(naked)]
extern "C" fn file_read_thunk() {
    naked_asm!(
        "push eax",                  // n, kept for the log
        "push dword ptr [esp + 4]",  // pos before, filled in below
        "mov ecx, [esp + 16]",       // h
        "mov ecx, [ecx + 0x15c]",
        "mov [esp], ecx",
        "push dword ptr [esp + 16]", // h
        "push dword ptr [esp + 16]", // dst
        "call dword ptr [{imp}]",    // EAX = n unchanged
        "add esp, 8",
        // [esp] = pos, [esp+4] = n, [esp+8] = ret, [esp+12] = dst, [esp+16] = h
        "push eax",
        "push eax",                  // got
        "push dword ptr [esp + 8]",  // pos before
        "push dword ptr [esp + 16]", // n
        "push dword ptr [esp + 28]", // dst
        "push dword ptr [esp + 36]", // h
        "call {log}",
        "add esp, 20",
        "pop eax",
        "add esp, 8",
        "ret",
        imp = sym FILE_READ_IMPL,
        log = sym file_read_log,
    )
}

static FILE_READ_LOG: std::sync::Mutex<Option<std::io::BufWriter<std::fs::File>>> = std::sync::Mutex::new(None);

/// One line per call: handle name, request, position, result, state and
/// position afterwards, and a hash of the bytes read.
extern "C" fn file_read_log(h: *const u8, dst: *const u8, n: u32, pos: u32, got: u32) {
    use std::io::Write as _;
    if !LOG_FILE_CALLS {
        return;
    }
    let line = unsafe {
        let rd = |o: usize| *(h.add(o) as *const u32);
        let data = std::slice::from_raw_parts(dst, if dst.is_null() { 0 } else { got as usize });
        let mut x = 0xCBF2_9CE4_8422_2325u64;
        let (words, tail) = data.as_chunks::<8>();
        for w in words {
            x = (x ^ u64::from_le_bytes(*w)).wrapping_mul(0x0100_0000_01B3);
        }
        for &b in tail {
            x = (x ^ b as u64).wrapping_mul(0x0100_0000_01B3);
        }
        format!(
            "{:?} type {} n {n:#x} pos {pos:#x} -> {got:#x} state {} pos {:#x} data {x:016x}",
            String::from_utf8_lossy(cstr(h)),
            rd(0x148) as i32,
            rd(0x158),
            rd(0x15C)
        )
    };
    let mut guard = FILE_READ_LOG.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("sr2-fileread.log")
            .ok()
            .map(std::io::BufWriter::new);
    }
    if let Some(f) = guard.as_mut() {
        let _ = writeln!(f, "{line}");
    }
}

/// Flush the buffered read log (called from the frame tick).
fn file_read_log_flush() {
    use std::io::Write as _;
    if let Some(f) = FILE_READ_LOG.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = f.flush();
    }
}

// --- 0xBFE360 file_write and 0xBFDF30 file_close --------------------------

/// LTCG: ESI = handle, EDI = n, EDX = src, returns bytes written; ESI/EDI
/// preserved. Prologue `test edi,edi; jne +3; xor eax,eax; ret`: the jne is
/// relative, so this one is replaced outright (body pinned below).
const FILE_WRITE: usize = 0x00BF_E360;
const FILE_WRITE_BODY: [u8; 0x9F] = [
    0x85, 0xFF, 0x75, 0x03, 0x33, 0xC0, 0xC3, 0x83, 0xBE, 0x58, 0x01, 0x00, 0x00, 0x00, 0x75, 0xF4, 0x53, 0xBB, 0x02,
    0x00, 0x00, 0x00, 0x33, 0xC0, 0x39, 0x9E, 0x54, 0x01, 0x00, 0x00, 0x74, 0x08, 0x89, 0x9E, 0x58, 0x01, 0x00, 0x00,
    0x5B, 0xC3, 0x39, 0x9E, 0x48, 0x01, 0x00, 0x00, 0x75, 0x38, 0x8B, 0x8E, 0x5C, 0x01, 0x00, 0x00, 0x55, 0x8D, 0x2C,
    0x39, 0x3B, 0xAE, 0x64, 0x01, 0x00, 0x00, 0x5D, 0x76, 0x06, 0x89, 0x9E, 0x58, 0x01, 0x00, 0x00, 0x39, 0x86, 0x58,
    0x01, 0x00, 0x00, 0x75, 0x28, 0x8B, 0x86, 0x4C, 0x01, 0x00, 0x00, 0x57, 0x52, 0x03, 0xC1, 0x50, 0xE8, 0x8F, 0x24,
    0x0A, 0x00, 0x83, 0xC4, 0x0C, 0x8B, 0xC7, 0xEB, 0x1B, 0x8B, 0x8E, 0x4C, 0x01, 0x00, 0x00, 0x51, 0x8B, 0xCF, 0xE8,
    0x9A, 0xA1, 0x01, 0x00, 0x83, 0xC4, 0x04, 0x3B, 0xC7, 0x74, 0x06, 0x89, 0x9E, 0x58, 0x01, 0x00, 0x00, 0x01, 0x86,
    0x5C, 0x01, 0x00, 0x00, 0x8B, 0x8E, 0x5C, 0x01, 0x00, 0x00, 0x39, 0x8E, 0x60, 0x01, 0x00, 0x00, 0x73, 0x06, 0x89,
    0x8E, 0x60, 0x01, 0x00, 0x00, 0x5B, 0xC3,
];
const OS_FILE_WRITE: usize = 0x00C1_8570; // ECX = n, EDX = src, stack os_file*, caller-clean -> bytes (WriteFile)
/// Offsets of the body's two `call rel32` (memcpy, os_file_write), fixed up
/// when the original is relocated for the observe run.
const FILE_WRITE_CALLS: [usize; 2] = [0x5C, 0x71];
/// The relocated original (see [`detour::relocate`]).
static FILE_WRITE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
const FILE_WRITE_USE_RUST: bool = true;

/// Call the original's convention: ESI = h, EDI = n, EDX = src.
unsafe fn call_file_write(f: usize, h: *mut u8, n: u32, src: *const u8) -> u32 {
    let r: u32;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "mov esi, {h}",
            "mov edi, {n}",
            "call {f}",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) f,
            h = in(reg) h,
            n = in(reg) n,
            in("edx") src,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    r
}

/// cdecl `(file_handle*) -> bool`. Prologue `push -1; push <SEH handler>;
/// mov eax,fs:[0]`: 13 bytes, absolute operands only.
const FILE_CLOSE_PROLOGUE: [u8; 13] = [0x6A, 0xFF, 0x68, 0x08, 0x3A, 0xDA, 0x00, 0x64, 0xA1, 0x00, 0x00, 0x00, 0x00];
const FILE_CLOSE_LOCK: usize = 0x033D_A2F0; // CRITICAL_SECTION
const IAT_ENTER_CRITICAL_SECTION: usize = 0x00DB_80CC;
const IAT_LEAVE_CRITICAL_SECTION: usize = 0x00DB_80D0;
const PACKFILE_STREAM_CLOSE: usize = 0x00C1_8400; // EAX = arg, EDI = stream -> AL
const OS_FILE_CLOSE: usize = 0x00C1_84C0; // ESI = os_file -> AL
const CRT_FREE: usize = 0x00C9_6BAD;

static FILE_CLOSE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static FILE_CLOSE_IMPL: AtomicUsize = AtomicUsize::new(0);
const FILE_CLOSE_USE_RUST: bool = true;

/// Log every file_write / file_close to sr2-filewrite.log / sr2-fileclose.log
/// (a save load is ~20k closes, a save a handful of writes). Off once verified.
const LOG_WRITE_CLOSE_CALLS: bool = false;

unsafe fn os_file_write(stream: *mut u8, src: *const u8, n: u32) -> u32 {
    let r: u32;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "push {stream}",
            "call {f}",
            "add esp, 4",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) OS_FILE_WRITE,
            stream = in(reg) stream,
            in("ecx") n,
            in("edx") src,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    r
}

/// Rust port of `file_write` (0xBFE360), docs/analysis/01_packfile.md Â§5g.
/// State +0x158 = 2 marks a write error; memory files have a capacity at +0x164.
extern "C" fn file_write_rust(h: *mut u8, n: u32, src: *const u8) -> u32 {
    unsafe {
        let field = |o: usize| h.add(o) as *mut u32;
        if n == 0 || *field(0x158) != 0 {
            return 0;
        }
        if *field(0x154) != 2 {
            *field(0x158) = 2;
            return 0;
        }
        let pos = *field(0x15C);
        let got = if *field(0x148) == 2 {
            if pos.wrapping_add(n) > *field(0x164) {
                *field(0x158) = 2;
            }
            if *field(0x158) == 0 {
                let memcpy: extern "C" fn(*mut u8, *const u8, u32) -> *mut u8 = std::mem::transmute(CRT_MEMCPY);
                memcpy((*field(0x14C) as *mut u8).add(pos as usize), src, n);
                n
            } else {
                0
            }
        } else {
            let got = os_file_write(*field(0x14C) as *mut u8, src, n);
            if got != n {
                *field(0x158) = 2;
            }
            got
        };
        *field(0x15C) = pos.wrapping_add(got);
        if *field(0x160) < *field(0x15C) {
            *field(0x160) = *field(0x15C);
        }
        got
    }
}

#[unsafe(naked)]
extern "C" fn file_write_thunk() {
    naked_asm!(
        "push edx", // src
        "push edi", // n
        "push esi", // h
        "call {f}",
        "add esp, 12",
        "ret",
        f = sym file_write_hook,
    )
}

extern "C" fn file_write_hook(h: *mut u8, n: u32, src: *const u8) -> u32 {
    let pos = unsafe { *(h.add(0x15C) as *const u32) };
    let got = if FILE_WRITE_USE_RUST {
        file_write_rust(h, n, src)
    } else {
        unsafe { call_file_write(FILE_WRITE_ORIGINAL.load(Ordering::Relaxed), h, n, src) }
    };
    if LOG_WRITE_CLOSE_CALLS {
        unsafe {
            let rd = |o: usize| *(h.add(o) as *const u32);
            let data = std::slice::from_raw_parts(src, if src.is_null() { 0 } else { got as usize });
            let mut x = 0x811C_9DC5u32;
            fnv(&mut x, data);
            call_log(
                1,
                &format!(
                    "{:?} type {} n {n:#x} pos {pos:#x} -> {got:#x} state {} pos {:#x} size {:#x} data {x:08x}",
                    String::from_utf8_lossy(cstr(h)),
                    rd(0x148) as i32,
                    rd(0x158),
                    rd(0x15C),
                    rd(0x160)
                ),
            );
        }
    }
    got
}

unsafe fn packfile_stream_close(arg: u32, stream: *mut u8) -> bool {
    let r: u32;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "mov edi, {stream}",
            "call {f}",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) PACKFILE_STREAM_CLOSE,
            stream = in(reg) stream,
            inout("eax") arg => r,
            clobber_abi("C"),
        );
    }
    r as u8 != 0
}

unsafe fn os_file_close(stream: *mut u8) -> bool {
    let r: u32;
    unsafe {
        asm!(
            "push ebx",
            "push esi",
            "push edi",
            "push ebp",
            "mov esi, {stream}",
            "call {f}",
            "pop ebp",
            "pop edi",
            "pop esi",
            "pop ebx",
            f = in(reg) OS_FILE_CLOSE,
            stream = in(reg) stream,
            lateout("eax") r,
            clobber_abi("C"),
        );
    }
    r as u8 != 0
}

/// Rust port of `file_close` (0xBFDF30): close the source by type, reset the
/// handle and return it to the pool, all under the close lock. An unknown type
/// returns false and leaves the handle allocated, as in the original.
extern "C" fn file_close_rust(h: *mut u8) -> u32 {
    unsafe {
        let enter: extern "system" fn(usize) = std::mem::transmute(*(IAT_ENTER_CRITICAL_SECTION as *const usize));
        let leave: extern "system" fn(usize) = std::mem::transmute(*(IAT_LEAVE_CRITICAL_SECTION as *const usize));
        let field = move |o: usize| h.add(o) as *mut u32;
        enter(FILE_CLOSE_LOCK);
        let stream = *field(0x14C) as *mut u8;
        let ok = match *field(0x148) {
            0 => *field(0x150) == 0 && os_file_close(stream),
            1 => packfile_stream_close(*field(0x150), stream),
            2 => {
                // Memory files opened for writing own their buffer, unless flag 0x10.
                if *field(0x154) == 2 && !stream.is_null() && *field(0x168) & 0x10 == 0 {
                    let free: extern "C" fn(*mut u8) = std::mem::transmute(CRT_FREE);
                    free(stream);
                    *field(0x14C) = 0;
                }
                true
            }
            _ => {
                leave(FILE_CLOSE_LOCK);
                return 0;
            }
        };
        *h = 0;
        *h.add(0x41) = 0;
        for (o, v) in [
            (0x144, u32::MAX),
            (0x150, u32::MAX),
            (0x154, 0),
            (0x158, 0),
            (0x15C, 0),
            (0x160, 0),
            (0x164, u32::MAX),
            (0x168, 0),
            (0x14C, 0),
            (0x148, u32::MAX),
        ] {
            *field(o) = v;
        }
        file_handle_free(h);
        leave(FILE_CLOSE_LOCK);
        ok as u32
    }
}

extern "C" fn file_close_hook(h: *mut u8) -> u32 {
    let before = if LOG_WRITE_CLOSE_CALLS {
        unsafe {
            let rd = |o: usize| *(h.add(o) as *const u32);
            format!(
                "{:?} type {} access {} arg {} flag {:#x} size {:#x}",
                String::from_utf8_lossy(cstr(h)),
                rd(0x148) as i32,
                rd(0x154),
                rd(0x150) as i32,
                rd(0x168),
                rd(0x160)
            )
        }
    } else {
        String::new()
    };
    let imp: extern "C" fn(*mut u8) -> u32 = unsafe { std::mem::transmute(FILE_CLOSE_IMPL.load(Ordering::Relaxed)) };
    let r = imp(h) as u8;
    if LOG_WRITE_CLOSE_CALLS {
        call_log(2, &format!("{before} -> {r}"));
    }
    r as u32
}

static CALL_LOGS: std::sync::Mutex<[Option<std::io::BufWriter<std::fs::File>>; 3]> =
    std::sync::Mutex::new([None, None, None]);
const CALL_LOG_NAMES: [&str; 3] = ["sr2-unused.log", "sr2-filewrite.log", "sr2-fileclose.log"];

fn call_log(which: usize, line: &str) {
    use std::io::Write as _;
    let mut logs = CALL_LOGS.lock().unwrap_or_else(|e| e.into_inner());
    let slot = &mut logs[which];
    if slot.is_none() {
        *slot = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(CALL_LOG_NAMES[which])
            .ok()
            .map(std::io::BufWriter::new);
    }
    if let Some(f) = slot.as_mut() {
        let _ = writeln!(f, "{line}");
    }
}

fn call_logs_flush() {
    use std::io::Write as _;
    for f in CALL_LOGS.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().flatten() {
        let _ = f.flush();
    }
}

fn search_path_table() -> String {
    unsafe {
        let n = *(G_SEARCH_PATH_COUNT as *const u32);
        let table = G_SEARCH_PATHS as *const [u32; 2];
        let items: Vec<String> = (0..n.min(16)).map(|i| format!("{:?}", *table.add(i as usize))).collect();
        format!("{n} [{}]", items.join(", "))
    }
}

fn file_resolve_summary() -> String {
    let h = |i: usize| FILE_RESOLVE_HITS[i].load(Ordering::Relaxed);
    format!(
        "file_resolve live: {} checked, {} mismatches | disk {} packfile {} indexed {} missing {} | search paths {}",
        FILE_RESOLVE_CALLS.load(Ordering::Relaxed),
        FILE_RESOLVE_MISMATCHES.load(Ordering::Relaxed),
        h(0),
        h(1),
        h(2),
        h(3),
        search_path_table()
    )
}

fn fnv(h: &mut u32, bytes: &[u8]) {
    for &b in bytes {
        *h = (*h ^ b as u32).wrapping_mul(0x0100_0193);
    }
}

pub(crate) unsafe fn cstr<'a>(p: *const u8) -> &'a [u8] {
    if p.is_null() { b"<null>" } else { unsafe { std::ffi::CStr::from_ptr(p.cast()) }.to_bytes() }
}

/// One line per call: inputs, result, and a fingerprint of everything the mount
/// writes (packfile slot fields, directory, file-record table).
extern "C" fn packfile_mount_log(path: *const u8, result: u32, caller: usize) {
    unsafe {
        let count = *(G_PACKFILE_COUNT as *const i32);
        let mut line = format!(
            "mount {:?} from {caller:#x} -> {} | packfiles {count}",
            String::from_utf8_lossy(cstr(path)),
            result as u8
        );
        if result as u8 != 0 && count > 0 {
            let pf = (*(G_PACKFILES as *const *const u8)).add((count as usize - 1) * PACKFILE_SIZE);
            line += " | last";
            line += &packfile_fingerprint(pf);
        }
        log(&line);
    }
}

/// The packfile's header fields, a hash of its directory, and a hash of the
/// global file-record table.
unsafe fn packfile_fingerprint(pf: *const u8) -> String {
    unsafe {
        let rd = |o: usize| *(pf.add(o) as *const u32);
        let mut h = 0x811C_9DC5u32;
        let entries = rd(0x170) as *const u32;
        for i in 0..rd(0x154) as usize {
            let e = entries.add(i * 7);
            fnv(&mut h, cstr(*e as *const u8));
            fnv(&mut h, b".");
            fnv(&mut h, cstr(*e.add(1) as *const u8));
            for k in 2..6 {
                fnv(&mut h, &(*e.add(k)).to_le_bytes());
            }
            fnv(&mut h, &[(*e.add(6) as *const u8 == pf) as u8]);
        }
        let (recs, nrec) = (*(G_FILE_RECORDS as *const *const FileRecord), *(G_FILE_RECORD_COUNT as *const i32));
        let mut rh = 0x811C_9DC5u32;
        for i in 0..nrec.max(0) as usize {
            fnv(&mut rh, &(*recs.add(i)).hash.to_le_bytes());
        }
        format!(
            " {:?} flags {:#x} data {:#x} n {} dir {:#x} names {:#x} ext {:#x} entries#{h:08x} | records {nrec} #{rh:08x}",
            String::from_utf8_lossy(cstr(pf.add(8))),
            rd(0x14C),
            rd(0x150),
            rd(0x154),
            rd(0x15C),
            rd(0x160),
            rd(0x164)
        )
    }
}

/// Install all hooks. Safe to call repeatedly; runs once.
pub fn install_all() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        match detour::install(
            GAME_FRAME_TICK,
            &GAME_FRAME_TICK_PROLOGUE,
            game_frame_tick_hook as extern "C" fn() -> i32 as usize,
        ) {
            Ok(tramp) => {
                FRAME_TICK_ORIGINAL.store(tramp, Ordering::Relaxed);
                log("hooked game_frame_tick @ 0x520450");
            }
            Err(e) => log(&format!("hook game_frame_tick failed: {e:?}")),
        }

        let current = std::slice::from_raw_parts(HASH_FILENAME_CI as *const u8, HASH_FILENAME_CI_BODY.len());
        if current != HASH_FILENAME_CI_BODY {
            log("hash_filename_ci: unexpected bytes, not replacing");
        } else if !hash_self_test() {
            log("hash_filename_ci: self-test failed, not replacing");
        } else {
            match detour::replace(
                HASH_FILENAME_CI,
                &HASH_FILENAME_CI_BODY,
                hash_filename_ci_thunk as extern "C" fn() as usize,
            ) {
                Ok(()) => log("replaced hash_filename_ci @ 0xbf2bd0 with Rust (self-test passed)"),
                Err(e) => log(&format!("replace hash_filename_ci failed: {e:?}")),
            }
        }

        let current =
            std::slice::from_raw_parts(PACKFILE_FIND_RECORD as *const u8, PACKFILE_FIND_RECORD_BODY.len());
        if current != PACKFILE_FIND_RECORD_BODY {
            log("packfile_find_record: unexpected bytes, not replacing");
        } else {
            match detour::install(
                PACKFILE_FIND_RECORD,
                &PACKFILE_FIND_RECORD_BODY[..PACKFILE_FIND_RECORD_PROLOGUE_LEN],
                packfile_find_record_hook as extern "C" fn(*const u8) -> *const FileRecord as usize,
            ) {
                Ok(tramp) => {
                    FIND_RECORD_ORIGINAL.store(tramp, Ordering::Relaxed);
                    log("replaced packfile_find_record @ 0xc0a3b0 with Rust (verify at frame 300)");
                }
                Err(e) => log(&format!("hook packfile_find_record failed: {e:?}")),
            }
        }

        match detour::install(
            PACKFILE_FIND_ENTRY,
            &PACKFILE_FIND_ENTRY_PROLOGUE,
            packfile_find_entry_thunk as extern "C" fn() as usize,
        ) {
            Ok(tramp) => {
                FIND_ENTRY_ORIGINAL.store(tramp, Ordering::Relaxed);
                log("replaced packfile_find_entry @ 0xc0ad80 with Rust (shadow-checked, verify at frame 300)");
            }
            Err(e) => log(&format!("hook packfile_find_entry failed: {e:?}")),
        }

        crate::crash::install();
        crate::mempool::install();
        crate::streamman::install();
        crate::hddio::install();
        crate::rsched::install();
        crate::input::install();
        crate::render::install();
        crate::gfx_cmd::install();
        crate::texguard::install();
        crate::vint_lua::install();
        crate::frame::install();
        crate::controls::install();
        crate::actions::install();
        crate::lua_bindings::install();
        if SURVEY {
            crate::survey::install(); // wraps whatever the slots hold, so after the ports
        }
        // Logs the callers of unnamed virtual methods; last, so ported slots are skipped.
        #[cfg(feature = "caller-survey")]
        crate::callers::install();
    });
}

/// Hooks on code that runs before the game's first `DirectInput8Create` (the
/// boot packfile mounts). Spawned from `DllMain`: spins until SteamStub has
/// decrypted the target bytes, then patches before the game reaches them.
pub fn install_early() {
    let start = std::time::Instant::now();
    let decrypted = || unsafe {
        std::slice::from_raw_parts(PACKFILE_MOUNT as *const u8, PACKFILE_MOUNT_PROLOGUE.len()) == PACKFILE_MOUNT_PROLOGUE
    };
    while !decrypted() {
        if start.elapsed().as_secs() >= 30 {
            log("install_early: packfile_mount bytes never matched, not hooking");
            return;
        }
        std::thread::yield_now();
    }
    let waited = start.elapsed();
    // The thunk goes live as soon as the jmp is written, so set its target first.
    // (In observe mode the trampoline address is only known afterwards; the game
    // does not mount within that window, since boot mounts start hundreds of ms later.)
    if MOUNT_USE_RUST {
        MOUNT_IMPL.store(packfile_mount_rust_ecx as extern "C" fn() as usize, Ordering::Relaxed);
    }
    match unsafe {
        detour::install(PACKFILE_MOUNT, &PACKFILE_MOUNT_PROLOGUE, packfile_mount_thunk as extern "C" fn() as usize)
    } {
        Ok(tramp) => {
            MOUNT_ORIGINAL.store(tramp, Ordering::Relaxed);
            let which = if MOUNT_USE_RUST {
                "Rust"
            } else {
                MOUNT_IMPL.store(tramp, Ordering::Relaxed);
                "original, observed"
            };
            log(&format!("hooked packfile_mount @ 0xc0b410 ({which}), {waited:?} after attach"));
        }
        Err(e) => log(&format!("hook packfile_mount failed: {e:?}")),
    }

    // Same section, so already decrypted. Same ordering caveat as above.
    if MOUNT_TEMP_USE_RUST {
        MOUNT_TEMP_IMPL.store(packfile_mount_temp_rust_conv as extern "C" fn() as usize, Ordering::Relaxed);
    }
    match unsafe {
        detour::install(
            PACKFILE_MOUNT_TEMP,
            &PACKFILE_MOUNT_TEMP_PROLOGUE,
            packfile_mount_temp_thunk as extern "C" fn() as usize,
        )
    } {
        Ok(tramp) => {
            MOUNT_TEMP_ORIGINAL.store(tramp, Ordering::Relaxed);
            let which = if MOUNT_TEMP_USE_RUST {
                "Rust"
            } else {
                MOUNT_TEMP_IMPL.store(tramp, Ordering::Relaxed);
                "original, observed"
            };
            log(&format!("hooked packfile_mount_temp @ 0xc0a970 ({which})"));
        }
        Err(e) => log(&format!("hook packfile_mount_temp failed: {e:?}")),
    }

    UNMOUNT_TEMP_IMPL.store(
        if UNMOUNT_TEMP_USE_RUST {
            packfile_unmount_temp_all_rust as extern "C" fn() as usize
        } else {
            0
        },
        Ordering::Relaxed,
    );
    match unsafe {
        detour::install(
            PACKFILE_UNMOUNT_TEMP_ALL,
            &PACKFILE_UNMOUNT_TEMP_ALL_PROLOGUE,
            packfile_unmount_temp_all_hook as extern "C" fn() as usize,
        )
    } {
        Ok(tramp) => {
            UNMOUNT_TEMP_ORIGINAL.store(tramp, Ordering::Relaxed);
            let which = if UNMOUNT_TEMP_USE_RUST {
                "Rust"
            } else {
                UNMOUNT_TEMP_IMPL.store(tramp, Ordering::Relaxed);
                "original, observed"
            };
            log(&format!("hooked packfile_unmount_temp_all @ 0xc0b8e0 ({which})"));
        }
        Err(e) => log(&format!("hook packfile_unmount_temp_all failed: {e:?}")),
    }

    // file_write: relocate the original first (the hook may call it), then replace.
    match unsafe { detour::relocate(FILE_WRITE, &FILE_WRITE_BODY, &FILE_WRITE_CALLS) } {
        Ok(copy) => {
            FILE_WRITE_ORIGINAL.store(copy, Ordering::Relaxed);
            match unsafe { detour::replace(FILE_WRITE, &FILE_WRITE_BODY, file_write_thunk as extern "C" fn() as usize) } {
                Ok(()) => log(&format!(
                    "hooked file_write @ 0xbfe360 ({})",
                    if FILE_WRITE_USE_RUST { "Rust" } else { "original, relocated, observed" }
                )),
                Err(e) => log(&format!("hook file_write failed: {e:?}")),
            }
        }
        Err(e) => log(&format!("relocate file_write failed: {e:?}")),
    }

    if FILE_CLOSE_USE_RUST {
        FILE_CLOSE_IMPL.store(file_close_rust as extern "C" fn(*mut u8) -> u32 as usize, Ordering::Relaxed);
    }
    match unsafe {
        detour::install(FILE_CLOSE, &FILE_CLOSE_PROLOGUE, file_close_hook as extern "C" fn(*mut u8) -> u32 as usize)
    } {
        Ok(tramp) => {
            FILE_CLOSE_ORIGINAL.store(tramp, Ordering::Relaxed);
            let which = if FILE_CLOSE_USE_RUST {
                "Rust"
            } else {
                FILE_CLOSE_IMPL.store(tramp, Ordering::Relaxed);
                "original, observed"
            };
            log(&format!("hooked file_close @ 0xbfdf30 ({which})"));
        }
        Err(e) => log(&format!("hook file_close failed: {e:?}")),
    }

    // Every file_open at boot goes through here, so hook it before the mounts.
    match unsafe { detour::install(FILE_RESOLVE, &FILE_RESOLVE_PROLOGUE, file_resolve_thunk as extern "C" fn() as usize) } {
        Ok(tramp) => {
            FILE_RESOLVE_ORIGINAL.store(tramp, Ordering::Relaxed);
            log(&format!(
                "replaced file_resolve @ 0xbfd880 with Rust (shadow-checked), search paths {}",
                search_path_table()
            ));
        }
        Err(e) => log(&format!("hook file_resolve failed: {e:?}")),
    }

    if FILE_OPEN_USE_RUST {
        FILE_OPEN_IMPL.store(file_open_rust_conv as extern "C" fn() as usize, Ordering::Relaxed);
    }
    match unsafe { detour::install(FILE_OPEN, &FILE_OPEN_PROLOGUE, file_open_thunk as extern "C" fn() as usize) } {
        Ok(tramp) => {
            FILE_OPEN_ORIGINAL.store(tramp, Ordering::Relaxed);
            let which = if FILE_OPEN_USE_RUST {
                "Rust"
            } else {
                FILE_OPEN_IMPL.store(tramp, Ordering::Relaxed);
                "original, observed"
            };
            log(&format!("hooked file_open @ 0xbfdc20 ({which}), calls in sr2-fileopen.log"));
        }
        Err(e) => log(&format!("hook file_open failed: {e:?}")),
    }

    if FILE_READ_USE_RUST {
        FILE_READ_IMPL.store(file_read_rust_conv as extern "C" fn() as usize, Ordering::Relaxed);
    }
    match unsafe { detour::install(FILE_READ, &FILE_READ_PROLOGUE, file_read_thunk as extern "C" fn() as usize) } {
        Ok(tramp) => {
            FILE_READ_ORIGINAL.store(tramp, Ordering::Relaxed);
            let which = if FILE_READ_USE_RUST {
                "Rust"
            } else {
                FILE_READ_IMPL.store(tramp, Ordering::Relaxed);
                "original, observed"
            };
            log(&format!("hooked file_read @ 0xbfe170 ({which}), calls in sr2-fileread.log"));
        }
        Err(e) => log(&format!("hook file_read failed: {e:?}")),
    }
}
