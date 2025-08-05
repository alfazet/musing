use anyhow::Result;
use std::io::{Error as IoError, ErrorKind as IoErrorKind};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    signal,
    sync::{broadcast, mpsc},
};

use crate::config::{Config, ServerConfig};

type SenderToPlayer = mpsc::UnboundedSender<()>;
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
                self.stream.shutdown();
                break;
            };

            // read the message
            let mut buf = vec![0; len as usize];
            let res = tokio::select! {
                res = self.stream.read_exact(&mut buf) => res,
                _ = self.rx_shutdown.recv() => break,
            };
            if let Err(e) = res {
                self.stream.shutdown();
                break;
            }
            let s = String::from_utf8(buf)?;

            // parse the message into a Request
            // send the Request bundled with the oneshot channel the the player
            // await the (json) response and send it back
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
    // let _ = tokio::spawn(async move || player::run(player_config, rx).await);

    let res = tokio::select! {
        res = server.run(tx, tx_shutdown) => res,
        _ = signal::ctrl_c() => Ok(()),
    };
    if let Err(e) = res {
        log::error!("{}", e);
    }

    // at this point tx_shutdown was dropped => all client handlers ended => ...
    // ... all clones of tx got dropped => the player task ended
}
