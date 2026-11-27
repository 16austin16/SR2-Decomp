//! tod_light_sets_parse 0x4C8B80 (2897 bytes): the loader of the time-of-day light sets (night lights,
//! the clothes store's light set, cutscene lights). Hand-ported from the disassembly.
//!
//! Input: ECX = the light-set file name, EDX = the pool object to allocate from (0: the default pool
//! at 0x2652998; the cutscene code passes 0x2771178). Output: EAX = the cache slot (16 bytes at
//! 0x2652198 + 16 * index: hash, sets array, set count, pool mark) or 0. The function parses the
//! xtbl tree with the original xtbl helpers (called by address with the registers the LTCG build
//! sets up) and fills, per set, 0x50 bytes (`rec`), per light 0x90 bytes (`L`), per ambient grid
//! 0x78 bytes (`G`):
//!
//! - rec: +0 name hash, +4 StartTime, +8 EndTime, +0xC lights, +0x10 light count, +0x14 byte 0,
//!   +0x18 grids, +0x1C grid count.
//! - L: +0 name copy, +4 name hash, +8 u16 type (0 omni, 2 circular spotlight), +0xC colour,
//!   +0x18 Multiplier, +0x1C colour * Multiplier, +0x28 flags, +0x2C Position (copied to +0x38),
//!   +0x5C orientation * (0,-1,0), +0x68 Hotspot, +0x70 Attenuation, +0x78 Slate_Name hash, +0x7C byte 0.
//! - G: see `grid`.
//!
//! What the earlier machine-lifted version (port_tables.rs) got wrong, from the disassembly: the
//! xtbl value getters 0xB747D0 / 0xB74EF0 copy the child's string into a 0x400-byte buffer at the
//! bottom of their own frame and parse it WITHOUT checking that the child exists. When the child is
//! missing they parse whatever string the previous getter call left in that buffer (all four getters
//! share one stack depth, and the caller's stack depth is the same at every call site). So an absent
//! LightCharacter / ShadowCharacter / LightLevel / ShadowLevel inherits the integer parse of the
//! Multiplier string ("1.5" -> 1: all four flag bits set) and then of the flag before it. The lift
//! ran the callees from its own fake frame, where that buffer held other bytes (and its fake frame is
//! zero-filled), so the light flags came out different. `call_stale` poisons the callee's buffer with
//! the same string. (The other things the lift could have lost, ECX across quat_to_mat33 -> vec3_mul_mat33,
//! are kept explicitly here: ECX = the matrix for both calls.)

#![allow(unsafe_op_in_unsafe_fn)]
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::portlib::*;
use crate::log;

// The offline differential test keeps the game's globals (0xD00000..0x350000) at a fixed offset (their
// addresses are taken in a test process); the port reads them through these shadows there.
#[cfg(test)]
pub(crate) const DELTA: usize = 0x5000_0000;
#[cfg(test)]
pub(crate) fn tr(p: usize) -> usize {
    if (0xD0_0000..0x350_0000).contains(&p) { p + DELTA } else { p }
}
#[cfg(test)]
unsafe fn rd(p: usize) -> u32 {
    unsafe { *(tr(p) as *const u32) }
}
#[cfg(test)]
unsafe fn rf(p: usize) -> f32 {
    unsafe { *(tr(p) as *const f32) }
}
#[cfg(test)]
unsafe fn wd(p: usize, v: u32) {
    unsafe { *(tr(p) as *mut u32) = v }
}
#[cfg(test)]
unsafe fn wb(p: usize, v: u8) {
    unsafe { *(tr(p) as *mut u8) = v }
}
#[cfg(test)]
unsafe fn wf(p: usize, v: f32) {
    unsafe { *(tr(p) as *mut f32) = v }
}
/// Every call into the game goes through the recording stubs in the offline test.
#[cfg(test)]
unsafe fn call_regs(f: usize, eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32, args: &[u32]) -> u32 {
    unsafe { crate::portlib::call_regs(difftest::redirect(f), eax, ecx, edx, esi, edi, args) }
}
/// A game address passed to a callee as a value.
#[cfg(test)]
fn ga(p: usize) -> u32 {
    tr(p) as u32
}
#[cfg(not(test))]
fn ga(p: usize) -> u32 {
    p as u32
}

unsafe fn ww(p: usize, v: u16) {
    unsafe { *(p as *mut u16) = v }
}

// --- the game's addresses --------------------------------------------------------------------------------

const TOD: usize = 0x4C8B80;
const CACHE_COUNT: usize = 0x0252_6D44;
const CACHE_BASE: usize = 0x0265_2198;
const DEFAULT_POOL: usize = 0x0265_2998;
const SCOPE_POOL: usize = 0x00E8_7138; // the pool object b743f0 left (ECX), whose vtable +0x34 ends the scope
const SCOPE_MARK: usize = 0x00E8_7134; // what its vtable +0x30 returned
const C_DEFAULT: usize = 0x00E8_7998; // -1.0f, the "unset" value
const DEFAULT_SLATE: usize = 0x033D_6308;

const F_PARSE_SCRIPT: usize = 0xBFD880; // EAX = out buffer (0x110 bytes), stack = file name, AL
const F_HASH: usize = 0xBDCB30; // ECX = string, stack = out ptr (ret 4), seed in EAX
const F_HASH_SLATE: usize = 0xBDCA30; // EAX = out ptr, EDX = seed, stack = string (ret 4)
const F_ROOT: usize = 0xB743F0; // EAX = file name, ECX = 0 -> root node
const F_FIND: usize = 0xBFBEE0; // EAX = node, EDI = name -> child or 0
const F_COUNT: usize = 0xBFC110; // EAX = node, stack = name -> number of children with that name
const F_STRICMP: usize = 0xC99030;
const F_SSCANF: usize = 0xC97089;
const F_STRNCPY: usize = 0xC990F0;
const F_QUAT: usize = 0xC057D0; // EAX = quaternion, ECX = out 3x3; keeps EAX/ECX/EDX
const F_VMUL: usize = 0xBDDE20; // ECX = matrix, EAX = out vec3, stack = vec3 (ret 4) -> EAX
const F_GET_INT: usize = 0xB747D0; // ECX = name, ESI = dest, stack = node
const F_GET_FLOAT: usize = 0xB74EF0; // same; float
const F_GET_FLOAT_OK: usize = 0xB74F60; // same; AL = found
const F_GET_INT_OK: usize = 0xB74860; // same; AL = found

// Strings (the exe's .rdata).
const S_LIGHTSET: usize = 0xDDC690;
const S_STARTTIME: usize = 0xDDC69C;
const S_ENDTIME: usize = 0xDDC6A8;
const S_LIGHTS: usize = 0xDDC6B0;
const S_LIGHT: usize = 0xDDC6B8;
const S_OMNI: usize = 0xDDC6C0;
const S_SPOT: usize = 0xDDC6C8;
const S_COLOR: usize = 0xDDC6DC;
const S_FMT3: usize = 0xDDC6E4;
const S_MULTIPLIER: usize = 0xDDC6F0;
const S_LIGHTCHARACTER: usize = 0xDDC6FC;
const S_SHADOWCHARACTER: usize = 0xDDC70C;
const S_LIGHTLEVEL: usize = 0xDDC71C;
const S_SHADOWLEVEL: usize = 0xDDC728;
const S_POSITION: usize = 0xDDC734;
const S_ORIENTATION: usize = 0xDDC740;
const S_FMT4: usize = 0xDDC74C;
const S_HOTSPOT: usize = 0xDDC758;
const S_FMT2: usize = 0xDDC760;
const S_ATTENUATION: usize = 0xDDC768;
const S_SLATE: usize = 0xDDC774;
const S_AGRIDS: usize = 0xDDC780;
const S_AGRID: usize = 0xDDC790;
const S_LEVEL_AMB: usize = 0xDDC7A0;
const S_CHAR_AMB: usize = 0xDDC7B4;
const S_CS_FRAME_START: usize = 0xDDC7C8;
const S_CS_FRAME_END: usize = 0xDDC7E0;
const S_CS_SHOT_START: usize = 0xDDC7F4;
const S_CS_SHOT_END: usize = 0xDDC808;
const S_DYNRANGE: usize = 0xDDC81C;
const S_MAIN_LIGHT: usize = 0xDDC830;
const S_BACK_LIGHT: usize = 0xDDC844;
const S_SHADOW_HARD: usize = 0xDDC858;
const S_LIGHT_AMP: usize = 0xDDC868;
const S_BLEACH: usize = 0xDDC87C;
const S_TINT: usize = 0xDDC884;
const S_BLOOM: usize = 0xDDC890;
const S_CUTOFF: usize = 0xDDC898;
const S_AMP: usize = 0xDDC8A0;
const S_CONTRAST: usize = 0xDDC1F4;
const S_BRIGHTNESS: usize = 0xDDC1E8;
const S_SATURATION: usize = 0xDDC200;
const S_RADIUS: usize = 0xDDC00C;
const S_NAME: usize = 0xDC5804;
const S_TYPE: usize = 0xDD92D4;

// --- counters ----------------------------------------------------------------------------------------------

const NB: usize = 32;
static HITS: [AtomicU32; NB] = [const { AtomicU32::new(0) }; NB];
const H_NAMES: [&str; NB] = [
    "calls",
    "no_script",
    "no_root",
    "cache_full",
    "default_pool",
    "zero_sets",
    "sets",
    "lights",
    "type_omni",
    "type_spot",
    "type_other",
    "slate_hash",
    "slate_default",
    "flag_stale_poisoned",
    "flag_unknown_stale",
    "multiplier_missing",
    "light_noname_slate",
    "grids",
    "grid_main_default",
    "grid_back_default",
    "cs_default",
    "dynrange_default",
    "tint",
    "bloom",
    "grids_zero",
    "light_zero",
    "no_lights_node",
    "no_grids_node",
    "completed",
    "pool_scope_end",
    "x30",
    "x31",
];
fn hit(i: usize) {
    HITS[i].fetch_add(1, Ordering::Relaxed);
}
const H_CALLS: usize = 0;
const H_NO_SCRIPT: usize = 1;
const H_NO_ROOT: usize = 2;
const H_FULL: usize = 3;
const H_DEFAULT_POOL: usize = 4;
const H_ZERO_SETS: usize = 5;
const H_SETS: usize = 6;
const H_LIGHTS: usize = 7;
const H_OMNI: usize = 8;
const H_SPOT: usize = 9;
const H_OTHER_TYPE: usize = 10;
const H_SLATE_HASH: usize = 11;
const H_SLATE_DEFAULT: usize = 12;
const H_STALE_POISON: usize = 13;
const H_STALE_UNKNOWN: usize = 14;
const H_MULT_MISSING: usize = 15;
const H_GRIDS: usize = 17;
const H_MAIN_DEFAULT: usize = 18;
const H_BACK_DEFAULT: usize = 19;
const H_CS_DEFAULT: usize = 20;
const H_DYN_DEFAULT: usize = 21;
const H_TINT: usize = 22;
const H_BLOOM: usize = 23;
const H_GRIDS_ZERO: usize = 24;
const H_LIGHTS_ZERO: usize = 25;
const H_NO_LIGHTS_NODE: usize = 26;
const H_NO_GRIDS_NODE: usize = 27;
const H_DONE: usize = 28;

// --- small helpers ---------------------------------------------------------------------------------------------

/// ASCII case-insensitive equality of two C strings (what _stricmp == 0 means in the C locale).
unsafe fn name_eq(a: u32, b: u32) -> bool {
    let (mut pa, mut pb) = (a as usize, b as usize);
    loop {
        let (x, y) = (*(pa as *const u8), *(pb as *const u8));
        if x.to_ascii_lowercase() != y.to_ascii_lowercase() {
            return false;
        }
        if x == 0 {
            return true;
        }
        pa += 1;
        pb += 1;
    }
}

unsafe fn cstr_len(p: u32) -> u32 {
    let mut n = 0usize;
    while *((p as usize + n) as *const u8) != 0 {
        n += 1;
    }
    n as u32
}

/// bfbee0 without the call: the first child of `node` whose name equals `name` (0: none).
unsafe fn find_child(node: u32, name: u32) -> u32 {
    if node == 0 {
        return 0;
    }
    let mut c = rd(node as usize + 8);
    while c != 0 {
        if name_eq(rd(c as usize), name) {
            return c;
        }
        c = rd(c as usize + 4);
    }
    0
}

/// The value string of the named child (0: none).
unsafe fn child_value(node: u32, name: u32) -> u32 {
    let c = find_child(node, name);
    if c == 0 { 0 } else { rd(c as usize + 0xC) }
}

/// `call 0xBFBEE0` (EAX = node, EDI = name): the child or 0.
unsafe fn call_find(node: u32, name: usize) -> u32 {
    call_regs(F_FIND, node, 0, 0, 0, ga(name), &[])
}

/// The value string of a child found with the original lookup (0 when absent): the
/// `eax = child ? [child + 0xC] : 0` sequence of the original.
unsafe fn lookup_value(node: u32, name: usize) -> u32 {
    let c = call_find(node, name);
    if c == 0 { 0 } else { rd(c as usize + 0xC) }
}

/// A loop of `stricmp(name of node, s)` over a sibling chain starting at `n` (0: none): the first node
/// whose name equals `s`.
unsafe fn scan(mut n: u32, s: usize) -> u32 {
    while n != 0 {
        let nm = rd(n as usize);
        if call_regs(F_STRICMP, 0, 0, 0, 0, 0, &[nm, ga(s)]) == 0 {
            return n;
        }
        n = rd(n as usize + 4);
    }
    0
}

unsafe fn sscanf(s: u32, fmt: usize, ptrs: &[u32]) {
    let mut a = [0u32; 8];
    a[0] = s;
    a[1] = ga(fmt);
    for (i, p) in ptrs.iter().enumerate() {
        a[2 + i] = *p;
    }
    call_regs(F_SSCANF, s, 0, 0, 0, 0, &a[..2 + ptrs.len()]);
}

/// Call a getter that copies the child's string to its buffer and parses it (0xB747D0 / 0xB74EF0):
/// with the child missing it parses what the previous getter left in the buffer. `stale` is that
/// string (0: not known; the callee then parses its own stack garbage).
unsafe fn call_parse(f: usize, name: usize, dest: u32, node: u32, stale: &mut u32) {
    let v = child_value(node, ga(name));
    if v != 0 {
        *stale = v;
        call_regs(f, 0, ga(name), 0, dest, 0, &[node]);
        return;
    }
    #[cfg(test)]
    difftest_hooks::expect_stale(*stale);
    if *stale != 0 {
        hit(H_STALE_POISON);
        call_stale(f, ga(name), dest, node, *stale);
    } else {
        hit(H_STALE_UNKNOWN);
        call_regs(f, 0, ga(name), 0, dest, 0, &[node]);
    }
}

/// Call a getter that returns AL = found (0xB74F60 / 0xB74860); a found child's string is what the
/// buffer holds afterwards.
unsafe fn call_get_ok(f: usize, name: usize, dest: u32, node: u32, stale: &mut u32) -> bool {
    let v = child_value(node, ga(name));
    if v != 0 {
        *stale = v;
    }
    call_regs(f, 0, ga(name), 0, dest, 0, &[node]) & 0xFF != 0
}

/// `call_regs` for a getter with its 0x400-byte buffer preloaded: the buffer is at the bottom of the
/// callee's frame, `ESP0 - 0x408` where ESP0 is the stack pointer at the call instruction (one stack
/// argument pushed). The string is copied there (below ESP) right before the call.
#[cfg(not(test))]
unsafe fn call_stale(f: usize, ecx: u32, esi: u32, node: u32, stale: u32) {
    let block: [u32; 5] = [f as u32, ecx, esi, node, stale];
    std::arch::asm!(
        "push ebx", "push esi", "push edi", "push ebp",
        "mov ebp, esp",
        "mov ebx, eax",
        "push dword ptr [ebx + 12]",
        "lea edi, [esp - 0x408]",
        "mov esi, [ebx + 16]",
        "mov ecx, 0x3ff",
        "2:",
        "lodsb",
        "test al, al",
        "jz 3f",
        "stosb",
        "dec ecx",
        "jnz 2b",
        "3:",
        "mov byte ptr [edi], 0",
        "mov ecx, [ebx + 4]",
        "mov esi, [ebx + 8]",
        "xor eax, eax",
        "call dword ptr [ebx]",
        "mov esp, ebp",
        "pop ebp", "pop edi", "pop esi", "pop ebx",
        inout("eax") block.as_ptr() => _, lateout("ecx") _, lateout("edx") _,
        clobber_abi("C"),
    );
}
/// In the offline test the callees are stubs that keep their own record of the buffer; the stub
/// compares it with what the port believes (`expect_stale`).
#[cfg(test)]
unsafe fn call_stale(f: usize, ecx: u32, esi: u32, node: u32, _stale: u32) {
    call_regs(f, 0, ecx, 0, esi, 0, &[node]);
}

