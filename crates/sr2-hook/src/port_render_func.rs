//! Port module: `game_render_func` (0x5250E0) as a whole-function replacement, plus its two leaves
//! `tod_occlusion_allowed` (0x4B4A60) and `human_camera_get` (0x9DFA00). Doc 18, "game_render_func
//! step-by-step port map", every detail rechecked at the disassembly.
//!
//! `game_render_func` runs on the **game thread** only (from `frame_end_handoff`, after
//! `render_lists_reset`). It is a skeleton: every step that is not trivial stays a call to the
//! original callee with the registers and stack arguments the original sets up. The render
//! thread reads what it leaves behind and is synchronised through the job object `[0xFA6BD8]`
//! (kick `vtbl+4` / sync `vtbl+0xC` after the effects release and after the second occluder
//! pass, wait `vtbl+0x10` and release at the end), so those pairs are kept exactly.
//!
//! Dropped from the original: the two 32-byte job descriptors on the stack (0x5250C0 is a
//! `jmp 0x511050`, which reads no stack argument; 0x5250D0 reads only its 2nd argument,
//! 0x2612D10) and the two `GetCurrentThreadId` calls whose results only fill the descriptors.
//!
//! The leaves have many callers, so they go through `pushad` thunks (all registers come back as
//! the caller left them, EAX carries the result), and for the first `SHADOW_CALLS` calls a
//! relocated copy of the original runs on the same registers and its results are compared; on a
//! mismatch the original's are kept.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::portlib::*;
use crate::{detour, log};

const SHADOW_CALLS: u32 = 3000;

// --- callees (all called by address; some are already Rust) --------------------------------------

const JOB_ALLOC: usize = 0x00BF_3F50; // cdecl (file, line) -> job
const EFFECTS_RELEASE_JMP: usize = 0x0052_50C0; // jmp 0x511050 effects_release_dead_particles, no args
const SKY_UPDATE: usize = 0x004D_7160;
const LIGHT_COLLECTORS: [usize; 6] = [0x00AE_0FD0, 0x0050_5350, 0x0094_DFA0, 0x004C_85D0, 0x0095_56A0, 0x0096_3750];
const GLOBAL_CONSTANTS_UPDATE: usize = 0x0052_4060;
const VIEWPORT_SETUP: usize = 0x0052_A3B0;
const COOP_PARTNER: usize = 0x009D_7BD0; // -> human or 0
const GFX_THREAD_STATE: usize = 0x00D2_2790; // -> eax
const COOP_VIEW_CLIP_BUILD: usize = 0x0052_3EB0; // cdecl (&pos, &mat, f32, f32)
const OCCLUDERS_SELECT: usize = 0x0052_3BD0; // cdecl (view)
const TOD_ZONE_UPDATE: usize = 0x004B_3FF0; // EAX = &camera pos, cdecl u8
const VIEWS_BUILD: usize = 0x004B_A270; // ECX = camera matrix, cdecl (&camera pos)
const VIEW_CLIP_PLANES_BUILD: usize = 0x00A0_2550; // EAX = out, ECX = eye, cdecl (a, b, f32)

// --- globals -------------------------------------------------------------------------------------

const JOBS_ON: usize = 0x0252_A3E8; // dword
const JOB: usize = 0x00FA_6BD8; // job object; never cleared after the release
const LIGHT_COUNT: usize = 0x0252_A364;
const COOP_A: usize = 0x0252_8B48; // byte
const COOP_B: usize = 0x0252_8B49; // byte
const SPLIT_MODE: usize = 0x0216_03CC; // dword
const LOCAL_PLAYER: usize = 0x0217_03D4;
const THREAD_VIEW_COPY: usize = 0x0277_CBF0; // 0x84 dwords
const LIGHT_SCALE_ON: usize = 0x0252_7393; // byte
const LIGHT_SCALE_SRC: usize = 0x00E9_8980; // f32
const LIGHT_SCALE: usize = 0x00FA_6BDC; // f32
const VIEW_COUNT: usize = 0x0252_6CCC; // dword
const CAMERA_POS: usize = 0x025F_5B14;
const CAMERA_MATRIX: usize = 0x025F_5B38;
const NO_OCCLUSION: usize = 0x0252_73B4; // byte
const TOD_ENABLED: usize = 0x00E9_96E4; // byte
const TOD_FORCE_A: usize = 0x0252_6CC3; // byte
const TOD_FORCE_B: usize = 0x0252_6CC4; // byte
const OCCLUSION_FLAG: usize = 0x00E9_9382; // byte
const RENDER_MODE: usize = 0x00E9_8948; // dword
const VIEW_LIST: usize = 0x0261_2D10;
const OCCLUDERS_B_COUNT: usize = 0x0252_739C; // dword
const FRAME_COUNTER: usize = 0x0252_6CD0; // word
const ONE_F: usize = 0x0252_73A4; // f32 = 1.0
const TOD_SNAPSHOT_SRC: usize = 0x0261_28F8; // 0x81 dwords
const TOD_SNAPSHOT_DST: usize = 0x0261_2B00;
const VIEW0_FLAGS: usize = 0x0261_2D18;
const VIEW0_FRUSTUM: usize = 0x0261_2D20;
const VIEW0_NEAR: usize = 0x0261_2D58; // f32
const VIEW0_COPY: usize = 0x0261_5510;
const CLIP_SET: usize = 0x0277_C9E0;
const JOB_POOL: u32 = 0x00EC_5B98;
const JOB_WAIT_TAG: u32 = 0x033D_6328;

