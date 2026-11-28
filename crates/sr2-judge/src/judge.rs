//! The judge proper: run the original bytes and the port on the same random worlds and compare
//! everything either one leaves behind.

use dinput8::registry::RegEntry;

use crate::analysis::{self, Analyzer, FnInfo, REG_NAMES};
use crate::exec::{self, RunResult};
use crate::image::{self, Image, PAGE};
use crate::stubs::{self, splitmix};
use crate::sys;
use crate::world::{self, ARENA_BASE, ARENA_SIZE};

pub struct Opts {
    pub cases: u32,
    pub seed: u64,
    pub verbose: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Pass,
    Fail,
    Inconclusive,
    Error,
}

impl Status {
    pub fn word(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::Inconclusive => "INCONCLUSIVE",
            Status::Error => "ERROR",
        }
    }
}

pub struct Verdict {
    pub status: Status,
    pub cases: u32,
    pub ok: u32,
    pub both_fault: u32,
    pub diffs: Vec<String>,
    pub note: String,
    /// What differed in the failing case: regs, esp, x87, xmm, stack, calls, memory, fault.
    pub kinds: Vec<&'static str>,
}

fn kinds_of(diffs: &[String]) -> Vec<&'static str> {
    let mut k: Vec<&'static str> = Vec::new();
    for d in diffs.iter().skip(1) {
        let kind = if d.starts_with("xmm") {
            "xmm"
        } else if d.starts_with("esp") {
            "esp"
        } else if d.starts_with("x87") || d.starts_with("ST0") {
            "x87"
        } else if d.starts_with("stack arg") {
            "stack"
        } else if d.starts_with("callee call") {
            "calls"
        } else if d.starts_with("arena") || d.starts_with("image") {
            "memory"
        } else if d.starts_with("the original faulted") || d.starts_with("the port faulted") {
            "fault"
        } else {
            "regs"
        };
        if !k.contains(&kind) {
            k.push(kind);
        }
    }
    k
}

pub struct Env {
    pub img: Image,
    pub an: Analyzer,
    arena_snap: Vec<u8>,
    arena_a: Vec<u8>,
}

impl Env {
    pub fn new() -> Env {
        sys::alloc_fixed(ARENA_BASE, ARENA_SIZE, false);
        sys::alloc_fixed(exec::STACK_BASE, exec::STACK_SIZE, false);
        sys::alloc_fixed(stubs::STUB_BASE, stubs::MAX_STUBS * stubs::STUB_SLOT, true);
        sys::alloc_fixed(stubs::VT_BASE, stubs::VT_SIZE, false);
        exec::install_faults();
        Env { img: Image::map(), an: Analyzer::new(), arena_snap: vec![0; ARENA_SIZE], arena_a: vec![0; ARENA_SIZE] }
    }
}

fn arena() -> &'static mut [u8] {
    unsafe { std::slice::from_raw_parts_mut(ARENA_BASE as *mut u8, ARENA_SIZE) }
}

fn fault_name(f: (u32, u32)) -> String {
    match f.0 {
        sys::HANG => "hang (over the time limit)".to_string(),
        sys::STUB_FLOOD => "over 1024 callee calls".to_string(),
        sys::PANIC => format!("panic: {}", sys::PANIC_MSG.lock().map(|m| m.clone()).unwrap_or_default()),
        0xC000_0005 => format!("access violation at {:#x}", f.1),
        c => format!("exception {c:#x} at {:#x}", f.1),
    }
}

/// The page's bytes as a case starts: pristine plus the case's globals.
fn baseline(img: &Image, page: usize, globals: &[(usize, Vec<u8>)]) -> Vec<u8> {
    let o = page - image::BASE;
    let n = PAGE.min(image::SIZE - o);
    let mut v = img.pristine[o..o + n].to_vec();
    for (a, b) in globals {
        for (k, x) in b.iter().enumerate() {
            let at = a + k;
            if at >= page && at < page + n {
                v[at - page] = *x;
            }
        }
    }
    v
}

