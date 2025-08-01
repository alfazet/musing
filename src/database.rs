use anyhow::Result;
use serde_json::{Map, Value as JsonValue};
use std::{
    collections::{HashMap, HashSet},
    iter::{FromIterator, IntoIterator, Iterator},
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{
    model::{response::Response, song::*},
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
    data_rows: Vec<DataRow>,
    last_update: SystemTime,
}

impl Database {
    fn to_data_rows(files: &[PathBuf]) -> impl Iterator<Item = DataRow> {
        files
            .iter()
            .enumerate()
            .filter_map(|(id, file)| match Song::try_from_file(file) {
                Ok(song) => Some(DataRow {
                    id: id as u32 + 1,
                    song,
                }),
                Err(e) => {
                    log::error!("{}", e);
                    None
                }
            })
    }

    pub fn from_dir(dir: &Path, ok_ext: Vec<String>) -> Self {
        let all_files = utils::walk_dir(dir, SystemTime::UNIX_EPOCH, ok_ext);
        let data_rows = Self::to_data_rows(&all_files).collect();
        let last_update = SystemTime::now();

        Self {
            root_dir: dir.to_path_buf(),
            data_rows,
            last_update,
        }
    }

    // TODO: this doesn't check if any new files have been created
    // call from_dir with .last_update timestamp and pass it to walk_dir
    // walk_dir should only check files that are newer than timestamp
    // also make sure that ids of new songs are increasing starting at the last
    // already present id
    pub fn update(&mut self) {
        for row in self.data_rows.iter_mut() {
            if let Ok(timestamp) = row
                .song
                .path
                .metadata()
                .and_then(|metadata| metadata.accessed())
            {
                if timestamp > self.last_update {
                    if let Ok(song) = Song::try_from_file(&row.song.path) {
                        row.song = song;
                    }
                }
            }
        }
        self.last_update = SystemTime::now();
    }

    /// Get ids of songs matching `filter`.
    pub fn select(&self, filter_str: String) -> Response {
        // let filter_expr = match parsing::filter::into_rpn(&filter_str) {
        //     Ok(expr) => expr,
        //     Err(e) => return Response::new_err(e),
        // };
        // let ids: Vec<_> = self
        //     .data_rows
        //     .iter()
        //     .filter(|row| parsing::filter::evaluate(&filter_expr, &row.song))
        //     .map(|row| &row.id)
        //     .collect();
        //
        // Response::new_ok().with_item("ids".into(), &ids)

        Response::new_ok()
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

    /// Get unique values of `tag` among songs matching `filter_str`, grouped by tags in `group_by`.
    pub fn unique(&self, tag: String, filter_str: String, group_by: Vec<String>) -> Response {
        // let filter_expr = match parsing::filter::into_rpn(&filter_str) {
        //     Ok(expr) => expr,
        //     Err(e) => return Response::new_err(e),
        // };
        // let mut groups = HashMap::new();
        // let filtered = self
        //     .data_rows
        //     .iter()
        //     .filter(|row| parsing::filter::evaluate(&filter_expr, &row.song))
        //     .map(|row| &row.song.song_meta);
        // for meta in filtered {
        //     let combination: Vec<_> = group_by
        //         .iter()
        //         .map(|group_tag| meta.get(group_tag).cloned())
        //         .collect();
        //     groups
        //         .entry(combination)
        //         .and_modify(|set: &mut HashSet<_>| {
        //             set.insert(meta.get(&tag).cloned());
        //         })
        //         .or_insert([meta.get(&tag).cloned()].into());
        // }
        // let values: Vec<_> = groups
        //     .into_iter()
        //     .map(|(combination, songs)| {
        //         let data = group_by
        //             .iter()
        //             .cloned()
        //             .zip(combination.into_iter().map(|value| value.into()));
        //         let mut json_map = Map::from_iter(data);
        //         json_map.insert(tag.clone(), songs.into_iter().collect());
        //
        //         json_map
        //     })
        //     .collect();
        //
        // Response::new_ok().with_item("values".into(), &values)

        Response::new_ok()
    }
}
