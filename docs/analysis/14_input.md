# 14: Input

Names are in `docs/functions.csv` (subsystem `input`).

## Sources

| Device | API | Where |
|---|---|---|
| keyboard | **window messages**, not DirectInput. The window procedure (around WinMain, 0x520BA0, and 0xC1EDC0) queues key events. | 32-entry ring of 8-byte events at `0x2348A80` (write index `0x252A574`, read index `0x252A570`), plus a key-up delay ring at `0x2348980` (`0x252A578` / `0x252A57C`) |
| mouse | the cursor position each frame (`GetCursorPos`, `SetCursorPos`) and window messages for buttons and wheel | `mouse_update` (0xC1F320) |
| gamepads (DirectInput) | `DirectInput8Create` → `0x252A580`, `EnumDevices(DI8DEVCLASS_GAMECTRL, 0xC12EA0, …, DIEDFL_ATTACHEDONLY)` | pad records |
| gamepad (XInput) | `XInputGetState` on the first connected user 0-3 | pad mode 3 |

**`input_dinput8_create`** (0xC13910, from `game_init_allocators_and_packfiles`):
1. Create DirectInput 8 and enumerate game controllers.
2. `input_pads_init` (0xC131B0):
   1. **No DirectInput pad** (count `0x252A584` is 0): use one XInput pad (mode `0x23486FC` = 3).
   2. **Otherwise**, for each pad:
      1. `EnumObjects`, then `SetCooperativeLevel(hwnd, DISCL_NONEXCLUSIVE | DISCL_BACKGROUND (9))`.
      2. `Acquire` and `GetCapabilities`.
      3. Classify it by name ("xbox 360" and other names; 0xC14AB0) into a layout 0-4.
   3. Dead zones start at 0.25.
3. 0xC12090 and 0xC120D0.
4. Load **`input.bin`** from the save folder (13_savegame.md). It must be exactly 0x2C8 bytes:
   - 0x2B8 bytes of bindings → `0x234E778`;
   - 16 bytes → `0x234E768`, four stick-axis indices that remap the XInput thumbsticks.

   If the file is missing or the wrong size, `input_save_bindings` (0xC12200) writes the defaults back out. The same function serves the Lua "restore defaults" and options bindings.
5. 0xC13500.

## Per-frame update

`game_frame_tick` calls **`input_do_frame`** (0xC063D0) early in the frame (11_frame.md), and `mouse_update` (0xC1F320) a little later.

