# Snapshot tests

`tests/differential.rs` holds one `#[test]` per gate. Every gate runs ares-gameserver
and compares the result with a committed snapshot under `tests/`. The snapshots
are the reference: they pin the behaviour rules in `docs/behaviour.md`, so a
change that moves pack bytes, wire bytes or state shows up as a failing gate.

```
cargo test --release --test differential            # all gates
cargo test --release --test differential vm_corpus  # one gate (name filter)
```

## Updating snapshots (bless)

When a change is intentional, rewrite the snapshots from Rust output, review
the diff, and commit it with the change:

```
BLESS=1 cargo test --release --test differential full_build_matches   # pack snapshots first
BLESS=1 cargo test --release --test differential                      # everything that reads them
cargo test --release --test differential                              # must be green
git diff tests/
```

The pack snapshots go first because the decoder, gamemap, VM, save and replay
gates load `tests/golden/<case>/data/pack`. `BLESS=1` rewrites only what
mismatches and leaves inputs stored inside an expected file untouched.
`BLESS=all` rewrites every JSON snapshot even when it matches: `git diff`
staying empty afterwards proves the writer reproduces the files byte for byte.
A few slots are not blessable (their input is derived from the expected value);
the gate says so and they are edited by hand.

## Pack snapshots (`tests/fixtures`, `tests/golden`)

`tests/fixtures/<case>/content/` is a content tree written by
`python -I tools/make_pack_fixtures.py [<Content>]`. Text configs and the `.jm2`
mapsquares are authored in the script so every edge case is explicit; binary
assets (`.png`, `.ob2`, `.anim`, `.mid`, fonts, `title.jpg`, `wordenc`) are
copied from a LostCity Content checkout (default `../Content`). Fixtures build
with `build.verify = false`: with verify on, the interface, sounds and config
jags are checked against real-content CRC oracles only the full tree satisfies.
Regenerating fixtures changes pack output, so re-bless afterwards.

`tests/golden/<case>/` holds the clean-build output of that tree:
`data/pack/**` (`server/*`, `client/*`, `main_file_cache.dat` + `.idx0-4`,
`mapview/worldmap.jag`, `.cache/maps-server.zip`), `pack/*.pack`, and
`meta.json` (`verify`, `walkOrder`, `scriptRoot`). Every case also holds
`decoders.json` (every config decoder loaded from the pack, gate
`decoders_match`), `gamemap.json` (the map loaded into a World,
`gamemap_matches`) and `symbols.json` (the compiler symbol tables
`ares-pack symbols` writes, `symbols_match`). `pack_goldens_present` checks
each case has its core files.

| case | exercises |
|---|---|
| `01-minimal` | one of every config kind, one interface, one sprite sheet with `.opt`, one font, one texture, one synth, one jingle, one song, one model + anim, one mapsquare, one `.rs2` script: the smallest tree that completes a full build |
| `02-ordering` | names whose bytewise and NTFS orders differ, several configs per file, transmitted packs with gaps and missing names, categories across loc/npc/obj, `cert_` and `_unpack` names |
| `03-numbers` | `^CONST` chains that shorten the value, hex and >32-bit ints, fractional (oz/lb) and negative weights, sparse recol/seq/stock/count/mesanim arrays, quoted CSV, CRLF and lone-CR files, trailing spaces |
| `04-maps` | several mapsquares: bridges, every wall/decor shape, active locs, mapscene/mapfunction, minimap-hidden npcs, obj counts over 255, the m33_72 level exception, ignored/multiway/free zones, labels; plus `tests/oneoffs/*.csv` for the map importers (`tests/golden/oneoffs`) |
| `05-vm` | scripts for the VM corpus |
| `06-npcs` | NPC scripts and spawns for the NPC simulation |
| `07-varo` | varo configs, a ring and a stackable obj, the player invs and `inv_slotobj` procs for `tests/varo.rs` |

`import_csv_matches` runs `ares-pack import-npc-csv` / `import-obj-csv` on a
copy of the `04-maps` maps and compares against `tests/golden/oneoffs/<kind>`.

### Comparison policy

