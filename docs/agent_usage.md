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
