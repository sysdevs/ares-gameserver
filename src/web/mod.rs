//! Web game server (cargo feature `web-client`): the web client page
//! (`/rs2.cgi`, rendered from `view/*.ejs`), static `public/`, the archive 0
//! jags by CRC, the WebSocket game bridge on `/` (fed to the World like a TCP
//! socket, including on-demand requests), and the debug map-editor routes.
//! The management server (`/setup`, `/prometheus`) is not ported.

use crate::cache::crc_table::CrcTable;
use crate::engine::world::Outgoing;
use crate::io::filestream::FileStream;
use crate::net::tcp::{Inbound, next_client_id};
use axum::Router;
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade, rejection::WebSocketUpgradeRejection};
use axum::extract::{ConnectInfo, Path as UrlPath, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{any, get};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use tower_http::services::ServeDir;

/// What the routes need from the world config.
pub struct WebConfig {
    /// The server root (`data/`, `public/`, `view/`).
    pub root: PathBuf,
    /// `build.srcDir`, resolved against the root.
    pub src_dir: PathBuf,
    pub port: u16,
    pub allowed_origin: String,
    pub node_id: i64,
    pub node_port: i64,
    pub members: bool,
    pub debug: bool,
}

struct Ctx {
    cfg: WebConfig,
    events: Sender<Inbound>,
}

type Shared = State<Arc<Ctx>>;

/// `startWeb()`: binds `0.0.0.0:web.port` on its own runtime thread.
pub fn spawn(cfg: WebConfig, events: Sender<Inbound>) -> std::io::Result<std::thread::JoinHandle<()>> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let listener = rt.block_on(tokio::net::TcpListener::bind(("0.0.0.0", cfg.port)))?;
    let app = router(Arc::new(Ctx { cfg, events }));
    Ok(std::thread::spawn(move || {
        rt.block_on(async move {
            if let Err(e) = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await {
                tracing::error!("web: {e}");
            }
        });
    }))
}

