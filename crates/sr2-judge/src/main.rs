//! sr2-judge: the one shared, strict judge for ports. Agents must not edit this crate.
//!
//!   sr2-judge <hex address or port name> [--cases N] [--seed S] [--verbose]
//!   sr2-judge --list
//!   sr2-judge --all [--cases N] [--seed S]
//!
//! The judge maps the game exe by section at 0x400000 (never running anything from it), registers every
//! port without installing it, and for each port runs the ORIGINAL bytes and the port's thunk on the
//! same seeded random worlds, comparing registers, flags-free state, the x87 and SSE state, all memory
//! the run touches and the ordered list of calls to the functions it does not contain.
//!
//! It re-runs itself as a child process created suspended, so that the game's address range can be
//! reserved before the system maps anything there (the judge itself links at 0x10000000).

#![cfg(all(windows, target_arch = "x86"))]
#![allow(static_mut_refs)]

mod analysis;
mod exec;
mod hookva;
mod image;
mod judge;
mod sha;
mod stubs;
mod sys;
mod world;

use dinput8::registry::{self, RegEntry};
use judge::{Env, Opts, Status};

fn usage() -> ! {
    eprintln!(
        "usage: sr2-judge <hex address | port name> [--cases N] [--seed S] [--verbose]\n       sr2-judge --list\n       sr2-judge --all [--cases N] [--seed S]"
    );
    std::process::exit(64);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        usage();
    }
    if std::env::var("SR2J_CHILD").is_err() {
        std::process::exit(parent(&args) as i32);
    }
    std::process::exit(child(&args));
}

// ---- parent: supervisor ---------------------------------------------------------------------------

fn parent(args: &[String]) -> u32 {
    println!("sr2-judge source hash: {}", sha::source_hash());
    let all = args.iter().any(|a| a == "--all");
    if !all {
        return sys::run_child(image::BASE, image::SIZE, &args.join(" "), |l| println!("{l}"));
    }
    // --all: one child runs ports until it finishes or dies; a port that kills it is a FAIL and the
    // next child resumes after it.
    // --from N (parent only): begin at port N, for resuming a long run.
    let mut start = args.iter().position(|a| a == "--from").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(0usize);
    let mut kept: Vec<String> = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a == "--from" {
            skip = true;
        } else {
            kept.push(a.clone());
        }
    }
    let base_args = kept.join(" ");
    let mut start_failures = 0;
    let mut rows: Vec<Row> = Vec::new();
    let mut total = 0usize;
    loop {
        let mut last_begin: Option<(usize, String)> = None;
        let mut last_row: Option<usize> = None;
        let mut done = false;
        let code = sys::run_child(image::BASE, image::SIZE, &format!("{base_args} --start {start}"), |l| {
            if let Some(r) = l.strip_prefix("BEGIN|") {
                let p: Vec<&str> = r.split('|').collect();
                last_begin = Some((p[0].parse().unwrap_or(0), format!("{}|{}", p[1], p[2])));
                if let Some(t) = p.get(3).and_then(|t| t.parse().ok()) {
                    total = t;
                }
            } else if let Some(r) = l.strip_prefix("ROW|") {
                if let Some(row) = Row::parse(r) {
                    println!("[{}/{}] {} {} {}", row.idx + 1, total, row.status, row.addr, row.name);
                    last_row = Some(row.idx);
                    rows.push(row);
                }
                last_begin = None;
            } else if l == "DONE" {
                done = true;
            } else {
                println!("{l}");
            }
        });
        if done && code == 0 {
            break;
        }
        if code == 77 {
            start = last_row.map(|i| i + 1).unwrap_or(start);
            continue;
        }
        match last_begin {
            Some((idx, an)) => {
                let (addr, name) = an.split_once('|').unwrap_or(("?", "?"));
                println!("[{}/{}] FAIL {addr} {name}: the judge process died (exit code {code:#x}) while running it", idx + 1, total);
                rows.push(Row {
                    idx,
                    addr: addr.to_string(),
                    name: name.to_string(),
                    status: "FAIL".into(),
                    ok: 0,
                    cases: 0,
                    both: 0,
                    note: format!("the judge process died with exit code {code:#x}: the port or its original aborted the process"),
                });
                start = idx + 1;
            }
            None if start_failures < 5 => {
                // The system could not start the child (e.g. 0xc0000142): try again.
                start_failures += 1;
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
            None => {
                println!("the judge child exited with code {code:#x} outside a port; stopping");
                break;
            }
        }
    }
    summary(&rows, total);
    0
}