// --- counters ------------------------------------------------------------------------------------

const STEP_NAMES: [&str; 18] = [
    "job_alloc",
    "effects_release+kick",
    "sky",
    "lights_reset",
    "collectors",
    "constants",
    "viewport+copy",
    "coop_view",
    "light_scale",
    "occluders_main",
    "tod_zones",
    "occlusion_flag",
    "views_build",
    "occluders_second",
    "occluders_skipped",
    "snapshot",
    "clip_set",
    "job_wait",
];
static STEPS: [AtomicU32; 18] = [const { AtomicU32::new(0) }; 18];
static CALLS: AtomicU32 = AtomicU32::new(0);

fn step(i: usize) {
    STEPS[i].fetch_add(1, Ordering::Relaxed);
}

// --- shared helpers --------------------------------------------------------------------------------

/// A `fld dword` / `fstp dword` round trip: the bits, except that a signalling NaN is quieted.
fn q32(bits: u32) -> u32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 { bits | 0x0040_0000 } else { bits }
}

/// The calling thread's render state (TLS block +0x2C -> [0], +0x58, then +0x740).
unsafe fn thread_view() -> u32 {
    let tls: u32;
    unsafe {
        std::arch::asm!("mov {}, fs:[0x2c]", out(reg) tls, options(nostack, readonly));
        rd(rd(rd(tls as usize) as usize + 0x58) as usize + 0x740)
    }
}

/// `rep movsd`, forward, one dword at a time.
unsafe fn fwd_copy(dst: usize, src: usize, n: usize) {
    for i in 0..n {
        unsafe { wd(dst + i * 4, rd(src + i * 4)) };
    }
}

// --- registers (for the shadow checks of the two leaves) ------------------------------------------

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
/// EBX / ESI / EDI / EBP: callee-saved in both originals.
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

/// Call `f` with all seven registers from `r` and one stack argument, then capture all seven
/// registers into `r`.
unsafe fn call_full(f: usize, r: &mut Regs, arg: u32) {
    let mut b = Block { f: f as u32, r: *r, arg, esp: 0, st0: 0.0, want_st0: 0 };
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
}

/// A naked thunk: `pushad`, then `f(&mut block, &stack args)`, then `popad`; `ret`.
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

struct Leaf {
    name: &'static str,
    /// Registers the original leaves unchanged on every path (checked in the shadow).
    preserved: u8,
    calls: AtomicU32,
    mismatches: AtomicU32,
    original: AtomicUsize,
}

const fn leaf(name: &'static str, preserved: u8) -> Leaf {
    Leaf { name, preserved, calls: AtomicU32::new(0), mismatches: AtomicU32::new(0), original: AtomicUsize::new(0) }
}

const TOD: usize = 0;
const CAM: usize = 1;
static LEAVES: [Leaf; 2] = [
    leaf("tod_occlusion_allowed", ECX | EDX | SAVED),
    leaf("human_camera_get", EAX | SAVED),
];

/// Count a call; `Some((call number, original copy))` while it should be shadowed.
fn shadowing(id: usize) -> Option<(u32, usize)> {
    let p = &LEAVES[id];
    let n = p.calls.fetch_add(1, Ordering::Relaxed);
    let orig = p.original.load(Ordering::Relaxed);
    if n == SHADOW_CALLS && orig != 0 {
        log(&format!("port_render_func: {}: {SHADOW_CALLS} calls shadowed, {} mismatched", p.name, p.mismatches.load(Ordering::Relaxed)));
    }
    (orig != 0 && n < SHADOW_CALLS).then_some((n, orig))
}

fn mismatch(id: usize, n: u32, what: String) {
    let m = LEAVES[id].mismatches.fetch_add(1, Ordering::Relaxed) + 1;
    if m <= 10 {
        log(&format!("port_render_func: {} mismatch (call {n}): {what}", LEAVES[id].name));
    }
}

static PRESERVE_LOGS: AtomicU32 = AtomicU32::new(0);

