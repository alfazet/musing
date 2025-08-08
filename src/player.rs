use anyhow::{Result, bail};
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
    error::MyError,
    model::{queue::Queue, request::*, response::Response},
};

type ReceiverFromServer = mpsc::UnboundedReceiver<Request>;

struct Player {
    queue: Queue,
    database: Database,
    audio_backend: Audio,
}

impl Player {
    // database requests are blocking and (relatively) CPU-intensive,
    // so we send them to rayon's thread pool
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
            Kind::Db(db_request_kind) => self.db_request(db_request_kind).await,
            Kind::Add(args) => {
                let AddArgs(db_ids, insert_pos) = args;
                let last_id = self.database.last_id();
                for (offset, id) in db_ids.into_iter().enumerate() {
                    if id > last_id {
                        return Ok(Response::new_err(format!(
                            "Song with id `{}` not found",
                            id
                        )));
                    }
                    match insert_pos {
                        Some(pos) => self.queue.add(id, Some(pos + offset)),
                        None => self.queue.add(id, None),
                    }
                }

                /*
                for entry in self.queue.as_inner() {
                    println!(
                        "{:?}: {}",
                        entry,
                        self.database
                            .song_by_id(entry.db_id)
                            .unwrap()
                            .song_meta
                            .get(&crate::model::tag_key::TagKey::try_from("tracktitle").unwrap())
                            .unwrap()
                    );
                }
                */

                Ok(Response::new_ok())
            }
            Kind::Play(args) => {
                let PlayArgs(queue_id) = args;

                Ok(Response::new_ok())
            }
            _ => todo!(),
        }
    }

    pub fn new(database: Database, audio_backend: Audio) -> Self {
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
    let audio_backend = Audio::new().with_default(default_audio_device.as_str())?;
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
