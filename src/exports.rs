use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Receipt {
    source: PathBuf,
    filename: String,
    hash: String,
    previous_hash: Option<String>,
}

fn hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let length = file.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn save(staged: &Path, source: &Path, folder: &Path, extension: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        !extension.is_empty() && extension.chars().all(|value| value.is_ascii_alphanumeric()),
        "Invalid export format"
    );
    fs::create_dir_all(folder)?;
    let folder = folder.canonicalize()?;
    let source = if source.exists() {
        source.canonicalize()?
    } else if source.is_absolute() {
        source.to_path_buf()
    } else {
        std::env::current_dir()?.join(source)
    };
    let mut digest = Sha256::new();
    digest.update(source.to_string_lossy().to_lowercase());
    digest.update(extension.to_lowercase());
    let key = format!("{:x}", digest.finalize());
    let records = folder.join(".atlas-exports");
    fs::create_dir_all(&records)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(records.join(format!("{key}.lock")))?;
    lock.lock()?;
    let record = records.join(format!("{key}.json"));
    let previous: Option<Receipt> = if record.exists() {
        Some(serde_json::from_slice(&fs::read(&record)?)?)
    } else {
        None
    };
    let stem = crate::model::safe_segment(
        source
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("image"),
    );
    let filename = if let Some(receipt) = &previous {
        anyhow::ensure!(
            receipt.source == source
                && Path::new(&receipt.filename)
                    .file_name()
                    .and_then(|name| name.to_str())
                    == Some(&receipt.filename),
            "Export receipt does not match this image"
        );
        receipt.filename.clone()
    } else {
        let plain = format!("{stem}.{extension}");
        if folder.join(&plain).exists() || folder.join(&plain) == source {
            format!("{stem}--export-{}.{extension}", &key[..12])
        } else {
            plain
        }
    };
    let destination = folder.join(&filename);
    anyhow::ensure!(
        destination != source,
        "An export cannot replace its source image"
    );
    let mut old_hash = None;
    if destination.exists() {
        let existing = hash(&destination)?;
        let receipt = previous
            .as_ref()
            .context("An unrelated file already occupies the export path")?;
        anyhow::ensure!(
            existing == receipt.hash || receipt.previous_hash.as_ref() == Some(&existing),
            "The exported file was changed outside Atlas. Choose another export folder to preserve those changes."
        );
        old_hash = Some(existing);
    }
    let new_hash = hash(staged)?;
    if old_hash.as_ref() == Some(&new_hash) {
        return Ok(destination);
    }
    let receipt = Receipt {
        source,
        filename,
        hash: new_hash,
        previous_hash: old_hash,
    };
    let mut receipt_temp = tempfile::NamedTempFile::new_in(&records)?;
    serde_json::to_writer(receipt_temp.as_file_mut(), &receipt)?;
    receipt_temp.as_file().sync_all()?;
    receipt_temp.persist(record).map_err(|error| error.error)?;
    let mut output = tempfile::NamedTempFile::new_in(&folder)?;
    std::io::copy(&mut fs::File::open(staged)?, output.as_file_mut())?;
    output.as_file().sync_all()?;
    output.persist(&destination).map_err(|error| error.error)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_edits_replace_one_output_and_preserve_the_source() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("source.png");
        fs::write(&source, b"original").unwrap();
        let staged = folder.path().join("staged");
        let output = folder.path().join("output");
        let mut paths = Vec::new();
        for content in ["edit one", "edit two", "edit three"] {
            fs::write(&staged, content).unwrap();
            paths.push(save(&staged, &source, &output, "webp").unwrap());
        }
        assert!(paths.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(fs::read(&paths[0]).unwrap(), b"edit three");
        assert_eq!(fs::read(&source).unwrap(), b"original");
        assert_eq!(
            fs::read_dir(output)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_file())
                .count(),
            1
        );
    }
    #[test]
    fn identical_names_from_different_sources_do_not_replace_each_other() {
        let folder = tempfile::tempdir().unwrap();
        let output = folder.path().join("output");
        let staged = folder.path().join("staged");
        fs::write(&staged, b"export").unwrap();
        let mut results = Vec::new();
        for name in ["first", "second"] {
            let directory = folder.path().join(name);
            fs::create_dir_all(&directory).unwrap();
            let source = directory.join("image.png");
            fs::write(&source, b"original").unwrap();
            results.push(save(&staged, &source, &output, "webp").unwrap());
        }
        assert_ne!(results[0], results[1]);
    }
    #[test]
    fn existing_original_and_external_changes_are_preserved() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("image.png");
        let staged = folder.path().join("staged");
        fs::write(&source, b"original").unwrap();
        fs::write(&staged, b"edited").unwrap();
        let output = save(&staged, &source, folder.path(), "png").unwrap();
        assert_ne!(output, source);
        fs::write(&output, b"manual edit").unwrap();
        assert!(save(&staged, &source, folder.path(), "png").is_err());
        assert_eq!(fs::read(output).unwrap(), b"manual edit");
        assert_eq!(fs::read(source).unwrap(), b"original");
    }
}