/// Log when the original changed a register it was found to preserve (the thunks preserve all).
fn check_preserved(id: usize, inp: &Regs, out: &Regs) {
    for i in 0..7 {
        if LEAVES[id].preserved & (1 << i) != 0 && inp.get(i) != out.get(i) && PRESERVE_LOGS.fetch_add(1, Ordering::Relaxed) < 10 {
            log(&format!(
                "port_render_func: {}: the original changed {} ({:#x} -> {:#x})",
                LEAVES[id].name,
                REG_NAMES[i],
                inp.get(i),
                out.get(i)
            ));
        }
    }
}

// --- 0x4B4A60 tod_occlusion_allowed (no args -> EAX = 0 / 1) --------------------------------------

/// v1 = f32(b + (a - b) * t), v = f32((v1 - 1) * s + 1), all in 53-bit arithmetic (a = [0x26128F8],
/// b = [0x26128FC], t = [0x2612900], s = [0x2612908]). The original compares v with the constant
/// at 0xDD32C0 (0.0) by `fcomp; fnstsw; test ah, 0x41; je`: the jump (result 1) is taken when
/// neither C0 (v < c) nor C3 (v == c) is set, i.e. v > c; a NaN sets both, so it falls through to
/// the byte test. Result 1 when v > 0.0, or when the byte [0x2612904] is 0; else 0.
unsafe fn tod_occlusion_allowed() -> bool {
    unsafe {
        let a = f32::from_bits(rd(0x0261_28F8)) as f64;
        let b = f32::from_bits(rd(0x0261_28FC)) as f64;
        let t = f32::from_bits(rd(0x0261_2900)) as f64;
        let s = f32::from_bits(rd(0x0261_2908)) as f64;
        let v1 = ((a - b) * t + b) as f32;
        let v = ((v1 as f64 - 1.0) * s + 1.0) as f32;
        v > 0.0 || rb(0x0261_2904) == 0
    }
}

extern "C" fn tod_occlusion_entry(r: &mut Pushad, _: *const u32) {
    let p = *r;
    let ours = unsafe { tod_occlusion_allowed() } as u32;
    r.eax = ours;
    let Some((n, orig)) = shadowing(TOD) else { return };
    let inp = Regs::of(&p);
    let mut out = inp;
    unsafe { call_full(orig, &mut out, 0) };
    check_preserved(TOD, &inp, &out);
    if out.eax != ours {
        mismatch(TOD, n, format!("original {:#x}, Rust {ours:#x}", out.eax));
        r.eax = out.eax;
    }
}
regs_thunk!(tod_occlusion_thunk, tod_occlusion_entry);

// --- 0x9DFA00 human_camera_get (ECX = human, EAX = &pos or 0, cdecl &mat3x3 or 0) ------------------

/// The local player ([0x21703D4], a plain compare) reads the camera globals, any other human its
/// +0x12A0 position and +0x12AC matrix.
unsafe fn human_camera_get(human: u32, pos: u32, mat: u32) {
    unsafe {
        let (sp, sm) = if human == rd(LOCAL_PLAYER) {
            (CAMERA_POS, CAMERA_MATRIX)
        } else {
            (human as usize + 0x12A0, human as usize + 0x12AC)
        };
        if pos != 0 {
            fwd_copy(pos as usize, sp, 3);
        }
        if mat != 0 {
            fwd_copy(mat as usize, sm, 9);
        }
    }
}

extern "C" fn camera_get_entry(r: &mut Pushad, args: *const u32) {
    let p = *r;
    let mat = unsafe { *args };
    let shadow = shadowing(CAM);
    // Scratch outputs for the original copy, primed with what is there now.
    let mut spos = [0u32; 3];
    let mut smat = [0u32; 9];
    if let Some((_, orig)) = shadow {
        unsafe {
            if p.eax != 0 {
                for (i, w) in spos.iter_mut().enumerate() {
                    *w = rd(p.eax as usize + i * 4);
                }
            }
            if mat != 0 {
                for (i, w) in smat.iter_mut().enumerate() {
                    *w = rd(mat as usize + i * 4);
                }
            }
            let inp = Regs::of(&p);
            // EAX goes in as the scratch pointer, so its preservation is checked against that.
            let eax_in = if p.eax != 0 { spos.as_mut_ptr() as u32 } else { 0 };
            let mut out = Regs { eax: eax_in, ..inp };
            call_full(orig, &mut out, if mat != 0 { smat.as_mut_ptr() as u32 } else { 0 });
            check_preserved(CAM, &Regs { eax: eax_in, ..inp }, &out);
        }
    }
    unsafe { human_camera_get(p.ecx, p.eax, mat) };
    if let Some((n, _)) = shadow {
        unsafe {
            for i in 0..3 {
                if p.eax != 0 && rd(p.eax as usize + i * 4) != spos[i] {
                    mismatch(CAM, n, format!("human {:#x} pos[{i}]: original {:#x}, Rust {:#x}", p.ecx, spos[i], rd(p.eax as usize + i * 4)));
                    wd(p.eax as usize + i * 4, spos[i]);
                }
            }
            for i in 0..9 {
                if mat != 0 && rd(mat as usize + i * 4) != smat[i] {
                    mismatch(CAM, n, format!("human {:#x} mat[{i}]: original {:#x}, Rust {:#x}", p.ecx, smat[i], rd(mat as usize + i * 4)));
                    wd(mat as usize + i * 4, smat[i]);
                }
            }
        }
    }
}
regs_thunk!(camera_get_thunk, camera_get_entry);

