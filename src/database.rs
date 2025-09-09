use anyhow::{Result, bail};
use jwalk::WalkDir;
use rayon::prelude::*;
use serde_json::Map;
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, prelude::*},
    iter::{FromIterator, IntoIterator, Iterator},
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{
    constants,
    model::{
        queue::Entry,
        request::{LsArgs, MetadataArgs, SelectArgs},
        response::Response,
        song::{Metadata, Song},
    },
};

#[derive(Clone, Debug)]
struct DataRow {
    song: Song,
    pending_delete: bool,
}

#[derive(Debug)]
pub struct Database {
    music_dir: PathBuf,
    playlist_dir: PathBuf,
    data_rows: Vec<DataRow>,
    playlists: HashSet<PathBuf>,
    last_update: SystemTime,
}

impl Database {
    fn to_data_rows(files: &[PathBuf]) -> Vec<DataRow> {
        let mut rows: Vec<DataRow> = files
            .par_iter()
            .filter_map(move |path| match Song::try_new(path) {
                Ok(song) => Some(DataRow {
                    song,
                    pending_delete: false,
                }),
                Err(e) => {
                    log::error!("decoding error ({}, file `{}`)", e, path.to_string_lossy());
                    None
                }
            })
            .collect();
        rows.par_sort_unstable_by(|lhs, rhs| lhs.song.path.cmp(&rhs.song.path));

        rows
    }

    fn build_playlists(playlist_dir: impl AsRef<Path> + Into<PathBuf>) -> HashSet<PathBuf> {
        let playlist_files = db_utils::walk_dir(
            playlist_dir.as_ref(),
            SystemTime::UNIX_EPOCH,
            &constants::DEFAULT_PLAYLIST_EXTS,
        )
        .unwrap_or_default();

        playlist_files.into_iter().collect()
    }

    pub fn try_new(
        music_dir: impl AsRef<Path> + Into<PathBuf>,
        playlist_dir: Option<&PathBuf>,
    ) -> Result<Self> {
        let files = db_utils::walk_dir(
            music_dir.as_ref(),
            SystemTime::UNIX_EPOCH,
            &constants::DEFAULT_ALLOWED_EXTS,
        )?;
        let data_rows = Self::to_data_rows(&files);
        let default_playlist_dir = music_dir
            .as_ref()
            .join(Path::new(constants::DEFAULT_PLAYLIST_DIR));
        let playlist_dir = playlist_dir.unwrap_or(&default_playlist_dir);
        let playlists = Self::build_playlists(playlist_dir);
        let last_update = SystemTime::now();

        Ok(Self {
            music_dir: music_dir.into(),
            playlist_dir: playlist_dir.into(),
            data_rows,
            playlists,
            last_update,
        })
    }

    // tries to find the song by the given (relative or absolute) path
    pub fn try_to_abs_path(&self, path: impl AsRef<Path>) -> Option<PathBuf> {
        let abs_path = db_utils::to_abs_path(&self.music_dir, path.as_ref());
        db_utils::binary_search_by_path(&self.data_rows, &abs_path).map(|_| abs_path)
    }

    pub fn playlists(&self) -> &HashSet<PathBuf> {
        &self.playlists
    }

    pub fn load_playlist(&self, path: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
        let abs_path = db_utils::to_abs_path(&self.playlist_dir, path.as_ref());
        let file = File::open(&abs_path)?;
        let stream = BufReader::new(file);
        // lines starting with `#` are comments in m3u files
        let playlist: Vec<_> = stream
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.starts_with("#"))
            .map(|l| l.into())
            .collect();

