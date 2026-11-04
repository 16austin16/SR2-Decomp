//! Offline differential tests for port_physics.rs: the original machine code against the ports
//! (through their real thunks) on random worlds. See `difftest` for the rig and how to run.

use super::difftest::*;
use super::*;

/// Callee keys: game addresses nobody else uses, for functions the tests stand in for.
const K1: usize = 0x00F0_0010;
const K2: usize = 0x00F0_0020;

const ALL: u8 = EAX | ECX | EDX | ESI | EDI;

/// Map the game data the physics ports touch.
fn begin() {
    reset();
    map_arena();
    map_data(g(0x0214_9C64), 0x1_0000); // handle table
    map_data(g(0x0169_3F60), 0x6000); // rigid slots
    map_data(g(0x0168_E970), 0x6000); // rigid free stack
    map_data(g(0x0252_7D00), 0x200); // world state words
    map_data(g(0x0169_9550), 0x4000); // filter list
    map_data(g(0x00E8_EC00), 0x100); // rigid sequence
    map_data(g(0x028F_25C0), 0x1_D500); // body table
    map_data(g(0x0174_3D80), 0x4000); // pair table
    map_data(g(0x0176_95F0), 0x2800); // constraint slots
    map_data(g(0x0279_7600), 0x200); // ragdoll part params
    map_data(g(0x00E8_7900), 0x200);
    map_data(g(0x00DD_3200), 0x200);
}

/// Handle table: all entries cleared, then `n` objects of `size` bytes with a type from `types`.
/// Returns (handle, object) pairs; some stale (+0x44 differs).
fn objects(rng: &mut Rng, ar: &mut Arena, n: usize, size: usize, types: &[u32]) -> Vec<(u32, u32)> {
    zero(HANDLES, 0x1_0000);
    let mut v = Vec::new();
    for _ in 0..n {
        let idx = rng.below(4096);
        let h = ((rng.below(0xFFFE) + 1) << 12) | idx;
        let o = ar.alloc(size);
        for k in 0..size / 4 {
            unsafe { put(o + k * 4, rng.u32()) };
        }
        let stale = rng.one_in(5);
        unsafe {
            put(o + 0x44, if stale { h ^ (1 << (12 + rng.below(8))) } else { h });
            put(o + 0x4C, rng.pick(types));
            put(HANDLES + idx as usize * 16, o as u32);
        }
        v.push((h, o as u32));
    }
    v
}

/// A handle to feed a lookup: a live one, a stale one, 0, or junk.
fn some_handle(rng: &mut Rng, objs: &[(u32, u32)]) -> u32 {
    match rng.below(8) {
        0 => 0,
        1 => rng.u32(),
        2 | 3 => objs[rng.below(objs.len() as u32) as usize].0 ^ (1 << 20),
        _ => objs[rng.below(objs.len() as u32) as usize].0,
    }
}

fn run(name: &str, orig: usize, port: usize, c: &Case) {
    if let Err(e) = check(orig, port, c) {
        panic!("{name}: {e}\n  regs {:x?} args {:x?} seed {:#x}", c.regs, c.args, c.seed);
    }
}

const ITERS: u32 = 60;
const CASES: u32 = 40;

#[test]
fn port_physics_handle_lookups() {
    let _g = lock();
    begin();
    set_callees(vec![callee(0x0096_0D60, 0, 0, ALL, Ret::Any)]);
    let (o1, o2, o3, o4) = (orig_HANDLE_TO_OBJECT(), orig_PLAYER_HUMAN(), orig_PTR_PLAYER_HUMAN(), orig_SEAT_BY_HANDLE());
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for it in 0..ITERS {
        randomize(&mut rng);
        let mut ar = Arena::new();
        let objs = objects(&mut rng, &mut ar, 24, 0x8C00, &[1, 2, 3, 5, 5, 5, 7]);
        for &(_, o) in &objs {
            unsafe {
                putb(o as usize + 0x3CC, rng.pick(&[0, 1, 1, 2]));
                putb(o as usize + 0x84D8, rng.pick(&[0, 1, 1]));
                put(o as usize + 0x6CE8, rng.pick(&[0, 1, 3, 8, 9, 0x8000_0000]));
            }
        }
        let cell = ar.alloc(16);
        for k in 0..CASES {
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            let h = some_handle(&mut rng, &objs);
            c.regs.ecx = h;
            c.regs.edx = rng.pick(&[0, 1, 2, 5, 7, 8, 9, 0xFFFF_FFFF, 100]);
            run("handle_to_object", o1, handle_to_object_thunk as usize, &c);
            run("handle_to_player_human", o2, player_human_thunk as usize, &c);
            run("vehicle_seat_by_handle", o4, seat_by_handle_thunk as usize, &c);
            unsafe { put(cell, h) };
            let mut c2 = Case::new(it as u64 * 1000 + k as u64);
            c2.regs.eax = cell as u32;
            run("handle_ptr_to_player_human", o3, ptr_player_human_thunk as usize, &c2);
        }
    }
}

