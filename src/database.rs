use anyhow::{Result, bail};
use jwalk::WalkDir;
use rayon::prelude::*;
use serde_json::Map;
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs::File,
    hash::{DefaultHasher, Hash, Hasher},
    io::{BufReader, prelude::*},
    iter::{FromIterator, IntoIterator, Iterator},
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{
    constants,
    model::{
        request::{LsArgs, MetadataArgs, SelectArgs, UniqueArgs},
        response::Response,
        song::{Metadata, Song, SongProxy},
    },
};

#[derive(Clone)]
struct DataRow {
    song: Song,
    pending_delete: bool,
}

pub struct Database {
    music_dir: PathBuf,
    allowed_exts: Vec<String>,
    data_rows: Vec<DataRow>,
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

    pub fn from_dir(
        music_dir: impl AsRef<Path> + Into<PathBuf>,
        allowed_exts: &[String],
    ) -> Result<Self> {
        let files = db_utils::walk_dir(music_dir.as_ref(), SystemTime::UNIX_EPOCH, &allowed_exts)?;
        let data_rows = Self::to_data_rows(&files);
        let last_update = SystemTime::now();

        Ok(Self {
            music_dir: music_dir.into(),
            allowed_exts: allowed_exts.to_vec(),
            data_rows,
            last_update,
        })
    }

    // tries to find the song by the given (relative or absolute) path
    pub fn try_to_abs_path(&self, path: impl AsRef<Path>) -> Option<PathBuf> {
        let abs_path = db_utils::to_abs_path(&self.music_dir, &path.as_ref());
        db_utils::binary_search_by_path(&self.data_rows, &abs_path).map(|_| abs_path)
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
        let values: Vec<_> = paths
            .into_par_iter()
            .map(|path| {
                let abs_path = db_utils::to_abs_path(&self.music_dir, path);
                db_utils::binary_search_by_path(&self.data_rows, abs_path).map(|i| {
                    let data = tags.iter().map(|tag| {
                        let value = self.data_rows[i].song.metadata.get(tag).into();
                        (tag.to_string(), value)
                    });

                    Some(Map::from_iter(data))
                })
            })
            .collect();

        Response::new_ok().with_item("values", &values)
    }

    // get paths of songs matching `filter_expr`, sorted by the comparators in `sort_by`
    pub fn select(&self, SelectArgs(filter_expr, sort_by): SelectArgs) -> Response {
        let compare = |lhs: &Metadata, rhs: &Metadata| -> Ordering {
            sort_by
                .iter()
                .map(|cmp| cmp.cmp(lhs, rhs))
                .find(|&ord| ord != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        };

        let mut filtered: Vec<_> = self
            .data_rows
            .par_iter()
            .filter(|row| filter_expr.evaluate(&row.song))
            .collect();
        filtered.par_sort_unstable_by(|lhs, rhs| compare(&lhs.song.metadata, &rhs.song.metadata));
        let paths: Vec<_> = filtered.into_par_iter().map(|row| &row.song.path).collect();

        Response::new_ok().with_item("paths", &paths)
    }

    // get unique values of `tag` among songs matching `filter_expr`, grouped by tags in `group_by`
    pub fn unique(&self, UniqueArgs(tag, filter_expr, group_by): UniqueArgs) -> Response {
        let mut groups = HashMap::new();
        let filtered = self
            .data_rows
            .iter()
            .filter(|row| filter_expr.evaluate(&row.song))
            .map(|row| &row.song.metadata);
        for meta in filtered {
            let combination: Vec<_> = group_by
                .iter()
                .map(|group_tag| meta.get(group_tag))
                .collect();
            groups
                .entry(combination)
                .and_modify(|set: &mut HashSet<_>| {
                    set.insert(meta.get(&tag));
                })
                .or_insert([meta.get(&tag)].into());
        }
        let values: Vec<_> = groups
            .into_iter()
            .map(|(combination, values)| {
                let data = group_by
                    .iter()
                    .map(|tag_key| tag_key.to_string())
                    .zip(combination.into_iter().map(|value| value.into()));
                let mut json_map = Map::from_iter(data);
                let values: Vec<_> = values.into_iter().collect();
                json_map.insert(tag.to_string(), values.into());

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
            return match Self::from_dir(&self.music_dir, &self.allowed_exts) {
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

        let added_songs =
            match db_utils::walk_dir(&self.music_dir, self.last_update, &self.allowed_exts) {
                Ok(added_songs) => added_songs,
                Err(e) => return Response::new_err(e.to_string()),
            };
        let mut added_data_rows = Self::to_data_rows(&added_songs);
        added_data_rows.par_sort_unstable_by(|lhs, rhs| lhs.song.path.cmp(&rhs.song.path));
        // merge old rows with new ones without destroying the order
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
        allowed_exts: &[String],
    ) -> Result<Vec<PathBuf>> {
        let is_ok = move |path: &Path| -> bool {
            if let Some(ext) = path.extension().and_then(|ext| ext.to_str())
                && allowed_exts.iter().any(|allowed_ext| allowed_ext == ext)
                && let Ok(creation_time) = path.metadata().and_then(|m| m.created())
            {
                return creation_time >= timestamp;
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
            for line in stream.lines() {
                if let Ok(line) = &line {
                    let abs_path = db_utils::to_abs_path(&root_dir, Path::new(line));
                    ignored.insert(abs_path);
                }
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