fn describe_call(c: &stubs::CallRec, info: &FnInfo) -> String {
    let t = info.callees.get(c.id as usize).map(|c| c.target).unwrap_or(0);
    let regs: Vec<String> = (0..8).filter(|&i| i != analysis::ESP).map(|i| format!("{}={:#x}", REG_NAMES[i], c.regs[i])).collect();
    format!("call {t:#x} depth {} {} stack {:x?}", c.depth, regs.join(" "), &c.stack[..4])
}

/// Two values passed to a callee agree when they are equal, or when both point into the run's own stack:
/// each is then the address of a local or an argument slot of its function, and Rust code cannot
/// reproduce the original's frame layout.
fn same_arg(x: u32, y: u32) -> bool {
    // The run's stack, its own argument slots included (both serve as scratch; what a callee writes there is
    // compared separately as the stack above the return address).
    let local = |v: u32| (exec::STACK_BASE as u32..(exec::ESP_IN + 4 + 4 * exec::STACK_ARGS) as u32).contains(&v);
    // Two NaNs passed by value: x87 and SSE keep different payloads and no game code depends on them.
    let nan = |v: u32| v & 0x7F80_0000 == 0x7F80_0000 && v & 0x007F_FFFF != 0;
    x == y || (local(x) && local(y)) || (nan(x) && nan(y))
}

/// Everything compared for one case; empty when the two runs agree.
fn compare(a: &RunResult, b: &RunResult, info: &FnInfo) -> Vec<String> {
    let mut d: Vec<String> = Vec::new();
    for i in 0..8 {
        if i == analysis::ESP {
            continue;
        }
        if (i == analysis::ECX || i == analysis::EDX) && a.poison.contains(&a.regs[i]) {
            continue; // the original left a value a callee clobbered there; nobody can depend on it
        }
        if a.regs[i] != b.regs[i] {
            d.push(format!("{}: original {:#010x}, port {:#010x}", REG_NAMES[i], a.regs[i], b.regs[i]));
        }
    }
    if a.esp != b.esp {
        d.push(format!("esp after return: original {:#x}, port {:#x}", a.esp, b.esp));
    }
    if a.x87_depth != b.x87_depth {
        d.push(format!("x87 depth: original {}, port {}", a.x87_depth, b.x87_depth));
    } else if a.x87_depth > 0 && a.st0 != b.st0 {
        d.push(format!("ST0: original {:02x?}, port {:02x?}", a.st0, b.st0));
    }
    for i in 0..8 {
        if a.xmm[i] != b.xmm[i] {
            d.push(format!("xmm{i}: original {:08x?}, port {:08x?}", a.xmm[i], b.xmm[i]));
        }
    }
    for k in 0..exec::STACK_ARGS {
        if a.stack_above[k] != b.stack_above[k] {
            d.push(format!("stack arg {k}: original {:#x}, port {:#x}", a.stack_above[k], b.stack_above[k]));
        }
    }
    // The ordered callee calls.
    let n = a.calls.len().min(b.calls.len());
    let mut shown = 0;
    for i in 0..n {
        let (x, y) = (&a.calls[i], &b.calls[i]);
        let callee = info.callees.get(x.id as usize);
        let mut why = Vec::new();
        if x.id != y.id {
            why.push("different callee".to_string());
        } else if let Some(c) = callee {
            for r in 0..8 {
                if r != analysis::ESP && c.live_in & (1 << r) != 0 && !same_arg(x.regs[r], y.regs[r]) {
                    why.push(format!("{} {:#x} vs {:#x}", REG_NAMES[r], x.regs[r], y.regs[r]));
                }
            }
            if x.depth != y.depth {
                why.push(format!("x87 depth {} vs {}", x.depth, y.depth));
            }
            for k in 0..c.nargs.min(stubs::NSTACK) {
                if !same_arg(x.stack[k], y.stack[k]) {
                    why.push(format!("stack arg {k} {:#x} vs {:#x}", x.stack[k], y.stack[k]));
                }
            }
        }
        if !why.is_empty() && shown < 3 {
            shown += 1;
            d.push(format!(
                "callee call #{i}: {}\n      original: {}\n      port:     {}",
                why.join(", "),
                describe_call(x, info),
                describe_call(y, info)
            ));
        }
    }
    if a.calls.len() != b.calls.len() {
        let extra = if a.calls.len() > b.calls.len() { ("original", &a.calls[n]) } else { ("port", &b.calls[n]) };
        d.push(format!(
            "callee call count: original {}, port {}; first extra call is the {}'s: {}",
            a.calls.len(),
            b.calls.len(),
            extra.0,
            describe_call(extra.1, info)
        ));
    }
    d
}

