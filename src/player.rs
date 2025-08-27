use anyhow::{Result, bail};
use serde_json::Map;
use std::path::{Path, PathBuf};
use tokio::sync::{
    mpsc::{self as tokio_chan},
    oneshot,
};

use crate::{
    audio::Audio,
    config::PlayerConfig,
    database::Database,
    model::{
        queue::Queue,
        request::{self, Request, RequestKind},
        response::Response,
        song::{SongEvent, SongProxy},
    },
};

struct Player {
    queue: Queue,
    database: Database,
    audio: Audio,
    rx_request: tokio_chan::UnboundedReceiver<Request>,
    rx_event: tokio_chan::UnboundedReceiver<SongEvent>,
}

impl Player {
    fn move_next_until_playable(&mut self) {
        self.queue.add_current_to_history();
        while let Some(entry) = self.queue.move_next() {
            match self.audio.play(&entry.path) {
                Ok(_) => break,
                Err(e) => log::error!("playback error ({})", e),
            }
        }
    }

    fn move_prev_until_playable(&mut self) {
        while let Some(entry) = self.queue.move_prev() {
            match self.audio.play(&entry.path) {
                Ok(_) => break,
                Err(e) => log::error!("playback error ({})", e),
            }
        }
    }

    // database requests are blocking and (mostly) parallelizable,
    // so we send them to rayon's thread pool
    async fn db_request(&mut self, req: request::DbRequestKind) -> Response {
        let (tx, rx) = oneshot::channel();
        rayon::scope(|s| {
            s.spawn(|_| {
                use request::DbRequestKind;

                let response = match req {
                    DbRequestKind::Ls(args) => self.database.ls(args),
                    DbRequestKind::Metadata(args) => self.database.metadata(args),
                    DbRequestKind::Select(args) => self.database.select(args),
                    DbRequestKind::Unique(args) => self.database.unique(args),
                    DbRequestKind::Update => self.database.update(),
                };
                let _ = tx.send(response);
            });
        });

        rx.await.unwrap()
    }

    fn device_request(&mut self, req: request::DeviceRequestKind) -> Response {
        use request::{DeviceRequestKind, DisableArgs, EnableArgs};

        match req {
            DeviceRequestKind::Disable(args) => {
                let DisableArgs(device) = args;
                self.audio.disable_device(device).into()
            }
            DeviceRequestKind::Enable(args) => {
                let EnableArgs(device) = args;
                self.audio.enable_device(&device).into()
            }
            DeviceRequestKind::ListDevices => {
                let devices: Vec<_> = self
                    .audio
                    .list_devices()
                    .into_iter()
                    .map(|(d, enabled)| {
                        let mut map = Map::new();
                        map.insert("device".into(), d.into());
                        map.insert("enabled".into(), enabled.into());

                        map
                    })
                    .collect();

                Response::new_ok().with_item("devices", &devices)
            }
        }
    }

    async fn playback_request(&mut self, req: request::PlaybackRequestKind) -> Response {
        use request::{ChangeVolumeArgs, PlaybackRequestKind, SeekArgs, SetVolumeArgs, SpeedArgs};

        match req {
            PlaybackRequestKind::ChangeVolume(args) => {
                let ChangeVolumeArgs(volume) = args;
                self.audio.change_volume(volume);

                Response::new_ok()
            }
            PlaybackRequestKind::Gapless => {
                self.audio.toggle_gapless();
                Response::new_ok()
            }
            PlaybackRequestKind::Pause => self.audio.pause().await.into(),
            PlaybackRequestKind::Resume => self.audio.resume().into(),
            PlaybackRequestKind::Seek(args) => {
                let SeekArgs(secs) = args;
                self.audio.seek(secs);

                Response::new_ok()
            }
            PlaybackRequestKind::SetVolume(args) => {
                let SetVolumeArgs(volume) = args;
                self.audio.set_volume(volume);

                Response::new_ok()
            }
            PlaybackRequestKind::Speed(args) => {
                let SpeedArgs(speed) = args;
                self.audio.set_speed(speed);

                Response::new_ok()
            }
            PlaybackRequestKind::Stop => {
                self.queue.reset_pos();
                self.audio.stop();

                Response::new_ok()
            }
            PlaybackRequestKind::Toggle => self.audio.toggle().await.into(),
        }
    }

