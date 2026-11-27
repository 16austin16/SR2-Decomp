//! Running one function: set every register from a struct, call the target on a private stack, and
//! save everything it leaves behind. Faults and hangs come back as a fault code instead of killing the
//! process.

use std::mem::offset_of;
use std::sync::atomic::Ordering;

use crate::stubs::{self, CallRec};
use crate::sys;
use crate::world::World;

pub const STACK_BASE: usize = 0x3800_0000;
pub const STACK_SIZE: usize = 0x4_0000;
/// Where the return address sits for every run; the arguments follow it.
pub const ESP_IN: usize = STACK_BASE + STACK_SIZE - 0x200;
/// The dwords above the return address that the world fills and the judge compares afterwards.
pub const STACK_ARGS: usize = 24;

#[repr(C, align(16))]
pub struct Ctx {
    regs_in: [u32; 8],
    efl_in: u32,
    esp_in: u32,
    target: u32,
    cw: u32,
    mxcsr: u32,
    _pad: [u32; 3],
    xmm_in: [[u32; 4]; 8],
    regs_out: [u32; 8],
    efl_out: u32,
    esp_out: u32,
    _pad2: [u32; 2],
    fx: [u8; 512],
}

static mut CTX: Ctx = Ctx {
    regs_in: [0; 8],
    efl_in: 0,
    esp_in: 0,
    target: 0,
    cw: 0,
    mxcsr: 0,
    _pad: [0; 3],
    xmm_in: [[0; 4]; 8],
    regs_out: [0; 8],
    efl_out: 0,
    esp_out: 0,
    _pad2: [0; 2],
    fx: [0; 512],
};
static mut HOST_ESP: u32 = 0;

const O_IN: usize = offset_of!(Ctx, regs_in);
const O_EFL: usize = offset_of!(Ctx, efl_in);
const O_ESP: usize = offset_of!(Ctx, esp_in);
const O_TARGET: usize = offset_of!(Ctx, target);
const O_CW: usize = offset_of!(Ctx, cw);
const O_MXCSR: usize = offset_of!(Ctx, mxcsr);
const O_XMM: usize = offset_of!(Ctx, xmm_in);
const O_OUT: usize = offset_of!(Ctx, regs_out);
const O_EFL_OUT: usize = offset_of!(Ctx, efl_out);
const O_ESP_OUT: usize = offset_of!(Ctx, esp_out);
const O_FX: usize = offset_of!(Ctx, fx);

/// Switch to the test stack with every register loaded, and jump to the target. The target returns to
/// `after_call`.
#[unsafe(naked)]
unsafe extern "C" fn run_raw() -> u32 {
    std::arch::naked_asm!(
        "push ebx",
        "push esi",
        "push edi",
        "push ebp",
        "mov [{host}], esp",
        "push dword ptr [{ctx} + {o_efl}]",
        "popfd",
        "fninit",
        "fldcw word ptr [{ctx} + {o_cw}]",
        "ldmxcsr dword ptr [{ctx} + {o_mxcsr}]",
        "movups xmm0, [{ctx} + {o_xmm}]",
        "movups xmm1, [{ctx} + {o_xmm} + 16]",
        "movups xmm2, [{ctx} + {o_xmm} + 32]",
        "movups xmm3, [{ctx} + {o_xmm} + 48]",
        "movups xmm4, [{ctx} + {o_xmm} + 64]",
        "movups xmm5, [{ctx} + {o_xmm} + 80]",
        "movups xmm6, [{ctx} + {o_xmm} + 96]",
        "movups xmm7, [{ctx} + {o_xmm} + 112]",
        "mov esp, [{ctx} + {o_esp}]",
        "mov ebx, [{ctx} + {o_in} + 12]",
        "mov ebp, [{ctx} + {o_in} + 20]",
        "mov esi, [{ctx} + {o_in} + 24]",
        "mov edi, [{ctx} + {o_in} + 28]",
        "mov ecx, [{ctx} + {o_in} + 4]",
        "mov edx, [{ctx} + {o_in} + 8]",
        "mov eax, [{ctx} + {o_in}]",
        "jmp dword ptr [{ctx} + {o_target}]",
        ctx = sym CTX,
        host = sym HOST_ESP,
        o_efl = const O_EFL,
        o_cw = const O_CW,
        o_mxcsr = const O_MXCSR,
        o_xmm = const O_XMM,
        o_esp = const O_ESP,
        o_in = const O_IN,
        o_target = const O_TARGET,
    )
}

/// Where the target returns to: save the registers, flags, stack pointer and the whole FPU / SSE state.
#[unsafe(naked)]
pub unsafe extern "C" fn after_call() {
    std::arch::naked_asm!(
        "mov [{ctx} + {o_out}], eax",
        "mov [{ctx} + {o_out} + 4], ecx",
        "mov [{ctx} + {o_out} + 8], edx",
        "mov [{ctx} + {o_out} + 12], ebx",
        "mov [{ctx} + {o_out} + 20], ebp",
        "mov [{ctx} + {o_out} + 24], esi",
        "mov [{ctx} + {o_out} + 28], edi",
        "mov [{ctx} + {o_esp_out}], esp",
        "mov esp, [{host}]",
        "pushfd",
        "pop dword ptr [{ctx} + {o_efl_out}]",
        "fxsave [{ctx} + {o_fx}]",
        "fninit",
        "pop ebp",
        "pop edi",
        "pop esi",
        "pop ebx",
        "xor eax, eax",
        "ret",
        ctx = sym CTX,
        host = sym HOST_ESP,
        o_out = const O_OUT,
        o_esp_out = const O_ESP_OUT,
        o_efl_out = const O_EFL_OUT,
        o_fx = const O_FX,
    )
}

