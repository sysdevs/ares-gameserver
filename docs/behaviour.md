# Behaviour rules

ares-gameserver promises **reproducible `data/pack` bytes** for a given content tree and
**deterministic outbound packet bytes per tick** for given client input bytes and
seeds. The game client and the content were written against JavaScript number,
string and collection semantics, so many of these rules are JS semantics kept on
purpose. Each item names where the rule lives and what must not change. The
committed goldens (`docs/testing.md`) pin all of it; a change that moves output
bytes is a deliberate re-bless, never a side effect.

The JS-derived semantics still in the code are all deliberate, because content
or the client observes them: script VM maths in `f64` folded back to int32
through `util::numbers::wrap_f64` (B.17); script strings measured and indexed in
UTF-16 units (B.21); the `Mulberry32` world RNG and the shared `JavaRandom`
stream (B.16, B.18); `linklist` processing order (B.12); readline-style config
splitting and `^CONST` substitution rules (B.7). Everything else is plain Rust:
the VM stacks are `Vec`s and an empty-stack pop is a script error, font widths
are integers with `None` for a missing glyph, and errors name what is missing.

## A. Crates

| crate | version | why | feature notes |
|---|---|---|---|
| `ares-compiler` | path `../ares-compiler` | `script.dat/.idx`, `CompilerTypeInfo` | — |
| `ares-shared` | path `../ares-shared` | the script ABI shared with ares-compiler: opcodes + pointer rules, triggers, var type codes (bytecode and config dialects), script file decode | — |
| `bzip2` | 0.6 | every jag archive | `default-features = false, features = ["bzip2-sys","static"]` → libbz2 1.0.8 C. The crate's default `libbz2-rs-sys` backend is not used |
| `flate2` | 1.1 | gzip for cache archives 1–4 and `maps-client`; gunzip at load | `zlib`, no default (B.3) |
| `png` | 0.18 | sprite / title / texture decode | content PNGs must be 8-bit RGBA; anything needing quantisation is an error |
| `crc32fast` | 1.5 | `Packet::getcrc`, CrcTable, versionlist | result cast to `i32` |
| `foldhash` | 0.2 | collision map hasher (never iterated into output) | — |
| `indexmap` | 2.14 | every insertion-ordered map/set that reaches output (B.1, B.12–B.16) | `serde` |
| `serde` / `serde_json` | 1 | `world.json`, link messages, test snapshots | `derive` / `preserve_order` (`Object.entries` emulation), `float_roundtrip` (snapshot floats parse to the exact f64) |
| `tokio` | 1.53 | sockets, internal links, OnDemand task | `rt-multi-thread,net,sync,time,macros,io-util,fs,signal` |
| `tokio-tungstenite` | 0.30 | internal WS links, client (World side) and server (the three link bins) | — |
| `axum` + `tower` + `tower-http` | 0.8 / 0.5 / 0.7 | optional, feature `web-client`: static `public/`, `/rs2.cgi`, archive routes, WS game bridge | `ws` / `util` / `fs` |
| `futures-util` | 0.3 | `SinkExt`/`StreamExt` on WebSocket streams | `sink`, no default |
| `sqlx` | 0.9 | login / friend / logger DB, through `AnyPool` | `runtime-tokio,sqlite,mysql,migrate,any,macros`; schema in `migrations/` |
| `bcrypt` | 0.19 | login server password hashes (`$2a$`/`$2b$`) | — |
| `rsa` | 0.9 | PEM parsing and `ares-pack keygen` only | `pem` |
| `num-bigint` | 0.5 | the raw CRT `modpow` for login decryption (B.19) | — |
| `notify` | 8.2 | dev hot-reload watcher | — |
| `tracing` / `tracing-subscriber` | 0.1 / 0.3 | logging | `env-filter` |

Deliberately **not** added: `zip` (`maps-server.zip` is a stored zip; the small
reader/writer in `pack/stored_zip.rs` controls entry order), `byteorder`, `clap`
(a handful of flags: `std::env::args`), `image` (no quantise path).

## B. Tooling rules (pack bytes)

### B.1 Directory walk order drives ids and sector layout — `pack/tools.rs`, `pack/parse.rs`, `pack/pack_file.rs`
Every directory listing is sorted **bytewise**, always. The order decides
config-name crawl ids (`pack.register(max++)`), category crawl (`.loc` first,
then `.npc`, then `.obj`), media jag entry order, and the order
`main_file_cache.dat` sectors are written for models/anims/midis.

