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
`input_pad_reset_buttons`. Still original: the XInput / DirectInput polls (called from the Rust
`input_poll_devices`), the dead zone, the rebind captures and `mouse_update`.

**How it was checked.** In shadow mode each hooked call runs the original on a snapshot of the
input state, records every `QueryPerformanceCounter` read (the pointer at `0xDB8114` is wrapped),
restores the snapshot, runs the Rust with the same clock readings replayed, and compares the
two results byte for byte. `tools/ingame/run_input.ps1` drives a scripted stretch of play (walk,
sprint, jump, mouse look, wheel, clicks, the pause menu, Ctrl and H / J). Result: 8,902 frames,
272 window messages, ~850,000 key queries, 0 differences, the clock replay exact.

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

## Open

- The names of the 87 buttons (match the default keys to the controls menu strings).
- The force-feedback setup 0xC13500 (DirectInput constant-force effect) and rumble.
