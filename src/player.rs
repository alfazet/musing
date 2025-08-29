use anyhow::Result;
use serde_json::Map;
use std::path::PathBuf;
use tokio::sync::{
    broadcast,
    mpsc::{self as tokio_chan},
    oneshot,
};

use crate::{
    audio::Audio,
    config::PlayerConfig,
    database::Database,
    model::{
        decoder::{Speed, Volume},
        queue::Queue,
        request::{self, Request, RequestKind},
        response::Response,
        song::SongEvent,
    },
    state::{AudioState, PlayerState, State},
};

struct Player {
    audio: Audio,
    database: Database,
    queue: Queue,
    rx_event: tokio_chan::UnboundedReceiver<SongEvent>,
    rx_request: tokio_chan::UnboundedReceiver<Request>,
}

impl Player {
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
            DeviceRequestKind::Devices => {
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

    fn playlist_request(&mut self, req: request::PlaylistRequestKind) -> Response {
        use request::{
            AddToPlaylistArgs, FromFileArgs, ListSongsArgs, LoadArgs, PlaylistRequestKind,
            RemoveFromPlaylistArgs, SaveArgs,
        };

        match req {
            PlaylistRequestKind::AddToPlaylist(args) => {
                let AddToPlaylistArgs(playlist_path, song_path) = args;
                self.database.add_to_playlist(playlist_path, song_path)
            }
            PlaylistRequestKind::FromFile(args) => {
                let FromFileArgs(path) = args;
                self.database.add_playlist_from_file(path)
            }
            PlaylistRequestKind::ListSongs(args) => {
                let ListSongsArgs(path) = args;
                match self.database.load_playlist(&path) {
                    Some(playlist) => Response::new_ok().with_item("songs", &playlist.inner()),
                    None => Response::new_err(format!(
                        "playlist `{}` not found",
                        path.to_string_lossy()
                    )),
                }
            }
            PlaylistRequestKind::Load(args) => {
                let LoadArgs(path, range, pos) = args;
                match self.database.load_playlist(&path) {
                    Some(playlist) => {
                        let not_found = add_to_queue(
                            &self.database,
                            &mut self.queue,
                            playlist.inner(),
                            range,
                            pos,
                        );
                        if not_found.is_empty() {
                            Response::new_ok()
                        } else {
                            Response::new_err(format!(
                                "song(s) `{}` not found in the database",
                                not_found
                                    .into_iter()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .collect::<Vec<_>>()
                                    .join(",")
                            ))
                        }
                    }
                    None => Response::new_err(format!(
                        "playlist `{}` not found",
                        path.to_string_lossy()
                    )),
                }
            }
            PlaylistRequestKind::RemoveFromPlaylist(args) => {
                let RemoveFromPlaylistArgs(path, pos) = args;
                self.database.remove_from_playlist(path, pos)
            }
            PlaylistRequestKind::Save(args) => {
                let SaveArgs(path) = args;
                self.database.save_as_playlist(path, self.queue.inner())
            }
        }
    }

    fn queue_request(&mut self, req: request::QueueRequestKind) -> Response {
        use request::{AddToQueueArgs, PlayArgs, QueueRequestKind, RemoveFromQueueArgs};

        match req {
            QueueRequestKind::AddToQueue(args) => {
                let AddToQueueArgs(paths, pos) = args;
                let not_found = add_to_queue(&self.database, &mut self.queue, &paths, None, pos);

                if not_found.is_empty() {
                    Response::new_ok()
                } else {
                    Response::new_err(format!(
                        "file(s) `{}` not found in the database",
                        not_found
                            .into_iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join(",")
                    ))
                }
            }
            QueueRequestKind::Clear => {
                self.queue.clear();
                self.audio.stop();

                Response::new_ok()
            }
            QueueRequestKind::Next => {
                move_next_until_playable(&mut self.queue, &mut self.audio);
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
                move_prev_until_playable(&mut self.queue, &mut self.audio);
                if self.queue.current().is_none() {
                    self.audio.stop();
                }

                Response::new_ok()
            }
            QueueRequestKind::Random => {
                self.queue.start_random();
                Response::new_ok()
            }
            QueueRequestKind::RemoveFromQueue(args) => {
                let RemoveFromQueueArgs(queue_ids) = args;
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
            StatusRequestKind::Playlists => {
                let playlists = self.database.playlists();
                Response::new_ok().with_item("playlists", &playlists.keys().collect::<Vec<_>>())
            }
            StatusRequestKind::Queue => {
                let queue: Vec<_> = self
                    .queue
                    .inner()
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
            RequestKind::Playlist(req) => self.playlist_request(req),
            RequestKind::Queue(req) => self.queue_request(req),
            RequestKind::Status(req) => self.status_request(req).await,
        }
    }

    pub fn new(
        state: Option<PlayerState>,
        audio: Audio,
        database: Database,
        rx_event: tokio_chan::UnboundedReceiver<SongEvent>,
        rx_request: tokio_chan::UnboundedReceiver<Request>,
    ) -> Self {
        let queue = state.map(|s| s.queue).unwrap_or_default();

        Self {
            audio,
            database,
            queue,
            rx_event,
            rx_request,
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
                        move_next_until_playable(&mut self.queue, &mut self.audio);
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

    pub fn state(&self) -> State {
        let volume = Volume::from(self.audio.volume());
        let speed = Speed::from(self.audio.speed());
        let gapless = self.audio.gapless();
        let mut queue = self.queue.clone();
        queue.reset_pos();

        let audio_state = AudioState {
            volume,
            speed,
            gapless,
        };
        let player_state = PlayerState { queue };

        State {
            audio_state,
            player_state,
        }
    }
}

fn move_next_until_playable(queue: &mut Queue, audio: &mut Audio) {
    queue.add_current_to_history();
    while let Some(entry) = queue.move_next() {
        match audio.play(&entry.path) {
            Ok(_) => break,
            Err(e) => log::error!("playback error ({})", e),
        }
    }
}

fn move_prev_until_playable(queue: &mut Queue, audio: &mut Audio) {
    while let Some(entry) = queue.move_prev() {
        match audio.play(&entry.path) {
            Ok(_) => break,
            Err(e) => log::error!("playback error ({})", e),
        }
    }
}

// returns the songs which weren't be found
fn add_to_queue<'a>(
    database: &Database,
    queue: &mut Queue,
    paths: &'a [PathBuf],
    range: Option<(usize, usize)>,
    pos: Option<usize>,
) -> Vec<&'a PathBuf> {
    let mut not_found = Vec::new();
    let range = match range {
        Some((start, end)) => {
            let start = start.min(paths.len() - 1);
            let end = end.clamp(start, paths.len() - 1);

            start..=end
        }
        None => 0..=(paths.len() - 1),
    };
    for (offset, path) in paths[range].iter().enumerate() {
        match database.try_to_abs_path(path) {
            Some(abs_path) => match pos {
                Some(pos) => queue.add(&abs_path, Some(pos + offset)),
                None => queue.add(&abs_path, None),
            },
            None => {
                not_found.push(path);
            }
        }
    }

    not_found
}

pub async fn run(
    config: PlayerConfig,
    rx_request: tokio_chan::UnboundedReceiver<Request>,
    mut rx_shutdown: broadcast::Receiver<()>,
) -> Result<()> {
    let PlayerConfig {
        music_dir,
        state_file,
        audio_device,
        playlist_dir,
    } = config;
    let (player_state, audio_state) = match State::try_from_file(&state_file) {
        Ok(s) => (Some(s.player_state), Some(s.audio_state)),
        Err(e) => {
            log::error!("state file error ({})", e);
            (None, None)
        }
    };

    let (tx_event, rx_event) = tokio_chan::unbounded_channel();
    let audio = Audio::new(audio_state, tx_event).with_default(audio_device.as_ref())?;
    // creating the db is blocking and parallelizable,
    // so we delegate it to rayon's thread pool
    let database = {
        let (tx, rx) = oneshot::channel();
        rayon::spawn(move || {
            let _ = tx.send(Database::try_new(music_dir, playlist_dir.as_ref()));
        });
        rx.await?
    }?;
    let mut player = Player::new(player_state, audio, database, rx_event, rx_request);

    tokio::select! {
        res = player.run() => res,
        _ = rx_shutdown.recv() => {
            let state = player.state();
            state.save(state_file)?;

            Ok(())
        }
    }
}