/// A float through the x87: `fld dword` / `fstp dword` round trip (quiets a signalling NaN).
fn q(v: f32) -> f32 {
    (v as f64) as f32
}

// --- the port ----------------------------------------------------------------------------------------------------

unsafe fn parse(name: u32, pool_in: u32) -> u32 {
    hit(H_CALLS);
    let mut script_buf = [0u32; 0x44]; // the 0x110 bytes 0xBFD880 fills (EAX)
    let ok = call_regs(F_PARSE_SCRIPT, script_buf.as_mut_ptr() as u32, name, pool_in, name, pool_in, &[name]);
    if ok & 0xFF == 0 {
        hit(H_NO_SCRIPT);
        return 0;
    }
    let mut hash = 0u32;
    call_regs(F_HASH, 0, name, pool_in, name, pool_in, &[&mut hash as *mut u32 as u32]);
    let root = call_regs(F_ROOT, name, 0, pool_in, name, pool_in, &[]);
    if root == 0 {
        hit(H_NO_ROOT);
        return 0;
    }
    let count = rd(CACHE_COUNT);
    if count == 0x80 {
        hit(H_FULL);
        return 0;
    }
    let slot = ga(CACHE_BASE).wrapping_add(count << 4);
    wd(CACHE_COUNT, count.wrapping_add(1));
    let mut pool = pool_in;
    if pool == 0 {
        pool = ga(DEFAULT_POOL);
        hit(H_DEFAULT_POOL);
    }
    let vt = rd(pool as usize);
    let mark = call_regs(rd(vt as usize + 0x30) as usize, 0, pool, vt, slot, pool, &[]);
    wd(slot as usize + 0xC, mark);
    let nsets = call_regs(F_COUNT, root, 0, vt, slot, pool, &[ga(S_LIGHTSET)]);
    wd(slot as usize + 8, nsets);
    let vt = rd(pool as usize);
    let sets = call_regs(rd(vt as usize + 0x1C) as usize, 0, pool, vt, slot, pool, &[nsets.wrapping_mul(5) << 4, 0]);
    wd(slot as usize + 4, sets);
    wd(slot as usize, hash);
    if nsets == 0 {
        hit(H_ZERO_SETS);
        return finish(slot);
    }
    // first LightSet child of the root
    let mut ls = scan(rd(root as usize + 8), S_LIGHTSET);
    if (rd(slot as usize + 8) as i32) <= 0 {
        return finish(slot);
    }
    let mut idx = 0u32;
    // The scratch locals the original keeps in its frame across iterations.
    let mut flag = 0u32; // [esp+0x10]: the int the flag getters write
    let mut tmp = 0u32;
    let mut stale = 0u32;
    let mut quat = [0u32; 4];
    let mut mat = [0u32; 9];
    let mut outv = [0u32; 3];
    let cbits = q(rf(C_DEFAULT)).to_bits();
    loop {
        hit(H_SETS);
        let rec = idx.wrapping_mul(5).wrapping_shl(4).wrapping_add(rd(slot as usize + 4));
        wb(rec as usize + 0x14, 0);
        // the set's name hash
        let nm = if ls == 0 { 0 } else { lookup_value(ls, S_NAME) };
        call_regs(F_HASH, 0, nm, 0, ls, 0, &[&mut tmp as *mut u32 as u32]);
        wd(rec as usize, tmp);
        call_parse(F_GET_INT, S_STARTTIME, rec.wrapping_add(4), ls, &mut stale);
        call_parse(F_GET_INT, S_ENDTIME, rec.wrapping_add(8), ls, &mut stale);
        // Lights
        let lights = if ls == 0 { 0 } else { scan(rd(ls as usize + 8), S_LIGHTS) };
        let nlights = call_regs(F_COUNT, lights, 0, 0, 0, 0, &[ga(S_LIGHT)]);
        wd(rec as usize + 0x10, nlights);
        let vt = rd(pool as usize);
        let larr = call_regs(rd(vt as usize + 0x1C) as usize, 0, pool, vt, lights, 0, &[nlights.wrapping_mul(9) << 4, 0]);
        wd(rec as usize + 0xC, larr);
        if lights == 0 {
            hit(H_NO_LIGHTS_NODE);
        }
        let mut n = if lights == 0 { 0 } else { scan(rd(lights as usize + 8), S_LIGHT) };
        let mut j = 0u32;
        if (rd(rec as usize + 0x10) as i32) > 0 {
            let vec3 = [0u32, cbits, 0u32]; // (0, c, 0): the direction the orientation rotates
            let mut off = 0u32;
            loop {
                hit(H_LIGHTS);
                let l = rd(rec as usize + 0xC).wrapping_add(off);
                // Name
                let nm = if n == 0 { 0 } else { lookup_value(n, S_NAME) };
                call_regs(F_HASH, 0, nm, 0, 0, 0, &[&mut tmp as *mut u32 as u32]);
                wd(l as usize + 4, tmp);
                let len = cstr_len(nm).wrapping_add(1);
                let vt = rd(pool as usize);
                let copy = call_regs(rd(vt as usize + 0x1C) as usize, 0, pool, vt, nm, 0, &[len, 1]);
                wd(l as usize, copy);
                call_regs(F_STRNCPY, len, 0, 0, nm, copy, &[copy, nm, len]);
                wb(copy as usize + len as usize - 1, 0);
                // Type
                let ty = if n == 0 { 0 } else { lookup_value(n, S_TYPE) };
                if call_regs(F_STRICMP, 0, 0, 0, ty, 0, &[ty, ga(S_OMNI)]) == 0 {
                    ww(l as usize + 8, 0);
                    hit(H_OMNI);
                } else if call_regs(F_STRICMP, 0, 0, 0, ty, 0, &[ty, ga(S_SPOT)]) == 0 {
                    ww(l as usize + 8, 2);
                    hit(H_SPOT);
                } else {
                    hit(H_OTHER_TYPE);
                }
                // Color, Multiplier
                let cv = if n == 0 { 0 } else { lookup_value(n, S_COLOR) };
                sscanf(cv, S_FMT3, &[l.wrapping_add(0xC), l.wrapping_add(0x10), l.wrapping_add(0x14)]);
                if child_value(n, ga(S_MULTIPLIER)) == 0 {
                    hit(H_MULT_MISSING);
                }
                call_parse(F_GET_FLOAT, S_MULTIPLIER, l.wrapping_add(0x18), n, &mut stale);
                let m = q(rf(l as usize + 0x18)) as f64;
                let (c0, c1, c2) = (rf(l as usize + 0xC) as f64, rf(l as usize + 0x10) as f64, rf(l as usize + 0x14) as f64);
                wf(l as usize + 0x1C, (m * c0) as f32);
                wf(l as usize + 0x20, (c1 * m) as f32);
                wf(l as usize + 0x24, (m * c2) as f32);
                wd(l as usize + 0x28, 0x4000);
                wb(l as usize + 0x7C, 0);
                // the four flag bits (each parse writes [esp+0x10])
                for (nm_s, bit) in [(S_LIGHTCHARACTER, 0x200u32), (S_SHADOWCHARACTER, 0x800), (S_LIGHTLEVEL, 0x100), (S_SHADOWLEVEL, 0x400)] {
                    call_parse(F_GET_INT, nm_s, &mut flag as *mut u32 as u32, n, &mut stale);
                    if flag != 0 {
                        wd(l as usize + 0x28, rd(l as usize + 0x28) | bit);
                    }
                }
                // Position (+0x2C), copied to +0x38
                let pv = if n == 0 { 0 } else { lookup_value(n, S_POSITION) };
                sscanf(pv, S_FMT3, &[l.wrapping_add(0x2C), l.wrapping_add(0x30), l.wrapping_add(0x34)]);
                let (a, b, c) = (rd(l as usize + 0x2C), rd(l as usize + 0x30), rd(l as usize + 0x34));
                wd(l as usize + 0x38, a);
                wd(l as usize + 0x3C, b);
                wd(l as usize + 0x40, c);
                // Orientation: quaternion -> matrix -> rotate (0, c, 0); ECX = the matrix for both calls
                let ov = if n == 0 { 0 } else { lookup_value(n, S_ORIENTATION) };
                sscanf(ov, S_FMT4, &[quat.as_mut_ptr() as u32, quat.as_mut_ptr() as u32 + 4, quat.as_mut_ptr() as u32 + 8, quat.as_mut_ptr() as u32 + 12]);
                let mp = mat.as_mut_ptr() as u32;
                let eax = call_regs(F_QUAT, quat.as_mut_ptr() as u32, mp, 0, 0, 0, &[]);
                let _ = eax;
                let r = call_regs(F_VMUL, outv.as_mut_ptr() as u32, mp, 0, 0, 0, &[vec3.as_ptr() as u32]);
                let (a, b, c) = (rd(r as usize), rd(r as usize + 4), rd(r as usize + 8));
                wd(l as usize + 0x5C, a);
                wd(l as usize + 0x60, b);
                wd(l as usize + 0x64, c);
                // Hotspot, Attenuation
                let hv = if n == 0 { 0 } else { lookup_value(n, S_HOTSPOT) };
                sscanf(hv, S_FMT2, &[l.wrapping_add(0x68), l.wrapping_add(0x6C)]);
                let av = if n == 0 { 0 } else { lookup_value(n, S_ATTENUATION) };
                sscanf(av, S_FMT2, &[l.wrapping_add(0x70), l.wrapping_add(0x74)]);
                // Slate_Name: hash of a non-empty string, else the default
                if n != 0 {
                    let sv = lookup_value(n, S_SLATE);
                    if sv != 0 && cstr_len(sv) != 0 {
                        call_regs(F_HASH_SLATE, &mut tmp as *mut u32 as u32, 0, 0, 0, 0, &[sv]);
                        wd(l as usize + 0x78, tmp);
                        hit(H_SLATE_HASH);
                    } else {
                        wd(l as usize + 0x78, rd(DEFAULT_SLATE));
                        hit(H_SLATE_DEFAULT);
                    }
                } else {
                    wd(l as usize + 0x78, rd(DEFAULT_SLATE));
                    hit(H_SLATE_DEFAULT);
                }
                wd(l as usize + 0x88, 0);
                wd(l as usize + 0x84, 0);
                wd(l as usize + 0x80, 0);
                // the next Light sibling
                n = if lights == 0 || n == 0 { 0 } else { scan(rd(n as usize + 4), S_LIGHT) };
                off = off.wrapping_add(0x90);
                j += 1;
                if (j as i32) >= (rd(rec as usize + 0x10) as i32) {
                    break;
                }
            }
        } else {
            hit(H_LIGHTS_ZERO);
        }
        // Ambient_Grids
        let grids = if ls == 0 { 0 } else { scan(rd(ls as usize + 8), S_AGRIDS) };
        if grids == 0 {
            hit(H_NO_GRIDS_NODE);
        }
        let ngrids = call_regs(F_COUNT, grids, 0, 0, 0, 0, &[ga(S_AGRID)]);
        wd(rec as usize + 0x1C, ngrids);
        let vt = rd(pool as usize);
        let garr = call_regs(rd(vt as usize + 0x1C) as usize, 0, pool, vt, grids, rec, &[ngrids.wrapping_mul(0x78), 0]);
        wd(rec as usize + 0x18, garr);
        if ngrids != 0 {
            let mut gn = if grids == 0 { 0 } else { scan(rd(grids as usize + 8), S_AGRID) };
            if (rd(rec as usize + 0x1C) as i32) > 0 {
                let mut g = garr;
                let mut gi = 0u32;
                loop {
                    hit(H_GRIDS);
                    grid(g, gn, cbits, &mut stale);
                    // next Ambient_Grid sibling
                    gn = if grids == 0 || gn == 0 { 0 } else { scan(rd(gn as usize + 4), S_AGRID) };
                    gi += 1;
                    g = g.wrapping_add(0x78);
                    if (gi as i32) >= (rd(rec as usize + 0x1C) as i32) {
                        break;
                    }
                }
            }
        } else {
            hit(H_GRIDS_ZERO);
        }
        // the next LightSet sibling
        ls = if ls == 0 { 0 } else { scan(rd(ls as usize + 4), S_LIGHTSET) };
        idx += 1;
        if (idx as i32) >= (rd(slot as usize + 8) as i32) {
            break;
        }
    }
    finish(slot)
}

/// One Ambient_Grid (0x78 bytes at `g`) from node `n`:
/// +0 slate hash, +4 / +0x10 level / character ambient colours, +0x1C..0x28 cutscene frame / shot
/// start / end (-1 when absent), +0x2C main light colour, +0x38 back light colour, +0x44 shadow
/// hardness, +0x48 light amplification, +0x4C contrast, +0x50 bleach, +0x54 light map dynamic range,
/// +0x58 brightness, +0x5C saturation, +0x60 tint colour, +0x6C bloom radius, +0x70 cutoff, +0x74 amp.
unsafe fn grid(g: u32, n: u32, cbits: u32, stale: &mut u32) {
    let gp = g as usize;
    let mut tmp = 0u32;
    let sv = if n == 0 { 0 } else { lookup_value(n, S_SLATE) };
    call_regs(F_HASH_SLATE, &mut tmp as *mut u32 as u32, 0, 0, 0, 0, &[sv]);
    wd(gp, tmp);
    let lv = if n == 0 { 0 } else { lookup_value(n, S_LEVEL_AMB) };
    sscanf(lv, S_FMT3, &[g.wrapping_add(4), g.wrapping_add(8), g.wrapping_add(0xC)]);
    let cv = if n == 0 { 0 } else { lookup_value(n, S_CHAR_AMB) };
    sscanf(cv, S_FMT3, &[g.wrapping_add(0x10), g.wrapping_add(0x14), g.wrapping_add(0x18)]);
    for (nm, off) in [(S_CS_FRAME_START, 0x1Cu32), (S_CS_FRAME_END, 0x20), (S_CS_SHOT_START, 0x24), (S_CS_SHOT_END, 0x28)] {
        if !call_get_ok(F_GET_INT_OK, nm, g.wrapping_add(off), n, stale) {
            wd(gp + off as usize, 0xFFFF_FFFF);
            hit(H_CS_DEFAULT);
        }
    }
    if !call_get_ok(F_GET_FLOAT_OK, S_DYNRANGE, g.wrapping_add(0x54), n, stale) {
        wf(gp + 0x54, 1.0);
        hit(H_DYN_DEFAULT);
    }
    // Main_Light_Color (+0x2C) and Light_Back_Color (+0x38): the value string, else three times c
    let mut main_set = false;
    if n != 0 {
        let v = lookup_value(n, S_MAIN_LIGHT);
        if v != 0 {
            sscanf(v, S_FMT3, &[g.wrapping_add(0x2C), g.wrapping_add(0x30), g.wrapping_add(0x34)]);
            main_set = true;
        }
    }
    if !main_set {
        wd(gp + 0x2C, cbits);
        wd(gp + 0x30, cbits);
        wd(gp + 0x34, cbits);
        hit(H_MAIN_DEFAULT);
    }
    let mut back_set = false;
    if n != 0 {
        let v = lookup_value(n, S_BACK_LIGHT);
        if v != 0 {
            sscanf(v, S_FMT3, &[g.wrapping_add(0x38), g.wrapping_add(0x3C), g.wrapping_add(0x40)]);
            back_set = true;
        }
    }
    if !back_set {
        wd(gp + 0x38, cbits);
        wd(gp + 0x3C, cbits);
        wd(gp + 0x40, cbits);
        hit(H_BACK_DEFAULT);
    }
    for (nm, off) in [(S_SHADOW_HARD, 0x44u32), (S_LIGHT_AMP, 0x48), (S_CONTRAST, 0x4C), (S_BLEACH, 0x50)] {
        if !call_get_ok(F_GET_FLOAT_OK, nm, g.wrapping_add(off), n, stale) {
            wd(gp + off as usize, cbits);
        }
    }
    // Bleach again (the original calls it twice, the second time into the same slot)
    if !call_get_ok(F_GET_FLOAT_OK, S_BLEACH, g.wrapping_add(0x50), n, stale) {
        wd(gp + 0x50, cbits);
    }
    wd(gp + 0x58, cbits);
    wd(gp + 0x5C, cbits);
    wd(gp + 0x60, cbits);
    wd(gp + 0x64, cbits);
    wd(gp + 0x68, cbits);
    wd(gp + 0x6C, cbits);
    wd(gp + 0x70, cbits);
    wd(gp + 0x74, cbits);
    call_get_ok(F_GET_FLOAT_OK, S_BRIGHTNESS, g.wrapping_add(0x58), n, stale);
    call_get_ok(F_GET_FLOAT_OK, S_SATURATION, g.wrapping_add(0x5C), n, stale);
    let mut bloom = 0u32;
    if n != 0 {
        let v = lookup_value(n, S_TINT);
        if v != 0 {
            sscanf(v, S_FMT3, &[g.wrapping_add(0x60), g.wrapping_add(0x64), g.wrapping_add(0x68)]);
            hit(H_TINT);
        }
        bloom = scan(rd(n as usize + 8), S_BLOOM);
    }
    if bloom != 0 {
        hit(H_BLOOM);
    }
    call_get_ok(F_GET_FLOAT_OK, S_RADIUS, g.wrapping_add(0x6C), bloom, stale);
    call_get_ok(F_GET_FLOAT_OK, S_CUTOFF, g.wrapping_add(0x70), bloom, stale);
    call_get_ok(F_GET_FLOAT_OK, S_AMP, g.wrapping_add(0x74), bloom, stale);
}

