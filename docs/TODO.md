# To do

The work agreed for later, in rough priority order. Done items move to the commit log.

## Decomp

1. **Judge improvements** (crates/sr2-judge). These are what keep the remaining 387 FAILs and 572 INCONCLUSIVEs from
   giving real verdicts:
   - stub indirect and vtable calls; this is the biggest gap, and every vint wrapper is INCONCLUSIVE because of it;
   - callee live-in analysis: a prologue `push reg` is not a read of `reg`;
   - `__security_check_cookie` 0xC9604F takes ECX and no stack arguments;
   - keep random pointers out of the judge's own heap (arena_a / arena_snap aliasing makes some cases flaky);
   - compare pointers to stack locals relative to the entry ESP;
   - float returns: 0x40EA20 and 0x49EE60 return ST0, and `_ftol2_sse` 0xDA0140 pops ST0;
   - callees that write out-structures, such as 0x4DDA50 for the weather blends.
   - Judge follow-ups found by the 2026-11-27 trial:
     - (Done 2026-11-28: the indirect-stub pop count now comes from the epilogue balance.) 0xB8A010 now crashes
       the judge process.
     - 0xB92270 crashes the judge process at 60+ cases.
   - Judge issues found in round 9:
     - (Done 2026-11-28: tail jmps count as exits for the pop count; import-table calls go to their own stub,
       which reads no register; a bad stub id is a fault, not a panic; game code outside the function under test
       is non-executable while judging.)
     - Still open: the judge process segfaults on 0x68F6F0 game_event_human_killed at case 145 (seed 1, 200
       cases), even with the code containment. It calls through a table of real function pointers. Debug it with
       --verbose and a single-case seed.
     - Newly visible real FAILs to fix: the render_list*_add family (0x52FDA0..0x5300A0, ECX / EDX leftovers
       after the import call), 0x47CDB0 audio_voice_slot_alloc, 0x68E560 gameplay_exit.
   - **More realistic random worlds**, to turn the ~265 INCONCLUSIVE ports into real verdicts. INCONCLUSIVE means
     that over 80% of the random cases crash on both sides before doing much, so most of the function goes untested.
     Ideas:
     - a valid Lua state (stack, top, base) for the script bindings;
     - sensible values for well-known game globals (the player human at 0x21703D4, the human ring, object and
       handle tables, the shop array) instead of random words;
     - handles that resolve through the game's own handle tables;
     - learn what each port reads through a short in-game trace, and seed the world with it.
2. **Porting round on small-function lists** with the Haiku trial: a few Haiku agents and the rest on Sonnet.
   - The judge is the only acceptance test.
   - Agents must not start sub-agents.
   - Track usage in docs/agent_usage.md.
3. **Hand-rewrite the 71 machine-lifted AI ports** in port_ai.rs (`mod lifted`), with the judge as referee.
4. **Navigation:**
   - run port_nav2's tests and play-test it;
   - finish the spline ports in port_nav;
   - the 0xA02A50 cop-chase crash, whose suspect is nav_tri_closest_point_xz 0xA04180 leaving best-edge = -1.
5. **The three excluded ports:** 0xBF65F0, 0xBB8A10 and 0x72A100.
6. Rewrite the 115 IDirect3DDevice9Lost stubs in port_small_b as real Rust.

## Game quality

- **Anti-aliasing, especially in cutscenes,** which show jagged edges. Options:
  - a post-process AA pass (FXAA or SMAA) drawn at Present, which fits with the ImGui renderer work, since both hook
    the end of the frame;
  - forcing MSAA when the game creates its D3D9 device. That may clash with the game's render targets and is riskier.
  - Possibly only during cutscenes, where the frame rate matters less.

## Tools

- **Own 2D renderer for the F7 menu.** Use Dear ImGui (Rust bindings with the Direct3D 9 backend) and draw at
  `EndScene` / `Present`. That gives:
  - real anti-aliased rounded corners, any font, icons and mouse support;
  - no dependence on the game's HUD state.

  The work:
  - save and restore the render state;
  - handle device loss (the game's IDirect3DDevice9Lost wrappers);
  - feed input to ImGui;
  - port the Player, World, Cheats, Shops and Logging pages and the F8 label box.
- **Remake of the in-game UI:** a modern, mouse-clickable version of the game's own menus and HUD, such as the
  pause menu, map, phone, store menus, options and HUD widgets.
  - Builds on the ImGui renderer above.
  - The game draws these through its vint UI system (Lua scripts + vint_* bindings, docs/analysis/49_vint_system.md).
    The likely route is to replace the screens one at a time, while the game logic underneath, such as buying an item
    or changing a setting, keeps calling the game's own functions.
  - Adds mouse support where the original is keyboard/controller only.
- **Automatic bisect script:** launch with a skip list, auto-press Continue, wait for a crash or steady frames,
  then halve the list.

## Later: optimisation (when most of the game is in Rust)

- **Frame drops while driving (reported 2026-11-28).** The log shows dips to about 32-35 fps while driving; the
  original game is known to run poorly on modern PCs. Agreed with the user to optimise near the end, once the slow
  paths are our own code. First check with `all_ports` that ports are not the cause.

