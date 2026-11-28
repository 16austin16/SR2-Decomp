# Porting rules (read fully before starting)

These are the rules every porting agent follows. This file is the master copy; keep it up to date when a round teaches something new.

Project: Saints Row 2 PC (x86, MSVC 2005 LTCG) rewritten in Rust, one function at a time, inside a hook DLL.

Where things are:
- Repo: F:\Saints Row 2 Decomp (Windows; Bash or PowerShell).
- Run python WITHOUT -I, because the tools need capstone.
- Exe bytes: bin/SR2_pc.unpacked.exe (image base 0x400000).
- Named functions: docs/functions.csv.
- Docs: docs/analysis/.
- Skipped or pending functions: docs/skipped_ports.md.

## Exactness
- Callees stay original. Call them BY ADDRESS through call_regs (crates/sr2-hook/src/portlib.rs), with exactly
  the registers and stack arguments the original sets up. That includes registers that pass through unchanged to
  a callee, so check what each callee reads on entry.
- Pop callees that return on ST0 (see call_regs_st0 in port_humans2.rs; copy helpers into your own file).
- Confirm EVERY comparison's polarity at the disassembly. That covers setcc / sbb / and masks, jae versus jge,
  and x87 `fnstsw; test ah, 5 / 0x41 / 0x44`:
  - 5 then jp: taken when ST0 >= operand, or NaN.
  - 0x41 then jne: taken when ST0 <= operand, or NaN.
  - 0x44 then jnp: taken on exact equality.
  comiss / ucomiss: an unordered result sets ZF, PF and CF. A port once inverted a mask and crashed the game.
- x87 runs at 53-bit precision. Use f64 arithmetic with `as f32` at every `fstp dword`, and read constants
  from the exe with their exact bits (doubles stay doubles). SSE scalar ops are f32 ops. `fld dword` quiets a
  signalling NaN.
- Reproduce exact outputs (full EAX vs AX vs AL, ST0, XMM0) and the registers the original preserves. LTCG
  callers may keep values in ECX / EDX / XMM registers across a call to a function that preserves them.
- Scratch registers are outputs too: when the original leaves a value in ECX or EDX (a `sete cl`, a
  loaded pointer, a zeroed counter), the port must leave the same value. Do NOT "safely" restore the
  caller's ECX / EDX instead. LTCG callers can read them: vehicle_is_helicopter's port restored ECX
  and police helicopters stopped tracking the player (2026-11-26).
- No inline / naked asm that copies the original body. A port must be Rust; asm is only for thunks.
- Decode jump tables from their bytes.
- For C++ objects built on the stack, reserve a buffer of the same size and alignment, and call the
  constructor / destructor by address on every path the original does.
- Size every stack buffer exactly as the original does, and bound every count.
- Keep the original's quirks, except on paths where the original corrupts memory or crashes; note any such
  deviation in the report.
