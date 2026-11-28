//! Static analysis of the original bytes: which functions the body calls or jumps to outside itself,
//! how each of those takes arguments and what it clobbers, and which globals the body touches.

use std::collections::{BTreeMap, HashMap, HashSet};

use iced_x86::{
    Decoder, DecoderOptions, FlowControl, Instruction, InstructionInfoFactory, Mnemonic, OpAccess, OpKind, Register,
};

use crate::image::{self, Image};

/// Register numbers in pushad order.
#[allow(dead_code)]
pub const EAX: usize = 0;
pub const ECX: usize = 1;
pub const EDX: usize = 2;
#[allow(dead_code)]
pub const EBX: usize = 3;
pub const ESP: usize = 4;
#[allow(dead_code)]
pub const EBP: usize = 5;
#[allow(dead_code)]
pub const ESI: usize = 6;
#[allow(dead_code)]
pub const EDI: usize = 7;
pub const REG_NAMES: [&str; 8] = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi"];
/// Registers a nested callee may read directly (custom conventions).
const ARGREGS: u8 = 0b1100_0111;

#[derive(Clone, Debug)]
pub struct Callee {
    pub target: usize,
    pub ret_n: u32,
    /// Stack argument dwords that are compared at each call.
    pub nargs: usize,
    pub float_st0: bool,
    pub xmm0: bool,
    /// Registers compared at each call (the callee reads them before writing them).
    pub live_in: u8,
    /// EAX / ECX / EDX that the stub overwrites with seeded values (the callee clobbers them).
    pub clobber: u8,
    #[allow(dead_code)]
    pub via_jmp: bool,
    /// The shared stub for indirect calls (vtables, function pointers): not patched into the image;
    /// the world's vtable page points at it.
    pub indirect: bool,
    /// The callee consumes ST0 (`_ftol2` and similar): the stub pops it.
    pub pop_st0: bool,
    /// The shared stub for calls through the exe's import table (Windows APIs, CRT): reads no register.
    pub import: bool,
    /// The callee fills an output structure through stack argument `.0`, `.1` bytes long: the stub writes
    /// seeded values there.
    pub out_arg: Option<(u8, u16)>,
}

#[derive(Default, Debug)]
pub struct FnInfo {
    pub callees: Vec<Callee>,
    /// Absolute data operands in the body: (address, size in bytes).
    pub globals: Vec<(usize, usize)>,
    pub own_ret_n: u32,
    pub indirect_calls: usize,
    /// Indirect call sites: (return address, bytes the callee pops).
    pub indirect_sites: Vec<(u32, u16)>,
    /// How many of those call through the import table.
    pub import_calls: usize,
}

#[derive(Clone, Copy, Default, Debug)]
struct Summary {
    ret_n: u32,
    live_in: u8,
    written: u8,
    has_call: bool,
}

#[derive(Default, Clone, Debug)]
pub struct Override {
    pub ret_n: Option<u32>,
    pub args: Option<usize>,
    pub float: Option<bool>,
    pub xmm0: Option<bool>,
    pub preserve: Option<u8>,
    pub clobber: Option<u8>,
    pub live_in: Option<u8>,
    pub pop_st0: Option<bool>,
    pub out_arg: Option<u8>,
    pub out_size: Option<u16>,
}

pub struct Analyzer {
    cache: HashMap<usize, Summary>,
    pub overrides: HashMap<usize, Override>,
}

fn reg_mask(names: &str) -> u8 {
    let mut m = 0u8;
    for n in names.split([',', '[', ']', '"', ' ']).filter(|s| !s.is_empty()) {
        if let Some(i) = REG_NAMES.iter().position(|r| r.eq_ignore_ascii_case(n)) {
            m |= 1 << i;
        }
    }
    m
}

