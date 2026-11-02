use crate::{
    convert,
    model::{Cancel, Event},
};
use std::{collections::HashSet, path::PathBuf};

pub fn scan(
    roots: Vec<PathBuf>,
    mut seen: HashSet<PathBuf>,
    cancel: Cancel,
    events: crossbeam_channel::Sender<Event>,
) {
    let mut batch = Vec::new();
    let mut last_sent = std::time::Instant::now();
    for root in roots {
        let entries = walkdir::WalkDir::new(root)
            .into_iter()
            .filter_entry(|entry| {
                entry.file_name() != ".atlas-cache"
                    && entry.file_name() != ".atlas-exports"
                    && !entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".atlas-export-")
            });
        for entry in entries {
            if cancel.stopped() {
                let _ = events.send(Event::ImportFinished);
                return;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    let _ = events.send(Event::Log(error.to_string()));
                    continue;
                }
            };
            if !entry.file_type().is_file() || !seen.insert(entry.path().to_path_buf()) {
                continue;
            }
            let path = entry.into_path();
            match convert::dimensions(&path) {
                Ok((width, height)) => batch.push((path, format!("{width} × {height} px"))),
                Err(error) => {
                    let _ = events.send(Event::Log(format!("{}: {error}", path.display())));
                }
            }
            if batch.len() >= 64 || last_sent.elapsed().as_millis() >= 100 {
                if !batch.is_empty()
                    && events
                        .send(Event::Imported(std::mem::take(&mut batch)))
                        .is_err()
                {
                    return;
                }
                last_sent = std::time::Instant::now();
            }
        }
    }
    if !batch.is_empty() {
        let _ = events.send(Event::Imported(batch));
    }
    let _ = events.send(Event::ImportFinished);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Manual read-only import check using ATLAS_IMPORT_SAMPLE"]
    fn import_local_folder() {
        let root = PathBuf::from(std::env::var_os("ATLAS_IMPORT_SAMPLE").unwrap());
        let (tx, rx) = crossbeam_channel::unbounded();
        let started = std::time::Instant::now();
        let worker =
            std::thread::spawn(move || scan(vec![root], HashSet::new(), Cancel::default(), tx));
        let mut files = 0;
        let mut errors = 0;
        let mut finished = false;
        for event in rx {
            match event {
                Event::Imported(batch) => files += batch.len(),
                Event::Log(_) => errors += 1,
                Event::ImportFinished => finished = true,
                _ => {}
            }
        }
        worker.join().unwrap();
        assert!(finished);
        assert!(files > 0);
        eprintln!(
            "Imported {files} images, skipped {errors} files, elapsed {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn folder_import_skips_cache_duplicates_and_non_images() {
        let folder = tempfile::tempdir().unwrap();
        let cache = folder.path().join(".atlas-cache/nested");
        std::fs::create_dir_all(&cache).unwrap();
        let first = folder.path().join("first.png");
        let second = folder.path().join("second.png");
        let image = image::RgbaImage::new(3, 4);
        image.save(&first).unwrap();
        image.save(&second).unwrap();
        image.save(cache.join("hidden.png")).unwrap();
        std::fs::write(folder.path().join("report.txt"), "report").unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        scan(
            vec![folder.path().into(), second.clone()],
            HashSet::from([first]),
            Cancel::default(),
            tx,
        );
        let mut files = Vec::new();
        let mut finished = 0;
        for event in rx.try_iter() {
            match event {
                Event::Imported(batch) => files.extend(batch),
                Event::ImportFinished => finished += 1,
                _ => {}
            }
        }
        assert_eq!(files, vec![(second, "3 × 4 px".into())]);
        assert_eq!(finished, 1);
    }

    #[test]
    fn cancelled_import_finishes_without_adding_files() {
        let folder = tempfile::tempdir().unwrap();
        image::RgbaImage::new(3, 4)
            .save(folder.path().join("image.png"))
            .unwrap();
        let cancel = Cancel::default();
        cancel.stop();
        let (tx, rx) = crossbeam_channel::unbounded();
        scan(vec![folder.path().into()], HashSet::new(), cancel, tx);
        assert!(matches!(rx.recv().unwrap(), Event::ImportFinished));
        assert!(rx.try_recv().is_err());
    }
}