// --- 0x5250E0 game_render_func (no args, no result; game thread) -----------------------------------

extern "C" fn game_render_func() {
    unsafe {
        CALLS.fetch_add(1, Ordering::Relaxed);

        // 1. The job object, when worker jobs are on. [0xFA6BD8] is left alone otherwise.
        if rd(JOBS_ON) != 0 {
            let j = call_regs(JOB_ALLOC, 0, 0, 0, 0, 0, &[0x00DE_2374, 0x7AE]);
            wd(JOB, j);
            step(0);
        }

        // 2. Effects release (0x5250C0 is a jmp to 0x511050; the descriptor was never read), then
        //    kick and sync the job.
        call(EFFECTS_RELEASE_JMP);
        let job = rd(JOB);
        if job != 0 {
            call_regs(vslot(job, 1), 0, job, 0, 0, 0, &[]);
            call_regs(vslot(job, 3), 0, job, 0, 0, 0, &[]);
        }
        step(1);

        // 3. Sky, light count reset, the six light collectors in order, shader constants.
        call(SKY_UPDATE);
        step(2);
        wd(LIGHT_COUNT, 0);
        step(3);
        calls(&LIGHT_COLLECTORS);
        step(4);
        call(GLOBAL_CONSTANTS_UPDATE);
        step(5);

        // 4. Viewport setup, then the thread view's 0x84 dwords to 0x277CBF0.
        call(VIEWPORT_SETUP);
        let view = thread_view();
        fwd_copy(THREAD_VIEW_COPY, view as usize, 0x84);
        step(6);

        // 5. Co-op second view: both bytes exactly 1, split mode 2, a partner exists.
        if rb(COOP_A) == 1 && rb(COOP_B) == 1 && rd(SPLIT_MODE) == 2 {
            let partner = call_regs(COOP_PARTNER, 0, 0, 0, 0, 0, &[]);
            if partner != 0 {
                let mut pos = [0u32; 3];
                let mut mat = [0u32; 9];
                human_camera_get(partner, pos.as_mut_ptr() as u32, mat.as_mut_ptr() as u32);
                // Two floats through fld/fstp dword: the thread state's +0x1710 / +0x1714 for the
                // local player, else the partner's +0x12D0 / +0x12D4.
                let (a, b) = if partner == rd(LOCAL_PLAYER) {
                    let st = call_regs(GFX_THREAD_STATE, 0, 0, 0, 0, 0, &[]) as usize;
                    (rd(st + 0x1710), rd(st + 0x1714))
                } else {
                    (rd(partner as usize + 0x12D0), rd(partner as usize + 0x12D4))
                };
                call_regs(COOP_VIEW_CLIP_BUILD, 0, 0, 0, 0, 0, &[pos.as_ptr() as u32, mat.as_ptr() as u32, q32(a), q32(b)]);
            }
            step(7);
        }

        // 6. Light scale and the view count reset.
        let scale = if rb(LIGHT_SCALE_ON) != 0 { q32(rd(LIGHT_SCALE_SRC)) } else { 1.0f32.to_bits() };
        wd(LIGHT_SCALE, scale);
        wd(VIEW_COUNT, 0);
        step(8);

        // 7. Main occluders (view 0).
        call_regs(OCCLUDERS_SELECT, 0, 0, 0, 0, 0, &[0]);
        step(9);

        // 8. Lighting zones: EAX = camera position, cdecl 1. DL = [0x25273B4] is read after the call.
        call_regs(TOD_ZONE_UPDATE, CAMERA_POS as u32, 0, 0, 0, 0, &[1]);
        let no_occlusion = rb(NO_OCCLUSION);
        step(10);

        // 9. The occlusion flag [0xE99382]: 0 unless [0xE996E4] is set and DL is 0; then
        //    [0x2526CC4] when [0x2526CC3] == 1, else tod_occlusion_allowed() (AL).
        let flag = if rb(TOD_ENABLED) != 0 && no_occlusion == 0 {
            if rb(TOD_FORCE_A) == 1 { rb(TOD_FORCE_B) } else { tod_occlusion_allowed() as u8 }
        } else {
            0
        };
        wb(OCCLUSION_FLAG, flag);
        step(11);

        // 10. Extra views, unless occlusion is off; render mode 1 or 2.
        if no_occlusion == 0 {
            let mode = rd(RENDER_MODE);
            if mode == 1 || mode == 2 {
                call_regs(VIEWS_BUILD, 0, CAMERA_MATRIX as u32, 0, 0, 0, &[CAMERA_POS as u32]);
                step(12);
            }
        }

        // 11. Second occluder pass (0x5250D0 = 0x523BD0(0x2612D10)), then kick and sync; or clear
        //     the second occluder count. The flag is re-read from memory; the job before the call.
        if rb(OCCLUSION_FLAG) != 0 {
            let job = rd(JOB);
            call_regs(OCCLUDERS_SELECT, 0, 0, 0, 0, 0, &[VIEW_LIST as u32]);
            if job != 0 {
                call_regs(vslot(job, 1), 0, job, 0, 0, 0, &[]);
                call_regs(vslot(job, 3), 0, job, 0, 0, 0, &[]);
            }
            step(13);
        } else {
            wd(OCCLUDERS_B_COUNT, 0);
            step(14);
        }

        // 12. Frame counter (word), 1.0, and the time-of-day snapshot. The view count is compared
        //     here, before the copy; the compare is signed.
        let views = rd(VIEW_COUNT) as i32;
        *(FRAME_COUNTER as *mut u16) = (*(FRAME_COUNTER as *const u16)).wrapping_add(1);
        wf(ONE_F, 1.0);
        fwd_copy(TOD_SNAPSHOT_DST, TOD_SNAPSHOT_SRC, 0x81);
        step(15);

        // 13. View-0 clip set: views > 0 (signed) and view flag 0x2000.
        if views > 0 && rd(VIEW0_FLAGS) & 0x2000 != 0 {
            call_regs(
                VIEW_CLIP_PLANES_BUILD,
                CLIP_SET as u32,
                CAMERA_POS as u32,
                0,
                0,
                0,
                &[VIEW0_COPY as u32, VIEW0_FRUSTUM as u32, q32(rd(VIEW0_NEAR))],
            );
            step(16);
        }

        // 14. Wait for the job and release it to the pool (the global keeps the stale pointer).
        if rd(JOBS_ON) != 0 {
            let job = rd(JOB);
            if job != 0 {
                let j = job as usize;
                wd(j + 0x34, JOB_WAIT_TAG);
                call_regs(vslot(job, 4), 0, job, 0, 0, 0, &[]);
                wd(j + 4, 0);
                wb(j + 8, 0);
                wd(j + 0x34, 0);
                wb(j + 9, 0);
                wd(j + 0x3C, 0);
                wd(j + 0x40, 0);
                wd(j + 0x44, 0);
                call_regs(vslot(JOB_POOL, 1), 0, JOB_POOL, 0, 0, 0, &[job - 8]);
                step(17);
            }
        }
    }
}