**`input_do_frame`:**
1. Clear the "eaten" flags of all 87 buttons and all 8 sticks (left from last frame's consumers).
2. **`input_poll_devices`** (0xC14780). Only runs while the window is active (`0x252A406`).
   1. **Keyboard events**: `input_keyboard_events` (0xC11710) replays the queued key events into the key table (0xC10B80) and releases keys from the key-up ring.
   2. **Map keyboard and mouse to buttons**: `input_map_keyboard_mouse` (0xC13660). It is skipped while a rebind is being captured; then every button value just moves to "previous".
      1. Each button value is 1.0 when its bound key is down (`input_key_down` 0xC111D0: keys 0-255, 0x100 / 0x101 / 0x102 = mouse buttons).
      2. Mouse X / Y → the look stick: (dx / 30) × sensitivity (`0x25F5C98`, `0x25F5C9C`), into stick +0x40.
      3. Bound keys → the move stick (stick 0).
      4. The wheel and mouse buttons set some fixed buttons.
   3. **Pads**: in mode 3, `input_poll_xinput` (0xC13C80). Otherwise the DirectInput pad (0xC141D0) for pad record `0x23478A8`.
      - XInput buttons go into a byte array (pad +0xF68): A, B, X, Y, shoulders, triggers (> 30 / 2), D-pad, start / back, thumbs.
      - Sticks are scaled by 1/32767 and remapped through the four axis indices from `input.bin`.
      - Button values come from the bindings at `0x234E784…` (×1/128).
   4. **Rebind capture**: while the controls menu is waiting for a key (`0x252A58F` keyboard, `0x252A5B3` callback, `0x252A5B0` pad), the matching capture function runs instead, then 0x75C700.
3. **Sticks**: for each of the 8 sticks:
   1. Apply the eaten (+0xE/+0xF) and invert (+0xC/+0xD) flags.
   2. Run four direction auto-repeat timers (`input_repeat_timer` 0xC061C0: first repeat after 500 ms, then every 150 ms).
   3. Any axis past 0.5 sets "any input" `0x23486F8`.
4. **Buttons**: for each of the 87 buttons, the value goes past the threshold → pressed, just pressed / released, repeat pulses, and timestamps.
5. **Idle time**: `0x23486F4` counts seconds without any input and resets on input.

**`mouse_update`** (0xC1F320), under the critical section `0x234F9E0`:
1. When the window is inactive, zero everything.
2. Otherwise, if the cursor is inside the client area (or mouse look is captured, `0x252A5D6`): the mouse delta (`0x234F45C` x, `0x234F458` y) is the cursor minus the window centre.
3. The wheel delta (`0x234EA34`) is the change of the wheel accumulator `0x234F460`, which the window procedure adds to.
4. With `0x252A5D5` set, the cursor is warped back to the centre (`SetCursorPos`). That warp is the mouse-look capture.

## Input state structs

**Buttons**: 87 records of 0x24 bytes per pad (pad stride 0x104C), from `0x23478B8`:

| Off | Field |
|---|---|
| +0x00 | value (float, 0..1) |
| +0x04 | previous value |
| +0x08 | press threshold |
| +0x0C | repeat interval (ms) |
| +0x10 | first repeat delay (ms) |
| +0x14 | press time |
| +0x18 | last repeat time |
| +0x1C | u8 pressed this frame |
| +0x1D | u8 pressed or repeated this frame |
| +0x1E | u8 released (set by input_do_frame, never cleared there; input_pad_reset_buttons clears it) |
| +0x1F | u8 held |
| +0x20 | u8 **eaten** (`0x23478D8`; set by vint, see 06_vint.md) |

**Sticks**: 8 records of 0x40 bytes from `0x23484F4`:

| Off | Field |
|---|---|
| +0x0 | x |
| +0x4 | y |
| +0xC / +0xD | invert x / y |
| +0xE / +0xF | eaten x / y (`0x2348502` / `0x2348503`) |
| +0x10 | four 12-byte repeat timers |

Stick 0 is the move stick; stick +0x40 (`0x2348534`) is the camera / look stick.

**Keys**: 256 records of 12 bytes at `0x2348B80`:

| Off | Field |
|---|---|
| +0 | u8 down |
| +2 | u8 eaten (`0x2348B82`) |
| +4 / +8 | times |

**Mouse buttons**: `0x234F473` / `0x234F47B` / `0x234F483` (left, right, middle; 8-byte stride).

## Bindings (input.bin)

`input.bin` is 0x2C8 bytes:

| Range | Contents |
|---|---|
| 0x000-0x2B7 | `0x234E778`: **87 entries of 8 bytes**, one per button record, `{u32 key, u32 pad_button}` |
| 0x2B8-0x2C7 | `0x234E768`: four u32 stick-axis indices (left x, left y, right x, right y) into the pad's raw axes |

- **key** is a DirectInput scan code (`DIK_*`, 0-255), or 0x100 / 0x101 / 0x102 for the left / right / middle mouse button. -1 means unbound.
- **pad_button** is an index into the pad's raw button bytes (pad +0xF68), or -1.

**Defaults:**
- `input_default_key_bindings` (0xC12090) copies 87 keys from `0xE829A0`: W, S, A, D, E, Space, Q, Space, Right, Up, Left, Down, Enter, Esc, … (`DIK_W` = 0x11, …). It then calls 0xBA45C0(0xBA2F60).
- `input_default_pad_bindings` (0xC120D0) copies the 87 pad buttons and the four axis indices for the pad layout `0x23486FC`:

  | Layout | Buttons | Axes |
  |---|---|---|
  | 0 (and 3 = XInput) | `0xE82B38` | `0xE830CC` |
  | 1 | `0xE82CB0` | `0xE830DC` |
  | 2 | `0xE82E10` | `0xE830EC` |
  | 4 | `0xE82F70` | `0xE830FC` |

`input_dinput8_create` runs both default functions, then overwrites the block with `input.bin` when the file exists and is the right size. Otherwise `input_save_bindings` (0xC12200) writes the defaults out.

**How they are used**: `input_map_keyboard_mouse` sets button record *i* to 1.0 when `input_key_down(bindings[i].key)` is true. The XInput / DirectInput pad code sets it to `raw[bindings[i].pad_button] / 128`. The two sources fill the same 87 records, so gameplay never sees which device a press came from.

## Window procedure side

`input_wndproc_keys` (0xC10CD0) takes (msg, lParam) cdecl. WinMain registers it as a message callback, and it only runs while `0x252A54E` is set. The scan code is `(lParam >> 16) & 0xFF`, plus 0x80 for extended keys (lParam bit 24).

| Message | Action |
|---|---|
| WM_KEYDOWN (0x100), WM_SYSKEYDOWN (0x104) | `input_key_queue_push`(scan, down = 1) into the event ring `0x2348A80`. A left Ctrl (0x1D) sent while right Alt is held (`GetAsyncKeyState(VK_RMENU)`) is dropped (AltGr). |
| WM_KEYUP (0x101), WM_SYSKEYUP (0x105) | push `{scan, delay 1}` into the key-up ring `0x2348980` |
| WM_SETFOCUS (7), WM_KILLFOCUS (8) | `input_keyboard_reset` |

The mouse has its own callback, 0xC19350 (also registered from WinMain). It accumulates the wheel into `0x234F460` and sets the button bytes `0x234F473 / 0x234F47B / 0x234F483`.

`input_keyboard_events` (0xC11710) then runs once per frame:
1. Pass every queued down event to `input_key_event` (0xC10B80; cdecl (scan, down)). It sets key +0, records the press time, and pushes a typed-key code with modifier bits into the text ring `0x2349780` (32 entries, write `0x234897C`, read `0x2349800`): 0x1000 Shift, 0x2000 Alt, 0x4000 Ctrl, 0x8000 the grave key.
2. Release keys from the key-up ring, but only after they have been down for at least one frame. The delay byte counts down.

While a rebind is being captured, key events go to `0x2349848[scan]` instead.

## The free-camera control table

`input_get_control_value` (0xC010C0) and its table at `0x23476FC` are **not** the gameplay bindings. They are the **debug / free camera's** 15 controls:
- `camera_controls_init` (0xC01570, from game init 0x51F700 and 0x493120) copies the table from the static data at `0xDC8308`, sets two sensitivities (`0x234789C` / `0x23478A0`), and sets `0x252A53A`.
- The only reader is the free camera, 0xC01650 (from 0x5011A0).

Each entry is 0x1C bytes:

| Off | Field |
|---|---|
| +0x00 | pad button record index |
| +0x04 | key |
| +0x08 | stick direction 0-7 |
| +0x0C | mouse direction 0-5 (X+, X-, Y+, Y-, wheel up, wheel down) |
| +0x10 | mouse button |
| +0x14 | scale (1.0) |
| +0x18 | id |

The defaults are W / S / A / D on the move stick, the numpad arrows on the look stick and mouse, LShift / LCtrl, Tab, Space and Q / E.

Mouse look in **gameplay** goes through the look stick (`0x2348534` / `0x2348538`), which `input_map_keyboard_mouse` fills from the mouse delta: (dx / 30) × sensitivity `0x25F5C98` / `0x25F5C9C`.

## Porting order (input_do_frame's tree, leaves first)

| Function | Convention / registers | Output |
|---|---|---|
| `fabsf` 0x49EE60 | cdecl (float) | ST0 |
| `input_key_down` 0xC111D0 | EAX = key | AL |
| `input_repeat_timer` 0xC061C0 | ESI = 12-byte timer, XMM0 = axis value | writes the timer (+8 = pulse) |
| `input_button_analog` 0xC06870 | EAX = button record | ST0 value |
| `input_key_queue_push` 0xC10B10 | cdecl (scan byte), DL = down | ring write |
| `input_key_event` 0xC10B80 | cdecl (scan, down) | key table, text ring |
| `input_keyboard_reset` 0xC10EB0 | none | |
| `input_keyboard_events` 0xC11710 | none | |
| `input_stick_deadzone` 0xC13A90 | EDI = &axis, cdecl (deadzone) | axis in place |
| `input_pad_reset_buttons` 0x75C700 | EAX = pad record | clears the 87 records |
| `input_map_keyboard_mouse` 0xC13660 | none | button records, sticks |
| `input_poll_xinput` 0xC13C80 | cdecl (pad record) | pad raw bytes, sticks, button records |
| `input_poll_dinput` 0xC141D0 | cdecl (pad record); device at +0xE58: Poll (vtable +0x64), GetDeviceState | same |
| `input_rebind_capture_keys` 0xC12910 / `input_rebind_capture_pad` 0xC12B00 | none | the bindings block |
| `input_poll_devices` 0xC14780 | none | |
| `input_do_frame` 0xC063D0 | none | button records, sticks, idle time |
| `mouse_update` 0xC1F320 | none (critical section `0x234F9E0`) | mouse deltas, wheel delta |

The pad record base is `0x23478A8` (stride 0x104C). Its 87 button records start at +0x10 (`0x23478B8`) and its 8 sticks at +0xC4C (`0x23484F4`).

## In Rust (input.rs)

`input_do_frame` and its tree run in Rust: keyboard events, key events, the keyboard/mouse
mapping, the repeat timers, button and stick state, idle time; also the window procedure's
key callback, the key queue, `input_keyboard_reset`, `input_key_down`, `input_button_analog` and
`input_pad_reset_buttons`, the XInput poll with its dead zone, and `mouse_update`. Still original:
the DirectInput poll (called from the Rust `input_poll_devices` when the pad is not XInput) and the
rebind captures.

**How it was checked.** In shadow mode each hooked call runs the original on a snapshot of the
input state, records every `QueryPerformanceCounter` read (the pointer at `0xDB8114` is wrapped),
restores the snapshot, runs the Rust with the same clock readings replayed, and compares the
two results byte for byte. `tools/ingame/run_input.ps1` drives a scripted stretch of play (walk,
sprint, jump, mouse look, wheel, clicks, the pause menu, Ctrl and H / J). Result: 8,902 frames,
272 window messages, ~850,000 key queries, 0 differences, the clock replay exact.

**The call tape.** The second round also replays the Windows calls: the import slots for
QueryPerformanceCounter, GetCursorPos, the two rectangle queries (0xDB8298 window rect,
0xDB8308 client rect), ClientToScreen, SetCursorPos, XInputGetState and GetAsyncKeyState are
wrapped. During a check the original's calls are recorded (kind, inputs, results) and the Rust's
are answered from the recording, which also checks they come in the same order with the same
inputs (SetCursorPos runs once). For the controller, `pad_selftest` (frame 600, shadow builds
only) runs 5,000 XInput polls against a fake pad: random buttons and triggers, sticks at 0, inside
the dead zone or at the extremes, one poll in 8 disconnected, one in 16 during a rebind capture.
Result: 0 differences, 0 tape mismatches; the play run's 7,199 `mouse_update` calls also 0.

- `input_stick_deadzone` (EDI = &a, ESI = &b, cdecl (dz)): both |a| and |b| below dz -> zero.
  Otherwise b == 0 becomes 1e-5; m = sqrt(a² + b²) clamped to 0..1; angle = atan(a/b), + pi
  when b < 0; t = the angle folded into ±45° (minus 90° while above 45°, plus 90° once if below
  -45°, then clamped); e = dz / cos t; r = (m - e) / (1 - e); b = cos(angle)·r, a = sin(angle)·r.
  It uses the game's CRT sqrtf 0x40EA20, atan 0xDA0DC0 (ST0 in and out), cosf 0x40B820 and
  sinf 0x428CD0, so Rust calls those.
- `input_poll_xinput`: the raw bytes are A, B, X, Y, LB, RB, Back, Start, LS, RS (bit 7 each), the
  triggers (> 30, halved) at +0xA / +0xB, and the D-pad right, up, left, down at +0x10..+0x13.
  The axes {LY, LX, RY, RX} / 32767 are remapped into sticks 4 (x, y) and 3 (x, y), dead-zoned
  with each stick's +8, then copied into sticks 0 / 7 (from 4), 1 / 6 (from 3) and 2 (from 0)
  only where those are still exactly zero. Buttons take raw / 128 only where still zero (so the
  keyboard wins). In a pad rebind capture the raw bytes become bools at 0x2349808 and the D-pad
  bits at 0x2349818..1B, and nothing else changes.
- `mouse_update`: rect A (0xDB8298) gives the centre the delta is measured from and the warp
  target; rect B (0xDB8308) with ClientToScreen gives the client area the cursor must be in.
  The warp path computes the client corners in screen space and never uses them.

Corrections and details found on the way:
- **Key events arrive on another thread** (the window procedure). `input_wndproc_keys` writes
  key-ups straight into the key-up ring (0xC10D2F), not through `input_key_queue_push`. The
  shadow check holds a lock that the key callbacks take; without it an event landing mid-check
  was lost.
- The button record's +0x0C is the repeat interval and +0x10 the first delay (the table above
  had them swapped), and +0x1C / +0x1D are "pressed this frame" / "pressed or repeated".