#[test]
fn port_physics_get_property_and_rigid() {
    let _g = lock();
    begin();
    set_callees(vec![callee(K1, 1, 4, ALL, Ret::Any)]);
    let o_prop = orig_GET_PROPERTY();
    let _ = orig_FILTER_CANCEL();
    let (o_alloc, o_body, o_free) = (orig_RIGID_ALLOC(), orig_RIGID_TO_BODY(), orig_RIGID_FREE());
    let mut rng = Rng(0xDEAD_BEEF_1234_0001);
    for it in 0..ITERS {
        randomize(&mut rng);
        let mut ar = Arena::new();
        let ents: Vec<u32> = (0..30)
            .map(|_| {
                let e = ar.alloc(0x40) as u32;
                let n = rng.pick(&[0i32, 1, 2, 5, -1, 3]);
                let arr = ar.alloc(16 * 8 + 16) as u32;
                for k in 0..(16 * 8) / 4 {
                    unsafe { put(arr as usize + k * 4, rng.mix(&[1, 2, 5, 8, 0xE])) };
                }
                let vt = ar.alloc(16);
                unsafe {
                    put(vt, stub_of(K1) as u32);
                    put(e as usize, vt as u32);
                    put(e as usize + 0x74, arr);
                    put(e as usize + 0x78, n as u32);
                    puth(e as usize + 4, rng.pick(&[0, 1, 2]));
                    puth(e as usize + 6, rng.pick(&[0, 1, 2, 3]));
                    put(e as usize + 0x2C, rng.u32());
                }
                e
            })
            .collect();
        let out = ar.alloc(16) as u32;
        for k in 0..CASES {
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.ecx = rng.pick(&ents);
            c.regs.eax = out;
            c.args = vec![rng.pick(&[1, 2, 5, 8, 8, 8, 0xE])];
            unsafe {
                put(out as usize, 0x1111_1111);
                put(out as usize + 4, 0x2222_2222);
            }
            run("get_property", o_prop, get_property_thunk as usize, &c);

            // rigid handle alloc
            unsafe {
                put(RIGID_USED, rng.pick(&[0, 1, 5, 0x157B, 0x157C, 0x157D, 0x1000, 100]));
                puth(RIGID_SEQ, rng.pick(&[0, 1, 0xFFFE, 0xFFFF, 0x1234, 2]));
            }
            let c = Case::new(k as u64);
            run("rigid_alloc", o_alloc, rigid_alloc_thunk as usize, &c);

            // slots: bodies whose property 8 may equal the handle
            let idx = rng.below(0x157C);
            let h = (rng.below(0xFFFF) << 16) | idx;
            let body = ar.alloc(0x40) as u32;
            let arr = ar.alloc(64) as u32;
            let vt = ar.alloc(16);
            let prop = if rng.one_in(3) { rng.u32() } else { h };
            unsafe {
                put(body as usize + 0x74, arr);
                put(body as usize + 0x78, 2);
                put(arr as usize, 8);
                put(arr as usize + 8, prop);
                put(arr as usize + 12, 0);
                put(arr as usize + 16, 1);
                put(arr as usize + 24, 7);
                put(body as usize, vt as u32);
                put(vt, stub_of(K1) as u32);
                puth(body as usize + 4, rng.pick(&[0, 1]));
                puth(body as usize + 6, rng.pick(&[1, 2]));
                put(RIGID_SLOTS + idx as usize * 4, if rng.one_in(5) { 0 } else { body });
            }
            let mut c = Case::new(k as u64);
            c.regs.edi = h;
            run("rigid_to_body", o_body, rigid_to_body_thunk as usize, &c);

            // free (the slot must hold a body)
            unsafe {
                put(RIGID_SLOTS + idx as usize * 4, body);
                put(RIGID_USED, rng.pick(&[1, 5, 100]));
                put(FILTER_COUNT, rng.below(6));
                for i in 0..6 {
                    put(FILTER_LIST + i * 4, rng.mix(&[0, body]));
                }
                putb(DIRTY_BYTE, 0);
            }
            let mut c = Case::new(k as u64);
            c.out = 0;
            c.args = vec![(rng.u32() & 0xFFFF_0000) | idx];
            run("rigid_free", o_free, rigid_free_thunk as usize, &c);
        }
    }
}

