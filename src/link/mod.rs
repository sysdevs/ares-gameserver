//! World ↔ server links (login and friend; the world has no logger link yet).
//! The World queues requests in its outboxes and receives replies through
//! `World::on_login_reply`; who carries them (a WebSocket to `ares-login`, or
//! [`login::inline`] for `login.enabled=false` and replays) is the caller's
//! choice.

pub mod friend;
pub mod login;
