# 44: Audio and weapons names

This round names 122 functions in the audio layer (0x464000-0x491000) and the weapon / ammo code (0xB68000-0xB74000, plus a few human weapon helpers at 0x9C3000-0x9C6000 and 0x558D90). The rows are in `docs/candidates_audio_weapons.csv`; none of the addresses were in `docs/functions.csv`, and none of the names are already used there. Each address is a call target or follows `int3` / `ret` padding.

Evidence came from linear-sweep call graphs of `.text`, the strings each function references, the Lua bindings that call them, and the xtbl parsers that fill the structures. "Confirmed" in a note means the code shows the behaviour directly. "Inferred" means the name rests on context.

## Structures recovered

**Cue tables (by `audio_play` params type +0x18):**

| Type | Table | Count | Find | Play |
|---|---|---|---|---|
| 0 foley | `0xEE5558[i]` (pointers; +0 hash, +4 cue array, +0x10 packed: bits 0-11 distance profile, 12-17 variant mode) | u16 `0x2526AF0` | `audio_foley_find_by_hash` | `audio_play_foley_cue` |
| 1 voice line (`voice.xtbl`, `cutscene_voice.xtbl`) | `0x2574360` (16-byte records, +0 hash) | u16 `0x2526AF8` | `audio_voice_line_find_by_hash` | `audio_play_voice_line` |
| 2 music | `0xEE7178[i]` (pointers, +0 hash) | u16 `0x2526AF4` | `audio_music_find_by_hash` | `audio_play_music_cue` |

`0xEE9178[i]` holds the distance profiles (from `voice_distances.xtbl`): +0 min, +4 / +0x28 max, +0x2C flags. The global cue table is `0xEFA208` (4-byte entries {u16 cue, u8 bank, category | flag << 7}, at most 0x16000, count `0x2526B54`), filled by `audio_cue_table_add`.

**Play params (0x5C bytes)** built by the `audio_play_params_init*` family:

| Off | Field |
|---|---|
| +0 | u16 cue id |
| +2 | u16; above 10 it becomes a start delay in ms (slot +0xC8) |
| +4 | dword; above 0x20 it becomes a fade-in in ms |
| +0xC / +0x10 | volume / pitch (1.0) |
| +0x18 | type (0 foley, 1 voice line, 2 music) |
| +0x1C..+0x24 | position |
| +0x28 | variant index |
| +0x30 / +0x32 | 0xFFFF / 1 |
| +0x4C | tag (copied to slot +0xB8) |
| +0x58 | u16 flags: bit 0 positional, bit 1 allowed while audio is paused |

Variants differ only in the stdcall argument list. 3D variants take EDI = &position.

**Voice slots** (adds to doc 10): +0x24 volume, +0x28 / +0x30 two more per-voice values with dirty bits 2 / 4, +0x3C pitch, +0x44..+0x58 orientation, +0x5C position, +0x68 velocity, +0xA4 category, +0xA8 fade rate, +0xB8 tag, +0x1C fade level. In +0x196: bit 0x1 is dirty, 0x80 fading out, 0x100 fading in, 0x800 3D (set from params flag bit 0). In +0x198: 0x400 is stop, 0x2000 pause, 0x4000 resume, 0x200 play request. The handle check (slot = low byte, generation = upper 24 bits against +0xB4, +0 != -1) is inlined at the top of every `audio_voice_*` wrapper.

**Ammo** (`ammo.xtbl`): records at `0x22D7B54`, count `0x22D7B50`. Each has +0 name, +4 hash, +8 flags (1 drops with weapon, 2 thrown, 4 lethal, 8 bullet, 0x10 projectile, 0x20 fire, 0x40 water, 0x80 sewage), +0xC Max_in_Reserve and +0x10 Max_in_Reserve_MP. The MP field is used when `0x2528B48 == 1 && 0x2528B49 == 0`. A player's ammo store is a pointer at human +0x11DC to {+0 entries *}: one 12-byte entry {type *, count, bonus} per ammo type.

**Weapon info** (0x498 bytes, `0x22D7BCC`, count `0x22D7BD0`). This answers the open "in-memory weapon info layout" item of doc 20:

| Off | Field |
|---|---|
| +0xC | flags 1: can zoom 1, brass during reload only 0x800, melee can dislodge movers 0x1000, one shot shatter 0x2000, clip separate 0x4000, cutscene only 0x8000, left hand 0x10000, disallowed in demos 0x20000, unlockable 0x40000, instant ragdoll 0x80000, apply force to live ragdolls 0x100000, causes convulsions 0x200000, revives humans 0x400000, dual wieldable 0x800000, manually detonates 0x1000000, oriented_explosion 0x2000000, no combat ready 0x4000000, always play melee vfx 0x10000000, attaches 0x20000000, constant vfx on combat ready only 0x40000000, cooldown 0x80000000 |
| +0x10 | flags 2: use block flinch anims 2, unlimited ammo 8, alt unlimited ammo 0x10, has alt fire 0x20, infinite magazine capacity 0x40, draw_both_hands 0x80, use strafe weapon hand swap 0x200, lethal melee 0x400, allow nohit combos 0x800, disallow forward throw in vehicle 0x1000, not allowed in human shield 0x4000, not allowed in vehicle 0x8000, left_hand_ik_during_attack_only 0x10000, no vehicle combat ready 0x20000 |
| +0x20 / +0x24 / +0x28 | Animation_Group / Fine_Aim_Animation_Group / Reload_Animation_Group |
| +0x2C / +0x30 | Spinebend_Limits / Strafe_Angles |
| +0x34 | Weapon_Class: 0 pistol, 1 smg, 2 rifle, 3 shotgun, 4 launcher, 5 thrown, 6 knife, 7 nightstick, 8 stungun, 9 bat, 10 sword, 11 pimp slap, 12 video camera, 13 knuckles, 14 paddles, 15 flamethrower, 16 cutscene only, 17 minigun, 18 pepper spray, 19 chainsaw, 20 waterspray, 21 script, 22 vehicle (table `0xDD0160`) |
| +0x38 / +0x3C / +0x40 | Category / Inv_Slot / name hash |
| +0x48..+0x74 | effects: Muzzle_Flash, Muzzle_Effect, Alt_Muzzle_Effect, Tracer_Effect, Melee_Effect +0x58, Target_Glare +0x60, Constant_Effect +0x74 |
| +0x7C..+0x84 | mesh filenames |
| +0x88..+0x9A | u16 sound ids: Fire, NPC_Fire, Secondary_Fire, Fire_Release, NPC_Fire_Release, Reload, No_Ammo, Start / Spinning / Stop_Spinning +0x96..+0x9A |
| +0x9C / +0x9E | Warmup / Cooldown_Delay (u16) |
| +0xA0..+0xA6 | ProneAttack / ImpactFoleyFlesh / Hit_Wall / ImpactFoleyWorld sounds |
| +0xCC / +0xD0 | Trigger_Type / Alt_Trigger_Type |
| +0xD4 / +0xD8 | Ammo / Alt_Ammo (ammo record pointers) |
| +0xDC | Magazine_Size (u16) |
| +0xE0 / +0xE4 / +0xE8 | Range_Max / AI_Range_Max / Sound_Radius |
| +0x108 / +0x10C / +0x110 | NPC / Player / MP_Vehicle damage (Damage_Max), copied to +0x114..+0x11C for Damage_Min |
| +0x2BC.. | MeleeAttacks |