#[test]
fn port_physics_filter_list() {
    let _g = lock();
    begin();
    set_callees(vec![callee(0x00D5_CC40, 3, 12, ALL, Ret::Any)]);
    let (o_def, o_can) = (orig_FILTER_DEFER(), orig_FILTER_CANCEL());
    let (o_set, o_tog) = (orig_SET_FILTER_INFO(), orig_COLLISION_TOGGLE());
    let mut rng = Rng(0x0F0F_1234_5678_ABCD);
    for it in 0..ITERS {
        randomize(&mut rng);
        let mut ar = Arena::new();
        let ents: Vec<u32> = (0..6).map(|_| ar.alloc(0x40) as u32).collect();
        let saved = ar.alloc(16);
        for k in 0..CASES {
            let n = rng.below(10);
            unsafe {
                put(FILTER_COUNT, n);
                for i in 0..12 {
                    put(FILTER_LIST + i * 4, rng.pick(&[ents[0], ents[1], ents[2], 0, 0x77]));
                }
                put(STEP_COUNTER, rng.pick(&[0, 0, 1, 5, 0x8000_0000]));
                put(WORLD, rng.u32());
                put(saved, rng.mix(&[0x8002E, 0, 5]));
                for &e in &ents {
                    put(e as usize + 0x2C, rng.mix(&[0x8002E, 0, 5]));
                }
            }
            let e = rng.pick(&ents);
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.edx = e;
            c.out = 0;
            run("filter_defer", o_def, filter_defer_thunk as usize, &c);
            run("filter_cancel", o_can, filter_cancel_thunk as usize, &c);
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.edx = e;
            c.regs.eax = rng.pick(&[0x8002E, 0, 5, 77]);
            c.regs.ecx = 0xCAFE_0000;
            c.out = 0;
            run("set_filter_info", o_set, set_filter_info_thunk as usize, &c);
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.edx = e;
            c.regs.ecx = saved as u32;
            c.out = 0;
            c.args = vec![rng.pick(&[0, 1, 2, 0x101, 0xFF01, 0x100, 7])];
            run("collision_toggle", o_tog, collision_toggle_thunk as usize, &c);
        }
    }
}

#[test]
fn port_physics_body_accessors() {
    let _g = lock();
    begin();
    // K1: the motion's setPosition (virtual, one stack arg, pops 4); 0xD607B0; the cookie check
    let mut motion = callee(K1, 1, 4, ALL, Ret::Any);
    motion.peek = vec![(0, 4)];
    let mut cookie = callee(0x00C9_604F, 0, 0, 0, Ret::Keep);
    cookie.silent = true;
    set_callees(vec![motion, callee(0x00D6_07B0, 0, 0, EAX, Ret::Any), cookie]);
    let _ = orig_GET_PROPERTY();
    let (o_gh, o_sp, o_pv) = (orig_BODY_GET_HANDLE(), orig_BODY_SET_POSITION(), orig_POINT_VELOCITY());
    let mut rng = Rng(0x7777_0000_1234_5678);
    for it in 0..ITERS {
        randomize(&mut rng);
        let mut ar = Arena::new();
        let bodies: Vec<u32> = (0..12)
            .map(|_| {
                let b = ar.alloc(0x300) as u32;
                for k in 0..0x300 / 4 {
                    let f = if (0x130..0x1D0).contains(&(k * 4)) { rng.float_bits() } else { rng.u32() };
                    unsafe { put(b as usize + k * 4, f) };
                }
                let arr = ar.alloc(64);
                let vt = ar.alloc(0x80);
                unsafe {
                    put(arr, rng.pick(&[8, 8, 1]));
                    put(arr + 8, rng.u32());
                    put(arr + 16, 8);
                    put(arr + 24, rng.u32());
                    put(b as usize + 0x74, arr as u32);
                    put(b as usize + 0x78, rng.pick(&[0, 1, 2]));
                    put(b as usize + 0xD0, vt as u32);
                    put(vt + 0x30, stub_of(K1) as u32);
                }
                b
            })
            .collect();
        for i in 0..0x1388usize {
            unsafe { put(BODY_TABLE + i * 0x18, bodies[rng.below(12) as usize]) };
        }
        let out = ar.alloc(32);
        let pt = ar.alloc(16);
        for k in 0..CASES {
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.eax = rng.mix(&[0, 1, 5, 0x1387]) % 0x1388;
            if rng.one_in(6) {
                c.regs.eax = rng.pick(&[0x1388, 0x1387, 0xFFFF_FFFF]);
            }
            c.regs.edx = 0x1234;
            let slot = BODY_TABLE + (c.regs.eax as usize % 0x1388) * 0x18;
            let keep = unsafe { get(slot) };
            if rng.one_in(3) {
                unsafe { put(slot, 0) };
            }
            run("body_get_handle", o_gh, body_get_handle_thunk as usize, &c);
            unsafe { put(slot, keep) };

            unsafe {
                put(STEP_COUNTER, rng.pick(&[0, 0, 0, 1, 3]));
                for j in 0..3 {
                    put(pt + j * 4, rng.float_bits());
                }
                put(F32_ZERO, rng.pick(&[0, 0x8000_0000, 0x3F80_0000]));
            }
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.ecx = rng.below(0x1388);
            c.out = 0;
            c.args = vec![pt as u32];
            run("body_set_position", o_sp, body_set_position_thunk as usize, &c);

            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.edx = rng.below(0x1388);
            c.regs.ecx = out as u32;
            c.out = 0;
            c.args = vec![pt as u32];
            for j in 0..4 {
                unsafe { put(out + j * 4, 0x4242_4242) };
            }
            run("point_velocity", o_pv, point_velocity_thunk as usize, &c);
        }
    }
}

