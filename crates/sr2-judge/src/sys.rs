//! Windows plumbing: the supervisor's child process, fixed-address allocations, the fault handler and
//! the hang watchdog.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use windows_sys::Win32::Foundation::{CloseHandle, DuplicateHandle, HANDLE, TRUE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::ReadFile;
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, CONTEXT, EXCEPTION_POINTERS, GetThreadContext, SetThreadContext,
};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, PAGE_READWRITE, VirtualAlloc, VirtualAllocEx,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessW, GetCurrentProcess, GetCurrentThread, GetExitCodeProcess, INFINITE,
    PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOW, SuspendThread, TerminateProcess,
    WaitForSingleObject,
};

pub const MEM_WRITE_WATCH: u32 = 0x0020_0000;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Run this exe again as a child with the game's address range reserved (with write watch) before it
/// starts, and `args` as its command line. Child output is passed to `on_line` line by line. Returns
/// the child's exit code.
pub fn run_child(base: usize, size: usize, args: &str, mut on_line: impl FnMut(&str)) -> u32 {
    unsafe {
        let exe = std::env::current_exe().unwrap();
        let exe = exe.to_str().unwrap().to_string();
        let app = wide(&exe);
        let mut cmd = wide(&format!("\"{exe}\" {args}"));
        std::env::set_var("SR2J_CHILD", "1");
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: TRUE,
        };
        let (mut rd, mut wr): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
        assert!(CreatePipe(&mut rd, &mut wr, &sa, 1 << 16) != 0, "CreatePipe failed");
        // The read end must not be inherited.
        windows_sys::Win32::Foundation::SetHandleInformation(rd, 1, 0);
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.dwFlags = STARTF_USESTDHANDLES;
        si.hStdOutput = wr;
        si.hStdError = wr;
        si.hStdInput = std::ptr::null_mut();
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            app.as_ptr(),
            cmd.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            TRUE,
            CREATE_SUSPENDED,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        );
        assert!(ok != 0, "CreateProcessW failed");
        CloseHandle(wr);
        let p = VirtualAllocEx(
            pi.hProcess,
            base as *const c_void,
            size,
            MEM_COMMIT | MEM_RESERVE | MEM_WRITE_WATCH,
            PAGE_EXECUTE_READWRITE,
        );
        if p as usize != base {
            TerminateProcess(pi.hProcess, 99);
            panic!("cannot reserve the game's address range in the child");
        }
        ResumeThread(pi.hThread);
        let mut buf = vec![0u8; 1 << 16];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let mut n = 0u32;
            let ok = ReadFile(rd, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut());
            if ok == 0 || n == 0 {
                break;
            }
            pending.extend_from_slice(&buf[..n as usize]);
            while let Some(i) = pending.iter().position(|&c| c == b'\n') {
                let line: Vec<u8> = pending.drain(..=i).collect();
                on_line(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']));
            }
        }
        if !pending.is_empty() {
            on_line(&String::from_utf8_lossy(&pending));
        }
        WaitForSingleObject(pi.hProcess, INFINITE);
        let mut code = 0u32;
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(rd);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        code
    }
}

/// Commit `size` bytes at exactly `addr`.
pub fn alloc_fixed(addr: usize, size: usize, exec: bool) -> *mut u8 {
    let p = unsafe {
        VirtualAlloc(
            addr as *const c_void,
            size,
            MEM_COMMIT | MEM_RESERVE,
            if exec { PAGE_EXECUTE_READWRITE } else { PAGE_READWRITE },
        )
    };
    assert!(p as usize == addr, "cannot allocate {size:#x} bytes at {addr:#x}");
    p as *mut u8
}

// ---- faults and hangs ---------------------------------------------------------------------------

/// True while a case runs on the test stack.
pub static IN_CASE: AtomicBool = AtomicBool::new(false);
pub static FAULT_CODE: AtomicU32 = AtomicU32::new(0);
pub static FAULT_ADDR: AtomicU32 = AtomicU32::new(0);
/// Tick (ms) when the current run started.
pub static RUN_START: AtomicU32 = AtomicU32::new(0);
static EXIT_EIP: AtomicUsize = AtomicUsize::new(0);
static MAIN_THREAD: AtomicUsize = AtomicUsize::new(0);
pub const HANG: u32 = 0xFFFF_0001;
pub const STUB_FLOOD: u32 = 0xFFFF_0002;
pub const PANIC: u32 = 0xFFFF_0003;
pub static PANIC_MSG: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
pub static PANICKED: AtomicBool = AtomicBool::new(false);
pub const RUN_LIMIT_MS: u32 = 700;