**Weapon instance** (`weapon_instance_alloc`; live list `0x22D7B6C`, free list `0x22D7BC4`): +8 info, +0xC flags, +0x10 clip, +0x14 reserve (non-player), +0x18 / +0x1C alt clip / alt reserve, +0x2C state (>= 0 while active), and handles -1 at +0x24..+0x34 and +0xA8 / +0xAC. A human's held weapon is human +0x10CC (inventory item) -> +0xC4 (instance).

## Names

| Address | Name | Status | Registers, evidence |
|---|---|---|---|
| `0x464970` | `audio_play_params_init_volume` | documented | ESI = params (0x5C bytes), stdcall (u16 cue, type +0x18, tag +0x4C, f32 volume +0xC) ret 0x10 -> EAX = ESI; audio_play_params_init plus a volume; 42 callers (ambient emitters, radio, sirens); confirmed |
| `0x464A10` | `audio_play_params_init_variant_volume` | documented | ESI = params, stdcall (u16 cue, tag +0x4C, variant +0x28, f32 volume) ret 0x10; type 0 (foley), 2D; confirmed |
| `0x464A60` | `audio_play_params_init_ext` | documented | ESI = params, stdcall (u16 cue, type, tag, f32 volume, u16 +2, dword +4) ret 0x18; 2D; layout confirmed; +2 (> 10 = start delay ms in audio_voice_start) and +4 (> 0x20 = fade-in ms) inferred |
| `0x464AC0` | `audio_play_params_init_3d` | documented | ESI = params, EDI = &position (copied to +0x1C..+0x24), stdcall (u16 cue, type, tag) ret 0xC; sets the positional flag +0x58 bit 0; 58 callers; confirmed |
| `0x464CA0` | `audio_play_params_init_3d_volume` | documented | ESI = params, EDI = &position, stdcall (u16 cue, type, tag, f32 volume) ret 0x10; positional; confirmed |
| `0x464D10` | `audio_play_params_init_3d_ext` | documented | ESI = params, EDI = &position, stdcall (u16 cue, type, tag, f32 volume, u16 +2, dword +4) ret 0x18; callers include projectile_create; confirmed |
| `0x464D90` | `audio_play_params_init_3d_variant` | documented | ESI = params, EDI = &position, stdcall (u16 cue, tag, variant +0x28) ret 0xC; type 0 (foley); callers human_bullet_hit, the explosion code; confirmed |
| `0x464E00` | `audio_play_params_init_3d_variant_volume` | documented | ESI = params, EDI = &position, stdcall (u16 cue, tag, variant, f32 volume) ret 0x10; type 0; confirmed |
| `0x465AB0` | `audio_find_id_by_name` | documented | ECX = name, cdecl (type: 0 foley / 1 voice line / 2 music) -> EAX u16 id or 0xFFFF; string_hash_lower then the matching *_find_by_hash; callers luafn_audio_get_audio_id, luafn_audio_play, luafn_audio_prepare, luafn_mp_play_audio; confirmed |
| `0x465B00` | `audio_prepare` | documented | EAX = params, ECX = &out handle (3 dwords) -> EAX = out; audio_play with prepare_only = 1 (started later by audio_voice_play_prepared); out = 0xFF / -1 / -1 for cue 0xFFFF or while paused (0x2526B44 > 0) without params +0x58 bit 1; luafn_audio_prepare; confirmed |
| `0x465C40` | `audio_play_foley_2d` | documented | ESI = &out handle, cdecl (u16 cue) -> EAX = ESI; builds 2D foley params on the stack and calls audio_play; 15 callers; confirmed |
| `0x465CD0` | `audio_play_foley_2d_tagged` | documented | ESI = &out handle, cdecl (u16 cue, tag +0x4C) -> EAX = ESI; 2D foley through audio_play; 54 callers (activities, HUD); confirmed |
| `0x465D40` | `audio_play_foley_3d` | documented | ECX = u16 cue, EAX = tag, EDI = &position, cdecl (&out handle) -> EAX = out; audio_play_params_init_3d (type 0) + audio_play; 16 callers (taxi_update, tagging, human weapon code); confirmed |
| `0x465D70` | `audio_play_voice_line_2d` | documented | ESI = &out handle, cdecl (u16 voice line id) -> EAX = ESI; type 1 (voice.xtbl), 2D; callers luafn_liquor_store_make_purchase, act_packet_sub_40; confirmed |
| `0x465F70` | `audio_voice_fade_out` | documented | ECX = &voice handle, cdecl (ms): validates the handle (slot = low byte, generation = upper 24 bits vs +0xB4, +0 != -1) and tail-jumps to audio_voice_slot_fade_out; 38 callers (vehicle audio, radio, sirens); confirmed |
| `0x465FD0` | `audio_voice_set_orientation` | documented | ECX = &voice handle, cdecl (transform): validates the handle, then audio_voice_slot_set_orientation; 9 callers (vehicle engine audio, sirens, radio); confirmed |
| `0x4660B0` | `audio_voice_is_2d` | documented | ECX = &voice handle -> AL = 1 when the voice is valid and not 3D (+0x196 bit 0x800 clear), 0 otherwise; bit confirmed, meaning inferred |
| `0x466140` | `audio_voice_get_volume` | documented | ECX = &voice handle -> ST0 = slot +0x24 (f32), 1.0 when the handle is stale; clobbers EAX, ECX; callers ambient emitters, 0x9BDA80; confirmed |
| `0x466180` | `audio_voice_get_param_30` | named | ECX = &voice handle -> ST0 = slot +0x30, 0.0 when stale; pairs with audio_voice_set_param_30; inferred |
| `0x466410` | `audio_play_voice_line_channels` | named | DX = voice line id, EBX = out (0x38 bytes: four handles + extra), cdecl (...): fills the handles with 0xFF / -1 / -1, then 0x47B3C0 starts one 2D voice per speaker channel (audio_voice_start_2d_channel); only caller 0x6CDE60 (cutscene); inferred |
| `0x46E000` | `audio_dsp_zones_load` | named | zone file parser for "$DSP:" blocks ($Navpoint, $Shape Sphere / Box, $Inner / $Outer Radius or Dimensions); from 0xA24AB0; inferred from strings |
| `0x46EA00` | `audio_focus_init` | documented | audio_init: xtbl_load_focused_audio, then registers the NoFocus / ScriptingFocus / ActivityFocus states (0x46EB50) and seeds the category volume bases 0xEE34F8..0xEE3504; confirmed by strings |
| `0x46ED20` | `audio_set_category_throttle` | documented | ECX = category (0-3), cdecl (f32 level): stores level to 0xEE34F8[cat] when 0 <= level <= 1 (NaN rejected), otherwise ignored; via 0x465860 from luafn_audio_throttle_type, also blackjack / poker start; confirmed |
| `0x46F0F0` | `audio_ear_ring_init` | documented | no args: 0xE99C40 = audio_foley_find_by_hash("SFX_EAR_RING"), 0xEE3550 = 0.0; no direct callers (init table); confirmed by string |
| `0x46F210` | `audio_ear_ring_update` | named | audio thread, from audio_voices_update: ramps the ear-ring level 0xEE3550 (audio_ramp_toward, dt 0xE99C44), sets the ear-ring voice volume (audio_voice_set_volume) and starts / stops cue 0xE99C40 (SFX_EAR_RING, param 0x190) based on the player state (human_is_dead_or_invalid); inferred |
| `0x46FA20` | `audio_idx_maps_load` | documented | loads audio_boot.idx_map / audio_level.idx_map (cue-name index maps) into 0xEE3554 / 0x2526AD0..0x2526AD8; callers audio_init, audio_hash_init, mission_load_script; confirmed by strings |
| `0x46FF40` | `audio_hash_init` | documented | allocates the "audio hash" tables (0x2526AC8 / 0x2526ACC) from the allocator 0x277307C and reloads the idx maps (audio_idx_maps_load); confirmed by string |
| `0x472B40` | `audio_emitter_parse` | named | "$Emitter:" block parser ($Volume, $Priority, $Min / $Max Delay, $Min / $Max ToD, $ToD Fade In); only caller audio_ambients_load; inferred from strings |
| `0x477700` | `audio_ambience_rain_setup` | named | registers the AMB_WTHR_RAIN_LHT / MED / HVY cues (audio_cue_table_add) for the weather ambience; caller 0x477E70; inferred from strings |
| `0x477960` | `audio_ambience_wind_setup` | named | registers the AMB_WTHR_WIND_* cues; caller 0x477E70; inferred from strings |
| `0x477BC0` | `audio_ambience_high_alt_wind_setup` | named | registers AMB_WIND_HIGH_ALT ("Ambients" bank); caller 0x477E70; inferred from strings |
| `0x477F90` | `audio_ambience_at_sea_setup` | named | registers AMB_AT_SEA_LT / MED / HVY; callers 0x465520, mission_load_script; inferred from strings |
| `0x478D70` | `audio_ambients_load` | named | zone file parser for "$Ambient:" blocks ($Navpoint, $Shape Sphere / Cylinder, radii, heights) feeding the ambient emitter tree; callers 0x466520, 0xA24AB0; inferred from strings |
| `0x479A50` | `audio_foley_find_by_hash` | documented | EDX = &name hash -> AX foley index (EAX = 0x0000FFFF when absent); linear scan of 0xEE5558[i]->+0 hash, u16 count 0x2526AF0; preserves ESI; 243 call sites (audio_find_id_by_name type 0, the foley xtbl parsers); confirmed |
| `0x479A80` | `audio_foley_max_distance` | documented | AX = foley id -> ST0 = its distance profile (0xEE9178[rec+0x10 & 0xFFF]) +0x28 x 1.05 (f64 0xE5C400), rounded to f32; 0.0 for 0xFFFF; clobbers ECX/EDX; meaning (max audible distance) inferred |
| `0x479B60` | `audio_play_foley_cue` | documented | EDI = &out handle, cdecl (params, bool prepare_only) -> EAX = EDI; picks the variant (params +0x28 or the record mode, bits 12-17 of +0x10), culls positional cues beyond the distance profile range from the listener 0xEEB254, then audio_voice_prepare or audio_voice_start; type 0 of audio_play; confirmed |
| `0x47A2B0` | `audio_music_find_by_hash` | documented | EDX = &name hash -> AX music index (0xFFFF absent); 0xEE7178[i]->+0 hash, u16 count 0x2526AF4; callers radio_groups_parse_*, frontend_enter, audio_find_id_by_name type 2; confirmed |
| `0x47A4A0` | `audio_play_music_cue` | documented | ESI = &out handle, cdecl (params, bool prepare_only); type 2 of audio_play: music record, distance cull, audio_voice_prepare / audio_voice_start; confirmed |
| `0x47A770` | `voice_parse_record` | documented | record parser under xtbl_load_voice: Name, AudioBanks, Focused, SphereProperties (MinRadius / MaxRadius), Variations / Variation; registers cues with audio_cue_table_add; confirmed by strings |
| `0x47AE30` | `xtbl_load_voice` | documented | loads voice.xtbl and cutscene_voice.xtbl ("Voice" records) into 0x2574360 / count 0x2526AF8 via voice_parse_record; from audio_init; confirmed by strings |
| `0x47AFB0` | `audio_voice_line_find_by_hash` | documented | EDX = &name hash -> AX index of the voice.xtbl / cutscene_voice.xtbl line (0xFFFF absent); 16-byte records 0x2574360 (+0 hash), u16 count 0x2526AF8; 58 call sites (cutscene_parse_audios, audio_find_id_by_name type 1); confirmed |
| `0x47B0B0` | `audio_play_voice_line` | documented | ESI = &out handle, cdecl (params, bool prepare_only); type 1 of audio_play: voice.xtbl line, distance cull, audio_voice_prepare / audio_voice_start, then 0x4803F0 and 0x46EB90 (focus); confirmed |
| `0x47BC90` | `audio_distance_profile_set` | documented | cdecl (profile index, f32 min, f32 max) -> AL: when min <= max and the profile 0xEE9178[i] has flag +0x2C bit 1, writes min +0, max +4 / +0x28, 0 +0x10, 1.0 +0x14 and a clamped 1 / (max - min) slope (+0x18..+0x24); 8 callers (ambient emitters); meaning inferred |
| `0x47C740` | `audio_voice_slot_fade_out` | documented | EAX = slot index, EDX = fade ms: +0x196 = (flags & ~0x100) / 0x80 (+0xC4 = 0 if 0x80 was clear); rate = +0x1C x 0.5 when ms < 32, else +0x1C / (ms >> 5) (unsigned fild + 2^32 fix-up), stored to +0xA8 only when not smaller than the current rate; luafn_audio_fade_out; confirmed |
| `0x47CDB0` | `audio_voice_slot_alloc` | named | EDI = &out slot byte, cdecl (&?, f32 priority): picks a free or lower-priority slot of the 128 at 0xEEB8F8 under the lock 0x25F5360; 0xFF when none; callers audio_voice_start / prepare / start_2d_channel; inferred |
| `0x47CED0` | `audio_voice_slot_fade_in` | documented | ECX = slot index, cdecl (ms, f32 start volume): unless +0x196 bit 0x100 is set: +0x1C = start, +0xC4 = 0, +0xA8 = (1 - start) / (ms / 32) (signed divide), +0x196 = (flags & ~0x80) / 0x100; confirmed |
| `0x47D3A0` | `audio_voice_slot_set_orientation` | documented | ECX = transform, cdecl (slot *): two vectors from 0xBE3A60 / 0xBE3150 / 0xBE2D90 stored at +0x44..+0x4C (one value splatted) and +0x50..+0x58; callers audio_voice_set_orientation(_2), audio_voice_set_3d_position, vehicle audio; meaning inferred |
| `0x47F020` | `audio_voices_fade_out_all` | documented | cdecl (ms): 0x2526B44 += 1 (pause depth), then every live or pending slot gets +0x194 bit 0 and audio_voice_slot_fade_out(EAX = index, EDX = ms); callers player_death_respawn_update, gamewide_func, ovl_unlockable::show; confirmed |
| `0x47F080` | `audio_pause_all` | documented | no args: while the window is inactive (0x252A406 == 0) pauses category 1 and mutes the mastering voice once (0x2526AFB) and sets 0x2526AD7; otherwise 0x2526B44 += 1 and each live slot gets +0x198 bit 0x2000 (pause) or 0x400 (stop, when +0x194 bit 1); 13 callers (gameplay_exit, game_state_enter); confirmed |
| `0x47F170` | `audio_resume_all` | documented | no args: undoes audio_pause_all: unmutes / unpauses when 0x2526AD7 was set, else 0x2526B44 -= 1 (floored at 0) and at 0 sets +0x198 bit 0x4000 (resume) on voices with +0x196 bit 0x40; callers video_stop, cutscene_do_frame, frontend_exit; confirmed |
| `0x47F200` | `audio_voices_stop_category` | documented | EDX = category (slot +0xA4) or -1 for all: each live or pending slot of that category gets +0x198 /= 0x400 (stop) and +0x194 &= ~0x80; loop unrolled x4 over 128 slots; 12 callers (gameplay_exit, pause menu quit); confirmed |
| `0x47F2E0` | `audio_voices_mark_dirty_all` | documented | no args: every live or pending 2D slot (+0x196 bit 0x800 clear) gets +0x196 /= 1 so audio_voices_update re-commits it; callers luafn_pause_menu_update_option / restore_defaults / revert_options; confirmed |
| `0x47F4B0` | `audio_voice_start` | documented | cdecl (out*, params, 0, xact cue, distance profile, flags, f32 a, f32 b) -> EAX = out {slot / gen << 8, -1, -1}; audio_voice_slot_alloc, then copies params (volume +0xC -> +0x24, pitch +0x10 -> +0x3C, position -> +0x5C, category +0x18 -> +0xA4, tag +0x4C -> +0xB8), flag bits from +0x58, optional fade-in (+4 > 0x20 ms), start delay (+2 > 10 ms -> +0xC8), generation counter 0xE99C00 (wraps at 16,000,000); confirmed |
| `0x47F800` | `audio_voice_play_prepared` | documented | ECX = &voice handle: if valid and not stopping (+0x198 bit 0x400), re-evaluates the mute bits (0x47D780, may set +0x196 bit 0x20) and copies +0x196 bit 0x200 inverted into +0x198 bit 0x200 (play request); luafn_audio_play_prepared; confirmed |
| `0x47F880` | `audio_voice_prepare` | documented | same arguments as audio_voice_start but leaves the cue prepared, not playing, for audio_voice_play_prepared; callers audio_play_foley_cue / voice_line / music_cue with prepare_only; inferred from the pairing |
| `0x47FBA0` | `audio_voice_start_2d_channel` | named | cdecl (out, cue, distance profile, f32 volume, channel, ...): starts a 2D voice slot (audio_voice_slot_alloc, position 0, volume 1.0, channel bits << 14 in +0x194); 9 call sites, all in 0x47B3C0; inferred |
| `0x47FF80` | `audio_voice_fade_out_2` | documented | byte-identical LTCG twin of audio_voice_fade_out (ECX = &handle, cdecl (ms)); 18 call sites in the ambient emitter updates (audio thread); confirmed |
| `0x480070` | `audio_voice_set_volume` | documented | ECX = &voice handle, cdecl (f32 volume) -> AL: slot +0x24 = volume; marks dirty (+0x196 /= 1) unless 3D (+0x196 bit 0x800); AL = 0 when the handle is stale; clobbers EAX, ECX; 34 callers on both threads; confirmed |
| `0x4800C0` | `audio_voice_set_param_28` | named | ECX = &voice handle, cdecl (f32) -> AL: slot +0x28 = value, +0x196 /= 2, dirty when +0x194 bit 3 and not 3D; meaning of +0x28 unknown; inferred |
| `0x480170` | `audio_voice_set_param_30` | named | ECX = &voice handle, cdecl (f32) -> AL: slot +0x30 = value (params +0x14 at start), +0x196 /= 4, dirty when +0x194 bit 4 and not 3D; meaning unknown; inferred |
| `0x4802D0` | `audio_voice_set_3d_position` | documented | ECX = &voice handle, EDI = &velocity, cdecl (&position, transform) -> AL: audio_voice_slot_set_orientation(transform), velocity -> +0x68..+0x70, position -> +0x5C..+0x64, +0x198 /= 1, +0x196 /= 0x812, +0x190 = 0, mute bits; 14 callers (vehicle audio, ambient emitters, sirens); confirmed |
| `0x4803B0` | `audio_voice_set_orientation_2` | documented | LTCG twin of audio_voice_set_orientation; callers the ambient emitter updates 0x473330 / 0x475130; confirmed |
| `0x480F00` | `xact3d_apply` | documented | the DirectX SDK XACT3DApply inline: sets the cue variables "Distance", "DopplerPitchScalar", "OrientationAngle" from the X3DAUDIO_DSP_SETTINGS and the matrix; E_POINTER (0x80004003) on null; only caller 0x4821C0 (per-voice 3D commit); confirmed by strings |
| `0x481760` | `audio_bank_find_by_name` | documented | ECX = bank name -> AL bank index in 0xEF8704 (0x20-byte entries, count 0x2526B50) or 0xFF; uses the hash table 0x4700D0 when 0x2526AC8 is set; callers audio_cue_table_add, foley_engine_parse_record; confirmed |
| `0x481D20` | `audio_cue_table_add` | documented | ECX = bank name, EAX = cue name, cdecl (category 0-127, bool flag) -> EAX global cue index or -1; finds the cue in the bank (0x470230) and appends {u16 cue, u8 bank, category / flag << 7} to 0xEFA208 (max 0x16000, count 0x2526B54), dedup by linear scan or the hash table 0x470180; callers foley / music / voice parsers and the ambience setups; confirmed |
| `0x4829B0` | `xtbl_load_reverb` | documented | loads reverb.xtbl ("Reverb" records: Name, Interior, Exterior) for the submix voices; only caller xaudio2_create_submix_voices; confirmed by strings |
| `0x48D900` | `radio_vehicle_station_stop` | documented | EAX = vehicle: when it owns the radio (+0x44 == 0xE9A530), fades the station voice 0xE9A524 out over 750 ms and clears 0xE9A530 / 0xE9A534 / 0x2527CD0; 9 callers; confirmed |
| `0x48D970` | `radio_dj_voice_stop` | named | cdecl (fade ms): fades the voice 0xE9A538 and clears 0xE9A544 / 0xE9A548 / 0x2527CD0; callers vehicle_radio_turn_off, vehicle_radio_block; role of 0xE9A538 (DJ / ad line) inferred |
| `0x48E330` | `vehicle_radio_turn_off` | documented | ESI = vehicle -> AL: when the vehicle info has the radio flag (+0x84E4 -> +0x5B0 bit 0x4000000) and the radio is not blocked (+0x8D84 bit 0), stops the three radio voices +0x8D40 / +0x8D4C / +0x8D58, radio_vehicle_station_stop and the DJ voice (750 ms); callers player_controls_vehicle, 0x48E470; confirmed |
| `0x48F780` | `radio_track_find_by_name` | documented | ECX = name -> AX track index or 0xFFFF; 0x14-byte records at *0xF53090 (+0 hash), u16 count 0x2526B94; callers radio_parse_slot, 0x7AF6E0 (music store); confirmed |
| `0x4904F0` | `radio_station_find_by_name` | documented | ECX = name -> AX station index; 0x48-byte records at *0xF53094 (+4 hash), u16 count 0x2526BC8; returns the default 0x2526BA0 when absent; callers xtbl_load_radio, fuzz_crime_noise_violation::init; confirmed |
| `0x490EB0` | `vehicle_radio_block` | documented | ESI = vehicle: sets +0x8D84 bit 0 once and stops the radio voices like vehicle_radio_turn_off; luafn_radio_block, fuzz_crime_public_indecency::init; confirmed |
| `0x558D90` | `inventory_item_weapon_slot` | documented | EAX = inventory owner -> EAX = Inv_Slot (+0x3C) of the weapon info of item +8 -> +0xC4, 0 when empty; 12 callers (AI, fuzz_crime_septic::update); confirmed |
| `0x9C3E50` | `weapon_remove_reserve_ammo` | documented | ECX = human, ESI = instance, EDX = amount (> 0) -> EAX removed: local player and not a vehicle weapon -> weapon_ammo_store_remove on the store human +0x11DC; others from instance +0x14; 0 on bad args; confirmed |
| `0x9C3F50` | `weapon_info_max_reserve` | documented | EAX = weapon info, cdecl (bool alt) -> EAX = ammo type (+0xD4 / +0xD8) Max_in_Reserve (MP field when 0x2528B48 == 1 and 0x2528B49 == 0); 0 without info or type; 16 callers (cheat_weapon_give, human_pkt_send_44); confirmed |
| `0x9C4060` | `human_has_melee_weapon` | documented | EAX = human -> EAX 1 when the held item (+0x10CC -> +0xC4 instance -> +8 info) class is melee (same set as weapon_class_is_melee); 12 callers; confirmed |
| `0x9C6060` | `human_has_thrown_weapon` | documented | EAX = human -> EAX 1 when the held weapon class is 5 (thrown); callers human_ai_think, 0x4FB860; confirmed |
| `0xB687F0` | `xtbl_load_ammo` | documented | loads ammo.xtbl ("Ammo" records) into 0x22D7B54, count 0x22D7B50, via ammo_parse_record; from 0xB69750 (game_init_systems); confirmed by strings |
| `0xB68910` | `ammo_parse_record` | documented | ammo record: +0 name, +4 name hash, +8 flags (1 drops with weapon, 2 thrown, 4 lethal, 8 bullet, 0x10 projectile, 0x20 fire, 0x40 water, 0x80 sewage), +0xC Max_in_Reserve, +0x10 Max_in_Reserve_MP; confirmed by strings |
| `0xB68C20` | `ammo_store_entry_add` | documented | ESI = entry, EAX = amount, stdcall (bool overflow) ret 4 -> EAX added: room = capacity + bonus - count; clamps to room unless overflow, in which case the excess goes to the bonus +8; only caller ammo_store_add_type; confirmed |
| `0xB68CE0` | `ammo_store_find` | documented | ECX = &store ({+0 entries *}), EDI = ammo type hash -> EAX entry (12 bytes {type *, count, bonus}) or 0; scans count 0x22D7B50; preserves EBX / ESI; confirmed |
| `0xB68EB0` | `ammo_store_count` | documented | ECX = &store, EDI = ammo type hash -> EAX count (entry +4) or 0; 13 callers (player_controls_weapon_select, HUD); confirmed |
| `0xB68F60` | `ammo_store_capacity` | documented | ECX = &store, EDI = ammo type hash -> EAX = type Max_in_Reserve (MP +0x10 when 0x2528B48 == 1 and 0x2528B49 == 0, else +0xC) + entry bonus +8; 0 when absent; confirmed |
| `0xB69020` | `weapon_total_ammo` | documented | ESI = weapon instance, stdcall (store, bool alt) ret 8 -> EAX = clip (+0x10) + store count of the primary (+0xD4) or alt (+0xD8) ammo type; clip only when no ammo type; 0 for no instance; callers humans_post_update and human weapon code; confirmed |
| `0xB690C0` | `ammo_store_add_type` | documented | ECX = &store, EDI = ammo type hash, EAX = amount, stdcall (bool overflow, ?) ret 8 -> EAX added via ammo_store_entry_add, 0 when the type is absent; callers ammo_store_add, 0x91DD40; confirmed |
| `0xB69190` | `ammo_store_remove` | documented | ECX = &store, EDI = ammo type hash, stdcall (amount) ret 4 -> EAX removed = min(amount, count); also lowers the bonus +8 (floored at 0); confirmed |
| `0xB69230` | `weapon_ammo_store_remove` | documented | ECX = weapon instance, EAX = amount, stdcall (store) ret 4 -> EAX removed: ammo_store_remove for the primary ammo type (+0xD4) of the instance info; 0 without one; callers weapon_refill_after_add, human_ai_think, weapon_remove_reserve_ammo; confirmed |
| `0xB69260` | `weapon_reload_from_store` | documented | stdcall (store, instance, human) ret 0xC: moves min(clip size - clip, count) from the store into the clip +0x10 (weapon_clip_size), lowering the bonus too; with alt fire (+0x10 bit 0x20) does the same for the alt clip +0x18; callers 0x9C5220, diversion_firetruck_extinguisher_reloaded_text; confirmed |
| `0xB693E0` | `ammo_store_clear` | documented | EDI = &store: zeroes count +4 and bonus +8 of all 0x22D7B50 entries; callers 0x91DD40, 0x5E3830; confirmed |
| `0xB69970` | `weapon_caltrop_effect` | named | starts the "Caltrop" effect (effect_find_by_name, effect_start) at a computed point; only caller 0xB72980; inferred |
| `0xB69B60` | `weapon_effect_alloc` | documented | no args -> EAX entry (0x78 bytes, zeroed) moved from the free list 0x22D7B5C to the live list 0x22D7B58 (links +0x70 / +0x74), or 0; only caller weapon_effect_create; confirmed |
| `0xB69D20` | `weapon_effect_muzzle_flash` | documented | places the "muzzle_flash" tag effect of a weapon effect entry (vec3_mul_mat33_in_place, tag lookup 0x6E5D40 / 0x6E5DB0); callers weapon_effect_update, 0xB6A320; confirmed by string |
| `0xB69ED0` | `weapon_effect_shell_eject` | documented | places the "shell_eject" tag effect; callers weapon_effect_update, 0xB6A320; confirmed by string |
| `0xB6A3D0` | `weapon_effects_init` | named | game_init_systems: clears the weapon effect pool (0x3179AC0..) and the live / free lists 0x22D7B58 / 0x22D7B5C; inferred |
| `0xB6A4F0` | `weapon_effect_create` | named | allocates a weapon effect (weapon_effect_alloc) and sets its transforms; callers the projectile code 0x94CFE0 / 0x94D830; inferred |
| `0xB6A8B0` | `weapon_lockon_reticule_update` | named | ECX, EDX, cdecl (f32): projects the lock target (gfx_world_to_screen) and updates the "sr2_local_player_lockon" HUD values (0x252A134, vint); 15 callers; inferred |
| `0xB6ABE0` | `weapon_lockon_hud_init` | documented | creates the vint data item "sr2_local_player_lockon" (0x252A12C..0x252A134); caller 0x698470; confirmed by string |
| `0xB6B460` | `weapon_target_lock_clear` | documented | no args: when a lock is active (0x252A12C) resets the reticule (weapon_lockon_reticule_update(0, 0, 0.0)), clears 0x252A130 / 0x252A12C and stops the beep voice 0xEBD05C (audio_voice_stop); callers weapon_target_lock_update, mission_restart_from_checkpoint; confirmed |
| `0xB6B4A0` | `vehicle_weapon_mount_find` | documented | EAX = vehicle, EDI = key -> EAX the 0x20-byte mount entry of the vehicle info (+0x84E4 -> +4 -> +0xCC, count +0xC8) whose +0 matches, or 0; 7 callers; confirmed |
| `0xB6B500` | `vehicle_weapon_find` | documented | ESI = vehicle, EDI = weapon info -> EAX the vehicle weapon whose +0 is EDI from the array +0x84C8 (count +0x84D0, only when +0x84D8 == 1), or 0; 9 callers (veh_pkt_send_1a, the weapon fire code); confirmed |
| `0xB6D4D0` | `weapon_parse_time_management` | documented | Time_Management block: Refire_Delay, Pre / Post_Detonate_Delay, Firecone_Ramp_In_Time, npc_refire_delay min / max; called for Time_Management and Alt_Time_Management; confirmed by strings |
| `0xB6D6E0` | `weapon_info_parse_record` | documented | one weapons.xtbl record: flags +0xC / +0x10, +0x34 Weapon_Class, +0x38 Category, +0x3C Inv_Slot, +0x40 hash, sounds +0x88.., +0xD4 / +0xD8 Ammo / Alt_Ammo, +0xDC Magazine_Size, +0xE0 Range_Max, +0xE4 AI_Range_Max, +0xE8 Sound_Radius, +0x108 / +0x10C / +0x110 damage (layout in doc 44); confirmed by strings |
| `0xB70C90` | `vehicle_weapon_parse_entry` | documented | one vehicle weapon: UID, Weapon_Class, Target_Reticule, Min / Max_Heading, Min / Max_Pitch, Max_Heading_Speed; confirmed by strings |
| `0xB70DF0` | `weapon_parse_vehicle_weapon` | documented | Vehicle_Weapon block: Primary_Weapons / Alt_Weapons lists of Weapon entries (vehicle_weapon_parse_entry); confirmed by strings |
| `0xB710C0` | `weapons_table_parse` | documented | body of xtbl_load_weapons: xtbl_load, counts the "Weapon" records, allocates 0x498 each (0x22D7BCC / 0x22D7BD0), weapon_info_parse_record each, then 0xB70FB0; also from gameplay_exit; confirmed |
| `0xB71760` | `weapon_info_find_by_hash` | documented | ESI = name hash -> EAX weapon info whose +0x40 matches, or 0; scans 0x22D7BCC (0x498 each, count 0x22D7BD0); callers script_item::parse, 0x930700; confirmed |
| `0xB71A30` | `weapon_info_ai_range` | documented | EAX = weapon info -> ST0 = AI_Range_Max (+0xE4), 1.275 (0xE5C110) when null; f32-rounded; AI callers 0x40E730..0x43B0A0; field confirmed |
| `0xB71A60` | `weapon_info_ai_range_sq` | documented | EAX = weapon info -> ST0 = AI_Range_Max ^ 2 (f32-rounded), 1.275 ^ 2 when null; 49 callers (AI); field confirmed |
| `0xB71AC0` | `weapon_instance_alloc` | documented | ESI = weapon info -> EAX instance from the free list 0x22D7BC4 (moved to the live list 0x22D7B6C), or 0; +8 info, clip +0x10 and reserve +0x14 = Magazine_Size, alt clip +0x18 / +0x1C with alt fire, handles -1, timers; callers net_recv_wpn_ammo_change, 0x91D7B0; confirmed |
| `0xB71C80` | `weapon_instance_free` | documented | EAX = instance: unlinks it from the live list 0x22D7B6C (circular, +0 prev / +4 next) and inserts it before the head of the free list 0x22D7BC4; preserves ESI; 16 callers (inventory_item::deactivate, object_item::deactivate); confirmed |
| `0xB72780` | `weapon_class_is_melee` | documented | EAX = weapon class -> AL: class in {6 knife, 7 nightstick, 9 bat, 10 sword, 11 pimp slap, 12 video camera, 13 knuckles, 14 paddles, 19 chainsaw} (jump table); callers 0x92F620, 0x9321C0, 0x932770; confirmed |
| `0xB72840` | `weapon_info_uses_sewage_ammo` | documented | ECX = weapon info -> EAX 1 when the primary or alt ammo type has flag 0x80 (sewage); clobbers EDX; callers weapon_fire, 0x9BE6F0; confirmed |
| `0xB728A0` | `weapon_info_uses_bullet_ammo` | documented | ECX = weapon info -> EAX 1 when the primary or alt ammo type has flag 8 (bullet); callers 0x9BD360, vehicle weapon code; confirmed |
| `0xB72C40` | `weapon_info_is_thrown` | documented | EAX = weapon info -> AL = class (+0x34) == 5 ("thrown"); 0 for null; callers weapon_refill_after_add, 0x9AE350; confirmed |
| `0xB72CA0` | `weapon_instance_has_ammo` | documented | EAX = instance -> AL: clip +0x10 > 0, reserve +0x14 > 0, Magazine_Size 0, or with alt fire alt clip +0x18 / reserve +0x1C > 0; crashes when info +8 is null and both counts are 0 (reads [0 + 0x10]); 7 callers; confirmed |
| `0xB72D30` | `weapon_usable_from_vehicle` | documented | EAX = weapon info, cdecl (human, bool driver) -> AL: null or class 22 (vehicle) -> 1; "not allowed in vehicle" (+0x10 bit 0x8000) -> 0; in a vehicle allows pistol / smg / thrown / script (driver) or also rifle / shotgun / launcher (passenger), sword on bikes, video camera / waterspray by player; 25 callers; shape confirmed, seat meaning inferred |
| `0xB72E90` | `weapon_info_uses_projectile_ammo` | documented | EAX = weapon info -> EAX 1 when the primary ammo type has flag 0x10 (projectile); callers 0x796F10, 0x9B3A40; confirmed |
| `0xB72F80` | `weapon_info_is_lethal` | documented | ECX = weapon info, cdecl (bool melee_only) -> AL / EAX: 1 for "lethal melee" (+0x10 bit 0x400); else 0 when melee_only; else 1 when the primary or alt ammo is lethal (flag 4); AI and damage-area callers; confirmed |
| `0xB730E0` | `weapon_debug_draw` | named | debug overlay: "Weapon: %s" / "Weapon: None Equipped", target coords, ammo; no direct callers; inferred from strings |
| `0xB73B70` | `weapon_info_index` | documented | EAX = weapon info -> EAX = (info - *0x22D7BCC) / 0x498 (magic 0x6F74AE27, sar 9); AL = 0xFF (upper EAX unchanged) for null or out of range; 20 callers (net compressors, human_take_damage, human_knockdown); confirmed |
| `0xB73CA0` | `weapon_instance_revives_humans` | documented | EAX = weapon instance -> AL: class 13 / 14 (knuckles / paddles) and "revives humans" (+0xC bit 0x400000); callers 0x97A3B0 / 0x97A580 / 0x97A800; confirmed |
| `0xB73D10` | `weapon_anim_group` | documented | EAX = human, ESI = instance -> EAX animation group: 0x29 when 0x9C9130 holds, 0x25 / 0x26 for class 10 (sword) on a bike, fine-aim group +0x24 when flagged (+0x1163 bit 3), else Animation_Group +0x20; 11 callers (remote_do_frame, human_ai_pre_think); fields confirmed |

