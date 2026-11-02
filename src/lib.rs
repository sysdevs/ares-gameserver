//! `ares-gameserver` — a LostCity rev 274 game server and its cache tooling in Rust.
//!
//! Binaries: `ares` (world + game port), `ares-login`, `ares-friend`,
//! `ares-logger` (standalone servers, also spawned by `ares` under
//! `easyStartup`), `ares-pack` (the build tooling).
//!
//! Module map:
//!
//! | module | contents |
//! |---|---|
//! | `util` | config, numbers, base37, JavaRandom, trig, quicksort |
//! | `io` | Packet, Isaac, bzip2, gzip, Jagfile, FileStream |
//! | `cache` | config decoders, CrcTable, wordenc, midi, OnDemand |
//! | `pack` | the pack build, map, unpack and RSA tooling |
//! | `script` | bytecode loader, VM, opcode handlers |
//! | `routefinder`, `rsbuf` | pathfinding, player/NPC info buffers |
//! | `engine` | coord, zones, map, entities, inventory, World |
//! | `net` | game protocol, client sockets, TCP |
//! | `link` | links to the login/friend/logger servers as `Link { Ws, Inline, Off }` |
//! | `server` | the login, friend and logger servers and their database |
//! | `web` (feature `web-client`) | web client game routes and WebSocket |

pub mod cache;
pub mod engine;
pub mod io;
pub mod link;
pub mod net;
pub mod pack;
pub mod routefinder;
pub mod rsbuf;
pub mod script;
pub mod server;
pub mod util;
#[cfg(feature = "web-client")]
pub mod web;
