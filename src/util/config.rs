//! `data/config/world.json` merged over the defaults with the `mergeConfig`
//! rules:
//! - only keys present in the defaults survive; nested objects merge recursively;
//! - a boolean default accepts `true`/`false` or the strings `"true"`/`"false"`;
//! - a number default accepts any JSON number as-is (floats survive, `tryParseInt`)
//!   or a string holding a whole integer (surrounding whitespace allowed); otherwise the default;
//! - a string default accepts only strings.
//!
//! There is no legacy `.env` migration.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldConfig {
    pub easy_startup: bool,
    pub website: Website,
    pub web: Web,
    pub engine: Engine,
    pub node: Node,
    pub login: Service,
    pub friend: Service,
    pub logger: Service,
    pub db: Db,
    pub build: Build,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Website {
    pub registration: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Web {
    #[serde(deserialize_with = "int")]
    pub port: i64,
    pub allowed_origin: String,
    #[serde(deserialize_with = "int")]
    pub management_port: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Engine {
    #[serde(deserialize_with = "int")]
    pub revision: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    #[serde(deserialize_with = "int")]
    pub id: i64,
    #[serde(deserialize_with = "int")]
    pub port: i64,
    pub members: bool,
    pub auto_subscribe_members: bool,
    /// Kept as `f64`: a fractional JSON value reaches the XP maths.
    pub xp_rate: f64,
    pub production: bool,
    #[serde(deserialize_with = "int")]
    pub minimum_wealth_value_event: i64,
    pub debug: bool,
    pub debug_profile: bool,
    pub client_routefinder: bool,
    pub profile: String,
    #[serde(deserialize_with = "int")]
    pub max_connected: i64,
    pub debug_proc_char: String,
    #[serde(deserialize_with = "int")]
    pub hop_time: i64,
    #[serde(deserialize_with = "int")]
    pub rate_limit_address_login: i64,
    #[serde(deserialize_with = "int")]
    pub rate_limit_device_login: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Service {
    pub enabled: bool,
    pub host: String,
    #[serde(deserialize_with = "int")]
    pub port: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Db {
    pub backend: String,
    pub host: String,
    #[serde(deserialize_with = "int")]
    pub port: i64,
    pub user: String,
    pub pass: String,
    pub name: String,
    pub verbose: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Build {
    pub verbose: bool,
    pub startup: bool,
    pub verify: bool,
    pub verify_folder: bool,
    pub verify_pack: bool,
    pub live_reload: bool,
    pub src_dir: String,
}

/// ponytail: integer fields truncate a fractional JSON value instead of
/// keeping it as a float; only `xpRate` is known to be used fractionally.
fn int<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(f64::deserialize(d)?.trunc() as i64)
}

impl Default for WorldConfig {
    fn default() -> Self {
        let svc = |port| Service {
            enabled: false,
            host: "localhost".into(),
            port,
        };
        WorldConfig {
            easy_startup: false,
            website: Website { registration: true },
            web: Web {
                port: if cfg!(any(windows, target_os = "macos")) {
                    80
                } else {
                    8888
                },
                allowed_origin: String::new(),
                management_port: 8898,
            },
            engine: Engine { revision: 274 },
            node: Node {
                id: 10,
                port: 43594,
                members: true,
                auto_subscribe_members: true,
                xp_rate: 1.0,
                production: false,
                minimum_wealth_value_event: 10,
                debug: true,
                debug_profile: false,
                client_routefinder: true,
                profile: "main".into(),
                max_connected: 1000,
                debug_proc_char: "~".into(),
                hop_time: 45000,
                rate_limit_address_login: 30,
                rate_limit_device_login: 5,
            },
            login: svc(43500),
            friend: svc(45099),
            logger: svc(43501),
            db: Db {
                backend: "sqlite".into(),
                host: "localhost".into(),
                port: 3306,
                user: "root".into(),
                pass: "password".into(),
                name: "lostcity".into(),
                verbose: false,
            },
            build: Build {
                verbose: false,
                startup: false,
                verify: false,
                verify_folder: false,
                verify_pack: true,
                live_reload: true,
                src_dir: "../content".into(),
            },
        }
    }
}

/// `mergeConfig(defaults, value)` over JSON values.
fn merge(default: &Value, value: Option<&Value>) -> Value {
    match default {
        Value::Object(d) => {
            let parsed = value.and_then(Value::as_object);
            Value::Object(
                d.iter()
                    .map(|(k, dv)| (k.clone(), merge(dv, parsed.and_then(|p| p.get(k)))))
                    .collect::<Map<_, _>>(),
            )
        }
        Value::Bool(b) => match value {
            Some(Value::Bool(v)) => Value::Bool(*v),
            Some(Value::String(s)) if s == "true" => Value::Bool(true),
            Some(Value::String(s)) if s == "false" => Value::Bool(false),
            _ => Value::Bool(*b),
        },
        Value::Number(_) => match value {
            Some(Value::Number(n)) => Value::Number(n.clone()),
            Some(Value::String(s)) => s
                .trim()
                .parse::<i64>()
                .map_or(default.clone(), |v| Value::Number(v.into())),
            _ => default.clone(),
        },
        Value::String(_) => match value {
            Some(Value::String(s)) => Value::String(s.clone()),
            _ => default.clone(),
        },
        other => other.clone(),
    }
}

impl WorldConfig {
    /// `normalizeWorldConfig(parsed)`.
    pub fn normalize(value: &Value) -> Self {
        let defaults = serde_json::to_value(WorldConfig::default()).expect("defaults serialize");
        let mut merged = merge(&defaults, Some(value));
        // Legacy compatibility: db.kyselyVerbose when db.verbose is absent.
        if let Some(db) = value.get("db").and_then(Value::as_object)
            && !db.contains_key("verbose")
        {
            let fallback = merged["db"]["verbose"].clone();
            merged["db"]["verbose"] = merge(&fallback, db.get("kyselyVerbose"));
        }
        serde_json::from_value(merged).expect("merged config has the default shape")
    }

    /// `loadWorldConfig()`: `data/config/world.json` relative to `root`,
    /// defaults when it is missing or unparsable.
    pub fn load(root: impl AsRef<std::path::Path>) -> Self {
        let path = root.as_ref().join("data/config/world.json");
        match std::fs::read_to_string(&path).map(|t| serde_json::from_str::<Value>(&t)) {
            Ok(Ok(v)) => WorldConfig::normalize(&v),
            Ok(Err(e)) => {
                tracing::warn!(
                    "[Config] Failed to parse {}: {e}. Using defaults.",
                    path.display()
                );
                WorldConfig::default()
            }
            Err(_) => WorldConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_follows_try_parse_rules() {
        let c = WorldConfig::normalize(&json!({
            "easyStartup": "true",
            "node": { "port": " 43595 ", "id": "10abc", "xpRate": 2.5, "members": "yes", "profile": 5, "bogus": 1 },
            "build": { "srcDir": "../c", "verify": false },
            "db": { "kyselyVerbose": true },
            "extra": {}
        }));
        assert!(c.easy_startup);
        assert_eq!(c.node.port, 43595);
        assert_eq!(
            c.node.id,
            WorldConfig::default().node.id,
            "malformed number string keeps the default"
        );
        assert_eq!(c.node.xp_rate, 2.5);
        assert!(c.node.members, "unparsable boolean keeps the default");
        assert_eq!(c.node.profile, "main", "non-string keeps the default");
        assert_eq!(c.build.src_dir, "../c");
        assert!(!c.build.verify);
        assert!(c.db.verbose, "legacy kyselyVerbose");
    }

    #[test]
    fn empty_object_is_defaults() {
        assert_eq!(WorldConfig::normalize(&json!({})), WorldConfig::default());
        assert_eq!(WorldConfig::normalize(&json!(null)), WorldConfig::default());
    }
}
