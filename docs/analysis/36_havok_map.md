# 36: Havok library map

The game links **Havok 5.5.0-r1** statically. 17_physics.md covers the game side of the boundary; this doc maps the library itself. Names are in `docs/functions.csv` (subsystem `havok`, notes ending "(36)").

## What is named and what isn't

- 0xC20000-0xD60000 holds about 3,540 functions (Havok plus some game code at the edges); about 1,820 of them are unnamed. Physics glue (`havok.cpp`) is at 0x6F0000-0x720000 and is in 17_physics.md.
- About 1,800 Havok functions are already named. They are the virtual methods of Havok's RTTI classes (`hkp*`, `hk*`, from 03_rtti_classes.md and `tools/harvest_names.py`).
- The unnamed rest are non-virtual helpers and math. There's no public source for this Havok version to match against. The release build keeps very few strings: only some source paths in asserts and a handful of error messages.

## Source files seen in the binary

Asserts that survived keep their source paths, which places these modules:

| Module | Range | Files |
|---|---|---|
| `Dynamics` | 0xC601D0 | `World\Simulation\Continuous\hkpContinuousSimulation.cpp` |
| `Collide` | 0xC65180-0xC90BF0 | the MOPP virtual machines (`Mopp\Machine\hkpMopp*VirtualMachine.cpp`: Obb, LongRay, EarlyExitObb, Sphere, KDopGeometries, FindAll, Modify) |
| `System` | 0xD250A0-0xD98EA0 | `Io\Socket\Bsd\hkBsdSocket.cpp`, `Error\hkError.cpp` |
| `Memory` | 0xD2B9D0 | `hkThreadMemory.cpp` |
| `Dispatch` | 0xD3B210 | `hkpCollisionDispatcher.cpp` |
| `Util` | 0xD6A310 | `hkStructureLayout.cpp` |
| `Copier` | 0xD6AA80-0xD6B5A0 | `hkObjectCopier.cpp` |
| `Constraint` | 0xD89EA0-0xD9DA30 | `Bilateral\hkpConstraintUtils.cpp`, `Chain\hkpConstraintChainUtil.cpp`, `Chain\hkpPoweredChainMapper.cpp` |

The game's own source paths that survive are listed for completeness:
- `audio\gameaudio.cpp` 0x465020;
- `city\city_render*.cpp` 0x4B6760-0x4B8CE0;
- `game_render\*.cpp` 0x522E00-0x5285F0;
- `gamewide\gamewide.cpp` 0x69AB80;
- `havok\havok.cpp` and `havok_water_physics.cpp`;
- `objects\humans\human_processing.cpp` 0x9A6110;
- `trees\trees.cpp` 0xA8EEA0;
- `vehicle\vehicle_system.cpp` 0xAE2050;
- `unified\gr_mesh_decals_container.cpp` 0xD07CF0.

## Names added

- `hkpContinuousSimulation_queue_toi_event` (0xC601D0): it raises "Toi event queue full".
- `havok_raycast_group_pool_init` (0x6FB040) and `scheduler_test` (0x503630, the `scheduler_tester.cpp` diagnostic).
- The other 29 functions that reference exactly one source path are named `<file>_func` (for example `hkpMoppSphereVirtualMachine_func` or `hkObjectCopier_func_2`). This places each function in its source file, but its role inside the file isn't known.

## For the port

17_physics.md recommends replacing Havok rather than porting it: a Rust build should link another physics engine behind the boundary documented there. Given that, the unnamed Havok helpers don't need names. The virtual methods named from RTTI are enough to see which Havok features the game uses: rigid bodies, the character proxy, vehicles (`hkpVehicle*`), MOPP mesh shapes, constraints and phantoms.

## Open

- A match against a later Havok SDK, if one becomes available, for the non-virtual helpers.