- `input_key_down`: key 0x101 reads `0x234F483` and 0x102 reads `0x234F47B`. For keys >= 0x100
  EAX keeps the key's upper bits (callers test AL).
- `input_map_keyboard_mouse` also fills: stick 2 from the mouse direction (|delta| > 5),
  stick 5 x from H / J (keys 0x23 / 0x24), stick 6 from the raw mouse delta (dx / 10,
  -dy / 30), stick 7 as a copy of the look stick, button 19 (left and right mouse together),
  and buttons 33 / 34 (wheel up / down). The move stick is y = bindings[0] / [1] and
  x = bindings[3] / [2].
- The stick values that `input_do_frame` derives (eaten / inverted) only drive the repeat timers
  and "any input"; they are not written back.
- The x87 precision control during the game is 53-bit (precision field 2), so unstored x87
  intermediates are doubles.
- A left Ctrl key-down is dropped while `GetAsyncKeyState(VK_RMENU)` is negative (AltGr).

## Consumers and "eating"

- **vint** runs `vint_input_do_frame` after `input_do_frame`. When a Lua subscription consumes an event, `vint_input_mark_consumed` sets the eaten flags of its buttons (+0x20), sticks (+0xE/+0xF) and keys (+2). From then on the game reads them as not pressed / zero that frame (06_vint.md).
- **The game** reads the button records and sticks directly (the free camera also goes through `input_get_control_value`). That is why eating at the record level hides input from gameplay.
- `0x252A54F` blocks all keyboard reads (text entry).

