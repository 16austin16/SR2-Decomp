//! Proxy `dinput8.dll` that loads into the original `SR2_pc.exe`.
//!
//! Forwards `DirectInput8Create` to the system DLL. The game's first call to it
//! happens after SteamStub has decrypted `.text`, so that is where detours go in
//! (patching in `DllMain` would be overwritten by the decryption).

#![cfg(all(windows, target_arch = "x86"))]

use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HINSTANCE, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress, LoadLibraryA};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryA;
use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

#[cfg(feature = "caller-survey")]
mod caller_targets;
#[cfg(feature = "caller-survey")]
mod callers;
mod controls;
mod crash;
mod detour;
mod display;
mod frame;
mod gfx_cmd;
mod hddio;
mod hooks;
mod input;
mod mempool;
mod menu;
mod missionlog;
mod render;
mod rsched;
mod streamman;
mod teleport;
mod texguard;
mod vint_lua;
#[allow(dead_code)] // enabled by hand when surveying a class family
mod survey;

/// Image base the original exe was linked at (no ASLR expected; verified at attach).
const SR2_IMAGE_BASE: usize = 0x0040_0000;

pub fn log(msg: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("sr2-hook.log") {
        let _ = writeln!(f, "{msg}");
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllMain(_module: HINSTANCE, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        let base = unsafe { GetModuleHandleA(std::ptr::null()) } as usize;
        log(&format!("sr2-hook attached, exe base {base:#x}"));
        if base != SR2_IMAGE_BASE {
            log("warning: exe is not at its preferred base; hooks will be disabled");
        } else {
            display::early();
            // Starts once the loader lock is released, racing SteamStub's decryption.
            std::thread::spawn(hooks::install_early);
        }
    }
    1
}

type DirectInput8CreateFn =
    unsafe extern "system" fn(HINSTANCE, u32, *const c_void, *mut *mut c_void, *mut c_void) -> i32;

fn real_dinput8() -> Option<DirectInput8CreateFn> {
    static REAL: OnceLock<Option<usize>> = OnceLock::new();
    let addr = *REAL.get_or_init(|| unsafe {
        let mut buf = [0u8; 260];
        let n = GetSystemDirectoryA(buf.as_mut_ptr(), buf.len() as u32) as usize;
        if n == 0 || n + 14 > buf.len() {
            return None;
        }
        buf[n..n + 13].copy_from_slice(b"\\dinput8.dll\0");
        let module: HMODULE = LoadLibraryA(buf.as_ptr());
        if module.is_null() {
            return None;
        }
        GetProcAddress(module, c"DirectInput8Create".as_ptr().cast()).map(|f| f as usize)
    });
    addr.map(|a| unsafe { std::mem::transmute::<usize, DirectInput8CreateFn>(a) })
}

#[unsafe(no_mangle)]
unsafe extern "system" fn DirectInput8Create(
    hinst: HINSTANCE,
    version: u32,
    riid: *const c_void,
    out: *mut *mut c_void,
    outer: *mut c_void,
) -> i32 {
    const E_FAIL: i32 = 0x8000_4005_u32 as i32;
    if unsafe { GetModuleHandleA(std::ptr::null()) } as usize == SR2_IMAGE_BASE {
        hooks::install_all();
    }
    match real_dinput8() {
        Some(f) => unsafe { f(hinst, version, riid, out, outer) },
        None => {
            log("failed to load system dinput8.dll");
            E_FAIL
        }
    }
}
