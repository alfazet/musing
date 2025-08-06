use anyhow::{Result, anyhow};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    signal,
    sync::{broadcast, mpsc, oneshot},
};

use crate::{
    config::{Config, ServerConfig},
    model::{request::*, response::Response},
    player,
};

type SenderToPlayer = mpsc::UnboundedSender<Request>;
type ShutdownSender = broadcast::Sender<()>;
type ShutdownReceiver = broadcast::Receiver<()>;

struct ClientHandler {
    stream: BufReader<TcpStream>,
    tx: SenderToPlayer,
    rx_shutdown: ShutdownReceiver,
}

struct Server {
    port: u16,
}

impl ClientHandler {
    pub fn new(stream: TcpStream, tx: SenderToPlayer, rx_shutdown: ShutdownReceiver) -> Self {
        let stream = BufReader::new(stream);
        Self {
            stream,
            tx,
            rx_shutdown,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        self.stream
            .write_all(format!("rustmpd v{}\n", env!("CARGO_PKG_VERSION")).as_bytes())
            .await?;
        loop {
            // read the length (2 bytes, big endian)
            let res = tokio::select! {
                res = self.stream.read_u16() => res,
                _ = self.rx_shutdown.recv() => break,
            };
            let Ok(len) = res else {
                let _ = self.stream.shutdown();
                break;
            };

            // read the message
            let mut buf = vec![0; len as usize];
            let res = tokio::select! {
                res = self.stream.read_exact(&mut buf) => res,
                _ = self.rx_shutdown.recv() => break,
            };
            if let Err(e) = res {
                let _ = self.stream.shutdown();
                break;
            }
            let s = String::from_utf8(buf)?;
            let response = match RequestKind::try_from(s.as_str()) {
                Ok(request_kind) => {
                    let (tx_response, rx_response) = oneshot::channel();
                    let _ = self.tx.send(Request::new(request_kind, tx_response));
                    rx_response.await?.into_json_string()?
                }
                Err(e) => Response::new_err(e.to_string()).into_json_string()?,
            };
            let _ = self.stream.write_all(response.as_bytes()).await?;
        }

        Ok(())
    }
}

impl Server {
    pub fn new(config: ServerConfig) -> Self {
        let ServerConfig { port } = config;
        Self { port }
    }

    pub async fn run(&self, tx: SenderToPlayer, tx_shutdown: ShutdownSender) -> Result<()> {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", self.port)).await?;
        loop {
            let (stream, _) = listener.accept().await?;
            let tx_ = tx.clone();
            let rx_shutdown = tx_shutdown.subscribe();
            tokio::spawn(async move {
                let mut client_handler = ClientHandler::new(stream, tx_, rx_shutdown);
                if let Err(e) = client_handler.run().await {
                    log::error!("{}", e);
                }
            });
        }

        Ok(())
    }
}

pub async fn run(config: Config) {
    let Config {
        server_config,
        player_config,
    } = config;
    let (tx, rx) = mpsc::unbounded_channel();
    let (tx_shutdown, rx_shutdown) = broadcast::channel(1);
    let server = Server::new(server_config);
    let player_task = tokio::spawn(async move { player::run(player_config, rx).await });

    let res = tokio::select! {
        res = server.run(tx, tx_shutdown) => res,
        res = player_task => res.map_err(|e| anyhow!(e)),
        _ = signal::ctrl_c() => Ok(()),
    };
    if let Err(e) = res {
        log::error!("{}", e);
    }

    // at this point tx_shutdown was dropped => all client handlers ended => ...
    // ... all clones of tx got dropped => the player task ended
    // or the player task could've been the one to end first (due to a panic)
}
