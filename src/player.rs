use anyhow::{Result, bail};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
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

    pub async fn run(&mut self, default_audio_device: &str, rx: ReceiverFromServer) -> Result<()> {
        self.audio_backend.add_device(default_audio_device)?;
        self.audio_backend.enable_device(default_audio_device);

        Ok(())
    }
}

pub async fn run(player_config: PlayerConfig, rx: ReceiverFromServer) {
    let PlayerConfig {
        default_audio_device,
        music_dir,
        allowed_exts,
    } = player_config;
    let database = Database::new(&music_dir, allowed_exts);
    let audio_backend = AudioBackend::new();
    let mut player = Player::new(database, audio_backend);
    if let Err(e) = player.run(default_audio_device.as_str(), rx).await {
        log::error!("{}", e);
    }
}
