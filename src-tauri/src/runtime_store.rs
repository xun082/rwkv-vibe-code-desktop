//! Fetch the pinned upstream Albatross runtime into per-user app data.

use crate::model_store::{ModelSpec, ModelStore, Notify, Progress, Source};
use flate2::read::GzDecoder;
use std::{
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub const VERSION: &str = "1.8.15";
const CUDA: &str = "12.9";
const VOCABULARY: &str = "rwkv_vocab_v20230424.txt";
const SOURCE: &str = "Albatross 运行时";
const MARKER: &str = ".archive-sha256";

#[derive(Clone, Copy)]
enum Archive {
    Linux,
    Windows,
}

impl Archive {
    fn host() -> Option<Self> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Some(Self::Linux),
            ("windows", "x86_64") => Some(Self::Windows),
            _ => None,
        }
    }

    fn executable(self) -> &'static str {
        match self {
            Self::Linux => "rwkv_lighting_cuda",
            Self::Windows => "rwkv_lighting_cuda.exe",
        }
    }

    fn spec(self) -> ModelSpec {
        // Canonical GitHub release asset sizes and upstream SHA-256 digests.
        let (system, extension, bytes, sha256) = match self {
            Self::Linux => (
                "linux",
                "tar.gz",
                986_150_311,
                "a1fc44038f8e1e3a477bc2af128fa2ff3a2ae1c351a3bc4c4b1a238e987cb6f6",
            ),
            Self::Windows => (
                "windows",
                "zip",
                612_780_108,
                "0b6f5dee7e2a9b18cb6e001286d059c02aac939e18f8b75e6d89320782fd11dd",
            ),
        };
        let filename = format!("rwkv-lightning-v{VERSION}-{system}-x64-cuda{CUDA}.{extension}");
        let url = format!(
            "https://github.com/Alic-Li/rwkv_lightning_cuda/releases/download/v{VERSION}/{filename}"
        );
        ModelSpec {
            filename,
            bytes,
            sha256: sha256.into(),
            sources: vec![Source {
                name: SOURCE.into(),
                url,
            }],
        }
    }

    fn select(self, path: &Path) -> Option<PathBuf> {
        let relative = path.strip_prefix("bundle").ok()?;
        if relative == Path::new(self.executable()) || relative == Path::new(VOCABULARY) {
            return Some(relative.into());
        }
        let bundled_library = match self {
            Self::Linux => relative.starts_with("lib"),
            Self::Windows => {
                relative.components().count() == 1
                    && relative
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
            }
        };
        bundled_library.then(|| relative.into())
    }

    fn runtime(self, directory: PathBuf) -> Option<Runtime> {
        let executable = directory.join(self.executable());
        let vocabulary = directory.join(VOCABULARY);
        let library_directory = match self {
            Self::Linux => directory.join("lib"),
            Self::Windows => directory.clone(),
        };
        let has_cuda = fs::read_dir(library_directory)
            .ok()?
            .flatten()
            .any(|entry| {
                entry.file_name().to_str().is_some_and(|name| {
                    (name.starts_with("libcudart.so") || name.starts_with("cudart64_"))
                        && entry.path().is_file()
                })
            });
        if !executable.is_file() || !vocabulary.is_file() || !has_cuda {
            return None;
        }
        #[cfg(unix)]
        if matches!(self, Self::Linux) {
            use std::os::unix::fs::PermissionsExt;
            if executable.metadata().ok()?.permissions().mode() & 0o111 == 0 {
                return None;
            }
        }
        Some(Runtime {
            executable,
            vocabulary,
            #[cfg(target_os = "linux")]
            directory,
        })
    }
}

#[derive(Debug)]
pub struct Runtime {
    pub executable: PathBuf,
    pub vocabulary: PathBuf,
    #[cfg(target_os = "linux")]
    pub directory: PathBuf,
}

fn cache(directory: &Path) -> PathBuf {
    directory
        .join("albatross")
        .join(format!("v{VERSION}-cuda{CUDA}"))
}

pub fn installed(directory: &Path) -> Option<Runtime> {
    let archive = Archive::host()?;
    let runtime = cache(directory).join("runtime");
    let digest = fs::read_to_string(runtime.join(MARKER)).ok()?;
    if digest != archive.spec().sha256 {
        return None;
    }
    archive.runtime(runtime)
}

