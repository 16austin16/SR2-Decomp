//! The F7 menu's Shops page: every shop in the city, grouped by chain, with a teleport to its door.
//!
//! The game keeps its shops in an array of 0xA0-byte records at `[0x21F2AA8]` (count `[0x21F2AA4]`),
//! filled by `store_chains_parse` from shops_sr2_city.xtbl. A record holds:
//! - `+0`: `string_hash_lower(Name)` (CRC-32 of the lowercased name);
//! - `+4`: the zone object; its trigger `[+0x10]` has `[trigger+0x18] + 0x1C` = door position x, y, z.
//!
//! The names come from that table (shops_data.rs). They are matched to the records at runtime with
//! the game's own hash function, so a label can only ever be shown for the record it belongs to.
//! Records with no matching name are listed under "Other".

use std::sync::Mutex;

use crate::gfx_cmd::call_regs;
use crate::shops_data::SHOPS;

const SHOP_COUNT: usize = 0x021F_2AA4;
const SHOP_ARRAY: usize = 0x021F_2AA8;
const SHOP_SIZE: usize = 0xA0;
/// `string_hash_lower`: ECX = string, EAX = seed, stack (u32 *out), callee pops; returns EAX = out.
const STRING_HASH_LOWER: usize = 0x00BD_CB30;
/// The player's position (three floats).
const PLAYER_POS: usize = 0x025F_5B14;

pub(crate) struct Chain {
    pub name: String,
    /// Indices into the game's shop array, with each one's display name.
    pub shops: Vec<(usize, String)>,
}

static INDEX: Mutex<Option<Vec<Chain>>> = Mutex::new(None);

fn readable(p: usize) -> bool {
    p >= 0x10000 && p < 0x8000_0000
}

unsafe fn rd(p: usize) -> u32 {
    unsafe { std::ptr::read_volatile(p as *const u32) }
}

fn pretty(s: &str) -> String {
    s.trim_end_matches('\0').replace('_', " ")
}

/// Builds the chain list from the live shop array (game thread). None until the world has loaded
/// its shops.
fn build() -> Option<Vec<Chain>> {
    unsafe {
        let count = rd(SHOP_COUNT) as usize;
        let base = rd(SHOP_ARRAY) as usize;
        if count == 0 || count > 1000 || !readable(base) {
            return None;
        }
        let mut by_hash = std::collections::HashMap::new();
        for &(name, chain) in SHOPS.iter() {
            let mut out = 0u32;
            call_regs(STRING_HASH_LOWER, 0, name.as_ptr() as u32, 0, 0, 0, &[&mut out as *mut u32 as u32]);
            by_hash.insert(out, (pretty(name), pretty(chain)));
        }
        let mut chains: Vec<Chain> = Vec::new();
        for i in 0..count {
            let h = rd(base + i * SHOP_SIZE);
            let (name, chain) = match by_hash.get(&h) {
                Some((n, c)) => (n.clone(), c.clone()),
                None => (format!("Shop {i}"), "Other".to_string()),
            };
            match chains.iter_mut().find(|c| c.name == chain) {
                Some(c) => c.shops.push((i, name)),
                None => chains.push(Chain { name: chain, shops: vec![(i, name)] }),
            }
        }
        chains.sort_by(|a, b| (a.name == "Other").cmp(&(b.name == "Other")).then(a.name.cmp(&b.name)));
        for c in &mut chains {
            c.shops.sort_by(|a, b| a.1.cmp(&b.1));
        }
        Some(chains)
    }
}

/// The chain list, built on first use.
pub(crate) fn chains() -> std::sync::MutexGuard<'static, Option<Vec<Chain>>> {
    let mut g = INDEX.lock().unwrap_or_else(|p| p.into_inner());
    if g.is_none() {
        *g = build();
    }
    g
}

fn vec_at(p: usize) -> Option<[f32; 3]> {
    if !readable(p) {
        return None;
    }
    let v = unsafe { [f32::from_bits(rd(p)), f32::from_bits(rd(p + 4)), f32::from_bits(rd(p + 8))] };
    if v.iter().all(|x| x.is_finite()) && v != [0.0; 3] { Some(v) } else { None }
}

/// Where to put the player for shop `i` (index into the game's shop array). In order of preference:
/// - the Shop_Trigger (the purchase spot, `[zone+0x38]`, position at trigger +0x14);
/// - the Ownership_Trigger outside (`[rec+0xC]`, +0x14);
/// - the clerk (`[[zone+0x10]+0x18]+0x1C`).
/// store_chains_parse fills these from shops_sr2_city.xtbl.
pub(crate) fn door(i: usize) -> Option<[f32; 3]> {
    unsafe {
        let count = rd(SHOP_COUNT) as usize;
        let base = rd(SHOP_ARRAY) as usize;
        if i >= count || !readable(base) {
            return None;
        }
        let rec = base + i * SHOP_SIZE;
        let zone = rd(rec + 4) as usize;
        if readable(zone) {
            let t = rd(zone + 0x38) as usize;
            if readable(t) {
                if let Some(v) = vec_at(t + 0x14) {
                    return Some(v);
                }
            }
        }
        let own = rd(rec + 0xC) as usize;
        if readable(own) {
            if let Some(v) = vec_at(own + 0x14) {
                return Some(v);
            }
        }
        if readable(zone) {
            let clerk = rd(zone + 0x10) as usize;
            if readable(clerk) {
                let o = rd(clerk + 0x18) as usize;
                if readable(o) {
                    return vec_at(o + 0x1C);
                }
            }
        }
        None
    }
}

/// "340 m" / "1.2 km" from the player to shop `i`, or "" if unknown.
pub(crate) fn distance_text(i: usize) -> String {
    let Some(d) = door(i) else { return String::new() };
    let p = unsafe { [f32::from_bits(rd(PLAYER_POS)), f32::from_bits(rd(PLAYER_POS + 4)), f32::from_bits(rd(PLAYER_POS + 8))] };
    let m = ((d[0] - p[0]).powi(2) + (d[1] - p[1]).powi(2) + (d[2] - p[2]).powi(2)).sqrt();
    if !m.is_finite() {
        String::new()
    } else if m < 1000.0 {
        format!("{:.0} m", m)
    } else {
        format!("{:.1} km", m / 1000.0)
    }
}