fn diff_bytes(label: &str, base: usize, a: &[u8], b: &[u8], out: &mut Vec<String>) {
    let mut shown = 0;
    let mut i = 0;
    while i < a.len() && shown < 4 {
        if a[i] != b[i] {
            let s = i & !3;
            let e = (s + 4).min(a.len());
            out.push(format!("{label} {:#x}: original {:02x?}, port {:02x?}", base + s, &a[s..e], &b[s..e]));
            shown += 1;
            i = s + 4;
        } else {
            i += 1;
        }
    }
}

enum CaseResult {
    BothFault,
    Same,
    Differ(Vec<String>),
}

fn run_case(env: &mut Env, e: &RegEntry, info: &FnInfo, seed: u64, verbose: bool) -> CaseResult {
    let w = world::make(seed, info);
    world::fill_arena(seed);
    stubs::reset_vtables(&info.indirect_sites);
    env.arena_snap.copy_from_slice(arena());
    for (a, b) in &w.globals {
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), *a as *mut u8, b.len()) };
    }
    image::reset_watch();
    let a = exec::run(e.addr, &w, seed);
    let dirty_a = image::dirty_pages();
    let saved_a: Vec<(usize, Vec<u8>)> = dirty_a
        .iter()
        .map(|&p| (p, unsafe { std::slice::from_raw_parts(p as *const u8, PAGE.min(image::BASE + image::SIZE - p)) }.to_vec()))
        .collect();
    env.arena_a.copy_from_slice(arena());
    // Put the world back.
    arena().copy_from_slice(&env.arena_snap);
    stubs::reset_vtables(&info.indirect_sites);
    for &p in &dirty_a {
        env.img.restore_page(p);
    }
    for (ad, b) in &w.globals {
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), *ad as *mut u8, b.len()) };
    }
    image::reset_watch();
    let b = exec::run(e.thunk, &w, seed);
    let dirty_b = image::dirty_pages();

    let result = (|| {
        if verbose {
            println!(
                "    seed {seed:#x}: original {} / port {}, {} / {} callee calls",
                a.fault.map(fault_name).unwrap_or_else(|| "ok".into()),
                b.fault.map(fault_name).unwrap_or_else(|| "ok".into()),
                a.calls.len(),
                b.calls.len()
            );
        }
        if let Some(f) = b.fault {
            if f.0 == sys::PANIC {
                // The original cannot panic; and a panic leaves the process in a state where the next one aborts.
                return CaseResult::Differ(vec![format!("the port faulted ({}) but the original {}", fault_name(f), a.fault.map(|_| "faulted too").unwrap_or("ran to the end"))]);
            }
        }
        match (a.fault, b.fault) {
            (Some(_), Some(_)) => return CaseResult::BothFault,
            (Some(f), None) => {
                return CaseResult::Differ(vec![format!("the original faulted ({}) but the port ran to the end", fault_name(f))]);
            }
            (None, Some(f)) => {
                return CaseResult::Differ(vec![format!("the port faulted ({}) but the original ran to the end", fault_name(f))]);
            }
            _ => {}
        }
        let mut d = compare(&a, &b, info);
        diff_bytes("arena", ARENA_BASE, &env.arena_a, arena(), &mut d);
        let mut pages: Vec<usize> = dirty_a.iter().chain(dirty_b.iter()).copied().collect();
        pages.sort_unstable();
        pages.dedup();
        let mut shown = 0;
        for p in pages {
            if shown >= 4 {
                break;
            }
            let base = baseline(&env.img, p, &w.globals);
            let ca = saved_a.iter().find(|(q, _)| *q == p).map(|(_, v)| v.as_slice()).unwrap_or(&base);
            let cb: &[u8] = if dirty_b.contains(&p) {
                unsafe { std::slice::from_raw_parts(p as *const u8, base.len()) }
            } else {
                &base
            };
            if ca != cb {
                let before = d.len();
                diff_bytes("image", p, ca, cb, &mut d);
                shown += d.len() - before;
            }
        }
        if d.is_empty() { CaseResult::Same } else { CaseResult::Differ(d) }
    })();

    // Leave the image clean for the next case.
    for &p in &dirty_b {
        env.img.restore_page(p);
    }
    for (ad, b) in &w.globals {
        let page = *ad & !(PAGE - 1);
        let _ = b;
        env.img.restore_page(page);
        let last = (*ad + b.len() - 1) & !(PAGE - 1);
        if last != page {
            env.img.restore_page(last);
        }
    }
    image::reset_watch();
    result
}

