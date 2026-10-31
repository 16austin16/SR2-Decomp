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
| +0x0C | repeat delay (ms) |
| +0x10 | repeat interval (ms) |
| +0x14 | press time |
| +0x18 | last repeat time |
| +0x1C | u8 repeat pulse |
| +0x1D | u8 just pressed |
| +0x1E | u8 just released |
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

## Controls (actions)

The game reads a control through `input_get_control_value` (0xC010C0; EAX = control, EDX = pad). It **sums every source bound to the control** and scales the result. The control table is `0x23476FC`, 0x1C bytes per control:

| Off | Source | Read |
|---|---|---|
| +0x00 | pad button index | 0 when that button is eaten |
| +0x04 | key (0-255, or a mouse button) | only when keyboard input is enabled (`0x252A545`) |
| +0x08 | stick direction 0-7 | ±x / ±y of the move stick (0-3) or the look stick (4-7) |
| +0x0C | mouse direction 0-5 | X+, X-, Y+, Y-, wheel up, wheel down. Only when mouse input is enabled (`0x252A546`). dx × `0xE83E1C` × 0.06; Y is inverted unless `0x252A547` is set. |
| +0x10 | mouse button | |
| +0x14 | scale (float) | |

Each field is -1 when unused. The mouse camera is control → +0x0C (mouse X/Y), together with the look stick that `input_map_keyboard_mouse` fills from the same deltas.

## Consumers and "eating"

- **vint** runs `vint_input_do_frame` after `input_do_frame`. When a Lua subscription consumes an event, `vint_input_mark_consumed` sets the eaten flags of its buttons (+0x20), sticks (+0xE/+0xF) and keys (+2). From then on `input_get_control_value` returns 0 for them that frame (06_vint.md).
- **The game** reads controls only through `input_get_control_value` and the button records. That is why eating at the record level hides input from gameplay.
- `0x252A54F` blocks all keyboard reads (text entry).

## Open

- The exact layout of the 0x2B8-byte bindings block in `input.bin`, and how it fills the control table `0x23476FC` (0xC12090 / 0xC120D0 / 0xC13500).
- The names of the 87 buttons (the controls menu strings).
- The window procedure itself.
