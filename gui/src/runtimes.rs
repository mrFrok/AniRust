// SPDX-License-Identifier: GPL-3.0-or-later

//! Fetching a vendor's runtime for the vs-mlrt engine: NVIDIA's TensorRT-RTX
//! or Intel's OpenVINO.
//!
//! The program ships its own part of that engine — the ported plugins,
//! vsmlrt and the networks, all under open licences — but not the vendors'
//! runtimes: TensorRT-RTX is NVIDIA's and licensed by NVIDIA, and OpenVINO,
//! though Apache-2.0, is large enough to be fetched only by those who want
//! it. The person watching fetches either from its vendor when they ask. This
//! does the fetching: the vendor's archive for this platform, unpacked into
//! the data folder ([`Backend::runtime_dir`]), the archive's own top folder
//! dropped, and only the parts the runtime needs kept.
//!
//! It is written to a folder beside the final one and moved into place only
//! once whole, so an interrupted download never looks like an install.

use std::io::Read;
use std::path::{Path, PathBuf};

use anirust_player::Backend;
use futures::StreamExt;

/// How far a fetch has got, for the line on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Fetching { done: u64, total: Option<u64> },
    Unpacking,
}

/// Fetches and unpacks `backend`'s runtime, reporting along the way.
/// Answers with the folder it is now in.
pub async fn fetch(
    http: reqwest::Client,
    backend: Backend,
    report: impl Fn(Progress) + Send + 'static,
) -> Result<PathBuf, String> {
    let url = backend
        .download_url()
        .ok_or("this runtime has no build to fetch for this platform")?;
    let target = backend
        .runtime_dir()
        .ok_or("there is no data folder to put it in")?;
    let parent = target.parent().ok_or("the data folder has no parent")?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| e.to_string())?;

    let name = target.file_name().map_or_else(
        || "runtime".into(),
        |name| name.to_string_lossy().into_owned(),
    );
    let archive = parent.join(if url.ends_with(".zip") {
        format!("{name}.zip")
    } else {
        format!("{name}.tar.gz")
    });

    // NVIDIA's link answers with a redirect to its download host; reqwest
    // follows it. No short timeout on the whole: it is a large file.
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
    let staging = parent.join(format!("{name}.partial"));
    let unpacked = {
        let archive = archive.clone();
        let staging = staging.clone();
        let keep = backend.kept_parts();
        tokio::task::spawn_blocking(move || unpack(&archive, &staging, keep))
            .await
            .map_err(|e| e.to_string())?
    };
    let _ = tokio::fs::remove_file(&archive).await;
    unpacked?;

    if !backend.is_runtime(&staging) {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err("the archive did not hold the runtime as expected".to_owned());
    }
    if tokio::fs::metadata(&target).await.is_ok() {
        tokio::fs::remove_dir_all(&target)
            .await
            .map_err(|e| e.to_string())?;
    }
    tokio::fs::rename(&staging, &target)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(?backend, dir = %target.display(), "runtime installed");
    Ok(target)
}

/// Unpacks `archive` into `into`, without the archive's own top folder, and
/// only the top folders named in `keep`.
fn unpack(archive: &Path, into: &Path, keep: &[&str]) -> Result<(), String> {
    let kept = |relative: &Path| keep.iter().any(|part| relative.starts_with(part));
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
            if !kept(&relative) {
                continue;
            }
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
            // Only what the runtime needs; the samples and the Python wheels
            // in the archives are left behind.
            if !kept(&relative) {
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
pub(crate) fn without_top(path: &Path) -> Option<PathBuf> {
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
        unpack(&archive, &into, Backend::TensorRt.kept_parts()).expect("unpacked");
        assert!(into.join("lib/libtensorrt_rtx.so.1").is_file());
        assert!(Backend::TensorRt.is_runtime(&into));
        assert!(!into.join("samples").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
