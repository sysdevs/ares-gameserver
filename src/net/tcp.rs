//! Accepts game connections on a tokio
//! runtime and forwards their bytes to the World thread as [`Inbound`]
//! events; each connection drains its own [`Outgoing`] queue.

use crate::engine::world::{ClientId, Outgoing};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

/// A connection event for the World thread.
pub enum Inbound {
    Connect { id: ClientId, remote: String, out: UnboundedSender<Outgoing> },
    Data(ClientId, Vec<u8>),
    Closed(ClientId),
}

/// Ids handed out to connections (the World uses them as `ClientId`s).
static NEXT_ID: AtomicU32 = AtomicU32::new(1 << 20);

pub fn next_client_id() -> ClientId {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Binds `0.0.0.0:port` and serves connections until the process exits.
pub fn listen(port: u16, events: Sender<Inbound>) -> std::io::Result<std::thread::JoinHandle<()>> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let listener = rt.block_on(TcpListener::bind(("0.0.0.0", port)))?;
    Ok(std::thread::spawn(move || {
        rt.block_on(async move {
            loop {
                let Ok((socket, addr)) = listener.accept().await else { continue };
                let _ = socket.set_nodelay(true);
                let id = next_client_id();
                let (out_tx, mut out_rx) = unbounded_channel::<Outgoing>();
                let remote = addr.ip().to_string();
                if events.send(Inbound::Connect { id, remote, out: out_tx }).is_err() {
                    return;
                }
                let (mut reader, writer) = socket.into_split();
                let tx = events.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 5000];
                    loop {
                        // `setTimeout(30000)`: an idle socket is destroyed
                        match tokio::time::timeout(Duration::from_secs(30), reader.read(&mut buf)).await {
                            Ok(Ok(n)) if n > 0 => {
                                if tx.send(Inbound::Data(id, buf[..n].to_vec())).is_err() {
                                    break;
                                }
                            }
                            _ => break,
                        }
                    }
                    let _ = tx.send(Inbound::Closed(id));
                });
                tokio::spawn(async move {
                    // a tick queues many small messages: buffer them and flush once the queue drains
                    let mut writer = BufWriter::new(writer);
                    while let Some(out) = out_rx.recv().await {
                        match out {
                            Outgoing::Data(b) => {
                                if writer.write_all(&b).await.is_err() || (out_rx.is_empty() && writer.flush().await.is_err()) {
                                    break;
                                }
                            }
                            Outgoing::Close => {
                                let _ = writer.shutdown().await;
                                break;
                            }
                            Outgoing::Terminate => {
                                let _ = writer.flush().await;
                                break;
                            }
                        }
                    }
                });
            }
        });
    }))
}