/// The common exit (0x4C969B): end the pool scope that 0xB743F0 opened, return the slot.
unsafe fn finish(slot: u32) -> u32 {
    let p = rd(SCOPE_POOL);
    let vt = rd(p as usize);
    call_regs(rd(vt as usize + 0x34) as usize, 0, p, vt, 0, 0, &[rd(SCOPE_MARK)]);
    wd(SCOPE_MARK, 0xFFFF_FFFF);
    hit(H_DONE);
    slot
}

extern "C" fn tod_entry(name: u32, pool: u32) -> u32 {
    unsafe { parse(name, pool) }
}

#[unsafe(naked)]
extern "C" fn tod_thunk() {
    std::arch::naked_asm!("push edx", "push ecx", "call {f}", "add esp, 8", "ret", f = sym tod_entry)
}

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    let mut ok = 0;
    if replace_pinned("tod_light_sets_parse", TOD, &TOD_BODY, tod_thunk as *const () as usize) {
        ok += 1;
    }
    log(&format!("port_tod_lights: replaced {ok}/1 with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let c = HITS[H_CALLS].load(Ordering::Relaxed);
    if c == 0 {
        return String::new();
    }
    let hits: Vec<String> = HITS
        .iter()
        .zip(H_NAMES)
        .skip(1)
        .filter_map(|(h, n)| {
            let c = h.load(Ordering::Relaxed);
            (c > 0).then(|| format!("{n}={c}"))
        })
        .collect();
    format!("port_tod_lights: tod_light_sets_parse {c} (difftest-checked) [{}]", hits.join(" "))
}

// --- the original body (tools/body_bytes.py) ---------------------------------------------------------------------

const TOD_BODY: [u8; 2897] = [
    0x55, 0x8B, 0xEC, 0x83, 0xE4, 0xF8, 0x81, 0xEC, 0xDC, 0x01, 0x00, 0x00, 0xA1, 0x50, 0x0B, 0xE8,
    0x00, 0x33, 0xC4, 0x89, 0x84, 0x24, 0xD8, 0x01, 0x00, 0x00, 0x53, 0x56, 0x57, 0x8B, 0xF1, 0x56,
    0x8D, 0x84, 0x24, 0xD4, 0x00, 0x00, 0x00, 0x8B, 0xFA, 0x89, 0x54, 0x24, 0x28, 0xE8, 0xCE, 0x4C,
    0x73, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x17, 0x33, 0xC0, 0x5F, 0x5E, 0x5B, 0x8B, 0x8C,
    0x24, 0xD8, 0x01, 0x00, 0x00, 0x33, 0xCC, 0xE8, 0x83, 0xD4, 0x7C, 0x00, 0x8B, 0xE5, 0x5D, 0xC3,
    0x8D, 0x44, 0x24, 0x14, 0x50, 0x33, 0xC0, 0x8B, 0xCE, 0xE8, 0x52, 0x3F, 0x71, 0x00, 0x33, 0xC9,
    0x8B, 0xC6, 0xE8, 0x09, 0xB8, 0x6A, 0x00, 0x8B, 0xD8, 0x85, 0xDB, 0x74, 0xCC, 0xA1, 0x44, 0x6D,
    0x52, 0x02, 0x3D, 0x80, 0x00, 0x00, 0x00, 0x74, 0xC0, 0x8B, 0xF0, 0xC1, 0xE6, 0x04, 0x81, 0xC6,
    0x98, 0x21, 0x65, 0x02, 0x83, 0xC0, 0x01, 0x85, 0xFF, 0x89, 0x74, 0x24, 0x28, 0xA3, 0x44, 0x6D,
    0x52, 0x02, 0x75, 0x09, 0xBF, 0x98, 0x29, 0x65, 0x02, 0x89, 0x7C, 0x24, 0x24, 0x8B, 0x17, 0x8B,
    0x42, 0x30, 0x8B, 0xCF, 0xFF, 0xD0, 0x89, 0x46, 0x0C, 0x68, 0x90, 0xC6, 0xDD, 0x00, 0x8B, 0xC3,
    0xE8, 0xDB, 0x34, 0x73, 0x00, 0x89, 0x46, 0x08, 0x8B, 0x17, 0x8B, 0x52, 0x1C, 0x83, 0xC4, 0x04,
    0x8D, 0x04, 0x80, 0x6A, 0x00, 0xC1, 0xE0, 0x04, 0x50, 0x8B, 0xCF, 0xFF, 0xD2, 0x83, 0x7E, 0x08,
    0x00, 0x89, 0x46, 0x04, 0x8B, 0x44, 0x24, 0x14, 0x89, 0x06, 0x0F, 0x84, 0x3B, 0x0A, 0x00, 0x00,
    0x8B, 0x73, 0x08, 0x85, 0xF6, 0x74, 0x1B, 0x8B, 0x0E, 0x68, 0x90, 0xC6, 0xDD, 0x00, 0x51, 0xE8,
    0xBC, 0x03, 0x7D, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x09, 0x8B, 0x76, 0x04, 0x85, 0xF6,
    0x75, 0xE5, 0x33, 0xF6, 0x8B, 0x54, 0x24, 0x28, 0x83, 0x7A, 0x08, 0x00, 0x89, 0x74, 0x24, 0x20,
    0xC7, 0x44, 0x24, 0x30, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x8E, 0xFD, 0x09, 0x00, 0x00, 0x8B, 0xFF,
    0x8B, 0x5C, 0x24, 0x30, 0x8B, 0x44, 0x24, 0x28, 0x8B, 0x74, 0x24, 0x20, 0x8D, 0x1C, 0x9B, 0xC1,
    0xE3, 0x04, 0x03, 0x58, 0x04, 0x85, 0xF6, 0x89, 0x5C, 0x24, 0x1C, 0xC6, 0x43, 0x14, 0x00, 0x75,
    0x04, 0x33, 0xC9, 0xEB, 0x17, 0xBF, 0x04, 0x58, 0xDC, 0x00, 0x8B, 0xC6, 0xE8, 0x0F, 0x32, 0x73,
    0x00, 0x85, 0xC0, 0x75, 0x04, 0x33, 0xC9, 0xEB, 0x03, 0x8B, 0x48, 0x0C, 0x8D, 0x94, 0x24, 0x98,
    0x00, 0x00, 0x00, 0x52, 0x33, 0xC0, 0xE8, 0x45, 0x3E, 0x71, 0x00, 0x8B, 0x00, 0x56, 0x8D, 0x73,
    0x04, 0xB9, 0x9C, 0xC6, 0xDD, 0x00, 0x89, 0x03, 0xE8, 0xD3, 0xBA, 0x6A, 0x00, 0x8B, 0x7C, 0x24,
    0x24, 0x83, 0xC4, 0x04, 0x57, 0x8D, 0x73, 0x08, 0xB9, 0xA8, 0xC6, 0xDD, 0x00, 0xE8, 0xBE, 0xBA,
    0x6A, 0x00, 0x83, 0xC4, 0x04, 0x85, 0xFF, 0x74, 0x22, 0x8B, 0x77, 0x08, 0x85, 0xF6, 0x74, 0x1B,
    0x8B, 0x0E, 0x68, 0xB0, 0xC6, 0xDD, 0x00, 0x51, 0xE8, 0x03, 0x03, 0x7D, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x74, 0x09, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5, 0x33, 0xF6, 0x68, 0xB8, 0xC6,
    0xDD, 0x00, 0x8B, 0xC6, 0x89, 0x74, 0x24, 0x38, 0xE8, 0xC3, 0x33, 0x73, 0x00, 0x8B, 0x4C, 0x24,
    0x28, 0x89, 0x43, 0x10, 0x8B, 0x11, 0x8B, 0x52, 0x1C, 0x83, 0xC4, 0x04, 0x8D, 0x04, 0xC0, 0x6A,
    0x00, 0xC1, 0xE0, 0x04, 0x50, 0xFF, 0xD2, 0x85, 0xF6, 0x89, 0x43, 0x0C, 0x74, 0x22, 0x8B, 0x76,
    0x08, 0x85, 0xF6, 0x74, 0x1B, 0x8B, 0x06, 0x68, 0xB8, 0xC6, 0xDD, 0x00, 0x50, 0xE8, 0xAE, 0x02,
    0x7D, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x09, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE5,
    0x33, 0xF6, 0x83, 0x7B, 0x10, 0x00, 0x89, 0x74, 0x24, 0x0C, 0xC7, 0x44, 0x24, 0x14, 0x00, 0x00,
    0x00, 0x00, 0x0F, 0x8E, 0x08, 0x04, 0x00, 0x00, 0xD9, 0xEE, 0xC7, 0x44, 0x24, 0x18, 0x00, 0x00,
    0x00, 0x00, 0xD9, 0x54, 0x24, 0x74, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0xD9, 0x5C, 0x24, 0x78,
    0xD9, 0x5C, 0x24, 0x7C, 0x8B, 0x4C, 0x24, 0x1C, 0x8B, 0x59, 0x0C, 0x03, 0x5C, 0x24, 0x18, 0x83,
    0x7C, 0x24, 0x0C, 0x00, 0x75, 0x04, 0x33, 0xF6, 0xEB, 0x19, 0x8B, 0x44, 0x24, 0x0C, 0xBF, 0x04,
    0x58, 0xDC, 0x00, 0xE8, 0xF8, 0x30, 0x73, 0x00, 0x85, 0xC0, 0x75, 0x04, 0x33, 0xF6, 0xEB, 0x03,
    0x8B, 0x70, 0x0C, 0x8D, 0x94, 0x24, 0x90, 0x00, 0x00, 0x00, 0x52, 0x33, 0xC0, 0x8B, 0xCE, 0xE8,
    0x2C, 0x3D, 0x71, 0x00, 0x8B, 0x00, 0x89, 0x43, 0x04, 0x8B, 0xC6, 0x8D, 0x50, 0x01, 0x8B, 0xFF,
    0x8A, 0x08, 0x83, 0xC0, 0x01, 0x84, 0xC9, 0x75, 0xF7, 0x8B, 0x4C, 0x24, 0x24, 0x2B, 0xC2, 0x8B,
    0x11, 0x83, 0xC0, 0x01, 0x6A, 0x01, 0x50, 0x8B, 0x42, 0x1C, 0xFF, 0xD0, 0x8B, 0xF8, 0x8B, 0xC6,
    0x89, 0x3B, 0x8D, 0x48, 0x01, 0x8A, 0x10, 0x83, 0xC0, 0x01, 0x84, 0xD2, 0x75, 0xF7, 0x2B, 0xC1,
    0x83, 0xC0, 0x01, 0x50, 0x56, 0x57, 0x89, 0x44, 0x24, 0x38, 0xE8, 0xA1, 0x02, 0x7D, 0x00, 0x8B,
    0x4C, 0x24, 0x38, 0x83, 0xC4, 0x0C, 0x83, 0x7C, 0x24, 0x0C, 0x00, 0xC6, 0x44, 0x0F, 0xFF, 0x00,
    0x75, 0x04, 0x33, 0xF6, 0xEB, 0x19, 0x8B, 0x44, 0x24, 0x0C, 0xBF, 0xD4, 0x92, 0xDD, 0x00, 0xE8,
    0x6C, 0x30, 0x73, 0x00, 0x85, 0xC0, 0x75, 0x04, 0x33, 0xF6, 0xEB, 0x03, 0x8B, 0x70, 0x0C, 0x68,
    0xC0, 0xC6, 0xDD, 0x00, 0x56, 0xE8, 0xA6, 0x01, 0x7D, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x75,
    0x06, 0x66, 0x89, 0x43, 0x08, 0xEB, 0x18, 0x68, 0xC8, 0xC6, 0xDD, 0x00, 0x56, 0xE8, 0x8E, 0x01,
    0x7D, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x75, 0x06, 0x66, 0xC7, 0x43, 0x08, 0x02, 0x00, 0x8B,
    0x44, 0x24, 0x0C, 0x85, 0xC0, 0x74, 0x11, 0xBF, 0xDC, 0xC6, 0xDD, 0x00, 0xE8, 0x1F, 0x30, 0x73,
    0x00, 0x85, 0xC0, 0x74, 0x03, 0x8B, 0x40, 0x0C, 0x8D, 0x53, 0x14, 0x52, 0x8D, 0x4B, 0x10, 0x51,
    0x8D, 0x7B, 0x0C, 0x57, 0x68, 0xE4, 0xC6, 0xDD, 0x00, 0x50, 0xE8, 0xAA, 0xE1, 0x7C, 0x00, 0x8B,
    0x54, 0x24, 0x20, 0x83, 0xC4, 0x14, 0x8D, 0x73, 0x18, 0x52, 0xB9, 0xF0, 0xC6, 0xDD, 0x00, 0xE8,
    0xFC, 0xBF, 0x6A, 0x00, 0xD9, 0x06, 0xD9, 0x5C, 0x24, 0x30, 0x83, 0xC4, 0x04, 0xD9, 0x44, 0x24,
    0x2C, 0x8D, 0x74, 0x24, 0x10, 0xD9, 0xC0, 0xD8, 0x0F, 0xD9, 0x5C, 0x24, 0x44, 0x8B, 0x44, 0x24,
    0x44, 0xD9, 0x47, 0x04, 0xD8, 0xC9, 0xD9, 0x5C, 0x24, 0x48, 0x8B, 0x4C, 0x24, 0x48, 0xD8, 0x4F,
    0x08, 0x8B, 0x7C, 0x24, 0x0C, 0x89, 0x43, 0x1C, 0x89, 0x4B, 0x20, 0xD9, 0x5C, 0x24, 0x4C, 0x8B,
    0x54, 0x24, 0x4C, 0x57, 0xB9, 0xFC, 0xC6, 0xDD, 0x00, 0x89, 0x53, 0x24, 0xC7, 0x43, 0x28, 0x00,
    0x40, 0x00, 0x00, 0xC6, 0x43, 0x7C, 0x00, 0xE8, 0x84, 0xB8, 0x6A, 0x00, 0x83, 0xC4, 0x04, 0x83,
    0x7C, 0x24, 0x10, 0x00, 0x74, 0x07, 0x81, 0x4B, 0x28, 0x00, 0x02, 0x00, 0x00, 0x57, 0xB9, 0x0C,
    0xC7, 0xDD, 0x00, 0x8D, 0x74, 0x24, 0x14, 0xE8, 0x64, 0xB8, 0x6A, 0x00, 0x83, 0xC4, 0x04, 0x83,
    0x7C, 0x24, 0x10, 0x00, 0x74, 0x07, 0x81, 0x4B, 0x28, 0x00, 0x08, 0x00, 0x00, 0x57, 0xB9, 0x1C,
    0xC7, 0xDD, 0x00, 0x8D, 0x74, 0x24, 0x14, 0xE8, 0x44, 0xB8, 0x6A, 0x00, 0x83, 0xC4, 0x04, 0x83,
    0x7C, 0x24, 0x10, 0x00, 0x74, 0x07, 0x81, 0x4B, 0x28, 0x00, 0x01, 0x00, 0x00, 0x57, 0xB9, 0x28,
    0xC7, 0xDD, 0x00, 0x8D, 0x74, 0x24, 0x14, 0xE8, 0x24, 0xB8, 0x6A, 0x00, 0x83, 0xC4, 0x04, 0x83,
    0x7C, 0x24, 0x10, 0x00, 0x74, 0x07, 0x81, 0x4B, 0x28, 0x00, 0x04, 0x00, 0x00, 0x85, 0xFF, 0x75,
    0x04, 0x33, 0xC0, 0xEB, 0x15, 0x8B, 0x44, 0x24, 0x0C, 0xBF, 0x34, 0xC7, 0xDD, 0x00, 0xE8, 0x0D,
    0x2F, 0x73, 0x00, 0x85, 0xC0, 0x74, 0x03, 0x8B, 0x40, 0x0C, 0x8D, 0x4B, 0x34, 0x51, 0x8D, 0x53,
    0x30, 0x52, 0x8D, 0x73, 0x2C, 0x56, 0x68, 0xE4, 0xC6, 0xDD, 0x00, 0x50, 0xE8, 0x98, 0xE0, 0x7C,
    0x00, 0x8B, 0x06, 0x8B, 0x4E, 0x04, 0x8B, 0x56, 0x08, 0x89, 0x43, 0x38, 0x83, 0xC4, 0x14, 0x83,
    0x7C, 0x24, 0x0C, 0x00, 0x89, 0x4B, 0x3C, 0x89, 0x53, 0x40, 0x75, 0x04, 0x33, 0xC0, 0xEB, 0x15,
    0x8B, 0x44, 0x24, 0x0C, 0xBF, 0x40, 0xC7, 0xDD, 0x00, 0xE8, 0xC2, 0x2E, 0x73, 0x00, 0x85, 0xC0,
    0x74, 0x03, 0x8B, 0x40, 0x0C, 0x8D, 0x8C, 0x24, 0x8C, 0x00, 0x00, 0x00, 0x51, 0x8D, 0x94, 0x24,
    0x8C, 0x00, 0x00, 0x00, 0x52, 0x8D, 0x8C, 0x24, 0x8C, 0x00, 0x00, 0x00, 0x51, 0x8D, 0x94, 0x24,
    0x8C, 0x00, 0x00, 0x00, 0x52, 0x68, 0x4C, 0xC7, 0xDD, 0x00, 0x50, 0xE8, 0x39, 0xE0, 0x7C, 0x00,
    0x83, 0xC4, 0x18, 0x8D, 0x8C, 0x24, 0xAC, 0x00, 0x00, 0x00, 0x8D, 0x84, 0x24, 0x80, 0x00, 0x00,
    0x00, 0xE8, 0x6A, 0xC7, 0x73, 0x00, 0x8D, 0x44, 0x24, 0x74, 0x50, 0x8D, 0x84, 0x24, 0xA4, 0x00,
    0x00, 0x00, 0xE8, 0xA9, 0x4D, 0x71, 0x00, 0x83, 0x7C, 0x24, 0x0C, 0x00, 0x8B, 0x08, 0x89, 0x4B,
    0x5C, 0x8B, 0x50, 0x04, 0x89, 0x53, 0x60, 0x8B, 0x40, 0x08, 0x89, 0x43, 0x64, 0x75, 0x04, 0x33,
    0xC0, 0xEB, 0x15, 0x8B, 0x44, 0x24, 0x0C, 0xBF, 0x58, 0xC7, 0xDD, 0x00, 0xE8, 0x3F, 0x2E, 0x73,
    0x00, 0x85, 0xC0, 0x74, 0x03, 0x8B, 0x40, 0x0C, 0x8D, 0x4B, 0x6C, 0x51, 0x8D, 0x53, 0x68, 0x52,
    0x68, 0x60, 0xC7, 0xDD, 0x00, 0x50, 0xE8, 0xCE, 0xDF, 0x7C, 0x00, 0x83, 0xC4, 0x10, 0x83, 0x7C,
    0x24, 0x0C, 0x00, 0x75, 0x04, 0x33, 0xC0, 0xEB, 0x15, 0x8B, 0x44, 0x24, 0x0C, 0xBF, 0x68, 0xC7,
    0xDD, 0x00, 0xE8, 0x09, 0x2E, 0x73, 0x00, 0x85, 0xC0, 0x74, 0x03, 0x8B, 0x40, 0x0C, 0x8D, 0x4B,
    0x74, 0x51, 0x8D, 0x53, 0x70, 0x52, 0x68, 0x60, 0xC7, 0xDD, 0x00, 0x50, 0xE8, 0x98, 0xDF, 0x7C,
    0x00, 0x8B, 0x74, 0x24, 0x1C, 0x83, 0xC4, 0x10, 0x85, 0xF6, 0x74, 0x3F, 0xBF, 0x74, 0xC7, 0xDD,
    0x00, 0x8B, 0xC6, 0xE8, 0xD8, 0x2D, 0x73, 0x00, 0x85, 0xC0, 0x74, 0x2F, 0x8B, 0x50, 0x0C, 0x85,
    0xD2, 0x74, 0x28, 0x8B, 0xC2, 0x8D, 0x78, 0x01, 0x8A, 0x08, 0x83, 0xC0, 0x01, 0x84, 0xC9, 0x75,
    0xF7, 0x2B, 0xC7, 0x74, 0x16, 0x52, 0x33, 0xD2, 0x8D, 0x84, 0x24, 0xA0, 0x00, 0x00, 0x00, 0xE8,
    0xFC, 0x38, 0x71, 0x00, 0x8B, 0x00, 0x89, 0x43, 0x78, 0xEB, 0x09, 0x8B, 0x0D, 0x08, 0x63, 0x3D,
    0x03, 0x89, 0x4B, 0x78, 0x83, 0x7C, 0x24, 0x34, 0x00, 0xC7, 0x83, 0x88, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xC7, 0x83, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0x83, 0x80,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x74, 0x21, 0x85, 0xF6, 0x74, 0x1D, 0x8B, 0x76, 0x04,
    0x85, 0xF6, 0x74, 0x16, 0x8B, 0x16, 0x68, 0xB8, 0xC6, 0xDD, 0x00, 0x52, 0xE8, 0xAF, 0xFE, 0x7C,
    0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x75, 0xE5, 0xEB, 0x02, 0x33, 0xF6, 0x8B, 0x44, 0x24, 0x14,
    0x8B, 0x4C, 0x24, 0x1C, 0x81, 0x44, 0x24, 0x18, 0x90, 0x00, 0x00, 0x00, 0x83, 0xC0, 0x01, 0x3B,
    0x41, 0x10, 0x89, 0x74, 0x24, 0x0C, 0x89, 0x44, 0x24, 0x14, 0x0F, 0x8C, 0x14, 0xFC, 0xFF, 0xFF,
    0x8B, 0x44, 0x24, 0x20, 0x85, 0xC0, 0x74, 0x27, 0x8B, 0x70, 0x08, 0x85, 0xF6, 0x74, 0x20, 0x90,
    0x8B, 0x16, 0x68, 0x80, 0xC7, 0xDD, 0x00, 0x52, 0xE8, 0x63, 0xFE, 0x7C, 0x00, 0x83, 0xC4, 0x08,
    0x85, 0xC0, 0x0F, 0x84, 0xB5, 0x00, 0x00, 0x00, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75, 0xE1, 0xC7,
    0x44, 0x24, 0x18, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x74, 0x24, 0x18, 0x68, 0x90, 0xC7, 0xDD, 0x00,
    0x8B, 0xC6, 0xE8, 0x19, 0x2F, 0x73, 0x00, 0x8B, 0x7C, 0x24, 0x20, 0x8B, 0x4C, 0x24, 0x28, 0x8B,
    0xD8, 0xC1, 0xE3, 0x04, 0x2B, 0xD8, 0x03, 0xDB, 0x83, 0xC4, 0x04, 0x89, 0x47, 0x1C, 0x8B, 0x11,
    0x8B, 0x42, 0x1C, 0x03, 0xDB, 0x6A, 0x00, 0x03, 0xDB, 0x53, 0xFF, 0xD0, 0x83, 0x7F, 0x1C, 0x00,
    0x89, 0x47, 0x18, 0x0F, 0x84, 0x28, 0x04, 0x00, 0x00, 0x85, 0xF6, 0x8B, 0xD8, 0x74, 0x22, 0x8B,
    0x76, 0x08, 0x85, 0xF6, 0x74, 0x1B, 0x8B, 0x0E, 0x68, 0x90, 0xC7, 0xDD, 0x00, 0x51, 0xE8, 0xED,
    0xFD, 0x7C, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x09, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x75,
    0xE5, 0x33, 0xF6, 0x8B, 0x54, 0x24, 0x1C, 0x83, 0x7A, 0x1C, 0x00, 0x89, 0x74, 0x24, 0x0C, 0xC7,
    0x44, 0x24, 0x14, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x8E, 0xE4, 0x03, 0x00, 0x00, 0xD9, 0x05, 0x98,
    0x79, 0xE8, 0x00, 0xD9, 0x54, 0x24, 0x68, 0xD9, 0x54, 0x24, 0x6C, 0xD9, 0x54, 0x24, 0x70, 0xD9,
    0x54, 0x24, 0x5C, 0xD9, 0x54, 0x24, 0x60, 0xD9, 0x5C, 0x24, 0x64, 0xEB, 0x0D, 0x89, 0x74, 0x24,
    0x18, 0xE9, 0x55, 0xFF, 0xFF, 0xFF, 0x8B, 0x74, 0x24, 0x0C, 0x85, 0xF6, 0x75, 0x04, 0x33, 0xC0,
    0xEB, 0x13, 0xBF, 0x74, 0xC7, 0xDD, 0x00, 0x8B, 0xC6, 0xE8, 0x32, 0x2C, 0x73, 0x00, 0x85, 0xC0,
    0x74, 0x03, 0x8B, 0x40, 0x0C, 0x50, 0x33, 0xD2, 0x8D, 0x84, 0x24, 0x98, 0x00, 0x00, 0x00, 0xE8,
    0x6C, 0x37, 0x71, 0x00, 0x85, 0xF6, 0x8B, 0x00, 0x89, 0x03, 0x75, 0x04, 0x33, 0xC0, 0xEB, 0x13,
    0xBF, 0xA0, 0xC7, 0xDD, 0x00, 0x8B, 0xC6, 0xE8, 0x04, 0x2C, 0x73, 0x00, 0x85, 0xC0, 0x74, 0x03,
    0x8B, 0x40, 0x0C, 0x8D, 0x4B, 0x0C, 0x51, 0x8D, 0x53, 0x08, 0x52, 0x8D, 0x4B, 0x04, 0x51, 0x68,
    0xE4, 0xC6, 0xDD, 0x00, 0x50, 0xE8, 0x8F, 0xDD, 0x7C, 0x00, 0x83, 0xC4, 0x14, 0x85, 0xF6, 0x75,
    0x04, 0x33, 0xC0, 0xEB, 0x13, 0xBF, 0xB4, 0xC7, 0xDD, 0x00, 0x8B, 0xC6, 0xE8, 0xCF, 0x2B, 0x73,
    0x00, 0x85, 0xC0, 0x74, 0x03, 0x8B, 0x40, 0x0C, 0x8D, 0x53, 0x18, 0x52, 0x8D, 0x4B, 0x14, 0x51,
    0x8D, 0x53, 0x10, 0x52, 0x68, 0xE4, 0xC6, 0xDD, 0x00, 0x50, 0xE8, 0x5A, 0xDD, 0x7C, 0x00, 0x8B,
    0x7C, 0x24, 0x20, 0x83, 0xC4, 0x14, 0x8D, 0x73, 0x1C, 0x57, 0xB9, 0xC8, 0xC7, 0xDD, 0x00, 0xE8,
    0x1C, 0xB5, 0x6A, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x06, 0xC7, 0x06, 0xFF, 0xFF, 0xFF,
    0xFF, 0x8D, 0x73, 0x20, 0x57, 0xB9, 0xE0, 0xC7, 0xDD, 0x00, 0xE8, 0x01, 0xB5, 0x6A, 0x00, 0x83,
    0xC4, 0x04, 0x84, 0xC0, 0x75, 0x06, 0xC7, 0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x8D, 0x73, 0x24, 0x57,
    0xB9, 0xF4, 0xC7, 0xDD, 0x00, 0xE8, 0xE6, 0xB4, 0x6A, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75,
    0x06, 0xC7, 0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x8D, 0x73, 0x28, 0x57, 0xB9, 0x08, 0xC8, 0xDD, 0x00,
    0xE8, 0xCB, 0xB4, 0x6A, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x06, 0xC7, 0x06, 0xFF, 0xFF,
    0xFF, 0xFF, 0x8D, 0x73, 0x54, 0x57, 0xB9, 0x1C, 0xC8, 0xDD, 0x00, 0xE8, 0xB0, 0xBB, 0x6A, 0x00,
    0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x04, 0xD9, 0xE8, 0xD9, 0x1E, 0x85, 0xFF, 0x8B, 0x74, 0x24,
    0x0C, 0x74, 0x39, 0xBF, 0x30, 0xC8, 0xDD, 0x00, 0x8B, 0xC6, 0xE8, 0x11, 0x2B, 0x73, 0x00, 0x85,
    0xC0, 0x74, 0x29, 0x8B, 0x40, 0x0C, 0x85, 0xC0, 0x74, 0x22, 0x8D, 0x4B, 0x34, 0x51, 0x8D, 0x53,
    0x30, 0x52, 0x8D, 0x4B, 0x2C, 0x51, 0x68, 0xE4, 0xC6, 0xDD, 0x00, 0x50, 0xE8, 0x98, 0xDC, 0x7C,
    0x00, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0x83, 0xC4, 0x14, 0xEB, 0x27, 0xD9, 0x05, 0x98, 0x79,
    0xE8, 0x00, 0xD9, 0x54, 0x24, 0x50, 0x8B, 0x54, 0x24, 0x50, 0xD9, 0x54, 0x24, 0x54, 0x8B, 0x44,
    0x24, 0x54, 0xD9, 0x54, 0x24, 0x58, 0x8B, 0x4C, 0x24, 0x58, 0x89, 0x53, 0x2C, 0x89, 0x43, 0x30,
    0x89, 0x4B, 0x34, 0x85, 0xF6, 0x74, 0x3B, 0xBF, 0x44, 0xC8, 0xDD, 0x00, 0xDD, 0xD8, 0x8B, 0xC6,
    0xE8, 0xAB, 0x2A, 0x73, 0x00, 0x85, 0xC0, 0x74, 0x23, 0x8B, 0x40, 0x0C, 0x85, 0xC0, 0x74, 0x1C,
    0x8D, 0x53, 0x40, 0x52, 0x8D, 0x4B, 0x3C, 0x51, 0x8D, 0x53, 0x38, 0x52, 0x68, 0xE4, 0xC6, 0xDD,
    0x00, 0x50, 0xE8, 0x32, 0xDC, 0x7C, 0x00, 0x83, 0xC4, 0x14, 0xEB, 0x27, 0xD9, 0x05, 0x98, 0x79,
    0xE8, 0x00, 0xD9, 0x54, 0x24, 0x38, 0x8B, 0x44, 0x24, 0x38, 0xD9, 0x54, 0x24, 0x3C, 0x8B, 0x4C,
    0x24, 0x3C, 0xD9, 0x5C, 0x24, 0x40, 0x8B, 0x54, 0x24, 0x40, 0x89, 0x43, 0x38, 0x89, 0x4B, 0x3C,
    0x89, 0x53, 0x40, 0x8B, 0x7C, 0x24, 0x0C, 0x8D, 0x73, 0x44, 0x57, 0xB9, 0x58, 0xC8, 0xDD, 0x00,
    0xE8, 0xCB, 0xBA, 0x6A, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x08, 0xD9, 0x05, 0x98, 0x79,
    0xE8, 0x00, 0xD9, 0x1E, 0x8D, 0x73, 0x48, 0x57, 0xB9, 0x68, 0xC8, 0xDD, 0x00, 0xE8, 0xAE, 0xBA,
    0x6A, 0x00, 0x83, 0xC4, 0x04, 0x84, 0xC0, 0x75, 0x08, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0xD9,
    0x1E, 0x8D, 0x73, 0x4C, 0x57, 0xB9, 0xF4, 0xC1, 0xDD, 0x00, 0xE8, 0x91, 0xBA, 0x6A, 0x00, 0x83,
    0xC4, 0x04, 0x84, 0xC0, 0x75, 0x08, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0xD9, 0x1E, 0x8D, 0x73,
    0x50, 0x57, 0xB9, 0x7C, 0xC8, 0xDD, 0x00, 0xE8, 0x74, 0xBA, 0x6A, 0x00, 0x83, 0xC4, 0x04, 0x84,
    0xC0, 0x75, 0x08, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0xD9, 0x1E, 0x57, 0xB9, 0x7C, 0xC8, 0xDD,
    0x00, 0xE8, 0x5A, 0xBA, 0x6A, 0x00, 0xD9, 0x05, 0x98, 0x79, 0xE8, 0x00, 0x83, 0xC4, 0x04, 0x84,
    0xC0, 0x75, 0x02, 0xD9, 0x16, 0x8B, 0x44, 0x24, 0x68, 0xD9, 0x53, 0x58, 0x8B, 0x4C, 0x24, 0x6C,
    0xD9, 0x5B, 0x5C, 0x8B, 0x54, 0x24, 0x70, 0x89, 0x43, 0x60, 0x8B, 0x44, 0x24, 0x5C, 0x89, 0x4B,
    0x64, 0x8B, 0x4C, 0x24, 0x60, 0x89, 0x43, 0x6C, 0x8B, 0x44, 0x24, 0x0C, 0x8D, 0x73, 0x58, 0x89,
    0x53, 0x68, 0x8B, 0x54, 0x24, 0x64, 0x89, 0x4B, 0x70, 0x8D, 0x7B, 0x5C, 0x50, 0xB9, 0xE8, 0xC1,
    0xDD, 0x00, 0x89, 0x53, 0x74, 0xE8, 0x06, 0xBA, 0x6A, 0x00, 0x8B, 0x4C, 0x24, 0x10, 0x83, 0xC4,
    0x04, 0x51, 0xB9, 0x00, 0xC2, 0xDD, 0x00, 0x8B, 0xF7, 0xE8, 0xF2, 0xB9, 0x6A, 0x00, 0x83, 0xC4,
    0x04, 0x83, 0x7C, 0x24, 0x0C, 0x00, 0x74, 0x57, 0x8B, 0x74, 0x24, 0x0C, 0xBF, 0x84, 0xC8, 0xDD,
    0x00, 0x8B, 0xC6, 0xE8, 0x58, 0x29, 0x73, 0x00, 0x85, 0xC0, 0x74, 0x21, 0x8B, 0x40, 0x0C, 0x85,
    0xC0, 0x74, 0x1A, 0x8D, 0x53, 0x68, 0x52, 0x8D, 0x4B, 0x64, 0x51, 0x8D, 0x4B, 0x60, 0x51, 0x68,
    0xE4, 0xC6, 0xDD, 0x00, 0x50, 0xE8, 0xDF, 0xDA, 0x7C, 0x00, 0x83, 0xC4, 0x14, 0x8B, 0x7E, 0x08,
    0x85, 0xFF, 0x74, 0x1B, 0x8B, 0x17, 0x68, 0x90, 0xC8, 0xDD, 0x00, 0x52, 0xE8, 0x6F, 0xFA, 0x7C,
    0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x74, 0x09, 0x8B, 0x7F, 0x04, 0x85, 0xFF, 0x75, 0xE5, 0x33,
    0xFF, 0x57, 0xB9, 0x0C, 0xC0, 0xDD, 0x00, 0x8D, 0x73, 0x6C, 0xE8, 0x81, 0xB9, 0x6A, 0x00, 0x83,
    0xC4, 0x04, 0x57, 0x8D, 0x73, 0x70, 0xB9, 0x98, 0xC8, 0xDD, 0x00, 0xE8, 0x70, 0xB9, 0x6A, 0x00,
    0x83, 0xC4, 0x04, 0x57, 0x8D, 0x73, 0x74, 0xB9, 0xA0, 0xC8, 0xDD, 0x00, 0xE8, 0x5F, 0xB9, 0x6A,
    0x00, 0x8B, 0x74, 0x24, 0x10, 0x83, 0xC4, 0x04, 0x83, 0x7C, 0x24, 0x18, 0x00, 0x74, 0x21, 0x85,
    0xF6, 0x74, 0x1D, 0x8B, 0x76, 0x04, 0x85, 0xF6, 0x74, 0x16, 0x8B, 0x06, 0x68, 0x90, 0xC7, 0xDD,
    0x00, 0x50, 0xE8, 0x09, 0xFA, 0x7C, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x75, 0xE5, 0xEB, 0x02,
    0x33, 0xF6, 0x8B, 0x44, 0x24, 0x14, 0x8B, 0x4C, 0x24, 0x1C, 0x83, 0xC0, 0x01, 0x83, 0xC3, 0x78,
    0x3B, 0x41, 0x1C, 0x89, 0x74, 0x24, 0x0C, 0x89, 0x44, 0x24, 0x14, 0x0F, 0x8C, 0x45, 0xFC, 0xFF,
    0xFF, 0x8B, 0x74, 0x24, 0x20, 0x85, 0xF6, 0x74, 0x24, 0x8D, 0xA4, 0x24, 0x00, 0x00, 0x00, 0x00,
    0x8B, 0x76, 0x04, 0x85, 0xF6, 0x74, 0x16, 0x8B, 0x16, 0x68, 0x90, 0xC6, 0xDD, 0x00, 0x52, 0xE8,
    0xBC, 0xF9, 0x7C, 0x00, 0x83, 0xC4, 0x08, 0x85, 0xC0, 0x75, 0xE5, 0xEB, 0x02, 0x33, 0xF6, 0x8B,
    0x44, 0x24, 0x30, 0x8B, 0x4C, 0x24, 0x28, 0x83, 0xC0, 0x01, 0x3B, 0x41, 0x08, 0x89, 0x74, 0x24,
    0x20, 0x89, 0x44, 0x24, 0x30, 0x0F, 0x8C, 0x05, 0xF6, 0xFF, 0xFF, 0x8B, 0x0D, 0x38, 0x71, 0xE8,
    0x00, 0x8B, 0x11, 0xA1, 0x34, 0x71, 0xE8, 0x00, 0x8B, 0x52, 0x34, 0x50, 0xFF, 0xD2, 0x8B, 0x8C,
    0x24, 0xE4, 0x01, 0x00, 0x00, 0x8B, 0x44, 0x24, 0x28, 0x5F, 0x5E, 0x5B, 0x33, 0xCC, 0xC7, 0x05,
    0x34, 0x71, 0xE8, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xE8, 0x82, 0xC9, 0x7C, 0x00, 0x8B, 0xE5, 0x5D,
    0xC3,
];
const TOD_CALLS: [usize; 73] = [0x2d, 0x47, 0x59, 0x62, 0xb0, 0xef, 0x14c, 0x166, 0x178, 0x18d, 0x1a8, 0x1c8, 0x1fd, 0x263, 0x27f, 0x2ca, 0x2ef, 0x305, 0x31d, 0x33c, 0x35a, 0x36f, 0x3c7, 0x3e7, 0x407, 0x427, 0x44e, 0x46c, 0x499, 0x4cb, 0x4e1, 0x4f2, 0x51c, 0x536, 0x552, 0x56c, 0x583, 0x5af, 0x5fc, 0x648, 0x672, 0x6be, 0x729, 0x73f, 0x757, 0x775, 0x78c, 0x7aa, 0x7bf, 0x7da, 0x7f5, 0x810, 0x82b, 0x84a, 0x86c, 0x8b0, 0x8d2, 0x910, 0x92d, 0x94a, 0x967, 0x981, 0x9d5, 0x9e9, 0xa03, 0xa25, 0xa3c, 0xa5a, 0xa6b, 0xa7c, 0xaa2, 0xaef, 0xb48];

#[cfg(test)]
const TOD_DISPS: [usize; 80] = [0xd, 0x6e, 0x80, 0x8e, 0x95, 0xaa, 0xea, 0x146, 0x172, 0x189, 0x1a3, 0x1be, 0x1f8, 0x238, 0x25f, 0x2eb, 0x300, 0x318, 0x338, 0x355, 0x36b, 0x3b5, 0x3df, 0x3ff, 0x41f, 0x44a, 0x467, 0x495, 0x4c6, 0x518, 0x531, 0x54e, 0x567, 0x57d, 0x5bd, 0x5f7, 0x643, 0x66c, 0x6b9, 0x6ef, 0x723, 0x751, 0x770, 0x786, 0x7a5, 0x7bb, 0x7d6, 0x7f1, 0x80c, 0x827, 0x844, 0x867, 0x873, 0x87e, 0x8a8, 0x8cd, 0x8de, 0x90c, 0x91e, 0x929, 0x93b, 0x946, 0x958, 0x963, 0x975, 0x97d, 0x988, 0x9ce, 0x9e3, 0x9fd, 0xa20, 0xa37, 0xa53, 0xa67, 0xa78, 0xa9d, 0xaea, 0xb1d, 0xb24, 0xb40];

#[cfg(test)]
pub(crate) mod difftest {
    #![allow(warnings)]
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};

    pub const EAX: u8 = 1;
    pub const ECX: u8 = 2;
    pub const EDX: u8 = 4;
    pub const ESI: u8 = 8;
    pub const EDI: u8 = 16;

    /// Where a callee's result comes from.
    #[derive(Clone, Copy, PartialEq)]
    pub enum Ret {
        /// Nothing in EAX that matters (a random value comes back anyway).
        Any,
        /// A flag: 0, 1, 0x100 (AL clear but EAX not), or random.
        Bool,
        /// 0 or a member of pool `n`.
        Ptr(usize),
        /// Always 0.
        Zero,
        /// Always a member of pool `n`.
        PtrNz(usize),
        /// Returns a double on the x87 stack (and a random value in EAX).
        St0,
        /// A random value below `n`.
        Mod(u32),
        /// EAX comes back unchanged (the /GS cookie check).
        Keep,
    }

    /// What a callee does besides returning.
    #[derive(Clone, Copy, PartialEq)]
    pub enum Fx {
        None,
        /// Writes a random u32 at [ESI] (the xtbl getters).
        WriteUint,
        /// Writes a random f32 at [ESI].
        WriteFloat,
        /// Writes a random small count (0..6) at [ESI].
        WriteSmall,
        /// Logs the bytes of a buffer: `args[0]` pointer, `args[1]` length (byte 3 masked when the
        /// length is 9: the original leaves it unwritten).
        LogBuf,
        /// Logs `ECX * 3` dwords at [EDX] (a vector list).
        LogVec,
        /// Logs the three bytes at [EDX].
        LogEdx3,
        /// Logs `n` dwords at the pointer in `args[0]`.
        LogArg0(usize),
        /// Logs `n` bytes at the pointer in EAX.
        LogEaxBytes(usize),
        /// Removes one handle record from the player's pending list (args: player, object).
        RemoveListEntry,
    }

    #[derive(Clone)]
    pub struct Callee {
        pub target: usize,
        pub nargs: usize,
        pub pop: usize,
        /// Registers the callee reads on entry (the only ones compared).
        pub reads: u8,
        pub ret: Ret,
        pub fx: Fx,
        pub silent: bool,
        /// Stack arguments the callee reads as one byte (the caller leaves stale upper bits).
        pub byte_args: u32,
        /// Registers the callee reads as one byte (the caller leaves stale upper bits).
        pub byte_regs: u8,
        /// A native replacement the original's call is aimed at (0: the recording stub).
        pub native: usize,
        pub func: Option<fn(&mut Pushad, *const u32) -> Option<u32>>,
        pub scanf: bool,
    }

    pub fn callee(target: usize, nargs: usize, pop: usize, reads: u8, ret: Ret, fx: Fx) -> Callee {
        Callee { target, nargs, pop, reads, ret, fx, silent: false, byte_args: 0, byte_regs: 0, native: 0, func: None, scanf: false }
    }

    /// Prints what already occupies the address space the game's globals need.
    pub fn dump_occupied() {
        use windows_sys::Win32::System::Memory::{MEMORY_BASIC_INFORMATION, MEM_FREE, VirtualQuery};
        let mut a = 0x40_0000usize;
        while a < 0x400_0000 {
            let mut m: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe { VirtualQuery(a as *const c_void, &mut m, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
                break;
            }
            if m.State != MEM_FREE {
                eprintln!("occupied {:#x}..{:#x} state {:#x} type {:#x}", m.BaseAddress as usize, m.BaseAddress as usize + m.RegionSize, m.State, m.Type);
            }
            a = m.BaseAddress as usize + m.RegionSize;
        }
    }

    pub struct Rng(pub u64);
    impl Rng {
        pub fn u32(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
        pub fn below(&mut self, n: u32) -> u32 {
            self.u32() % n
        }
        /// True once in `n`.
        pub fn one_in(&mut self, n: u32) -> bool {
            self.below(n) == 0
        }
        /// One of `v`, or (one time in four) a random value below `modulus` (any u32 when 0).
        pub fn mix(&mut self, v: &[u32], modulus: u32) -> u32 {
            if self.below(4) == 0 {
                let r = self.u32();
                if modulus == 0 { r } else { r % modulus }
            } else {
                self.pick(v)
            }
        }
        pub fn pick<T: Copy>(&mut self, v: &[T]) -> T {
            v[self.below(v.len() as u32) as usize]
        }
        /// A float bit pattern with the awkward cases over-represented.
        pub fn float_bits(&mut self) -> u32 {
            match self.below(12) {
                0 => 0,
                1 => 0x8000_0000,
                2 => 0x3F80_0000,
                3 => 0xBF80_0000,
                4 => 0x7FC0_0000,
                5 => 0x7F80_0001, // signalling NaN
                6 => 0x7F80_0000,
                7 => 0x0000_0001, // denormal
                8 => (self.below(2000) as f32 / 1000.0 - 0.5).to_bits(),
                9 => (self.below(20000) as f32 / 10.0).to_bits(),
                _ => self.u32(),
            }
        }
    }

    pub fn fixed(addr: usize, len: usize) {
        let mut p = unsafe { VirtualAlloc(addr as *const c_void, len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
        if p.is_null() {
            p = unsafe { VirtualAlloc(addr as *const c_void, len, MEM_COMMIT, PAGE_EXECUTE_READWRITE) };
        }
        assert_eq!(p as usize, addr, "cannot map {addr:#x}: error {}", unsafe { windows_sys::Win32::Foundation::GetLastError() });
    }

    /// The fixed regions the original touches: snapshotted, restored and compared as one.
    pub struct Mem {
        pub regions: Vec<(usize, usize)>,
    }

    impl Mem {
        /// Regions: every absolute address in `body` that falls in the game's data range (moved by
        /// `DELTA`), plus `extra` (game addresses), plus the arena; all rounded out to 64 KB.
        pub fn new(body: &[u8], extra: &[(usize, usize)]) -> Mem {
            let mut spans: Vec<(usize, usize)> = extra.iter().map(|&(a, l)| (super::tr(a), l)).collect();
            for i in 0..body.len().saturating_sub(3) {
                let v = u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]) as usize;
                if (0xD0_0000..0x350_0000).contains(&v) {
                    spans.push((super::tr(v), 1));
                }
            }
            spans.push((ARENA, ARENA_LEN));
            let mut pages: Vec<usize> = Vec::new();
            for (a, l) in spans {
                let mut p = a & !0xFFFF;
                while p < a + l {
                    pages.push(p);
                    p += 0x1_0000;
                }
            }
            pages.sort_unstable();
            pages.dedup();
            let mut regions: Vec<(usize, usize)> = Vec::new();
            for p in pages {
                match regions.last_mut() {
                    Some(r) if r.0 + r.1 == p => r.1 += 0x1_0000,
                    _ => regions.push((p, 0x1_0000)),
                }
            }
            for &(a, l) in &regions {
                fixed(a, l);
            }
            Mem { regions }
        }

        pub fn fill_random(&self, rng: &mut Rng) {
            for &(a, l) in &self.regions {
                let s = unsafe { std::slice::from_raw_parts_mut(a as *mut u32, l / 4) };
                for w in s.iter_mut() {
                    *w = rng.u32();
                }
            }
        }

        /// Zero `l` bytes at the game address `a`.
        pub fn zero(&self, a: usize, l: usize) {
            unsafe { std::ptr::write_bytes(super::tr(a) as *mut u8, 0, l) };
        }

        pub fn snapshot(&self) -> Vec<u8> {
            let mut v = Vec::new();
            for &(a, l) in &self.regions {
                v.extend_from_slice(unsafe { std::slice::from_raw_parts(a as *const u8, l) });
            }
            v
        }

        pub fn restore(&self, snap: &[u8]) {
            let mut at = 0;
            for &(a, l) in &self.regions {
                unsafe { std::ptr::copy_nonoverlapping(snap[at..at + l].as_ptr(), a as *mut u8, l) };
                at += l;
            }
        }

        /// The first difference between two snapshots, as an address.
        pub fn first_diff(&self, a: &[u8], b: &[u8]) -> Option<(usize, u8, u8)> {
            let mut at = 0;
            for &(base, l) in &self.regions {
                for i in 0..l {
                    if a[at + i] != b[at + i] {
                        return Some((base + i, a[at + i], b[at + i]));
                    }
                }
                at += l;
            }
            None
        }

        pub fn count_diffs(&self, a: &[u8], b: &[u8]) -> usize {
            a.iter().zip(b).filter(|(x, y)| x != y).count()
        }
    }

    #[repr(C)]
    pub struct Pushad {
        pub edi: u32,
        pub esi: u32,
        pub ebp: u32,
        pub esp: u32,
        pub ebx: u32,
        pub edx: u32,
        pub ecx: u32,
        pub eax: u32,
    }

    thread_local! {
        static CALLEES: RefCell<Vec<Callee>> = const { RefCell::new(Vec::new()) };
        static STUBS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
        static POOLS: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
        static LOG: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
        static SRNG: Cell<u64> = const { Cell::new(1) };
    }

    pub fn set_pools(p: Vec<Vec<u32>>) {
        POOLS.with(|x| *x.borrow_mut() = p);
    }

    /// The address a call to `f` should really go to: its stub when it has one.
    pub fn redirect(f: usize) -> usize {
        CALLEES.with(|c| {
            let c = c.borrow();
            match c.iter().position(|x| x.target == f) {
                Some(i) if c[i].native != 0 => c[i].native,
                Some(i) => STUBS.with(|s| s.borrow()[i]),
                None => f,
            }
        })
    }

    pub fn srng() -> u32 {
        SRNG.with(|s| {
            let mut r = Rng(s.get());
            let v = r.u32();
            s.set(r.0);
            v
        })
    }

    extern "C" fn stub_rs(regs: *mut Pushad, idx: u32, args: *const u32) -> u32 {
        let c = CALLEES.with(|c| c.borrow()[idx as usize].clone());
        let r = unsafe { &*regs };
        if TRACE.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("stub {:#x} eax {:#x} ecx {:#x} edx {:#x} esi {:#x} edi {:#x} a0 {:#x} a1 {:#x}", c.target, r.eax, r.ecx, r.edx, r.esi, r.edi, unsafe { *args }, unsafe { *args.add(1) });
        }
        // Pointers into the stack differ between the two runs: log a marker for them.
        let mask = |v: u32| if (STACK..STACK + STACK_LEN).contains(&(v as usize)) { 0x57AC_0000 } else { v };
        if !c.silent {
            let mut e = vec![c.target as u32];
            for (bit, v) in [(EAX, r.eax), (ECX, r.ecx), (EDX, r.edx), (ESI, r.esi), (EDI, r.edi)] {
                e.push(if c.reads & bit == 0 { 0 } else if c.byte_regs & bit != 0 { v & 0xFF } else { mask(v) });
            }
            let nargs = if c.scanf { 2 + cstr_pct(unsafe { *args.add(1) }) } else { c.nargs };
            for k in 0..nargs {
                let v = unsafe { *args.add(k) };
                e.push(if c.byte_args >> k & 1 != 0 { v & 0xFF } else { mask(v) });
            }
            match c.fx {
                Fx::LogVec => {
                    for k in 0..(r.ecx.min(40) * 3) as usize {
                        e.push(mask(unsafe { *((r.edx as usize + k * 4) as *const u32) }));
                    }
                }
                Fx::LogEdx3 => {
                    for k in 0..3usize {
                        e.push(unsafe { *((r.edx as usize + k) as *const u8) } as u32);
                    }
                }
                Fx::LogEaxBytes(n) => {
                    for k in 0..n {
                        e.push(unsafe { *((r.eax as usize + k) as *const u8) } as u32);
                    }
                }
                Fx::LogArg0(n) => {
                    let p = unsafe { *args } as usize;
                    for k in 0..n {
                        e.push(mask(unsafe { *((p + k * 4) as *const u32) }));
                    }
                }
                _ => {}
            }
            if c.fx == Fx::LogBuf {
                let (p, len) = unsafe { (*args as usize, *args.add(1) as usize) };
                assert!(len < 64, "LogBuf: callee {:#x} len {len:#x} ptr {p:#x}", c.target);
                for k in 0..len {
                    let mut b = unsafe { *((p + k) as *const u8) };
                    if (len == 9 || len == 7) && k == 3 {
                        b = 0;
                    }
                    e.push(b as u32);
                }
            }
            LOG.with(|l| l.borrow_mut().push(e));
        }
        if let Some(f) = c.func {
            if let Some(v) = f(unsafe { &mut *regs }, args) {
                return v;
            }
        }
        if c.fx == Fx::RemoveListEntry {
            unsafe {
                let p = *args as usize;
                let o = *args.add(1);
                let cnt = *((p + 0xC3C) as *const u32) as usize;
                if cnt > 0 && cnt < 64 {
                    let obj_of = |h: u32| -> u32 {
                        if h == 0 {
                            return 0;
                        }
                        let e = *(super::tr(0x0214_9C64 + (((h & 0xFFF) as usize) << 4)) as *const u32);
                        if e != 0 && *((e as usize + 0x44) as *const u32) == h { e } else { 0 }
                    };
                    let hk = |k: usize| *((p + 0xC40 + k * 0x18) as *const u32);
                    let mut sel = None;
                    if o != 0 {
                        sel = (0..cnt).find(|&k| obj_of(hk(k)) == o);
                    }
                    if sel.is_none() {
                        sel = (0..cnt).find(|&k| obj_of(hk(k)) == 0);
                    }
                    let j = sel.unwrap_or(0);
                    for k in j..cnt - 1 {
                        std::ptr::copy((p + 0xC40 + (k + 1) * 0x18) as *const u8, (p + 0xC40 + k * 0x18) as *mut u8, 0x18);
                    }
                    *((p + 0xC3C) as *mut u32) = (cnt - 1) as u32;
                }
            }
        }
        match c.fx {
            Fx::WriteUint => {
                let v = match srng() % 6 {
                    0 => 0,
                    1 => srng() % 10,
                    2 => srng() % 100,
                    3 => srng() % 5000,
                    4 => 0x7FFF_FFFF,
                    _ => srng(),
                };
                unsafe { *(super::tr(r.esi as usize) as *mut u32) = v };
            }
            Fx::WriteSmall => {
                let v = srng() % 6;
                unsafe { *(super::tr(r.esi as usize) as *mut u32) = v };
            }
            Fx::WriteFloat => {
                let mut rng = Rng(srng() as u64 | 1 << 40);
                unsafe { *(super::tr(r.esi as usize) as *mut u32) = rng.float_bits() };
            }
            _ => {}
        }
        match c.ret {
            Ret::Zero => 0,
            Ret::Mod(n) => srng() % n,
            Ret::Keep => r.eax,
            Ret::Any => srng(),
            Ret::St0 => {
                let v = match srng() % 8 {
                    0 => 0.0,
                    1 => f32::from_bits(srng()) as f64,
                    2 => srng() as f64 / 7.0,
                    3 => (srng() % 5000) as f64 / 10.0,
                    4 => f64::from_bits((srng() as u64) << 32 | srng() as u64),
                    5 => 1000.0,
                    _ => (srng() % 100) as f64,
                };
                unsafe { ST0V = v };
                srng()
            }
            Ret::PtrNz(n) => POOLS.with(|p| {
                let p = p.borrow();
                let pool = &p[n];
                pool[srng() as usize % pool.len()]
            }),
            Ret::Bool => match srng() % 6 {
                0 | 1 => 0,
                2 | 3 => 1,
                4 => 0x100,
                _ => srng(),
            },
            Ret::Ptr(n) => {
                if srng() % 4 == 0 {
                    0
                } else {
                    POOLS.with(|p| {
                        let p = p.borrow();
                        let pool = &p[n];
                        pool[srng() as usize % pool.len()]
                    })
                }
            }
        }
    }

    /// The number of conversions in the printf-style format at `p`.
    fn cstr_pct(p: u32) -> usize {
        let mut n = 0;
        let mut q = p as usize;
        unsafe {
            while *(q as *const u8) != 0 {
                if *(q as *const u8) == b'%' {
                    n += 1;
                }
                q += 1;
            }
        }
        n
    }

    static mut ST0V: f64 = 0.0;
    pub static TRACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// `stub_common` for the callees that return on ST0.
    #[unsafe(naked)]
    extern "C" fn stub_common_st0() {
        std::arch::naked_asm!(
            "pushad",
            "lea eax, [esp + 44]",
            "push eax",
            "push dword ptr [esp + 40]",
            "lea eax, [esp + 8]",
            "push eax",
            "call {f}",
            "add esp, 12",
            "mov [esp + 28], eax",
            "popad",
            "fld qword ptr [{v}]",
            "ret 4",
            f = sym stub_rs,
            v = sym ST0V,
        )
    }

    #[unsafe(naked)]
    extern "C" fn stub_common() {
        std::arch::naked_asm!(
            "pushad",
            "lea eax, [esp + 44]",
            "push eax",
            "push dword ptr [esp + 40]",
            "lea eax, [esp + 8]",
            "push eax",
            "call {f}",
            "add esp, 12",
            "mov [esp + 28], eax",
            "popad",
            "ret 4",
            f = sym stub_rs,
        )
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub struct Regs {
        pub eax: u32,
        pub ecx: u32,
        pub edx: u32,
        pub ebx: u32,
        pub esi: u32,
        pub edi: u32,
        pub ebp: u32,
    }

    #[repr(C)]
    struct Block {
        f: u32,
        r: Regs,
        args: [u32; 3],
        esp: u32,
        stack: u32,
    }

    /// Call `f` with all seven registers from `r` and three stack arguments, capture them after.
    pub unsafe fn call_orig(f: usize, r: &mut Regs, args: [u32; 3]) {
        let mut b = Block { f: f as u32, r: *r, args, esp: 0, stack: (STACK + STACK_LEN - 0x100) as u32 };
        unsafe {
            std::arch::asm!(
                "push ebp", "push ebx", "push esi", "push edi",
                "mov [eax + 44], esp",
                "mov esp, [eax + 48]",
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
                "mov esp, [ebx + 44]",
                "pop edi", "pop esi", "pop ebx", "pop ebp",
                inout("eax") &mut b as *mut Block => _,
                lateout("ecx") _,
                lateout("edx") _,
                clobber_abi("C"),
            );
        }
        *r = b.r;
    }

    /// Reports a crash with the faulting address and where it happened (original copy offset).
    unsafe extern "system" fn veh(info: *mut windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
        unsafe {
            let rec = &*(*info).ExceptionRecord;
            let ctx = &*(*info).ContextRecord;
            let orig = ORIG_AT.with(|o| o.get());
            eprintln!(
                "EXCEPTION {:#x} at eip {:#x} (original +{:#x}) access {:x?} eax {:#x} ecx {:#x} edx {:#x} ebx {:#x} esi {:#x} edi {:#x} ebp {:#x}",
                rec.ExceptionCode, ctx.Eip, (ctx.Eip as usize).wrapping_sub(orig), &rec.ExceptionInformation[..2], ctx.Eax, ctx.Ecx, ctx.Edx, ctx.Ebx, ctx.Esi, ctx.Edi, ctx.Ebp
            );
            eprintln!("{}", std::backtrace::Backtrace::force_capture());
        }
        0
    }

    thread_local! {
        static ORIG_AT: Cell<usize> = const { Cell::new(0) };
    }

    pub struct Harness {
        pub mem: Mem,
        /// The original body, relocated to a private copy with its calls aimed at the stubs.
        pub orig: usize,
        pub thunk: usize,
        /// Also compare EAX after the call.
        pub cmp_eax: bool,
        /// The registers the call starts with.
        pub input: Cell<Regs>,
        pub reset: Option<fn()>,
    }

    /// Executable memory for the stubs and the patched copy.
    pub fn exec_alloc(len: usize) -> usize {
        let p = unsafe { VirtualAlloc(std::ptr::null(), len, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
        assert!(p != 0);
        p
    }

    impl Harness {
        /// `base`: the original's address; `body` / `calls`: its bytes and the offsets of its `call
        /// rel32`s; `thunk`: the Rust replacement's entry.
        pub fn new(base: usize, body: &[u8], calls: &[usize], disps: &[usize], callees: Vec<Callee>, extra: &[(usize, usize)], thunk: usize) -> Harness {
            let mem = Mem::new(body, extra);
            fixed(STACK, STACK_LEN);
            let stubs = exec_alloc(callees.len() * 16 + 16);
            let mut addrs = Vec::new();
            for (i, c) in callees.iter().enumerate() {
                let a = stubs + i * 16;
                let mut code = [0x90u8; 16];
                code[0] = 0x68;
                code[1..5].copy_from_slice(&(i as u32).to_le_bytes());
                code[5] = 0xE8;
                let common = if c.ret == Ret::St0 { stub_common_st0 as usize } else { stub_common as usize };
                let rel = (common as i32).wrapping_sub((a + 10) as i32);
                code[6..10].copy_from_slice(&rel.to_le_bytes());
                if c.pop == 0 {
                    code[10] = 0xC3;
                } else {
                    code[10] = 0xC2;
                    code[11..13].copy_from_slice(&(c.pop as u16).to_le_bytes());
                }
                unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), a as *mut u8, 16) };
                addrs.push(a);
            }
            CALLEES.with(|c| *c.borrow_mut() = callees.clone());
            STUBS.with(|s| *s.borrow_mut() = addrs.clone());
            let orig = exec_alloc(body.len() + 16);
            unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), orig as *mut u8, body.len()) };
            for &off in disps {
                let v = u32::from_le_bytes(body[off..off + 4].try_into().unwrap()).wrapping_add(super::DELTA as u32);
                unsafe { std::ptr::write_unaligned((orig + off) as *mut u32, v) };
            }
            for &off in calls {
                let rel = i32::from_le_bytes(body[off + 1..off + 5].try_into().unwrap());
                let target = (base + off + 5).wrapping_add(rel as usize);
                let i = callees.iter().position(|c| c.target == target).unwrap_or_else(|| panic!("call at {off:#x} to {target:#x} has no callee entry"));
                let dest = if callees[i].native != 0 { callees[i].native } else { addrs[i] };
                let r = (dest as i32).wrapping_sub((orig + off + 5) as i32);
                unsafe { std::ptr::write_unaligned((orig + off + 1) as *mut i32, r) };
            }
            ORIG_AT.with(|o| o.set(orig));
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| unsafe {
                windows_sys::Win32::System::Diagnostics::Debug::AddVectoredExceptionHandler(1, Some(veh));
            });
            Harness { mem, orig, thunk, cmp_eax: false, reset: None, input: Cell::new(Regs { eax: 0xAAAA_0001, ecx: 0, edx: 0xDDDD_0003, ebx: 0xB1B1_B1B1, esi: 0x5151_5151, edi: 0xD1D1_D1D1, ebp: 0xE9E9_E9E9 }) }
        }

        fn run(&self, f: usize, args: [u32; 3], seed: u64) -> (Vec<Vec<u32>>, Regs, Regs) {
            if let Some(r) = self.reset {
                r();
            }
            SRNG.with(|s| s.set(seed | 1));
            LOG.with(|l| l.borrow_mut().clear());
            let before = self.input.get();
            let mut r = before;
            unsafe { call_orig(f, &mut r, args) };
            (LOG.with(|l| std::mem::take(&mut *l.borrow_mut())), before, r)
        }

        /// One case on the current memory: original, then Rust, from the same snapshot.
        pub fn check(&self, args: [u32; 3], seed: u64) -> Result<(), String> {
            let snap = self.mem.snapshot();
            let (la, ba, ra) = self.run(self.orig, args, seed);
            let after_a = self.mem.snapshot();
            self.mem.restore(&snap);
            let (lb, _bb, rb) = self.run(self.thunk, args, seed);
            let after_b = self.mem.snapshot();
            if self.cmp_eax && ra.eax != rb.eax {
                return Err(format!("eax: original {:#x} rust {:#x}", ra.eax, rb.eax));
            }
            // callee-saved registers must come back unchanged
            for (name, x, y, z) in [("ebx", ba.ebx, ra.ebx, rb.ebx), ("esi", ba.esi, ra.esi, rb.esi), ("edi", ba.edi, ra.edi, rb.edi), ("ebp", ba.ebp, ra.ebp, rb.ebp)] {
                if x != y || x != z {
                    return Err(format!("register {name}: in {x:#x} original {y:#x} rust {z:#x}"));
                }
            }
            for (i, (a, b)) in la.iter().zip(&lb).enumerate() {
                if a != b {
                    let prev: Vec<&Vec<u32>> = la[i.saturating_sub(3)..i].iter().collect();
                    return Err(format!("call #{i} differs:
  previous {:x?}
  original {:x?}
  rust     {:x?}", prev, a, b));
                }
            }
            if la.len() != lb.len() {
                let longer = if la.len() > lb.len() { ("original", &la) } else { ("rust", &lb) };
                return Err(format!("original made {} calls, rust {}; the {} next calls {:x?}", la.len(), lb.len(), longer.0, longer.1[la.len().min(lb.len())]));
            }
            if let Some((a, x, y)) = self.mem.first_diff(&after_a, &after_b) {
                return Err(format!("memory differs at {a:#x}: original {x:#x} rust {y:#x} ({} bytes differ)", self.mem.count_diffs(&after_a, &after_b)));
            }
            Ok(())
        }
    }

    pub fn lock() -> std::sync::MutexGuard<'static, ()> {
        static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Where the test world's objects live.
    pub const ARENA: usize = 0x6000_0000;
    /// The tests run on a stack of their own, outside the game's address range.
    pub const STACK: usize = 0x6100_0000;
    pub const STACK_LEN: usize = 0x10_0000;
    pub const ARENA_LEN: usize = 0x18_0000;

    /// A bump allocator over the arena for one case.
    pub struct Arena(pub usize);
    impl Arena {
        pub fn new() -> Arena {
            Arena(ARENA)
        }
        /// `n` objects of `size` bytes (random contents are the caller's business).
        pub fn pool(&mut self, n: usize, size: usize) -> Vec<u32> {
            let v = (0..n).map(|i| (self.0 + i * size) as u32).collect();
            self.0 += n * size;
            assert!(self.0 <= ARENA + ARENA_LEN, "arena full");
            v
        }
    }

    pub unsafe fn put(a: u32, off: usize, v: u32) {
        unsafe { *((a as usize + off) as *mut u32) = v }
    }
    pub unsafe fn putb(a: u32, off: usize, v: u8) {
        unsafe { *((a as usize + off) as *mut u8) = v }
    }
}