## Port candidates

Fifteen small functions, picked for easy shadow checks: leaves or simple call sequences, most of them run every frame. "Preserved" lists the registers the original keeps. Callers may rely on registers the original leaves unchanged by accident, so a port should keep the same clobber set.

### 1. `weapon_total_ammo` 0xB69020 (game thread, every frame from `humans_post_update`)
- **In:** ESI = instance (may be 0); stack (store *, u8 alt); `ret 8`.
- **Out:** EAX.
- **Steps:**
  1. ESI == 0 → EAX = 0.
  2. type = info(+8)->(alt ? +0xD8 : +0xD4). type == 0 → EAX = instance +0x10.
  3. Otherwise EAX = instance +0x10 + `ammo_store_count`(ECX = store, EDI = type +4).
- **Preserved:** EBX, ESI, EDI (EDI is pushed around the call). ECX and EDX are clobbered.
- **Writes:** nothing.
- **Note:** the alt path still adds the primary clip +0x10, not the alt clip +0x18. Keep that.

### 2. `ammo_store_count` 0xB68EB0
- **In:** ECX = &store, EDI = hash.
- **Loop:** i over 0..`*0x22D7B50` (signed `jle` exit), entry = `*store + 12*i`, compare `entry->type->+4` with EDI.
- **Out:** EAX = entry +4, or 0.
- **Preserved:** EBX, ESI. ECX and EDX are clobbered.
- **Writes:** nothing.

