//! `ares-bots`: load test with mock players over TCP. Each bot logs in like the client, teleports to its anchor
//! with `::tele` (so it knows where it is without decoding server packets),
//! then random-walks around it, sometimes chats, and drains whatever the
//! server sends. Needs a dev world (`production` off, so bots may `::tele`).
//!
//! ```text
//! ares-bots [options]
//!   --host <addr>      game port (default 127.0.0.1:43594)
//!   --root <dir>       server root, for data/pack CRCs and data/config/public.pem (default .)
//!   --count <n>        bots (default 100)
//!   --rate <n>         logins started per second (default 20)
//!   --at <x,z>         anchor tile (default 3222,3218, Lumbridge)
//!   --spread <n>       each bot's anchor is offset up to n tiles (default 0: one crowd)
//!   --radius <n>       walk radius around the anchor (default 8)
//!   --chat <p>         chance per tick of a public chat line (default 0.02)
//!   --secs <n>         stop after n seconds (default: until Ctrl-C)
//!   --prefix <s>       username prefix (default bot)
//! ```

use ares_gameserver::cache::crc_table::CrcTable;
use ares_gameserver::io::filestream::FileStream;
use ares_gameserver::io::isaac::Isaac;
use ares_gameserver::io::packet::{Packet, RsaKey};
use ares_gameserver::net::client::{frame_packet, login_packet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MOVE_GAMECLICK: u8 = 207;
const CLIENT_CHEAT: u8 = 224;
const MESSAGE_PUBLIC: u8 = 253;
const NO_TIMEOUT: u8 = 120;
const CHAT: [&str; 4] = ["hello", "selling lobsters", "anyone want to trade", "nice weather"];

struct Opts {
    host: String,
    root: PathBuf,
    count: u32,
    rate: f64,
    at: (i32, i32),
    spread: i32,
    radius: i32,
    chat: f64,
    secs: Option<u64>,
    prefix: String,
}

fn parse() -> Result<Opts, String> {
    let mut o = Opts { host: "127.0.0.1:43594".into(), root: ".".into(), count: 100, rate: 20.0, at: (3222, 3218), spread: 0, radius: 8, chat: 0.02, secs: None, prefix: "bot".into() };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let v = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let num = |v: &str| v.parse::<f64>().map_err(|_| format!("{flag}: not a number: {v}"));
        match flag.as_str() {
            "--host" => o.host = v.clone(),
            "--root" => o.root = v.into(),
            "--count" => o.count = num(v)? as u32,
            "--rate" => o.rate = num(v)?.max(0.1),
            "--at" => {
                let (x, z) = v.split_once(',').ok_or("--at wants x,z")?;
                o.at = (num(x)? as i32, num(z)? as i32);
            }
            "--spread" => o.spread = num(v)? as i32,
            "--radius" => o.radius = num(v)? as i32,
            "--chat" => o.chat = num(v)?,
            "--secs" => o.secs = Some(num(v)? as u64),
            "--prefix" => o.prefix = v.clone(),
            _ => return Err(format!("unknown option {flag}")),
        }
    }
    if o.prefix.len() + o.count.to_string().len() > 12 {
        return Err("usernames are at most 12 characters: shorten --prefix".into());
    }
    Ok(o)
}

#[derive(Default)]
struct Stats {
    /// Bot tasks alive (logging in or online).
    connecting: AtomicU64,
    online: AtomicU64,
    failed: AtomicU64,
    dropped: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    /// Login reply codes other than 2, by code.
    codes: std::sync::Mutex<std::collections::BTreeMap<u8, u64>>,
}

/// xorshift64*: per-bot randomness without a new dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, r: i32) -> i32 {
        (self.next() * (2 * r + 1) as f64) as i32 - r
    }
}

struct Login {
    key: RsaKey,
    crc: Vec<u8>,
}

fn main() -> std::process::ExitCode {
    let o = match parse() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}\n(see the header of src/bin/ares-bots.rs for options)");
            return std::process::ExitCode::from(2);
        }
    };
    let key = match std::fs::read_to_string(o.root.join("data/config/public.pem")).map_err(|e| e.to_string()).and_then(|p| RsaKey::from_public_pem(&p)) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("data/config/public.pem: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    if key.e > key.n {
        // the server would decrypt garbage: every login fails, a few crash pre-fix servers
        eprintln!("data/config/public.pem has its modulus and exponent swapped (exponent is larger).");
        eprintln!("Regenerate it from the private key: openssl rsa -in data/config/private.pem -pubout -out data/config/public.pem");
        return std::process::ExitCode::FAILURE;
    }
    let crc = match FileStream::open(o.root.join("data/pack"), false) {
        Ok(mut cache) => CrcTable::new(&mut cache).buffer,
        Err(e) => {
            eprintln!("data/pack: {e} (run from the server root, or pass --root)");
            return std::process::ExitCode::FAILURE;
        }
    };
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    rt.block_on(run(Arc::new(o), Arc::new(Login { key, crc })));
    std::process::ExitCode::SUCCESS
}

