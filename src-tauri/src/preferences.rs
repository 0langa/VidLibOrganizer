use std::{collections::HashSet, fs, io::Write, path::Path};

pub fn load(path: &Path) -> Result<Vec<String>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("Saved folders could not be read: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("Saved folders could not be opened: {error}")),
    }
}

pub fn save(path: &Path, paths: Vec<String>) -> Result<(), String> {
    let mut seen = HashSet::new();
    let paths: Vec<String> = paths
        .into_iter()
        .map(|path| path.trim().to_owned())
        .filter(|path| {
            !path.is_empty()
                && seen.insert(if cfg!(windows) {
                    path.to_lowercase()
                } else {
                    path.clone()
                })
        })
        .collect();
    let parent = path.parent().ok_or("Invalid preferences location.")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("pending");
    let mut file = fs::File::create(&temporary).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(&paths).map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(temporary, path).map_err(|error| format!("Could not save folders: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_offline_folders_and_replaces_previous_preferences() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings").join("libraries.json");
        assert!(load(&path).unwrap().is_empty());
        save(
            &path,
            vec!["  Z:\\Movies  ".into(), "Z:\\Movies".into(), "".into()],
        )
        .unwrap();
        assert_eq!(load(&path).unwrap(), vec!["Z:\\Movies"]);
        save(&path, vec!["E:\\映画".into()]).unwrap();
        assert_eq!(load(&path).unwrap(), vec!["E:\\映画"]);
    }
    #[test]
    fn damaged_settings_are_reported_instead_of_silently_erased() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("libraries.json");
        fs::write(&path, "broken").unwrap();
        assert!(load(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "broken");
    }
}