### 3. `ammo_store_find` 0xB68CE0
- The same loop as `ammo_store_count`.
- **Out:** EAX = the entry address, or 0.

### 4. `ammo_store_capacity` 0xB68F60
- The same loop. On a hit:
  - EAX = entry +8 + (MP ? type +0x10 : type +0xC).
  - MP = `byte 0x2528B48 == 1 && byte 0x2528B49 == 0`.
- DL ends up holding one of those bytes. ECX ends up as the type pointer.

### 5. `weapon_info_index` 0xB73B70 (net compressors, `human_take_damage`)
- **In:** EAX = info.
- **Valid:** `base <= info < base + count*0x498` (unsigned) → EAX = (info - base) / 0x498. This is `imul 0x6F74AE27; sar edx, 9`, plus the sign bit, which equals the exact quotient for in-range values.
- **Invalid:** AL = 0xFF and **bits 8-31 of EAX keep the input pointer**.
- **Clobbers:** ECX, EDX.

### 6. `weapon_info_is_lethal` 0xB72F80 (siblings: `_uses_sewage_ammo` 0xB72840, `_uses_bullet_ammo` 0xB728A0, `_uses_projectile_ammo` 0xB72E90)
- **In:** ECX = info; cdecl (u8 melee_only).
- **Steps:**
  1. Null → `xor al, al` (upper EAX unchanged).
  2. `+0x10 & 0x400` → `mov al, 1` (upper EAX unchanged).
  3. melee_only → AL = 0.
  4. Otherwise EAX = 1 when `+0xD4` is non-null with `flag & 4`, or `+0xD8` is non-null with `flag & 4`; else EAX = 0 (full register).