async fn run(o: Arc<Opts>, login: Arc<Login>) {
    let stats = Arc::new(Stats::default());
    let start = Instant::now();
    let spawner = {
        let (o, login, stats) = (o.clone(), login.clone(), stats.clone());
        tokio::spawn(async move {
            let gap = Duration::from_secs_f64(1.0 / o.rate);
            for i in 1..=o.count {
                let (o, login, stats) = (o.clone(), login.clone(), stats.clone());
                tokio::spawn(async move {
                    stats.connecting.fetch_add(1, Relaxed);
                    let r = bot(i, &o, &login, &stats).await;
                    stats.connecting.fetch_sub(1, Relaxed);
                    if let Err(e) = r {
                        eprintln!("{}{i}: {e}", o.prefix);
                    }
                });
                tokio::time::sleep(gap).await;
            }
        })
    };
    let mut report = tokio::time::interval(Duration::from_secs(5));
    report.tick().await;
    let (mut last_in, mut last_out, mut last_t) = (0, 0, Instant::now());
    loop {
        tokio::select! {
            _ = report.tick() => {}
            _ = tokio::signal::ctrl_c() => break,
        }
        let (bin, bout) = (stats.bytes_in.load(Relaxed), stats.bytes_out.load(Relaxed));
        let dt = last_t.elapsed().as_secs_f64();
        let codes: Vec<String> = stats.codes.lock().unwrap().iter().map(|(c, n)| format!("{c}x{n}")).collect();
        println!(
            "[{:>4}s] online {} connecting {} failed {} dropped {} | in {:.1} KB/s out {:.1} KB/s{}",
            start.elapsed().as_secs(),
            stats.online.load(Relaxed),
            stats.connecting.load(Relaxed).saturating_sub(stats.online.load(Relaxed)),
            stats.failed.load(Relaxed),
            stats.dropped.load(Relaxed),
            (bin - last_in) as f64 / dt / 1024.0,
            (bout - last_out) as f64 / dt / 1024.0,
            if codes.is_empty() { String::new() } else { format!(" | reject codes {}", codes.join(" ")) },
        );
        (last_in, last_out, last_t) = (bin, bout, Instant::now());
        if o.secs.is_some_and(|s| start.elapsed().as_secs() >= s) {
            break;
        }
    }
    spawner.abort();
    // dropping the runtime closes every socket; the server logs the bots out
}

/// One bot: handshake, login, then the behaviour loop until the socket dies.
async fn bot(i: u32, o: &Opts, login: &Login, stats: &Stats) -> Result<(), String> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ ((i as u64) << 17 | 1));
    let mut sock = TcpStream::connect(&o.host).await.map_err(|e| format!("connect: {e}"))?;
    sock.set_nodelay(true).ok();
    sock.write_all(&[14, 0]).await.map_err(|e| e.to_string())?;
    let mut hello = [0u8; 17];
    sock.read_exact(&mut hello).await.map_err(|e| format!("handshake: {e}"))?;
    let seeds = [rng.next(), rng.next(), rng.next(), rng.next()].map(|f| (f * i32::MAX as f64) as i32);
    let username = format!("{}{i}", o.prefix);
    sock.write_all(&login_packet(&login.key, &login.crc, &seeds, i as i32, &username, "password", false, false)).await.map_err(|e| e.to_string())?;
    let mut code = [0u8; 1];
    sock.read_exact(&mut code).await.map_err(|e| format!("login reply: {e}"))?;
    if code[0] != 2 {
        stats.failed.fetch_add(1, Relaxed);
        *stats.codes.lock().unwrap().entry(code[0]).or_default() += 1;
        return Ok(());
    }
    let mut rest = [0u8; 2];
    sock.read_exact(&mut rest).await.map_err(|e| e.to_string())?;
    stats.online.fetch_add(1, Relaxed);
    let result = play(sock, seeds, o, stats, &mut rng).await;
    stats.online.fetch_sub(1, Relaxed);
    stats.dropped.fetch_add(1, Relaxed);
    result
}

async fn play(sock: TcpStream, seeds: [i32; 4], o: &Opts, stats: &Stats, rng: &mut Rng) -> Result<(), String> {
    let (mut rd, mut wr) = sock.into_split();
    let mut isaac = Isaac::new(&seeds);
    let anchor = (o.at.0 + rng.range(o.spread), o.at.1 + rng.range(o.spread));
    let cheat = |s: &str| {
        let mut b = s.as_bytes().to_vec();
        b.push(10);
        b
    };
    // `::tele level,mx,mz,lx,lz`
    let tele = format!("tele 0,{},{},{},{}", anchor.0 >> 6, anchor.1 >> 6, anchor.0 & 63, anchor.1 & 63);
    let mut out = frame_packet(&mut isaac, CLIENT_CHEAT, &cheat(&tele));
    let mut drain = [0u8; 8192];
    let mut tick = tokio::time::interval(Duration::from_millis(600));
    let mut since_send = 0;
    loop {
        tokio::select! {
            n = rd.read(&mut drain) => {
                let n = n.map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("server closed the connection".into());
                }
                stats.bytes_in.fetch_add(n as u64, Relaxed);
            }
            _ = tick.tick() => {
                if rng.next() < 0.25 {
                    let (x, z) = (anchor.0 + rng.range(o.radius), anchor.1 + rng.range(o.radius));
                    let mut p = Packet::with_len(5);
                    p.p1(0);
                    p.p2(x as i64);
                    p.p2(z as i64);
                    out.extend(frame_packet(&mut isaac, MOVE_GAMECLICK, &p.data));
                }
                if rng.next() < o.chat {
                    let mut p = Packet::with_len(100);
                    ares_gameserver::cache::word_pack::pack(&mut p, CHAT[(rng.next() * CHAT.len() as f64) as usize]);
                    let mut b = vec![0, 0];
                    b.extend_from_slice(&p.data[..p.pos]);
                    out.extend(frame_packet(&mut isaac, MESSAGE_PUBLIC, &b));
                }
                since_send += 1;
                if out.is_empty() && since_send >= 30 {
                    out.extend(frame_packet(&mut isaac, NO_TIMEOUT, &[]));
                }
                if !out.is_empty() {
                    since_send = 0;
                    stats.bytes_out.fetch_add(out.len() as u64, Relaxed);
                    wr.write_all(&out).await.map_err(|e| e.to_string())?;
                    out.clear();
                }
            }
        }
    }
}
