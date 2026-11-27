//! The game exe, mapped by section at its real base 0x400000 without running anything from it, with
//! page-level write tracking so each case can restore and compare exactly the pages it touched.

use std::ffi::c_void;

use windows_sys::Win32::System::Memory::{GetWriteWatch, MEMORY_BASIC_INFORMATION, VirtualQuery};

pub const BASE: usize = 0x40_0000;
pub const SIZE: usize = 0x31A_D000;
pub const PAGE: usize = 0x1000;
/// The code section: RVA 0x1000, size 0x9b61f4.
pub const TEXT_LO: usize = 0x40_1000;
pub const TEXT_HI: usize = 0x40_1000 + 0x9b_61f4;
/// Writable data: .data / .bss (RVA 0xa80000, 0x262639c bytes) and .tls.
pub const DATA_LO: usize = 0xe8_0000;
pub const DATA_HI: usize = 0x30a_9000;

pub struct Image {
    /// What the image holds when nothing has run: the sections, plus the judge's callee patches.
    pub pristine: Vec<u8>,
}

pub fn exe_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bin/SR2_pc.unpacked.exe")
}

pub fn in_text(a: usize) -> bool {
    (TEXT_LO..TEXT_HI).contains(&a)
}

pub fn in_image(a: usize) -> bool {
    (BASE..BASE + SIZE).contains(&a)
}

impl Image {
    /// Copy the exe's sections into the reserved range. The range must have been reserved by the parent.
    pub fn map() -> Image {
        unsafe {
            let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            VirtualQuery(BASE as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
            assert!(
                mbi.State == 0x1000 && mbi.RegionSize >= SIZE,
                "the game's address range is not reserved: run sr2-judge normally, not as the child"
            );
        }
        let exe = std::fs::read(exe_path()).expect("read bin/SR2_pc.unpacked.exe");
        let u32at = |o: usize| u32::from_le_bytes(exe[o..o + 4].try_into().unwrap()) as usize;
        let u16at = |o: usize| u16::from_le_bytes(exe[o..o + 2].try_into().unwrap()) as usize;
        let pe = u32at(0x3C);
        let nsec = u16at(pe + 6);
        let opt = u16at(pe + 20);
        let mut pristine = vec![0u8; SIZE];
        for i in 0..nsec {
            let o = pe + 24 + opt + i * 40;
            let (vsz, va, rsz, raw) = (u32at(o + 8), u32at(o + 12), u32at(o + 16), u32at(o + 20));
            let n = rsz.min(vsz);
            pristine[va..va + n].copy_from_slice(&exe[raw..raw + n]);
        }
        let img = Image { pristine };
        img.restore_all();
        img
    }

    pub fn restore_all(&self) {
        unsafe { std::ptr::copy_nonoverlapping(self.pristine.as_ptr(), BASE as *mut u8, SIZE) };
        reset_watch();
    }

    pub fn orig(&self, addr: usize, len: usize) -> &[u8] {
        &self.pristine[addr - BASE..addr - BASE + len]
    }

    /// Write bytes to the live image and to the pristine copy (callee patches).
    pub fn patch(&mut self, addr: usize, bytes: &[u8]) {
        self.pristine[addr - BASE..addr - BASE + bytes.len()].copy_from_slice(bytes);
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len()) };
    }

    /// Put page `page_addr` (page aligned) back to the pristine bytes.
    pub fn restore_page(&self, page_addr: usize) {
        let o = page_addr - BASE;
        let n = PAGE.min(SIZE - o);
        unsafe { std::ptr::copy_nonoverlapping(self.pristine[o..].as_ptr(), page_addr as *mut u8, n) };
    }
}

/// The pages written since the last reset (page-aligned addresses), and reset the tracking.
pub fn dirty_pages() -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut buf = vec![std::ptr::null_mut::<c_void>(); 16384];
    let mut start = BASE;
    unsafe {
        loop {
            let mut count = buf.len();
            let mut gran = 0u32;
            let r = GetWriteWatch(
                1, // WRITE_WATCH_FLAG_RESET
                start as *const c_void,
                BASE + SIZE - start,
                buf.as_mut_ptr(),
                &mut count,
                &mut gran,
            );
            assert!(r == 0, "GetWriteWatch failed");
            out.extend(buf[..count].iter().map(|&p| p as usize));
            if count < buf.len() {
                break;
            }
            start = *out.last().unwrap() + PAGE;
        }
    }
    out
}

pub fn reset_watch() {
    let _ = dirty_pages();
}