        Ok(playlist)
    }

    pub fn add_to_playlist(
        &mut self,
        playlist_path: impl AsRef<Path>,
        song_path: impl AsRef<Path> + Into<PathBuf>,
    ) -> Response {
        let Some(abs_song_path) = self.try_to_abs_path(&song_path) else {
            return Response::new_err(format!(
                "song `{}` not found in the database",
                &song_path.as_ref().to_string_lossy()
            ));
        };
        let abs_playlist_path = db_utils::to_abs_path(&self.playlist_dir, playlist_path.as_ref());
        let Ok(mut playlist_file) = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&abs_playlist_path)
        else {
            return Response::new_err(format!(
                "playlist `{}` not found",
                abs_playlist_path.to_string_lossy()
            ));
        };
        // we use relative song paths in playlist files, since that makes it cross-platform
        // (absolute paths differ between Unix and Windows, relative ones don't)
        //
        // this unwrap is fine because we know that the path is absolute and
        // points to somewhere within the music_dir
        let rel_song_path = abs_song_path.strip_prefix(&self.music_dir).unwrap();

        playlist_file
            .write_all(rel_song_path.as_os_str().as_encoded_bytes())
            .and_then(|_| playlist_file.write_all(b"\n"))
            .map_err(|e| e.into())
            .into()
    }

    pub fn remove_from_playlist(
        &mut self,
        playlist_path: impl AsRef<Path>,
        pos: usize,
    ) -> Response {
        let abs_playlist_path = db_utils::to_abs_path(&self.playlist_dir, playlist_path.as_ref());
        let Ok(content) = fs::read_to_string(&abs_playlist_path) else {
            return Response::new_err(format!(
                "playlist `{}` not found",
                abs_playlist_path.to_string_lossy()
            ));
        };
        let new_content = content
            .lines()
            .enumerate()
            .filter_map(|(i, line)| if i != pos { Some(line) } else { None })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";

        fs::write(&abs_playlist_path, new_content)
            .map_err(|e| e.into())
            .into()
    }

    pub fn save_as_playlist(&self, path: impl AsRef<Path>, entries: &[Entry]) -> Response {
        let abs_path = db_utils::to_abs_path(&self.playlist_dir, path.as_ref());
        let Ok(file) = File::create(&abs_path) else {
            return Response::new_err(format!(
                "couldn't open file `{}`",
                abs_path.to_string_lossy()
            ));
        };
        let mut stream = BufWriter::new(file);
        for entry in entries {
            // this unwrap is fine because we know that the path is absolute and
            // points to somewhere within the music_dir
            let rel_path = entry.path.strip_prefix(&self.music_dir).unwrap();
            if let Err(e) = stream
                .write_all(rel_path.as_os_str().as_encoded_bytes())
                .and_then(|_| stream.write_all(b"\n"))
            {
                return Response::new_err(e.to_string());
            }
        }
        let _ = stream.flush();

        Response::new_ok()
    }

    // get paths of songs located in `path`
    // allows to use musing with untagged music collections
    // `path` can be relative (to the provided music dir) or absolute
    // if `path` points to a single file, ls returns the path of that file
    pub fn ls(&self, LsArgs(path): LsArgs) -> Response {
        let abs_path = db_utils::to_abs_path(&self.music_dir, &path);
        match abs_path.metadata() {
            Ok(meta) => {
                let paths = if meta.is_file() {
                    match self
                        .data_rows
                        .par_iter()
                        .find_any(|&row| row.song.path == abs_path)
                    {
                        Some(row) => vec![&row.song.path],
                        None => vec![],
                    }
                } else {
                    self.data_rows
                        .par_iter()
                        .filter_map(|row| {
                            if let Some(parent) = row.song.path.parent()
                                && parent == abs_path
                            {
                                Some(&row.song.path)
                            } else {
                                None
                            }
                        })
                        .collect()
                };

                Response::new_ok().with_item("paths", &paths)
            }
            Err(e) => Response::new_err(e.to_string()),
        }
    }

    // get values of `tags` for songs in `paths`
    pub fn metadata(&self, MetadataArgs(paths, tags): MetadataArgs) -> Response {
        let metadata: Vec<_> = paths
            .into_par_iter()
            .map(|path| {
                let abs_path = db_utils::to_abs_path(&self.music_dir, path);
                db_utils::binary_search_by_path(&self.data_rows, abs_path).map(|i| {
                    let data = tags.iter().map(|tag| {
                        let value = self.data_rows[i].song.metadata.get(tag).into();
                        (tag.to_string(), value)
                    });

                    // additional non-standard tags that clients
                    // will generally want to use
                    let mut map = Map::from_iter(data);
                    map.insert(
                        "duration".to_string(),
                        self.data_rows[i]
                            .song
                            .duration
                            .map(|d| d.to_string())
                            .into(),
                    );

                    Some(map)
                })
            })
            .collect();

        Response::new_ok().with_item("metadata", &metadata)
    }

    // get paths of songs (together with their `tags` metadata), matching `filter_expr`
    // grouped by tags in `group_by` with each group sorted by tags in `sort_by`
    pub fn select(&self, SelectArgs(tags, filter_expr, group_by, sort_by): SelectArgs) -> Response {
        let compare = |lhs: &Metadata, rhs: &Metadata| -> Ordering {
            sort_by
                .iter()
                .map(|cmp| cmp.cmp(lhs, rhs))
                .find(|&ord| ord != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        };

        let mut groups = HashMap::new();
        let mut filtered: Vec<_> = self
            .data_rows
            .par_iter()
            .filter(|row| filter_expr.evaluate(&row.song))
            .collect();
        filtered.par_sort_unstable_by(|lhs, rhs| compare(&lhs.song.metadata, &rhs.song.metadata));

        for row in filtered {
            let song = &row.song;
            let combination: Vec<_> = group_by
                .iter()
                .map(|group_tag| song.metadata.get(group_tag))
                .collect();

            let make_song_data = || {
                let mut song_data: Vec<_> = tags
                    .iter()
                    .map(|tag| song.metadata.get(tag).map(String::from))
                    .collect();
                song_data.push(Some(song.path.to_string_lossy().into_owned()));

                song_data
            };
            groups
                .entry(combination)
                .and_modify(|songs: &mut Vec<_>| {
                    songs.push(make_song_data());
                })
                .or_insert([make_song_data()].into());
        }
        let values: Vec<_> = groups
            .into_iter()
            .map(|(combination, values)| {
                let group_by_data = group_by
                    .iter()
                    .map(|tag_key| tag_key.to_string())
                    .zip(combination.into_iter().map(|value| value.into()));
                let mut json_map = Map::from_iter(group_by_data);
                json_map.insert("data".into(), values.into());

                json_map
            })
            .collect();

        Response::new_ok().with_item("values", &values)
    }

    pub fn update(&mut self) -> Response {
        // do a full rescan if the ignore file changed recently
        if let Ok(ignore_mod_time) = self
            .music_dir
            .join(Path::new(constants::DEFAULT_IGNORE_FILE))
            .metadata()
            .and_then(|m| m.modified())
            && ignore_mod_time >= self.last_update
        {
            return match Self::try_new(&self.music_dir, Some(&self.playlist_dir)) {
                Ok(db) => {
                    let n_removed = self.data_rows.len();
                    *self = db;

                    Response::new_ok()
                        .with_item("added_songs", &self.data_rows.len())
                        .with_item("removed_songs", &n_removed)
                }
                Err(e) => Response::new_err(e.to_string()),
            };
        }

        self.data_rows.par_iter_mut().for_each(|row| {
            if let Ok(mod_time) = row.song.path.metadata().and_then(|m| m.modified()) {
                if mod_time >= self.last_update
                    && let Ok(song) = Song::try_new(&row.song.path)
                {
                    row.song = song;
                }
            } else {
                // delete unreadable songs
                row.pending_delete = true;
            }
        });
        let old_len = self.data_rows.len();
        self.data_rows.retain(|row| !row.pending_delete);
        let n_removed = old_len - self.data_rows.len();

        let added_songs = match db_utils::walk_dir(
            &self.music_dir,
            self.last_update,
            &constants::DEFAULT_ALLOWED_EXTS,
        ) {
            Ok(added_songs) => added_songs,
            Err(e) => return Response::new_err(e.to_string()),
        };
        let mut added_data_rows = Self::to_data_rows(&added_songs);
        added_data_rows.par_sort_unstable_by(|lhs, rhs| lhs.song.path.cmp(&rhs.song.path));
        // merge old rows with new ones while keeping the sorted order
        let mut new_data_rows = Vec::with_capacity(self.data_rows.len() + added_data_rows.len());
        {
            let mut drain_old = self.data_rows.drain(..).peekable();
            let mut drain_new = added_data_rows.drain(..).peekable();
            while let (Some(a), Some(b)) = (drain_old.peek(), drain_new.peek()) {
                if a.song.path < b.song.path {
                    let a = drain_old.next().unwrap();
                    new_data_rows.push(a);
                } else {
                    let b = drain_new.next().unwrap();
                    new_data_rows.push(b);
                }
            }
            for a in drain_old {
                new_data_rows.push(a);
            }
            for b in drain_new {
                new_data_rows.push(b);
            }
        }
        self.data_rows = new_data_rows;
        self.playlists = Self::build_playlists(&self.playlist_dir);
        self.last_update = SystemTime::now();

        Response::new_ok()
            .with_item("added_songs", &added_songs.len())
            .with_item("removed_songs", &n_removed)
    }
}