### B.2 bzip2 parameters and the jag hash — `io/jagfile.rs`, `io/bzip2.rs`
Compress with **block size 1, work factor 30**, strip the 4-byte `BZh1` header;
decompress prepends it. Output buffer is `len + 1024` (min 128) and overflow is
an error. Whole-archive mode (`interface`) vs per-file mode. `gen_hash`:
`(h*61 + c - 32)` with `i32` wrapping over the **Unicode-upper-cased** name.

### B.3 gzip bytes — `io/gzip.rs`
Level 6, OS byte zeroed, MTIME 0. Used for archives 1–4 and `maps-client`; the
CRCs of those gzip bytes land in `versionlist`. The archive-1–4 snapshots were
first produced by a zlib fork whose bytes stock zlib does not reproduce, so the
pack gate compares members decompressed and checks versionlist CRCs against
each side's own gzip bytes (`docs/testing.md`). `PACK_STRICT_GZIP=1` makes it
byte-exact.

### B.4 `FileStream` sector algorithm — `io/filestream.rs`
First sector `max(1, trunc((dat_len + 519) / 520))`; each sector = 8-byte header
`p2 file, p2 part, p3 next, p1 archive+1` + 512 data bytes; the next sector
index is recomputed from the **current** `.dat` length; the last sector is
**not** padded; idx entry `p3 size, p3 sector` at `file * 6`; archives 1–4
append `p2 version(1)` so an empty file is `[0,1]`. Archive 0 is written in the
order 2, 3, 1, 4, 6, 7, 8, 5 (`pack/all.rs` step order). Builds are always
clean; `read` after `write` returns what was written.

### B.5 `Packet` write coercions and two bugs — `io/packet.rs`
`pjstr` writes the low byte of each UTF-16 unit (`é` → 0xE9, U+FFFD → 0xFD);
`p1/p2` wrap mod 2^8/2^16, `p4` wraps to i32, NaN → 0 (`p2(-1)` = `FFFF`).
`Packet` takes `i64` and wraps like JS `ToUint8/16/ToInt32`; a script `f64`
goes through `util::numbers::wrap_f64` first. Two bugs are kept (section C): `pdata`
advances `pos` by `length - offset`; `getcrc` loops `i < length`, not
`offset + length`. `gjstr` drops the last byte when unterminated; `g_var_int`
is unsigned.

### B.6 Content numbers are strict ints — `pack/parse.rs` (`int`, `int_at`)
Every number in content text (configs, `.if`, `.jm2`, csvs, `labels.txt`,
`.order`, `.pack`, `.opt`, numbered keys like `frame3`/`recol2s`/`stock1`) is
`-?` then decimal digits or `0x`/`0X` hex, parsed as `i64`; anything else
(`12abc`, `1.5`, `''`) is a pack error naming `file:line`. An absent optional
key still takes its default. Values wider than the field wrap explicitly at
the write (`PackedData`/`Packet` `p1..p4` on `i64`), so `0xFFFFFFFF` and
`4294967297` keep their old bytes.

### B.7 Config text rules — `pack/config/shared.rs`, `pack/parse.rs`
Config files are split readline-style (`\n`, `\r\n` and a lone `\r`) and **not**
trimmed; full-file parsing trims and strips `//` and `/* */` (and so splits on `\r?\n`
only: a lone-`\r` file shows the name crawl only its first `[config]`). `^CONST`
substitution keeps scanning from the **old** string index after a replacement;
duplicate constants are an error in configs but silently overwrite in the
script compiler's constant table (C). The value part of `key,value` is the
whole string when there is no comma (autoint enum `val=` lines rely on it).

### B.8 Sparse arrays — `pack/config/client.rs`, `pack/config/server.rs`
Recolour and seq arrays leave holes: `Vec<Option<T>>` sized by the highest index
written; a hole is written as 0. Seq frames: missing iframe → `-1`, missing
delay → `0`.

### B.9 Floating point that reaches output — `pack/config/client.rs`, `util/color.rs`, `pack/worldmap.rs`
Weight is an int with a `kg`/`oz`/`lb`/`g` unit: `n * 1000` / `* 28.3495` /
`* 453.592` in f64, range-checked, then truncated toward zero; HSL conversion
in f64 then int32 truncation (`util::numbers::wrap_f64`). Same operations in the same order on `f64`. The
worldmap overlay shape `(opcode - 2) / 4` is the integer `(opcode - 2) >> 2`
(opcodes there are 2..=49, so the float and integer forms agree).

### B.10 `Object.entries`, symbol tables and map step maps — `pack/compiler.rs`, `pack/map.rs`
Integer-like keys ascending first, then insertion order;
dbcolumn id `(table << 12) | (col << 4) | tuple`; land/loc/npc/obj maps keep
first-appearance order and server npc/obj blobs are written in that order;
`loc_list` sorts stably. Scripts are compiled from `../content/scripts`
regardless of `build.srcDir` (C); `script.dat` embeds that root
(`meta.json.scriptRoot`).