pub async fn prepare(
    directory: &Path,
    cancel: &CancellationToken,
    notify: &Notify,
) -> Result<Runtime, String> {
    let archive = Archive::host().ok_or("本机 Albatross 支持 Windows 或 Linux x64")?;
    check_cancel(cancel).map_err(|error| error.to_string())?;
    let cache = cache(directory);
    fs::create_dir_all(&cache).map_err(install_error)?;
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(cache.join(".runtime-install.lock"))
        .map_err(install_error)?;
    lock.try_lock()
        .map_err(|_| "另一个应用实例正在准备 Albatross 运行时，请稍后重试".to_string())?;
    if let Some(runtime) = installed(directory) {
        return Ok(runtime);
    }

    let spec = archive.spec();
    let checksum = spec.sha256.clone();
    let size = spec.bytes;
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .user_agent(concat!("RWKVVibeCode/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())?;
    let download_progress: Notify = {
        let notify = notify.clone();
        Arc::new(move |mut progress| {
            if matches!(progress.phase, "checking" | "downloading" | "verifying") {
                progress.source = SOURCE.into();
                progress.message = progress.message.replace("模型", SOURCE);
                notify(progress);
            }
        })
    };
    let downloaded = ModelStore::new(spec, client)
        .prepare_with_cancel(cache.clone(), vec![], download_progress, cancel.clone())
        .await
        .map_err(|error| error.replace("模型", SOURCE))?;
    notify(Progress {
        phase: "importing",
        downloaded_bytes: size,
        total_bytes: size,
        speed_bps: 0.0,
        source: SOURCE.into(),
        message: "正在解压 Albatross 运行时".into(),
    });

    let cancel = cancel.clone();
    tokio::task::spawn_blocking(move || {
        // Keep the cross-process lock alive through extraction, including if the
        // asynchronous caller leaves while the blocking worker is running.
        let _lock = lock;
        let staging = cache.join(".extracting");
        let target = cache.join("runtime");
        remove_directory(&staging).map_err(install_error)?;
        fs::create_dir(&staging).map_err(install_error)?;
        let result = (|| {
            extract(archive, &downloaded, &staging, &cancel)?;
            archive
                .runtime(staging.clone())
                .ok_or_else(|| invalid_archive("缺少推理程序、词表或 CUDA 动态库"))?;
            fs::write(staging.join(MARKER), checksum)?;
            check_cancel(&cancel)?;
            remove_directory(&target)?;
            fs::rename(&staging, &target)?;
            archive
                .runtime(target)
                .ok_or_else(|| invalid_archive("无法读取已解压运行时"))
        })();
        if result.is_ok() {
            // After a successful install the extracted files are the cache. A
            // cancelled or failed extraction keeps the verified archive to retry.
            let _ = fs::remove_file(downloaded);
        } else {
            let _ = remove_directory(&staging);
        }
        result.map_err(install_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn check_cancel(cancel: &CancellationToken) -> io::Result<()> {
    if cancel.is_cancelled() {
        return Err(io::Error::other("Albatross 运行时准备已暂停"));
    }
    Ok(())
}

fn install_error(error: io::Error) -> String {
    format!("无法安装 Albatross 运行时：{error}")
}

fn invalid_archive(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn remove_directory(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn copy_file(mut source: impl Read, target: &Path, cancel: &CancellationToken) -> io::Result<()> {
    fs::create_dir_all(target.parent().expect("staged file parent"))?;
    let mut destination =
        BufWriter::new(File::options().write(true).create_new(true).open(target)?);
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        check_cancel(cancel)?;
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        destination.write_all(&buffer[..count])?;
    }
    destination.flush()
}

fn extract(
    archive: Archive,
    path: &Path,
    staging: &Path,
    cancel: &CancellationToken,
) -> io::Result<()> {
    let file = File::open(path)?;
    match archive {
        Archive::Windows => {
            let mut zip = zip::ZipArchive::new(file)?;
            for index in 0..zip.len() {
                check_cancel(cancel)?;
                let entry = zip.by_index(index)?;
                let path = entry
                    .enclosed_name()
                    .filter(|path| safe_relative(path))
                    .ok_or_else(|| invalid_archive("ZIP 中包含不安全的路径"))?;
                if let Some(relative) = archive.select(&path) {
                    if !entry.is_file() {
                        return Err(invalid_archive("运行时 ZIP 包含意外的文件类型"));
                    }
                    copy_file(entry, &staging.join(relative), cancel)?;
                }
            }
        }
        Archive::Linux => {
            let mut tar = tar::Archive::new(GzDecoder::new(file));
            let mut symlinks = Vec::new();
            for entry in tar.entries()? {
                check_cancel(cancel)?;
                let entry = entry?;
                let path = entry.path()?;
                if !safe_relative(&path) {
                    return Err(invalid_archive("TAR 中包含不安全的路径"));
                }
                let Some(relative) = archive.select(&path) else {
                    continue;
                };
                let kind = entry.header().entry_type();
                let target = staging.join(&relative);
                if kind.is_dir() && relative.starts_with("lib") {
                    fs::create_dir_all(target)?;
                } else if kind.is_file() {
                    let mode = entry.header().mode()?;
                    copy_file(entry, &target, cancel)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&target, fs::Permissions::from_mode(mode & 0o777))?;
                    }
                    #[cfg(not(unix))]
                    let _ = mode;
                } else if kind.is_symlink()
                    && relative.starts_with("lib")
                    && relative != Path::new("lib")
                {
                    let link = entry
                        .link_name()?
                        .filter(|path| safe_relative(path))
                        .ok_or_else(|| invalid_archive("运行时动态库链接超出安装目录"))?;
                    symlinks.push((link.into_owned(), target));
                } else {
                    return Err(invalid_archive("运行时 TAR 包含意外的文件类型"));
                }
            }
            // Defer links until files are written, keeping every archive write
            // independent of links supplied by earlier archive entries.
            for (link, target) in &symlinks {
                check_cancel(cancel)?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(link, target)?;
                #[cfg(not(unix))]
                std::os::windows::fs::symlink_file(link, target)?;
            }
            if symlinks.iter().any(|(_, target)| !target.is_file()) {
                return Err(invalid_archive("运行时包含断开的动态库链接"));
            }
        }
    }
    check_cancel(cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("albatross-runtime-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn windows_zip_keeps_shared_libraries_beside_the_executable() {
        let temp = TestDirectory::new();
        let archive = temp.0.join("runtime.zip");
        let stage = temp.0.join("stage");
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for name in [
            "bundle/rwkv_lighting_cuda.exe",
            "bundle/rwkv_vocab_v20230424.txt",
            "bundle/cudart64_12.dll",
            "bundle/rwkv_state_tune.exe",
            "bundle/tools/dummy.dll",
        ] {
            writer.start_file(name, options).unwrap();
            writer.write_all(b"test data").unwrap();
        }
        writer.finish().unwrap();
        extract(
            Archive::Windows,
            &archive,
            &stage,
            &CancellationToken::new(),
        )
        .unwrap();
        let runtime = Archive::Windows.runtime(stage.clone()).unwrap();
        assert_eq!(runtime.executable, stage.join("rwkv_lighting_cuda.exe"));
        assert_eq!(runtime.vocabulary, stage.join(VOCABULARY));
        assert!(stage.join("cudart64_12.dll").is_file());
        assert!(!stage.join("rwkv_state_tune.exe").exists());
        assert!(!stage.join("tools").exists());
    }

    #[cfg(unix)]
    fn linux_fixture(archive: &Path, link: &str) {
        let encoder = flate2::write::GzEncoder::new(
            File::create(archive).unwrap(),
            flate2::Compression::fast(),
        );
        let mut writer = tar::Builder::new(encoder);
        for (name, mode) in [
            ("bundle/rwkv_lighting_cuda", 0o755),
            ("bundle/rwkv_vocab_v20230424.txt", 0o644),
            ("bundle/lib/libcudart.so.12.9.0", 0o644),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(1);
            header.set_mode(mode);
            writer.append_data(&mut header, name, &b"x"[..]).unwrap();
        }
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        writer
            .append_link(&mut header, "bundle/lib/libcudart.so.12", link)
            .unwrap();
        writer.into_inner().unwrap().finish().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn linux_tar_preserves_exec_permissions_and_safe_relative_library_links() {
        let temp = TestDirectory::new();
        let archive = temp.0.join("runtime.tar.gz");
        let stage = temp.0.join("stage");
        linux_fixture(&archive, "libcudart.so.12.9.0");
        extract(Archive::Linux, &archive, &stage, &CancellationToken::new()).unwrap();
        assert!(Archive::Linux.runtime(stage.clone()).is_some());
        let library = stage.join("lib/libcudart.so.12");
        assert_eq!(
            fs::read_link(&library).unwrap(),
            Path::new("libcudart.so.12.9.0")
        );
        assert_eq!(fs::read(library).unwrap(), b"x");
    }

    #[cfg(unix)]
    #[test]
    fn linux_tar_rejects_escaping_library_links() {
        let temp = TestDirectory::new();
        let archive = temp.0.join("runtime.tar.gz");
        let stage = temp.0.join("stage");
        linux_fixture(&archive, "../outside");
        assert!(extract(Archive::Linux, &archive, &stage, &CancellationToken::new()).is_err());
        assert!(!stage.join("lib/libcudart.so.12").exists());
    }
}
