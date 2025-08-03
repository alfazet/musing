use anyhow::Result;
use serde_json::{Map, Value as JsonValue};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    iter::{FromIterator, IntoIterator, Iterator},
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{
    model::{filter::FilterExpr, response::Response, song::*},
    utils,
};

#[derive(Debug)]
pub struct DataRow {
    pub id: u32,
    pub song: Song,
    pub to_delete: bool,
}

#[derive(Debug)]
pub struct Database {
    root_dir: PathBuf,
    ok_ext: Vec<String>,
    data_rows: Vec<DataRow>,
    last_update: SystemTime,
}

impl Database {
    fn to_data_rows(files: &[PathBuf], id_offset: u32) -> impl Iterator<Item = DataRow> {
        files
            .iter()
            .enumerate()
            .filter_map(move |(id, file)| match Song::try_from_file(file) {
                Ok(song) => Some(DataRow {
                    id: id as u32 + id_offset + 1,
                    song,
                    to_delete: false,
                }),
                Err(e) => {
                    log::error!("{}", e);
                    None
                }
            })
    }

    pub fn from_dir(dir: &Path, ok_ext: Vec<String>) -> Self {
        let files = utils::walk_dir(dir, SystemTime::UNIX_EPOCH, &ok_ext);
        let data_rows = Self::to_data_rows(&files, 0).collect();
        let last_update = SystemTime::now();

        Self {
            root_dir: dir.to_path_buf(),
            ok_ext,
            data_rows,
            last_update,
        }
    }

    pub fn update(&mut self) {
        for row in self.data_rows.iter_mut() {
            if let Ok(mod_time) = row
                .song
                .path
                .metadata()
                .and_then(|metadata| metadata.modified())
            {
                if mod_time >= self.last_update {
                    if let Ok(song) = Song::try_from_file(&row.song.path) {
                        row.song = song;
                    }
                }
            } else {
                row.to_delete = true;
            }
        }
        self.data_rows.retain(|row| !row.to_delete);
        let new_files = utils::walk_dir(&self.root_dir, self.last_update, &self.ok_ext);
        let mut new_data_rows = Self::to_data_rows(
            &new_files,
            self.data_rows.last().map(|row| row.id).unwrap_or(0),
        )
        .collect();
        self.data_rows.append(&mut new_data_rows);
        self.last_update = SystemTime::now();
    }

    /// Get ids of songs matching `filter_expr`, sorted by the values of tags in `sort_by`.
    pub fn select(&self, (filter_expr, sort_by): (FilterExpr, Vec<String>)) -> Response {
        let cmp = |lhs: &SongMeta, rhs: &SongMeta| -> Ordering {
            for tag in sort_by.iter() {
                match (lhs.get(tag)).cmp(&rhs.get(tag)) {
                    Ordering::Equal => (),
                    other => return other,
                }
            }

            Ordering::Equal
        };

        let mut filtered: Vec<_> = self
            .data_rows
            .iter()
            .filter(|row| filter_expr.evaluate(&row.song))
            .collect();
        filtered.sort_by(|lhs, rhs| cmp(&lhs.song.song_meta, &rhs.song.song_meta));
        let ids: Vec<_> = filtered.into_iter().map(|row| row.id).collect();

        Response::new_ok().with_item("ids".into(), &ids)
    }

    /// Get values of `tags` for songs with `ids`.
    pub fn metadata(&self, (ids, tags): (Vec<u32>, Vec<String>)) -> Response {
        let values: Vec<_> = ids
            .into_iter()
            .map(|id| {
                if let Ok(i) = self.data_rows.binary_search_by_key(&id, |row| row.id) {
                    let data = tags.iter().cloned().map(|tag| {
                        let value = self.data_rows[i].song.song_meta.get(&tag).cloned().into();
                        (tag, value)
                    });

                    Some(Map::from_iter(data))
                } else {
                    None
                }
            })
            .collect();

        Response::new_ok().with_item("values".into(), &values)
    }

    /// Get unique values of `tag` among songs matching `filter_expr`, grouped by tags in `group_by`.
    pub fn unique(
        &self,
        (tag, filter_expr, group_by): (String, FilterExpr, Vec<String>),
    ) -> Response {
        let mut groups = HashMap::new();
        let filtered = self
            .data_rows
            .iter()
            .filter(|row| filter_expr.evaluate(&row.song))
            .map(|row| &row.song.song_meta);
        for meta in filtered {
            let combination: Vec<_> = group_by
                .iter()
                .map(|group_tag| meta.get(group_tag).cloned())
                .collect();
            groups
                .entry(combination)
                .and_modify(|set: &mut HashSet<_>| {
                    set.insert(meta.get(&tag).cloned());
                })
                .or_insert([meta.get(&tag).cloned()].into());
        }
        let values: Vec<_> = groups
            .into_iter()
            .map(|(combination, values)| {
                let data = group_by
                    .iter()
                    .cloned()
                    .zip(combination.into_iter().map(|value| value.into()));
                let mut json_map = Map::from_iter(data);
                let values: Vec<_> = values.into_iter().collect();
                json_map.insert(tag.clone(), values.into());

                json_map
            })
            .collect();

        Response::new_ok().with_item("values".into(), &values)
    }
}