    fn queue_request(&mut self, req: request::QueueRequestKind) -> Response {
        use request::{AddArgs, PlayArgs, QueueRequestKind, RemoveArgs};

        match req {
            QueueRequestKind::Add(args) => {
                let AddArgs(paths, insert_pos) = args;
                let mut not_found = Vec::new();
                for (offset, path) in paths.into_iter().enumerate() {
                    match self.database.try_to_abs_path(&path) {
                        Some(abs_path) => match insert_pos {
                            Some(pos) => self.queue.add(&abs_path, Some(pos + offset)),
                            None => self.queue.add(&abs_path, None),
                        },
                        None => {
                            not_found.push(path.to_string_lossy().into_owned());
                        }
                    }
                }

                if not_found.is_empty() {
                    Response::new_ok()
                } else {
                    Response::new_err(format!("file(s) `{}` not found", not_found.join(",")))
                }
            }
            QueueRequestKind::Clear => {
                self.queue.clear();
                self.audio.stop();

                Response::new_ok()
            }
            QueueRequestKind::Next => {
                self.move_next_until_playable();
                if self.queue.current().is_none() {
                    self.audio.stop();
                }

                Response::new_ok()
            }
            QueueRequestKind::Play(args) => {
                let PlayArgs(id) = args;
                match self.queue.move_to(id) {
                    Some(entry) => {
                        let res = self.audio.play(&entry.path);
                        if res.is_err() {
                            self.queue.reset_pos();
                            self.audio.stop();
                        }
                        res.into()
                    }
                    None => Response::new_err(format!("song with queue id `{}` not found", id)),
                }
            }
            QueueRequestKind::Previous => {
                self.move_prev_until_playable();
                if self.queue.current().is_none() {
                    self.audio.stop();
                }

                Response::new_ok()
            }
            QueueRequestKind::Random => {
                self.queue.start_random();
                Response::new_ok()
            }
            QueueRequestKind::Remove(args) => {
                let RemoveArgs(queue_ids) = args;
                for queue_id in queue_ids {
                    if self.queue.remove(queue_id) {
                        self.audio.stop();
                    }
                }

                Response::new_ok()
            }
            QueueRequestKind::Sequential => {
                self.queue.start_sequential();
                Response::new_ok()
            }
            QueueRequestKind::Single => {
                self.queue.start_single();
                Response::new_ok()
            }
        }
    }

    async fn status_request(&self, req: request::StatusRequestKind) -> Response {
        use request::StatusRequestKind;

        match req {
            StatusRequestKind::Queue => {
                let queue: Vec<_> = self
                    .queue
                    .as_inner()
                    .iter()
                    .map(|entry| {
                        let mut map = Map::new();
                        map.insert("id".into(), entry.id.into());
                        map.insert("path".into(), entry.path.to_string_lossy().into());

                        map
                    })
                    .collect();

                Response::new_ok().with_item("queue", &queue)
            }
            StatusRequestKind::State => {
                let mut response = Response::new_ok()
                    .with_item("gapless", &self.audio.gapless())
                    .with_item("mode", &self.queue.mode())
                    .with_item("state", &self.audio.state())
                    .with_item("speed", &self.audio.speed())
                    .with_item("volume", &self.audio.volume());
                if let Some(timer) = self.audio.playback_timer().await {
                    response = response
                        .with_item("elapsed", &timer.elapsed)
                        .with_item("out_of", &timer.out_of);
                }
                if let Some(current) = self.queue.current() {
                    response = response
                        .with_item("id", &current.id)
                        .with_item("path", &current.path.to_string_lossy());
                }

                response
            }
        }
    }

    async fn handle_request(&mut self, req: RequestKind) -> Response {
        match req {
            RequestKind::Db(req) => self.db_request(req).await,
            RequestKind::Device(req) => self.device_request(req),
            RequestKind::Playback(req) => self.playback_request(req).await,
            RequestKind::Queue(req) => self.queue_request(req),
            RequestKind::Status(req) => self.status_request(req).await,
        }
    }

    pub fn new(
        database: Database,
        audio: Audio,
        rx_request: tokio_chan::UnboundedReceiver<Request>,
        rx_event: tokio_chan::UnboundedReceiver<SongEvent>,
    ) -> Self {
        Self {
            queue: Queue::new(),
            database,
            audio,
            rx_request,
            rx_event,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            tokio::select! {
                res = self.rx_request.recv() => match res {
                    Some(request) => {
                        let Request { kind, tx_response } = request;
                        let response = self.handle_request(kind).await;
                        let _ = tx_response.send(response);
                    }
                    // breaks when all client handlers go out of scope
                    None => break Ok(()),
                },
                Some(event) = self.rx_event.recv() => match event {
                    SongEvent::Over => {
                        self.move_next_until_playable();
                        if self.queue.current().is_none() {
                            self.queue.reset_pos();
                            self.audio.stop();
                        }
                    }
                },
                else => break Ok(())
            }
        }
    }
}

pub async fn run(
    config: PlayerConfig,
    rx_request: tokio_chan::UnboundedReceiver<Request>,
) -> Result<()> {
    let PlayerConfig {
        audio_device,
        music_dir,
        allowed_exts,
    } = config;

    let (tx_event, rx_event) = tokio_chan::unbounded_channel();
    let audio = Audio::new(tx_event).with_default(audio_device.as_ref())?;
    // creating the db is blocking and parallelizable,
    // so we delegate it to rayon's thread pool
    let database = {
        let (tx, rx) = oneshot::channel();
        rayon::spawn(move || {
            let _ = tx.send(Database::from_dir(music_dir, &allowed_exts));
        });
        rx.await?
    }?;
    let mut player = Player::new(database, audio, rx_request, rx_event);

    player.run().await
}