// --- install and summary ---------------------------------------------------------------------------

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {
    unsafe {
        // Shadow copies first: replacing patches the entry.
        if let Ok(c) = detour::relocate(0x004B_4A60, &TOD_OCC_BODY, &TOD_OCC_BODY_CALLS) {
            LEAVES[TOD].original.store(c, Ordering::Relaxed);
        }
        if let Ok(c) = detour::relocate(0x009D_FA00, &CAM_GET_BODY, &CAM_GET_BODY_CALLS) {
            LEAVES[CAM].original.store(c, Ordering::Relaxed);
        }
    }
    let mut ok = 0;
    if replace_pinned("tod_occlusion_allowed", 0x004B_4A60, &TOD_OCC_BODY, tod_occlusion_thunk as extern "C" fn() as usize) {
        ok += 1;
    }
    if replace_pinned("human_camera_get", 0x009D_FA00, &CAM_GET_BODY, camera_get_thunk as extern "C" fn() as usize) {
        ok += 1;
    }
    if replace_pinned("game_render_func", 0x0052_50E0, &RENDER_FUNC_BODY, game_render_func as extern "C" fn() as usize) {
        ok += 1;
    }
    log(&format!("port_render_func: replaced {ok}/3 with Rust"));
}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    let mut parts: Vec<String> = Vec::new();
    let c = CALLS.load(Ordering::Relaxed);
    if c > 0 {
        let steps: Vec<String> = STEP_NAMES
            .iter()
            .zip(STEPS.iter())
            .filter_map(|(n, s)| {
                let v = s.load(Ordering::Relaxed);
                (v > 0).then(|| format!("{n} {v}"))
            })
            .collect();
        parts.push(format!("game_render_func {c} [{}]", steps.join(", ")));
    }
    for l in &LEAVES {
        let n = l.calls.load(Ordering::Relaxed);
        if n > 0 {
            parts.push(format!("{} {n} ({} mismatched)", l.name, l.mismatches.load(Ordering::Relaxed)));
        }
    }
    if parts.is_empty() { String::new() } else { format!("port_render_func: {}", parts.join(", ")) }
}

