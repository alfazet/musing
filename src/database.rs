use anyhow::{Result, bail};
use jwalk::WalkDir;
use rayon::prelude::*;
use serde_json::{Map, Value as JsonValue};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    iter::{FromIterator, IntoIterator, Iterator},
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::model::{
    comparator::Comparator,
    filter::FilterExpr,
    request::{MetadataArgs, SelectArgs, UniqueArgs},
    response::Response,
    song::{Metadata, Song},
    tag_key::TagKey,
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
    fn to_data_rows(files: &[PathBuf], id_offset: u32) -> Vec<DataRow> {
        files
            .par_iter()
            .enumerate()
            .filter_map(move |(id, file)| match Song::try_from(file.as_path()) {
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
            })
            .collect()
    }

    pub fn from_dir(music_dir: &Path, allowed_exts: &[String]) -> Result<Self> {
        let files = db_utils::walk_dir(music_dir, SystemTime::UNIX_EPOCH, allowed_exts)?;
        let data_rows = Self::to_data_rows(&files, 0);
        let last_update = SystemTime::now();

        Ok(Self {
            music_dir: music_dir.to_path_buf(),
            allowed_exts: allowed_exts.to_vec(),
            data_rows,
            last_update,
        })
    }

    pub fn song_by_id(&self, id: u32) -> Option<&Song> {
        self.data_rows
            .binary_search_by_key(&id, |row| row.id)
            .map(|i| &self.data_rows[i].song)
            .ok()
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

    // get values of `tags` for songs with `ids`
    pub fn metadata(&self, MetadataArgs(ids, tags): MetadataArgs) -> Response {
        let values: Vec<_> = ids
            .into_par_iter()
            .map(|id| {
                self.data_rows
                    .binary_search_by_key(&id, |row| row.id)
                    .map(|i| {
                        let data = tags.iter().cloned().map(|tag| {
                            let value = self.data_rows[i].song.metadata.get(&tag).into();
                            (tag.to_string(), value)
                        });

                        Some(Map::from_iter(data))
                    })
                    .ok()
            })
            .collect();

        Response::new_ok().with_item("values".into(), &values)
    }

    // get ids of songs matching `filter_expr`, sorted by the comparators in `sort_by`
    fn select(&self, filter_expr: FilterExpr, sort_by: Vec<Comparator>) -> Vec<u32> {
        let compare = |lhs: &Metadata, rhs: &Metadata| -> Ordering {
            sort_by
                .iter()
                .map(|cmp| cmp.cmp(lhs, rhs))
                .find(|ord| *ord != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        };

        let mut filtered: Vec<_> = self
            .data_rows
            .par_iter()
            .filter(|row| filter_expr.evaluate(&row.song))
            .collect();
        filtered.par_sort_by(|lhs, rhs| compare(&lhs.song.metadata, &rhs.song.metadata));

        filtered.into_par_iter().map(|row| row.id).collect()
    }

    pub fn select_inner(&self, SelectArgs(filter_expr, sort_by): SelectArgs) -> Vec<u32> {
        self.select(filter_expr, sort_by)
    }

    pub fn select_outer(&self, SelectArgs(filter_expr, sort_by): SelectArgs) -> Response {
        let ids = self.select(filter_expr, sort_by);
        Response::new_ok().with_item("ids".into(), &ids)
    }

    // get unique values of `tag` among songs matching `filter_expr`, grouped by tags in `group_by`
    pub fn unique(&self, UniqueArgs(tag, group_by, filter_expr): UniqueArgs) -> Response {
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
                    .cloned()
                    .map(|tag_key| tag_key.to_string())
                    .zip(combination.into_iter().map(|value| value.into()));
                let mut json_map = Map::from_iter(data);
                let values: Vec<_> = values.into_iter().collect();
                json_map.insert(tag.to_string(), values.into());

                json_map
            })
            .collect();

        Response::new_ok().with_item("values".into(), &values)
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
                    && let Ok(song) = Song::try_from(row.song.path.as_path())
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
            &new_files,
            self.data_rows.last().map(|row| row.id).unwrap_or(0),
        );
        self.data_rows.append(&mut new_data_rows);
        self.last_update = SystemTime::now();

        Response::new_ok().with_item("new_files".into(), &new_files.len())
    }
}

mod db_utils {
    use super::*;

    pub fn walk_dir(
        root_dir: &Path,
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

        // TODO: ignore specified directories (like .gitignore)
        if !root_dir.exists() {
            bail!(format!(
                "directory `{}` doesn't exist",
                root_dir.to_string_lossy()
            ));
        }
        let list = WalkDir::new(root_dir)
            .into_iter()
            .filter_map(|entry| {
                if let Ok(entry) = entry
                    && let Ok(full_path) = entry.path().canonicalize()
                    && entry.file_type.is_file()
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
