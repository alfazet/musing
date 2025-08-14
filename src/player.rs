use anyhow::{Result, bail};
use std::pin::Pin;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    task,
};

use crate::{
    audio::Audio,
    config::PlayerConfig,
    database::Database,
    model::{
        queue::Queue,
        request::{ReceiverRequest, Request, RequestKind},
        response::Response,
        song::Song,
    },
};

pub async fn run(config: PlayerConfig, rx_request: ReceiverRequest) -> Result<()> {
    let PlayerConfig {
        default_audio_device,
        music_dir,
        allowed_exts,
    } = config;
    let audio = Audio::new().with_default(default_audio_device.as_str())?;
    let database = {
        let (tx, rx) = oneshot::channel();
        rayon::spawn(move || {
            let _ = tx.send(Database::from_dir(&music_dir, &allowed_exts));
        });
        rx.await?
    }?;
    let mut player = Player::new(database, audio);

    player.run(rx_request).await
}