| artifact | compare | gate |
|---|---|---|
| `pack/*.pack`, `server/*.dat/.idx`, `server/interface.dat`, `server/script.dat/.idx` | byte-exact | `pack_ids_match`, `config_jag_matches`, `interface_matches`, `script_pack_matches` |
| bzip2 jags: `client/{config,interface,title,media,textures,wordenc,sounds,versionlist}`, `mapview/worldmap.jag`, cache archive 0 | byte-exact (B.2) | `jagfile_matches`, `sprite_jags_match`, … |
| archives 1–4 members: models, anims, midi, maps-client | per `(archive, file)`: decompressed payload + trailing version bytes equal; the Rust `.dat` re-read through its own FileStream. `PACK_STRICT_GZIP=1` → byte-exact | `archive_models_anims_midi_match`, `maps_match` |
| `client/versionlist` | model index, versions, entry order byte-exact; `*_crc` entries checked as `crc == crc32(own gzip bytes)` on each side (B.3) | `versionlist_matches` |
| `.cache/maps-server.zip` | entry names, order and bytes; zip headers ignored | `maps_match` |
| everything above, end to end | one clean build of the fixture | `full_build_matches` |

The partial gates re-derive one stage each from the same snapshot; only
`full_build_matches` blesses the pack.

## Wire replay (`tests/replay`)

`<name>.json` is a scenario (seed, clients with IP, ISAAC seeds and login
details, client message bytes per tick, optional `friendIn`); `<name>.expected.json`
holds the account saves fed in and, per tick, every send (bytes as sent, in
send order), close and save. The World is built from a pack snapshot, login
requests are answered in process by `link::login::inline` and fed back
through `World::on_login_reply`, and everything nondeterministic is pinned:

| source | pinned as |
|---|---|
| world RNG | `Mulberry32(seed)` (wander, hunt, AFK rolls, login seed, pmId) |
| `JavaRandom` | seeded from the scenario |
| clock | `base + tick * 600 + call offset` |
| link replies | queued and drained at the start of the next tick (B.15) |
| player order | each client's scenario IP (B.12) |
| NPC ids | the pack snapshot's `maps-server.zip` entry order (B.22) |
| login RSA key | RSA corpus key 0 (it only encrypts the client's login block) |

Scenarios: `idle`, `interact`, `multi` (several IPs, staggered logins, logout and
reconnect), `social` (friend-server messages, shutdown: `cycle()` reports
completion on the recorded tick).

## Server links (`tests/servers`, gate `servers_match`)

`python -I tools/make_server_steps.py` writes `steps.json`: messages on named
WebSocket connections (`w10`/`w11` login as nodes 10 and 11,
`f10`/`f11`/`fbad`/`relay` friend, `log` logger). The gate starts the login,
friend and logger servers in a scratch root (sqlite, schema from `migrations/`),
sends each step, waits for a `replyTo` answer when there is one, and compares
what each connection received (ending in `"close"`), every table by rowid and
the saves. Normalised: `password` → `<hash>`, wall-clock columns → `<now>`, a
positive hop-timer `remaining` → `<ms>`.

## Engine checks

| gate | checks |
|---|---|
| `dev_reload_rebuilds` | the live-reload watcher rebuilds `data/pack` once a source change settles, reports a failed build, and `World::reload` picks the new pack up |
| `malformed_login_terminates_client` | a login block with an unterminated username terminates that client and the world survives |
| `npc_script_parks_on_player` | an NPC script that suspends on its player (`p_delay`) is parked on that player |

`tests/varo.rs` (its own test binary) runs the `07-varo` procs against a
hand-built World: obj vars on inventory items, read and written through
`active_invobj`, and their save round trip.

## Unit corpora (`tests/<corpus>/`)

| corpus | covers |
|---|---|
| `packet` (+ `color`) | `Packet` p*/g* op sequences incl. wrap, negative, NaN, strings, bits, smart, varint; colour conversion. Inputs from `tools/make_packet_corpus.py` |
| `rsa`, `isaac` | key handling and block decryption; keystreams |
| `jag` | `gen_hash` over names (gate `gen_hash_matches`) |
| `util` | JavaRandom sequences, trig tables, base37, XP table, quicksort |
| `chat` | WordEnc filter, word pack/unpack |
| `routefinder` | seeded collision maps + path/naive/reach/line queries |
| `rsbuf` | per-tick positions/appearances → player/npc info bytes (`calls.json.gz`) |
| `engine`, `zones` | inventory and zone-event op sequences → state and bytes (`zones`: gate `zone_events_match`, on the `04-maps` World) |
| `linklist` | the intrusive list arena |
| `vm` | `05-vm` scripts run on a stub world → execution state, both stacks and their depths, script error |
| `messages`, `client` | server message encoding, client message decoding |
| `save` | player save load/resave |
| `npcsim` | `06-npcs` World with no players: per tick changed NPC rows, a crc32 of fast-changing counters, removed NPCs, tracked zone buffers, script errors |

## Keeping snapshots honest

- Never hand-edit a snapshot except the documented non-blessable slots; bless
  and review the diff instead.
- A new gate that passes first time gets a mutation check: break one expected
  value, confirm the gate fails, restore (or `BLESS=1` and confirm the diff is
  empty).
