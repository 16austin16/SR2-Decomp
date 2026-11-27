//! Twins: functions whose machine code is identical to an already-ported function (template copies,
//! LTCG duplicates). tools/gen_twins.py finds the pairs offline (twins_table.rs); here each pair is
//! checked again against the live bytes, then the twin is pointed at the ported function's thunk.
//!
//! The check, over the ported function's pinned body length:
//! - every byte equal, except the rel32 fields of calls / jumps that leave the function;
//! - those must resolve to the same absolute target in both copies;
//! - the twin's own extent must cover that length.
//!
//! A twin that another hook already patched no longer matches, so it is skipped. Each twin is
//! installed through replace_pinned under the name `<ported name>~<twin address>`, so it appears in
//! the installed-address list, the crash trace and the skip file like any port.

use crate::portlib::{PINNED, replace_pinned};
use crate::twins_table::TWINS;

pub fn install() {
    let pinned = match PINNED.lock() {
        Ok(m) => m.clone(),
        Err(_) => return,
    };
    let (mut ok, mut no_port, mut mismatch) = (0u32, 0u32, 0u32);
    for &(twin, orig, extent, fix) in TWINS.iter() {
        let (twin, orig) = (twin as usize, orig as usize);
        let Some((name, body, thunk)) = pinned.get(&orig) else {
            no_port += 1;
            continue;
        };
        let n = body.len();
        if n == 0 || n > extent as usize {
            mismatch += 1;
            continue;
        }
        let live = unsafe { std::slice::from_raw_parts(twin as *const u8, n) };
        let mut same = true;
        let mut skip = vec![false; n];
        for &f in fix {
            let f = f as usize;
            if f + 4 > n {
                continue;
            }
            skip[f..f + 4].iter_mut().for_each(|x| *x = true);
            let rt = i32::from_le_bytes(live[f..f + 4].try_into().unwrap());
            let ro = i32::from_le_bytes(body[f..f + 4].try_into().unwrap());
            let at = (twin + f + 4).wrapping_add(rt as usize);
            let ao = (orig + f + 4).wrapping_add(ro as usize);
            if at != ao {
                same = false;
                break;
            }
        }
        if same {
            same = (0..n).all(|i| skip[i] || live[i] == body[i]);
        }
        if !same {
            mismatch += 1;
            continue;
        }
        let tname = format!("{name}~{twin:x}");
        if replace_pinned(&tname, twin, live.to_vec().as_slice(), *thunk) {
            ok += 1;
        } else {
            mismatch += 1;
        }
    }
    crate::log(&format!(
        "twins: installed {ok} of {} (port not installed {no_port}, bytes differ or skipped {mismatch})",
        TWINS.len()
    ));
}