fn is_fault(code: u32) -> bool {
    matches!(
        code,
        0xC000_0005 // access violation
            | 0xC000_0006 // in page error
            | 0xC000_001D // illegal instruction
            | 0xC000_0094 // integer divide by zero
            | 0xC000_0095 // integer overflow
            | 0xC000_0096 // privileged instruction
            | 0xC000_008C // array bounds
            | 0xC000_0025 // noncontinuable
            | 0xC000_00FD // stack overflow
            | 0x8000_0003 // breakpoint
            | 0x8000_0004 // single step
            | 0x8000_0002 // misaligned
            | 0xC000_008D..=0xC000_0093 // x87 / SSE exceptions
            | 0xC000_0026 // invalid disposition
    )
}

unsafe extern "system" fn veh(ep: *mut EXCEPTION_POINTERS) -> i32 {
    unsafe {
        if !IN_CASE.load(Ordering::SeqCst) {
            return 0;
        }
        let rec = (*ep).ExceptionRecord;
        let code = (*rec).ExceptionCode as u32;
        if !is_fault(code) {
            return 0;
        }
        FAULT_CODE.store(code, Ordering::SeqCst);
        FAULT_ADDR.store((*rec).ExceptionAddress as u32, Ordering::SeqCst);
        (*(*ep).ContextRecord).Eip = EXIT_EIP.load(Ordering::SeqCst) as u32;
        -1 // EXCEPTION_CONTINUE_EXECUTION
    }
}

fn tick() -> u32 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}

pub fn mark_run_start() {
    RUN_START.store(tick(), Ordering::SeqCst);
}

/// Install the fault handler and the hang watchdog. `exit_eip` is where a faulting run resumes.
pub fn install_fault_machinery(exit_eip: usize) {
    EXIT_EIP.store(exit_eip, Ordering::SeqCst);
    unsafe {
        AddVectoredExceptionHandler(1, Some(veh));
        let mut h: HANDLE = std::ptr::null_mut();
        let cur = GetCurrentProcess();
        DuplicateHandle(cur, GetCurrentThread(), cur, &mut h, 0, 0, 2 /* DUPLICATE_SAME_ACCESS */);
        MAIN_THREAD.store(h as usize, Ordering::SeqCst);
    }
    // A panic in a port (or anything it calls) ends the run as a fault. The unwinder cannot cross the
    // hand-written thunks, so the hook leaves directly; the process must not panic again afterwards, so
    // the supervisor restarts it after a port that panicked (PANICKED).
    std::panic::set_hook(Box::new(move |info| {
        if IN_CASE.load(Ordering::SeqCst) && !PANICKED.load(Ordering::SeqCst) {
            PANICKED.store(true, Ordering::SeqCst);
            FAULT_CODE.store(PANIC, Ordering::SeqCst);
            FAULT_ADDR.store(0, Ordering::SeqCst);
            if let Ok(mut m) = PANIC_MSG.lock() {
                *m = info.to_string().replace('\n', " ");
            }
            unsafe { std::arch::asm!("jmp {}", in(reg) exit_eip, options(noreturn)) }
        }
        eprintln!("{info}");
    }));
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if IN_CASE.load(Ordering::SeqCst) && tick().wrapping_sub(RUN_START.load(Ordering::SeqCst)) > RUN_LIMIT_MS {
                unsafe {
                    let h = MAIN_THREAD.load(Ordering::SeqCst) as HANDLE;
                    SuspendThread(h);
                    if IN_CASE.load(Ordering::SeqCst) {
                        let mut c: CONTEXT = std::mem::zeroed();
                        c.ContextFlags = 0x0001_0007; // CONTEXT_i386 | CONTROL | INTEGER | SEGMENTS
                        if GetThreadContext(h, &mut c) != 0 {
                            c.Eip = EXIT_EIP.load(Ordering::SeqCst) as u32;
                            FAULT_CODE.store(HANG, Ordering::SeqCst);
                            FAULT_ADDR.store(0, Ordering::SeqCst);
                            SetThreadContext(h, &c);
                        }
                    }
                    ResumeThread(h);
                }
                // Do not fire again for the same run.
                RUN_START.store(tick(), Ordering::SeqCst);
            }
        }
    });
}