// --- original bodies (tools/body_bytes.py) ---------------------------------------------------------

const TOD_OCC_BODY: [u8; 86] = [
    0x51, 0xD9, 0x05, 0xF8, 0x28, 0x61, 0x02, 0xD9, 0x05, 0xFC, 0x28, 0x61, 0x02, 0xD9, 0xC0, 0xDE,
    0xEA, 0xD9, 0x05, 0x00, 0x29, 0x61, 0x02, 0xDE, 0xCA, 0xDE, 0xC1, 0xD9, 0x1C, 0x24, 0xD9, 0x04,
    0x24, 0xD9, 0xE8, 0xDC, 0xE9, 0xD9, 0x05, 0x08, 0x29, 0x61, 0x02, 0xDE, 0xCA, 0xDE, 0xC1, 0xD9,
    0x1C, 0x24, 0xD9, 0x04, 0x24, 0xD8, 0x1D, 0xC0, 0x32, 0xDD, 0x00, 0xDF, 0xE0, 0xF6, 0xC4, 0x41,
    0x74, 0x0D, 0x80, 0x3D, 0x04, 0x29, 0x61, 0x02, 0x00, 0x74, 0x04, 0x33, 0xC0, 0x59, 0xC3, 0xB8,
    0x01, 0x00, 0x00, 0x00, 0x59, 0xC3,
];
const TOD_OCC_BODY_CALLS: [usize; 0] = [];
const CAM_GET_BODY: [u8; 113] = [
    0x3B, 0x0D, 0xD4, 0x03, 0x17, 0x02, 0x56, 0x57, 0x8B, 0x7C, 0x24, 0x0C, 0x74, 0x32, 0x85, 0xC0,
    0x74, 0x1A, 0x8B, 0x91, 0xA0, 0x12, 0x00, 0x00, 0x89, 0x10, 0x8B, 0x91, 0xA4, 0x12, 0x00, 0x00,
    0x89, 0x50, 0x04, 0x8B, 0x91, 0xA8, 0x12, 0x00, 0x00, 0x89, 0x50, 0x08, 0x85, 0xFF, 0x74, 0x3E,
    0x8D, 0xB1, 0xAC, 0x12, 0x00, 0x00, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0x5F, 0x5E, 0xC3,
    0x85, 0xC0, 0x74, 0x1A, 0x8B, 0x0D, 0x14, 0x5B, 0x5F, 0x02, 0x89, 0x08, 0x8B, 0x15, 0x18, 0x5B,
    0x5F, 0x02, 0x89, 0x50, 0x04, 0x8B, 0x0D, 0x1C, 0x5B, 0x5F, 0x02, 0x89, 0x48, 0x08, 0x85, 0xFF,
    0x74, 0x0C, 0xBE, 0x38, 0x5B, 0x5F, 0x02, 0xB9, 0x09, 0x00, 0x00, 0x00, 0xF3, 0xA5, 0x5F, 0x5E,
    0xC3,
];
const CAM_GET_BODY_CALLS: [usize; 0] = [];
const RENDER_FUNC_BODY: [u8; 783] = [
    0x83, 0xEC, 0x7C, 0x53, 0x55, 0x33, 0xDB, 0x39, 0x1D, 0xE8, 0xA3, 0x52, 0x02, 0x56, 0x57, 0x74,
    0x17, 0x68, 0xAE, 0x07, 0x00, 0x00, 0x68, 0x74, 0x23, 0xDE, 0x00, 0xE8, 0x50, 0xEE, 0x6C, 0x00,
    0x83, 0xC4, 0x08, 0xA3, 0xD8, 0x6B, 0xFA, 0x00, 0x8B, 0x2D, 0x80, 0x80, 0xDB, 0x00, 0x89, 0x5C,
    0x24, 0x28, 0x89, 0x5C, 0x24, 0x40, 0xFF, 0xD5, 0x8B, 0x4C, 0x24, 0x2C, 0x8B, 0x35, 0xD8, 0x6B,
    0xFA, 0x00, 0x89, 0x44, 0x24, 0x3C, 0x8B, 0x44, 0x24, 0x30, 0x50, 0x51, 0x8D, 0x54, 0x24, 0x2C,
    0x52, 0xC7, 0x44, 0x24, 0x30, 0xC0, 0x50, 0x52, 0x00, 0xC7, 0x44, 0x24, 0x40, 0x06, 0x00, 0x00,
    0x00, 0xE8, 0x7A, 0xFF, 0xFF, 0xFF, 0x83, 0xC4, 0x0C, 0x3B, 0xF3, 0x74, 0x16, 0x89, 0x74, 0x24,
    0x28, 0x8B, 0x06, 0x8B, 0x50, 0x04, 0x8B, 0xCE, 0xFF, 0xD2, 0x8B, 0x06, 0x8B, 0x50, 0x0C, 0x8B,
    0xCE, 0xFF, 0xD2, 0xE8, 0xF8, 0x1F, 0xFB, 0xFF, 0x89, 0x1D, 0x64, 0xA3, 0x52, 0x02, 0xE8, 0x5D,
    0xBE, 0x5B, 0x00, 0xE8, 0xD8, 0x01, 0xFE, 0xFF, 0xE8, 0x23, 0x8E, 0x42, 0x00, 0xE8, 0x4E, 0x34,
    0xFA, 0xFF, 0xE8, 0x19, 0x05, 0x43, 0x00, 0xE8, 0xC4, 0xE5, 0x43, 0x00, 0xE8, 0xCF, 0xEE, 0xFF,
    0xFF, 0xE8, 0x1A, 0x52, 0x00, 0x00, 0x64, 0xA1, 0x2C, 0x00, 0x00, 0x00, 0x8B, 0x08, 0x8B, 0x51,
    0x58, 0xA0, 0x48, 0x8B, 0x52, 0x02, 0x3C, 0x01, 0x8B, 0xB2, 0x40, 0x07, 0x00, 0x00, 0xB9, 0x84,
    0x00, 0x00, 0x00, 0xBF, 0xF0, 0xCB, 0x77, 0x02, 0xF3, 0xA5, 0x0F, 0x85, 0x88, 0x00, 0x00, 0x00,
    0x8A, 0x0D, 0x49, 0x8B, 0x52, 0x02, 0x3A, 0xC8, 0x75, 0x7E, 0x83, 0x3D, 0xCC, 0x03, 0x16, 0x02,
    0x02, 0x75, 0x75, 0xE8, 0xF8, 0x29, 0x4B, 0x00, 0x8B, 0xF0, 0x3B, 0xF3, 0x74, 0x6A, 0x8D, 0x54,
    0x24, 0x64, 0x52, 0x8D, 0x44, 0x24, 0x1C, 0x8B, 0xCE, 0xE8, 0x12, 0xA8, 0x4B, 0x00, 0x83, 0xC4,
    0x04, 0x3B, 0x35, 0xD4, 0x03, 0x17, 0x02, 0x74, 0x12, 0xD9, 0x86, 0xD0, 0x12, 0x00, 0x00, 0xD9,
    0x5C, 0x24, 0x14, 0xD9, 0x86, 0xD4, 0x12, 0x00, 0x00, 0xEB, 0x15, 0xE8, 0x80, 0xD5, 0x7F, 0x00,
    0xD9, 0x80, 0x10, 0x17, 0x00, 0x00, 0xD9, 0x5C, 0x24, 0x14, 0xD9, 0x80, 0x14, 0x17, 0x00, 0x00,
    0xD9, 0x5C, 0x24, 0x10, 0x83, 0xEC, 0x08, 0xD9, 0x44, 0x24, 0x18, 0x8D, 0x44, 0x24, 0x6C, 0xD9,
    0x5C, 0x24, 0x04, 0x8D, 0x4C, 0x24, 0x20, 0xD9, 0x44, 0x24, 0x1C, 0xD9, 0x1C, 0x24, 0x50, 0x51,
    0xE8, 0x6B, 0xEC, 0xFF, 0xFF, 0x83, 0xC4, 0x10, 0x38, 0x1D, 0x93, 0x73, 0x52, 0x02, 0x74, 0x08,
    0xD9, 0x05, 0x80, 0x89, 0xE9, 0x00, 0xEB, 0x02, 0xD9, 0xE8, 0x53, 0xD9, 0x1D, 0xDC, 0x6B, 0xFA,
    0x00, 0x89, 0x1D, 0xCC, 0x6C, 0x52, 0x02, 0xE8, 0x64, 0xE9, 0xFF, 0xFF, 0x83, 0xC4, 0x04, 0x6A,
    0x01, 0xB8, 0x14, 0x5B, 0x5F, 0x02, 0xE8, 0x75, 0xED, 0xF8, 0xFF, 0x8A, 0x15, 0xB4, 0x73, 0x52,
    0x02, 0x83, 0xC4, 0x04, 0x38, 0x1D, 0xE4, 0x96, 0xE9, 0x00, 0x74, 0x1B, 0x3A, 0xD3, 0x75, 0x17,
    0x80, 0x3D, 0xC3, 0x6C, 0x52, 0x02, 0x01, 0x75, 0x07, 0xA0, 0xC4, 0x6C, 0x52, 0x02, 0xEB, 0x09,
    0xE8, 0xBB, 0xF7, 0xF8, 0xFF, 0xEB, 0x02, 0x32, 0xC0, 0x3A, 0xD3, 0xA2, 0x82, 0x93, 0xE9, 0x00,
    0x75, 0x21, 0xA1, 0x48, 0x89, 0xE9, 0x00, 0x83, 0xF8, 0x01, 0x74, 0x05, 0x83, 0xF8, 0x02, 0x75,
    0x12, 0x68, 0x14, 0x5B, 0x5F, 0x02, 0xB9, 0x38, 0x5B, 0x5F, 0x02, 0xE8, 0xA0, 0x4F, 0xF9, 0xFF,
    0x83, 0xC4, 0x04, 0x38, 0x1D, 0x82, 0x93, 0xE9, 0x00, 0x74, 0x5F, 0x89, 0x5C, 0x24, 0x48, 0x89,
    0x5C, 0x24, 0x60, 0xFF, 0xD5, 0x8B, 0x54, 0x24, 0x50, 0x8B, 0x35, 0xD8, 0x6B, 0xFA, 0x00, 0x52,
    0x89, 0x44, 0x24, 0x60, 0x8D, 0x44, 0x24, 0x48, 0x68, 0x10, 0x2D, 0x61, 0x02, 0x50, 0xC7, 0x44,
    0x24, 0x50, 0xD0, 0x50, 0x52, 0x00, 0xC7, 0x44, 0x24, 0x60, 0x06, 0x00, 0x00, 0x00, 0xC7, 0x44,
    0x24, 0x58, 0x10, 0x2D, 0x61, 0x02, 0xE8, 0xB5, 0xFD, 0xFF, 0xFF, 0x83, 0xC4, 0x0C, 0x3B, 0xF3,
    0x74, 0x1E, 0x89, 0x74, 0x24, 0x48, 0x8B, 0x16, 0x8B, 0x42, 0x04, 0x8B, 0xCE, 0xFF, 0xD0, 0x8B,
    0x16, 0x8B, 0x42, 0x0C, 0x8B, 0xCE, 0xFF, 0xD0, 0xEB, 0x06, 0x89, 0x1D, 0x9C, 0x73, 0x52, 0x02,
    0xD9, 0xE8, 0x66, 0x83, 0x05, 0xD0, 0x6C, 0x52, 0x02, 0x01, 0x39, 0x1D, 0xCC, 0x6C, 0x52, 0x02,
    0xD9, 0x1D, 0xA4, 0x73, 0x52, 0x02, 0xB9, 0x81, 0x00, 0x00, 0x00, 0xBE, 0xF8, 0x28, 0x61, 0x02,
    0xBF, 0x00, 0x2B, 0x61, 0x02, 0xF3, 0xA5, 0x7E, 0x32, 0xF7, 0x05, 0x18, 0x2D, 0x61, 0x02, 0x00,
    0x20, 0x00, 0x00, 0x74, 0x26, 0xD9, 0x05, 0x58, 0x2D, 0x61, 0x02, 0x51, 0xD9, 0x1C, 0x24, 0x68,
    0x20, 0x2D, 0x61, 0x02, 0x68, 0x10, 0x55, 0x61, 0x02, 0xB8, 0xE0, 0xC9, 0x77, 0x02, 0xB9, 0x14,
    0x5B, 0x5F, 0x02, 0xE8, 0xB8, 0xD1, 0x4D, 0x00, 0x83, 0xC4, 0x0C, 0x39, 0x1D, 0xE8, 0xA3, 0x52,
    0x02, 0x74, 0x44, 0x8B, 0x0D, 0xD8, 0x6B, 0xFA, 0x00, 0x3B, 0xCB, 0x8B, 0xF1, 0x74, 0x38, 0x8B,
    0x11, 0x8B, 0x42, 0x10, 0x8D, 0x79, 0x34, 0xC7, 0x07, 0x28, 0x63, 0x3D, 0x03, 0xFF, 0xD0, 0x89,
    0x5E, 0x04, 0x88, 0x5E, 0x08, 0x89, 0x1F, 0x88, 0x5E, 0x09, 0x89, 0x5E, 0x3C, 0x89, 0x5E, 0x40,
    0x89, 0x5E, 0x44, 0x8B, 0x15, 0x98, 0x5B, 0xEC, 0x00, 0x8B, 0x42, 0x04, 0x83, 0xC6, 0xF8, 0x56,
    0xB9, 0x98, 0x5B, 0xEC, 0x00, 0xFF, 0xD0, 0x5F, 0x5E, 0x5D, 0x5B, 0x83, 0xC4, 0x7C, 0xC3,
];
