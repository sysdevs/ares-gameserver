//! Session logs, wealth events, abuse reports and input
//! tracking written to the database.

use super::db::{Db, date, to_db_date};
use super::ws::{self, Conn};
use crate::util::base64;
use crate::util::config::WorldConfig;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

pub struct LoggerServer {
    db: Db,
    port: u16,
}

/// World → logger server.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LogMsg {
    SessionLog { logs: Vec<SessionLog> },
    WealthEvent { events: Vec<WealthEvent> },
    Report {
        session_uuid: String,
        #[serde(with = "date")]
        timestamp: i64,
        coord: i64,
        offender: String,
        reason: i64,
    },
    InputTrack {
        session_uuid: String,
        #[serde(with = "date")]
        timestamp: i64,
        buf: String,
    },
}

#[derive(Debug, Deserialize)]
struct SessionLog {
    session_uuid: String,
    #[serde(with = "date")]
    timestamp: i64,
    coord: i64,
    event: String,
    event_type: i64,
}

/// The item lists are stored as their JSON text.
#[derive(Debug, Deserialize)]
struct WealthEvent {
    session_uuid: String,
    #[serde(with = "date")]
    timestamp: i64,
    coord: i64,
    event_type: i64,
    account_items: Value,
    account_value: i64,
    recipient_session: Option<String>,
    recipient_items: Option<Value>,
    recipient_value: Option<i64>,
}

impl LoggerServer {
    pub async fn new(config: WorldConfig, root: PathBuf) -> Result<Arc<Self>, String> {
        let db = Db::connect(&config.db, &root).await?;
        Ok(Arc::new(LoggerServer { db, port: config.logger.port as u16 }))
    }

    pub async fn run(self: Arc<Self>) -> std::io::Result<()> {
        tracing::info!("Logger server listening on port {}", self.port);
        let port = self.port;
        ws::serve(port, move |conn| self.clone().connection(conn)).await
    }

    async fn connection(self: Arc<Self>, mut conn: Conn) {
        while let Some(msg) = conn.incoming.recv().await {
            let msg = match serde_json::from_value(msg) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!("[Logger]: bad message: {e}");
                    continue;
                }
            };
            if let Err(e) = self.handle(msg).await {
                tracing::error!("{e}");
            }
        }
    }

    async fn handle(&self, msg: LogMsg) -> Result<(), sqlx::Error> {
        let pool = &self.db.pool;
        match msg {
            LogMsg::SessionLog { logs } => {
                for x in logs {
                    sqlx::query("INSERT INTO session_log (session_uuid, timestamp, coord, event, event_type) VALUES (?, ?, ?, ?, ?)")
                        .bind(x.session_uuid)
                        .bind(to_db_date(x.timestamp))
                        .bind(x.coord)
                        .bind(x.event)
                        .bind(x.event_type)
                        .execute(pool)
                        .await?;
                }
            }
            LogMsg::WealthEvent { events } => {
                for x in events {
                    sqlx::query("INSERT INTO session_wealth (session_uuid, timestamp, coord, event_type, account_items, account_value, recipient_session, recipient_items, recipient_value) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
                        .bind(x.session_uuid)
                        .bind(to_db_date(x.timestamp))
                        .bind(x.coord)
                        .bind(x.event_type)
                        .bind(x.account_items.to_string())
                        .bind(x.account_value)
                        .bind(x.recipient_session)
                        .bind(x.recipient_items.map(|v| v.to_string()))
                        .bind(x.recipient_value)
                        .execute(pool)
                        .await?;
                }
            }
            LogMsg::Report { session_uuid, timestamp, coord, offender, reason } => {
                sqlx::query("INSERT INTO report (session_uuid, timestamp, coord, offender, reason) VALUES (?, ?, ?, ?, ?)")
                    .bind(session_uuid)
                    .bind(to_db_date(timestamp))
                    .bind(coord)
                    .bind(offender)
                    .bind(reason)
                    .execute(pool)
                    .await?;
            }
            LogMsg::InputTrack { session_uuid, timestamp, buf } => {
                sqlx::query("INSERT INTO input_report (session_uuid, timestamp, data) VALUES (?, ?, ?)")
                    .bind(session_uuid)
                    .bind(to_db_date(timestamp))
                    .bind(base64::decode(&buf))
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }
}
