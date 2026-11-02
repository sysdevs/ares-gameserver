//! The login RSA key, kept in sync across its three homes: the server's
//! `data/config/private.pem`, `data/config/public.pem` (read by `ares-bots`)
//! and the modulus/exponent compiled into the web client
//! (`public/client/client.js`, built from Client-TS with `LOGIN_RSAN` /
//! `LOGIN_RSAE`).

use crate::io::packet::RsaKey;
use std::path::Path;
use std::process::Command;

const CLIENT_TS: &str = "https://github.com/LostCityRS/Client-TS";
/// The Client-TS build outputs the web client needs (`SCC1_Florestan.sf2` is not built).
const CLIENT_FILES: [&str; 3] = ["client.js", "ondemandworker.js", "tinymidipcm.wasm"];

/// A key pair as PEM text (SPKI public, PKCS#1 private) and
/// its exponent and modulus in decimal, as the client takes them.
pub struct KeyPair {
    pub private_pem: String,
    pub public_pem: String,
    pub e: String,
    pub n: String,
}

/// A new 1024-bit key.
pub fn generate() -> Result<KeyPair, String> {
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::pkcs8::EncodePublicKey;
    use rsa::traits::PublicKeyParts;
    let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 1024).map_err(|e| e.to_string())?;
    let public = key.to_public_key();
    Ok(KeyPair {
        private_pem: key.to_pkcs1_pem(rsa::pkcs8::LineEnding::CRLF).map_err(|e| e.to_string())?.to_string(),
        public_pem: public.to_public_key_pem(rsa::pkcs8::LineEnding::CRLF).map_err(|e| e.to_string())?,
        e: public.e().to_string(),
        n: public.n().to_string(),
    })
}

pub fn write_pems(root: &Path, k: &KeyPair) -> Result<(), String> {
    let dir = root.join("data/config");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("private.pem"), &k.private_pem).map_err(|e| format!("private.pem: {e}"))?;
    std::fs::write(dir.join("public.pem"), &k.public_pem).map_err(|e| format!("public.pem: {e}"))
}

/// The SPKI PEM of a key's modulus and exponent. `new_unchecked`: the
/// Client-TS default key has a 256-bit exponent, which `rsa` would refuse.
fn public_pem_of(k: &RsaKey) -> Result<String, String> {
    use rsa::pkcs8::EncodePublicKey;
    let big = |b: &num_bigint::BigInt| rsa::BigUint::from_bytes_be(&b.to_bytes_be().1);
    rsa::RsaPublicKey::new_unchecked(big(&k.n), big(&k.e)).to_public_key_pem(rsa::pkcs8::LineEnding::CRLF).map_err(|e| e.to_string())
}

/// Whether a built `client.js` carries this modulus and exponent. The bundle
/// inlines both as `BigInt("<decimal>")` strings.
pub fn client_embeds(js: &str, n: &str, e: &str) -> bool {
    js.contains(&format!("\"{n}\"")) && js.contains(&format!("\"{e}\""))
}

/// `ares-pack keycheck`: compares the three copies against `private.pem`
/// (the one the server decrypts with). `fix` rewrites a mismatched
/// `public.pem` from it; a mismatched client needs `rekey`. Returns the
/// report, or the report as the error when something is out of sync.
pub fn keycheck(root: &Path, fix: bool) -> Result<Vec<String>, String> {
    let config = root.join("data/config");
    let pem = std::fs::read_to_string(config.join("private.pem")).map_err(|e| format!("data/config/private.pem: {e}"))?;
    let key = RsaKey::from_pem(&pem)?;
    let (n, e) = (key.n.to_string(), key.e.to_string());
    let mut lines = vec![format!("private.pem: {}-bit modulus", key.n.bits())];
    let mut ok = true;
    match std::fs::read_to_string(config.join("public.pem")).map_err(|e| e.to_string()).and_then(|p| RsaKey::from_public_pem(&p)) {
        Ok(p) if p.n == key.n && p.e == key.e => lines.push("public.pem: matches".into()),
        found => {
            let why = match found {
                Ok(p) if p.n == key.e && p.e == key.n => "modulus and exponent swapped".to_string(),
                Ok(_) => "a different key".to_string(),
                Err(e) => e,
            };
            if fix {
                std::fs::write(config.join("public.pem"), public_pem_of(&key)?).map_err(|e| format!("public.pem: {e}"))?;
                lines.push(format!("public.pem: {why}; rewritten from private.pem"));
            } else {
                ok = false;
                lines.push(format!("public.pem: {why} (ares-pack keycheck --fix rewrites it)"));
            }
        }
    }
    match std::fs::read_to_string(root.join("public/client/client.js")) {
        Ok(js) if client_embeds(&js, &n, &e) => lines.push("public/client/client.js: matches".into()),
        Ok(_) => {
            ok = false;
            lines.push("public/client/client.js: built with a different key; browser logins will fail (ares-pack rekey <Client-TS dir>)".into());
        }
        Err(_) => lines.push("public/client/client.js: not present (no web client)".into()),
    }
    if ok { Ok(lines) } else { Err(lines.join("\n")) }
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what} failed ({status})")) }
}