fn router(ctx: Arc<Ctx>) -> Router {
    let public = ServeDir::new(ctx.cfg.root.join("public"));
    let mut app = Router::new().route("/", any(root)).route("/rs2.cgi", get(rs2));
    if ctx.cfg.debug {
        app = app
            .route("/worldmap.jag", get(worldmap))
            .route("/maped", get(|s: Shared| async move { view(&s.cfg, "maped", &[]) }))
            .route("/content/{*path}", get(get_content).put(put_content))
            .nest_service("/data", ServeDir::new(ctx.cfg.root.join("data")));
    }
    // the `/<name>:crc` routes are one-segment prefixes; anything else is `public/`
    app.fallback(move |s: Shared, uri: Uri, req: axum::extract::Request| async move {
        if let Some(r) = archive(&s, uri.path()) {
            return r;
        }
        use tower::ServiceExt;
        match public.oneshot(req).await {
            Ok(r) => r.into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    })
    .with_state(ctx)
}

/// `GET /` redirects; a WebSocket upgrade on `/` is a game connection.
async fn root(s: Shared, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap, ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>) -> Response {
    // `reply.redirect('/rs2.cgi', 302)`
    let Ok(ws) = ws else { return (StatusCode::FOUND, [(header::LOCATION, "/rs2.cgi")]).into_response() };
    let origin = &s.cfg.allowed_origin;
    if !origin.is_empty() && headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) != Some(origin.as_str()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let events = s.events.clone();
    ws.max_message_size(1600).on_upgrade(move |socket| bridge(socket, addr.ip().to_string(), events))
}

/// `WSClientSocket`: binary frames in and out, as a TCP client to the World.
async fn bridge(socket: WebSocket, remote: String, events: Sender<Inbound>) {
    let id = next_client_id();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Outgoing>();
    if events.send(Inbound::Connect { id, remote, out: out_tx }).is_err() {
        return;
    }
    let (mut sink, mut source) = socket.split();
    let writer = tokio::spawn(async move {
        while let Some(out) = out_rx.recv().await {
            match out {
                Outgoing::Data(b) => {
                    if sink.send(Message::Binary(b.into())).await.is_err() {
                        break;
                    }
                }
                Outgoing::Close => {
                    let _ = sink.close().await;
                    break;
                }
                Outgoing::Terminate => break,
            }
        }
    });
    while let Some(Ok(msg)) = source.next().await {
        let data = match msg {
            Message::Binary(b) => b.to_vec(),
            Message::Text(t) => t.as_bytes().to_vec(),
            Message::Close(_) => break,
            _ => continue,
        };
        if events.send(Inbound::Data(id, data)).is_err() {
            break;
        }
    }
    let _ = events.send(Inbound::Closed(id));
    writer.abort();
}

/// `GET /rs2.cgi?plugin&lowmem`.
async fn rs2(s: Shared, Query(q): Query<HashMap<String, String>>) -> Response {
    let int = |k: &str| q.get(k).and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(0);
    let (plugin, lowmem) = (int("plugin"), int("lowmem").to_string());
    let (nodeid, members) = (s.cfg.node_id.to_string(), s.cfg.members.to_string());
    if s.cfg.debug && plugin == 1 {
        let portoff = (s.cfg.node_port - 43594).to_string();
        view(&s.cfg, "java", &[("portoff", &portoff), ("nodeid", &nodeid), ("members", &members), ("lowmem", &lowmem)])
    } else {
        view(&s.cfg, "client", &[("nodeid", &nodeid), ("members", &members), ("lowmem", &lowmem)])
    }
}

/// Renders `view/<name>.ejs`; the templates only use `<%= name %>` with numbers and booleans.
fn view(cfg: &WebConfig, name: &str, vars: &[(&str, &str)]) -> Response {
    let Ok(mut html) = std::fs::read_to_string(cfg.root.join("view").join(format!("{name}.ejs"))) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    for (k, v) in vars {
        html = html.replace(&format!("<%= {k} %>"), v);
    }
    Html(html).into_response()
}

/// `/crc:cachebust` and `/<jag>:crc` (archive 0 files 1-8, served only for their current CRC).
fn archive(s: &Ctx, path: &str) -> Option<Response> {
    const NAMES: [&str; 9] = ["crc", "title", "config", "interface", "media", "versionlist", "textures", "wordenc", "sounds"];
    let seg = path.strip_prefix('/').filter(|p| !p.contains('/'))?;
    let (file, rest) = NAMES.iter().enumerate().find_map(|(i, n)| seg.strip_prefix(n).map(|r| (i, r)))?;
    // ponytail: opens the cache per request; keep one handle if this gets hot
    let Ok(mut cache) = FileStream::open(s.cfg.root.join("data/pack"), false) else {
        return Some(StatusCode::NOT_FOUND.into_response());
    };
    let table = CrcTable::new(&mut cache);
    if file == 0 {
        return Some(table.buffer.into_response());
    }
    if rest.parse::<i32>().ok() != Some(table.crcs.get(file).copied().unwrap_or(0)) {
        return Some(StatusCode::NOT_FOUND.into_response());
    }
    Some(cache.read(0, file, false).map_or_else(|| StatusCode::NOT_FOUND.into_response(), |d| d.into_response()))
}

async fn worldmap(s: Shared) -> Response {
    match std::fs::read(s.cfg.root.join("data/pack/mapview/worldmap.jag")) {
        Ok(d) => ([(header::CONTENT_TYPE, "application/octet-stream")], d).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `GET /content/*`: the content checkout (`build.srcDir`).
async fn get_content(s: Shared, UrlPath(rel): UrlPath<String>) -> Response {
    let Some(target) = inside(&s.cfg.src_dir, &rel) else { return StatusCode::BAD_REQUEST.into_response() };
    match tokio::fs::read(&target).await {
        Ok(d) => d.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `PUT /content/*`: the map editor saves into `build.srcDir`, never outside it.
async fn put_content(s: Shared, UrlPath(rel): UrlPath<String>, body: Bytes) -> Response {
    let Some(target) = inside(&s.cfg.src_dir, &rel) else { return StatusCode::BAD_REQUEST.into_response() };
    if let Some(dir) = target.parent()
        && let Err(e) = tokio::fs::create_dir_all(dir).await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    match tokio::fs::write(&target, &body).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// `resolveContentPath`: `rel` joined under `root`, or `None` if it would escape.
fn inside(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    rel.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)).then(|| root.join(rel))
}

#[cfg(test)]
mod tests {
    use super::inside;
    use std::path::Path;

    #[test]
    fn content_paths_stay_inside() {
        let root = Path::new("content");
        assert!(inside(root, "maps/m50_50.jm2").is_some());
        assert!(inside(root, "../secret").is_none());
        assert!(inside(root, "maps/../../x").is_none());
        assert!(inside(root, "/etc/passwd").is_none());
    }
}
