//! One connection pool for the login, friend and logger
//! servers, sqlite (`db.sqlite` in the server root) or mysql by
//! `db.backend`. The schema is the prisma migrations in `migrations/`.

use crate::util::config::Db as DbConfig;
use sqlx::AnyPool;
use std::path::Path;

pub struct Db {
    pub pool: AnyPool,
    pub mysql: bool,
}

impl Db {
    /// Connects and brings the schema up to date. A database created by
    /// prisma (its bookkeeping, no `_sqlx_migrations`) is used as is.
    pub async fn connect(cfg: &DbConfig, root: &Path) -> Result<Db, String> {
        sqlx::any::install_default_drivers();
        let mysql = cfg.backend == "mysql";
        let url = if mysql {
            format!("mysql://{}:{}@{}:{}/{}", cfg.user, cfg.pass, cfg.host, cfg.port, cfg.name)
        } else {
            format!("sqlite:{}?mode=rwc", root.join("db.sqlite").display()).replace('\\', "/")
        };
        let pool = AnyPool::connect(&url).await.map_err(|e| format!("db: {e}"))?;
        let db = Db { pool, mysql };
        let adopted = db.has_table("account").await? && !db.has_table("_sqlx_migrations").await?;
        if !adopted {
            let migrator = if mysql { sqlx::migrate!("migrations/mysql") } else { sqlx::migrate!("migrations/sqlite") };
            migrator.run(&db.pool).await.map_err(|e| format!("db migrate: {e}"))?;
        }
        Ok(db)
    }

    async fn has_table(&self, name: &str) -> Result<bool, String> {
        let sql = if self.mysql {
            "SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = ?"
        } else {
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?"
        };
        let row = sqlx::query(sql).bind(name).fetch_optional(&self.pool).await.map_err(|e| e.to_string())?;
        Ok(row.is_some())
    }

    /// A DATETIME column read as text (`Any` decodes neither backend's DATETIME).
    pub fn text(&self, col: &str) -> String {
        if self.mysql { format!("CAST({col} AS CHAR)") } else { format!("CAST({col} AS TEXT)") }
    }
}

/// `YYYY-MM-DD HH:MM:SS` in UTC.
pub fn to_db_date(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, (rem / 60) % 60, rem % 60)
}

/// Parses a [`to_db_date`] string (or ISO with `T`/`Z`) to ms since the epoch.
/// ponytail: the zone-less form is read as UTC, not server-local time.
pub fn parse_db_date(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('Z').replace('T', " ");
    let (date, time) = s.split_once(' ').unwrap_or((&s, "00:00:00"));
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|p| p.split('.').next().unwrap_or("0").parse::<i64>().ok());
    let (hh, mm, ss) = (t.next().flatten().unwrap_or(0), t.next().flatten().unwrap_or(0), t.next().flatten().unwrap_or(0));
    Some((days_from_civil(y, m, day) * 86400 + hh * 3600 + mm * 60 + ss) * 1000)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// A date on a link: written as a `DATETIME` string, read from ms (possibly
/// fractional, floored) or a date string (`0` if unparseable).
pub mod date {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(ms: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::to_db_date(*ms))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Ms(f64),
            Text(String),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Ms(t) => t.floor() as i64,
            Raw::Text(t) => super::parse_db_date(&t).unwrap_or(0),
        })
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates_round_trip() {
        let ms = 1_700_000_123_000;
        let s = super::to_db_date(ms);
        assert_eq!(s, "2023-11-14 22:15:23");
        assert_eq!(super::parse_db_date(&s), Some(ms));
        assert_eq!(super::parse_db_date("2023-11-14T22:15:23.000Z"), Some(ms));
    }
}
