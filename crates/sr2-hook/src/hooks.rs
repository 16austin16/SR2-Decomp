//! Detours into the original game, installed once `.text` is decrypted.
//!
//! Addresses are for SR2_pc.exe sha256 e20156a0…cfe8 (Steam, AppID 9480);
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
static FRAMES: AtomicU32 = AtomicU32::new(0);

extern "C" fn game_frame_tick_hook() -> i32 {
    let original: extern "C" fn() -> i32 =
        unsafe { std::mem::transmute(FRAME_TICK_ORIGINAL.load(Ordering::Relaxed)) };
    let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n % 3600 == 0 {
        log(&format!("game_frame_tick: frame {n}"));
    }
    if n == FIND_RECORD_VERIFY_FRAME {
        find_record_verify();
    }
    original()
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

/// Globals (docs/analysis/01_packfile.md §4).
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

/// Run the verify pass once the main menu is up (packfiles mounted at boot).
const FIND_RECORD_VERIFY_FRAME: u32 = 300;

static FIND_RECORD_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Set if the verify pass finds a mismatch: the hook then defers to the original.
static FIND_RECORD_FALLBACK: AtomicBool = AtomicBool::new(false);

/// `_stricmp(query, entry_full_name) == 0`, where the full name is `"name.ext"`,
/// or just `name` when ext is NULL or empty. `_stricmp` in the "C" locale folds
/// only ASCII A–Z, as here.
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
                if *e.name != 0 || (!e.ext.is_null() && *e.ext != 0) {
                    if query_matches_entry(query, e) {
                        found = rec;
                    }
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
    });
}