// =================================================================================================
// Offline differential test
// =================================================================================================
//
// Runs the ORIGINAL bytes (relocated to a private copy whose absolute references are shifted by DELTA and whose
// calls go to functional stubs: a real xtbl node tree, stricmp, sscanf, hashes, getters, pool allocator) and the
// Rust port (through its real thunk) on the same random light-set world, and compares: EAX, the callee-saved
// registers, every byte of every mapped region (the light-set table, the cache slots, the allocated records and
// the whole heap), and the stream of calls into the game (target, the registers each callee reads, its stack
// arguments).
//
// Run (the test exe must not sit on the game's addresses):
//   RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000" cargo test --release
//   -p sr2-hook --target i686-pc-windows-msvc tod_lights -- --ignored --test-threads=1 --nocapture

#[cfg(test)]
pub(crate) mod difftest_hooks {
    use std::cell::{Cell, RefCell};
    thread_local! {
        /// What the port believes the getter buffer holds when it calls a getter with a missing child.
        pub static EXPECT: Cell<Option<u32>> = const { Cell::new(None) };
        /// What the stubs' model of the getter buffer holds (the string the previous getter parsed).
        pub static STALE: Cell<u32> = const { Cell::new(0) };
        /// Garbage-path calls where the port's belief differed from the stub model.
        pub static BAD: Cell<u32> = const { Cell::new(0) };
        pub static GARBAGE_CALLS: Cell<u32> = const { Cell::new(0) };
        pub static SCRIPT_OK: Cell<bool> = const { Cell::new(true) };
        /// (file name string, root node)
        pub static ROOTS: RefCell<Vec<(u32, u32)>> = const { RefCell::new(Vec::new()) };
        pub static ROOT_POOL: Cell<u32> = const { Cell::new(0) };
    }
    pub fn expect_stale(v: u32) {
        EXPECT.with(|e| e.set(Some(v)));
    }
}

