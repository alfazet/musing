use anyhow::Result;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{
        broadcast,
        mpsc::{self as tokio_chan},
        oneshot,
    },
};

use crate::{
    config::ServerConfig,
    model::{
        request::{Request, RequestKind},
        response::Response,
    },
};

struct ClientHandler {
    stream: BufReader<TcpStream>,
}

struct Server {
    port: u16,
}

impl ClientHandler {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream: BufReader::new(stream),
        }
    }

    pub async fn run(
        &mut self,
        tx_request: tokio_chan::UnboundedSender<Request>,
        mut rx_shutdown: broadcast::Receiver<()>,
    ) -> Result<()> {
        let welcome = json!({"version": env!("CARGO_PKG_VERSION")}).to_string();
        let bytes = welcome.as_bytes();
        self.stream.write_u32(bytes.len() as u32).await?;
        self.stream.write_all(bytes).await?;

        loop {
            // read the length (4 bytes, big endian)
            let res = tokio::select! {
                res = self.stream.read_u32() => res,
                _ = rx_shutdown.recv() => break,
            };
            let Ok(len) = res else {
                let _ = self.stream.shutdown().await;
                break;
            };

            // read the message
            let mut buf = vec![0; len as usize];
            let res = tokio::select! {
                res = self.stream.read_exact(&mut buf) => res,
                _ = rx_shutdown.recv() => break,
            };
            if res.is_err() {
                let _ = self.stream.shutdown().await;
                break;
            }
            let s = String::from_utf8(buf)?;

            // respond
            let response = match RequestKind::try_from(s.as_str()) {
                Ok(kind) => {
                    let (tx_response, rx_response) = oneshot::channel();
                    let _ = tx_request.send(Request { kind, tx_response });
                    rx_response.await?.into_json_string()?
                }
                Err(e) => Response::new_err(e.to_string()).into_json_string()?,
            };
            let bytes = response.as_bytes();
            self.stream.write_u32(bytes.len() as u32).await?;
            self.stream.write_all(bytes).await?;
        }

        Ok(())
    }
}

impl Server {
    pub fn new(config: ServerConfig) -> Self {
        let ServerConfig { port } = config;
        Self { port }
    }

    pub async fn run(
        &self,
        tx_request: tokio_chan::UnboundedSender<Request>,
        tx_shutdown: broadcast::Sender<()>,
    ) -> Result<()> {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", self.port)).await?;
        loop {
            let (stream, _) = listener.accept().await?;
            let tx_request_ = tx_request.clone();
            let rx_shutdown = tx_shutdown.subscribe();
            tokio::spawn(async move {
                let mut client_handler = ClientHandler::new(stream);
                if let Err(e) = client_handler.run(tx_request_, rx_shutdown).await {
                    log::error!("client handler error ({})", e);
                }
            });
        }
    }
}

pub async fn run(
    config: ServerConfig,
    tx_request: tokio_chan::UnboundedSender<Request>,
    mut rx_shutdown: broadcast::Receiver<()>,
) -> Result<()> {
    // the "shutdown" channel keeps one sender and many receivers
    // each client handler gets its own receiver
    // the only sender gets dropped whenever the server stops
    //
    // after that happens, all client handlers will error out
    // of any attempt to receive on the channel, which tells them to shut down
    let (tx_shutdown, _) = broadcast::channel(1);
    let server = Server::new(config);

    tokio::select! {
        res = server.run(tx_request, tx_shutdown) => res,
        _ = rx_shutdown.recv() => Ok(()),
    }
}