#[test]
fn port_physics_layers_collide() {
    let _g = lock();
    begin();
    set_callees(vec![]);
    let o = orig_LAYERS_COLLIDE();
    let mut rng = Rng(0x1357_9BDF_0246_8ACE);
    for it in 0..ITERS {
        randomize(&mut rng);
        let mut ar = Arena::new();
        let block = ar.alloc(0x100);
        let filter = ar.alloc(0x100);
        let bits = ar.alloc(2048 + 64);
        for k in 0..(2048 + 64) / 4 {
            unsafe { put(bits + k * 4, rng.u32()) };
        }
        unsafe { put(filter + 0x34, bits as u32) };
        let out = ar.alloc(16);
        for k in 0..200u32 {
            unsafe { putb(block + 0x6C, rng.pick(&[0, 0x10, 0xFF, 0x0F])) };
            let r5 = rng.below(5);
            let hi_a = rng.pick(&[0u32, 0, 1, 0xFFFF, r5]);
            let hi_b = if rng.one_in(2) { hi_a } else { rng.below(5) };
            let (r1, r2) = (rng.below(128), rng.below(128));
            let la = rng.pick(&[0x3Fu32, r1, r2, 0]);
            let (r1, r2) = (rng.below(128), rng.below(128));
            let lb = rng.pick(&[0x3Fu32, r1, r2, 0]);
            let (a, b) = (hi_a << 16 | la, hi_b << 16 | lb);
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.ecx = a;
            c.regs.edx = b;
            c.tls = block as u32;
            c.args = vec![filter as u32, out as u32];
            unsafe { putb(out, 0x99) };
            run("layers_collide", o, layers_collide_thunk as usize, &c);
        }
    }
}

#[test]
fn port_physics_pair_dispatch() {
    let _g = lock();
    begin();
    set_callees(vec![callee(K1, 2, 0, 0, Ret::Any), callee(K2, 2, 0, 0, Ret::Any)]);
    let o = orig_PAIR_DISPATCH();
    let mut rng = Rng(0x2468_ACE0_1357_9BDF);
    for it in 0..ITERS {
        randomize(&mut rng);
        let mut ar = Arena::new();
        let objs = objects(&mut rng, &mut ar, 30, 0x80, &[0, 1, 2, 3, 4, 5, 6, 7, 12]);
        for i in 0..0xD0usize {
            unsafe {
                put(PAIR_TABLE + i * 20 + 4, rng.pick(&[0, stub_of(K1) as u32, stub_of(K2) as u32]));
                put(PAIR_TABLE + i * 20 + 8, rng.pick(&[0, stub_of(K1) as u32, stub_of(K2) as u32]));
            }
        }
        let ev = ar.alloc(0x20);
        for k in 0..CASES {
            unsafe {
                put(ev, some_handle(&mut rng, &objs));
                put(ev + 8, some_handle(&mut rng, &objs));
                put(ev + 0xC, rng.pick(&[0, 1, 1, 0, 2, 0xFFFF]));
            }
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.edi = ev as u32;
            c.out = 0;
            run("pair_dispatch", o, pair_dispatch_thunk as usize, &c);
        }
    }
}

/// Link `slots` (indices) into a circular list through +0xC (next) / +0x10 (prev); the head.
fn link(slots: &[usize]) -> u32 {
    let a = |i: usize| (CSLOTS + i * 20) as u32;
    for (n, &i) in slots.iter().enumerate() {
        let next = slots[(n + 1) % slots.len()];
        let prev = slots[(n + slots.len() - 1) % slots.len()];
        unsafe {
            put(a(i) as usize + 0xC, a(next));
            put(a(i) as usize + 0x10, a(prev));
        }
    }
    if slots.is_empty() { 0 } else { a(slots[0]) }
}