fn parse_num(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// callees.toml: `[0xADDR]` sections with ret_n, args, float, xmm0, preserve, clobber, live_in.
pub fn load_overrides() -> HashMap<usize, Override> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("callees.toml");
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut map = HashMap::new();
    let mut cur: Option<usize> = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(h) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            cur = parse_num(h).map(|v| v as usize);
            if let Some(a) = cur {
                map.entry(a).or_insert_with(Override::default);
            }
            continue;
        }
        let (Some(a), Some((k, v))) = (cur, line.split_once('=')) else { continue };
        let (k, v) = (k.trim(), v.trim());
        let o = map.get_mut(&a).unwrap();
        match k {
            "ret_n" => o.ret_n = parse_num(v).map(|x| x as u32),
            "args" => o.args = parse_num(v).map(|x| x as usize),
            "float" => o.float = Some(v == "true"),
            "xmm0" => o.xmm0 = Some(v == "true"),
            "preserve" => o.preserve = Some(reg_mask(v)),
            "clobber" => o.clobber = Some(reg_mask(v)),
            "live_in" => o.live_in = Some(reg_mask(v)),
            "pop_st0" => o.pop_st0 = Some(v == "true"),
            "out_arg" => o.out_arg = parse_num(v).map(|x| x as u8),
            "out_size" => o.out_size = parse_num(v).map(|x| x as u16),
            _ => {}
        }
    }
    map
}

fn gpr_bit(r: Register) -> Option<(u8, bool)> {
    if !r.is_gpr() {
        return None;
    }
    let full = r.full_register32();
    Some((1u8 << full.number(), r.is_gpr32()))
}

/// (reads, sure writes, any writes) of general registers (ESP ignored).
fn reg_effects(f: &mut InstructionInfoFactory, i: &Instruction) -> (u8, u8, u8) {
    let (mut reads, mut kill, mut any) = (0u8, 0u8, 0u8);
    let m = i.mnemonic();
    // push reg saves a callee-saved register: not a use of its value.
    let skip_read = m == Mnemonic::Push && i.op0_kind() == OpKind::Register;
    let zero_idiom = matches!(m, Mnemonic::Xor | Mnemonic::Sub)
        && i.op0_kind() == OpKind::Register
        && i.op1_kind() == OpKind::Register
        && i.op0_register() == i.op1_register();
    let info = f.info(i);
    for ur in info.used_registers() {
        let Some((bit, full)) = gpr_bit(ur.register()) else { continue };
        if bit == 1 << ESP {
            continue;
        }
        match ur.access() {
            OpAccess::Read | OpAccess::CondRead => {
                if !skip_read && !zero_idiom {
                    reads |= bit
                }
            }
            OpAccess::Write => {
                any |= bit;
                if full {
                    kill |= bit;
                } else {
                    reads |= bit; // a partial write keeps the rest of the register
                }
            }
            OpAccess::CondWrite => any |= bit,
            OpAccess::ReadWrite | OpAccess::ReadCondWrite => {
                if !zero_idiom {
                    reads |= bit;
                }
                any |= bit;
                if full && zero_idiom {
                    kill |= bit;
                }
            }
            _ => {}
        }
    }
    (reads, kill, any)
}

