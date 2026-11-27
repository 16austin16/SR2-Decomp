# Agent usage log

This log records how much each round of agents costs, so we can choose how many agents to run per round.
The plan is **Claude Pro**. Each round records:
- the 5-hour and weekly usage percentages, read just before the agents start and again after they finish;
- each agent's token count, as reported when it finishes;
- what each agent produced.

How to read it:
- **tokens per function** shows how efficient each kind of task is;
- **plan % per agent** sets how many agents fit in one 5-hour window and in one week.

## Rounds

### 2026-11-06 (6 porting + 2 analysis agents, Sonnet)

No plan readings were taken for this round.

| Agent | Tokens | Result | Tokens / function |
|---|---:|---|---:|
| hot57a (8 math leaves) | 240k | 8 ported | 30k |
| hot57b (10 math leaves) | 269k | 10 ported | 27k |
| render3 | 281k | 10 ported | 28k |
| frame3 | 418k | 6 ported | 70k |
| weather2 | 451k | 15 ported | 30k |
| cutscene2 | 369k | 1 ported | 369k |
| doc 59 (analysis) | 516k | port maps, 20 names | - |
| doc 60 (analysis) | 292k | 188 names | 1.6k / name |
| **Total** | **2.84M** | 50 ports, 208 names | |

### 2026-11-23 (12 porting + 1 analysis agents, Sonnet)

No plan readings were taken for this round.

| Agent | Tokens | Result | Tokens / function |
|---|---:|---|---:|
| small_a (class methods A-G) | 264k | 814 ported | 0.3k |
| small_b (class methods H-Q) | 475k | 85 ported (+115 copied-asm stubs, not counted) | 5.6k |
| small_c (class methods R-Z) | 312k | 86 ported | 3.6k |
| small_vint | 178k | 156 ported | 1.1k |
| small_data | 450k | 101 ported | 4.5k |
| small_game | 434k | 83 ported | 5.2k |
| hot60 | 288k | 20 ported | 14k |
| render4 | 338k | 11 ported | 31k |
| frame4 | 361k | 10 ported | 36k |
| tod_lights (hand port) | 327k | 1 ported | 327k |
| cutscene_load (hand ports) | 361k | 2 ported | 180k |
| nav2 | 525k+ | unfinished (shut down) | - |
| doc 61 (analysis) | 264k | 68 names | 3.9k / name |
| **Total** | **~4.6M** | about 1,370 ports | |

The lesson from these two rounds:
- Small-function list agents cost about 1k-6k tokens per function.
- Big hand ports cost 30k-370k tokens per function.
- An agent costs about 180k-525k tokens whatever it is given, so feeding it small functions gives far more ports for the same usage.

### 2026-11-26: shared test tool (judge)

| Time | 5-hour | Weekly (all models) |
|---|---:|---:|
| Before | 9% | 9% |
| After | 16% | 10% |

| Agent | Tokens | Result |
|---|---:|---|
| judge builder | about 274k after resuming, plus an unknown amount before the app restart | crates/sr2-judge built; judged all 3,975 ports |

That is about **7% of the 5-hour window and 1% of the week** for one big agent, including my own
coordination in the same window. So about 10 to 12 agents of this size fit in one 5-hour window,
and the weekly limit allows a few such rounds per week.

### 2026-11-26: judge fix round (9 Sonnet agents)

The user was also using another session in this window, so part of the change comes from that session.

| Time | 5-hour | Weekly (all models) |
|---|---:|---:|
| Before (agents launched) | 20% | 10% |
| After | 77% | 18% |

| Agent | Modules | FAILs given | Tokens | Now PASS |
|---|---|---:|---:|---:|
| fix_leaves_fp | port_leaves_fp | 202 | 280k | 175 PASS, 19 INCONCLUSIVE, 8 FAIL (judge limits) |
| fix_ai | port_ai | 135 | 352k | 68 PASS, 8 INCONCLUSIVE, 1 FAIL (judge); 71 ports replaced by lifted code (pending decision) |
| fix_small1 | small_vint, small_b, small_c | 168 | 365k | small_c 21/21 PASS; small_b 18 PASS; vint 109 INCONCLUSIVE (judge cannot stub vtable calls), 18 judge issues |
| fix_small2 | small_data, small_game | 132 | 449k | 115 PASS, 12 INCONCLUSIVE, 1 FAIL (judge limit) |
| fix_net | net, bindings2 | 100 (+26 exposed) | 558k | net 91 PASS, 15 INCONCLUSIVE, 11 FAIL (judge limits); bindings2 9 FAIL (judge) |
| fix_bindings | bindings3/4/5 | 134 | 320k | partial: 62 pass in a scratch judge with callee overrides, 72 unfixed |
| fix_leaves_nav | leaves, nav | 131 (116 its own) | 435k | 76 PASS, 12 INCONCLUSIVE, 28 FAIL (about 17 judge limits, 6 spline ports unfinished) |
| fix_gameplay | weapons_audio, activities, cutscene*, camanim, humans | ~120 | 478k | partial: weapons 62 PASS, activities2 29 PASS, cutscene 27 PASS; about 60 still FAIL |
| fix_engine | frame2-4, registrars, vint2, big, weather, physics, hot*, render*, shops, effects | ~250 | 371k | partial: about 80 PASS, most of frame3/4, hot57, render2, effects, big, weather still FAIL |

Total: about 3.6M tokens across the 9 agents, plus 15 helper agents the agents started themselves (stopped after about
30 minutes; they are counted in the plan % but not in the token totals). Plan cost: **57% of a 5-hour window and 8%
of the week**, including the user's other session in that window.

Lessons from this round:
- Fix work costs much more than porting. An agent cost about 280k-560k tokens, and the hard modules did not finish.
- Agents must be told not to start sub-agents; the helpers burned about 27% of a 5-hour window in 30 minutes.
- At this cost, about 6 agents fit in one 5-hour window with room to spare, and 9 agents use most of it.
