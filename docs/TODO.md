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
- **Automatic bisect script:** launch with a skip list, auto-press Continue, wait for a crash or steady frames,
  then halve the list.