impl Analyzer {
fn summarize(&mut self, img: &Image, entry: usize, depth: u32) -> Summary {
    if let Some(s) = self.cache.get(&entry) {
        return *s;
    }
    // Placeholder while analysing, so a recursive callee ends the cycle.
    self.cache.insert(entry, Summary::default());
    let mut sum = Summary::default();
    let mut factory = InstructionInfoFactory::new();
    let mut work: Vec<(usize, u8)> = vec![(entry, 0)];
    let mut seen: HashSet<(usize, u8)> = HashSet::new();
    let mut budget = 6000i32;
    let mut rets: Vec<u32> = Vec::new();
    while let Some((start, w0)) = work.pop() {
        if !image::in_text(start) || !seen.insert((start, w0)) || budget <= 0 {
            continue;
        }
        let off = start - image::BASE;
        let end = (off + 0x300).min(image::TEXT_HI - image::BASE);
        let mut dec = Decoder::with_ip(32, &img.pristine[off..end], start as u64, DecoderOptions::NONE);
        let mut w = w0;
        let mut instr = Instruction::default();
        let mut ended = false;
        while dec.can_decode() && budget > 0 {
            dec.decode_out(&mut instr);
            budget -= 1;
            if instr.is_invalid() {
                ended = true;
                break;
            }
            let (reads, kill, any) = reg_effects(&mut factory, &instr);
            sum.live_in |= reads & !w;
            w |= kill;
            sum.written |= any;
            match instr.flow_control() {
                FlowControl::Return => {
                    rets.push(if instr.op_count() == 1 { instr.immediate16() as u32 } else { 0 });
                    ended = true;
                    break;
                }
                FlowControl::UnconditionalBranch => {
                    if instr.op0_kind() == OpKind::NearBranch32 {
                        work.push((instr.near_branch_target() as usize, w));
                    }
                    ended = true;
                    break;
                }
                FlowControl::ConditionalBranch => {
                    if instr.op0_kind() == OpKind::NearBranch32 {
                        work.push((instr.near_branch_target() as usize, w));
                    }
                }
                FlowControl::Call => {
                    // What the nested callee reads on entry is read here too (pass-through registers).
                    sum.has_call = true;
                    let mut clob = 0b111u8;
                    // Pass-through reads count one level deep only, and never for EBX / EBP (callee-saved
                    // in every MSVC convention): deeper unions made almost every register look live.
                    if instr.op0_kind() == OpKind::NearBranch32 && depth < 5 && image::in_text(instr.near_branch_target() as usize) {
                        let b = self.summarize(img, instr.near_branch_target() as usize, depth + 1);
                        if depth == 0 {
                            sum.live_in |= b.live_in & ARGREGS & !w;
                        }
                        clob = (b.written | if b.has_call { 7 } else { 0 }) & 0b111;
                    } else if depth == 0 {
                        sum.live_in |= (1 << ECX | 1 << EAX | 1 << EDX) & !w;
                    }
                    w |= clob;
                    sum.written |= clob;
                }
                FlowControl::IndirectCall => {
                    sum.has_call = true;
                    sum.live_in |= (1 << ECX) & !w; // thiscall is the usual shape
                    w |= 0b111;
                    sum.written |= 0b111;
                }
                FlowControl::IndirectBranch => {
                    ended = true;
                    break;
                }
                FlowControl::Exception | FlowControl::Interrupt => {}
                _ => {}
            }
        }
        if !ended && budget > 0 {
            work.push((instr.next_ip() as usize, w));
        }
    }
    // Most common ret operand.
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for r in &rets {
        *counts.entry(*r).or_default() += 1;
    }
    sum.ret_n = counts.iter().max_by_key(|&(n, c)| (*c, std::cmp::Reverse(*n))).map(|(n, _)| *n).unwrap_or(0);
    self.cache.insert(entry, sum);
    sum
}
}

/// x87 instructions that do not use the value on ST0 (loads and control).
fn x87_neutral(name: &str) -> bool {
    name.starts_with("Fld")
        || name.starts_with("Fild")
        || name.starts_with("Fbld")
        || matches!(name, "Fnstcw" | "Fnstsw" | "Fxch" | "Fwait" | "Fnclex" | "Fninit" | "Fnop" | "Ffree" | "Fincstp" | "Fdecstp")
}

