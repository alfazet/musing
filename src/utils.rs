use anyhow::Result;
use std::{
    fs::{self, DirEntry, File, Metadata},
    path::{Path, PathBuf},
    time::SystemTime,
};

macro_rules! enum_stringify {
    ($variant:expr) => {{
        let s = format!("{:?}", $variant);
        s.split("::").last().unwrap().to_string()
    }};
}
pub(crate) use enum_stringify;

/// Returns absolute paths of files in this directory and its sub-dirs.
/// Only files with creation times greater than `timestamp`
/// and extensions contained in `ok_extensions` are taken into account.
pub fn walk_dir(root_dir: &Path, timestamp: SystemTime, ok_extensions: &[String]) -> Vec<PathBuf> {
    let is_ok = |path: &Path| -> bool {
        if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
            if ok_extensions.iter().any(|ok_ext| ok_ext == ext) {
                if let Ok(mod_time) = path.metadata().and_then(|meta| meta.created()) {
                    return mod_time >= timestamp;
                }
            }
        }

        false
    };

    let mut list = Vec::new();
    let mut stack = vec![root_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        match fs::read_dir(dir) {
            Ok(entries) => {
                for path in entries.filter_map(|entry| entry.map(|entry| entry.path()).ok()) {
                    if path.is_dir() {
                        stack.push(path);
                    } else if is_ok(&path) {
                        if let Ok(absolute) = path.canonicalize() {
                            list.push(absolute);
                        }
                    }
                }
            }
            Err(e) => log::warn!("{}", e),
        }
    }

    list
}

#[cfg(test)]
mod test {
    use super::*;
    use rand::{
        distr::{Alphanumeric, SampleString},
        prelude::*,
    };

    #[test]
    fn find_all_mp3s() {
        let mut rng = rand::rng();
        let prefix = Alphanumeric.sample_string(&mut rng, 10);
        let path = PathBuf::from("/tmp");
        let timestamp = SystemTime::now();
        for i in 0..10 {
            let _ = File::create(path.join(format!("{}-test{}.mp3", prefix, i)));
        }
        let ok_ext = vec!["mp3".into()];
        let files = walk_dir(&path, timestamp, &ok_ext);
        for file in files {
            assert!(file.exists() && file.is_file() && file.extension().unwrap() == "mp3");
        }
        for i in 0..10 {
            let _ = fs::remove_file(path.join(format!("{}-test{}.mp3", prefix, i)));
        }
    }
}
