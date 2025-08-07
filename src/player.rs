use anyhow::{Result, bail};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    task,
};

use crate::{
    audio::AudioBackend,
    config::PlayerConfig,
    database::Database,
    error::MyError,
    model::{queue::Queue, request::*, response::Response},
};

type ReceiverFromServer = mpsc::UnboundedReceiver<Request>;

struct Player {
    queue: Queue,
    database: Database,
    audio_backend: AudioBackend,
}

impl Player {
    async fn db_request(&mut self, kind: DbRequestKind) -> Result<Response> {
        let (tx, rx) = oneshot::channel();
        rayon::scope(|s| {
            s.spawn(|_| {
                use DbRequestKind as DbKind;

                let response = match kind {
                    DbKind::Update => self.database.update(),
                    DbKind::Select(args) => self.database.select_outer(args),
                    DbKind::Metadata(args) => self.database.metadata(args),
                    DbKind::Unique(args) => self.database.unique(args),
                };
                let _ = tx.send(response);
            });
        });

        Ok(rx.await?)
    }

    async fn handle_request(&mut self, kind: RequestKind) -> Result<Response> {
        use RequestKind as Kind;

        match kind {
            Kind::DbRequestKind(db_request_kind) => self.db_request(db_request_kind).await,
            _ => todo!(),
        }
    }

    pub fn new(database: Database, audio_backend: AudioBackend) -> Self {
        Self {
            queue: Queue::default(),
            database,
            audio_backend,
        }
    }

    pub async fn run(&mut self, mut rx: ReceiverFromServer) -> Result<()> {
        loop {
            tokio::select! {
                res = rx.recv() => match res {
                    Some(request) => {
                        let Request { kind, tx_response } = request;
                        let response = self.handle_request(kind).await?;
                        let _ = tx_response.send(response);
                    }
                    // breaks when all client handlers drop
                    None => break,
                },
                // rx_playback received an Over event
                else => break
            }
        }

        Ok(())
    }
}

pub async fn run(player_config: PlayerConfig, rx: ReceiverFromServer) -> Result<()> {
    let PlayerConfig {
        default_audio_device,
        music_dir,
        allowed_exts,
    } = player_config;
    let audio_backend = AudioBackend::new().with_default(default_audio_device.as_str())?;
    let database = {
        let (tx, rx) = oneshot::channel();
        rayon::spawn(move || {
            let _ = tx.send(Database::new(music_dir, allowed_exts));
        });
        rx.await?
    };
    let mut player = Player::new(database, audio_backend);

    player.run(rx).await
}
