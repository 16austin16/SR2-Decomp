# 24: GameSpy SDK

The PC build statically links the GameSpy SDK (the online service shut down in 2014). Names are in `docs/functions.csv` (subsystem `gamespy`, notes start with "auto: GameSpy SDK match").

## Layout

The SDK sits at about 0x8D3000-0x910000. SR2's own glue code comes just before it, at 0x8CF000-0x8D3000.

Each SDK library is linked as one block. Inside a block the object files are in alphabetical order, and each file's functions keep their source order.

| Library | Range (matched part) | Matched | Files seen |
|---|---|---|---|
| core (root + `common/`) | 0x8D3490-0x8DD5D0 | 63 | darray, hashtable, md5c, gsLargeInt, gsPlatformUtil, gsStringUtil, gsUdpEngine, gsXML |
| sake (cloud storage) | 0x8DDDF0-0x8DF6B0 | 44 | sakeRequest, sakeRequestMisc, sakeRequestModify, sakeRequestRead |
| sc (ATLAS competition / stats) | 0x8DF960-0x8E1880 | 14 | sciInterface, sciMain, sciSerialize, sciWebServices |
| qr2 (query and reporting) | 0x8E2160-0x8E36A0 | 6 | qr2 |
| serverbrowsing | 0x8E3F50-0x8E7240 | 37 | sb_queryengine, sb_server, sb_serverbrowsing, sb_serverlist |
| webservices (login) | 0x8E8E30-0x8EA8A0 | 12 | AuthService |
| GP (presence and profiles) | 0x8EACA0-0x8FD5A0 | 226 | gp, gpi, gpiBuddy, gpiBuffer, gpiConnect, gpiInfo, gpiKeys, gpiOperation, gpiPeer, gpiProfile, gpiSearch, gpiTransfer, gpiUnique, gpiUtility |
| ghttp (HTTP / SOAP) | 0x8FDAD0-0x904B10 | 42 | ghttpBuffer, ghttpCallbacks, ghttpCommon, ghttpMain, ghttpPost, ghttpProcess |
| natneg (NAT negotiation) | from about 0x904B10; `natneg1-3.gamespy.com` at 0x905E90 | 1 | NATify |
| pt (patching / MOTD) | about 0x90B490-0x90BB80 | 2 | ptMain (`motd.gamespy.com`, `fileplanet.com`) |

The block after ghttp (0x904B10-0x910000, about 335 functions) holds natneg, pt and probably gt2 / Voice2. It has almost no strings, so most of it is still unnamed.

The SR2 glue at 0x8CF000-0x8D3000 holds the `gamespy_*_module` classes (03_rtti_classes.md):
- **Matchmaking**: game keys `gameplay_mode`, `gamever`, `mapid`, `custom_game` and `password`, and the game name `srow2pc`.
- **Sake storage**: the tables `user_page_storage` / `user_file_storage`, with filters such as `ownerid=%i AND page_id=%i`.
- **Search filters**: for example `%s and %s LIKE '%%%s%%'`.

## How the names were matched

- **Source**: the GameSpy SDK copy in openmohaa (`code/gamespy`, GPL), which is close to the version SR2 shipped. The game's own files there (`gserver.c`, `sv_gqueryreporting.c`, `cl_gamespy.c` and so on) and the console-platform files were left out.
- **Anchors**: a string literal or a numeric constant (with `#define` values resolved) that appears in exactly one SDK function and is referenced by exactly one exe function. Release builds drop the debug strings (`gsDebugFormat`), so only about a fifth of the SDK's strings are in the exe.
- **Call order**: for each matched pair, the source's call sequence is aligned with the exe's direct calls. Already-known callees, CRT functions and Winsock imports (`wsock32` by ordinal) act as fixed points. Unknown callees in equal-sized gaps are paired.
- **Link order**: between two anchors in the same library, the remaining source functions are aligned with the exe functions in address order. The alignment scores how well calls, strings and constants agree, and skips SDK functions the linker dropped as unused. Library edges and gaps with extra exe functions only keep strong pairs.
- **Clean-up**: per file, only matches in increasing address order are kept, and weak matches surrounded by other files are dropped.

These steps repeat until no new pairs appear.

Spot checks (callee lists and strings against the source) agreed. The notes carry the similarity score; scores under about 2 come from tight gaps, where position alone decides the match.

## Open

- The block after ghttp (natneg, gt2, Voice2): it needs anchors from the SR2 modules that call it.
- SR2's glue modules, at 0x8CF000-0x8D3000.
- The game no longer needs any of this for single player. A Rust port can stub the `gamespy_*_module` interfaces instead of porting the SDK.