/// `ares-pack rekey <client-dir>`: a new key, compiled into a fresh Client-TS
/// build (cloned on branch `revision` when the directory is missing), then
/// installed. The server files change only after the build succeeded and its
/// `client.js` was checked to carry the new key, so a failure leaves the old,
/// consistent set in place. Restart `ares` afterwards.
pub fn rekey(root: &Path, client_dir: &Path, revision: i64) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    if !client_dir.join("bundle.ts").exists() {
        if client_dir.exists() && std::fs::read_dir(client_dir).map_err(|e| e.to_string())?.next().is_some() {
            return Err(format!("{} exists but is not a Client-TS checkout", client_dir.display()));
        }
        run(Command::new("git").args(["clone", "--depth", "1", "-b", &revision.to_string(), CLIENT_TS]).arg(client_dir), "git clone")?;
        lines.push(format!("cloned Client-TS {revision} into {}", client_dir.display()));
    }
    if !client_dir.join("node_modules").exists() {
        run(Command::new("bun").arg("install").current_dir(client_dir), "bun install")?;
    }
    let key = generate()?;
    // process env wins over a Client-TS `.env`
    run(Command::new("bun").args(["run", "build"]).env("LOGIN_RSAE", &key.e).env("LOGIN_RSAN", &key.n).current_dir(client_dir), "bun run build")?;
    let out = client_dir.join("out");
    let js = std::fs::read_to_string(out.join("client.js")).map_err(|e| format!("out/client.js: {e}"))?;
    if !client_embeds(&js, &key.n, &key.e) {
        return Err("the built client.js does not contain the new key; nothing was changed".into());
    }
    for f in CLIENT_FILES {
        if !out.join(f).exists() {
            return Err(format!("out/{f} is missing; nothing was changed"));
        }
    }
    let dest = root.join("public/client");
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    for f in CLIENT_FILES {
        std::fs::copy(out.join(f), dest.join(f)).map_err(|e| format!("copy {f}: {e}"))?;
    }
    write_pems(root, &key)?;
    lines.push(format!("installed {} and data/config/{{private,public}}.pem", CLIENT_FILES.map(|f| format!("public/client/{f}")).join(", ")));
    lines.extend(keycheck(root, false)?);
    lines.push("restart ares and hard-refresh the browser".into());
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swapped_public_pem_is_reported_and_fixed() {
        let dir = std::env::temp_dir().join(format!("ares-keycheck-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let k = generate().unwrap();
        write_pems(&dir, &k).unwrap();
        assert!(keycheck(&dir, false).is_ok(), "fresh pair is in sync");
        let key = RsaKey::from_pem(&k.private_pem).unwrap();
        let swapped = RsaKey { n: key.e.clone(), e: key.n.clone(), ..RsaKey::from_pem(&k.private_pem).unwrap() };
        std::fs::write(dir.join("data/config/public.pem"), public_pem_of(&swapped).unwrap()).unwrap();
        assert!(keycheck(&dir, false).unwrap_err().contains("swapped"));
        assert!(keycheck(&dir, true).is_ok());
        assert!(keycheck(&dir, false).is_ok(), "--fix restored it");
        std::fs::create_dir_all(dir.join("public/client")).unwrap();
        std::fs::write(dir.join("public/client/client.js"), format!("x(BigInt(\"{}\"),BigInt(\"{}\"))", k.n, k.e)).unwrap();
        assert!(keycheck(&dir, false).is_ok());
        std::fs::write(dir.join("public/client/client.js"), "x(BigInt(\"1\"),BigInt(\"3\"))").unwrap();
        assert!(keycheck(&dir, false).unwrap_err().contains("client.js: built with a different key"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
