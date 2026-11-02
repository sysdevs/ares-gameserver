//! The JSON-over-WebSocket transport the internal servers share.

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message;

/// One accepted connection: incoming JSON messages, and a sender for replies.
pub struct Conn {
    pub id: u64,
    pub incoming: UnboundedReceiver<Value>,
    pub out: UnboundedSender<Value>,
}

/// Binds `0.0.0.0:port` and hands each WebSocket connection to `on_conn`.
pub async fn serve<F, Fut>(port: u16, on_conn: F) -> std::io::Result<()>
where
    F: Fn(Conn) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    let mut next = 0u64;
    loop {
        let (stream, _) = listener.accept().await?;
        next += 1;
        let id = next;
        let Ok(ws) = tokio_tungstenite::accept_async(stream).await else { continue };
        let (mut sink, mut source) = ws.split();
        let (in_tx, in_rx) = unbounded_channel();
        let (out_tx, mut out_rx) = unbounded_channel::<Value>();
        let (gone_tx, mut gone) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let _gone = gone_tx;
            while let Some(Ok(msg)) = source.next().await {
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
                    Message::Close(_) => break,
                    _ => continue,
                };
                match serde_json::from_str::<Value>(&text) {
                    Ok(v) => {
                        if in_tx.send(v).is_err() {
                            break;
                        }
                    }
                    Err(e) => tracing::error!("bad message: {e}"),
                }
            }
        });
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    v = out_rx.recv() => match v {
                        Some(v) if sink.send(Message::Text(v.to_string().into())).await.is_ok() => {}
                        _ => break,
                    },
                    // peer closed: finish the close handshake and drop the socket
                    _ = &mut gone => break,
                }
            }
            let _ = sink.close().await;
        });
        tokio::spawn(on_conn(Conn { id, incoming: in_rx, out: out_tx }));
    }
}

/// A reconnecting client: messages sent while the link is down are dropped.
pub struct Client {
    out: UnboundedSender<Value>,
}

impl Client {
    /// Connects to `ws://host:port`, delivering incoming messages to `incoming`.
    /// `hello` is sent first on every connection (`once`: only the first).
    pub fn spawn(host: String, port: u16, incoming: UnboundedSender<Value>, mut hello: Option<Value>, once: bool) -> Client {
        let (out_tx, mut out_rx) = unbounded_channel::<Value>();
        tokio::spawn(async move {
            loop {
                let url = format!("ws://{host}:{port}");
                match tokio_tungstenite::connect_async(&url).await {
                    Ok((ws, _)) => {
                        let (mut sink, mut source) = ws.split();
                        let greeting = if once { hello.take() } else { hello.clone() };
                        if let Some(h) = greeting
                            && sink.send(Message::Text(h.to_string().into())).await.is_err()
                        {
                            continue;
                        }
                        let tx = incoming.clone();
                        let reader = tokio::spawn(async move {
                            while let Some(Ok(Message::Text(t))) = source.next().await {
                                if let Ok(v) = serde_json::from_str::<Value>(&t) {
                                    let _ = tx.send(v);
                                }
                            }
                        });
                        while let Some(v) = out_rx.recv().await {
                            if reader.is_finished() || sink.send(Message::Text(v.to_string().into())).await.is_err() {
                                break;
                            }
                        }
                        reader.abort();
                    }
                    Err(_) => {
                        // drain what was queued while down, then retry
                        while out_rx.try_recv().is_ok() {}
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });
        Client { out: out_tx }
    }

    pub fn send(&self, v: Value) {
        let _ = self.out.send(v);
    }
}
