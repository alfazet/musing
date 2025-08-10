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
    error::MyError,
    model::{
        queue::Queue,
        request::*,
        response::Response,
        song::{PlayerSong, Song},
    },
};

type ReceiverFromServer = mpsc::UnboundedReceiver<Request>;
type ReceiverSongOver = mpsc::Receiver<()>;

struct Player {
    queue: Queue,
    database: Database,
    audio: Audio,
    rx_over: ReceiverSongOver,
}

impl Player {
    fn song_by_db_id(&self, db_id: u32) -> Result<&Song> {
        let Some(song) = self.database.song_by_id(db_id) else {
            bail!(MyError::Database(format!(
                "Song with db_id `{}` not found in the database",
                db_id
            )));
        };

        Ok(song)
    }

    fn play(&mut self, db_id: u32) -> Result<()> {
        self.queue.add_current_to_history();
        let song = self.song_by_db_id(db_id)?.try_into()?;
        let (tx_over, rx_over) = mpsc::channel(1);
        self.rx_over = rx_over;
        self.audio.start(song, tx_over)?;

        Ok(())
    }

    // database requests are blocking and (mostly) parallelizable,
    // so we send them to rayon's thread pool
    async fn db_request(&mut self, req: DbRequestKind) -> Response {
        let (tx, rx) = oneshot::channel();
        rayon::scope(|s| {
            s.spawn(|_| {
                let response = match req {
                    DbRequestKind::Update => self.database.update(),
                    DbRequestKind::Select(args) => self.database.select_outer(args),
                    DbRequestKind::Metadata(args) => self.database.metadata(args),
                    DbRequestKind::Unique(args) => self.database.unique(args),
                };
                let _ = tx.send(response);
            });
        });

        rx.await.unwrap()
    }

    fn playback_request(&mut self, req: PlaybackRequestKind) -> Response {
        match req {
            PlaybackRequestKind::Pause => self.audio.pause().into(),
            PlaybackRequestKind::Resume => self.audio.resume().into(),
            PlaybackRequestKind::Seek(args) => {
                let SeekArgs(secs) = args;
                self.audio.seek(secs);

                Response::new_ok()
            }
            PlaybackRequestKind::Stop => {
                self.queue.reset_pos();
                self.audio.stop().into()
            }
            PlaybackRequestKind::Toggle => self.audio.toggle().into(),
            PlaybackRequestKind::Volume(args) => {
                let VolumeArgs(volume) = args;
                match volume {
                    Volume::Change(x) => self.audio.change_volume(x),
                    Volume::Set(x) => self.audio.set_volume(x),
                }

                Response::new_ok()
            }
            _ => todo!(),
        }
    }

    fn queue_request(&mut self, req: QueueRequestKind) -> Response {
        match req {
            QueueRequestKind::Add(args) => {
                let AddArgs(db_ids, insert_pos) = args;
                for (offset, db_id) in db_ids.into_iter().enumerate() {
                    match self.song_by_db_id(db_id) {
                        Ok(_) => match insert_pos {
                            Some(pos) => self.queue.add(db_id, Some(pos + offset)),
                            None => self.queue.add(db_id, None),
                        },
                        Err(e) => return Response::new_err(e.to_string()),
                    }
                }

                Response::new_ok()
            }
            QueueRequestKind::Play(args) => {
                let PlayArgs(queue_id) = args;
                match self.queue.move_to(queue_id) {
                    Some(entry) => self.play(entry.db_id).into(),
                    None => Response::new_err(format!(
                        "Song with queue_id `{}` not found in the queue",
                        queue_id
                    )),
                }
            }
            QueueRequestKind::Next => match self.queue.move_next() {
                Some(entry) => self.play(entry.db_id).into(),
                None => self.audio.stop().into(),
            },
            QueueRequestKind::Previous => match self.queue.move_prev() {
                Some(entry) => self.play(entry.db_id).into(),
                None => self.audio.stop().into(),
            },
            _ => todo!(),
        }
    }

    fn status_request(&self, req: StatusRequestKind) -> Response {
        match req {
            StatusRequestKind::Current => match self.queue.current() {
                Some(entry) => Response::new_ok()
                    .with_item("db_id".into(), &entry.db_id)
                    .with_item("queue_id".into(), &entry.queue_id),
                None => Response::new_err("No song is playing right now".into()),
            },
            StatusRequestKind::Elapsed => {
                Response::new_ok().with_item("elapsed".into(), &self.audio.elapsed())
            }
            StatusRequestKind::Volume => {
                Response::new_ok().with_item("volume".into(), &self.audio.volume())
            }
            _ => todo!(),
        }
    }

    async fn handle_request(&mut self, req: RequestKind) -> Response {
        match req {
            RequestKind::Db(req) => self.db_request(req).await,
            RequestKind::Playback(req) => self.playback_request(req),
            RequestKind::Queue(req) => self.queue_request(req),
            RequestKind::Status(req) => self.status_request(req),
        }
    }

    pub fn new(database: Database, audio: Audio) -> Self {
        let (_, rx_over) = mpsc::channel(1);
        Self {
            queue: Queue::default(),
            database,
            audio,
            rx_over,
        }
    }

    pub async fn run(&mut self, mut rx: ReceiverFromServer) {
        loop {
            tokio::select! {
                res = rx.recv() => match res {
                    Some(request) => {
                        let Request { kind, tx_response } = request;
                        let response = self.handle_request(kind).await;
                        let _ = tx_response.send(response);
                    }
                    // breaks when all client handlers drop
                    None => break,
                },
                Some(_) = self.rx_over.recv() => {
                    match self.queue.move_next() {
                        Some(entry) => {
                            let _ = self.play(entry.db_id);
                        }
                        None => {
                            self.queue.reset_pos();
                            let _ = self.audio.stop();
                        }
                    }
                },
                else => break
            }
        }
    }
}

pub async fn run(player_config: PlayerConfig, rx: ReceiverFromServer) -> Result<()> {
    let PlayerConfig {
        default_audio_device,
        music_dir,
        allowed_exts,
    } = player_config;
    let audio = Audio::new().with_default(default_audio_device.as_str())?;
    let database = {
        let (tx, rx) = oneshot::channel();
        rayon::spawn(move || {
            let _ = tx.send(Database::from_dir(music_dir, allowed_exts));
        });
        rx.await?
    }?;
    let mut player = Player::new(database, audio);
    player.run(rx).await;

    Ok(())
}