struct Row {
    idx: usize,
    addr: String,
    name: String,
    status: String,
    ok: u32,
    cases: u32,
    both: u32,
    note: String,
}

impl Row {
    fn parse(s: &str) -> Option<Row> {
        let p: Vec<&str> = s.splitn(8, '|').collect();
        if p.len() < 8 {
            return None;
        }
        Some(Row {
            idx: p[0].parse().ok()?,
            addr: p[1].to_string(),
            name: p[2].to_string(),
            status: p[3].to_string(),
            ok: p[4].parse().ok()?,
            cases: p[5].parse().ok()?,
            both: p[6].parse().ok()?,
            note: p[7].to_string(),
        })
    }
}

fn summary(rows: &[Row], total: usize) {
    println!();
    println!("=== summary table ===");
    println!("{:<10} {:<14} {:<48} {:>8} {:>8}  notes", "address", "verdict", "name", "ok/cases", "both-flt");
    for r in rows {
        println!(
            "{:<10} {:<14} {:<48} {:>8} {:>8}  {}",
            r.addr,
            r.status,
            r.name,
            format!("{}/{}", r.ok, r.cases),
            r.both,
            r.note.lines().next().unwrap_or("")
        );
    }
    let count = |s: &str| rows.iter().filter(|r| r.status == s).count();
    println!();
    println!(
        "judged {} of {} registered ports: PASS {}  FAIL {}  INCONCLUSIVE {}  ERROR {}",
        rows.len(),
        total,
        count("PASS"),
        count("FAIL"),
        count("INCONCLUSIVE"),
        count("ERROR")
    );
    let mut by_kind: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for r in rows.iter().filter(|r| r.status == "FAIL") {
        let k = r.note.strip_prefix('[').and_then(|n| n.split_once(']')).map(|(k, _)| k.to_string()).unwrap_or_else(|| "died".into());
        *by_kind.entry(k).or_default() += 1;
    }
    println!("FAIL by what differs: {}", by_kind.iter().map(|(k, n)| format!("{k}: {n}")).collect::<Vec<_>>().join("; "));
    for r in rows.iter().filter(|r| r.status == "FAIL") {
        println!("FAIL {} {}: {}", r.addr, r.name, r.note.replace("\\n", " | "));
    }
}

// ---- child: the judge -----------------------------------------------------------------------------

struct Cli {
    target: Option<String>,
    list: bool,
    all: bool,
    cases: Option<u32>,
    seed: u64,
    verbose: bool,
    start: usize,
}

fn parse_cli(args: &[String]) -> Cli {
    let mut c = Cli { target: None, list: false, all: false, cases: None, seed: 1, verbose: false, start: 0 };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--list" => c.list = true,
            "--all" => c.all = true,
            "--verbose" => c.verbose = true,
            "--cases" => {
                i += 1;
                c.cases = args.get(i).and_then(|s| s.parse().ok());
            }
            "--seed" => {
                i += 1;
                c.seed = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(1);
            }
            "--start" => {
                i += 1;
                c.start = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            a if a.starts_with("--") => usage(),
            a => c.target = Some(a.to_string()),
        }
        i += 1;
    }
    c
}

fn parse_addr(s: &str) -> Option<usize> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X");
    usize::from_str_radix(t, 16).ok()
}