/// A faulting or hanging run resumes here.
#[unsafe(naked)]
pub unsafe extern "C" fn fault_exit() {
    std::arch::naked_asm!(
        "fninit",
        "mov esp, [{host}]",
        "pop ebp",
        "pop edi",
        "pop esi",
        "pop ebx",
        "mov eax, 1",
        "ret",
        host = sym HOST_ESP,
    )
}

/// The common part of every callee stub. See `stubs::record` for the frame it hands over.
#[unsafe(naked)]
pub unsafe extern "C" fn stub_common() {
    std::arch::naked_asm!(
        "pushfd",
        "pushad",
        "sub esp, 144",
        "movups [esp], xmm0",
        "movups [esp + 16], xmm1",
        "movups [esp + 32], xmm2",
        "movups [esp + 48], xmm3",
        "movups [esp + 64], xmm4",
        "movups [esp + 80], xmm5",
        "movups [esp + 96], xmm6",
        "movups [esp + 112], xmm7",
        "lea eax, [esp + 144]",
        "push eax",
        "call {record}",
        "add esp, 4",
        "test eax, eax",
        "jnz 2f",
        "movups xmm0, [esp]",
        "movups xmm1, [esp + 16]",
        "movups xmm2, [esp + 32]",
        "movups xmm3, [esp + 48]",
        "movups xmm4, [esp + 64]",
        "movups xmm5, [esp + 80]",
        "movups xmm6, [esp + 96]",
        "movups xmm7, [esp + 112]",
        "add esp, 144",
        "popad",
        "popfd",
        "ret",
        "2:",
        "jmp {fault}",
        record = sym stubs::record,
        fault = sym fault_exit,
    )
}

pub fn install_faults() {
    sys::install_fault_machinery(fault_exit as *const () as usize);
}

#[derive(Clone)]
pub struct RunResult {
    pub fault: Option<(u32, u32)>,
    pub regs: [u32; 8],
    pub esp: u32,
    pub x87_depth: u32,
    pub st0: [u8; 10],
    pub xmm: [[u32; 4]; 8],
    pub stack_above: [u32; STACK_ARGS],
    pub calls: Vec<CallRec>,
    pub poison: Vec<u32>,
}

/// Run `target` once in `world`'s register / stack state. The caller restores memory around it.
pub fn run(target: usize, w: &World, seed: u64) -> RunResult {
    unsafe {
        // Stack: zero the working area below the arguments, then the arguments.
        std::ptr::write_bytes((ESP_IN - 0x8000) as *mut u8, 0, 0x8000);
        *(ESP_IN as *mut u32) = after_call as *const () as usize as u32;
        for (k, v) in w.stack.iter().enumerate() {
            *((ESP_IN + 4 + k * 4) as *mut u32) = *v;
        }
        let c = &raw mut CTX;
        (*c).regs_in = w.regs;
        (*c).efl_in = 0x202;
        (*c).esp_in = ESP_IN as u32;
        (*c).target = target as u32;
        (*c).cw = 0x027F; // 53-bit precision, round to nearest, all exceptions masked
        (*c).mxcsr = 0x1F80;
        (*c).xmm_in = w.xmm;
        (*c).regs_out = [0; 8];
        (*c).fx = [0; 512];
        stubs::reset_run(seed);
        sys::FAULT_CODE.store(0, Ordering::SeqCst);
        sys::FAULT_ADDR.store(0, Ordering::SeqCst);
        sys::mark_run_start();
        sys::IN_CASE.store(true, Ordering::SeqCst);
        let faulted = run_raw();
        sys::IN_CASE.store(false, Ordering::SeqCst);
        let mut r = RunResult {
            fault: None,
            regs: [0; 8],
            esp: 0,
            x87_depth: 0,
            st0: [0; 10],
            xmm: [[0; 4]; 8],
            stack_above: [0; STACK_ARGS],
            calls: Vec::new(),
            poison: Vec::new(),
        };
        if faulted != 0 {
            r.fault = Some((sys::FAULT_CODE.load(Ordering::SeqCst), sys::FAULT_ADDR.load(Ordering::SeqCst)));
        }
        // Even a faulting run reports the call log it got through.
        r.calls = (0..stubs::LOG_N).map(|i| stubs::LOG[i]).collect();
        r.poison = (0..stubs::POISON_N).map(|i| stubs::POISON[i]).collect();
        if r.fault.is_none() {
            r.regs = (*c).regs_out;
            r.esp = (*c).esp_out;
            let fx = &(*c).fx;
            r.x87_depth = fx[4].count_ones();
            r.st0.copy_from_slice(&fx[32..42]);
            for i in 0..8 {
                for k in 0..4 {
                    let o = 160 + i * 16 + k * 4;
                    r.xmm[i][k] = u32::from_le_bytes(fx[o..o + 4].try_into().unwrap());
                }
            }
            for k in 0..STACK_ARGS {
                r.stack_above[k] = *((ESP_IN + 4 + k * 4) as *const u32);
            }
        }
        r
    }
}
