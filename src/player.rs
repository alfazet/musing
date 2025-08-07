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
    pub fn new(database: Database, audio_backend: AudioBackend) -> Self {
        Self {
            queue: Queue::default(),
            database,
            audio_backend,
        }
    }

    async fn handle_request(&mut self, kind: RequestKind) -> Result<Response> {
        use RequestKind as Kind;

        let response = match kind {
            Kind::Update => self.database.update(),
            Kind::Select(args) => self.database.select_outer(args),
            Kind::Metadata(args) => self.database.metadata(args),
            Kind::Unique(args) => self.database.unique(args),
            _ => todo!(),
        };

        Ok(response)
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
    // TODO: speed up database operations with rayon
    let audio_backend = AudioBackend::new().with_default(default_audio_device.as_str())?;
    let database = task::spawn_blocking(move || Database::new(&music_dir, allowed_exts)).await?;
    let mut player = Player::new(database, audio_backend);

    player.run(rx).await
}