- **Clobbers:** DL = 4. On the alt path ECX = +0xD8.
- The siblings have the same shape with mask 0x80, 8 and 0x10 (0x10: primary only), and always return a full EAX.

### 7. `weapon_instance_has_ammo` 0xB72CA0
- **In:** EAX = instance.
- **Out:** AL = 1 if any of these holds:
  - `+0x10 > 0` or `+0x14 > 0` (signed);
  - info != 0 and `u16 info+0xDC == 0`;
  - `info+0x10 & 0x20` and (`+0x18 > 0` or `+0x1C > 0`).
- **Clobbers:** ECX = info.
- **Crash:** with info == 0 and both counts <= 0, the original reads `[0x10]`. A port must keep that crash, or make the guard visible to the shadow check.

### 8. `weapon_info_ai_range_sq` 0xB71A60 (AI, every frame; sibling `weapon_info_ai_range` 0xB71A30)
- **In:** EAX = info.
- **Out:** ST0 = f32(r * r), with r = f32 `info+0xE4`, or `f32 0xE5C110` = 1.275 when null.
- The product of two f32 values is exact at 53 or 64 bits, so a single f32 multiply in Rust is bit-exact.
- **Clobbers:** ECX (push / pop slot only).
- **Writes:** nothing.

### 9. `weapon_instance_free` 0xB71C80
- **In:** EAX = node.
- **Unlink from the live list** (`L = 0x22D7B6C`; node +0 prev, +4 next):
  - If node is the head and alone (`next == node`): L = 0 and skip the unlink.
  - If node is the head and not alone: L = next, then unlink.
  - Unlink: `if prev: prev->next = next; if next: next->prev = prev`.
