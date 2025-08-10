use anyhow::{Result, bail};
use jwalk::WalkDir;
use std::{
    fs::{self, DirEntry, File, Metadata},
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::error::MyError;

macro_rules! enum_stringify {
    ($variant:expr) => {{
        let s = format!("{:?}", $variant);
        s.split("::").last().unwrap().to_string()
    }};
}
pub(crate) use enum_stringify;

/// Returns absolute paths of files in `root_dir`.
/// Only files with creation times greater than `timestamp`
/// and extensions contained in `allowed_exts` are taken into account.
pub fn walk_dir(
    root_dir: &Path,
    timestamp: SystemTime,
    allowed_exts: &[String],
) -> Result<Vec<PathBuf>> {
    let is_ok = move |path: &Path| -> bool {
        if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
            if allowed_exts.iter().any(|allowed_ext| allowed_ext == ext) {
                if let Ok(mod_time) = path.metadata().and_then(|meta| meta.created()) {
                    return mod_time >= timestamp;
                }
            }
        }

        false
    };

    // TODO: ignore specified directories (like .gitignore)
    if !root_dir.exists() {
        bail!(MyError::Database(format!(
            "Directory `{}` doesn't exist",
            root_dir.to_string_lossy()
        )));
    }
    let list = WalkDir::new(root_dir);

    Ok(list
        .into_iter()
        .filter_map(|entry| {
            if let Ok(entry) = entry {
                if let Ok(full_path) = entry.path().canonicalize()
                    && entry.file_type.is_file()
                    && is_ok(&full_path)
                {
                    return Some(full_path);
                }
            }
            None
        })
        .collect())
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
        let prefix = Alphanumeric.sample_string(&mut rng, 5);
        let path = PathBuf::from("/tmp");
        let timestamp = SystemTime::now();
        for i in 0..100 {
            let _ = File::create(path.join(format!("{}-test{}.mp3", prefix, i)));
        }
        let allowed_exts = vec!["mp3".into()];
        let files = walk_dir(&path, timestamp, &allowed_exts).unwrap();
        for file in files {
            assert!(file.exists() && file.is_file() && file.extension().unwrap() == "mp3");
        }
        for i in 0..100 {
            let _ = fs::remove_file(path.join(format!("{}-test{}.mp3", prefix, i)));
        }
    }
}
