use anyhow::{Result, bail};
use jwalk::WalkDir;
use rayon::prelude::*;
use serde_json::Map;
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    iter::{FromIterator, IntoIterator, Iterator},
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::model::{
    request::{LsArgs, MetadataArgs, SelectArgs, UniqueArgs},
    response::Response,
    song::{Metadata, Song, SongProxy},
};

struct DataRow {
    id: u32,
    song: Song,
    to_delete: bool,
}

pub struct Database {
    music_dir: PathBuf,
    allowed_exts: Vec<String>,
    data_rows: Vec<DataRow>,
    last_update: SystemTime,
}

impl Database {
    fn to_data_rows(music_dir: &Path, files: &[PathBuf], id_offset: u32) -> Vec<DataRow> {
        files
            .par_iter()
            .enumerate()
            .filter_map(
                move |(id, file)| match Song::try_new(music_dir, file.as_path()) {
                    Ok(song) => Some(DataRow {
                        id: id as u32 + id_offset + 1,
                        song,
                        to_delete: false,
                    }),
                    Err(e) => {
                        log::error!(
                            "could not read any audio from {} ({})",
                            file.to_string_lossy(),
                            e
                        );
                        None
                    }
                },
            )
            .collect()
    }

    pub fn from_dir(music_dir: &Path, allowed_exts: &[String]) -> Result<Self> {
        let files = db_utils::walk_dir(music_dir, SystemTime::UNIX_EPOCH, allowed_exts)?;
        let data_rows = Self::to_data_rows(music_dir, &files, 0);
        let last_update = SystemTime::now();

        Ok(Self {
            music_dir: music_dir.to_path_buf(),
            allowed_exts: allowed_exts.to_vec(),
            data_rows,
            last_update,
        })
    }

    pub fn song_by_id(&self, id: u32) -> Option<SongProxy> {
        let rel_path = self
            .data_rows
            .binary_search_by_key(&id, |row| row.id)
            .map(|i| &self.data_rows[i].song.path)
            .ok()?;

        Some(SongProxy {
            path: self.music_dir.join(rel_path),
        })
    }

    pub fn reset(&mut self) -> Response {
        match Self::from_dir(&self.music_dir, &self.allowed_exts) {
            Ok(db) => {
                *self = db;
                Response::new_ok()
            }
            Err(e) => Response::new_err(e.to_string()),
        }
    }

    // get ids of songs located in `path`
    // allows to use musing with untagged music collections
    // `path` can be relative (to the provided music dir) or absolute
    // if `path` points to a single file, ls returns the id of that file
    pub fn ls(&self, LsArgs(mut path): LsArgs) -> Response {
        if path.is_absolute() {
            if let Ok(rel_path) = path.strip_prefix(&self.music_dir).map(|p| p.to_path_buf()) {
                path = rel_path;
            } else {
                return Response::new_err(format!(
                    "path `{}` points outside the music directory",
                    path.to_string_lossy()
                ));
            }
        }
        // at this point path is relative to the music dir
        let abs_path = self.music_dir.join(&path);
        match abs_path.metadata() {
            Ok(meta) => {
                let ids = if meta.is_file() {
                    match self
                        .data_rows
                        .par_iter()
                        .find_any(|&row| row.song.path == path)
                    {
                        Some(row) => vec![row.id],
                        None => vec![],
                    }
                } else {
                    self.data_rows
                        .par_iter()
                        .filter_map(|row| {
                            if let Some(parent) = row.song.path.parent()
                                && parent == path
                            {
                                Some(row.id)
                            } else {
                                None
                            }
                        })
                        .collect()
                };

                Response::new_ok().with_item("ids", &ids)
            }
            Err(e) => Response::new_err(e.to_string()),
        }
    }

    // get values of `tags` for songs with `ids`
    pub fn metadata(&self, MetadataArgs(ids, tags): MetadataArgs) -> Response {
        let values: Vec<_> = ids
            .into_par_iter()
            .map(|id| {
                self.data_rows
                    .binary_search_by_key(&id, |row| row.id)
                    .map(|i| {
                        let data = tags.iter().map(|tag| {
                            let value = self.data_rows[i].song.metadata.get(tag).into();
                            (tag.to_string(), value)
                        });

                        Some(Map::from_iter(data))
                    })
                    .ok()
            })
            .collect();

        Response::new_ok().with_item("values", &values)
    }

    // get ids of songs matching `filter_expr`, sorted by the comparators in `sort_by`
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
        filtered.par_sort_by(|lhs, rhs| compare(&lhs.song.metadata, &rhs.song.metadata));
        let ids: Vec<_> = filtered.into_par_iter().map(|row| row.id).collect();

        Response::new_ok().with_item("ids", &ids)
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
        self.data_rows.par_iter_mut().for_each(|row| {
            if let Ok(mod_time) = row
                .song
                .path
                .metadata()
                .and_then(|metadata| metadata.modified())
            {
                if mod_time >= self.last_update
                    && let Ok(song) = Song::try_new(&self.music_dir, row.song.path.as_path())
                {
                    row.song = song;
                }
            } else {
                row.to_delete = true;
            }
        });
        self.data_rows.retain(|row| !row.to_delete);
        let new_files =
            match db_utils::walk_dir(&self.music_dir, self.last_update, &self.allowed_exts) {
                Ok(new_files) => new_files,
                Err(e) => return Response::new_err(e.to_string()),
            };
        let mut new_data_rows = Self::to_data_rows(
            &self.music_dir,
            &new_files,
            self.data_rows.last().map(|row| row.id).unwrap_or(0),
        );
        self.data_rows.append(&mut new_data_rows);
        self.last_update = SystemTime::now();

        Response::new_ok().with_item("new_files", &new_files.len())
    }
}

mod db_utils {
    use super::*;

    pub fn walk_dir(
        music_dir: &Path,
        timestamp: SystemTime,
        allowed_exts: &[String],
    ) -> Result<Vec<PathBuf>> {
        let is_ok = move |path: &Path| -> bool {
            if let Some(ext) = path.extension().and_then(|ext| ext.to_str())
                && allowed_exts.iter().any(|allowed_ext| allowed_ext == ext)
                && let Ok(creation_time) = path.metadata().and_then(|meta| meta.created())
            {
                return creation_time >= timestamp;
            }

            false
        };

        // TODO: ignore specified directories (similar to a .gitignore)
        if !music_dir.exists() {
            bail!(format!(
                "directory `{}` doesn't exist",
                music_dir.to_string_lossy()
            ));
        }
        let list = WalkDir::new(music_dir)
            .into_iter()
            .filter_map(|entry| {
                if let Ok(entry) = entry
                    && entry.file_type.is_file()
                    && let Ok(full_path) = dunce::canonicalize(entry.path())
                    && is_ok(&full_path)
                    && let Ok(rel_path) = full_path.strip_prefix(music_dir)
                {
                    return Some(rel_path.to_path_buf());
                }
                None
            })
            .collect();

        Ok(list)
    }
}