- **Insert into the free list** (`F = 0x22D7BC4`):
  - First zero node prev / next.
  - F == 0 → node->prev = node->next = node, F = node.
  - Otherwise insert before F: `node->prev = F->prev; node->next = F; F->prev->next = node; F->prev = node`.
- **Clobbers:** ECX, EDX. **Preserved:** ESI.
- **Writes:** the two list heads and the links of up to four nodes. This is the same pattern as `audio_stream_issue_reads`.

### 10. `weapon_info_max_reserve` 0x9C3F50
- **In:** EAX = info; cdecl (u8 alt).
- **Out:** EAX = 0 when info or the ammo type is null; else `type +0x10` (MP) or `type +0xC`.
- **Clobbers:** CL, DL.
- **Writes:** nothing.

### 11. `audio_foley_find_by_hash` 0x479A50 (siblings: `audio_voice_line_find_by_hash` 0x47AFB0, `audio_music_find_by_hash` 0x47A2B0)
- **In:** EDX = &hash.
- **Steps:** EAX = 0; CX = u16 `0x2526AF0`; loop with u16 i < CX, comparing `*(0xEE5558[i])` with hash.
- **Out:**
  - Found: EAX = i (upper 16 bits 0).
  - Not found: `or ax, 0xFFFF` → EAX = 0x0000FFFF.