## Game actions and the player control block

Gameplay and menus do not read the button records directly. They go through **game actions**, which `player_controls_update` (0x4FE760) prepares once per frame from the pad record, and a set of small accessors.

### The action table `0xF9EA38`

- **132 entries of 16 bytes** (0x840 bytes):
  - +0: primary button record (0-86, or −1 = unbound);
  - +4: secondary button record;
  - +8: stick direction code (−1..13), checked by the `input_action_stick_dir_*` fallbacks 0x4FE380..0x4FE6A0;
  - +0xC: name pointer (see the instruction-level notes below).
- `input_actions_reset` (0x500170, from game init 0x51F600) sets every entry to −1 / −1 / −1 / 0 and calls `input_actions_load_defaults` (0x4FF3F0).
- `input_actions_load_defaults` copies the 87 defaults from `0xDD3B98` (20-byte records `{action, primary, secondary, dir, flags}`) and 23 stick actions from `0xDD42A0` into `0xF9F278`. Then it applies the control scheme (0x500340, 0x500630; also run by the pause menu's change-scheme / restore-defaults / revert bindings).
- Setters: `input_action_set` (0x4FF4B0, ECX = record), `input_actions_set_many` (0x4FF4E0, ECX = records, EDX = count), and `input_stick_action_set` / `_set_many` (0x4FF520 / 0x4FF550).
- `input_action_table` (0x4FF590) returns the table.
- The stick-action table `0xF9F278` has 16-byte entries `{u32 kind, u8, u32, u8}`. `input_stick_action_consume` (0x500230) sets the eaten flags of the move or look stick by kind 0-3.

### The control block `0x2659908`, written by `player_controls_update`

The pad is `0xE98DB0` (−1 → pad 0). The control block is cleared or rebuilt every frame.

**Sticks: 8 entries of 0x28 bytes from `0x2659908`.**

Entries are filled for pad sticks 0, 1, 3, 4, 5, 6 and 7. Stick 2 is not copied, and stick 7 only gets its raw copy.

| Off | Field |
|---|---|
| +0x00 / +0x04 | x / y: eaten → 0, inverted → negated. If the sign flipped since last frame, the value is 0 for this frame. |
| +0x08 / +0x0C | last frame's x / y |
| +0x10 / +0x14 | x / y without the sign-flip filter |
| +0x18 / +0x1C | step x / y (sticks 0 and 1): −1 / +1 from the stick's four auto-repeat timers (pulses at stick +0x18 / +0x24 / +0x30 / +0x3C), else 0 |
| +0x20..+0x23 | u8 per-direction "taken" flags, cleared every frame |
| +0x24 / +0x25 | u8 x / y taken, cleared every frame; `player_stick_get` / `player_stick_get_raw` return 0 while set |

- `player_stick_get` (0x4FF5F0): EAX = entry, ECX = &y, cdecl (&x); returns +0 / +4.
- `player_stick_get_raw` (0x4FF5A0): same, returns +0x10 / +0x14. It has 15 callers, among them the vehicle customisation lights, tagging and the minigames.

**Buttons: six arrays of 87 entries (0x57), one per button record.**

| Address | Contents |
|---|---|
| `0x2659A48` | down: value > threshold (+8) and not eaten |
| `0x2659A9F` | pressed or repeated this frame (+0x1D), not eaten |
| `0x2659AF6` | "available" mask for down / repeat: set to 0xFF; a *take* clears it |
| `0x2659B4D` | just pressed (+0x1C), not eaten |
| `0x2659BA4` | "available" mask for just-pressed: set to 0xFF |
| `0x2659BFC` | u32 per button: the frame time `0x252A3C8` of the last press |

- Buttons 0x23 and 0x24 (the analog triggers) have an adaptive threshold. While a trigger is up, its threshold drops to at most value + 0.1; while it is down, it rises to at least value − 0.1. So a half-held trigger can be pressed again without fully releasing it.

### The accessors

All of them map the action through the table. A button counts if either the primary or the secondary record matches. When neither does, the stick-direction fallback decides.

| Function | Args | Returns |
|---|---|---|
| `input_action_value` 0x4FF770 | EAX = action | ST0 primary value (0 if eaten) |
| `input_action_alt_value` 0x4FF810 | EAX | ST0 secondary value |
| `input_action_prev_value` 0x4FF7C0 | EAX | ST0 previous primary value. No direct callers. |
| `input_action_crossed` 0x4FF8B0 | EDX = action, cdecl (threshold) | 1 when the primary went from below the threshold to at or above it this frame |
| `input_action_released_below` 0x4FF930 | cdecl (action, threshold) | 1 when either button went from at or above the threshold to below it |
| `input_action_down` 0x4FF640 | ECX | AL: down & available |
| `input_action_repeat` 0x4FF6A0 | ECX | AL: pressed-or-repeated & available |
| `input_action_just_pressed` 0x4FF700 | ECX | AL: just pressed & available |
| `input_action_take_repeat` 0x4FFA30 | EAX | like `_repeat`, and clears both availability masks when true |
| `input_action_take_pressed` 0x4FFAF0 | EAX | like `_just_pressed`, and clears both masks when true |
| `input_action_press_time` 0x4FFB80 | EAX | time of the last press |
| `input_action_button_name` 0x4FFBB0 | EAX | wide string of the bound button (0x7F36D0), or "Button String Unbound" |
| `input_action_consume` 0x5001D0 | EAX | clears both masks for the action's button |
| `input_consume_all` 0x5002B0 | none | clears all masks and sets the move / look stick taken flags; menus call it when they open |

The *take* forms let one consumer per frame have a press, for example a menu over gameplay. The non-take forms only check the masks, so an earlier take hides the press from them.

### Who reads which action

Found from the immediate action ids at the call sites of the accessors (`item11b.py`, about 520 sites; some pass the action in a variable). Action names are not known yet.

| Actions | Readers |
|---|---|
| 2, 0x2D, 0x2E | `player_controls_interact` (0x4F82C0, diversions and pick-ups), diversion instances |
| 3, 6, 7, 9, 0xB, 0xC, 0x11, 0x12 | `player_controls_on_foot` (0x4F9380), `player_controls_attack` (0x4F8CA0). 6 / 7 are analog (value, crossed, released); 0x12 is used by both. |
| 4, 5 | crossed in 0x4F9380, 0x4F9C80, 0x9DA290 |
| 0x18-0x20, 0x2A | `player_controls_vehicle` (0x4F9C80). 0x18 / 0x19 are read as values. |
| 0x24, 0x27, 0x28 | `vehicle_player_input` (0xAA6540): 0x24 primary and secondary values, so the analog pedal pair |
| 0x22, 0x23, 0x2B, 0x2C | 0xAA55E0 (vehicle) and 0x498770 |
| 0x2E, 0x2F, 0x32, 0x33 | `player_controls_weapon_select` (0x4F7E90), weapon slots through `inventory_find_weapon`, and the number keys through `input_key_down` |
| 0x36-0x52 | menus: options, pause, garage, the overlay screens, multiplayer lobby. 0x36 / 0x37, 0x38 / 0x39 and 0x3A / 0x3B are taken with repeat, so they are likely the navigation directions. 0x48 / 0x49 are the most-used takes, likely select / back. |
| 0x59 | in `player_controls_do_frame` while the cellphone or map is up |
| 0x5E-0x63 | diversions and minigames (0x560300, 0x560360, 0x5E1D30, tagging) |
| 0x71-0x77 | 0x62DCA0, 0x6321A0, 0x62E0B0, 0x426CC0 |

### The player's per-frame path

- `game_frame_tick`:
  1. `input_do_frame` (Rust).
  2. `vint_input_do_frame`: UI subscriptions eat records.
  3. **`player_controls_update`**: builds the block above.
  4. `input_direction_history_update` (0x500CA0): two channels with 20-entry rings at `0xF9F4C0`. Each pushes the current code from 0x5008D0 when it changes, then runs the matcher 0x500AF0. Inferred to be a stick-gesture / sequence detector.
- `sim_frame:1,2,3` → **`input_context_do_frame`** (0x4F5040): a small state machine over the input context table `0xE98E50` (16 bytes per context: enter, exit, update, …).
  - The wanted context is the override `0xE9D47C` (when `0xE9D480` is set), else the top of the context stack `0x26598B4` (count `0x26598BC`, 8 bytes per entry), else 2.
  - The current context is `0xF9EA00`; the previous one is `0xF9E9FC`.
  - Contexts 0 and 1 enter through 0x4F4F70 (`input_consume_all`, then one `player_controls_do_frame`) and update with 0x4FAB20.
  - Context 2 (gameplay) updates with **`player_controls_do_frame`** (0x4FB160).
- `player_controls_do_frame`:
  - Does nothing while `0x2527C08` > 0 or `0x2528989` is set.
  - Otherwise works on the local player `0x21703D4`:
    1. clears its control block at human +0x139C..+0x13AC;
    2. handles death (0x9DA290) and the vehicle it is in;
    3. takes action 0x59 while the phone / map UI is up;
    4. runs **`player_controls_dispatch`** (0x4FAA90): `player_controls_weapon_select` → `player_controls_on_foot` → `player_controls_vehicle` → 0x9F1D00 → `player_controls_interact`;
    5. runs 0x4FAE90.
- `player_controls_vehicle` → `vehicle_player_input` (0xAA6540) stores throttle, steering and brake in the vehicle; `vehicle_drive_update` applies them (16_vehicles.md).

### The accessors at instruction level (for the port)

**Common addressing:**
- pad = `[0xE98DB0]`, with −1 → 0 (`sub -1 / neg / sbb / and`). Record = `0x23478B8 + pad × 0x104C + button × 0x24`.
- action entry = `0xF9EA38 + action × 16`: +0 primary, +4 secondary, +8 stick-direction code, **+0xC name pointer**. The earlier "flags" for +0xC was wrong: `input_actions_reset` clears it, and `input_action_button_name_by_name` compares it with `_stricmp`.
- Control-block entries are `0x2659908 + entry × 0x28`.

**Float accessors.** Each pushes `ECX` as a 4-byte temporary and returns ST0 after `fstp [esp]` / `fld [esp]`, so the value is **rounded to f32**. Unbound → `fldz`, also through the f32 store.

| Function | In | Reads | Eaten test |
|---|---|---|---|
| `input_action_value` 0x4FF770 | EAX = action | primary +0 (value) | +0x20 == 1 → 0 |
| `input_action_prev_value` 0x4FF7C0 | EAX | primary +4 (previous) | none |
| `input_action_alt_value` 0x4FF810 | EAX | secondary +0 | +0x20 == 1 → 0 |
| `input_action_alt_prev_value` 0x4FF860 | EAX | secondary +4 | none |

Only the value reads test eaten, with `== 1`, not `!= 0`.

**`input_action_crossed` 0x4FF8B0** (`EDX` = action, cdecl (f32 t), plain `ret`):
1. p = primary previous (f32-rounded; unbound → 0).
2. If not t > p → 0. NaN p fails here.
3. v = `input_action_value(EAX = EDX)`.
4. Result 1 if v ≥ t; NaN v → 0.
5. The result is **written into the caller's argument slot** `[esp+4]` and returned with `fild`, i.e. **0.0 / 1.0 in ST0, not AL**.

**`input_action_released_below` 0x4FF930** (cdecl (action, f32 t), plain `ret`):
1. Reads p1 = primary previous, v1 = `input_action_value`, p2 = secondary previous, v2 = `input_action_alt_value`.
2. p1 is stored **over the action argument slot**.
3. Result = (p1 ≥ t && v1 < t) || (p2 ≥ t && v2 < t). NaN compares count as false, but a NaN p1 still lets the secondary test run.
4. Returned with `fild` as 0.0 / 1.0 in ST0.

**Boolean accessors (`AL`).** Masks: avail A = `0x2659AF6[b]`, avail B = `0x2659BA4[b]`. All buttons are indexed by record number b.

| Function | In | Button test | Takes? | Stick fallback (`EAX` = entry +8) |
|---|---|---|---|---|
| `input_action_down` 0x4FF640 | ECX | down `0x2659A48` & A | no | tail-jump to `_stick_dir_down` 0x4FE380, only when false |
| `input_action_repeat` 0x4FF6A0 | ECX | repeat `0x2659A9F` & A | no | tail-jump to `_stick_dir_repeat` 0x4FE5E0, only when false |
| `input_action_just_pressed` 0x4FF700 | ECX | just `0x2659B4D` & B | no | tail-jump to `_stick_dir_just_pressed` 0x4FE460, only when false |
| `input_action_take_repeat` 0x4FFA30 | EAX | repeat & A, clears A and B of that button | yes | `_stick_dir_take_repeat` 0x4FE6A0, see below |
| `input_action_take_pressed` 0x4FFAF0 | EAX | B & just, clears A and B | yes | `_stick_dir_take_pressed` 0x4FE520, **always called**; result ORed in |

**Quirks to keep:**
1. **Button selection.**
   - The "first button" is the primary, or the secondary when the primary is −1. The second test is always the secondary. So with an unbound primary, the secondary is tested twice; harmless, because the masks are the same and a take already cleared them.
   - The mask is read before the state, as `test byte [state], mask` → `setne`.
2. **`input_action_take_repeat`:**
   - Per button: if repeat & A, take it.
   - Else, if the button is **held (down & A) but has no repeat pulse this frame**: call the stick fallback (it may take a stick pulse), **ignore its result** and return 0.
   - Otherwise the fallback is always called, and the result is button-taken || fallback.
   - So a held button blocks the stick for that action until its next repeat pulse.
3. **`input_action_take_pressed`** always calls the stick fallback, even when a button already answered. It can take a stick press in the same frame as a button press.
4. **`input_action_press_time`** 0x4FFB80 (`EAX` = action):
   - primary == 0 → returns 0, so button record 0 is treated as "none";
   - primary == −1 → reads `0x2659BFC[−1]` = the dword at `0x2659BF8`, which is the last three bytes of mask B plus a pad byte. That is a real out-of-bounds read of the original.
5. **`input_action_consume`** 0x5001D0 (`EAX`) clears A and B of the **first button only** (primary, or the secondary when the primary is −1). The takes, by contrast, clear whichever button they matched.
6. **`input_consume_all`** 0x5002B0: memsets A and B to 0 (0x57 bytes each), then sets the taken bytes +0x20..+0x25 of entries 0 and 1 to 1.
7. **`input_actions_reset`** 0x500170: for all 0x84 entries, +0 / +4 / +8 = −1 and +0xC = 0. Then `[0xF8549B]` = (b & 0xF1) | 1, then a **tail call** to `input_actions_load_defaults`.
8. **`input_action_button_name`** 0x4FFBB0 (`EAX`): unbound → `L"Button String Unbound"` (0xDDFE9C); otherwise cdecl `0x7F36D0(primary)`.
9. **`input_action_button_name_by_name`** 0x4FFBD0 (cdecl (name)):
   - finds the first entry whose +0xC name matches by `_stricmp` (0xC99030);
   - not found → 0xDDF0B8;
   - found → **writes the primary over its own argument and tail-jumps** to 0x7F36D0.
10. **`input_just_pressed_count`** 0x4FD690, reached through the jump 0x4FF760: the number of non-zero bytes in the just-pressed array. It ignores the masks.

### Stick accessors on the control block

- **`player_stick_get` 0x4FF5F0 / `player_stick_get_raw` 0x4FF5A0:** `EAX` = entry, **cdecl (&x), `ECX` = &y**. This corrects the note above, which had the pointers swapped.
  - x = taken byte +0x24 ? 0 : +0 (raw: +0x10); y = +0x25 ? 0 : +4 (raw: +0x14).
  - x is written through the stack pointer. y is first stored **over that argument slot**, then written to `[ECX]`.
- **Per-axis readers** (`EAX` = entry → ST0, 0 when the axis byte is set): x 0x4FD510 (+0x24, +0), y 0x4FD540 (+0x25, +4), raw x 0x4FD570 (+0x10), raw y 0x4FD5A0 (+0x14). The curved forms 0x4FD630 / 0x4FD660 apply `stick_response_curve`.
- **`stick_response_curve` 0x4FD5D0** (stdcall (f32 v), `ret 4`):
  1. c = f32(v·v·0.9·v), with 0.9 a double at 0xE5C210.
  2. If c > 0: c = f32(c + 0.1).
  3. Then if c < 0: c = f32(c − 0.1).

  So any non-zero input jumps to at least ±0.1, and 0 stays 0.
- **Stick actions** `0xF9F278[i]` (16 bytes): +0 primary axis kind, +4 u8 invert, +8 secondary kind (0-3), +0xC u8 invert.

  | Kind | Stick and axis |
  |---|---|
  | 0 / 1 | entry 1 x / y |
  | 2 / 3 | entry 0 x / y |
  | 4 / 5 | entry 5 x / y |
  | 6 / 7 | entry 3 / entry 4 y |
  | 8 / 9 | entry 3 / entry 4 x |
  | 10 / 11 | entry 6 x / y |

  - `input_stick_action_value` 0x4FFE60 (`EAX` = i → ST0):
    1. Read the primary kind's axis, f32-rounded.
    2. If it is non-zero, negate it when +4 is set.
    3. Otherwise read the secondary kind and negate it when +0xC is set.
    4. A secondary kind outside 0-3 gives 0 (negated if +0xC).
  - `input_stick_action_value_curved` 0x4FFCC0: the same, but kinds 0-5 and the secondary go through the curve. Kinds 6-11 are not curved.
  - `input_stick_action_value_raw` 0x4FFFF0: kinds 0-3 only, with raw +0x10 / +0x14. Then `float_clamp_in_place(ECX = &v, cdecl (−1.0, 1.0))` (0x417120).
  - `input_stick_action_consume` 0x500230: kinds 0-3 set the taken byte +0x24 / +0x25 of entry 1 / entry 0. Other kinds do nothing.
  - 0x4FFFD0: `input_stick_action_value(13)`, then `input_stick_action_consume(13)`, returning the value in ST0. Used by `player_controls_vehicle`.

### Stick-direction fallbacks (`EAX` = the action's +8 code)

The code is the action's +8 field; −1 means none. Each dispatcher does `EAX + 1`, a bounds check, and a jump table. The leaf it jumps to takes `EAX` = control-block entry.

| Code | Entry | Direction |
|---|---|---|
| 0 / 1 / 2 / 3 | 0 | +y, −y, +x, −x |
| 4-7 | 1 | the same order |
| 8-11 | 5 | the same order |
| 12 / 13 | 6 | +x, −x (held test only) |

| Dispatcher | Codes | Leaf test (per direction, with its taken byte +0x20 −x, +0x21 +x, +0x22 +y, +0x23 −y; a set byte → false) |
|---|---|---|
| `_stick_dir_down` 0x4FE380 | 0-13 | held: axis < −0.4 or > 0.4 (doubles at 0xE5C350 / 0xE5C358; NaN → false). Leaves 0x4FD6D0 −x, 0x4FD700 +x, 0x4FD730 −y, 0x4FD760 +y |
| `_stick_dir_just_pressed` 0x4FE460 | 0-11 | just: past ±0.4 now and not past it last frame (+8 / +0xC previous). Leaves 0x4FD790 −x, 0x4FD7D0 +x, 0x4FD810 +y, 0x4FD850 −y |
| `_stick_dir_take_pressed` 0x4FE520 | 0-11 | as just, and on true sets the taken byte to 1. Leaves 0x4FD890 −x, 0x4FD8E0 +x, 0x4FD930 +y, 0x4FD980 −y |
| `_stick_dir_repeat` 0x4FE5E0 | 0-11 | step value: x step +0x18 < 0 → −x, > 0 → +x; y step +0x1C < 0 → **+y**, > 0 → −y. Leaves 0x4FD9D0 / 0x4FD9F0 / 0x4FDA10 / 0x4FDA30 |
| `_stick_dir_take_repeat` 0x4FE6A0 | 0-11 | as repeat, and on true zeroes the step and sets the taken byte. Leaves 0x4FDA50 / 0x4FDA90 / 0x4FDAD0 / 0x4FDB10 |

**Quirks to keep:**
- **"Previous" in the just leaves is +8 / +0xC**: last frame's filtered +0 / +4, which `player_controls_update` copies before computing the new values.
- **The y step sign is inverted relative to y.** +y repeats on a negative step: the step is −1 from the stick timer pulsed at stick +0x18, which `player_controls_update` copies into +0x1C.
- **Codes 12 / 13 exist only for the held test.** The other four dispatchers stop at 11, and anything outside their range returns 0 (`xor al, al`).
- **Leaf return types differ.** The held / just / repeat leaves return `EAX` = 0 / 1. The take leaves return only `AL`, and their false path leaves `AH` as it was.

### Reaching the accessors in a scripted run (keyboard)

**Which keys.**
- Keys below are the **shipped defaults**: the record → key table `0xE829A0` (`input_default_key_bindings`) plus the scheme tables of `input_actions_load_defaults`.
- An `input.bin` in the game folder overrides the record → key part, so check it first.
- Records are button-record indexes (`r`).

**Keyboard on-foot scheme** (table `0xDD44A8`, scheme 0):

| Action | Name | Key (record) |
|---|---|---|
| 0 | weapon menu | Q (r6) |
| 2 | reload | R (r14) |
| 3 | shoulder camera | V (r16) |
| 4 / 5 | zoom in / out | H (r66) / N (r67) |
| 6 | attack primary | LMB (r35) |
| 7 | attack secondary | RMB (r36) |
| 8 | jump / climb | Space (r5) |
| 9 | pratfall | LMB + RMB (r35 / r36) |
| 0xB | multi pratfall | LCtrl (r17) |
| 0xC | sprint | LShift (r18) |
| 0xD | walk | Caps (r63) |
| 0x11 | crouch | C (r15) |
| 0x12 | pick up Havok object | LCtrl (r17) |
| 0xE-0x10 | taunts | Up / Left / Right (r52 / r54 / r55) |

**Default-table actions:**

| Action | Name | Key (record) |
|---|---|---|
| 0x2D | action / use | E (r4) |
| 0x2E | recruit | Up (r52) |
| 0x2F | cancel | Down (r53) |
| 0x30 | pause | Esc (r21) |
| 0x31 | pause map | Tab (r20) |
| 0x32 / 0x33 | weapon cycle | records 33 / 34, which are unbound by default |
| 0x36 / 0x37 | menu up / down | W / S (r70 / r71), or Up / Down (r9 / r11) |
| 0x48 | select | Enter (r12) |
| 0x49 | abort | Esc (r13) |
| 0x59 | skip | Esc (r21) |

**Vehicle scheme** (`0xDD4BA8`, from 0x500630):

| Action | Name | Key (record) |
|---|---|---|
| 0x1E | firearm | LMB (r35) |
| 0x1F | secondary firearm | RMB (r36) |
| 0x18 | accelerate | LShift (r50) |
| 0x19 | brake | Space (r51) |
| 0x24 | look behind | X (r49) |
| 0x15 | weapon menu | Q (r6) |

Steering and throttle also come from the move stick (stick 0 = W / A / S / D through `input_map_keyboard_mouse`).

| Path | How to reach it |
|---|---|
| `input_action_take_repeat` 0x4FFA30 | Any menu list. Open the pause menu (Esc, action 0x30), then **hold** W / S or Up / Down (actions 0x36 / 0x37). The first take happens on press, then one every auto-repeat pulse (first after 500 ms, then every 150 ms). The "held but no pulse → 0 and the stick fallback is still called" quirk shows on the frames between pulses. |
| `input_action_press_time` 0x4FFB80 | Only call site: 0x4F91E4 in `player_controls_attack`, with action 7. On foot with a weapon, **press and release RMB** (secondary attack). |
| `input_action_consume` 0x5001D0 | **E** near a vehicle, pick-up or diversion (`player_controls_interact`, actions 0x2D / 2); **C** on foot (`player_controls_on_foot`, action 0x11); **LMB / RMB in a vehicle** (`player_controls_vehicle`, actions 0x1E / 0x20). |
| `input_consume_all` 0x5002B0 | Any menu or overlay opening: **Esc** (pause, through `game_pause_do_frame`), **Tab** (map), or a change of input context (`input_context_enter_consume`, e.g. a phone call or a cutscene start). |
| `player_stick_get` 0x4FF5F0, `input_action_prev_value` 0x4FF7C0, `input_action_alt_prev_value` 0x4FF860 | **No callers in the exe**: no direct or tail call and no stored pointer. They cannot be reached from gameplay; test them with a direct call in a harness. (`player_stick_get_raw` 0x4FF5A0 does have 15 callers.) |

### Reaching the death / respawn path

`gameplay_do_frame` runs `player_death_respawn_update` for the local player once its death timer (+0x122C) has run out, no death script thread (+0x1290) is running, and +0xFA4 bit 0x10 is set.

Any real death sets these. Ways to die from a script, untested:
1. With the F7 menu, turn god mode off and lower health, then take normal damage: jump off a building roof, or stand next to a car and shoot it until it explodes. The F7 vehicle drop (`cheat_vehicle_drop`, doc 38) places a car next to the player.
2. Drown: deep water kills on foot.
3. Get run over: drive a dropped car into the player from a co-op partner (two players needed).

No shipped cheat kills the player directly.

### Getting into a vehicle

1. Stand next to the driver door and press **E** (action 0x2D, "action", record 4). `player_controls_interact` starts the entry; the human goes through `vehicle_interactions_update` (vi_enter.xtbl animations).
2. Conditions:
   - on foot, not in a cutscene or menu;
   - the vehicle not locked or reserved (`vehicle_seat_reservations_expire` handles reservations);
   - the player not ragdolled.
3. The F7 vehicle drop puts a car next to the player for this.
4. Then drive with the vehicle scheme above. The driver throttle, steering and brake come from `vehicle_player_input` (actions 0x24 / 0x27 / 0x28) and the move stick.

The exact key that accelerates on PC (W through the stick or LShift through action 0x18) is not confirmed statically. Try both.

### Porting notes

- `player_controls_update` reads only the pad record, the pad index and the frame time, and writes only the block. That makes it a self-contained next step after `input_do_frame`. It can be checked with a compare-against-original run, as `input.rs` does for the input tree.
- Port the accessors, setters and `input_consume_all` as one group, since they share the action table and the masks. They are leaves; only the stick-direction fallbacks 0x4FE380..0x4FE6A0 call further (0x4FD6D0..0x4FD760).
- The player handlers (0x4F7E90, 0x4F82C0, 0x4F8CA0, 0x4F9380, 0x4F9C80) reach into humans, weapons, vehicles and networking. They are better left as original calls behind a Rust `player_controls_do_frame` for now.

## Open

- The names of the 87 buttons and the 132 game actions (match the defaults at `0xDD3B98` to the controls menu strings).
- The force-feedback setup 0xC13500 (DirectInput constant-force effect) and rumble.
