// SPDX-License-Identifier: GPL-3.0-or-later

//! Fetching NVIDIA's TensorRT-RTX for the TensorRT engine.
//!
//! The program ships its own part of that engine — the vstrt plugin, vsmlrt
//! and the networks, all under open licences — but not TensorRT-RTX: that is
//! NVIDIA's, licensed by NVIDIA, and the person watching fetches it from
//! NVIDIA when they ask for it. This does the fetching: NVIDIA's archive for
//! this platform, unpacked into the data folder ([`TensorRt::runtime_dir`]),
//! the archive's own top folder dropped so `lib/` and `bin/` sit at the top.
//!
//! It is written to a folder beside the final one and moved into place only
//! once whole, so an interrupted download never looks like an install.

use std::io::Read;
use std::path::{Path, PathBuf};

use anirust_player::TensorRt;
use futures::StreamExt;

/// How far a fetch has got, for the line on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Fetching { done: u64, total: Option<u64> },
    Unpacking,
}

/// Fetches and unpacks TensorRT-RTX, reporting along the way. Answers with
/// the folder it is now in.
pub async fn fetch(
    http: reqwest::Client,
    report: impl Fn(Progress) + Send + 'static,
) -> Result<PathBuf, String> {
    let url = TensorRt::download_url().ok_or("TensorRT-RTX has no build for this platform")?;
    let target = TensorRt::runtime_dir().ok_or("there is no data folder to put it in")?;
    let parent = target.parent().ok_or("the data folder has no parent")?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| e.to_string())?;

    let archive = parent.join(if url.ends_with(".zip") {
        "tensorrt-rtx.zip"
    } else {
        "tensorrt-rtx.tar.gz"
    });

    // NVIDIA's link answers with a redirect to its download host; reqwest
    // follows it. No timeout on the whole: it is a large file.
    let response = http
        .get(url)
        .timeout(std::time::Duration::from_secs(3600))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?;
    let total = response.content_length();
    let mut file = tokio::fs::File::create(&archive)
        .await
        .map_err(|e| e.to_string())?;
    let mut done = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        report(Progress::Fetching { done, total });
    }
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|e| e.to_string())?;
    drop(file);

    report(Progress::Unpacking);
    let staging = parent.join("tensorrt-rtx.partial");
    let unpacked = {
        let archive = archive.clone();
        let staging = staging.clone();
        tokio::task::spawn_blocking(move || unpack(&archive, &staging))
            .await
            .map_err(|e| e.to_string())?
    };
    let _ = tokio::fs::remove_file(&archive).await;
    unpacked?;

    if !TensorRt::is_runtime(&staging) {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err("the archive did not hold TensorRT-RTX as expected".to_owned());
    }
    if tokio::fs::metadata(&target).await.is_ok() {
        tokio::fs::remove_dir_all(&target)
            .await
            .map_err(|e| e.to_string())?;
    }
    tokio::fs::rename(&staging, &target)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(dir = %target.display(), "TensorRT-RTX installed");
    Ok(target)
}

/// Unpacks `archive` into `into`, without the archive's own top folder.
fn unpack(archive: &Path, into: &Path) -> Result<(), String> {
    if into.exists() {
        std::fs::remove_dir_all(into).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;

    if archive.extension().is_some_and(|ext| ext == "zip") {
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
            // `enclosed_name` refuses paths that would climb out of `into`.
            let Some(path) = entry.enclosed_name() else {
                continue;
            };
            let Some(relative) = without_top(&path) else {
                continue;
            };
            let out = into.join(relative);
            if entry.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
            entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            std::fs::write(&out, bytes).map_err(|e| e.to_string())?;
        }
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        tar.set_preserve_permissions(true);
        for entry in tar.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path().map_err(|e| e.to_string())?.into_owned();
            let Some(relative) = without_top(&path) else {
                continue;
            };
            // Only what NVIDIA's runtime needs; the samples and the Python
            // wheels in the archive are left behind.
            if !(relative.starts_with("lib") || relative.starts_with("bin")) {
                continue;
            }
            let out = into.join(&relative);
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            entry.unpack(&out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// `path` without its first component, or `None` for the top folder itself
/// and for anything that is not a plain path downwards — no `..`, no root —
/// so nothing can be written outside the folder it is unpacked into.
fn without_top(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    components.next()?;
    let rest: PathBuf = components.collect();
    let plain = rest
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)));
    (plain && !rest.as_os_str().is_empty()).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_archive_top_folder_is_dropped() {
        assert_eq!(
            without_top(Path::new("TensorRT-RTX-1.3.0.35/lib/libtensorrt_rtx.so.1")),
            Some(PathBuf::from("lib/libtensorrt_rtx.so.1"))
        );
        assert_eq!(without_top(Path::new("TensorRT-RTX-1.3.0.35/")), None);
        assert_eq!(without_top(Path::new("top/../../etc/passwd")), None);
    }

    /// A small tar.gz shaped like NVIDIA's unpacks into lib/ and bin/ only.
    #[test]
    fn a_tarball_unpacks_its_runtime_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("anirust-trt-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let archive = dir.join("t.tar.gz");
        {
            let file = std::fs::File::create(&archive).expect("archive");
            let gz = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
            let mut builder = tar::Builder::new(gz);
            for (path, body) in [
                ("TensorRT-RTX-1.3/lib/libtensorrt_rtx.so.1", "lib"),
                ("TensorRT-RTX-1.3/bin/tensorrt_rtx", "bin"),
                ("TensorRT-RTX-1.3/samples/readme", "skip"),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_size(body.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder
                    .append_data(&mut header, path, body.as_bytes())
                    .expect("an entry");
            }
            builder.into_inner().expect("tar").finish().expect("gz");
        }
        let into = dir.join("out");
        unpack(&archive, &into).expect("unpacked");
        assert!(into.join("lib/libtensorrt_rtx.so.1").is_file());
        assert!(TensorRt::is_runtime(&into));
        assert!(!into.join("samples").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