fn child(args: &[String]) -> i32 {
    let cli = parse_cli(args);
    let mut env = Env::new();
    // Register every port without installing it. Executable allocations fail meanwhile, which keeps
    // the ports' shadow copies of the originals switched off.
    if !hookva::set_blocking(true) {
        println!("ERROR: could not patch VirtualAlloc; shadows would stay live");
        return 70;
    }
    let mut entries: Vec<RegEntry> = registry::collect();
    hookva::set_blocking(false);
    // A module may have patched the mapped image directly; start from the exe's bytes.
    env.img.restore_all();
    entries.sort_by_key(|e| e.addr);

    if cli.list {
        for e in &entries {
            println!(
                "{:#010x} {:<48} {:>5} bytes{}{}",
                e.addr,
                e.name,
                e.body.len(),
                if e.excluded.is_some() { "  [EXCLUDED]" } else { "" },
                if e.held { "  [held back, not installed]" } else { "" }
            );
        }
        println!("{} registered ports", entries.len());
        return 0;
    }

    let cases = cli.cases.unwrap_or(if cli.all { 40 } else { 200 });
    let opts = Opts { cases, seed: cli.seed, verbose: cli.verbose };

    if cli.all {
        let list: Vec<&RegEntry> = entries.iter().collect();
        let judged: Vec<(usize, &RegEntry)> =
            list.iter().enumerate().filter(|(_, e)| e.excluded.is_none()).map(|(i, e)| (i, *e)).collect();
        let total = judged.len();
        for (n, (_, e)) in judged.iter().enumerate().skip(cli.start) {
            println!("BEGIN|{n}|{:#x}|{}|{total}", e.addr, e.name);
            let v = judge::judge(&mut env, e, &opts);
            println!(
                "ROW|{n}|{:#x}|{}|{}|{}|{}|{}|{}",
                e.addr,
                e.name,
                v.status.word(),
                v.ok,
                v.cases,
                v.both_fault,
                format!("{}{}{}{}", if v.kinds.is_empty() { String::new() } else { format!("[{}] ", v.kinds.join(",")) }, v.note, if v.note.is_empty() || v.diffs.is_empty() { "" } else { " ; " }, v.diffs.join(" \\n ").replace('\n', " "))
            );
            if sys::PANICKED.load(std::sync::atomic::Ordering::SeqCst) {
                return 77; // a panic was turned into a fault; restart before another one aborts
            }
        }
        println!("DONE");
        return 0;
    }

    let Some(t) = cli.target.clone() else { usage() };
    let by_addr = parse_addr(&t);
    let found: Vec<&RegEntry> = entries
        .iter()
        .filter(|e| e.name == t || Some(e.addr) == by_addr || e.name.starts_with(&format!("{t}~")))
        .collect();
    if found.is_empty() {
        println!("no registered port matches {t:?} (use --list)");
        return 3;
    }
    let mut worst = 0;
    for e in found {
        println!("judging {} @ {:#x} ({} bytes), {} cases, seed {}{}", e.name, e.addr, e.body.len(), opts.cases, opts.seed,
            e.excluded.map(|w| format!("  [EXCLUDED in portlib: {w}]")).unwrap_or_default());
        let v = judge::judge(&mut env, e, &opts);
        println!(
            "{} {:#x} {}  cases={} ran_clean={} both_faulted={}{}{}",
            v.status.word(),
            e.addr,
            e.name,
            v.cases,
            v.ok,
            v.both_fault,
            if v.kinds.is_empty() { String::new() } else { format!("  differs in: {}", v.kinds.join(", ")) },
            if v.note.is_empty() { String::new() } else { format!("  ({})", v.note) }
        );
        for d in &v.diffs {
            println!("  {d}");
        }
        worst = worst.max(match v.status {
            Status::Pass => 0,
            Status::Fail => 1,
            Status::Inconclusive => 2,
            Status::Error => 4,
        });
    }
    worst
}