pub fn judge(env: &mut Env, e: &RegEntry, o: &Opts) -> Verdict {
    let mut v = Verdict { status: Status::Error, cases: 0, ok: 0, both_fault: 0, diffs: Vec::new(), note: String::new(), kinds: Vec::new() };
    let len = e.body.len();
    if len < 5 || !image::in_image(e.addr) || !image::in_image(e.addr + len - 1) {
        v.note = "bad address or body".into();
        return v;
    }
    if env.img.orig(e.addr, len) != e.body.as_slice() {
        v.note = "the pinned body differs from the exe's bytes".into();
        return v;
    }
    let info = env.an.analyze(&env.img, e.addr, &e.body);
    // The callee patches must not touch each other or the body.
    let mut targets: Vec<usize> = info.callees.iter().filter(|c| !c.indirect).map(|c| c.target).collect();
    targets.sort_unstable();
    for w in targets.windows(2) {
        if w[1] < w[0] + 5 {
            v.note = format!("callees {:#x} and {:#x} are closer than 5 bytes", w[0], w[1]);
            return v;
        }
    }
    if targets.iter().any(|&t| t + 5 > e.addr && t < e.addr + len) {
        v.note = "a callee entry overlaps the function's own body".into();
        return v;
    }
    let saved: Vec<(usize, [u8; 5])> =
        targets.iter().map(|&t| (t, env.img.orig(t, 5).try_into().unwrap())).collect();
    if let Err(m) = stubs::install(&mut env.img, &info.callees) {
        v.note = m;
        return v;
    }
    // Containment: only the function's own pages and the stubbed callee entries stay executable, so a jump
    // through a data table into real, unstubbed game code faults (on both sides) instead of running it.
    let mut exec_pages: Vec<usize> = (e.addr & !(PAGE - 1)..e.addr + len).step_by(PAGE).collect();
    for &t in &targets {
        exec_pages.push(t & !(PAGE - 1));
        exec_pages.push((t + 4) & !(PAGE - 1));
    }
    exec_pages.sort_unstable();
    exec_pages.dedup();
    sys::protect_text(&exec_pages, false);
    v.status = Status::Pass;
    let mut notes = Vec::new();
    if info.indirect_calls > 0 {
        notes.push(format!("{} indirect call site(s), stubbed through the vtable page", info.indirect_calls));
    }
    for case in 0..o.cases {
        let seed = splitmix(o.seed ^ (e.addr as u64).wrapping_mul(0x9E37_79B9) ^ ((case as u64) << 40));
        v.cases += 1;
        match run_case(env, e, &info, seed, o.verbose) {
            CaseResult::BothFault => v.both_fault += 1,
            CaseResult::Same => v.ok += 1,
            CaseResult::Differ(d) => {
                v.status = Status::Fail;
                v.diffs = std::iter::once(format!("case {case}, seed {seed:#x} (rerun with --seed {} --cases {})", o.seed, case + 1))
                    .chain(d)
                    .collect();
                v.kinds = kinds_of(&v.diffs);
                break;
            }
        }
    }
    sys::protect_text(&exec_pages, true);
    stubs::remove(&mut env.img, &saved);
    image::reset_watch();
    if v.status == Status::Pass && (v.ok as u64) * 5 < v.cases as u64 {
        v.status = Status::Inconclusive;
        notes.push(format!(
            "only {} of {} cases ran without faulting ({:.0}% fault rate, both sides)",
            v.ok,
            v.cases,
            100.0 * v.both_fault as f64 / v.cases.max(1) as f64
        ));
    }
    v.note = notes.join("; ");
    v
}
