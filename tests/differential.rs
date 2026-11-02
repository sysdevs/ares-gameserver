//! Snapshot gates: every golden under `tests/` is a frozen snapshot of ares-gameserver
//! output, and each `#[test]` requires the current output to reproduce it.
//!
//! Layout:
//!   tests/fixtures/<case>/content/   a content tree (see tools/make_pack_fixtures.py)
//!   tests/golden/<case>/             its build: data/pack/**, pack/*.pack, meta.json and
//!                                    decoders.json, gamemap.json, symbols.json
//!   tests/<corpus>/                  inputs with the expected outputs
//!
//! Byte comparisons go through `assert_bytes_eq` so a failure points at an offset.
//!
//! Blessing an intentional behaviour change rewrites the expected files from
//! the current output; review the result with `git diff`:
//!   BLESS=1 cargo test --test differential full_build_matches
//!   BLESS=1 cargo test --test differential
//!   cargo test --test differential
//! The pack goes first and alone: decoders, gamemap, vm, save, replay and the
//! pack sub-gates read the pack goldens. `BLESS=1` rewrites only on a mismatch
//! (the pack only then); `BLESS=all` rewrites every JSON golden, which must
//! leave `git diff` empty (the writer's self-test).

use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn cases() -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(root().join("tests/fixtures"))
        .expect("tests/fixtures missing")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn golden(case: &str, file: &str) -> PathBuf {
    root().join("tests/golden").join(case).join(file)
}

fn golden_json(case: &str, file: &str) -> serde_json::Value {
    let p = golden(case, file);
    let text = fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Byte comparison with a hex window around the first mismatch.
fn assert_bytes_eq(what: &str, expected: &[u8], actual: &[u8]) {
    if expected == actual {
        return;
    }
    let first = expected
        .iter()
        .zip(actual)
        .position(|(a, b)| a != b)
        .unwrap_or(expected.len().min(actual.len()));
    let lo = first.saturating_sub(16);
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ");
    panic!(
        "{what}: first mismatch at offset {first} (expected len {}, actual len {})\n expected: {}\n actual:   {}",
        expected.len(),
        actual.len(),
        hex(&expected[lo..expected.len().min(first + 16)]),
        hex(&actual[lo..actual.len().min(first + 16)]),
    );
}

// ---------------------------------------------------------------- golden set

#[test]
fn pack_goldens_present() {
    let cases = cases();
    assert!(!cases.is_empty(), "no fixtures");
    for case in &cases {
        for f in [
            "meta.json",
            "pack/loc.pack",
            "pack/script.pack",
            "data/pack/server/script.dat",
            "data/pack/server/script.idx",
            "data/pack/server/loc.dat",
            "data/pack/client/config",
            "data/pack/client/versionlist",
            "data/pack/main_file_cache.dat",
            "data/pack/main_file_cache.idx0",
            "data/pack/main_file_cache.idx4",
            "data/pack/mapview/worldmap.jag",
            "data/pack/.cache/maps-server.zip",
        ] {
            assert!(golden(case, f).is_file(), "{case}: missing golden {f}");
        }
        let meta = golden_json(case, "meta.json");
        assert_eq!(meta["walkOrder"], "bytewise", "{case}: walk order not pinned");
    }
}

// ---------------------------------------------------------------- packet, color, rsa

fn corpus(dir: &str, file: &str) -> serde_json::Value {
    let p = corpus_path(dir, file);
    let text = fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The login RSA key for replay tests: key 0 of the RSA corpus. The key only
/// encrypts the client's login block, which is never part of a recording.
fn test_key() -> ares_gameserver::io::packet::RsaKey {
    ares_gameserver::io::packet::RsaKey::from_pem(corpus("rsa", "cases.json")["keys"][0].as_str().unwrap()).unwrap()
}

/// A JS number from the corpus (`"NaN"`, `"Infinity"`, `"-Infinity"` allowed).
fn js_num(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::String(s) if s == "NaN" => f64::NAN,
        serde_json::Value::String(s) if s == "Infinity" => f64::INFINITY,
        serde_json::Value::String(s) if s == "-Infinity" => f64::NEG_INFINITY,
        v => v.as_f64().unwrap_or_else(|| panic!("not a number: {v}")),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// Runs `f` with panics silenced; `Err` means it panicked.
fn quiet<R>(f: impl FnOnce() -> R) -> std::thread::Result<R> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    std::panic::set_hook(hook);
    r
}

fn run_packet_case(c: &serde_json::Value) -> serde_json::Value {
    use ares_gameserver::io::packet::Packet;
    use serde_json::{Value, json};
    let size = c["size"].as_u64().unwrap() as usize;
    let mut p = Packet::with_len(size);
    if let Some(init) = c["init"].as_str() {
        let b = unhex(init);
        let n = b.len().min(size);
        p.data[..n].copy_from_slice(&b[..n]);
    }
    let int = |v: &Value| ares_gameserver::util::numbers::wrap_f64(js_num(v)) as i64;
    let mut results = Vec::new();
    let mut error = Value::Null;
    for (i, op) in c["ops"].as_array().unwrap().iter().enumerate() {
        let op = op.as_array().unwrap();
        let name = op[0].as_str().unwrap();
        let a = &op[1..];
        let r = quiet(|| -> Value {
            match name {
                "setpos" => p.pos = a[0].as_u64().unwrap() as usize,
                "p1" => p.p1(int(&a[0])),
                "p2" => p.p2(int(&a[0])),
                "ip2" => p.ip2(int(&a[0])),
                "p3" => p.p3(int(&a[0])),
                "p4" => p.p4(int(&a[0])),
                "ip4" => p.ip4(int(&a[0])),
                "pVarInt" => p.p_var_int(int(&a[0])),
                "p8" => p.p8(a[0].as_str().unwrap().parse().unwrap()),
                "pbool" => p.pbool(a[0].as_bool().unwrap()),
                "pjstr" => p.pjstr(a[0].as_str().unwrap()),
                "psmart" => p.psmart(a[0].as_i64().unwrap() as i32),
                "psmarts" => p.psmarts(a[0].as_i64().unwrap() as i32),
                "psize1" => p.psize1(a[0].as_u64().unwrap() as usize),
                "psize2" => p.psize2(a[0].as_u64().unwrap() as usize),
                "psize4" => p.psize4(a[0].as_u64().unwrap() as usize),
                "pdata" => {
                    let src = unhex(a[0].as_str().unwrap());
                    p.pdata(&src, a[1].as_u64().unwrap() as usize, a[2].as_u64().unwrap() as usize)
                }
                "bitStart" => p.bit_start(),
                "bitEnd" => p.bit_end(),
                "pBit" => p.p_bit(a[0].as_u64().unwrap() as usize, int(&a[1])),
                "gBit" => return json!(p.g_bit(a[0].as_u64().unwrap() as usize)),
                "g1" => return json!(p.g1()),
                "g1b" => return json!(p.g1b()),
                "g2" => return json!(p.g2()),
                "g2s" => return json!(p.g2s()),
                "ig2" => return json!(p.ig2()),
                "g3" => return json!(p.g3()),
                "g3s" => return json!(p.g3s()),
                "g4" => return json!(p.g4()),
                "g4s" => return json!(p.g4s()),
                "g8" => return json!(p.g8().to_string()),
                "gbool" => return json!(p.gbool()),
                "gsmart" => return json!(p.gsmart()),
                "gsmarts" => return json!(p.gsmarts()),
                "gVarInt" => return json!(p.g_var_int()),
                "gjstr" => return json!(p.gjstr(a.first().map_or(10, |v| v.as_u64().unwrap() as u8))),
                other => panic!("unknown op {other}"),
            }
            Value::Null
        });
        match r {
            Ok(v) => results.push(v),
            Err(_) => {
                error = json!(i);
                break;
            }
        }
    }
    json!({ "results": results, "pos": p.pos, "bitPos": p.bit_pos, "data": hex(&p.data), "error": error })
}

/// Collects every failure and reports the first few.
fn report(what: &str, failures: Vec<String>) {
    if !failures.is_empty() {
        let first: Vec<_> = failures.iter().take(5).cloned().collect();
        panic!("{what}: {} mismatches, first 5:\n{}", failures.len(), first.join("\n"));
    }
}

/// `BLESS` unset: `None`; `BLESS=all`: `Some(true)` (always rewrite); any other value: `Some(false)`.
fn bless() -> Option<bool> {
    std::env::var("BLESS").ok().map(|v| v == "all")
}

/// A Rust value as the golden writes it: whole floats are integers.
fn js(v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Number(n) if n.is_f64() => match n.as_f64().unwrap() {
            f if f.fract() == 0.0 && f.abs() < 9e15 => Value::from(f as i64),
            _ => Value::Number(n),
        },
        Value::Array(a) => Value::Array(a.into_iter().map(js).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, js(v))).collect()),
        v => v,
    }
}

/// Rewrites `e` to equal `a`, keeping `e`'s key order and every part already equal.
fn patch(e: &mut serde_json::Value, a: serde_json::Value) {
    use serde_json::Value;
    if *e == a {
        return;
    }
    match (e, a) {
        (Value::Object(eo), Value::Object(ao)) => {
            eo.retain(|k, _| ao.contains_key(k));
            for (k, v) in ao {
                match eo.get_mut(&k) {
                    Some(ev) => patch(ev, v),
                    None => {
                        eo.insert(k, js(v));
                    }
                }
            }
        }
        (Value::Array(ea), Value::Array(aa)) => {
            ea.truncate(aa.len());
            for (i, v) in aa.into_iter().enumerate() {
                match ea.get_mut(i) {
                    Some(ev) => patch(ev, v),
                    None => ea.push(js(v)),
                }
            }
        }
        (e, a) => *e = js(a),
    }
}

/// Compact JSON, or one-space indented when the file being replaced is; LF-terminated.
fn write_json(path: &std::path::Path, v: &serde_json::Value) {
    use serde::Serialize;
    let mut out = Vec::new();
    if fs::read(path).is_ok_and(|b| b.get(1) == Some(&b'\n')) {
        v.serialize(&mut serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b" "))).unwrap();
    } else {
        serde_json::to_writer(&mut out, v).unwrap();
    }
    out.push(b'\n');
    fs::write(path, out).unwrap();
}

/// Under `BLESS` writes the patched expected value (on a mismatch, or always for `BLESS=all`); otherwise reports.
fn settle(what: &str, path: &std::path::Path, e: &serde_json::Value, failures: Vec<String>) {
    match bless() {
        Some(all) if all || !failures.is_empty() => write_json(path, e),
        Some(_) => {}
        None => report(what, failures),
    }
}

fn corpus_path(dir: &str, file: &str) -> PathBuf {
    root().join("tests").join(dir).join(file)
}

#[test]
fn packet_corpus_matches() {
    let inputs = corpus("packet", "inputs.json");
    let mut expected = corpus("packet", "expected.json");
    let mut failures = Vec::new();
    for (i, (c, e)) in inputs.as_array().unwrap().iter().zip(expected.as_array_mut().unwrap()).enumerate() {
        let actual = run_packet_case(c);
        if &actual != e {
            failures.push(format!("case {i}: ops {}\n  expected {e}\n  actual   {actual}", c["ops"]));
            patch(e, actual);
        }
    }
    settle("packet corpus", &corpus_path("packet", "expected.json"), &expected, failures);
}

#[test]
fn color_corpus_matches() {
    use ares_gameserver::util::color;
    let inputs = corpus("color", "inputs.json");
    let mut e = corpus("color", "expected.json");
    let ints = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap() as i32).collect::<Vec<_>>();
    let rgb24 = ints(&inputs["rgb24"]);
    let hsl: Vec<i32> = inputs["hsl24"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let t = ints(t);
            color::hsl24to16(t[0], t[1], t[2])
        })
        .collect();
    let got = serde_json::json!({
        "rgb15_hsl16": color::rgb15_hsl16_table().to_vec(),
        "rgb24_hsl16": rgb24.iter().map(|&v| color::rgb24_to_hsl16(v)).collect::<Vec<_>>(),
        "rgb24to15": rgb24.iter().map(|&v| color::rgb24to15(v)).collect::<Vec<_>>(),
        "rgb15to24": (0..32768).map(color::rgb15to24).collect::<Vec<_>>(),
        "hsl24to16": hsl,
    });
    let mut failures = Vec::new();
    for (k, v) in got.as_object().unwrap() {
        if e[k] != *v {
            failures.push(format!("{k}: {}", first_json_diff(&e[k], v)));
            patch(&mut e[k], v.clone());
        }
    }
    settle("color", &corpus_path("color", "expected.json"), &e, failures);
}

#[test]
fn rsa_corpus_matches() {
    use ares_gameserver::io::packet::{Packet, RsaKey};
    let mut c = corpus("rsa", "cases.json");
    let keys: Vec<RsaKey> = c["keys"].as_array().unwrap().iter().map(|k| RsaKey::from_pem(k.as_str().unwrap()).unwrap()).collect();
    let mut failures = Vec::new();
    for (i, d) in c["dec"].as_array_mut().unwrap().iter_mut().enumerate() {
        let mut p = Packet::new(unhex(d["buf"].as_str().unwrap()));
        p.rsadec(&keys[d["key"].as_u64().unwrap() as usize]);
        if hex(&p.data) != d["out"].as_str().unwrap() {
            failures.push(format!("dec {i}"));
            d["out"] = hex(&p.data).into();
        }
    }
    for (i, d) in c["enc"].as_array_mut().unwrap().iter_mut().enumerate() {
        let mut p = Packet::new(unhex(d["buf"].as_str().unwrap()));
        p.pos = d["pos"].as_u64().unwrap() as usize;
        p.rsaenc(&keys[d["key"].as_u64().unwrap() as usize]);
        if hex(p.written()) != d["out"].as_str().unwrap() {
            failures.push(format!("enc {i}"));
            d["out"] = hex(p.written()).into();
        }
    }
    settle("rsa corpus", &corpus_path("rsa", "cases.json"), &c, failures);
}

// ---------------------------------------------------------------- jagfiles

/// Every jag in a golden pack (bzip2 per entry or whole).
const JAGS: &[&str] = &[
    "data/pack/client/config",
    "data/pack/client/interface",
    "data/pack/client/title",
    "data/pack/client/media",
    "data/pack/client/textures",
    "data/pack/client/sounds",
    "data/pack/client/versionlist",
    "data/pack/mapview/worldmap.jag",
];

#[test]
fn gen_hash_matches() {
    let mut c = corpus("jag", "hashes.json");
    let names: Vec<String> = c["names"].as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_string()).collect();
    let mut failures = Vec::new();
    for (n, h) in names.iter().zip(c["hashes"].as_array_mut().unwrap()) {
        let got = ares_gameserver::io::jagfile::gen_hash(n) as i64;
        if got != h.as_i64().unwrap() {
            failures.push(format!("{n:?}"));
            *h = got.into();
        }
    }
    settle("genHash", &corpus_path("jag", "hashes.json"), &c, failures);
}

/// Decode every golden jag with the Rust reader, rebuild it from the decoded
/// entries with the Rust writer, and require the original bytes: covers the
/// container layout, entry order and bzip2 output together.
fn check_jags(label: &str, golden_dir: &std::path::Path) {
    use ares_gameserver::io::jagfile::Jagfile;
    for &jag in JAGS {
        let bytes = fs::read(golden_dir.join(jag)).unwrap();
        let src = Jagfile::from_bytes(bytes.clone());
        let mut out = Jagfile::new(src.compress_whole);
        for i in 0..src.len() {
            out.write_hash(src.hash(i), &src.get(i).unwrap());
        }
        assert_bytes_eq(&format!("{label}: {jag}"), &bytes, &out.to_bytes());
    }
}

#[test]
fn jagfile_matches() {
    for case in cases() {
        check_jags(&case, &root().join("tests/golden").join(&case));
    }
}

// ---------------------------------------------------------------- pack ids

/// A scratch copy of a fixture content tree with the regenerated packs removed,
/// and the build settings the golden was built with (`meta.json.verify`).
fn clean_content(case: &str, tag: &str) -> (PathBuf, ares_gameserver::util::config::Build) {
    let dst = std::env::temp_dir().join(format!("ares-{tag}-{case}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dst);
    copy_dir(&root().join("tests/fixtures").join(case).join("content"), &dst);
    for kind in ares_gameserver::pack::pack_file::REGENERATED {
        let _ = fs::remove_file(dst.join(format!("pack/{kind}.pack")));
    }
    let mut build = ares_gameserver::util::config::WorldConfig::default().build;
    build.src_dir = dst.to_string_lossy().replace('\\', "/");
    build.verify = golden_json(case, "meta.json")["verify"].as_bool().unwrap();
    (dst, build)
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            fs::copy(e.path(), to).unwrap();
        }
    }
}