mod db_utils {
    use super::*;

    pub fn to_abs_path<S, T>(root_dir: S, path: T) -> PathBuf
    where
        S: AsRef<Path>,
        T: AsRef<Path> + Into<PathBuf>,
    {
        if path.as_ref().is_absolute() {
            path.into()
        } else {
            root_dir.as_ref().join(path)
        }
    }

    pub fn binary_search_by_path(rows: &[DataRow], path: impl AsRef<Path>) -> Option<usize> {
        let n = rows.len();
        let (mut i, mut step) = (0, n / 2);
        while step >= 1 {
            while i + step < n && rows[i + step].song.path <= path.as_ref() {
                i += step;
            }
            step /= 2;
        }

        (rows[i].song.path == path.as_ref()).then_some(i)
    }

    // returns absolute paths
    pub fn walk_dir(
        root_dir: impl AsRef<Path>,
        timestamp: SystemTime,
        allowed_exts: &HashSet<String>,
    ) -> Result<Vec<PathBuf>> {
        let is_ok = move |path: &Path| -> bool {
            if let Some(ext) = path.extension().and_then(|ext| ext.to_str())
                && allowed_exts.contains(ext)
                && let Ok(mod_time) = path.metadata().and_then(|m| m.modified())
            {
                return mod_time >= timestamp;
            }

            false
        };

        if !root_dir.as_ref().exists() {
            bail!(format!(
                "directory `{}` doesn't exist",
                root_dir.as_ref().to_string_lossy()
            ));
        }
        let mut ignored = HashSet::new();
        if let Ok(file) = File::open(root_dir.as_ref().join(constants::DEFAULT_IGNORE_FILE)) {
            let stream = BufReader::new(file);
            for line in stream.lines().map_while(Result::ok) {
                let abs_path = db_utils::to_abs_path(&root_dir, Path::new(&line));
                ignored.insert(abs_path);
            }
        }
        let list = WalkDir::new(root_dir)
            .process_read_dir(move |_, _, _, children| {
                children.retain(|entry| {
                    entry
                        .as_ref()
                        .map(|e| !ignored.contains(&*(e.parent_path)))
                        .unwrap_or(false)
                });
            })
            .into_iter()
            .filter_map(|entry| {
                if let Ok(entry) = entry
                    && entry.file_type.is_file()
                    && let Ok(full_path) = dunce::canonicalize(entry.path())
                    && is_ok(&full_path)
                {
                    return Some(full_path);
                }
                None
            })
            .collect();

        Ok(list)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn walk_dir_with_ignore() {
        use std::fs;

        let tmp = std::env::temp_dir();
        let dir = tmp.join(format!(
            "musing_test_{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        ));

        let n = 10;
        let _ = fs::create_dir(&dir);
        for i in 1..=n {
            let _ = File::create(dir.join(format!("song{}.xyz", i)));
        }
        let _ = fs::create_dir(dir.join("ok_dir"));
        for i in 1..=n {
            let _ = File::create(dir.join(format!("ok_dir/song_ok{}.xyz", i)));
        }
        let _ = fs::create_dir(dir.join("bad_dir"));
        for i in 1..=n {
            let _ = File::create(dir.join(format!("bad_dir/song_bad{}.xyz", i)));
        }

        let mut ignore = File::create(dir.join(constants::DEFAULT_IGNORE_FILE)).unwrap();
        let _ = ignore.write_all(b"bad_dir");
        let res = db_utils::walk_dir(&dir, SystemTime::UNIX_EPOCH, &HashSet::from(["xyz".into()]))
            .unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(res.len(), 20);
        assert!(
            res.iter()
                .all(|path| !path.to_string_lossy().contains("bad_dir"))
        );
    }
}
