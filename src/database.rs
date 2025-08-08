use anyhow::Result;
use rayon::prelude::*;
use rayon::prelude::*;
use serde_json::{Map, Value as JsonValue};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    iter::{FromIterator, IntoIterator, Iterator},
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{
    error::MyError,
    model::{
        comparator::Comparator,
        filter::FilterExpr,
        request::{MetadataArgs, SelectArgs, UniqueArgs},
        response::Response,
        song::*,
        tag_key::TagKey,
    },
    utils,
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
                    log::warn!("{}", e);
                    None
                }
            })
            .collect()
    }

    pub fn new(music_dir: PathBuf, allowed_exts: Vec<String>) -> Self {
        let files = utils::walk_dir(&music_dir, SystemTime::UNIX_EPOCH, &allowed_exts);
        let data_rows = Self::to_data_rows(&files, 0);
        let last_update = SystemTime::now();

        Self {
            music_dir,
            allowed_exts,
            data_rows,
            last_update,
        }
    }

    pub fn last_id(&self) -> u32 {
        self.data_rows.last().map(|row| row.id).unwrap_or(0)
    }

    pub fn song_by_id(&self, id: u32) -> Result<&Song> {
        self.data_rows
            .binary_search_by_key(&id, |row| row.id)
            .map(|i| &self.data_rows[i].song)
            .map_err(|_| MyError::Database(format!("Song with id `{}` not found", id)).into())
    }

    pub fn update(&mut self) -> Response {
        self.data_rows.par_iter_mut().for_each(|row| {
            if let Ok(mod_time) = row
                .song
                .path
                .metadata()
                .and_then(|metadata| metadata.modified())
            {
                if mod_time >= self.last_update {
                    if let Ok(song) = Song::try_from(row.song.path.as_path()) {
                        row.song = song;
                    }
                }
            } else {
                row.to_delete = true;
            }
        });
        self.data_rows.retain(|row| !row.to_delete);
        let new_files = utils::walk_dir(&self.music_dir, self.last_update, &self.allowed_exts);
        let mut new_data_rows = Self::to_data_rows(
            &new_files,
            self.data_rows.last().map(|row| row.id).unwrap_or(0),
        );
        self.data_rows.append(&mut new_data_rows);
        self.last_update = SystemTime::now();

        Response::new_ok().with_item("new_files".into(), &new_files.len())
    }

    /// Get ids of songs matching `filter_expr`, sorted by the comparators in `sort_by`.
    fn select(&self, filter_expr: FilterExpr, sort_by: Vec<Comparator>) -> Vec<u32> {
        let compare = |lhs: &SongMeta, rhs: &SongMeta| -> Ordering {
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
        filtered.par_sort_by(|lhs, rhs| compare(&lhs.song.song_meta, &rhs.song.song_meta));

        filtered.into_par_iter().map(|row| row.id).collect()
    }

    /// ("inner" because this returns a Vec "inside" = to other rustmpd functions)
    pub fn select_inner(&self, SelectArgs(filter_expr, sort_by): SelectArgs) -> Vec<u32> {
        self.select(filter_expr, sort_by)
    }

    /// ("outer" because this returns JSON "outside" = to the client)
    pub fn select_outer(&self, SelectArgs(filter_expr, sort_by): SelectArgs) -> Response {
        let ids = self.select(filter_expr, sort_by);
        Response::new_ok().with_item("ids".into(), &ids)
    }

    /// Get values of `tags` for songs with `ids`.
    pub fn metadata(&self, MetadataArgs(ids, tags): MetadataArgs) -> Response {
        let values: Vec<_> = ids
            .into_par_iter()
            .map(|id| {
                self.data_rows
                    .binary_search_by_key(&id, |row| row.id)
                    .map(|i| {
                        let data = tags.iter().cloned().map(|tag| {
                            let value = self.data_rows[i].song.song_meta.get(&tag).into();
                            (tag.to_string(), value)
                        });

                        Some(Map::from_iter(data))
                    })
                    .ok()
            })
            .collect();

        Response::new_ok().with_item("values".into(), &values)
    }

    /// Get unique values of `tag` among songs matching `filter_expr`, grouped by tags in `group_by`.
    pub fn unique(&self, UniqueArgs(tag, group_by, filter_expr): UniqueArgs) -> Response {
        let mut groups = HashMap::new();
        let filtered = self
            .data_rows
            .iter()
            .filter(|row| filter_expr.evaluate(&row.song))
            .map(|row| &row.song.song_meta);
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
}