#[test]
fn port_physics_constraint_slots() {
    let _g = lock();
    begin();
    set_callees(vec![callee(K1, 1, 4, ALL, Ret::Any)]);
    let (o_alloc, o_rel) = (orig_CSLOT_ALLOC(), orig_CSLOT_RELEASE());
    let mut rng = Rng(0x9999_8888_7777_6666);
    for it in 0..ITERS * 2 {
        randomize(&mut rng);
        let mut ar = Arena::new();
        let mut idx: Vec<usize> = (0..0x200).collect();
        for i in (1..idx.len()).rev() {
            idx.swap(i, rng.below(i as u32 + 1) as usize);
        }
        let nf = rng.pick(&[0usize, 1, 1, 2, 5, 12]);
        let nu = rng.pick(&[0usize, 1, 1, 2, 3, 6, 12]);
        let free = idx[..nf].to_vec();
        let used = idx[nf..nf + nu].to_vec();
        unsafe {
            put(C_FREE_HEAD, link(&free));
            put(C_USED_HEAD, link(&used));
            put(C_USED_COUNT, rng.below(100));
            for &i in free.iter().chain(&used) {
                let s = CSLOTS + i * 20;
                let rf = if rng.one_in(3) { 0 } else { ar.alloc(16) };
                if rf != 0 {
                    let vt = ar.alloc(16);
                    put(vt, stub_of(K1) as u32);
                    put(rf, vt as u32);
                    puth(rf + 4, rng.pick(&[0, 1]));
                    puth(rf + 6, rng.pick(&[1, 1, 2, 0]));
                }
                put(s + 8, rf as u32);
                put(s, rng.u32());
                put(s + 4, rng.u32());
            }
        }
        let c = Case::new(it as u64);
        run("constraint_slot_alloc", o_alloc, cslot_alloc_thunk as usize, &c);
        if !used.is_empty() {
            for &s in used.iter().take(3) {
                let mut c = Case::new(it as u64);
                c.regs.esi = (CSLOTS + s * 20) as u32;
                c.out = 0;
                run("constraint_slot_release", o_rel, cslot_release_thunk as usize, &c);
            }
        }
    }
}

#[test]
fn port_physics_part_param_and_attached() {
    let _g = lock();
    begin();
    set_callees(vec![callee(0x0096_3580, 0, 0, ALL, Ret::Any), callee(0x00AB_6E20, 1, 0, ALL, Ret::Any)]);
    let (o_pp, o_ha) = (orig_PART_PARAM(), orig_HAS_ATTACHED());
    let mut rng = Rng(0x5555_AAAA_1234_4321);
    for it in 0..ITERS {
        randomize(&mut rng);
        let mut ar = Arena::new();
        for k in 0..30u32 {
            for a in [0x0279_76D8usize, 0x0279_76DC, 0x0279_76E0, 0x0279_76E4, 0x0279_76E8, 0x0279_76EC, 0x0279_76F0, 0x00E8_7998] {
                unsafe { put(g(a), rng.float_bits()) };
            }
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.eax = rng.pick(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0xFFFF_FFFF, 0x8000_0000, 100]);
            c.want_st0 = true;
            c.out = 0;
            run("ragdoll_part_param_get", o_pp, part_param_thunk as usize, &c);
        }
        for k in 0..CASES {
            let o = ar.alloc(0x100);
            unsafe {
                for j in 0..0x40 {
                    put(o + j * 4, rng.u32());
                }
                put(o + 0x4C, rng.pick(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0xFFFF_FFFF]));
                put(o + 0xCC, rng.pick(&[0, 1, 0x100, 0x1_0000]));
                put(o + 0xC4, rng.pick(&[0, 1, 0x100, 0x1_0000]));
            }
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.eax = o as u32;
            run("object_has_attached_state", o_ha, has_attached_thunk as usize, &c);
        }
    }
}

