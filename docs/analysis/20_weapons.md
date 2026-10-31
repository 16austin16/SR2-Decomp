# 20: Weapons and damage

Names are in `docs/functions.csv` (subsystem `weapon`). Humans are in 15_humans.md, vehicles in 16_vehicles.md and contacts in 17_physics.md.

## Tables

| File | Loader | Contents |
|---|---|---|
| `weapons.xtbl` | 0xB71200 (from the game init 0x51F700; also opened by 0x68E560), which also loads `weapon_categories.xtbl` | one record per weapon; see the field list below |
| `weapon_categories.xtbl` | same | categories (`WPNCAT_PISTOL`, `_SUB_MACHINE_GUN`, `_SHOTGUN`, `_RIFLE`, `_THROWN`, `_MELEE`, `_SPECIAL`, `_DRUG`) |
| `explosions.xtbl` | `xtbl_load_explosions` (0x4BFE90) | explosion types |
| `damage_regions.xtbl` | `xtbl_load_damage_regions` (0x918920) | body-part damage regions |
| `melee.xtbl` | `xtbl_load_melee` (0x977540) | melee attacks |

**Weapon record fields**, as named in the binary:

| Group | Fields |
|---|---|
| Timing | `Refire_Delay`, `Pre/Post_Detonate_Delay`, `Warmup/Cooldown_Delay` |
| Inventory | `Inv_Slot`, `Weapon_Class` |
| Animation | `Fine_Aim_Animation_Group`, `Reload_Animation_Group` |
| Meshes | `Clip_Mesh`, `Left_Hand_Mesh` |
| Effects | `Muzzle_Flash` / `Muzzle_Effect` / `Tracer_Effect` |
| Sounds | `Fire_Sound` / `NPC_Fire_Sound` / … |
| Triggers | `Trigger_Type`, `Alt_Trigger_Type`, `Alt_Ammo` |
| Ammo and range | `Magazine_Size`, `Max_in_Reserve(_MP)`, `Range_Max`, `AI_Range_Max`, `Shots_Per_Round`, `Tracer_Frequency` |
| Damage | `NPC_Damage`, `Player_Damage`, `MP_Vehicle_Damage`, `Damage_Min/Max_Dist` |
| Spread | `PlayerWeaponSpread` / `NPCWeaponSpread` with movement multipliers (crouch, walk, run, sprint, vehicle, fine aim) |
| Fire cone | `Fire_Cone_*` (flame weapons) |
| Physics | `Ragdoll_Force_Shoot`, `Object_Bullet_Hit_Impulse_Magnitude` |
| Projectiles | `Projectile_Info` (speed, fuse, gravity, damping, ignition) |
| Zoom | `Zoom_Type` / FOV |
| Lock-on | `Target_Lockon` (`Lockon_Time_MS`, reticule angles) |
| Vehicle | `Vehicle_Weapon` |

## Firing

- `humans_post_update` (15_humans.md) keeps each human's held weapon attached to the hand (0x9C31B0), and runs **`weapon_fire_update`** (0x9C8EE0) when the trigger state says so.
- Out of ammo goes through 0x9707C0 (113 callers: the generic "has ammo / can fire" check).
- **`weapon_fire`** (0x9C0500, "firearm"): the firing entry. It is also called by co-op / multiplayer replay (`player_multi_registration::remote_do_frame`, `npc_…`) and by scripted shooters.
  - **Projectile weapons** (grenades, rockets, pipe bombs): 0x9BE6F0 → `projectile_create` (0x94C1A0). That creates a **type-4 object** (`projectile`, list `0x215E07C`) with a Havok body (`havok_body_create`). Projectiles update in `projectiles_update` (0x94C0A0, sim_frame:5) and explode through the explosion table.
  - **Hitscan** (bullets): **`weapon_bullet_trace`** (0x97B920) casts with `havok_all_ray_hit_collector` along the aim within `Range_Max`, resolves the human body part hit (l-thigh, r-thigh, pelvis, …), and applies damage.
- **Melee**: the melee table (`xtbl_load_melee`) and the kick-range debug commands (0x9722D0).

## Damage

**Humans.** `human_bullet_hit` (0x9763C0, debug text "you did %5.1f damage!") picks the body region and multiplier. Then:

**`human_take_damage`** (0x9847A0): ECX = attacker (or 0), EDX = damage flags (or the default `0x2C29218`), cdecl (victim, amount, vehicle, weapon).
1. If this machine doesn't own the victim, send the damage to its owner (packet 0x68, 08_multi_objects.md) and stop.
2. Scale the amount:
   - by difficulty (tables `0x14092F0` / `0x1409290` by level `0xE986F4`);
   - by multiplayer team / friendly-fire rules;
   - by armour (+0xD34, a percentage);
   - by the victim's multiplier (+0x1128) and the attacker's (+0x112C);
   - a negative amount means "kill".
3. Ignore it when the victim is invulnerable (+0x1160 / +0x1163 flags, +0xF80) or scripted not to take damage from this attacker (+0x110C bits 6 and 10).
4. Record the last attacker +0xC0C, the vehicle / weapon +0xC10 / +0xC14 and the damage flags +0xC08. With +0x110C bit 7 and a fatal hit, the hit becomes "pending" (+0xC18) instead.
5. **`human_adjust_hit_points`**(-amount) (0x96A0A0) → **`human_set_hit_points`** (0x969C90): clamp to [0, max +0x1120] and store +0x1124. Remote humans send a "human/hit_points" property packet instead. Crossing 0 calls **`human_on_death`** (0x985370).
6. Reactions and flinch (0x98A900 / 0x98AAB0), stats, the health HUD, and the hit sound (0x6AE180).
7. At 0 HP: 0x9C8C20(victim, 1) (ragdoll / death).

`human_set_max_hit_points` (0x969ED0) sets +0x1120 and clamps the current value. The Lua bindings `set_current_hit_points` / `set_max_hit_points` call these directly.

**Vehicles.** `vehicle_take_damage` (0xAABCC0; (vehicle, amount, flags, attacker)) has 27 callers. Destruction raises `game_event_vehicle_destroyed` (16_vehicles.md).

**Contacts.** 0x70A230 (called from `contact_point_added`) is **not** a damage function. It records the pair (object, other object, body) into a per-object contact list (`0x175CD80[handle & 0xFFF]`, 12-byte nodes from the free list `0x2527DE4`, with a hit count). While a physics step is running it is processed later (0x70A020), otherwise at once (0x70A190). Impact damage is applied from those records and from the per-type contact handlers in 17_physics.md.

## Open

- The in-memory weapon info layout (field offsets from `weapons.xtbl`).
- The trigger / refire state machine in `weapon_fire_update`, and spread calculation.
- The handlers that turn contact records into impact damage.