### B.11 Misc tooling cliffs
`pack/worldmap.rs` errors when there are more flo configs than
`worldmap_colors::REF_COLORS` rows; `pack/name_map.rs` reads `.order`/`.pack`
files as ASCII (high bit cleared); media sheets with an `.opt` file sort
(stably) to the end; `verify=false` registers and saves ids for transmitted
packs too. Both modes are supported: fixtures build with `verify=false` (their
jags cannot match the real-content CRC oracles), real content with
`verify=true`. Packed data buffers are 2 MB and overflow is an error.
`hitsplat` is a client-visible config the original client lacks: it goes into
the `config` jag after `varbit` with no CRC check (`client_unchecked`). Client
opcodes: `1` `sprite=<media sheet>,<index>` as one jstr, `40+N`/`50+N`
`recolNs`/`recolNd` as `p3` RGB24 pixel values (no HSL conversion). The server
half holds only the debug name. `damage`/`npc_damage` reject an id with no
hitsplat config.
Config `npc_stat` param values use their own order (`pack/config/shared.rs`
`NPC_STATS`: hitpoints, attack, strength, defence, magic, ranged), not the
script symbol ids (`script/tables.rs`: attack 0 … hitpoints 3).

## B. Engine rules (wire bytes and state)

### B.12 Player processing order — `engine/world/player_tick.rs`, `engine/linklist.rs`
The player loop is a hash table with 8 buckets keyed by **IP address**:
iteration is bucket `(ip & 7)` ascending, then insertion order within a bucket.
It decides who acts first every tick. New players are an insertion-ordered set
whose order decides slot (pid) assignment (first free in 1..2046). The replay
scenarios give each virtual client its own IP to exercise it.

### B.13 Set-ordered entity selection in rsbuf — `rsbuf/build.rs`
Zone player/npc sets are walked in insertion order when choosing which new
entities enter a player's view under the preferred-count cap; the packet
contents depend on it. `IndexSet` with `shift_remove`; never `HashSet`.

### B.14 Zone events, build area and other ordered sets — `engine/zone/`, `rsbuf/build.rs`, `engine/entity/player.rs`
Zone event order gives zone message order and the shared enclosed buffer;
build-area active/loaded zone order gives zone-update message order (and
updating deletes while iterating); player timers (re-set keeps position),
player invs (save file order), world invs, tracked zones and wealth transaction
groups are all insertion-ordered. `IndexMap`/`IndexSet` everywhere one is
iterated; `shift_remove` to delete.

### B.15 Login replies arrive between cycles — `link/login.rs`, `engine/world/client.rs`
Link replies are delivered between `cycle()` calls, never mid-phase. Reply codes
map to client bytes (`-1`→8, `1`→3, `3`→5, `5`→4, `6`→9, `7`→11, `8`→16,
`9`→12, `10`→`[21, secs]`, `0/2/4` proceed). The World queues requests in
`login_out` / `friend_out`; the runtime (or the replay harness, through
`link::login::inline`) hands replies back with `World::on_login_reply`
between cycles, so a reply lands at the start of the next tick, exactly once.

### B.16 `JavaRandom` — `util/java_random.rs`
The seed is **global**: every `JavaRandom::new(seed)` reseeds the one generator
everyone shares; the 48-bit LCG uses 16-bit limbs; the `next_int(bound)`
rejection test `bits - val + (bound-1) < 0` can never fire (C), so it differs
from Java; scripts use `next_double() * n` truncated to int32.

### B.17 Number ops arithmetic — `script/ops/number.rs`, `script/state.rs`
Every push is int32-truncated. `MULTIPLY` is f64 (loses precision past 2^53 —
not `wrapping_mul`); `DIVIDE`/`MODULO`/`SCALE` by 0 give 0; `POW` overflow to
infinity gives 0, a finite result past 2^32 wraps through `wrap_f64` (past 2^53
its low bits follow `powf`); `1 << bit` masks with `& 31`; `ABS(i32::MIN)`
stays; `INTERPOLATE` floor-divides; `DATE_MINUTES`/`DATE_RUNEDAY` read the
injected clock. Popping an empty stack aborts the script (`int stack underflow`
/ `string stack underflow`). `ENUM` with a missing key pushes
the *int* default even for string enums. Out-of-range bit-range masks are 0.

