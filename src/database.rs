use anyhow::Result;
use serde_json::{Map, Value as JsonValue};
use std::{
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
            }
        }
        let new_files = utils::walk_dir(&self.root_dir, self.last_update, &self.ok_ext);
        let mut new_data_rows = Self::to_data_rows(
            &new_files,
            self.data_rows.last().map(|row| row.id).unwrap_or(0),
        )
        .collect();
        self.data_rows.append(&mut new_data_rows);
        self.last_update = SystemTime::now();
    }

    /// Get ids of songs matching `filter_expr`.
    pub fn select(&self, filter_expr: FilterExpr) -> Response {
        let ids: Vec<_> = self
            .data_rows
            .iter()
            .filter(|row| filter_expr.evaluate(&row.song))
            .map(|row| &row.id)
            .collect();

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

    /// Get unique values of `tag` among songs matching `filter_expr`, sorted by `sort_by`, grouped by tags in `group_by`.
    pub fn unique(
        &self,
        main_tag: String,
        filter_expr: FilterExpr,
        sort_by: Vec<String>,
        group_by: Vec<String>,
    ) -> Response {
        // TODO: sorting duplicates values?
        let song_to_map_entry = |meta: &SongMeta| -> (Vec<Option<String>>, Option<String>) {
            let mut entry = (Vec::new(), meta.get(&main_tag).cloned());
            for tag in sort_by.iter() {
                entry.0.push(meta.get(&tag).cloned());
            }

            entry
        };

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
                    set.insert(song_to_map_entry(&meta));
                })
                .or_insert([song_to_map_entry(&meta)].into());
        }
        let values: Vec<_> = groups
            .into_iter()
            .map(|(combination, values)| {
                let data = group_by
                    .iter()
                    .cloned()
                    .zip(combination.into_iter().map(|value| value.into()));
                let mut json_map = Map::from_iter(data);
                let mut values: Vec<_> = values.into_iter().collect();
                values.sort_by(|e1, e2| (e1.0).cmp(&e2.0));
                let values = values.into_iter().map(|entry| entry.1).collect();
                json_map.insert(main_tag.clone(), values);

                json_map
            })
            .collect();

        Response::new_ok().with_item("values".into(), &values)
    }
}