impl Analyzer {
    /// How many bytes the indirect callee at `instrs[idx]` pops, from the stack balance: the function returns with
    /// its stack back at the entry depth, so (depth at the call) + (linear change up to the next epilogue) - (bytes
    /// the epilogue removes) is what the callee took. Depth is tracked along the instructions in order from the
    /// start: push / pop, add / sub esp, direct calls pop their known ret N. None when this cannot be trusted:
    /// another indirect call or a frame-pointer epilogue in the way, or a negative result.
    fn pop_by_epilogue(&mut self, img: &Image, instrs: &[Instruction], idx: usize) -> Option<u16> {
        // Bytes one instruction adds to the stack depth (None: unknown effect).
        let delta = |this: &mut Analyzer, i: &Instruction| -> Option<i64> {
            let reg0 = i.op_count() > 0 && i.op0_kind() == OpKind::Register;
            match i.mnemonic() {
                Mnemonic::Push => Some(4),
                Mnemonic::Pushad => Some(32),
                Mnemonic::Pushfd => Some(4),
                Mnemonic::Pop => Some(-4),
                Mnemonic::Popad => Some(-32),
                Mnemonic::Popfd => Some(-4),
                Mnemonic::Sub if reg0 && i.op0_register() == Register::ESP => {
                    if i.op1_kind() == OpKind::Register { None } else { Some(i.immediate(1) as i32 as i64) }
                }
                Mnemonic::Add if reg0 && i.op0_register() == Register::ESP => {
                    if i.op1_kind() == OpKind::Register { None } else { Some(-(i.immediate(1) as i32 as i64)) }
                }
                Mnemonic::Mov | Mnemonic::Lea | Mnemonic::And if reg0 && i.op0_register() == Register::ESP => None,
                Mnemonic::Leave => None,
                Mnemonic::Call => match i.flow_control() {
                    FlowControl::Call if i.op0_kind() == OpKind::NearBranch32 => {
                        let t = i.near_branch_target() as usize;
                        if image::in_text(t) {
                            let rn = this.overrides.get(&t).and_then(|o| o.ret_n).unwrap_or_else(|| this.summarize(img, t, 0).ret_n);
                            Some(-(rn as i64))
                        } else {
                            None
                        }
                    }
                    _ => None, // another indirect call
                },
                _ => Some(0),
            }
        };
        // Depth at the call: walk from the start. After a ret, the code that follows is another path that
        // runs at the body depth, i.e. the depth before that epilogue started.
        let mut depth: i64 = 0;
        let mut epi_start_depth: i64 = 0;
        for (k, i) in instrs[..idx].iter().enumerate() {
            if i.flow_control() == FlowControl::Return {
                depth = epi_start_depth;
                continue;
            }
            // Remember the depth before a run of pops / add esp that ends in a ret (an epilogue).
            let next_is_epi = instrs[k..].iter().take_while(|n| matches!(n.mnemonic(), Mnemonic::Pop | Mnemonic::Add)).count() > 0
                && instrs[k..].iter().skip_while(|n| matches!(n.mnemonic(), Mnemonic::Pop | Mnemonic::Add)).next().is_some_and(|n| n.flow_control() == FlowControl::Return);
            if next_is_epi && (k == 0 || !matches!(instrs[k - 1].mnemonic(), Mnemonic::Pop | Mnemonic::Add)) {
                epi_start_depth = depth;
            }
            if i.flow_control() == FlowControl::IndirectCall {
                return None; // an earlier indirect call: its pop is unknown here
            }
            depth += delta(self, i)?;
        }
        // From after the call to the next exit: the change in depth, then what the epilogue removes. An exit is
        // a ret, or a tail jmp out of the function (the stack is back at the entry depth there too).
        let lo = instrs.first()?.ip() as usize;
        let hi = instrs.last()?.next_ip() as usize;
        let is_exit = |i: &Instruction| {
            i.flow_control() == FlowControl::Return
                || (i.flow_control() == FlowControl::UnconditionalBranch
                    && i.op0_kind() == OpKind::NearBranch32
                    && !(lo..hi).contains(&(i.near_branch_target() as usize)))
        };
        let ret_at = instrs.iter().enumerate().skip(idx + 1).find(|(_, i)| is_exit(i))?.0;
        let mut epi = ret_at;
        while epi > idx + 1 && matches!(instrs[epi - 1].mnemonic(), Mnemonic::Pop | Mnemonic::Add) {
            epi -= 1;
        }
        let mut after: i64 = 0;
        for i in &instrs[idx + 1..epi] {
            if matches!(i.flow_control(), FlowControl::IndirectCall | FlowControl::Return) {
                return None;
            }
            // A jump inside the function: the depth along the jump's path may differ, so stop trusting it.
            if i.flow_control() == FlowControl::UnconditionalBranch {
                return None;
            }
            after += delta(self, i)?;
        }
        let mut removed: i64 = 0;
        for i in &instrs[epi..ret_at] {
            removed -= delta(self, i)?;
        }
        let pop = depth + after - removed;
        if (0..=256).contains(&pop) && pop % 4 == 0 { Some(pop as u16) } else { None }
    }