### B.18 Time and randomness — `engine/world.rs`, `engine/world/npc.rs`, `engine/entity/pathing.rs`, `routefinder/naive.rs`, `script/ops/server.rs`
The clock drives the tick scheduler, `last_login_time` (`g8`), mute checks and
logger timestamps (UTC); the world RNG (`Mulberry32`, standing in for
`Math.random`) drives NPC wander and hunt targets, AFK rolls, pmId and the
login seed (`p4(floor(random * 0xffffffff))` wraps). Both are injected:
`World { clock: Clock, rng }`, so replays pin them.

### B.19 RSA with signed big integers — `io/packet.rs`
The login block is read as a **signed** two's-complement big integer;
decryption is CRT (`p, q, dP, dQ, qInv`); the result must start with 10.
`BigInt::from_signed_bytes_be` + `modpow`; the PEM is read as raw PKCS#8/PKCS#1
ASN.1 fields as stored. Never `rsa::RsaPrivateKey`: it validates, and keys the
client has shipped with fail its checks (`KeyMalformed`).

### B.20 Output granularity and buffer cliffs — `engine/world/client.rs`, `net/tcp.rs`
One socket send per message: `p1(opcode + isaac)`, 1- or 2-byte size
placeholder back-patched; `out` is 5000 bytes so a larger message is an error
at the same site; `in` is 65535 bytes; per-tick decode limits USER 5 /
CLIENT 20 / RESTRICTED 2. `write_inner` pushes one `Vec<u8>` per message; the
replay snapshots record bytes per send.

### B.21 Script loader and VM quirks — `script/provider.rs`, `script/runner.rs`, `script/iterators.rs`, `script/ops/string.rs`
The lookup key is read with `g4s` but compared to `0xffffffff`, so `-1` keys are
always inserted (C); opcode handler tables must not overlap (asserted);
iterators are guarded by `current_tick > tick`; string ops use UTF-16 length,
`substring` argument swap/clamp, Unicode lower-casing. A string containing a
unit with no glyph has no width (`FontType::string_width` is `None`): it fits no
`split_init` width and exceeds none.

### B.22 NPC ids follow zip entry order — `engine/game_map.rs`
`maps-server.zip` entries are read in archive order (never sorted) and decide
nid assignment at startup; the pack side writes them bytewise-sorted.

### B.23 Save file and config quirks — `engine/entity/player.rs`, `util/config.rs`
Save: magic `0x2004`, version 8, `g2 x, g2 z, g1 level`, 7 body bytes
(255 = -1), 5 colours, gender, `g2 runenergy`, `g4 playtime`, 21 × (`g4 xp`,
`g1 level`), varps as `g2 n` × (`g2 id`, VarInt), invs in player inv order with
`255` + `g4` for large counts, afk zones, chat modes byte, `g8 lastLoginTime`,
CRC32 trailer; the login server refuses a save whose `playtime` regressed.
From v8 each occupied inv slot is followed by its varo block: `g1 n` ×
(`g2 varo id`, `g4 value`), holding only the vars that differ from their type
default (int 0, other -1), so an item without vars writes `00`. On load an id
`>= varos.count()` is dropped, and `vars` is allocated only when an entry
remains; saves before v8 load with no vars.
Config: `node.hopTime` falls back to `NODE_MAX_NPCS` (C); integer fields pass
JSON numbers through unchanged (floats survive) and strings holding a whole
integer (trimmed); any other string keeps the default.

### B.24 Server links and the database — `server/`, `server/db.rs`
sqlx `Any` decodes neither backend's DATETIME: read them through `Db::text`
(`CAST(.. AS CHAR)` on mysql, `CAST(.. AS TEXT)` on sqlite). An sqlite URL must
be `sqlite:<path>`; `sqlite://C:/..` parses `C` as a host. Dynamic SQL goes
through `sqlx::AssertSqlSafe`. `world_startup` clears this node's `logged_in`
rows, so it is sent once, never on a reconnect. Each connection's messages are
handled in order.

## C. Deliberate bugs (they reach output; do not fix)

| where | behaviour |
|---|---|
| `io/packet.rs` `pdata` | advances `pos` by `length - offset` |
| `io/packet.rs` `getcrc` | loop bound ignores `offset` |
| `pack/compiler.rs` | duplicate script constants silently overwrite (configs error) |
| `pack/compiler.rs` | compiles `../content/scripts` regardless of `build.srcDir` |
| `util/config.rs` | `node.hopTime` reads `NODE_MAX_NPCS` |
| `script/provider.rs` | `-1` lookup keys inserted |
| `util/java_random.rs` | `next_int` rejection test never fires |
| `engine/world` login | login seed `p4` of a value up to `0xffffffff` wraps |