- **Clobbers:** ECX (CX = count, upper bits kept), EDX = the hash value.
- **Preserved:** ESI.
- **Siblings:** the voice-line version compares `[0x2574360 + 16*i]` directly; the music version uses `0xEE7178` and `0x2526AF4`.
- **Threads:** both. Read-only after load.

### 12. `audio_voice_set_volume` 0x480070 (both threads: ambient emitters on the audio thread, vehicle audio on the game thread)
- **In:** ECX = &handle; cdecl (f32).
- **Handle check:**
  - AL = byte [ECX]. AL == 0xFF → fail.
  - ECX = dword >> 8. ECX == 0 → fail.
  - slot = AL × 0x19C. `gen != slot+0xB4` or `slot+0 == -1` → fail.
- **Fail:** AL = 0.
- **Success:**
  - `slot+0x24 = volume`. This goes through `fld` / `fstp`, so an sNaN comes out quieted.
  - If `!(u16 +0x196 & 0x800)`, set `+0x196 |= 1`.
  - AL = 1. Upper EAX holds slot × 0x19C bits.
- **Clobbers:** EAX, ECX.
- **Writes:** +0x24 and +0x196. There is no lock: snapshot both fields around the call.

### 13. `audio_voice_get_volume` 0x466140
- The same handle check as `audio_voice_set_volume`.
- **Out:** ST0 = f32 `slot+0x24`, or 1.0 when the handle is stale.
- **Clobbers:** EAX, ECX. One stack slot is used for f32 rounding, which is exact for f32 input.