    pub fn new() -> Analyzer {
        Analyzer { cache: HashMap::new(), overrides: load_overrides() }
    }

    pub fn analyze(&mut self, img: &Image, addr: usize, body: &[u8]) -> FnInfo {
        let mut info = FnInfo::default();
        let mut dec = Decoder::with_ip(32, body, addr as u64, DecoderOptions::NONE);
        let mut instrs: Vec<Instruction> = Vec::new();
        let mut instr = Instruction::default();
        while dec.can_decode() {
            dec.decode_out(&mut instr);
            if instr.is_invalid() {
                continue; // data inside the body
            }
            instrs.push(instr);
        }
        let lo = addr;
        let hi = addr + body.len();
        #[derive(Default)]
        struct Ext {
            is_call: bool,
            is_jmp: bool,
            float: bool,
            cleanup: usize,
        }
        let mut ext: BTreeMap<usize, Ext> = BTreeMap::new();
        let mut own_rets: BTreeMap<u32, usize> = BTreeMap::new();
        let mut factory = InstructionInfoFactory::new();
        let mut globals: BTreeMap<usize, usize> = BTreeMap::new();
        for (idx, ins) in instrs.iter().enumerate() {
            match ins.flow_control() {
                FlowControl::Return => {
                    let n = if ins.op_count() == 1 { ins.immediate16() as u32 } else { 0 };
                    *own_rets.entry(n).or_default() += 1;
                }
                FlowControl::IndirectCall => {
                    info.indirect_calls += 1;
                    // Stack cleanup: a cdecl site cleans up after the call; otherwise the callee pops what was
                    // pushed for it (counted back to the previous call or ESP adjustment).
                    let mut pop = 0u16;
                    let cdecl = instrs.get(idx + 1).is_some_and(|n| {
                        n.mnemonic() == Mnemonic::Add && n.op0_kind() == OpKind::Register && n.op0_register() == Register::ESP
                    });
                    if !cdecl {
                        for p in instrs[..idx].iter().rev() {
                            if p.flow_control() != FlowControl::Next {
                                break;
                            }
                            match p.mnemonic() {
                                Mnemonic::Push => pop += 4,
                                Mnemonic::Pushad => pop += 32,
                                _ => {
                                    if p.op_count() > 0 && p.op0_kind() == OpKind::Register && p.op0_register() == Register::ESP {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    // Prefer the epilogue balance, which is exact on straight-line code: the guess above also
                    // counts registers the function itself saved before the call.
                    if !cdecl {
                        if let Some(p2) = self.pop_by_epilogue(img, &instrs, idx) {
                            pop = p2;
                        }
                    }
                    info.indirect_sites.push((ins.next_ip() as u32, pop));
                    if ins.op0_kind() == OpKind::Memory
                        && ins.memory_base() == Register::None
                        && ins.memory_index() == Register::None
                        && (0xDB_8000..0xDB_8440).contains(&(ins.memory_displacement32() as usize))
                    {
                        info.import_calls += 1;
                    }
                }
                FlowControl::Call | FlowControl::UnconditionalBranch | FlowControl::ConditionalBranch => {
                    if ins.op0_kind() == OpKind::NearBranch32 {
                        let t = ins.near_branch_target() as usize;
                        if t >= lo && t < hi {
                            // internal
                        } else if image::in_text(t) {
                            let e = ext.entry(t).or_default();
                            if ins.flow_control() == FlowControl::Call {
                                e.is_call = true;
                                // cdecl cleanup and a result consumed from ST0, read off the call site
                                if let Some(n) = instrs.get(idx + 1) {
                                    if n.mnemonic() == Mnemonic::Add
                                        && n.op0_kind() == OpKind::Register
                                        && n.op0_register() == Register::ESP
                                        && n.op1_kind() != OpKind::Register
                                    {
                                        e.cleanup = e.cleanup.max(n.immediate(1) as usize / 4);
                                    }
                                }
                                for n in instrs.iter().skip(idx + 1).take(6) {
                                    if n.flow_control() != FlowControl::Next {
                                        break;
                                    }
                                    let name = format!("{:?}", n.mnemonic());
                                    if name.starts_with('F') {
                                        if !x87_neutral(&name) {
                                            e.float = true;
                                            break;
                                        } else if name.starts_with("Fld") || name.starts_with("Fild") {
                                            break;
                                        }
                                    }
                                }
                            } else {
                                e.is_jmp = true;
                            }
                        }
                    }
                }
                _ => {}
            }
            // absolute data operands
            if instr_touches_data(&mut factory, ins) {
                let inf = factory.info(ins);
                for um in inf.used_memory() {
                    if um.base() != Register::None || um.index() != Register::None {
                        continue;
                    }
                    if matches!(um.segment(), Register::FS | Register::GS) {
                        continue;
                    }
                    if matches!(um.access(), OpAccess::None | OpAccess::NoMemAccess) {
                        continue;
                    }
                    let a = um.displacement() as u32 as usize;
                    if (image::DATA_LO..image::DATA_HI).contains(&a) {
                        let sz = um.memory_size().size().clamp(1, 16);
                        let e = globals.entry(a).or_insert(0);
                        *e = (*e).max(sz);
                    }
                }
            }
        }
        info.own_ret_n = own_rets.iter().max_by_key(|&(n, c)| (*c, std::cmp::Reverse(*n))).map(|(n, _)| *n).unwrap_or(0);
        info.globals = globals.into_iter().take(96).collect();
        for (t, e) in ext {
            let s = self.summarize(img, t, 0);
            let ov = self.overrides.get(&t).cloned().unwrap_or_default();
            let ret_n = ov.ret_n.unwrap_or(s.ret_n);
            let mut nargs = if ret_n > 0 { (ret_n / 4) as usize } else { e.cleanup };
            if let Some(a) = ov.args {
                nargs = a;
            }
            let mut clobber = (s.written | if s.has_call { 7 } else { 0 }) & 0b111;
            if let Some(p) = ov.preserve {
                clobber &= !p;
            }
            if let Some(c) = ov.clobber {
                clobber |= c & 0b111;
            }
            let mut live_in = s.live_in & !(1 << ESP);
            if let Some(l) = ov.live_in {
                live_in = l;
            }
            info.callees.push(Callee {
                target: t,
                ret_n,
                nargs,
                float_st0: ov.float.unwrap_or(e.float),
                xmm0: ov.xmm0.unwrap_or(false),
                live_in,
                clobber,
                via_jmp: e.is_jmp && !e.is_call,
                indirect: false,
                import: false,
                pop_st0: ov.pop_st0.unwrap_or(false),
                out_arg: ov.out_arg.map(|a| (a, ov.out_size.unwrap_or(4))),
            });
        }
        if !info.indirect_sites.is_empty() {
            let most = info.indirect_sites.iter().map(|s| s.1 as usize / 4).max().unwrap_or(0);
            info.callees.push(Callee {
                target: 0,
                ret_n: 0,
                nargs: most,
                float_st0: false,
                xmm0: false,
                live_in: 1 << ECX,
                clobber: 0b111,
                via_jmp: false,
                indirect: true,
                import: false,
                pop_st0: false,
                out_arg: None,
            });
            if info.import_calls > 0 {
                // Imports are stdcall / cdecl Windows and CRT functions: no register argument.
                info.callees.push(Callee {
                    target: 0,
                    ret_n: 0,
                    nargs: most,
                    float_st0: false,
                    xmm0: false,
                    live_in: 0,
                    clobber: 0b111,
                    via_jmp: false,
                    indirect: true,
                    import: true,
                    pop_st0: false,
                    out_arg: None,
                });
            }
        }
        info
    }
}

fn instr_touches_data(_f: &mut InstructionInfoFactory, i: &Instruction) -> bool {
    (0..i.op_count()).any(|k| i.op_kind(k) == OpKind::Memory) && i.memory_base() == Register::None && i.memory_index() == Register::None
}