fn assert_dir_files_eq(what: &str, expected_dir: &std::path::Path, actual_dir: &std::path::Path) {
    let mut names: Vec<_> = fs::read_dir(expected_dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    names.sort();
    for name in names {
        let name = name.to_string_lossy();
        let expected = fs::read(expected_dir.join(&*name)).unwrap();
        let actual = fs::read(actual_dir.join(&*name)).unwrap_or_else(|_| panic!("{what}: Rust did not write {name}"));
        if bless().is_some() {
            if expected != actual {
                fs::write(expected_dir.join(&*name), &actual).unwrap();
            }
        } else {
            assert_bytes_eq(&format!("{what}: {name}"), &expected, &actual);
        }
    }
}

// ---------------------------------------------------------------- decoders

/// Every decoder loaded from `<golden>/data/pack`, serialized with the
/// golden's field names, against `<golden>/decoders.json`.
fn check_decoders(label: &str, golden_dir: &std::path::Path) {
    use ares_gameserver::cache::config::{
        ConfigSet, component::Components, dbtable::DbTableType, flo::FloType, inv::InvType, loc::LocType, npc::NpcType, obj::ObjType,
        param::ParamType, var,
        category::CategoryType, dbrow::{DbKey, DbRowType, DbTableIndex}, enum_type::EnumType, hitsplat::HitsplatType, hunt::HuntType, idk::IdkType, mesanim::MesanimType,
        seq::{SeqType, load_anim_delays}, spotanim::SpotanimType, struct_type::StructType,
    };
    use ares_gameserver::cache::font::load_fonts;
    let pack = golden_dir.join("data/pack");
    let mut expected = golden_json_at(golden_dir, "decoders.json");
    let params = ConfigSet::<ParamType>::load(&pack, "param.dat").unwrap();
    let mut cache = ares_gameserver::io::filestream::FileStream::open(&pack, false).unwrap();
    let delays = std::sync::Arc::new(load_anim_delays(&mut cache));
    let dbtables = ConfigSet::<DbTableType>::load(&pack, "dbtable.dat").unwrap();
    let dbrows = ConfigSet::<DbRowType>::load(&pack, "dbrow.dat").unwrap();
    let fonts = load_fonts(&pack).unwrap();
    let v = |x: &dyn erased::Ser| x.value();
    let actual = [
        ("param", v(&params.configs)),
        ("dbtable", v(&dbtables.configs)),
        ("inv", v(&ConfigSet::<InvType>::load(&pack, "inv.dat").unwrap().configs)),
        ("varp", v(&ConfigSet::<var::VarPlayerType>::load_with_client(&pack, "varp.dat").unwrap().configs)),
        ("varbit", v(&ConfigSet::<var::VarBitType>::load_with_client(&pack, "varbit.dat").unwrap().configs)),
        ("varn", v(&ConfigSet::<var::VarNpcType>::load(&pack, "varn.dat").unwrap().configs)),
        ("vars", v(&ConfigSet::<var::VarSharedType>::load(&pack, "vars.dat").unwrap().configs)),
        ("flo", v(&ConfigSet::<FloType>::load_with_client(&pack, "flo.dat").unwrap().configs)),
        ("loc", v(&ConfigSet::<LocType>::load_with_client(&pack, "loc.dat").unwrap().configs)),
        ("npc", v(&ConfigSet::<NpcType>::load_with_client(&pack, "npc.dat").unwrap().configs)),
        ("obj", v(&ConfigSet::<ObjType>::load_objs(&pack, true, &params).unwrap().configs)),
        ("component", v(&Components::load(&pack).unwrap().components)),
        ("seq", v(&SeqType::load(&pack, delays).unwrap().configs)),
        ("idk", v(&ConfigSet::<IdkType>::load_with_client(&pack, "idk.dat").unwrap().configs)),
        ("spotanim", v(&ConfigSet::<SpotanimType>::load_with_client(&pack, "spotanim.dat").unwrap().configs)),
        ("category", v(&ConfigSet::<CategoryType>::load(&pack, "category.dat").unwrap().configs)),
        ("enum", v(&ConfigSet::<EnumType>::load(&pack, "enum.dat").unwrap().configs)),
        ("struct", v(&ConfigSet::<StructType>::load(&pack, "struct.dat").unwrap().configs)),
        ("mesanim", v(&ConfigSet::<MesanimType>::load(&pack, "mesanim.dat").unwrap().configs)),
        ("hitsplat", v(&ConfigSet::<HitsplatType>::load(&pack, "hitsplat.dat").unwrap().configs)),
        ("hunt", v(&ConfigSet::<HuntType>::load(&pack, "hunt.dat").unwrap().configs)),
        ("dbrow", v(&dbrows.configs)),
        ("font", v(&fonts)),
    ];
    let mut failures = Vec::new();
    for (kind, actual) in actual {
        // a kind the golden lacks reads as empty, so a bless can add it
        let e = expected[kind].as_array().map_or(&[][..], Vec::as_slice);
        let a = actual.as_array().unwrap();
        if e.len() != a.len() {
            failures.push(format!("{label}: {kind}: {} configs, expected {}", a.len(), e.len()));
        } else {
            for (i, (e, a)) in e.iter().zip(a).enumerate() {
                if e != a {
                    failures.push(format!("{label}: {kind} {i}: {}", first_json_diff(e, a)));
                }
            }
        }
        patch(&mut expected[kind], actual);
    }

    // DbTableIndex.rows: [[packed, [[value, [row ids]], ...]], ...]
    let index = DbTableIndex::new(&dbtables, &dbrows);
    let mut want = std::collections::HashMap::new();
    for entry in expected["dbindex"].as_array().unwrap() {
        let packed = entry[0].as_i64().unwrap() as i32;
        for pair in entry[1].as_array().unwrap() {
            let key = match &pair[0] {
                serde_json::Value::String(s) => DbKey::Str(s.clone()),
                n => DbKey::Int(n.as_i64().unwrap() as i32),
            };
            let ids: Vec<usize> = pair[1].as_array().unwrap().iter().map(|i| i.as_u64().unwrap() as usize).collect();
            want.entry(packed).or_insert_with(std::collections::HashMap::new).insert(key, ids);
        }
    }
    if want != index.rows {
        failures.push(format!("{label}: dbindex differs ({} columns, expected {})", index.rows.len(), want.len()));
        // written sorted: the comparison ignores order
        let mut cols: Vec<_> = index.rows.iter().collect();
        cols.sort_by_key(|c| c.0);
        let key = |k: &DbKey| match k {
            DbKey::Int(i) => (0, *i, String::new()),
            DbKey::Str(s) => (1, 0, s.clone()),
        };
        expected["dbindex"] = cols
            .into_iter()
            .map(|(packed, m)| {
                let mut kv: Vec<_> = m.iter().collect();
                kv.sort_by_key(|(k, _)| key(k));
                let kv: Vec<_> = kv.into_iter().map(|(k, ids)| serde_json::json!([match k { DbKey::Int(i) => serde_json::json!(i), DbKey::Str(s) => serde_json::json!(s) }, ids])).collect();
                serde_json::json!([packed, kv])
            })
            .collect();
    }

    let crc = ares_gameserver::cache::crc_table::CrcTable::new(&mut cache);
    let want_crc = serde_json::json!({ "table": crc.crcs, "buffer": crc.buffer, "crc32": crc.crc32 });
    if want_crc != expected["crc"] {
        failures.push(format!("{label}: crc table: {}", first_json_diff(&expected["crc"], &want_crc)));
        patch(&mut expected["crc"], want_crc);
    }
    let lengths = ares_gameserver::cache::midi::load_lengths(&mut cache);
    let want_midi: Vec<f64> = expected["midi"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    if lengths != want_midi {
        let i = lengths.iter().zip(&want_midi).position(|(a, b)| a != b);
        failures.push(format!("{label}: midi lengths ({} vs {}), first differing id {i:?}", lengths.len(), want_midi.len()));
        patch(&mut expected["midi"], serde_json::json!(lengths));
    }

    check_scripts(label, &pack, &mut expected["scripts"], &mut failures);

    for c in expected["fontSplit"].as_array_mut().unwrap() {
        let font = &fonts[c["font"].as_u64().unwrap() as usize];
        let text = c["str"].as_str().unwrap().to_string();
        let width = font.string_width(&text);
        let want_width = c["width"].as_i64();
        if width != want_width {
            failures.push(format!("{label}: stringWidth {text:?}: {width:?}, expected {want_width:?}"));
            patch(&mut c["width"], serde_json::json!(width));
        }
        for (max, want) in [50, 120, 300].iter().zip(c["split"].as_array_mut().unwrap()) {
            let got = serde_json::json!(font.split(&text, *max));
            if &got != want {
                failures.push(format!("{label}: split {text:?} {max}: {got}, expected {want}"));
                patch(want, got);
            }
        }
    }
    settle("decoders", &golden_dir.join("decoders.json"), &expected, failures);
}

/// `serde::Serialize` behind a trait object so the table above stays flat.
mod erased {
    pub trait Ser {
        fn value(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Ser for T {
        fn value(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap()
        }
    }
}

/// The first differing path between two JSON values, with both sides.
fn first_json_diff(e: &serde_json::Value, a: &serde_json::Value) -> String {
    use serde_json::Value;
    fn walk(path: String, e: &Value, a: &Value) -> Option<String> {
        match (e, a) {
            (Value::Object(eo), Value::Object(ao)) => {
                for (k, ev) in eo {
                    match ao.get(k) {
                        Some(av) => {
                            if let Some(d) = walk(format!("{path}.{k}"), ev, av) {
                                return Some(d);
                            }
                        }
                        None => return Some(format!("{path}.{k}: missing in Rust (expected {ev})")),
                    }
                }
                ao.keys().find(|k| !eo.contains_key(*k)).map(|k| format!("{path}.{k}: extra in Rust"))
            }
            (Value::Array(ea), Value::Array(aa)) if ea.len() == aa.len() => {
                ea.iter().zip(aa).enumerate().find_map(|(i, (ev, av))| walk(format!("{path}[{i}]"), ev, av))
            }
            _ if e == a => None,
            _ => Some(format!("{path}: expected {e}, actual {a}")),
        }
    }
    walk(String::new(), e, a).unwrap_or_default()
}

#[test]
fn decoders_match() {
    for case in cases() {
        check_decoders(&case, &root().join("tests/golden").join(&case));
    }
}

#[test]
fn pack_ids_match() {
    for case in cases() {
        let (dir, build) = clean_content(&case, "packs");
        ares_gameserver::pack::pack_file::revalidate(&build).unwrap_or_else(|e| panic!("{case}: {e}"));
        let mut names: Vec<_> = fs::read_dir(golden(&case, "pack")).unwrap().map(|e| e.unwrap().file_name()).collect();
        names.sort();
        for name in names {
            let name = name.to_string_lossy();
            let expected = fs::read(golden(&case, &format!("pack/{name}"))).unwrap();
            let actual = fs::read(dir.join("pack").join(&*name)).unwrap_or_else(|_| panic!("{case}: Rust did not write pack/{name}"));
            assert_bytes_eq(&format!("{case}: pack/{name}"), &expected, &actual);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------- configs

const CONFIG_TYPES: &[&str] = &[
    "param", "dbtable", "dbrow", "enum", "inv", "mesanim", "struct", "seq", "loc", "flo", "spotanim", "npc", "obj", "idk", "varp", "varbit", "hitsplat", "hunt", "varn", "vars",
];

/// Revalidates the packs and runs `packConfigs` on a clean content copy, then
/// compares every server config file and the client `config` jag (whose
/// entries are compared one by one first, so a failure names the type).
fn check_configs(label: &str, dir: &std::path::Path, build: &ares_gameserver::util::config::Build, golden_dir: &std::path::Path) {
    use ares_gameserver::io::{filestream::FileStream, jagfile::Jagfile};
    let packs = ares_gameserver::pack::pack_file::revalidate(build).unwrap_or_else(|e| panic!("{label}: {e}"));
    let out = dir.join("out/data/pack");
    let mut cache = FileStream::open(&out, true).unwrap();
    let flags = vec![0; packs.model.max as usize];
    ares_gameserver::pack::config::pack_configs(build, &packs, &out, &mut cache, flags).unwrap_or_else(|e| panic!("{label}: {e}"));

    let gpack = golden_dir.join("data/pack");
    let expected = Jagfile::from_bytes(fs::read(gpack.join("client/config")).unwrap());
    let actual = Jagfile::from_bytes(fs::read(out.join("client/config")).unwrap());
    assert_eq!(actual.len(), expected.len(), "{label}: config jag entry count");
    let names: Vec<String> = CONFIG_TYPES.iter().flat_map(|t| [format!("{t}.dat"), format!("{t}.idx")]).collect();
    for i in 0..expected.len() {
        let name = names.iter().find(|n| ares_gameserver::io::jagfile::gen_hash(n) == expected.hash(i)).map_or("?", String::as_str);
        assert_eq!(actual.hash(i), expected.hash(i), "{label}: config jag entry {i} order");
        assert_bytes_eq(&format!("{label}: client/config {name}"), &expected.get(i).unwrap(), &actual.get(i).unwrap());
    }
    assert_bytes_eq(&format!("{label}: client/config"), &fs::read(gpack.join("client/config")).unwrap(), &fs::read(out.join("client/config")).unwrap());

    let mut files = vec!["category.dat".to_string()];
    files.extend(CONFIG_TYPES.iter().flat_map(|t| [format!("{t}.dat"), format!("{t}.idx")]));
    for f in files {
        let expected = fs::read(gpack.join("server").join(&f)).unwrap();
        let actual = fs::read(out.join("server").join(&f)).unwrap_or_else(|_| panic!("{label}: Rust did not write server/{f}"));
        assert_bytes_eq(&format!("{label}: server/{f}"), &expected, &actual);
    }
}

// ---------------------------------------------------------------- versionlist

const VERSIONLIST: &[&str] = &[
    "model_version", "model_crc", "model_index", "anim_version", "anim_crc", "anim_index", "midi_version", "midi_crc", "midi_index", "map_version", "map_crc",
    "map_index",
];

/// `*_crc` must be the CRC of that side's own stored member; every other
/// versionlist entry, the entry order and the model flags are byte-exact.
fn check_versionlist(label: &str, dir: &std::path::Path, build: &ares_gameserver::util::config::Build, golden_dir: &std::path::Path) {
    use ares_gameserver::io::filestream::FileStream;
    use ares_gameserver::pack;
    let packs = pack::pack_file::revalidate(build).unwrap_or_else(|e| panic!("{label}: {e}"));
    let out = dir.join("out/data/pack");
    let mut cache = FileStream::open(&out, true).unwrap();
    let mut flags = pack::config::pack_configs(build, &packs, &out, &mut cache, vec![0; packs.model.max as usize]).unwrap_or_else(|e| panic!("{label}: {e}"));
    pack::interface::pack_client_interface(build, &packs, &out, &mut cache, &mut flags).unwrap_or_else(|e| panic!("{label}: {e}"));
    pack::graphics::pack_client_graphics(build, &packs, &mut cache, &flags).unwrap_or_else(|e| panic!("{label}: {e}"));
    pack::graphics::pack_client_midi(build, &packs, &mut cache).unwrap_or_else(|e| panic!("{label}: {e}"));
    let server = pack::map::pack_maps(build, &packs, &out, &mut cache, &mut flags).unwrap_or_else(|e| panic!("{label}: {e}"));
    let _ = server;
    let flags = pack::versionlist::model_flags(&flags, packs.model.max);
    pack::versionlist::pack_client_versionlist(build, &packs, &out, &mut cache, &flags).unwrap_or_else(|e| panic!("{label}: {e}"));

    let mut golden = FileStream::open(golden_dir.join("data/pack"), false).unwrap();
    let (e, a) = (fs::read(golden_dir.join("data/pack/client/versionlist")).unwrap(), fs::read(out.join("client/versionlist")).unwrap());
    assert_versionlist_eq(label, &e, &a, &mut golden, &mut cache);
}

/// Versionlist jags: same entry order, every entry byte-exact except `*_crc`,
/// whose values must be the CRCs of each side's own stored members.
fn assert_versionlist_eq(label: &str, expected: &[u8], actual: &[u8], golden: &mut ares_gameserver::io::filestream::FileStream, cache: &mut ares_gameserver::io::filestream::FileStream) {
    use ares_gameserver::io::{filestream::FileStream, jagfile::Jagfile, packet::Packet};
    let ej = Jagfile::from_bytes(expected.to_vec());
    let aj = Jagfile::from_bytes(actual.to_vec());
    assert_eq!((0..aj.len()).map(|i| aj.hash(i)).collect::<Vec<_>>(), (0..ej.len()).map(|i| ej.hash(i)).collect::<Vec<_>>(), "{label}: versionlist entry order");
    for (i, name) in VERSIONLIST.iter().enumerate() {
        let (e, a) = (ej.read(name).unwrap().data, aj.read(name).unwrap().data);
        assert_eq!(ej.hash(i), ares_gameserver::io::jagfile::gen_hash(name), "{label}: versionlist entry {i} is {name}");
        if let Some(kind) = name.strip_suffix("_crc") {
            let archive = ["model", "anim", "midi", "map"].iter().position(|k| *k == kind).unwrap() + 1;
            assert_eq!(e.len(), a.len(), "{label}: {name} length");
            for (j, (ec, ac)) in e.chunks(4).zip(a.chunks(4)).enumerate() {
                let own = |c: &mut FileStream| c.read(archive, j, false).map_or(0, |d| Packet::getcrc(&d, 0, d.len() - 2));
                assert_eq!(i32::from_be_bytes(ec.try_into().unwrap()), own(golden), "{label}: golden {name}[{j}] is not the CRC of its member");
                assert_eq!(i32::from_be_bytes(ac.try_into().unwrap()), own(cache), "{label}: Rust {name}[{j}] is not the CRC of its member");
            }
        } else {
            assert_bytes_eq(&format!("{label}: versionlist {name}"), &e, &a);
        }
    }
}

#[test]
fn versionlist_matches() {
    for case in cases() {
        let (dir, build) = clean_content(&case, "versionlist");
        check_versionlist(&case, &dir, &build, &root().join("tests/golden").join(&case));
        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------- maps

/// Configs first (the map step reads npc.dat, the worldmap flo/loc/npc), then
/// maps and the worldmap; archive 4 is compared decompressed, the server map
/// store by entries in order, the worldmap jag byte-exact.
fn check_maps(label: &str, dir: &std::path::Path, build: &ares_gameserver::util::config::Build, golden_dir: &std::path::Path) {
    use ares_gameserver::io::filestream::FileStream;
    let packs = ares_gameserver::pack::pack_file::revalidate(build).unwrap_or_else(|e| panic!("{label}: {e}"));
    let out = dir.join("out/data/pack");
    let mut cache = FileStream::open(&out, true).unwrap();
    let flags = ares_gameserver::pack::config::pack_configs(build, &packs, &out, &mut cache, vec![0; packs.model.max as usize]).unwrap_or_else(|e| panic!("{label}: {e}"));
    let mut flags = flags;
    let server = ares_gameserver::pack::map::pack_maps(build, &packs, &out, &mut cache, &mut flags).unwrap_or_else(|e| panic!("{label}: {e}"));
    ares_gameserver::pack::worldmap::pack_worldmap(build, &packs, &out, &server).unwrap_or_else(|e| panic!("{label}: {e}"));

    let gpack = golden_dir.join("data/pack");
    let mut golden = FileStream::open(&gpack, false).unwrap();
    assert_gzip_archive_eq(label, 4, &mut golden, &mut cache);

    let expected = ares_gameserver::pack::stored_zip::read(&fs::read(gpack.join(".cache/maps-server.zip")).unwrap()).unwrap();
    let actual = ares_gameserver::pack::stored_zip::read(&fs::read(out.join(".cache/maps-server.zip")).unwrap()).unwrap();
    assert_eq!(actual.keys().collect::<Vec<_>>(), expected.keys().collect::<Vec<_>>(), "{label}: maps-server.zip entry order");
    for (name, e) in &expected {
        assert_bytes_eq(&format!("{label}: maps-server.zip {name}"), e, &actual[name]);
    }

    let (e, a) = (fs::read(gpack.join("mapview/worldmap.jag")).unwrap(), fs::read(out.join("mapview/worldmap.jag")).unwrap());
    if e != a {
        let (ej, aj) = (ares_gameserver::io::jagfile::Jagfile::from_bytes(e.clone()), ares_gameserver::io::jagfile::Jagfile::from_bytes(a.clone()));
        for i in 0..ej.len().min(aj.len()) {
            assert_bytes_eq(&format!("{label}: worldmap.jag entry {i}"), &ej.get(i).unwrap(), &aj.get(i).unwrap());
        }
    }
    assert_bytes_eq(&format!("{label}: worldmap.jag"), &e, &a);
}

#[test]
fn maps_match() {
    for case in cases() {
        let (dir, build) = clean_content(&case, "maps");
        check_maps(&case, &dir, &build, &root().join("tests/golden").join(&case));
        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------- models, anims, midi

/// Every member of a gzip archive: same entries, same trailing version bytes,
/// same payload after gunzip (gzip bytes depend on the zlib build). With
/// `PACK_STRICT_GZIP=1` the stored bytes must match too.
fn assert_gzip_archive_eq(label: &str, archive: usize, expected: &mut ares_gameserver::io::filestream::FileStream, actual: &mut ares_gameserver::io::filestream::FileStream) {
    use ares_gameserver::io::gzip::decompress_gz;
    let strict = std::env::var_os("PACK_STRICT_GZIP").is_some();
    assert_eq!(actual.file_count(archive), expected.file_count(archive), "{label}: archive {archive} entry count");
    for f in 0..expected.file_count(archive) {
        let (e, a) = (expected.read(archive, f, false), actual.read(archive, f, false));
        let what = format!("{label}: archive {archive} file {f}");
        match (e, a) {
            (None, None) => {}
            (Some(e), Some(a)) => {
                assert!(e.len() >= 2 && a.len() >= 2, "{what}: no version suffix");
                assert_eq!(e[e.len() - 2..], a[a.len() - 2..], "{what}: version");
                let (eb, ab) = (&e[..e.len() - 2], &a[..a.len() - 2]);
                if strict || eb.is_empty() || ab.is_empty() {
                    assert_bytes_eq(&what, eb, ab);
                } else {
                    let (ed, ad) = (decompress_gz(eb).unwrap_or_else(|| panic!("{what}: golden gunzip")), decompress_gz(ab).unwrap_or_else(|| panic!("{what}: Rust gunzip")));
                    assert_bytes_eq(&format!("{what} (decompressed)"), &ed, &ad);
                }
            }
            (e, a) => panic!("{what}: present in golden {}, in Rust {}", e.is_some(), a.is_some()),
        }
    }
}

fn check_graphics(label: &str, dir: &std::path::Path, build: &ares_gameserver::util::config::Build, golden_dir: &std::path::Path) {
    use ares_gameserver::io::filestream::FileStream;
    let packs = ares_gameserver::pack::pack_file::revalidate(build).unwrap_or_else(|e| panic!("{label}: {e}"));
    let out = dir.join("out/data/pack");
    let mut cache = FileStream::open(&out, true).unwrap();
    ares_gameserver::pack::graphics::pack_client_graphics(build, &packs, &mut cache, &[]).unwrap_or_else(|e| panic!("{label}: {e}"));
    ares_gameserver::pack::graphics::pack_client_midi(build, &packs, &mut cache).unwrap_or_else(|e| panic!("{label}: {e}"));
    let mut golden = FileStream::open(golden_dir.join("data/pack"), false).unwrap();
    for archive in 1..=3 {
        assert_gzip_archive_eq(label, archive, &mut golden, &mut cache);
    }
}

#[test]
fn archive_models_anims_midi_match() {
    for case in cases() {
        let (dir, build) = clean_content(&case, "graphics");
        check_graphics(&case, &dir, &build, &root().join("tests/golden").join(&case));
        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------- sprite jags

fn check_sprite_jags(label: &str, dir: &std::path::Path, build: &ares_gameserver::util::config::Build, golden_dir: &std::path::Path) {
    use ares_gameserver::io::{filestream::FileStream, jagfile::Jagfile};
    use ares_gameserver::pack::sprite;
    let packs = ares_gameserver::pack::pack_file::revalidate(build).unwrap_or_else(|e| panic!("{label}: {e}"));
    let out = dir.join("out/data/pack");
    let mut cache = FileStream::open(&out, true).unwrap();
    let fail = |e: String| panic!("{label}: {e}");
    sprite::pack_client_title(build, &out, &mut cache).unwrap_or_else(fail);
    sprite::pack_client_media(build, &out, &mut cache).unwrap_or_else(fail);
    sprite::pack_client_texture(build, &packs, &out, &mut cache).unwrap_or_else(fail);
    let raw = root().join("data/raw/wordenc");
    sprite::pack_client_wordenc(build, &raw, &mut cache).unwrap_or_else(fail);
    sprite::pack_client_sound(build, &packs.synth, &out, &mut cache).unwrap_or_else(fail);

    let gpack = golden_dir.join("data/pack");
    for name in ["title", "media", "textures", "sounds"] {
        let (e, a) = (fs::read(gpack.join("client").join(name)).unwrap(), fs::read(out.join("client").join(name)).unwrap());
        if e != a {
            // name the first differing entry before the whole-file diff
            let (ej, aj) = (Jagfile::from_bytes(e.clone()), Jagfile::from_bytes(a.clone()));
            for i in 0..ej.len().min(aj.len()) {
                assert_eq!(ej.hash(i), aj.hash(i), "{label}: client/{name} entry {i} order");
                assert_bytes_eq(&format!("{label}: client/{name} entry {i}"), &ej.get(i).unwrap(), &aj.get(i).unwrap());
            }
        }
        assert_bytes_eq(&format!("{label}: client/{name}"), &e, &a);
    }
    assert_bytes_eq(&format!("{label}: cache 0/7 (wordenc)"), &fs::read(&raw).unwrap(), &cache.read(0, 7, false).unwrap());
}

#[test]
fn sprite_jags_match() {
    for case in cases() {
        let (dir, build) = clean_content(&case, "sprites");
        check_sprite_jags(&case, &dir, &build, &root().join("tests/golden").join(&case));
        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------- interface

fn check_interface(label: &str, dir: &std::path::Path, build: &ares_gameserver::util::config::Build, golden_dir: &std::path::Path) {
    use ares_gameserver::io::{filestream::FileStream, jagfile::Jagfile};
    let packs = ares_gameserver::pack::pack_file::revalidate(build).unwrap_or_else(|e| panic!("{label}: {e}"));
    let out = dir.join("out/data/pack");
    let mut cache = FileStream::open(&out, true).unwrap();
    let mut flags = vec![0; packs.model.max as usize];
    ares_gameserver::pack::interface::pack_client_interface(build, &packs, &out, &mut cache, &mut flags).unwrap_or_else(|e| panic!("{label}: {e}"));
    let gpack = golden_dir.join("data/pack");
    let data = |p: &std::path::Path| Jagfile::from_bytes(fs::read(p.join("client/interface")).unwrap()).read("data").unwrap().data;
    assert_bytes_eq(&format!("{label}: interface data"), &data(&gpack), &data(&out));
    assert_bytes_eq(&format!("{label}: client/interface"), &fs::read(gpack.join("client/interface")).unwrap(), &fs::read(out.join("client/interface")).unwrap());
    assert_bytes_eq(&format!("{label}: server/interface.dat"), &fs::read(gpack.join("server/interface.dat")).unwrap(), &fs::read(out.join("server/interface.dat")).unwrap());
}

#[test]
fn interface_matches() {
    for case in cases() {
        let (dir, build) = clean_content(&case, "interface");
        check_interface(&case, &dir, &build, &root().join("tests/golden").join(&case));
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn config_jag_matches() {
    for case in cases() {
        let (dir, build) = clean_content(&case, "configs");
        check_configs(&case, &dir, &build, &root().join("tests/golden").join(&case));
        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------- symbols, scripts

/// The symbol tables `run_server_compiler` hands the compiler, built after a
/// clean config + interface pack; returns the pack dir and the tables.
fn pack_symbols(label: &str, dir: &std::path::Path, build: &ares_gameserver::util::config::Build) -> (PathBuf, serde_json::Value) {
    use ares_gameserver::io::filestream::FileStream;
    use ares_gameserver::pack;
    let packs = pack::pack_file::revalidate(build).unwrap_or_else(|e| panic!("{label}: {e}"));
    let out = dir.join("out/data/pack");
    let mut cache = FileStream::open(&out, true).unwrap();
    let mut flags = pack::config::pack_configs(build, &packs, &out, &mut cache, vec![0; packs.model.max as usize]).unwrap_or_else(|e| panic!("{label}: {e}"));
    pack::interface::pack_client_interface(build, &packs, &out, &mut cache, &mut flags).unwrap_or_else(|e| panic!("{label}: {e}"));
    let actual = pack::compiler::symbols(build, &out).unwrap_or_else(|e| panic!("{label}: {e}"));
    (out, actual)
}

fn check_symbols(label: &str, dir: &std::path::Path, build: &ares_gameserver::util::config::Build, golden_dir: &std::path::Path) {
    let (_, actual) = pack_symbols(label, dir, build);
    let mut expected = golden_json_at(golden_dir, "symbols.json");
    if let Some(all) = bless() {
        if all || actual != expected {
            patch(&mut expected, actual);
            write_json(&golden_dir.join("symbols.json"), &expected);
        }
        return;
    }
    for (kind, e) in expected.as_object().unwrap() {
        let a = &actual[kind];
        assert!(a == e, "{label}: symbols.{kind}: {}", first_json_diff(e, a));
    }
    assert_eq!(actual.as_object().unwrap().len(), expected.as_object().unwrap().len(), "{label}: symbol kinds");
}

#[test]
fn symbols_match() {
    for case in cases() {
        let (dir, build) = clean_content(&case, "symbols");
        check_symbols(&case, &dir, &build, &root().join("tests/golden").join(&case));
        let _ = fs::remove_dir_all(&dir);
    }
}

/// `script.dat`/`.idx` from ares-compiler with the Rust-built symbols, with
/// source names under `meta.json.scriptRoot` (`../content/scripts/<rel>`).
fn check_script_pack(label: &str, dir: &std::path::Path, build: &ares_gameserver::util::config::Build, golden_dir: &std::path::Path) {
    let (out, _) = pack_symbols(label, dir, build);
    let root = golden_json_at(golden_dir, "meta.json")["scriptRoot"].as_str().unwrap().to_string();
    let output = ares_gameserver::pack::compiler::run_server_compiler(build, &out, ares_compiler::compiler::SourceNames::Prefixed(Box::leak(root.into_boxed_str())))
        .unwrap_or_else(|e| panic!("{label}: {e}"));
    assert!(output.stopped.is_none(), "{label}: compilation stopped at {:?}", output.stopped);
    for f in ["script.dat", "script.idx"] {
        let expected = fs::read(golden_dir.join("data/pack/server").join(f)).unwrap();
        assert_bytes_eq(&format!("{label}: server/{f}"), &expected, &fs::read(out.join("server").join(f)).unwrap());
    }
}

fn golden_json_at(dir: &std::path::Path, file: &str) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(dir.join(file)).unwrap()).unwrap()
}

#[test]
fn script_pack_matches() {
    for case in cases() {
        let (dir, build) = clean_content(&case, "scripts");
        check_script_pack(&case, &dir, &build, &root().join("tests/golden").join(&case));
        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------- full build, tools, util

fn files_under(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

/// `pack_all` (the `ares-pack build` path) into a scratch engine directory,
/// then every file of the golden `data/pack`: gzip archive members after
/// gunzip, the versionlist by the CRC rule, `maps-server.zip` by entries,
/// everything else (archive 0 included) byte-exact.
fn check_full_build(label: &str, content: &std::path::Path, golden_dir: &std::path::Path, verify: bool) {
    use ares_gameserver::io::filestream::FileStream;
    let root = std::env::temp_dir().join(format!("ares-build-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let engine = root.join("engine");
    copy_dir(content, &root.join("content"));
    let _ = fs::remove_dir_all(root.join("content/.git"));
    fs::create_dir_all(engine.join("data/raw")).unwrap();
    fs::copy(crate::root().join("data/raw/wordenc"), engine.join("data/raw/wordenc")).unwrap();
    let mut build = ares_gameserver::util::config::WorldConfig::default().build;
    build.src_dir = "../content".into();
    build.verify = verify;

    ares_gameserver::pack::all::clean(&engine, &build).unwrap();
    let names = ares_compiler::compiler::SourceNames::Prefixed("../content/scripts");
    ares_gameserver::pack::all::pack_all(&engine, &build, names).unwrap_or_else(|e| panic!("{label}: {e}"));

    // dev mode's data/symbols.json: the tables the compiler got, as the symbols gate pins them
    let written = ares_gameserver::pack::all::write_symbols(&engine, &build).unwrap_or_else(|e| panic!("{label}: {e}"));
    assert_eq!(written, engine.join("data/symbols.json"));
    if bless().is_none() {
        let actual: serde_json::Value = serde_json::from_str(&fs::read_to_string(&written).unwrap()).unwrap();
        assert!(actual == golden_json_at(golden_dir, "symbols.json"), "{label}: data/symbols.json differs from symbols.json");
    }

    let (gpack, apack) = (golden_dir.join("data/pack"), engine.join("data/pack"));
    let cmp = || {
        let mut golden = FileStream::open(&gpack, false).unwrap();
        let mut cache = FileStream::open(&apack, false).unwrap();
        let mut expected_files = files_under(&gpack);
        expected_files.retain(|f| !f.starts_with("main_file_cache."));
        let mut actual_files = files_under(&apack);
        actual_files.retain(|f| !f.starts_with("main_file_cache."));
        assert_eq!(actual_files, expected_files, "{label}: data/pack file set");
        for f in &expected_files {
            let (e, a) = (fs::read(gpack.join(f)).unwrap(), fs::read(apack.join(f)).unwrap());
            match f.as_str() {
                "client/versionlist" => assert_versionlist_eq(label, &e, &a, &mut golden, &mut cache),
                ".cache/maps-server.zip" => {
                    let (ez, az) = (ares_gameserver::pack::stored_zip::read(&e).unwrap(), ares_gameserver::pack::stored_zip::read(&a).unwrap());
                    assert!(ez == az, "{label}: maps-server.zip entries");
                }
                _ => assert_bytes_eq(&format!("{label}: {f}"), &e, &a),
            }
        }
        assert_eq!(cache.file_count(0), golden.file_count(0), "{label}: archive 0 entry count");
        for file in 0..golden.file_count(0) {
            let (e, a) = (golden.read(0, file, false), cache.read(0, file, false));
            if file == 5 {
                assert_versionlist_eq(label, &e.unwrap(), &a.unwrap(), &mut golden, &mut cache);
            } else {
                assert_eq!(e.is_some(), a.is_some(), "{label}: cache 0/{file} presence");
                if let (Some(e), Some(a)) = (e, a) {
                    assert_bytes_eq(&format!("{label}: cache 0/{file}"), &e, &a);
                }
            }
        }
        for archive in 1..=4 {
            assert_gzip_archive_eq(label, archive, &mut golden, &mut cache);
        }
    };
    if bless().is_none() {
        cmp();
    } else if quiet(cmp).is_err() {
        // the golden pack becomes this build: data/pack and the content's packs
        for (from, to) in [(apack.clone(), gpack.clone()), (root.join("content/pack"), golden_dir.join("pack"))] {
            fs::remove_dir_all(&to).unwrap();
            copy_dir(&from, &to);
        }
    }
    let _ = fs::remove_dir_all(&root);
}

/// `ares-pack import-npc-csv` / `import-obj-csv` on a copy of the 04-maps maps
/// against `tests/golden/oneoffs/<kind>`.
#[test]
fn import_csv_matches() {
    for (kind, objs) in [("npc", false), ("obj", true)] {
        let dir = std::env::temp_dir().join(format!("ares-import-{kind}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        copy_dir(&root().join("tests/fixtures/04-maps/content/maps"), &dir.join("maps"));
        let mut build = ares_gameserver::util::config::WorldConfig::default().build;
        build.src_dir = dir.to_string_lossy().replace('\\', "/");
        let csv = root().join(format!("tests/oneoffs/{kind}.csv"));
        let left = ares_gameserver::pack::tools::import_csv(&build, csv.to_str().unwrap(), objs).unwrap();
        assert_eq!(left.len(), 1, "{kind}: leftover spawns {left:?}");
        assert_dir_files_eq(&format!("import-{kind}-csv"), &root().join("tests/golden/oneoffs").join(kind), &dir.join("maps"));
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn full_build_matches() {
    for case in cases() {
        let verify = golden_json(&case, "meta.json")["verify"].as_bool().unwrap();
        check_full_build(&case, &root().join("tests/fixtures").join(&case).join("content"), &root().join("tests/golden").join(&case), verify);
    }
}

#[test]
fn util_corpus_matches() {
    use ares_gameserver::engine::xp;
    use ares_gameserver::util::{java_random::JavaRandom, jstring, numbers};
    use serde_json::{json, Value};
    let mut e = corpus("util", "expected.json");
    let mut failures = Vec::new();
    // numbers compare as f64: the golden writes 0.0 as `0`
    let mut check = |what: String, got: Value, want: &mut Value| {
        let same = match (got.as_f64(), want.as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => &got == want,
        };
        if !same {
            failures.push(format!("{what}: got {got}, want {want}"));
            patch(want, got);
        }
    };

    for case in e["javaRandom"].as_array_mut().unwrap() {
        let seed = case["seed"].as_i64().unwrap();
        let mut r = JavaRandom::new(seed);
        for (i, want) in case["seq"].as_array_mut().unwrap().iter_mut().enumerate() {
            let got = match i % 9 {
                0 => json!(r.next_int()),
                1 => json!(r.next_int_bounded(100)),
                2 => json!(r.next_int_bounded(1 << (i % 31))),
                3 => json!(r.next_int_bounded(0x7fffffff)),
                4 => json!(r.next_int_bounded(3 + (i % 50) as i32)),
                5 => json!(r.next_long().to_string()),
                6 => json!(r.next_boolean()),
                7 => json!(r.next_float()),
                _ => json!(r.next_double()),
            };
            check(format!("seed {seed} call {i}"), got, want);
        }
    }

    let t = &mut e["trig"];
    for i in 0..16384 {
        check(format!("sin {i}"), json!(numbers::trig_sin(i)), &mut t["sin"][i as usize]);
        check(format!("cos {i}"), json!(numbers::trig_cos(i)), &mut t["cos"][i as usize]);
    }
    for (x, want) in [0, 1, 4096, 8192, 16383, 16384, -1].iter().zip(t["radians"].as_array_mut().unwrap()) {
        check(format!("radians {x}"), json!(numbers::trig_radians(*x)), want);
    }
    for c in t["atan2"].as_array_mut().unwrap() {
        let (y, x) = (c["y"].as_f64().unwrap(), c["x"].as_f64().unwrap());
        check(format!("atan2 {y} {x}"), json!(numbers::trig_atan2(y, x)), &mut c["v"]);
    }

    let n = &mut e["numbers"];
    for c in n["bitcount"].as_array_mut().unwrap() {
        let v = c[0].as_i64().unwrap() as i32;
        check(format!("bitcount {v}"), json!(numbers::bitcount(v)), &mut c[1]);
    }
    for c in n["ranges"].as_array_mut().unwrap() {
        let (v, a, b) = (c[0].as_i64().unwrap() as i32, c[1].as_u64().unwrap() as usize, c[2].as_u64().unwrap() as usize);
        check(format!("setBitRange {v} {a} {b}"), json!(numbers::set_bit_range(v, a, b)), &mut c[3]);
        check(format!("clearBitRange {v} {a} {b}"), json!(numbers::clear_bit_range(v, a, b)), &mut c[4]);
    }

    for c in e["jstring"].as_array_mut().unwrap() {
        let name = c["n"].as_str().unwrap().to_string();
        let name = name.as_str();
        check(format!("toBase37 {name:?}"), json!(jstring::to_base37(name).to_string()), &mut c["b37"]);
        check(format!("toSafeName {name:?}"), json!(jstring::to_safe_name(name)), &mut c["safe"]);
        check(format!("toDisplayName {name:?}"), json!(jstring::to_display_name(name)), &mut c["display"]);
    }
    for c in e["back"].as_array_mut().unwrap() {
        let v: i64 = c[0].as_str().unwrap().parse().unwrap();
        check(format!("fromBase37 {v}"), json!(jstring::from_base37(v)), &mut c[1]);
    }

    for (t, c) in e["quick"].as_array_mut().unwrap().iter_mut().enumerate() {
        let keys: Vec<i64> = c["keys"].as_array().unwrap().iter().map(|k| k.as_i64().unwrap()).collect();
        let order: Vec<i64> = c["order"].as_array().unwrap().iter().map(|k| k.as_i64().unwrap()).collect();
        // rebuild the input: element i had key keys[pos of i in order]
        let mut input = vec![(0i64, 0i64); order.len()];
        for (pos, &i) in order.iter().enumerate() {
            input[i as usize] = (i, keys[pos]);
        }
        let high = input.len() as isize - 1;
        numbers::quicksort(0, high, &mut input, &mut |a, b| a.1 - b.1);
        let got: Vec<i64> = input.iter().map(|e| e.0).collect();
        if json!(got) != c["order"] && bless().is_some() {
            panic!("util quick: input encoded in expected order; edit by hand");
        }
        check(format!("quicksort case {t}"), json!(got), &mut c["order"]);
    }

    for (i, want) in e["xp"]["byLevel"].as_array_mut().unwrap().iter_mut().enumerate() {
        check(format!("getExpByLevel {}", i + 2), json!(xp::get_exp_by_level(i as i32 + 2)), want);
    }
    for c in e["xp"]["byExp"].as_array_mut().unwrap() {
        let v = c[0].as_i64().unwrap() as i32;
        check(format!("getLevelByExp {v}"), json!(xp::get_level_by_exp(v)), &mut c[1]);
    }
    settle("util", &corpus_path("util", "expected.json"), &e, failures);
}

#[test]
fn isaac_corpus_matches() {
    use ares_gameserver::io::isaac::Isaac;
    let mut failures = Vec::new();
    let mut e = corpus("isaac", "expected.json");
    for case in e.as_array_mut().unwrap() {
        let seed: Vec<i32> = case["seed"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect();
        let mut r = Isaac::new(&seed);
        let got: Vec<i64> = (0..case["out"].as_array().unwrap().len()).map(|_| r.next_int() as i64).collect();
        if let Some(i) = got.iter().zip(case["out"].as_array().unwrap()).position(|(g, w)| *g != w.as_i64().unwrap()) {
            failures.push(format!("seed {seed:?} call {i}: got {}, want {}", got[i], case["out"][i]));
            case["out"] = got.into();
        }
    }
    settle("isaac", &corpus_path("isaac", "expected.json"), &e, failures);
}

#[test]
fn chat_corpus_matches() {
    use ares_gameserver::cache::{word_pack, wordenc::WordEnc};
    use ares_gameserver::io::packet::Packet;
    let mut e = corpus("chat", "expected.json");
    let enc = WordEnc::load(root().join("tests/chat/wordenc")).unwrap();
    let mut failures = Vec::new();
    for c in e["filter"].as_array_mut().unwrap() {
        let got = enc.filter(c[0].as_str().unwrap());
        if got != c[1] {
            failures.push(format!("filter {}: {got:?}, expected {}", c[0], c[1]));
            c[1] = got.into();
        }
    }
    let alphabet = "abcdefghijklmnopqrstuvwxyz'0123456789";
    let mut got_bad = Vec::new();
    for a in alphabet.chars() {
        let mut words = vec![vec![a]];
        for b in alphabet.chars() {
            words.push(vec![a, b]);
            words.extend(alphabet.chars().map(|c| vec![a, b, c]));
        }
        for w in words {
            if enc.is_bad_fragment(&w) {
                got_bad.push(w.iter().collect::<String>());
            }
        }
    }
    let want_bad: Vec<String> = e["badFragments"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    if got_bad != want_bad {
        failures.push(format!("isBadFragment: {} true, expected {}", got_bad.len(), want_bad.len()));
        e["badFragments"] = got_bad.into();
    }
    for c in e["fragmentSelf"].as_array_mut().unwrap() {
        let w: Vec<char> = c[0].as_str().unwrap().chars().collect();
        let got = enc.is_bad_fragment(&w);
        if got != c[1].as_bool().unwrap() {
            failures.push(format!("isBadFragment {}: expected {}", c[0], c[1]));
            c[1] = got.into();
        }
    }
    let bytes = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|b| b.as_u64().unwrap() as u8).collect::<Vec<u8>>();
    for c in e["pack"].as_array_mut().unwrap() {
        let mut p = Packet::with_len(200);
        word_pack::pack(&mut p, c[0].as_str().unwrap());
        if p.data[..p.pos] != bytes(&c[1])[..] {
            failures.push(format!("pack {}", c[0]));
            c[1] = p.data[..p.pos].to_vec().into();
        }
    }
    for c in e["unpack"].as_array_mut().unwrap() {
        let b = bytes(&c[0]);
        let n = b.len();
        let got = word_pack::unpack(&mut Packet::new(b), n);
        if got != c[1].as_str().unwrap() {
            failures.push(format!("unpack {:?}: {got:?}, expected {}", c[0], c[1]));
            c[1] = got.into();
        }
    }
    settle("chat", &corpus_path("chat", "expected.json"), &e, failures);
}

/// `scripts` from decoders.json. Line numbers are probed at -1, 0, the end
/// and around every line-table pc; switch tables compare as maps.
fn check_scripts(label: &str, pack: &std::path::Path, e: &mut serde_json::Value, failures: &mut Vec<String>) {
    use ares_gameserver::script::provider::ScriptProvider;
    use serde_json::{Value, json};
    use std::collections::HashMap;
    let p = ScriptProvider::load(pack).unwrap_or_else(|err| panic!("{label}: {err}"));
    if json!(p.loaded) != e["loaded"] || p.count() != e["scripts"].as_array().unwrap().len() {
        failures.push(format!("{label}: scripts: loaded {} of {}, expected {} of {}", p.loaded, p.count(), e["loaded"], e["scripts"].as_array().unwrap().len()));
        e["loaded"] = json!(p.loaded);
    }
    let mut scripts = Vec::new();
    for id in 0..p.count() {
        let want = e["scripts"].get(id).unwrap_or(&Value::Null);
        let Some(sc) = p.get(id as i32) else {
            if !want.is_null() {
                failures.push(format!("{label}: script {id} missing"));
            }
            scripts.push(Value::Null);
            continue;
        };
        let tables: Option<Vec<HashMap<i32, i32>>> = want["switchTables"].as_array().map(|w| {
            w.iter().map(|t| t.as_array().unwrap().iter().map(|kv| (kv[0].as_str().unwrap().parse().unwrap(), kv[1].as_i64().unwrap() as i32)).collect()).collect()
        });
        let tables = if tables.as_ref() == Some(&sc.switch_tables) {
            want["switchTables"].clone()
        } else {
            json!(sc.switch_tables.iter().map(|t| {
                let mut kv: Vec<_> = t.iter().collect();
                kv.sort();
                kv.into_iter().map(|(k, v)| json!([k.to_string(), v])).collect::<Vec<_>>()
            }).collect::<Vec<_>>())
        };
        let probes = [-1, 0, sc.opcodes.len() as i32].into_iter().chain(sc.info.pcs.iter().flat_map(|&pc| [pc - 1, pc, pc + 1]));
        let line_numbers: Vec<Value> = probes.map(|pc| json!([pc, sc.line_number(pc)])).collect();
        let got = json!({
            "id": sc.id,
            "info": {
                "script_name": sc.info.script_name, "source_file_path": sc.info.source_file_path, "lookup_key": sc.info.lookup_key,
                "parameter_types": sc.info.parameter_types, "pcs": sc.info.pcs, "lines": sc.info.lines,
            },
            "intLocalCount": sc.int_local_count,
            "stringLocalCount": sc.string_local_count,
            "intArgCount": sc.int_arg_count,
            "stringArgCount": sc.string_arg_count,
            "switchTables": tables,
            "opcodes": sc.opcodes,
            "intOperands": sc.opcodes.iter().zip(&sc.int_operands).map(|(&o, &v)| if o == 3 { Value::Null } else { json!(v) }).collect::<Vec<_>>(),
            "stringOperands": sc.string_operands,
            "fileName": sc.file_name(),
            "lineNumbers": line_numbers,
        });
        if &got != want {
            failures.push(format!("{label}: script {id} ({}): {}", sc.name(), first_json_diff(want, &got)));
        }
        scripts.push(got);
    }
    patch(&mut e["scripts"], Value::Array(scripts));
    for kv in e["lookup"].as_array_mut().unwrap() {
        let key = kv[0].as_i64().unwrap() as i32;
        // with type and category -1 the "trigger" is the raw lookup key
        let got = json!(p.get_by_trigger_specific(key, -1, -1).map(|s| s.id));
        if got != kv[1] {
            failures.push(format!("{label}: lookup key {key} should be script {}", kv[1]));
            kv[1] = got;
        }
    }
    for kv in e["names"].as_array_mut().unwrap() {
        let got = json!(p.get_by_name(kv[0].as_str().unwrap()).map(|s| s.id));
        if got != kv[1] {
            failures.push(format!("{label}: name {} should be script {}", kv[0], kv[1]));
            kv[1] = got;
        }
    }
    for q in e["byTrigger"].as_array_mut().unwrap() {
        let [t, ty, cat] = [0, 1, 2].map(|i| q[i].as_i64().unwrap() as i32);
        let a = p.get_by_trigger(t, ty, cat).map(|s| s.id);
        let b = p.get_by_trigger_specific(t, ty, cat).map(|s| s.id);
        if json!([a, b]) != json!([q[3], q[4]]) {
            failures.push(format!("{label}: getByTrigger({t}, {ty}, {cat}): {a:?} {b:?}, expected {} {}", q[3], q[4]));
            (q[3], q[4]) = (json!(a), json!(b));
        }
    }
}

// ---------------------------------------------------------------- routefinder

#[test]
fn routefinder_corpus_matches() {
    use ares_gameserver::routefinder::{CollisionEngine, PathFinder, line, naive, reach, step};
    use serde_json::{Value, json};
    let mut failures = Vec::new();
    let mut e = corpus("routefinder", "cases.json");
    for (m, case) in e.as_array_mut().unwrap().iter_mut().enumerate() {
        let mut f = CollisionEngine::default();
        let mut pf = PathFinder::default();
        for (n, st) in case["steps"].as_array_mut().unwrap().iter_mut().enumerate() {
            let st = st.as_array_mut().unwrap();
            let name = st[1].as_str().unwrap();
            let i = |k: usize| st[k + 2].as_i64().unwrap() as i32;
            let u = |k: usize| st[k + 2].as_u64().unwrap() as u32;
            let b = |k: usize| st[k + 2].as_bool().unwrap();
            if st[0] == "op" {
                match name {
                    "allocateIfAbsent" => f.allocate_if_absent(i(0), i(1), i(2)),
                    "deallocateIfPresent" => f.deallocate_if_present(i(0), i(1), i(2)),
                    "changeFloor" => f.change_floor(i(0), i(1), i(2), b(3)),
                    "changeRoof" => f.change_roof(i(0), i(1), i(2), b(3)),
                    "changeLoc" => f.change_loc(i(0), i(1), i(2), i(3), i(4), b(5), b(6)),
                    "changeWall" => f.change_wall(i(0), i(1), i(2), i(3), i(4), b(5), b(6)),
                    "changeNpc" => f.change_npc(i(0), i(1), i(2), i(3), b(4)),
                    "changeBlock" => f.change_block(i(0), i(1), i(2), i(3), b(4)),
                    "changePlayerOcc" => f.change_player_occ(i(0), i(1), i(2), i(3), b(4)),
                    "__set" => f.set(i(0), i(1), i(2), u(3)),
                    _ => panic!("unknown op {name}"),
                }
                continue;
            }
            let got: Value = match name {
                "findPath" => json!(pf.find_path(&f, i(0), i(1), i(2), i(3), i(4), i(5), i(6), i(7), i(8), i(9), b(10), i(11), i(12), i(13))),
                "findNaivePath" => {
                    let r = st[13].as_f64().unwrap();
                    json!(naive::find_naive_path(&f, i(0), i(1), i(2), i(3), i(4), i(5), i(6), i(7), i(8), u(9), i(10), &mut || r))
                }
                "canTravel" => json!(step::can_travel(&f, i(0), i(1), i(2), i(3), i(4), i(5), u(6), i(7))),
                "hasLineOfSight" => json!(line::has_line_of_sight(&f, i(0), i(1), i(2), i(3), i(4), i(5), i(6), i(7), i(8), u(9))),
                "hasLineOfWalk" => json!(line::has_line_of_walk(&f, i(0), i(1), i(2), i(3), i(4), i(5), i(6), i(7), i(8), u(9))),
                "lineOfSight" => json!(line::line_of_sight(&f, i(0), i(1), i(2), i(3), i(4), i(5), i(6), i(7), i(8), u(9))),
                "lineOfWalk" => json!(line::line_of_walk(&f, i(0), i(1), i(2), i(3), i(4), i(5), i(6), i(7), i(8), u(9))),
                "reached" => json!(reach::reached(&f, i(0), i(1), i(2), i(3), i(4), i(5), i(6), i(7), i(8), i(9), i(10))),
                // a missing mask is `undefined`, which masks to 0
                "isFlagged" => json!(f.is_flagged(i(0), i(1), i(2), if st.len() > 6 { u(3) } else { 0 })),
                "isZoneAllocated" => json!(f.is_zone_allocated(i(0), i(1), i(2))),
                _ => panic!("unknown query {name}"),
            };
            if &got != st.last().unwrap() {
                failures.push(format!("map {m} step {n}: {name}{:?}: {got}, expected {}", &st[2..st.len() - 1], st.last().unwrap()));
                patch(st.last_mut().unwrap(), got);
            }
        }
        let mut zones: Vec<(i32, Vec<u32>)> = f.zones().map(|(k, v)| (k, v.to_vec())).collect();
        zones.sort();
        if json!(zones) != case["zones"] {
            failures.push(format!("map {m}: collision zones differ ({} zones, expected {})", zones.len(), case["zones"].as_array().unwrap().len()));
            patch(&mut case["zones"], json!(zones));
        }
    }
    settle("routefinder", &corpus_path("routefinder", "cases.json"), &e, failures);
}

// ---------------------------------------------------------------- rsbuf

#[test]
fn rsbuf_corpus_matches() {
    use ares_gameserver::rsbuf::{Chat, ExactMove, NpcUpdate, PlayerUpdate, Rsbuf};
    use serde_json::{Value, json};
    use std::io::Read;
    let mut text = String::new();
    flate2::read::GzDecoder::new(fs::File::open(root().join("tests/rsbuf/calls.json.gz")).unwrap()).read_to_string(&mut text).unwrap();
    let mut calls: Vec<Vec<Value>> = serde_json::from_str(&text).unwrap();
    let mut rs = Rsbuf::default();
    let mut failures = Vec::new();
    let mut patches = Vec::new();
    let bytes = |v: &Value| v.as_array().unwrap().iter().map(|b| b.as_u64().unwrap() as u8).collect::<Vec<u8>>();
    for (n, c) in calls.iter().enumerate() {
        let i = |k: usize| c[k + 1].as_i64().unwrap() as i32;
        let b = |k: usize| c[k + 1].as_bool().unwrap();
        let s = |k: usize| c[k + 1].as_str().map(str::to_string);
        let mut check = |failures: &mut Vec<String>, got: Value| {
            if &got != c.last().unwrap() {
                failures.push(format!("call {n} {}({:?}): {got}, expected {}", c[0], &c[1..c.len() - 1], c.last().unwrap()));
                patches.push((n, got));
            }
        };
        match c[0].as_str().unwrap() {
            "addPlayer" => rs.add_player(i(0)),
            "removePlayer" => rs.remove_player(i(0)),
            "addNpc" => rs.add_npc(i(0), i(1)),
            "removeNpc" => rs.remove_npc(i(0)),
            "cleanup" => rs.cleanup(),
            "cleanupPlayerBuildArea" => rs.cleanup_player_build_area(i(0)),
            "computePlayer" => {
                let u = PlayerUpdate {
                    x: i(0),
                    y: i(1),
                    z: i(2),
                    origin_x: i(3),
                    origin_z: i(4),
                    tele: b(6),
                    jump: b(7),
                    run_dir: i(8),
                    walk_dir: i(9),
                    visibility: i(10),
                    active: b(11),
                    masks: c[13].as_u64().unwrap() as u32,
                    appearance: bytes(&c[14]),
                    last_appearance: i(14),
                    face_entity: i(15),
                    face_x: i(16),
                    face_z: i(17),
                    orientation_x: i(18),
                    orientation_z: i(19),
                    damage_taken: i(20),
                    damage_type: i(21),
                    damage_taken2: i(22),
                    damage_type2: i(23),
                    current_hitpoints: i(24),
                    base_hitpoints: i(25),
                    anim_id: i(26),
                    anim_delay: i(27),
                    say: s(28),
                    chat: (!c[30].is_null()).then(|| Chat { bytes: bytes(&c[30]), color: i(30), effect: i(31), ignored: i(32) }),
                    graphic_id: i(33),
                    graphic_height: i(34),
                    graphic_delay: i(35),
                    exact_move: (i(36) != -1).then(|| ExactMove { start_x: i(36), start_z: i(37), end_x: i(38), end_z: i(39), begin: i(40), finish: i(41), dir: i(42) }),
                };
                rs.compute_player(i(5), u);
            }
            "computeNpc" => {
                let u = NpcUpdate {
                    x: i(0),
                    y: i(1),
                    z: i(2),
                    ntype: i(4),
                    tele: b(5),
                    jump: b(6),
                    run_dir: i(7),
                    walk_dir: i(8),
                    active: b(9),
                    masks: c[11].as_u64().unwrap() as u32,
                    face_entity: i(11),
                    face_x: i(12),
                    face_z: i(13),
                    orientation_x: i(14),
                    orientation_z: i(15),
                    damage_taken: i(16),
                    damage_type: i(17),
                    damage_taken2: i(18),
                    damage_type2: i(19),
                    current_hitpoints: i(20),
                    base_hitpoints: i(21),
                    anim_id: i(22),
                    anim_delay: i(23),
                    say: s(24),
                    graphic_id: i(25),
                    graphic_height: i(26),
                    graphic_delay: i(27),
                };
                rs.compute_npc(i(3), u);
            }
            name @ ("playerInfo" | "npcInfo") => {
                let out = if name == "playerInfo" { rs.player_info(i(0), i(1), i(2), i(3), b(4)) } else { rs.npc_info(i(0), i(1), i(2), i(3), b(4)) };
                // whole packets are kept only for these sample pids
                let got = if [1, 2, 7, 40, 61, 200, 333].contains(&i(1)) { json!(out) } else { json!([out.len(), crc32fast::hash(&out)]) };
                check(&mut failures, got);
            }
            "hasPlayer" => check(&mut failures, json!(rs.has_player(i(0), i(1)))),
            "hasNpc" => check(&mut failures, json!(rs.has_npc(i(0), i(1)))),
            "getNpcObservers" => check(&mut failures, json!(rs.npc_observers(i(0)))),
            other => panic!("unknown call {other}"),
        }
        if failures.len() > 20 && bless().is_none() {
            break;
        }
    }
    // rewritten only when something changed: the stored gzip bytes come from another zlib
    match bless() {
        Some(true) if patches.is_empty() => assert_eq!(serde_json::to_string(&calls).unwrap() + "\n", text, "rsbuf: calls.json does not round-trip"),
        Some(_) if !patches.is_empty() => {
            for (n, got) in patches {
                patch(calls[n].last_mut().unwrap(), got);
            }
            let mut out = serde_json::to_vec(&calls).unwrap();
            out.push(b'\n');
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            std::io::Write::write_all(&mut gz, &out).unwrap();
            fs::write(root().join("tests/rsbuf/calls.json.gz"), gz.finish().unwrap()).unwrap();
        }
        Some(_) => {}
        None => report("rsbuf", failures),
    }
}

// ---------------------------------------------------------------- engine

#[test]
fn engine_corpus_matches() {
    use ares_gameserver::cache::config::{ConfigSet, ConfigType, inv::InvType, obj::ObjType};
    use ares_gameserver::engine::inventory::{Inventory, Item};
    use ares_gameserver::engine::zone::ZoneGrid;
    use serde_json::{Value, json};
    let mut e = corpus("engine", "expected.json");
    let mut objs = ConfigSet::<ObjType>::default();
    for o in e["objs"].as_array().unwrap() {
        let mut t = ObjType::new(o["id"].as_u64().unwrap() as usize);
        t.stackable = o["stackable"].as_bool().unwrap();
        objs.configs.push(t);
    }
    let mut invs = ConfigSet::<InvType>::default();
    let u16s = |v: &Value| v.as_array().map(|a| a.iter().map(|x| x.as_u64().unwrap() as u16).collect::<Vec<_>>());
    for (id, i) in e["invs"].as_array().unwrap().iter().enumerate() {
        let mut t = InvType::new(id);
        t.size = i["size"].as_i64().unwrap() as i32;
        t.stackall = i["stackall"].as_bool().unwrap();
        t.stockobj = u16s(&i["stockobj"]);
        t.stockcount = u16s(&i["stockcount"]);
        invs.configs.push(t);
    }
    let snapshot = |c: &Inventory| json!({
        "items": c.items.iter().map(|i| i.as_ref().map(|i| json!([i.id, i.count])).unwrap_or(Value::Null)).collect::<Vec<_>>(),
        "dirty": c.dirty_slots(),
        "update": c.update,
    });
    let mut failures = Vec::new();
    for (n, case) in e["inventory"].as_array_mut().unwrap().iter_mut().enumerate() {
        let inv = case["inv"].as_i64().unwrap() as i32;
        let mut c = if case["fromType"].as_bool().unwrap() {
            Inventory::from_type(&invs, inv)
        } else {
            Inventory::new(inv, case["capacity"].as_u64().unwrap() as usize, case["stackType"].as_i64().unwrap() as i32)
        };
        for (k, op) in case["ops"].as_array_mut().unwrap().iter_mut().enumerate() {
            let op = op.as_array_mut().unwrap();
            let a = |i: usize| op[i + 1].as_i64().unwrap() as i32;
            let (got, rest): (Value, usize) = match op[0].as_str().unwrap() {
                "add" => (json!(c.add(&objs, a(0), a(1) as i64, a(2))), 4),
                "remove" => (json!(c.remove(&invs, a(0), a(1) as i64, a(2))), 4),
                "set" => {
                    let item = op[2].as_array().map(|v| Item::new(v[0].as_i64().unwrap() as i32, v[1].as_i64().unwrap()));
                    c.set(a(0) as usize, item);
                    (Value::Null, 3)
                }
                "delete" => {
                    c.delete(a(0) as usize);
                    (Value::Null, 2)
                }
                "removeAll" => {
                    c.remove_all();
                    (Value::Null, 1)
                }
                "resetTracking" => {
                    c.reset_tracking();
                    (Value::Null, 1)
                }
                "hasAt" => (json!(c.has_at(a(0) as usize, a(1))), 3),
                "query" => {
                    let id = a(0);
                    (json!([c.next_free_slot().map_or(-1, |i| i as i64), c.free_slot_count(), c.item_count(id), c.item_index(id).map_or(-1, |i| i as i64), c.items_filtered().len()]), 2)
                }
                "validSlot" => (json!(c.valid_slot(a(0))), 2),
                "dirty" => (json!(c.dirty_slots()), 1),
                "end" => (snapshot(&c), 1),
                other => panic!("unknown op {other}"),
            };
            if got != op[rest] {
                failures.push(format!("inventory {n} op {k} {:?}: {got}, expected {}", op[0], op[rest]));
                patch(&mut op[rest], got);
            }
            if let Some(snap) = op.get_mut(rest + 1)
                && snapshot(&c) != *snap
            {
                failures.push(format!("inventory {n} op {k}: state {}", first_json_diff(snap, &snapshot(&c))));
                patch(snap, snapshot(&c));
            }
        }
    }
    let mut zg = ZoneGrid::default();
    for (k, op) in e["zoneGrid"].as_array_mut().unwrap().iter_mut().enumerate() {
        let a = |i: usize| op[i + 1].as_i64().unwrap() as i32;
        match op[0].as_str().unwrap() {
            "flag" => zg.flag(a(0), a(1)),
            "unflag" => zg.unflag(a(0), a(1)),
            _ => {
                let got = zg.is_flagged(a(0), a(1), a(2));
                if json!(got) != op[4] {
                    failures.push(format!("zoneGrid op {k} isFlagged({}, {}, {}): {got}", a(0), a(1), a(2)));
                    op[4] = json!(got);
                }
            }
        }
    }
    settle("engine", &corpus_path("engine", "expected.json"), &e, failures);
}

// ---------------------------------------------------------------- server messages

#[test]
fn message_corpus_matches() {
    use ares_gameserver::cache::wordenc::WordEnc;
    use ares_gameserver::engine::inventory::{Inventory, Item};
    use ares_gameserver::io::{isaac::Isaac, packet::Packet};
    use ares_gameserver::net::server::{ServerMessage as M, frame};
    use serde_json::Value;
    let wordenc = WordEnc::load(root().join("tests/chat/wordenc")).unwrap();
    let mut isaac = Isaac::new(&[1, 2, 3, 4]);
    let mut failures = Vec::new();
    let mut e = corpus("messages", "expected.json");
    for (n, c) in e.as_array_mut().unwrap().iter_mut().enumerate() {
        let name = c[0].as_str().unwrap().to_string();
        let a = c[1].as_array().unwrap().clone();
        let i = |k: usize| a[k].as_i64().unwrap() as i32;
        let l = |k: usize| a[k].as_i64().unwrap();
        let b = |k: usize| a[k].as_bool().unwrap();
        let s = |k: usize| a[k].as_str().unwrap().to_string();
        let big = |v: &Value| v.as_str().unwrap().parse::<i64>().unwrap();
        let bytes = |v: &Value| v.as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u8).collect::<Vec<u8>>();
        let inv = |v: &Value| {
            let mut inv = Inventory::new(0, v["capacity"].as_u64().unwrap() as usize, 0);
            inv.items = v["items"].as_array().unwrap().iter().map(|it| it.as_array().map(|p| Item::new(p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap()))).collect();
            inv
        };
        let msg = match name.as_str() {
            "CamLookAt" => M::CamLookAt { x: i(0), z: i(1), height: i(2), rate: i(3), rate2: i(4) },
            "CamMoveTo" => M::CamMoveTo { x: i(0), z: i(1), height: i(2), rate: i(3), rate2: i(4) },
            "CamReset" => M::CamReset,
            "CamShake" => M::CamShake { axis: i(0), random: i(1), amplitude: i(2), rate: i(3) },
            "ChatFilterSettings" => M::ChatFilterSettings { public_chat: i(0), private_chat: i(1), trade_duel: i(2) },
            "FriendlistLoaded" => M::FriendlistLoaded { status: i(0) },
            "HintArrow" => M::HintArrow { kind: i(0), nid: i(1), player_slot: i(2), x: i(3), z: i(4), y: i(5) },
            "IfClose" => M::IfClose,
            "IfOpenChat" => M::IfOpenChat { component: i(0) },
            "IfOpenMain" => M::IfOpenMain { component: i(0) },
            "IfOpenMainSide" => M::IfOpenMainSide { main: i(0), side: i(1) },
            "IfOpenOverlay" => M::IfOpenOverlay { component: i(0) },
            "IfOpenSide" => M::IfOpenSide { component: i(0) },
            "IfSetAnim" => M::IfSetAnim { component: i(0), seq: i(1) },
            "IfSetColour" => M::IfSetColour { component: i(0), colour: i(1) },
            "IfSetHide" => M::IfSetHide { component: i(0), hidden: b(1) },
            "IfSetModel" => M::IfSetModel { component: i(0), model: i(1) },
            "IfSetNpcHead" => M::IfSetNpcHead { component: i(0), npc: i(1) },
            "IfSetObject" => M::IfSetObject { component: i(0), obj: i(1), scale: i(2) },
            "IfSetPlayerHead" => M::IfSetPlayerHead { component: i(0) },
            "IfSetPosition" => M::IfSetPosition { component: i(0), x: i(1), y: i(2) },
            "IfSetScrollPos" => M::IfSetScrollPos { component: i(0), y: i(1) },
            "IfSetTab" => M::IfSetTab { component: i(0), tab: i(1) },
            "IfSetTabActive" => M::IfSetTabActive { tab: i(0) },
            "IfSetText" => M::IfSetText { component: i(0), text: s(1) },
            "LastLoginInfo" => M::LastLoginInfo { last_login_ip: l(0), days_since_login: i(1), days_since_recovery_change: i(2), unread_message_count: i(3), warn_members_in_non_members: b(4) },
            "LocAddChange" => M::LocAddChange { coord: i(0), loc: i(1), shape: i(2), angle: i(3) },
            "LocAnim" => M::LocAnim { coord: i(0), shape: i(1), angle: i(2), seq: i(3) },
            "LocDel" => M::LocDel { coord: i(0), shape: i(1), angle: i(2) },
            "LocMerge" => M::LocMerge { src_x: i(0), src_z: i(1), shape: i(2), angle: i(3), loc_id: i(4), start_cycle: i(5), end_cycle: i(6), player_slot: i(7), east: i(8), south: i(9), west: i(10), north: i(11) },
            "Logout" => M::Logout,
            "MapAnim" => M::MapAnim { coord: i(0), spotanim: i(1), height: i(2), delay: i(3) },
            "MapProjAnim" => M::MapProjAnim { src_x: i(0), src_z: i(1), dst_x: i(2), dst_z: i(3), target: i(4), spotanim: i(5), src_height: i(6), dst_height: i(7), start_delay: i(8), end_delay: i(9), peak: i(10), arc: i(11) },
            "MessageGame" => M::MessageGame { msg: s(0) },
            "MessagePrivate" => M::MessagePrivate { from: big(&a[0]), message_id: i(1), staff_mod_level: i(2), msg: s(3) },
            "MidiJingle" => M::MidiJingle { id: i(0), delay: i(1) },
            "MidiSong" => M::MidiSong { id: i(0) },
            "MinimapToggle" => M::MinimapToggle { minimap_type: i(0) },
            "NpcInfo" => M::NpcInfo { bytes: bytes(&a[0]) },
            "ObjAdd" => M::ObjAdd { coord: i(0), obj: i(1), count: l(2) },
            "ObjCount" => M::ObjCount { coord: i(0), obj: i(1), old_count: l(2), new_count: l(3) },
            "ObjDel" => M::ObjDel { coord: i(0), obj: i(1) },
            "ObjReveal" => M::ObjReveal { coord: i(0), obj: i(1), count: l(2), receiver_id: i(3) },
            "PCountDialog" => M::PCountDialog,
            "PlayerInfo" => M::PlayerInfo { bytes: bytes(&a[0]) },
            "RebuildNormal" => M::RebuildNormal { zone_x: i(0), zone_z: i(1) },
            "ResetAnims" => M::ResetAnims,
            "ResetClientVarCache" => M::ResetClientVarCache,
            "SetMultiway" => M::SetMultiway { hidden: b(0) },
            "SetPlayerOp" => M::SetPlayerOp { op: i(0), value: s(1), primary: i(2) },
            "SynthSound" => M::SynthSound { synth: i(0), loops: i(1), delay: i(2) },
            "TutFlash" => M::TutFlash { tab: i(0) },
            "TutOpen" => M::TutOpen { component: i(0) },
            "UnsetMapFlag" => M::UnsetMapFlag,
            "UpdateFriendList" => M::UpdateFriendList { name: big(&a[0]), node_id: i(1) },
            "UpdateIgnoreList" => M::UpdateIgnoreList { names: a[0].as_array().unwrap().iter().map(big).collect() },
            "UpdateInvFull" => M::inv_full(i(0), &inv(&a[1]), i(2)),
            "UpdateInvPartial" => {
                let mut v = inv(&a[1]);
                for d in a[1]["dirty"].as_array().unwrap() {
                    let slot = d.as_u64().unwrap() as usize;
                    let item = v.get(slot);
                    v.set(slot, item);
                }
                M::inv_partial(i(0), &v)
            }
            "UpdateInvStopTransmit" => M::UpdateInvStopTransmit { component: i(0) },
            "UpdatePid" => M::UpdatePid { uid: i(0), members: b(1) },
            "UpdateRebootTimer" => M::UpdateRebootTimer { ticks: i(0) },
            "UpdateRunEnergy" => M::UpdateRunEnergy { energy: i(0) },
            "UpdateRunWeight" => M::UpdateRunWeight { kg: i(0) },
            "UpdateStat" => M::UpdateStat { stat: i(0), exp: i(1), level: i(2) },
            "UpdateZoneFullFollows" => M::UpdateZoneFullFollows { zone_x: i(0), zone_z: i(1), origin_x: i(2), origin_z: i(3) },
            "UpdateZonePartialEnclosed" => M::UpdateZonePartialEnclosed { zone_x: i(0), zone_z: i(1), origin_x: i(2), origin_z: i(3), data: bytes(&a[4]) },
            "UpdateZonePartialFollows" => M::UpdateZonePartialFollows { zone_x: i(0), zone_z: i(1), origin_x: i(2), origin_z: i(3) },
            "VarpLarge" => M::VarpLarge { varp: i(0), value: i(1) },
            "VarpSmall" => M::VarpSmall { varp: i(0), value: i(1) },
            other => panic!("unknown model {other}"),
        };
        let mut buf = Packet::with_len(5000);
        let framed = frame(&msg, Some(&mut isaac), &wordenc, &mut buf);
        if framed != bytes(&c[2]) {
            failures.push(format!("message {n} {name}{a:?}: framed {framed:?}, expected {}", c[2]));
            c[2] = framed.into();
        }
        let enclosed = msg.is_zone().then(|| {
            let mut z = Packet::with_len(5000);
            msg.enclose(&mut z, &wordenc);
            z.data[..z.pos].to_vec()
        });
        if serde_json::json!(enclosed) != c[3] {
            failures.push(format!("message {n} {name}: enclosed {enclosed:?}, expected {}", c[3]));
            c[3] = serde_json::json!(enclosed);
        }
    }
    settle("messages", &corpus_path("messages", "expected.json"), &e, failures);
}

#[test]
fn client_corpus_matches() {
    use ares_gameserver::io::packet::Packet;
    use ares_gameserver::net::client::decode;
    let mut in_buf = Packet::with_len(5000);
    let mut failures = Vec::new();
    let mut e = corpus("client", "expected.json");
    for (n, c) in e.as_array_mut().unwrap().iter_mut().enumerate() {
        let id = c[0].as_u64().unwrap() as u8;
        let bytes: Vec<u8> = c[1].as_array().unwrap().iter().map(|b| b.as_u64().unwrap() as u8).collect();
        let len = c[2].as_u64().unwrap() as usize;
        in_buf.data[..bytes.len()].copy_from_slice(&bytes);
        in_buf.pos = 0;
        let msg = decode(id, &mut in_buf, len);
        let got = match &msg {
            None => serde_json::json!([null, null, null]),
            Some(m) => {
                let name = format!("{m:?}");
                let name = name.split([' ', '{']).next().unwrap().to_string();
                serde_json::json!([name, m, m.category().0])
            }
        };
        let want = serde_json::json!([c[3], c[4], c[5]]);
        if got != want {
            failures.push(format!("packet {n} opcode {id} len {len}: {got}, expected {want}"));
            for (k, v) in got.as_array().unwrap().iter().enumerate() {
                patch(&mut c[3 + k], v.clone());
            }
        }
    }
    settle("client", &corpus_path("client", "expected.json"), &e, failures);
}

#[test]
fn linklist_corpus_matches() {
    use ares_gameserver::engine::linklist::{HashTable, Links, NodeId};
    use serde_json::{Value, json};
    let mut links = Links::<usize>::default();
    let lists: Vec<_> = (0..3).map(|_| links.list()).collect();
    let mut table = HashTable::new(&mut links, 8);
    let nodes: Vec<NodeId> = (0..24).map(|i| links.node(i)).collect();
    let id = |links: &Links<usize>, n: Option<NodeId>| n.map(|n| *links.get(n).unwrap());
    let mut failures = Vec::new();
    // one step; returns its result to compare (if any)
    fn apply(links: &mut Links<usize>, lists: &[ares_gameserver::engine::linklist::List], table: &mut HashTable, nodes: &[NodeId], st: &[Value]) -> Option<Value> {
        let a = |k: usize| st[k + 1].as_u64().unwrap() as usize;
        let id = |links: &Links<usize>, n: Option<NodeId>| n.map(|n| *links.get(n).unwrap());
        match st[0].as_str().unwrap() {
            "addTail" => links.add_tail(lists[a(0)], nodes[a(1)]),
            "addHead" => links.add_head(lists[a(0)], nodes[a(1)]),
            "unlink" => links.unlink(nodes[a(0)]),
            "clear" => links.clear(lists[a(0)]),
            "removeHead" => {
                let head = links.remove_head(lists[a(0)]);
                return Some(json!(id(links, head)));
            }
            "contents" => return Some(json!(links.collect(lists[a(0)]).into_iter().map(|n| *links.get(n).unwrap()).collect::<Vec<_>>())),
            "hadd" => table.add(links, a(0) as i128, nodes[a(1)]),
            "hfind" => return Some(json!(id(links, table.find(links, a(0) as i128)))),
            other => panic!("unknown step {other}"),
        }
        None
    }
    let mut e = corpus("linklist", "expected.json");
    for (n, slot) in e.as_array_mut().unwrap().iter_mut().enumerate() {
        let mut out = slot.clone();
        let st = slot.as_array().unwrap();
        match st[0].as_str().unwrap() {
            "iter" | "hiter" => {
                let hash = st[0] == "hiter";
                let (ai, wi) = if hash { (1, 2) } else { (3, 4) };
                let (actions, want) = (&st[ai], &st[wi]);
                let actions = actions.as_array().unwrap();
                let mut yielded = Vec::new();
                if hash {
                    let mut c = table.iter();
                    while let Some(node) = table.next(&links, &mut c) {
                        yielded.push(links.get(node).copied());
                        let Some(acts) = actions.get(yielded.len() - 1) else { break };
                        for act in acts.as_array().unwrap() {
                            apply(&mut links, &lists, &mut table, &nodes, act.as_array().unwrap());
                        }
                        if yielded.len() > 60 {
                            break;
                        }
                    }
                } else {
                    let mut c = links.iter(lists[st[1].as_u64().unwrap() as usize], st[2].as_bool().unwrap());
                    while let Some(node) = links.next(&mut c) {
                        // a cached next node moved to another list walks into that
                        // list's sentinel, which is yielded too (id null)
                        yielded.push(links.get(node).copied());
                        let g = yielded.len() - 1;
                        let Some(acts) = actions.get(g) else { break };
                        for (j, act) in acts.as_array().unwrap().iter().enumerate() {
                            let act = act.as_array().unwrap();
                            if let Some(got) = apply(&mut links, &lists, &mut table, &nodes, act)
                                && got != *act.last().unwrap()
                            {
                                failures.push(format!("step {n} nested {act:?}: {got}"));
                                *out[ai][g][j].as_array_mut().unwrap().last_mut().unwrap() = got;
                            }
                        }
                        if yielded.len() > 60 {
                            break;
                        }
                    }
                }
                if json!(yielded) != *want {
                    failures.push(format!("step {n} {}: yielded {yielded:?}, expected {want}", st[0]));
                    out[wi] = json!(yielded);
                }
            }
            _ => {
                if let Some(got) = apply(&mut links, &lists, &mut table, &nodes, st)
                    && got != *st.last().unwrap()
                {
                    failures.push(format!("step {n} {st:?}: {got}"));
                    *out.as_array_mut().unwrap().last_mut().unwrap() = got;
                }
            }
        }
        *slot = out;
        if failures.len() > 10 && bless().is_none() {
            break;
        }
    }
    let _ = id;
    settle("linklist", &corpus_path("linklist", "expected.json"), &e, failures);
}

/// `gamemap.json`: the map loaded into a World.
fn check_gamemap(label: &str, golden_dir: &std::path::Path, src_dir: &std::path::Path) {
    use ares_gameserver::engine::{configs::Configs, game_map, world::World};
    use serde_json::json;
    let pack = golden_dir.join("data/pack");
    let mut w = World::new(true, Configs::load(&pack, true).unwrap());
    game_map::init(&mut w, src_dir, &pack).unwrap_or_else(|e| panic!("{label}: {e}"));
    let mut expected = golden_json_at(golden_dir, "gamemap.json");
    let sorted = |s: &std::collections::HashSet<i32>| {
        let mut v: Vec<i32> = s.iter().copied().collect();
        v.sort();
        v
    };
    let a = &w.arenas;
    let zones: Vec<serde_json::Value> = w
        .zones
        .zones
        .values()
        .map(|z| {
            let locs: Vec<_> = a.locs.collect(z.locs).into_iter().map(|id| {
                let l = a.locs.get(id).unwrap();
                json!([l.kind(), l.shape(), l.angle(), l.layer(), l.e.x, l.e.z, l.e.level, l.e.width, l.e.length, l.e.is_active])
            }).collect();
            let objs: Vec<_> = a.objs.collect(z.objs).into_iter().map(|id| {
                let o = a.objs.get(id).unwrap();
                json!([o.kind, o.count, o.e.x, o.e.z, o.e.level, o.e.is_active])
            }).collect();
            let npcs: Vec<_> = a.members.collect(z.npcs).into_iter().map(|id| match a.members.get(id).unwrap() {
                ares_gameserver::engine::zone::Member::Npc(n) => *n,
                ares_gameserver::engine::zone::Member::Player(p) => -p,
            }).collect();
            json!([z.index, locs, objs, npcs])
        })
        .collect();
    let npcs: Vec<_> = w.npcs.iter().map(|n| json!([n.nid, n.kind, n.e.x, n.e.z, n.e.level, n.e.width, n.block_walk()])).collect();
    let mut collision: Vec<(i32, u32)> = w
        .collision
        .zones()
        .map(|(k, flags)| (k, crc32fast::hash(&flags.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>())))
        .collect();
    collision.sort();
    let mut failures = Vec::new();
    for (what, got) in [
        ("multimap", json!(sorted(&w.map.multimap))),
        ("freemap", json!(sorted(&w.map.freemap))),
        ("npcs", json!(npcs)),
        ("collision", json!(collision)),
        ("zones", json!(zones)),
    ] {
        if got != expected[what] {
            failures.push(format!("{label}: {what}: {}", first_json_diff(&expected[what], &got)));
            patch(&mut expected[what], got);
        }
    }
    settle("gamemap", &golden_dir.join("gamemap.json"), &expected, failures);
}

#[test]
fn gamemap_matches() {
    for case in cases() {
        check_gamemap(&case, &root().join("tests/golden").join(&case), &root().join("tests/fixtures").join(&case).join("content"));
    }
}

#[test]
fn zone_events_match() {
    use ares_gameserver::cache::wordenc::WordEnc;
    use ares_gameserver::engine::entity::{Loc, Obj, lifecycle};
    use ares_gameserver::engine::{configs::Configs, game_map, world::World};
    use ares_gameserver::io::packet::Packet;
    use serde_json::{Value, json};
    let case = "04-maps";
    let golden_dir = root().join("tests/golden").join(case);
    let pack = golden_dir.join("data/pack");
    let mut w = World::new(true, Configs::load(&pack, true).unwrap());
    game_map::init(&mut w, &root().join("tests/fixtures").join(case).join("content"), &pack).unwrap();
    let (mut locs, mut objs) = (Vec::new(), Vec::new());
    for z in w.zones.zones.values() {
        locs.extend(w.arenas.locs.collect(z.locs));
        objs.extend(w.arenas.objs.collect(z.objs));
    }
    let mut e = corpus("zones", &format!("{case}.json"));
    let wordenc = WordEnc::default();
    let mut failures = Vec::new();
    for (t, tick) in e["ticks"].as_array_mut().unwrap().iter_mut().enumerate() {
        w.current_tick = t as i32 + 1;
        for op in tick["ops"].as_array_mut().unwrap() {
            let op = op.as_array_mut().unwrap();
            let i = |k: usize| op[k + 1].as_i64().unwrap() as i32;
            let big = |k: usize| op[k + 1].as_str().unwrap().parse::<i64>().unwrap();
            match op[0].as_str().unwrap() {
                "addLoc" => {
                    let kind = i(4);
                    let (width, length) = w.configs.locs.get(kind as usize).map(|t| (t.width, t.length)).unwrap();
                    let id = w.new_loc(Loc::new(i(1), i(2), i(3), width, length, lifecycle::DESPAWN, kind, i(5), i(6)));
                    assert_eq!(locs.len() as i32, i(0));
                    locs.push(id);
                    w.add_loc(id, i(7));
                }
                "changeLoc" => w.change_loc(locs[i(0) as usize], i(1), i(2), i(3), i(4)),
                "removeLoc" => w.remove_loc(locs[i(0) as usize], i(1)),
                "animLoc" => w.anim_loc(locs[i(0) as usize], i(1)),
                "addObj" => {
                    let id = w.new_obj(Obj::new(i(1), i(2), i(3), lifecycle::DESPAWN, i(4), op[6].as_i64().unwrap()));
                    objs.push(id);
                    w.add_obj(id, big(6), i(7));
                }
                "changeObj" => w.change_obj(objs[i(0) as usize], op[2].as_i64().unwrap()),
                "removeObj" => w.remove_obj(objs[i(0) as usize], i(1)),
                "revealObj" => w.reveal_obj(objs[i(0) as usize]),
                "animMap" => w.anim_map(i(0), i(1), i(2), i(3), i(4), i(5)),
                "mapProjAnim" => w.map_proj_anim(i(0), i(1), i(2), i(3), i(4), i(5), i(6), i(7), i(8), i(9), i(10), i(11), i(12)),
                "getLoc" => {
                    let got = w.get_loc(i(0), i(1), i(2), i(3)).map(|id| locs.iter().position(|&l| l == id).unwrap());
                    if json!(got) != op[5] {
                        failures.push(format!("tick {} getLoc {op:?}: {got:?}", t + 1));
                        op[5] = json!(got);
                    }
                }
                "getObj" => {
                    let got = w.get_obj(i(0), i(1), i(2), i(3), big(4)).map(|id| objs.iter().position(|&o| o == id).unwrap());
                    if json!(got) != op[6] {
                        failures.push(format!("tick {} getObj {op:?}: {got:?}", t + 1));
                        op[6] = json!(got);
                    }
                }
                other => panic!("unknown op {other}"),
            }
        }
        w.process_zones();
        let zones: Vec<Value> = w
            .zones_tracking
            .iter()
            .map(|&z| {
                let q = w.zone_events.get(&z);
                let events: Vec<Value> = q.map_or(vec![], |q| {
                    q.events.values().map(|ev| {
                        let mut b = Packet::with_len(5000);
                        ev.message.enclose(&mut b, &wordenc);
                        json!([ev.enclosed, ev.receiver64.to_string(), b.data[..b.pos].to_vec()])
                    }).collect()
                });
                json!([z, events, q.and_then(|q| q.shared.clone())])
            })
            .collect();
        if json!(zones) != tick["zones"] {
            failures.push(format!("tick {} zones: {}", t + 1, first_json_diff(&tick["zones"], &json!(zones))));
            patch(&mut tick["zones"], json!(zones));
        }
        let mut entries: Vec<_> = w.collision.zones().collect();
        entries.sort_by_key(|e| e.0);
        let flat: Vec<u8> = entries.iter().flat_map(|(k, v)| std::iter::once(*k as u32).chain(v.iter().copied())).flat_map(|v| v.to_le_bytes()).collect();
        if json!(crc32fast::hash(&flat)) != tick["collision"] {
            failures.push(format!("tick {} collision differs", t + 1));
            tick["collision"] = json!(crc32fast::hash(&flat));
        }
        w.cleanup_zones();
        if failures.len() > 10 && bless().is_none() {
            break;
        }
    }
    let final_locs: Vec<Value> = locs.iter().map(|&id| {
        let l = w.loc(id);
        json!([l.e.is_active, l.e.lifecycle_tick, l.e.last_lifecycle_tick, l.kind(), l.shape(), l.angle()])
    }).collect();
    let final_objs: Vec<Value> = objs.iter().map(|&id| {
        let o = w.obj(id);
        json!([o.e.is_active, o.e.lifecycle_tick, o.e.last_lifecycle_tick, o.count, o.receiver64.to_string(), o.reveal])
    }).collect();
    if json!(final_locs) != e["final"]["locs"] {
        failures.push(format!("final locs: {}", first_json_diff(&e["final"]["locs"], &json!(final_locs))));
        patch(&mut e["final"]["locs"], json!(final_locs));
    }
    if json!(final_objs) != e["final"]["objs"] {
        failures.push(format!("final objs: {}", first_json_diff(&e["final"]["objs"], &json!(final_objs))));
        patch(&mut e["final"]["objs"], json!(final_objs));
    }
    settle("zone events", &corpus_path("zones", &format!("{case}.json")), &e, failures);
}

// ---------------------------------------------------------------- vm, npcsim, save, replay

#[test]
fn vm_corpus_matches() {
    use ares_gameserver::engine::{configs::Configs, world::World};
    use ares_gameserver::script::{provider::ScriptProvider, runner, state::{Arg, ScriptState}};
    use ares_gameserver::util::java_random::JavaRandom;
    use serde_json::{Value, json};
    let pack = root().join("tests/golden/05-vm/data/pack");
    let mut w = World::new(true, Configs::load(&pack, true).unwrap());
    w.scripts = ScriptProvider::load(&pack).unwrap();
    let mut failures = Vec::new();
    let mut e = corpus("vm", "expected.json");
    for (n, c) in e.as_array_mut().unwrap().iter_mut().enumerate() {
        let name = c[0].as_str().unwrap().to_string();
        let args: Vec<Arg> = c[1].as_array().unwrap().iter().map(|a| match a {
            Value::String(s) => Arg::Str(s.clone()),
            v => Arg::Int(v.as_i64().unwrap() as i32),
        }).collect();
        let script = w.scripts.get_by_name(&format!("[proc,{name}]")).unwrap();
        w.random = JavaRandom::new(42);
        w.last_script_error = None;
        let mut s = ScriptState::new(script, &args);
        let execution = runner::execute(&mut w, &mut s);
        let got = json!([execution, s.int_stack, s.string_stack, s.int_stack.len(), s.string_stack.len(), w.last_script_error]);
        let want = json!([c[2], c[3], c[4], c[5], c[6], c[7]]);
        if got != want {
            failures.push(format!("run {n} {name}{:?}: {got}, expected {want}", c[1]));
            for (k, v) in got.as_array().unwrap().iter().enumerate() {
                patch(&mut c[2 + k], v.clone());
            }
        }
    }
    // compiled scripts never pop an empty stack; a hand-built one is a script error
    use ares_shared::opcode::op;
    for (opcode, want) in [(op::POP_INT_DISCARD, "pop_int_discard int stack underflow"), (op::POP_STRING_DISCARD, "pop_string_discard string stack underflow")] {
        let mut f = (*w.scripts.get_by_name("[proc,vm_split]").unwrap()).clone();
        (f.opcodes, f.int_operands, f.string_operands) = (vec![opcode, op::RETURN], vec![0, 0], vec![None, None]);
        let mut s = ScriptState::new(std::sync::Arc::new(f), &[]);
        w.last_script_error = None;
        let execution = runner::execute(&mut w, &mut s);
        if w.last_script_error.as_deref() != Some(want) {
            failures.push(format!("underflow: {execution:?} {:?}, expected {want}", w.last_script_error));
        }
    }
    settle("vm", &corpus_path("vm", "expected.json"), &e, failures);
}

/// An NPC script that suspends on its player (`p_delay`) is parked on that player.
#[test]
fn npc_script_parks_on_player() {
    use ares_gameserver::engine::{configs::Configs, entity::player::Player, world::World};
    use ares_gameserver::script::{PointerType, provider::ScriptProvider, state::ScriptState};
    use ares_shared::opcode::op;
    let pack = root().join("tests/golden/05-vm/data/pack");
    let mut w = World::new(true, Configs::load(&pack, true).unwrap());
    w.scripts = ScriptProvider::load(&pack).unwrap();
    w.players[1] = Some(Box::new(Player::new("a".into(), 1, 1, &w.configs.varps, false)));
    let mut f = (*w.scripts.get_by_name("[proc,vm_split]").unwrap()).clone();
    (f.opcodes, f.int_operands, f.string_operands) = (vec![op::PUSH_CONSTANT_INT, op::P_DELAY, op::RETURN], vec![0, 0, 0], vec![None, None, None]);
    let mut s = Box::new(ScriptState::new(std::sync::Arc::new(f), &[]));
    s.set_active_player(1);
    s.pointer_add(PointerType::PActivePlayer);
    w.execute_npc_script(s).unwrap();
    assert!(w.player(1).unwrap().active_script.is_some(), "{:?}", w.last_script_error);
}

/// `npcsim.json`: an NPC-only World run tick by
/// tick. Stops at the first tick that differs (under `BLESS`, patches it and runs on).
fn check_npcsim(label: &str, golden_dir: &std::path::Path, src_dir: &std::path::Path, path: &std::path::Path) {
    use ares_gameserver::engine::{configs::Configs, game_map, world::{Clock, World}};
    use ares_gameserver::script::provider::ScriptProvider;
    use ares_gameserver::util::java_random::JavaRandom;
    use serde_json::{Value, json};
    let mut expected: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let seed = expected["seed"].as_i64().unwrap();
    let pack = golden_dir.join("data/pack");
    let mut w = World::new(true, Configs::load(&pack, true).unwrap());
    w.scripts = ScriptProvider::load(&pack).unwrap();
    w.clock = Clock::Fixed(1700000000000);
    w.math_seed(seed as i32);
    w.random = JavaRandom::new(seed);
    game_map::init(&mut w, src_dir, &pack).unwrap_or_else(|e| panic!("{label}: {e}"));
    let row = |w: &World, nid: i32| {
        let n = w.npc(nid).unwrap();
        let p = &n.p;
        let vars: Vec<Value> = n.vars.iter().enumerate().filter(|(_, v)| **v != 0).map(|(i, v)| json!([i, v])).collect();
        json!([
            n.nid, n.kind, n.e.x, n.e.z, n.e.level, n.e.is_active as i32, p.masks, p.walk_dir, p.run_dir, p.target_op,
            p.face_angle_x, p.face_angle_z, p.face_square_x, p.face_square_z, p.face_entity, p.anim_id, p.anim_delay,
            p.say_message, p.spotanim_id, p.spotanim_height, p.spotanim_time, p.hitmark_damage, p.hitmark_type,
            p.hitmark2_damage, p.hitmark2_type, n.levels, p.delayed as i32, p.delayed_until, n.hunt_mode, n.huntrange,
            p.tele as i32, p.jump as i32, p.waypoint_index, n.timer_interval, p.walktrigger, p.last_movement,
            n.active_script.is_some() as i32, vars
        ])
    };
    let fast = |w: &World, nid: i32| {
        let n = w.npc(nid).unwrap();
        let queue: Vec<Value> = n.queue_links.collect(n.queue).into_iter().map(|id| {
            let r = n.queue_links.get(id).unwrap();
            json!([r.queue_id, r.delay, r.last_int])
        }).collect();
        json!([n.e.lifecycle_tick, n.stuck_counter, n.timer_clock, n.regen_clock, n.hunt_clock, queue])
    };
    let mut last: std::collections::HashMap<i32, Value> = std::collections::HashMap::new();
    let mut failures = Vec::new();
    for t in 0..expected["ticks"].as_array().unwrap().len() {
        let want = &expected["ticks"][t];
        w.script_errors.clear();
        w.process_world();
        w.process_npc_event_queue();
        w.process_npcs();
        w.process_zones();
        let mut rows = Vec::new();
        let mut all = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for nid in 0..w.npcs.capacity() as i32 {
            if w.npcs.get(nid as usize).is_none() {
                continue;
            }
            seen.insert(nid);
            all.push(fast(&w, nid));
            let r = row(&w, nid);
            if last.get(&nid) != Some(&r) {
                last.insert(nid, r.clone());
                rows.push(r);
            }
        }
        let mut gone: Vec<i32> = last.keys().copied().filter(|k| !seen.contains(k)).collect();
        gone.sort();
        for k in &gone {
            last.remove(k);
        }
        let zones: Vec<Value> = w.zones_tracking.iter().map(|z| {
            let shared = w.zone_events.get(z).and_then(|e| e.shared.as_ref()).map(|b| b.iter().map(|x| format!("{x:02x}")).collect::<String>());
            json!([z, shared])
        }).collect();
        let digest = crc32fast::hash(serde_json::to_string(&all).unwrap().as_bytes());
        let mut want_gone: Vec<i64> = want["gone"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        want_gone.sort();
        let got = json!({ "npcs": rows, "digest": digest, "gone": gone, "zones": zones, "errors": w.script_errors });
        let want = json!({ "npcs": want["npcs"], "digest": want["digest"], "gone": want_gone, "zones": want["zones"], "errors": want["errors"] });
        if got != want {
            let keys: Vec<&str> = ["npcs", "digest", "gone", "zones", "errors"].into_iter().filter(|k| got[*k] != want[*k]).collect();
            if bless().is_some() {
                failures.push(format!("tick {t}"));
                for k in keys {
                    patch(&mut expected["ticks"][t][k], got[k].clone());
                }
            } else {
                let detail = keys.iter().map(|k| format!("{k}: {}", first_json_diff(&want[*k], &got[*k]))).collect::<Vec<_>>().join("\n");
                let rows = |v: &Value| v["npcs"].as_array().cloned().unwrap_or_default();
                let (wr, gr) = (rows(&want), rows(&got));
                let first = wr.iter().zip(gr.iter()).find(|(a, b)| a != b).map(|(a, b)| format!("\nwant {a}\ngot  {b}")).unwrap_or_default();
                panic!("{label}: tick {t} differs\n{detail}{first}");
            }
        }
        w.cleanup_zones();
        w.cleanup_npcs();
        w.current_tick += 1;
    }
    settle("npcsim", path, &expected, failures);
}

#[test]
fn npcsim_matches() {
    let case = "06-npcs";
    check_npcsim(case, &root().join("tests/golden").join(case), &root().join("tests/fixtures").join(case).join("content"), &root().join("tests/npcsim/06-npcs.json"));
}

/// `tests/save/expected.json`: `Player::load` and `Player::save` on random
/// and corrupt saves.
#[test]
fn save_corpus_matches() {
    use ares_gameserver::engine::{configs::Configs, entity::Player};
    use serde_json::{Value, json};
    let pack = root().join("tests/golden/06-npcs/data/pack");
    let c = Configs::load(&pack, true).unwrap();
    let mut failures = Vec::new();
    let mut e = corpus("save", "expected.json");
    for (n, case) in e.as_array_mut().unwrap().iter_mut().enumerate() {
        let name = case["name"].as_str().unwrap().to_string();
        let name = name.as_str();
        let got = match Player::load(name, &unhex(case["save"].as_str().unwrap()), &c.varps, &c.invs, &c.varos, false) {
            Err(e) => json!({ "error": e }),
            Ok(p) => {
                let invs: Vec<Value> = p.invs.iter().map(|(t, inv)| {
                    let items: Vec<Value> = (0..inv.items.len()).filter_map(|s| inv.get(s).map(|o| json!([s, o.id, o.count]))).collect();
                    json!([t, inv.capacity, items])
                }).collect();
                let vars: Vec<Value> = p.vars.iter().enumerate().filter(|(_, v)| **v != 0).map(|(i, v)| json!([i, v])).collect();
                json!({ "fields": {
                    "x": p.e.x, "z": p.e.z, "level": p.e.level, "body": p.body, "colors": p.colors, "gender": p.gender,
                    "runenergy": p.runenergy, "playtime": p.playtime, "stats": p.stats, "levels": p.levels, "baseLevels": p.base_levels,
                    "vars": vars, "invs": invs, "afkZones": p.afk_zones, "lastAfkZone": p.last_afk_zone,
                    "chat": [p.public_chat, p.private_chat, p.trade_duel], "lastLoginTime": p.last_login_time.to_string(),
                    "combatLevel": p.combat_level, "username": p.username, "username37": p.username37.to_string()
                }, "resave": hex(&p.save(&c.varps, &c.invs, &c.varos)) })
            }
        };
        let want = match case.get("error") {
            Some(e) => json!({ "error": e }),
            None => json!({ "fields": case["fields"], "resave": case["resave"] }),
        };
        if got != want {
            failures.push(format!("case {n} {name}: {}", first_json_diff(&want, &got)));
            let mut new = json!({ "name": case["name"], "save": case["save"] });
            new.as_object_mut().unwrap().extend(got.as_object().unwrap().clone());
            patch(case, new);
        }
    }
    settle("save", &corpus_path("save", "expected.json"), &e, failures);
}

/// `tests/replay/<name>.json` through the World; every send, close and save
/// must match `<name>.expected.json` tick by tick.
fn check_replay(name: &str) {
    check_replay_at(name, &root().join("tests/replay").join(format!("{name}.expected.json")));
}

fn check_replay_at(name: &str, expected_path: &std::path::Path) {
    use ares_gameserver::cache::crc_table::CrcTable;
    use ares_gameserver::engine::{configs::Configs, game_map, world::{Clock, Outgoing, World}};
    use ares_gameserver::io::{filestream::FileStream, isaac::Isaac, packet::Packet};
    use ares_gameserver::link::login::{self, LoginRequest};
    use ares_gameserver::script::provider::ScriptProvider;
    use ares_gameserver::util::java_random::JavaRandom;
    use serde_json::{Value, json};
    let scenario: Value = serde_json::from_str(&fs::read_to_string(root().join("tests/replay").join(format!("{name}.json"))).unwrap()).unwrap();
    let mut expected: Value = serde_json::from_str(&fs::read_to_string(expected_path).unwrap()).unwrap();
    let seed = scenario["seed"].as_i64().unwrap();
    let golden = scenario["golden"].as_str().unwrap();
    let pack = root().join("tests/golden").join(golden).join("data/pack");
    let mut w = World::new(true, Configs::load(&pack, true).unwrap());
    w.scripts = ScriptProvider::load(&pack).unwrap();
    w.crc32 = CrcTable::new(&mut FileStream::open(&pack, false).unwrap()).crc32;
    let key = test_key();
    game_map::init(&mut w, &root().join(scenario["src"].as_str().unwrap()), &pack).unwrap();
    let dir = std::env::temp_dir().join(format!("ares-replay-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (user, sav) in expected["accounts"].as_object().unwrap() {
        fs::write(dir.join(format!("{user}.sav")), unhex(sav.as_str().unwrap())).unwrap();
    }
    w.math_seed(seed as i32);
    w.random = JavaRandom::new(seed);
    w.wordenc = ares_gameserver::cache::wordenc::WordEnc::load(root().join("data/raw/wordenc")).unwrap();
    w.rsa = Some(key.clone());
    let public = key.clone();
    let crc_buffer = CrcTable::new(&mut FileStream::open(&pack, false).unwrap()).buffer;
    struct C {
        cfg: Value,
        id: Option<u32>,
        isaac: Isaac,
    }
    let mut clients: Vec<C> = scenario["clients"].as_array().unwrap().iter().map(|c| {
        let seeds: Vec<i32> = c["seeds"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect();
        C { cfg: c.clone(), id: None, isaac: Isaac::new(&seeds) }
    }).collect();
    let login_packet = |c: &Value| {
        let seeds: Vec<i32> = c["seeds"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect();
        let s = |k: &str| c[k].as_str().unwrap();
        ares_gameserver::net::client::login_packet(&public, &crc_buffer, &seeds.try_into().unwrap(), c["uid"].as_i64().unwrap() as i32, s("username"), s("password"), c["lowmem"].as_bool().unwrap_or(false), c["reconnect"].as_bool().unwrap_or(false))
    };
    let mut pending: Vec<login::LoginReply> = Vec::new();
    let mut by_client: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    // under BLESS: run as the recording did, to the scenario's end or the shutdown tick
    let ticks = if bless().is_some() { scenario["ticks"].as_u64().unwrap() as usize } else { expected["ticks"].as_array().unwrap().len() };
    let (mut failures, mut ran) = (Vec::new(), 0);
    for t in 0..ticks {
        let t = t as i64;
        w.clock = Clock::Fixed(1700000000000 + t * 600);
        let mut saves: Vec<Value> = Vec::new();
        for r in std::mem::take(&mut pending) {
            w.on_login_reply(r);
        }
        for (i, c) in clients.iter_mut().enumerate() {
            let connect = c.cfg["connect"].as_i64().unwrap();
            if t == connect {
                let id = w.connect(c.cfg["remote"].as_str().unwrap(), format!("client-{i}"));
                c.id = Some(id);
                by_client.insert(id, i);
                w.client_data(id, &[14, 0]);
            }
            if t == connect + 1 {
                let bytes = login_packet(&c.cfg);
                w.client_data(c.id.unwrap(), &bytes);
            }
            if c.cfg["disconnect"].as_i64() == Some(t) {
                w.client_closed(c.id.unwrap());
            }
        }
        for f in scenario["friendIn"].as_array().cloned().unwrap_or_default() {
            if f["tick"].as_i64() == Some(t) {
                let mut data = f["data"].clone();
                data["type"] = f["opcode"].clone();
                match ares_gameserver::link::friend::from_wire(data) {
                    Some(m) => w.on_friend_message(m),
                    None => tracing::error!("Unknown friend message: {}", f["opcode"]),
                }
            }
        }
        for p in scenario["packets"].as_array().cloned().unwrap_or_default() {
            if p["tick"].as_i64() == Some(t) {
                let c = &mut clients[p["client"].as_u64().unwrap() as usize];
                let opcode = p["opcode"].as_i64().unwrap() as i32;
                let payload = if let Some(cheat) = p["cheat"].as_str() {
                    let mut b: Vec<u8> = cheat.chars().map(|c| c as u8).collect();
                    b.push(10);
                    b
                } else if let Some(chat) = p["chat"].as_str() {
                    let mut out = Packet::with_len(100);
                    ares_gameserver::cache::word_pack::pack(&mut out, chat);
                    let mut b = vec![p["colour"].as_i64().unwrap_or(0) as u8, p["effect"].as_i64().unwrap_or(0) as u8];
                    b.extend_from_slice(&out.data[..out.pos]);
                    b
                } else {
                    unhex(p["payload"].as_str().unwrap())
                };
                let bytes = ares_gameserver::net::client::frame_packet(&mut c.isaac, opcode as u8, &payload);
                w.client_data(c.id.unwrap(), &bytes);
            }
        }
        let exited = w.cycle();
        for req in std::mem::take(&mut w.login_out) {
            if let LoginRequest::PlayerLogout { username, save } | LoginRequest::PlayerAutosave { username, save } = &req {
                saves.push(json!([username, hex(save)]));
            }
            if let Some(r) = login::inline(&dir, false, true, &req).unwrap() {
                pending.push(r);
            }
        }
        let mut sends = Vec::new();
        let mut events = Vec::new();
        for (id, out) in std::mem::take(&mut w.outbox) {
            let i = by_client[&id];
            match out {
                Outgoing::Data(b) => sends.push(json!([i, hex(&b)])),
                Outgoing::Close => events.push(json!([i, "close"])),
                Outgoing::Terminate => events.push(json!([i, "terminate"])),
            }
        }
        let friend = std::mem::take(&mut w.friend_out);
        let got = json!({ "sends": sends, "events": events, "saves": saves, "friend": friend });
        let want = expected["ticks"].get(t as usize).unwrap_or(&Value::Null);
        if &got != want {
            if bless().is_some() {
                failures.push(format!("tick {t}"));
                let ticks = expected["ticks"].as_array_mut().unwrap();
                match ticks.get_mut(t as usize) {
                    Some(e) => patch(e, got),
                    None => ticks.push(js(got)),
                }
            } else {
                let detail = ["sends", "events", "saves", "friend"]
                    .iter()
                    .filter(|k| got[**k] != want[**k])
                    .map(|k| format!("{k}: {}", first_json_diff(&want[*k], &got[*k])))
                    .collect::<Vec<_>>()
                    .join("\n");
                panic!("{name}: tick {t} differs\n{detail}");
            }
        }
        ran += 1;
        if bless().is_some() {
            if exited {
                break;
            }
            continue;
        }
        let last = t as usize + 1 == expected["ticks"].as_array().unwrap().len();
        let recorded_exit = last && t + 1 < scenario["ticks"].as_i64().unwrap();
        assert_eq!(exited, recorded_exit, "{name}: shutdown completion differs at tick {t}");
    }
    let recorded = expected["ticks"].as_array_mut().unwrap();
    if recorded.len() > ran {
        failures.push(format!("{} ticks recorded, {ran} run", recorded.len()));
        recorded.truncate(ran);
    }
    let _ = fs::remove_dir_all(&dir);
    settle(name, expected_path, &expected, failures);
}

#[test]
fn replay_idle_matches() {
    check_replay("idle");
}

#[test]
fn replay_interact_matches() {
    check_replay("interact");
}

#[test]
fn replay_multi_matches() {
    check_replay("multi");
}

#[test]
fn replay_social_matches() {
    check_replay("social");
}

/// The login, friend and logger servers answer the step sequence of
/// tests/servers/steps.json as recorded in tests/servers/expected.json:
/// the messages every connection received, the database rows and the saves,
/// with the same normalisation of clock- and salt-dependent values.
#[test]
fn servers_match() {
    use ares_gameserver::server::{friend::FriendServer, logger::LoggerServer, login::LoginServer};
    use ares_gameserver::util::config::WorldConfig;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Map, Value, json};
    use sqlx::{Column, Row, TypeInfo, ValueRef};
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::Message;

    const PORTS: [u16; 3] = [47210, 47211, 47212];
    let steps: Vec<Value> = serde_json::from_str(&fs::read_to_string(root().join("tests/servers/steps.json")).unwrap()).unwrap();
    let expected: Value = serde_json::from_str(&fs::read_to_string(root().join("tests/servers/expected.json")).unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!("ares-servers-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("data/config")).unwrap();
    let world = json!({ "website": { "registration": false }, "login": { "port": PORTS[0] }, "friend": { "port": PORTS[1] }, "logger": { "port": PORTS[2] } });
    fs::write(dir.join("data/config/world.json"), world.to_string()).unwrap();

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let got = rt.block_on(async {
        let config = WorldConfig::load(&dir);
        let login = LoginServer::new(config.clone(), dir.clone()).await.unwrap();
        let friend = FriendServer::new(config.clone(), dir.clone()).await.unwrap();
        let logger = LoggerServer::new(config, dir.clone()).await.unwrap();
        tokio::spawn(login.run());
        tokio::spawn(friend.run());
        tokio::spawn(logger.run());
        tokio::time::sleep(Duration::from_millis(300)).await;

        let port = |c: &str| if c.starts_with('w') { PORTS[0] } else if c == "log" { PORTS[2] } else { PORTS[1] };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, Value)>();
        let mut sinks = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        let mut received: HashMap<String, Vec<Value>> = HashMap::new();
        let drain = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<(String, Value)>, received: &mut HashMap<String, Vec<Value>>| {
            while let Ok((c, v)) = rx.try_recv() {
                received.entry(c).or_default().push(v);
            }
        };
        for step in &steps {
            let conn = step["conn"].as_str().unwrap().to_string();
            if !sinks.contains_key(&conn) {
                let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}", port(&conn))).await.unwrap();
                let (sink, mut source) = ws.split();
                let (tx, name) = (tx.clone(), conn.clone());
                tokio::spawn(async move {
                    while let Some(Ok(m)) = source.next().await {
                        match m {
                            Message::Text(t) => {
                                let _ = tx.send((name.clone(), serde_json::from_str(&t).unwrap()));
                            }
                            Message::Close(_) => break,
                            _ => {}
                        }
                    }
                    let _ = tx.send((name, json!("close")));
                });
                sinks.insert(conn.clone(), sink);
                order.push(conn.clone());
                received.entry(conn.clone()).or_default();
            }
            sinks.get_mut(&conn).unwrap().send(Message::Text(step["msg"].to_string().into())).await.unwrap();
            // a reply proves the step was handled; fire-and-forget steps get time to land
            let mut settle = Duration::from_millis(150);
            if let Some(id) = step["msg"]["replyTo"].as_str() {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                while !received.values().flatten().any(|v| v["replyTo"] == id) && tokio::time::Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    drain(&mut rx, &mut received);
                }
                settle = Duration::from_millis(20);
            }
            tokio::time::sleep(settle).await;
            drain(&mut rx, &mut received);
        }
        for s in sinks.values_mut() {
            let _ = s.close().await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        drain(&mut rx, &mut received);
        for msgs in received.values_mut() {
            for m in msgs.iter_mut() {
                if m["remaining"].as_f64().is_some_and(|r| r > 0.0) {
                    m["remaining"] = json!("<ms>");
                }
            }
        }

        let now_cols = ["account.registration_date", "account_login.login_time", "account_login.logout_time", "hiscore.date", "hiscore_large.date", "private_chat.timestamp", "friendlist.created", "ignorelist.created"];
        let url = format!("sqlite:{}", dir.join("db.sqlite").display()).replace('\\', "/");
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE '\\_%' ESCAPE '\\' AND name != 'sqlite_sequence' ORDER BY name").fetch_all(&pool).await.unwrap();
        let mut tables = Map::new();
        for name in names {
            let rows = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM \"{name}\" ORDER BY rowid"))).fetch_all(&pool).await.unwrap();
            let rows: Vec<Value> = rows
                .iter()
                .map(|r| {
                    let mut o = Map::new();
                    for (i, c) in r.columns().iter().enumerate() {
                        let raw = r.try_get_raw(i).unwrap();
                        let v = match raw.type_info().name() {
                            _ if raw.is_null() => Value::Null,
                            "INTEGER" => json!(r.try_get_unchecked::<i64, _>(i).unwrap()),
                            "REAL" => json!(r.try_get_unchecked::<f64, _>(i).unwrap()),
                            "BLOB" => json!(r.try_get_unchecked::<Vec<u8>, _>(i).unwrap().iter().map(|b| format!("{b:02x}")).collect::<String>()),
                            _ => json!(r.try_get_unchecked::<String, _>(i).unwrap()),
                        };
                        let key = c.name().to_string();
                        let v = if v.is_null() {
                            v
                        } else if key == "password" {
                            json!("<hash>")
                        } else if now_cols.contains(&format!("{name}.{key}").as_str()) {
                            json!("<now>")
                        } else {
                            v
                        };
                        o.insert(key, v);
                    }
                    Value::Object(o)
                })
                .collect();
            tables.insert(name, Value::Array(rows));
        }
        pool.close().await;
        let mut saves = Map::new();
        let players = dir.join("data/players/main");
        let mut files: Vec<_> = fs::read_dir(&players).map(|d| d.map(|e| e.unwrap().path()).collect()).unwrap_or_default();
        files.sort();
        for f in files {
            let hex = hex(&fs::read(&f).unwrap());
            saves.insert(f.file_name().unwrap().to_string_lossy().into_owned(), json!(hex));
        }
        json!({ "received": received, "tables": tables, "saves": saves })
    });
    rt.shutdown_timeout(Duration::from_millis(100));
    let _ = fs::remove_dir_all(&dir);
    if let Some(all) = bless() {
        let mut expected = expected;
        let mut got = got;
        let changed = ["received", "tables", "saves"].iter().any(|k| got[*k] != expected[*k]);
        for key in ["received", "tables", "saves"] {
            patch(&mut expected[key], got[key].take());
        }
        if all || changed {
            write_json(&root().join("tests/servers/expected.json"), &expected);
        }
        return;
    }
    for key in ["received", "tables", "saves"] {
        for (k, want) in expected[key].as_object().unwrap() {
            assert_eq!(&got[key][k], want, "servers: {key}.{k} differs");
        }
        assert_eq!(got[key].as_object().unwrap().len(), expected[key].as_object().unwrap().len(), "servers: {key} entries differ");
    }
}

/// The live-reload watcher rebuilds `data/pack` after a source change
/// settles and reports a failed build; `World::reload` picks the pack up.
#[test]
fn dev_reload_rebuilds() {
    use ares_gameserver::pack::dev::{DevMsg, DevWatcher};
    use std::time::Duration;
    let case = "01-minimal";
    let root = std::env::temp_dir().join(format!("ares-dev-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let engine = root.join("engine");
    copy_dir(&crate::root().join("tests/fixtures").join(case).join("content"), &root.join("content"));
    fs::create_dir_all(engine.join("data/raw")).unwrap();
    fs::copy(crate::root().join("data/raw/wordenc"), engine.join("data/raw/wordenc")).unwrap();
    let mut build = ares_gameserver::util::config::WorldConfig::default().build;
    build.src_dir = "../content".into();
    build.verify = golden_json(case, "meta.json")["verify"].as_bool().unwrap();
    ares_gameserver::pack::all::clean(&engine, &build).unwrap();
    ares_gameserver::pack::all::pack_all(&engine, &build, ares_compiler::compiler::SourceNames::Absolute).unwrap();

    let dev = DevWatcher::spawn(engine.clone(), build.clone()).unwrap();
    let script = files_under(&root.join("content/scripts")).into_iter().find(|f| f.ends_with(".rs2")).expect("a fixture script");
    let script = root.join("content/scripts").join(script);
    let original = fs::read_to_string(&script).unwrap();
    let wait = |what: &str| dev.messages.recv_timeout(Duration::from_secs(120)).unwrap_or_else(|_| panic!("no dev message after {what}"));

    fs::write(&script, format!("{original}\n// touched\n")).unwrap();
    assert!(matches!(wait("a touch"), DevMsg::Reload), "touch: build should succeed");

    fs::write(&script, format!("{original}\n[proc,dev_broken]\nthis is not runescript\n")).unwrap();
    assert!(matches!(wait("a broken script"), DevMsg::Failure(_)), "broken script: build should fail");

    fs::write(&script, &original).unwrap();
    assert!(matches!(wait("the fix"), DevMsg::Reload), "restored script: build should succeed");

    let pack = engine.join("data/pack");
    let mut w = ares_gameserver::engine::world::World::new(true, ares_gameserver::engine::configs::Configs::load(&pack, true).unwrap());
    w.pack_dir = pack;
    w.debug = false;
    w.reload(true);
    assert!(w.reloaded && w.scripts.loaded > 0, "reload loads the rebuilt scripts");
    assert_eq!(w.vars.len(), w.configs.varss.count());
    drop(dev);
    let _ = fs::remove_dir_all(&root);
}

/// A login block whose username has no terminator: `gjstr` runs to the end
/// of the buffer and the password read goes past it. The world must
/// terminate that client and survive.
#[test]
fn malformed_login_terminates_client() {
    use ares_gameserver::cache::crc_table::CrcTable;
    use ares_gameserver::engine::{configs::Configs, world::{Outgoing, World}};
    use ares_gameserver::io::{filestream::FileStream, packet::Packet};
    let pack = root().join("tests/golden/06-npcs/data/pack");
    let mut w = World::new(true, Configs::load(&pack, true).unwrap());
    let table = CrcTable::new(&mut FileStream::open(&pack, false).unwrap());
    w.crc32 = table.crc32;
    let key = test_key();
    let mut rsa = Packet::with_len(128);
    rsa.p1(10);
    for v in [1, 2, 3, 4, 5] {
        rsa.p4(v);
    }
    rsa.pdata(b"abc", 0, 3);
    rsa.rsaenc(&key);
    let mut body = vec![0xff, 1, 18, 0];
    body.extend_from_slice(&table.buffer[..36]);
    body.extend_from_slice(&rsa.data[..rsa.pos]);
    let mut login = vec![16, body.len() as u8];
    login.extend_from_slice(&body);
    w.rsa = Some(key);
    let id = w.connect("127.0.0.1", "malformed".into());
    w.client_data(id, &[14, 0]);
    w.client_data(id, &login);
    assert!(w.outbox.iter().any(|(c, o)| *c == id && matches!(o, Outgoing::Terminate)), "client not terminated");
}
