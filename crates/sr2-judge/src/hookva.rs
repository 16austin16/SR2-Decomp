//! While the registry is collected, executable allocations fail. The ports' install() functions copy the
//! original bodies into executable blocks to run them as "shadows" next to the Rust code; with the
//! allocation refused those shadows stay switched off, so the judge sees the Rust code alone.
//!
//! Done by patching this exe's own import slot for VirtualAlloc.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use windows_sys::Win32::Foundation::SetLastError;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};

static BLOCK: AtomicBool = AtomicBool::new(false);
static REAL: AtomicUsize = AtomicUsize::new(0);
pub static REFUSED: AtomicUsize = AtomicUsize::new(0);

type VaFn = unsafe extern "system" fn(*const c_void, usize, u32, u32) -> *mut c_void;

unsafe extern "system" fn hook(addr: *const c_void, size: usize, ty: u32, prot: u32) -> *mut c_void {
    unsafe {
        if BLOCK.load(Ordering::SeqCst) && matches!(prot, 0x10 | 0x20 | 0x40 | 0x80) {
            REFUSED.fetch_add(1, Ordering::SeqCst);
            SetLastError(8); // not enough memory
            return std::ptr::null_mut();
        }
        let real: VaFn = std::mem::transmute(REAL.load(Ordering::SeqCst));
        real(addr, size, ty, prot)
    }
}

/// Patch the import slot (once) and switch refusing on or off.
pub fn set_blocking(on: bool) -> bool {
    if REAL.load(Ordering::SeqCst) == 0 {
        unsafe {
            let base = GetModuleHandleW(std::ptr::null()) as usize;
            let rd = |o: usize| *((base + o) as *const u32) as usize;
            let pe = rd(0x3C);
            let imp = rd(pe + 24 + 96 + 8); // data directory 1: imports
            let mut d = base + imp;
            let mut found = false;
            while *(d as *const u32).add(3) != 0 {
                let oft = *(d as *const u32) as usize;
                let ft = *(d as *const u32).add(4) as usize;
                let names = if oft != 0 { oft } else { ft };
                let mut i = 0;
                loop {
                    let ent = *((base + names + i * 4) as *const u32) as usize;
                    if ent == 0 {
                        break;
                    }
                    if ent & 0x8000_0000 == 0 {
                        let nm = std::ffi::CStr::from_ptr((base + ent + 2) as *const _);
                        if nm.to_bytes() == b"VirtualAlloc" {
                            let slot = (base + ft + i * 4) as *mut usize;
                            let mut old = 0u32;
                            VirtualProtect(slot as *const c_void, 4, PAGE_READWRITE, &mut old);
                            REAL.store(*slot, Ordering::SeqCst);
                            *slot = hook as *const () as usize;
                            VirtualProtect(slot as *const c_void, 4, old, &mut old);
                            found = true;
                        }
                    }
                    i += 1;
                }
                d += 20;
            }
            if !found {
                return false;
            }
        }
    }
    BLOCK.store(on, Ordering::SeqCst);
    true
}