### 14. `audio_voices_stop_category` 0x47F200 (game thread: `gameplay_exit`, pause-menu quit)
- **In:** EDX = category (-1 = all).
- **Loop:** over 128 slots (unrolled x4; base `0xEEB8F8`, stride 0x19C). For each slot:
  - If `+0 != -1 || (+0x196 & 0x200)`, and `+0xA4 == EDX || EDX == -1`:
  - set `+0x198 |= 0x400` and `+0x194 &= 0xFF7F`.
- **Preserved:** EBX, EBP, ESI, EDI. Clobbers EAX, ECX.
- **Writes:** +0x194 and +0x198 of the matching slots. A shadow check can snapshot all 128 × 2 words.
- **Sibling:** `audio_voices_mark_dirty_all` 0x47F2E0 has the same loop, with condition `!(+0x196 & 0x800)` and action `+0x196 |= 1`.

### 15. `audio_set_category_throttle` 0x46ED20
- **In:** ECX = category (unsigned, must be < 4); cdecl (f32 v).
- **Store:** `0xEE34F8[ECX] = v` exactly when `!(v < 0)` and `!(1.0 < v)` on the x87 compare. NaN sets C0, so it is rejected.
- **Clobbers:** EAX (`fnstsw`).
- **Writes:** one f32.
- **Threads:** called on the game thread. `audio_volume_zones_update` reads the value on the audio thread.

Next in line: `audio_voice_slot_fade_out` 0x47C740. It is x87 with an unsigned `fild` fix-up (+2^32 when the int is negative), a 0.5 multiply for ms < 32, and an f32 round trip before the max compare (`test ah, 5; jp` keeps the old value when the new one is smaller or unordered). Its wrappers `audio_voice_fade_out` / `_2` are a handle check and a tail jump.

## Bugs, doc errors and surprises

- **`light_find_by_name` 0x91DD40 is misnamed** (doc 33 / functions.csv). It references no light string. It sends `human_pkt_send_3e` and `human_pkt_send_3c` and calls `ammo_store_clear`, `ammo_store_find` and `ammo_store_add_type` twice. It is an ammo-store reset / restore (callers 0x693D40 / 0x693D80 / 0x694140, near the save code). It should be renamed (for example `player_ammo_store_restore`) after checking the callers.
- **`ambient_audio_sequences_update` 0x490940 probably is the radio station update.** Its records (0x48 bytes at `*0xF53094`) are the radio stations: `radio_station_find_by_name` and `luafn_radio_set_station` (via 0x48E470) use the same table.
- **`0x2526B44` is a pause depth, not only a cutscene flag** (doc 10, per-frame step 10). `audio_pause_all` and `audio_voices_fade_out_all` increment it and `audio_resume_all` decrements it. `audio_play` / `audio_prepare` refuse cues without params flag bit 1 while it is positive.
- `weapon_instance_has_ammo` dereferences a null info (see recipe 7).
- `weapon_total_ammo`'s alt path adds the primary clip (see recipe 1).
- Doc 10 lists X3DAudioCalculate at 0x4821C0. 0x4821C0 is the per-voice 3D commit, which calls 0x480920 (probably the calculate) and `xact3d_apply` 0x480F00.
- `audio_voice_fade_out` / `audio_voice_fade_out_2` and `audio_voice_set_orientation` / `_2` are byte-identical LTCG duplicates. A detour must hook both copies.
- Several 0x465xxx stubs (0x465570..0x4666D0) have no direct callers. They look like an exported audio interface table and are left unnamed.
- 0x464E70 (a 7-argument 3D params init) has no callers at all. It is dead code and is left out.