#[cfg(test)]
mod tests {
    use super::difftest::*;
    use super::difftest_hooks::*;
    use super::*;

    const V: usize = 0xCB20_0000; // virtual-function stub ids: V + vtable offset
    const HEAP: usize = ARENA + 0x10_0000; // [HEAP] = bump cursor; allocations follow

    fn cases() -> u32 {
        std::env::var("CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(4000)
    }

    fn arg(a: *const u32, i: usize) -> u32 {
        unsafe { *a.add(i) }
    }
    fn cs(p: u32) -> Vec<u8> {
        let mut v = Vec::new();
        if p != 0 {
            let mut q = p as usize;
            unsafe {
                while *(q as *const u8) != 0 && v.len() < 0x400 {
                    v.push(*(q as *const u8));
                    q += 1;
                }
            }
        }
        v
    }
    fn lc(v: &[u8]) -> Vec<u8> {
        v.iter().map(|b| b.to_ascii_lowercase()).collect()
    }
    fn hsh(s: &[u8]) -> u32 {
        let mut h = 0x811C_9DC5u32;
        for b in lc(s) {
            h = (h ^ b as u32).wrapping_mul(0x0100_0193);
        }
        h
    }
    fn child(node: u32, name: u32) -> u32 {
        if node == 0 {
            return 0;
        }
        let want = lc(&cs(name));
        let mut c = unsafe { *((node + 8) as *const u32) };
        while c != 0 {
            let nm = unsafe { *(c as *const u32) };
            if lc(&cs(nm)) == want {
                return c;
            }
            c = unsafe { *((c + 4) as *const u32) };
        }
        0
    }
    fn value(node: u32, name: u32) -> u32 {
        let c = child(node, name);
        if c == 0 { 0 } else { unsafe { *((c + 0xC) as *const u32) } }
    }
    fn parse_i(s: &[u8]) -> u32 {
        let t = String::from_utf8_lossy(s).to_string();
        let t = t.trim_start();
        let mut end = 0;
        for (i, ch) in t.char_indices() {
            if ch.is_ascii_digit() || (i == 0 && (ch == '-' || ch == '+')) {
                end = i + 1;
            } else {
                break;
            }
        }
        t[..end].parse::<i32>().unwrap_or(0) as u32
    }
    fn parse_f(s: &[u8]) -> f32 {
        let t = String::from_utf8_lossy(s).to_string();
        let t = t.trim_start();
        let mut end = 0;
        for (i, ch) in t.char_indices() {
            if ch.is_ascii_digit() || ch == '.' || ch == 'e' || ch == 'E' || (i == 0 && (ch == '-' || ch == '+')) {
                end = i + 1;
            } else {
                break;
            }
        }
        t[..end].parse::<f64>().unwrap_or(0.0) as f32
    }
    unsafe fn put_dword(p: u32, v: u32) {
        unsafe { *(p as *mut u32) = v }
    }

    // --- functional stubs (fn(registers, stack args) -> EAX) ----------------------------------------------

    fn s_script(_r: &mut Pushad, _a: *const u32) -> Option<u32> {
        let ok = SCRIPT_OK.with(|s| s.get());
        Some((srng() & !0xFF) | ok as u32)
    }
    fn s_hash(r: &mut Pushad, a: *const u32) -> Option<u32> {
        let out = arg(a, 0);
        unsafe { put_dword(out, if r.ecx == 0 { 0 } else { hsh(&cs(r.ecx)) }) };
        Some(out)
    }
    fn s_hash_slate(r: &mut Pushad, a: *const u32) -> Option<u32> {
        let s = arg(a, 0);
        unsafe { put_dword(r.eax, if s == 0 { 0 } else { hsh(&cs(s)) ^ 0x5A5A }) };
        Some(r.eax)
    }
    fn s_root(r: &mut Pushad, _a: *const u32) -> Option<u32> {
        unsafe {
            put_dword(super::tr(SCOPE_POOL) as u32, ROOT_POOL.with(|p| p.get()));
            put_dword(super::tr(SCOPE_MARK) as u32, srng());
        }
        let want = lc(&cs(r.eax));
        let hit = ROOTS.with(|r| r.borrow().iter().find(|(n, _)| lc(&cs(*n)) == want).map(|x| x.1));
        Some(hit.unwrap_or(0))
    }
    fn s_find(r: &mut Pushad, _a: *const u32) -> Option<u32> {
        Some(child(r.eax, r.edi))
    }
    fn s_count(r: &mut Pushad, a: *const u32) -> Option<u32> {
        if r.eax == 0 {
            return Some(0);
        }
        let want = lc(&cs(arg(a, 0)));
        let mut n = 0;
        let mut c = unsafe { *((r.eax + 8) as *const u32) };
        while c != 0 {
            if lc(&cs(unsafe { *(c as *const u32) })) == want {
                n += 1;
            }
            c = unsafe { *((c + 4) as *const u32) };
        }
        Some(n)
    }
    fn s_stricmp(_r: &mut Pushad, a: *const u32) -> Option<u32> {
        let (x, y) = (lc(&cs(arg(a, 0))), lc(&cs(arg(a, 1))));
        Some(match x.cmp(&y) {
            std::cmp::Ordering::Less => 0xFFFF_FFFF,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        })
    }
    /// The getters 0xB747D0 / 0xB74EF0 (parse even when the child is missing: the buffer then holds the string
    /// the previous getter parsed) and 0xB74860 / 0xB74F60 (AL = found).
    fn getter(r: &mut Pushad, a: *const u32, float: bool, ok_variant: bool) -> Option<u32> {
        let node = arg(a, 0);
        let v = if node == 0 { 0 } else { value(node, r.ecx) };
        let mut s = v;
        if v != 0 {
            STALE.with(|x| x.set(v));
        } else if ok_variant {
            return Some(srng() & !0xFF);
        } else {
            s = STALE.with(|x| x.get());
            GARBAGE_CALLS.with(|g| g.set(g.get() + 1));
            if let Some(e) = EXPECT.with(|e| e.take()) {
                if e != s {
                    BAD.with(|b| b.set(b.get() + 1));
                }
            }
        }
        EXPECT.with(|e| e.set(None));
        let bits = if float { parse_f(&cs(s)).to_bits() } else { parse_i(&cs(s)) };
        unsafe { put_dword(r.esi, bits) };
        Some(if ok_variant { (srng() & !0xFF) | 1 } else { r.eax })
    }
    fn s_get_int(r: &mut Pushad, a: *const u32) -> Option<u32> {
        getter(r, a, false, false)
    }
    fn s_get_float(r: &mut Pushad, a: *const u32) -> Option<u32> {
        getter(r, a, true, false)
    }
    fn s_get_int_ok(r: &mut Pushad, a: *const u32) -> Option<u32> {
        getter(r, a, false, true)
    }
    fn s_get_float_ok(r: &mut Pushad, a: *const u32) -> Option<u32> {
        getter(r, a, true, true)
    }
    fn s_sscanf(_r: &mut Pushad, a: *const u32) -> Option<u32> {
        let s = arg(a, 0);
        if s == 0 {
            return Some(0xFFFF_FFFF);
        }
        let fmt = cs(arg(a, 1));
        let toks: Vec<String> = String::from_utf8_lossy(&cs(s)).split_whitespace().map(|x| x.to_string()).collect();
        let n = fmt.iter().filter(|&&c| c == b'%').count();
        let mut done = 0;
        for i in 0..n {
            if i >= toks.len() {
                break;
            }
            let t = &toks[i];
            if t.parse::<f64>().is_err() {
                break;
            }
            let f = t.parse::<f64>().unwrap() as f32;
            unsafe { put_dword(arg(a, 2 + i), f.to_bits()) };
            done += 1;
        }
        Some(done)
    }
    fn s_strncpy(_r: &mut Pushad, a: *const u32) -> Option<u32> {
        let (d, s, n) = (arg(a, 0), arg(a, 1), arg(a, 2) as usize);
        let src = cs(s);
        for i in 0..n.min(0x400) {
            let b = if i < src.len() { src[i] } else { 0 };
            unsafe { *((d as usize + i) as *mut u8) = b };
        }
        Some(d)
    }
    fn s_quat(r: &mut Pushad, _a: *const u32) -> Option<u32> {
        let q: Vec<f32> = (0..4).map(|i| unsafe { *((r.eax as usize + i * 4) as *const f32) }).collect();
        for i in 0..9usize {
            unsafe { *((r.ecx as usize + i * 4) as *mut f32) = q[i % 4] * 0.5 + (i as f32) * 0.125 };
        }
        Some(r.eax)
    }
    fn s_vmul(r: &mut Pushad, a: *const u32) -> Option<u32> {
        let v = arg(a, 0);
        let m: Vec<f32> = (0..9).map(|i| unsafe { *((r.ecx as usize + i * 4) as *const f32) }).collect();
        let x: Vec<f32> = (0..3).map(|i| unsafe { *((v as usize + i * 4) as *const f32) }).collect();
        for i in 0..3usize {
            let s = m[i * 3] * x[0] + m[i * 3 + 1] * x[1] + m[i * 3 + 2] * x[2];
            unsafe { *((r.eax as usize + i * 4) as *mut f32) = s };
        }
        Some(r.eax)
    }
    fn s_alloc(_r: &mut Pushad, a: *const u32) -> Option<u32> {
        let size = arg(a, 0) as usize;
        unsafe {
            let cur = *(HEAP as *const u32) as usize;
            let p = (cur + 15) & !15;
            assert!(p + size <= ARENA + ARENA_LEN, "heap exhausted");
            *(HEAP as *mut u32) = (p + size) as u32;
            Some(p as u32)
        }
    }
    fn s_mark(_r: &mut Pushad, _a: *const u32) -> Option<u32> {
        Some(srng())
    }
    fn s_end_scope(r: &mut Pushad, _a: *const u32) -> Option<u32> {
        Some(r.eax)
    }
    fn reset_state() {
        STALE.with(|x| x.set(0));
        EXPECT.with(|e| e.set(None));
    }

    fn mk(target: usize, nargs: usize, pop: usize, reads: u8, f: fn(&mut Pushad, *const u32) -> Option<u32>) -> Callee {
        let mut c = callee(target, nargs, pop, reads, Ret::Any, Fx::None);
        c.func = Some(f);
        c
    }

    fn callees() -> Vec<Callee> {
        let mut sscanf = mk(F_SSCANF, 0, 0, 0, s_sscanf);
        sscanf.scanf = true;
        let mut cookie = callee(0xC9604F, 0, 0, 0, Ret::Keep, Fx::None);
        cookie.silent = true;
        vec![
            mk(F_PARSE_SCRIPT, 1, 0, EAX, s_script),
            mk(F_HASH, 1, 4, EAX | ECX, s_hash),
            mk(F_ROOT, 0, 0, EAX | ECX, s_root),
            mk(F_COUNT, 1, 0, EAX, s_count),
            mk(F_FIND, 0, 0, EAX | EDI, s_find),
            mk(F_STRICMP, 2, 0, 0, s_stricmp),
            mk(F_GET_INT, 1, 0, ECX | ESI, s_get_int),
            mk(F_GET_FLOAT, 1, 0, ECX | ESI, s_get_float),
            mk(F_GET_FLOAT_OK, 1, 0, ECX | ESI, s_get_float_ok),
            mk(F_GET_INT_OK, 1, 0, ECX | ESI, s_get_int_ok),
            sscanf,
            mk(F_HASH_SLATE, 1, 4, EAX | EDX, s_hash_slate),
            mk(F_STRNCPY, 3, 0, 0, s_strncpy),
            mk(F_QUAT, 0, 0, EAX | ECX, s_quat),
            mk(F_VMUL, 1, 4, EAX | ECX, s_vmul),
            cookie,
            mk(V + 0x1C, 2, 8, ECX, s_alloc),
            mk(V + 0x30, 0, 0, ECX, s_mark),
            mk(V + 0x34, 1, 4, ECX, s_end_scope),
        ]
    }

    // --- the world --------------------------------------------------------------------------------------

    struct B<'a> {
        a: &'a mut Arena,
        rng: &'a mut Rng,
    }
    impl B<'_> {
        fn s(&mut self, t: &str) -> u32 {
            let p = self.a.pool(1, t.len() + 1)[0];
            unsafe {
                for (i, b) in t.bytes().enumerate() {
                    putb(p, i, b);
                }
                putb(p, t.len(), 0);
            }
            p
        }
        fn cased(&mut self, n: &str) -> String {
            match self.rng.below(6) {
                0 => n.to_ascii_uppercase(),
                1 => n.to_ascii_lowercase(),
                _ => n.to_string(),
            }
        }
        fn node(&mut self, name: &str, val: Option<&str>, kids: &[u32]) -> u32 {
            let n = self.a.pool(1, 16)[0];
            let nm = self.cased(name);
            let nm = self.s(&nm);
            let v = match val {
                Some(t) => self.s(t),
                None => 0,
            };
            unsafe {
                put(n, 0, nm);
                put(n, 4, 0);
                put(n, 8, kids.first().copied().unwrap_or(0));
                put(n, 0xC, v);
                for i in 0..kids.len() {
                    put(kids[i], 4, if i + 1 < kids.len() { kids[i + 1] } else { 0 });
                }
            }
            n
        }
        fn shuffle(&mut self, v: &mut Vec<u32>) {
            for i in (1..v.len()).rev() {
                let j = self.rng.below(i as u32 + 1) as usize;
                v.swap(i, j);
            }
        }
        fn fl(&mut self, scale: f32) -> String {
            match self.rng.below(12) {
                0 => "0".to_string(),
                1 => "1".to_string(),
                2 => format!("{:.2}", scale),
                _ => format!("{:.3}", (self.rng.below(2001) as f32 - 1000.0) / 1000.0 * scale),
            }
        }
        fn vec3(&mut self, scale: f32) -> String {
            format!("{} {} {}", self.fl(scale), self.fl(scale), self.fl(scale))
        }
        fn opt(&mut self, kids: &mut Vec<u32>, name: &str, val: String, one_in_missing: u32) {
            if !self.rng.one_in(one_in_missing) {
                let n = self.node(name, Some(&val), &[]);
                kids.push(n);
            }
        }
        fn noise(&mut self, kids: &mut Vec<u32>) {
            for _ in 0..self.rng.below(3) {
                let nm = if self.rng.one_in(2) { "Extra" } else { "Lights2" };
                let n = self.node(nm, Some("x"), &[]);
                kids.push(n);
            }
        }
        fn light(&mut self, i: u32) -> u32 {
            let mut k = Vec::new();
            let nm = format!("lamp{i}_{}", self.rng.below(100));
            let nn = self.node("Name", Some(&nm), &[]);
            k.push(nn);
            let ty = ["omni", "circular spotlight", "Omni", "OMNI", "spot", "Circular Spotlight", "directional"][self.rng.below(7) as usize];
            self.opt(&mut k, "Type", ty.to_string(), 30);
            let col = format!("{} {} {}", self.fl(1.5), self.fl(1.5), self.fl(1.5));
            self.opt(&mut k, "Color", col, 30);
            let mult = ["1.5", "0.25", "2", "3.75", "0.8", "10", "1", "0", "-1", "100.5"][self.rng.below(10) as usize];
            self.opt(&mut k, "Multiplier", mult.to_string(), 6);
            for n in ["LightCharacter", "ShadowCharacter", "LightLevel", "ShadowLevel"] {
                let v = ["0", "1", "2", "-1", "1"][self.rng.below(5) as usize];
                self.opt(&mut k, n, v.to_string(), 2);
            }
            let p = self.vec3(100.0);
            self.opt(&mut k, "Position", p, 20);
            let o = format!("{} {} {} {}", self.fl(1.0), self.fl(1.0), self.fl(1.0), self.fl(1.0));
            self.opt(&mut k, "Orientation", o, 20);
            let h = format!("{} {}", self.fl(60.0), self.fl(90.0));
            self.opt(&mut k, "Hotspot", h, 20);
            let at = format!("{} {}", self.fl(20.0), self.fl(80.0));
            self.opt(&mut k, "Attenuation", at, 20);
            let sl = if self.rng.one_in(3) { "".to_string() } else { format!("slate_{}", self.rng.below(5)) };
            self.opt(&mut k, "Slate_Name", sl, 2);
            self.noise(&mut k);
            self.shuffle(&mut k);
            self.node("Light", None, &k)
        }
        fn grid(&mut self, i: u32) -> u32 {
            let mut k = Vec::new();
            let sl = format!("g{i}_{}", self.rng.below(7));
            self.opt(&mut k, "Slate_Name", sl, 4);
            let c = self.vec3(1.0);
            self.opt(&mut k, "Level_Ambient_Color", c, 12);
            let c = self.vec3(1.0);
            self.opt(&mut k, "Char_Ambient_Color", c, 12);
            for n in ["Cutscene_Frame_Start", "Cutscene_Frame_End", "Cutscene_Shot_Start", "Cutscene_Shot_End"] {
                let v = ["0", "10", "-3", "250"][self.rng.below(4) as usize];
                self.opt(&mut k, n, v.to_string(), 3);
            }
            let f = self.fl(2.0);
            self.opt(&mut k, "Light_Map_DynRange", f, 3);
            let c = self.vec3(2.0);
            self.opt(&mut k, "Main_Light_Color", c, 3);
            let c = self.vec3(2.0);
            self.opt(&mut k, "Light_Back_Color", c, 3);
            for n in ["Shadow_Hardness", "Light_Amplication", "Contrast", "Bleach", "Brightness", "Saturation"] {
                let f = self.fl(2.0);
                self.opt(&mut k, n, f, 3);
            }
            let c = self.vec3(1.0);
            self.opt(&mut k, "Tint_Color", c, 3);
            if !self.rng.one_in(3) {
                let mut bk = Vec::new();
                for n in ["Radius", "Cutoff", "Amp"] {
                    let f = self.fl(5.0);
                    self.opt(&mut bk, n, f, 3);
                }
                let b = self.node("Bloom", None, &bk);
                k.push(b);
            }
            self.noise(&mut k);
            self.shuffle(&mut k);
            self.node("Ambient_Grid", None, &k)
        }
        fn lightset(&mut self, i: u32) -> u32 {
            let mut k = Vec::new();
            let nm = format!("set{i}");
            self.opt(&mut k, "Name", nm, 10);
            let t = format!("{}", self.rng.below(24));
            self.opt(&mut k, "StartTime", t, 5);
            let t = format!("{}", self.rng.below(24));
            self.opt(&mut k, "EndTime", t, 5);
            if !self.rng.one_in(6) {
                let mut lk = Vec::new();
                for j in 0..self.rng.below(5) {
                    let l = self.light(j);
                    lk.push(l);
                    if self.rng.one_in(3) {
                        let n = self.node("Other", None, &[]);
                        lk.push(n);
                    }
                }
                let l = self.node("Lights", None, &lk);
                k.push(l);
            }
            if !self.rng.one_in(4) {
                let mut gk = Vec::new();
                for j in 0..self.rng.below(4) {
                    let g = self.grid(j);
                    gk.push(g);
                    if self.rng.one_in(3) {
                        let n = self.node("Other", None, &[]);
                        gk.push(n);
                    }
                }
                let g = self.node("Ambient_Grids", None, &gk);
                k.push(g);
            }
            self.noise(&mut k);
            self.shuffle(&mut k);
            self.node("LightSet", None, &k)
        }
        fn root(&mut self) -> u32 {
            let mut k = Vec::new();
            for i in 0..self.rng.below(5) {
                let s = self.lightset(i);
                k.push(s);
                if self.rng.one_in(3) {
                    let n = self.node("Other", None, &[]);
                    k.push(n);
                }
            }
            self.noise(&mut k);
            self.node("Root", None, &k)
        }
    }

    const STRINGS: &[(usize, &str)] = &[
        (S_LIGHTSET, "LightSet"),
        (S_STARTTIME, "StartTime"),
        (S_ENDTIME, "EndTime"),
        (S_LIGHTS, "Lights"),
        (S_LIGHT, "Light"),
        (S_OMNI, "omni"),
        (S_SPOT, "circular spotlight"),
        (S_COLOR, "Color"),
        (S_FMT3, "%f %f %f"),
        (S_MULTIPLIER, "Multiplier"),
        (S_LIGHTCHARACTER, "LightCharacter"),
        (S_SHADOWCHARACTER, "ShadowCharacter"),
        (S_LIGHTLEVEL, "LightLevel"),
        (S_SHADOWLEVEL, "ShadowLevel"),
        (S_POSITION, "Position"),
        (S_ORIENTATION, "Orientation"),
        (S_FMT4, "%f %f %f %f"),
        (S_HOTSPOT, "Hotspot"),
        (S_FMT2, "%f %f"),
        (S_ATTENUATION, "Attenuation"),
        (S_SLATE, "Slate_Name"),
        (S_AGRIDS, "Ambient_Grids"),
        (S_AGRID, "Ambient_Grid"),
        (S_LEVEL_AMB, "Level_Ambient_Color"),
        (S_CHAR_AMB, "Char_Ambient_Color"),
        (S_CS_FRAME_START, "Cutscene_Frame_Start"),
        (S_CS_FRAME_END, "Cutscene_Frame_End"),
        (S_CS_SHOT_START, "Cutscene_Shot_Start"),
        (S_CS_SHOT_END, "Cutscene_Shot_End"),
        (S_DYNRANGE, "Light_Map_DynRange"),
        (S_MAIN_LIGHT, "Main_Light_Color"),
        (S_BACK_LIGHT, "Light_Back_Color"),
        (S_SHADOW_HARD, "Shadow_Hardness"),
        (S_LIGHT_AMP, "Light_Amplication"),
        (S_BLEACH, "Bleach"),
        (S_TINT, "Tint_Color"),
        (S_BLOOM, "Bloom"),
        (S_CUTOFF, "Cutoff"),
        (S_AMP, "Amp"),
        (S_CONTRAST, "Contrast"),
        (S_BRIGHTNESS, "Brightness"),
        (S_SATURATION, "Saturation"),
        (S_RADIUS, "Radius"),
        (S_NAME, "Name"),
        (S_TYPE, "Type"),
    ];

    fn g(addr: usize, v: u32) {
        unsafe { wd(addr, v) }
    }

    fn run(name: &str, h: &Harness, mut world: impl FnMut(&Harness, &mut Rng) -> [u32; 3]) {
        let mut rng = Rng(0x1234_5678_9ABC_DEF1 ^ name.len() as u64 * 0x9E37_79B9);
        let n = cases();
        let mut bad = 0;
        for i in 0..n {
            let args = world(h, &mut rng);
            if let Err(e) = h.check(args, rng.u32() as u64) {
                bad += 1;
                if bad <= 5 {
                    eprintln!("{name} case {i}: regs {:x?} args {args:x?}: {e}", h.input.get());
                }
            }
        }
        eprintln!("{name}: {n} cases, {bad} differ");
        assert_eq!(bad, 0, "{bad} of {n} cases differ");
    }

    #[test]
    #[ignore = "needs the test exe linked at /BASE:0x10000000 (see the comment above)"]
    fn tod_lights_matches_original() {
        let _g = lock();
        let mut h = Harness::new(TOD, &TOD_BODY, &TOD_CALLS, &TOD_DISPS, callees(), &[(0x0252_6000, 0x1000), (0x0265_2000, 0x2000), (0x0277_0000, 0x2000), (0x00E8_7000, 0x1000), (0x033D_6000, 0x1000)], tod_thunk as *const () as usize);
        h.cmp_eax = true;
        h.reset = Some(reset_state);
        BAD.with(|b| b.set(0));
        GARBAGE_CALLS.with(|b| b.set(0));
        run("tod_lights", &h, |h, rng| unsafe {
            h.mem.fill_random(rng);
            for &(a, s) in STRINGS {
                for (i, b) in s.bytes().enumerate() {
                    wb(a + i, b);
                }
                wb(a + s.len(), 0);
            }
            g(C_DEFAULT, (-1.0f32).to_bits());
            g(DEFAULT_SLATE, rng.u32());
            let mut a = Arena::new();
            put(ARENA as u32, 0x10_0000, ARENA as u32 + 0x10_0010);
            // pool objects and their vtable
            let vt = a.pool(1, 0x40)[0];
            put(vt, 0x1C, redirect(V + 0x1C) as u32);
            put(vt, 0x30, redirect(V + 0x30) as u32);
            put(vt, 0x34, redirect(V + 0x34) as u32);
            let pool = a.pool(1, 0x20)[0];
            put(pool, 0, vt);
            g(DEFAULT_POOL, vt);
            ROOT_POOL.with(|p| p.set(super::tr(0x0277_0FE0) as u32));
            wd(0x0277_0FE0, vt);
            // the cache
            let cnt = rng.pick(&[0u32, 0, 1, 2, 5, 17, 0x7E, 0x7F, 0x80]);
            g(CACHE_COUNT, cnt);
            // the script / root
            let mut b = B { a: &mut a, rng };
            let want = ["night_lights", "store_clothes", "cs_lights_01", "Pcu_Store"][b.rng.below(4) as usize];
            let want = b.cased(want);
            let name_p = b.s(&want);
            let other = b.s("something_else");
            let root = b.root();
            let root2 = b.root();
            ROOTS.with(|r| *r.borrow_mut() = vec![(other, root2), (name_p, root)]);
            SCRIPT_OK.with(|s| s.set(!b.rng.one_in(15)));
            if b.rng.one_in(20) {
                // the file exists but has no root
                ROOTS.with(|r| *r.borrow_mut() = vec![(other, root2)]);
            }
            let pool_in = if rng.one_in(3) { 0 } else { pool };
            h.input.set(regs(rng.u32(), name_p, pool_in, rng.u32(), rng.u32()));
            [0; 3]
        });
        eprintln!("getter garbage-path calls {}, model mismatches {}", GARBAGE_CALLS.with(|b| b.get()), BAD.with(|b| b.get()));
        assert_eq!(BAD.with(|b| b.get()), 0, "the port's belief about the getter buffer differs from the model");
        eprintln!("{}", summary());
    }

    fn regs(eax: u32, ecx: u32, edx: u32, esi: u32, edi: u32) -> Regs {
        Regs { eax, ecx, edx, ebx: 0xB1B1_B1B1, esi, edi, ebp: 0xE9E9_E9E9 }
    }
}