- Never replace an address whose docs/functions.csv status is `rust`, or that appears in any other
  crates/sr2-hook/src/*.rs file (unless your task says so explicitly). Check both forms: 0x00XX_XXXX and plain hex.
- Pin each port to its whole original body with replace_pinned and `python tools/body_bytes.py NAME 0xADDR 0xEND`.
  Pass the end explicitly whenever the default would run past the function.

## The judge (the only test that accepts a port)
- `crates/sr2-judge` is the one shared, strict judge. A port is accepted ONLY when `sr2-judge <address or name>` prints
  PASS. Your own offline differential tests help you debug but never accept a port.
- NEVER edit `crates/sr2-judge` (its sources, `build.rs`, `callees.toml`) or `crates/sr2-hook/src/registry.rs`, and never
  change how `portlib::replace_pinned` behaves in registry mode. Do not work around the judge: no code that
  detects it, no `cfg` switches for it, no thunks that behave differently under it.
- Inline asm may only be used for thunks (entering your Rust from the game's register / stack convention). The port body
  itself is Rust. Copying the original's bytes into asm, or running the original, is not a port.
- Build and run (each run prints the judge's source hash on its first line; it must equal the hash below):
  `cargo build --release -p sr2-judge --target i686-pc-windows-msvc`
  `target/i686-pc-windows-msvc/release/sr2-judge.exe 0xADDR --cases 500` (or a port name; `--verbose` prints every case;
  `--list` lists the registered ports; `--all [--cases N]` judges every port except portlib's EXCLUDED ones).
- Verdicts: PASS (every non-faulting case matched and at least 20% of the cases ran without faulting), FAIL (first
  differences are printed), INCONCLUSIVE (fewer than 20% of the cases ran clean: the function needs inputs the random
  worlds rarely give; say so in the report, do not call it passed), ERROR (the pinned body does not match the exe).
- What it compares on every case: all general registers (ECX / EDX too), ESP, x87 depth and ST0, XMM0-7, the stack
  arguments, every byte of the arena and every page of the exe image that was written, and the ordered list of calls
  to functions outside the body (target, the registers the callee reads, its stack arguments, x87 depth). A fault on
  one side only is a FAIL. Replace nothing the judge cannot see: a call through a function pointer or a vtable is not
  stubbed and ends the case in a fault on both sides.
- To override a wrongly detected callee (stack size, float result, which registers it keeps), ask the coordinator; do not
  edit `callees.toml` yourself.
- Judge source hash (SHA-256 of `crates/sr2-judge/src/*.rs`, `build.rs`, `callees.toml` and `crates/sr2-hook/src/registry.rs`):
  `673aa7df3ceee44a4136a0e060b798290652ff9ccf8b6868b10cd84fd1e2a553`. If a run prints a different hash, stop and tell the coordinator.

## Checks
- Pure functions: a relocated-original shadow (detour::relocate) for the first 3000 calls, keeping the
  original's result on a mismatch.
- Writers with small, known writes: a snapshot shadow (run original, save, restore, run Rust, compare,
  keep the original's writes on a mismatch).
- Functions that write too much, or whose callees have side effects: per-branch call counters only.
  Say which you used for each.
- Patterns to copy: port_shops.rs, port_frame3.rs, port_hot57b.rs, port_render3.rs, port_cutscene2.rs.
- Offline native differential testing against the original bytes is required for every port. Use ignored
  #[cfg(test)] tests, linked at a fixed base, as in those files:
  RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/DYNAMICBASE:NO -C link-arg=/BASE:0x10000000"
  cargo test --release -p sr2-hook --target i686-pc-windows-msvc <name> -- --ignored --test-threads=1
  Run each test in its own process. If another agent's file breaks `cargo test`, test from a scratch copy of
  the repo with that file stubbed.

## Performance (a port once dropped the game to 5 fps)
- After the shadow window, a port must cost about what the original does: no per-call allocation, no
  logging, no write recording, no big copies.
- If a shadow's writes can exceed 64 KB, shadow only the first 16 calls.
- Mechanically translated loops can be ~100x slower than the original; hand-write any loop that runs over
  big tables or many elements.
- State on the render thread must be atomics.

## Rules
- NEVER launch sub-agents (no Agent tool, no forks). Do all the work yourself. Helper agents
  multiply plan usage: on 2026-11-26, 9 agents started 15 helpers and used 27% of a 5-hour window in 30 minutes.
- Edit ONLY your own file(s), named in your task. Do not commit, do not run the game, and do not touch docs
  or functions.csv.
- install() logs "<module>: replaced N/M with Rust".
- summary() returns "<module>: name calls (mismatches), ...", or an empty string if nothing ran.
- Build with ZERO warnings from your file: `cargo build --release -p sr2-hook --target i686-pc-windows-msvc`.
  - About 7 agents edit other files at the same time. If the build fails in someone else's file, wait a minute
    and retry; never edit it. If it stays broken, build from a scratch copy of the repo.
  - No cargo fmt.
- The recursive register checker can use many GB of memory. Use depth 2, ALWAYS run it under `timeout 120`,
  and never leave it running. Kill your own background jobs before you finish.
- Put your scripts in your own scratchpad subfolder (the coordinator names it in your task), not in the repo.
- FINAL REPORT (under 40 lines, no code):
  - per function: address, true size, check type, difftest case count;
  - discrepancies with the docs;
  - skips with reasons;
  - total bytes;
  - build result.
