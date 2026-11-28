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
     - The indirect-stub pop count counts saved-register pushes before a virtual call as arguments, so the original
       runs with a wrong ESP (seen at 8799e0, 5cde70, 609960). Count only the pushes after the last instruction that
       moves the arguments' base, or read the callee's ret N where the vtable target is known.
     - 0xB92270 crashes the judge process at 60+ cases.
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