#[test]
fn port_physics_box_to_mesh() {
    let _g = lock();
    begin();
    map_data(g(0x00E8_0B00), 0x100);
    let mut cookie = callee(0x00C9_604F, 0, 0, 0, Ret::Keep);
    cookie.silent = true;
    set_callees(vec![cookie]);
    let o = orig_BOX_TO_MESH();
    let mut rng = Rng(0x3141_5926_5358_9793);
    for it in 0..ITERS * 3 {
        randomize(&mut rng);
        let mut ar = Arena::new();
        let (vl, tl, tri) = (ar.alloc(4096), ar.alloc(4096), ar.alloc(4096));
        let (cen, rot, half) = (ar.alloc(16), ar.alloc(48), ar.alloc(16));
        for k in 0..CASES {
            unsafe {
                puth(vl, rng.pick(&[0u16, 0, 1, 7, 30, 0xFFF0]));
                puth(tl, rng.pick(&[0u16, 1, 9, 20, 0xFFFF]));
                puth(tri, rng.pick(&[0u16, 3, 12, 40, 0xFFFE]));
                for j in 0..3 {
                    put(cen + j * 4, rng.float_bits());
                    put(half + j * 4, rng.float_bits());
                }
                for j in 0..9 {
                    put(rot + j * 4, rng.float_bits());
                }
            }
            if rng.one_in(3) {
                // plain finite numbers too
                unsafe {
                    for j in 0..3 {
                        wf(cen + j * 4, rng.below(2000) as f32 / 10.0 - 100.0);
                        wf(half + j * 4, rng.below(100) as f32 / 10.0);
                    }
                    for j in 0..9 {
                        wf(rot + j * 4, rng.below(2000) as f32 / 1000.0 - 1.0);
                    }
                }
            }
            if unsafe { rh(vl) } > 100 {
                // keep the lists inside the arena allocations
                unsafe { puth(vl, 5) };
            }
            if unsafe { rh(tl) } > 100 {
                unsafe { puth(tl, 5) };
            }
            if unsafe { rh(tri) } > 100 {
                unsafe { puth(tri, 5) };
            }
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.ecx = cen as u32;
            c.regs.edx = if rng.one_in(3) { 0 } else { tri as u32 };
            c.out = 0;
            c.args = vec![vl as u32, if rng.one_in(3) { 0 } else { tl as u32 }, rot as u32, half as u32];
            if let Err(e) = check(o, box_to_mesh_thunk as usize, &c) {
                let f = |a: usize, n: usize| (0..n).map(|j| format!("{:#010x}", unsafe { get(a + j * 4) })).collect::<Vec<_>>().join(" ");
                panic!("box_to_mesh: {e}
 cen {}
 half {}
 rot {}
 counts v {} t {} tri {}", f(cen, 3), f(half, 3), f(rot, 9), unsafe { rh(vl) }, unsafe { rh(tl) }, unsafe { rh(tri) });
            }
        }
    }
}

#[test]
fn port_physics_limb_update() {
    let _g = lock();
    begin();
    map_data(g(0x00E8_0B00), 0x100); // cookie
    map_data(g(0x00E5_0000), 0x1_0000); // the x87 constants
    map_data(g(0x00E8_4300), 0x100); // clock
    map_data(g(0x031B_F200), 0x100); // heap slot
    map_data(g(0x00DB_8000), 0x100); // import slots
    let mut cookie = callee(0x00C9_604F, 0, 0, 0, Ret::Keep);
    cookie.silent = true;
    let mut ray = callee(0x006F_A330, 2, 0, ECX | ESI | EDI, Ret::Flag);
    ray.fill = vec![(0, 12)];
    let mut adjust = callee(0x0071_57D0, 0, 0, ECX | EDX | ESI | EDI, Ret::Any);
    adjust.fill = vec![(102, 12)];
    let mut sweep = callee(0x0071_33F0, 3, 0, ECX | ESI | EDI, Ret::Flag);
    sweep.fill = vec![(101, 4), (105, 12)];
    let mut vel = callee(K2, 1, 4, ECX, Ret::Any);
    vel.peek = vec![(0, 4)];
    set_callees(vec![
        cookie,
        ray,
        callee(0x0096_0D60, 0, 0, EAX, Ret::Any),
        adjust,
        callee(K1, 1, 4, 0, Ret::Any),
        callee(0x00D2_B7D0, 3, 12, ALL, Ret::Any),
        sweep,
        callee(0x006F_5740, 0, 0, EDX, Ret::Any),
        callee(0x00D5_CC40, 3, 12, EAX, Ret::Any),
        callee(0x006F_DBB0, 0, 0, EAX | EDX, Ret::Any),
        callee(0x00D5_8FF0, 0, 0, ESI, Ret::Any),
        vel,
    ]);
    let o = orig_LIMB_UPDATE();
    let mut rng = Rng(0x0BAD_CAFE_1234_5678);
    for it in 0..ITERS * 2 {
        randomize(&mut rng);
        unsafe {
            std::ptr::copy_nonoverlapping(exe_bytes(0xE5BFA0, 8).as_ptr(), g(0xE5BFA0) as *mut u8, 8);
            std::ptr::copy_nonoverlapping(exe_bytes(0xE5D6A8, 8).as_ptr(), g(0xE5D6A8) as *mut u8, 8);
            put(g(0xDB807C), stub_of(K1) as u32);
        }
        let mut ar = Arena::new();
        zero(HANDLES, 0x1_0000);
        // the owner: a human (type 1) at a known handle
        let owner_h = (rng.below(0xFFF) + 1) << 12 | rng.below(4096);
        let human = ar.alloc(0x1000);
        let now = rng.below(0x6B49_D200) as i32;
        for k in 0..0x1000 / 4 {
            unsafe { put(human + k * 4, rng.u32()) };
        }
        unsafe {
            put(human + 0x44, owner_h);
            put(human + 0x4C, 1);
            putb(human + 0x3CC, rng.pick(&[0, 1, 1]));
            put(HANDLES + (owner_h as usize & 0xFFF) * 16, human as u32);
            put(g(0xE843AC), now as u32);
        }
        let bodies: Vec<u32> = (0..12)
            .map(|_| {
                let b = ar.alloc(0x300);
                let vt = ar.alloc(0x80);
                for k in 0..0x300 / 4 {
                    unsafe { put(b + k * 4, rng.u32()) };
                }
                for j in 0..4 {
                    unsafe { put(b + 0x1A0 + j * 4, rng.float_bits()) };
                }
                for j in 0..3 {
                    unsafe { put(b + 0x110 + j * 4, rng.float_bits()) };
                }
                unsafe {
                    put(b + 0xD0, vt as u32);
                    put(vt + 0x40, stub_of(K2) as u32);
                    put(b + 0x2C, rng.mix(&[NO_COLLIDE, 0, 5]));
                }
                b as u32
            })
            .collect();
        let arr = ar.alloc(64);
        for (j, &b) in bodies.iter().enumerate() {
            unsafe { put(arr + j * 4, b) };
        }
        let ents = ar.alloc(0x80);
        let rec = ar.alloc(0x1400);
        for k in 0..CASES {
            let t_kind = rng.below(5);
            let t: i32 = match t_kind {
                0 => -1 - rng.below(100) as i32,
                1 => (now - rng.below(1000) as i32).max(0),
                2 => now.wrapping_sub(0x35A4_E900 + rng.below(1000) as i32).max(0),
                3 => now + 0x35A4_E900 + rng.below(1000) as i32,
                _ => now + rng.below(1000) as i32,
            };
            unsafe {
                put(human + 0xF94, t as u32);
                for j in 0..0x1400 / 4 {
                    put(rec + j * 4, rng.u32());
                }
                putb(rec + 0x1298, rng.pick(&[0u8, 0x08, 0, 0xFF, 0x10]));
                for j in 0..3 {
                    put(rec + 0x11AC + j * 4, rng.float_bits());
                }
                put(rec + 0xF70, owner_h);
                put(rec + 0xF74, arr as u32);
                put(rec + 0xF7C, rng.pick(&[0, 1, 3, 11, 11, 11, 12]));
                put(rec + 0xFAC, ents as u32);
                for j in 0..11 {
                    putb(ents + j * 8, rng.pick(&[0u8, 1, 2, 3, 3, 3, 0xFF, 0xFC]));
                    put(ents + j * 8 + 4, rng.mix(&[NO_COLLIDE, 0, 5]));
                }
                put(g(0x0252_7DAC), rng.pick(&[0, 0, 1, 3]));
                put(g(0x0252_7D94), rng.u32());
                put(g(0x031B_F2BC), rng.u32());
            }
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.ecx = rec as u32;
            c.out = 0;
            run("limb_update", o, limb_update_thunk as usize, &c);
        }
    }
}

use std::sync::atomic::{AtomicU32, Ordering as AO};

static T_HITS: AtomicU32 = AtomicU32::new(0);
static T_COMP: AtomicU32 = AtomicU32::new(0);
static T_POOL: AtomicU32 = AtomicU32::new(0);

/// A slot of the scratch pool the allocations come from (the same sequence in both runs).
fn pool_slot() -> u32 {
    T_POOL.load(AO::Relaxed) + (hook_rand() % 32) * 0x400
}

fn h_cast(_: &difftest::Pushad, args: *const u32) -> Option<u32> {
    unsafe {
        let c = *args.add(3) as usize;
        if get(c) == CLOSEST_VT {
            // closest hit: the hit id at +0x60 (sometimes 0) and the position at +0x70
            put(c + 0x60, if hook_rand() % 4 == 0 { 0 } else { hook_rand() | 1 });
            for j in 0..3 {
                put(c + 0x70 + j * 4, hook_rand());
            }
        } else {
            put(c + 0x10, T_HITS.load(AO::Relaxed));
            put(c + 0x14, hook_rand() % 4);
            put(c + 0x320, T_COMP.load(AO::Relaxed));
            put(c + 0x324, hook_rand() % 4);
        }
    }
    None
}

fn h_alloc(_: &difftest::Pushad, _: *const u32) -> Option<u32> {
    Some(pool_slot())
}

fn h_grow(_: &difftest::Pushad, args: *const u32) -> Option<u32> {
    unsafe {
        let arr = *args as usize;
        put(arr, pool_slot());
        put(arr + 8, 8);
    }
    None
}

#[test]
fn port_physics_raycast() {
    let _g = lock();
    begin();
    map_data(g(0x00E8_0B00), 0x100); // cookie
    map_data(g(0x00E1_8900), 0x100); // vtables (only their addresses matter)
    map_data(g(0x031B_F200), 0x100); // heap slot
    map_data(g(0x00DB_8000), 0x100); // import slots
    map_data(g(0x0290_FA00), 0x100); // the world box
    let mut cookie = callee(0x00C9_604F, 0, 0, 0, Ret::Keep);
    cookie.silent = true;
    let mut clamp = callee(R_CLAMP, 1, 0, EAX | ECX | EDX | ESI | EDI, Ret::Any);
    clamp.st0 = true;
    clamp.fill = vec![(104, 12)];
    let mut cast = callee(CAST_RAY, 4, 16, ALL, Ret::Any);
    cast.hook = Some(h_cast);
    let mut alloc = callee(HEAP_ALLOC, 2, 8, ECX, Ret::Any);
    alloc.hook = Some(h_alloc);
    let mut grow = callee(ARRAY_GROW, 2, 0, 0, Ret::Any);
    grow.hook = Some(h_grow);
    let mut fill = callee(HIT_FILL, 1, 4, ECX, Ret::Any);
    fill.fill = vec![(101, 0x60)];
    let mut post = callee(HIT_POST, 10, 0x28, ECX | EDX, Ret::Any);
    post.byte_args = 1 << 6 | 1 << 7 | 1 << 9;
    set_callees(vec![
        cookie,
        clamp,
        callee(COLL_CTOR, 0, 0, ECX, Ret::Any),
        callee(COLL_DTOR, 0, 0, ECX, Ret::Any),
        cast,
        alloc,
        grow,
        fill,
        post,
        callee(K1, 1, 4, 0, Ret::Any),
        callee(HEAP_FREE, 3, 12, ECX, Ret::Any),
    ]);
    let o = orig_RAYCAST();
    let mut rng = Rng(0x1111_2222_3333_4444);
    for it in 0..ITERS * 2 {
        randomize(&mut rng);
        unsafe { put(g(0xDB807C), stub_of(K1) as u32) };
        let mut ar = Arena::new();
        let block = ar.alloc(0x100);
        let world = ar.alloc(0x100);
        let pool = ar.alloc(32 * 0x400 + 0x400);
        let pool = (pool + 15) & !15;
        T_POOL.store(pool as u32, AO::Relaxed);
        let hits = ar.alloc(4 * 0x60 + 16);
        let hits = (hits + 15) & !15;
        let comp = ar.alloc(4 * 12 + 16);
        T_HITS.store(hits as u32, AO::Relaxed);
        T_COMP.store(comp as u32, AO::Relaxed);
        let req = ar.alloc(0x80);
        let out = ar.alloc(32);
        unsafe {
            put(g(0x0252_7D94), world as u32);
            put(world + 0x54, rng.u32());
            put(world + 0x70, rng.u32());
        }
        for k in 0..CASES {
            unsafe {
                for j in 0..4 * 0x60 / 4 {
                    put(hits + j * 4, rng.mix(&[0, 1, 5, 7]));
                }
                for j in 0..4 {
                    for i in 0..8 {
                        put(hits + j * 0x60 + 0x20 + i * 4, rng.mix(&[0xFFFF_FFFF, 3, 9]));
                    }
                    put(hits + j * 0x60 + 0x10, rng.float_bits());
                }
                for j in 0..12 {
                    put(comp + j * 4, rng.u32());
                }
                for j in 0..6 {
                    put(req + j * 4, rng.float_bits());
                }
                put(req + 0x44, rng.u32());
                put(req + 0x48, rng.u32());
                put(req + 0x54, rng.u32());
                put(req + 0x4C, rng.pick(&[0x52, 0x56, 0x5D, 7, 0x52, 0x56]));
                put(req + 0x50, rng.pick(&[0, 1, 2, 0xFFFF_FFFF, 0, 1, 1]));
                putb(block + 0x6C, 0x55);
                for j in 0..8 {
                    put(out + j * 4, rng.u32());
                }
            }
            let mut c = Case::new(it as u64 * 1000 + k as u64);
            c.regs.ecx = req as u32;
            c.tls = block as u32;
            c.args = vec![out as u32, rng.pick(&[0, 1, 0x101, 0xFF01, 0xFFFF_FF00])];
            if let Err(e) = check(o, raycast_thunk as usize, &c) {
                panic!("raycast: {e}\n  mode {} kind {:#x} args {:x?}", unsafe { get(req + 0x50) }, unsafe { get(req + 0x4C) }, c.args);
            }
        }
    }
}
