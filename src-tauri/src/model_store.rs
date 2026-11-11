//! Model weights live in Application Support, never inside the signed app bundle.
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Deserialize)]
pub struct Source {
    pub name: String,
    pub url: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct ModelSpec {
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
    pub sources: Vec<Source>,
}
impl ModelSpec {
    pub fn production() -> Self {
        serde_json::from_str(include_str!("../model-manifest.json")).expect("pinned model manifest")
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: &'static str,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub source: String,
    pub message: String,
}
pub type Notify = Arc<dyn Fn(Progress) + Send + Sync>;
#[derive(Clone, PartialEq)]
struct Fingerprint {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}
fn fingerprint(path: &Path) -> Result<Fingerprint, String> {
    let m = std::fs::metadata(path).map_err(|e| format!("无法读取模型文件：{e}"))?;
    Ok(Fingerprint {
        path: path.into(),
        bytes: m.len(),
        modified: m.modified().map_err(|e| e.to_string())?,
    })
}
fn io_error(e: std::io::Error) -> String {
    format!("无法写入本地模型，请检查磁盘空间和目录权限：{e}")
}
fn check_cancel(token: &CancellationToken) -> Result<(), String> {
    if token.is_cancelled() {
        Err("模型准备已暂停".into())
    } else {
        Ok(())
    }
}
fn emit(
    spec: &ModelSpec,
    notify: &Notify,
    phase: &'static str,
    bytes: u64,
    source: &str,
    message: &str,
    speed: f64,
) {
    notify(Progress {
        phase,
        downloaded_bytes: bytes,
        total_bytes: spec.bytes,
        speed_bps: speed,
        source: source.into(),
        message: message.into(),
    });
}
async fn verify(path: PathBuf, spec: ModelSpec, token: CancellationToken) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        check_cancel(&token)?;
        let mut file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.to_string()),
        };
        if file.metadata().map_err(|e| e.to_string())?.len() != spec.bytes {
            return Ok(false);
        }
        let mut hash = Sha256::new();
        let mut buffer = vec![0u8; 4 * 1024 * 1024];
        loop {
            check_cancel(&token)?;
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        Ok(format!("{:x}", hash.finalize()) == spec.sha256)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub struct ModelStore {
    pub spec: ModelSpec,
    gate: tokio::sync::Mutex<()>,
    active: Mutex<Option<CancellationToken>>,
    verified: Mutex<Option<Fingerprint>>,
    client: Client,
}
impl ModelStore {
    pub fn production() -> Self {
        Self::new(
            ModelSpec::production(),
            Client::builder()
                .https_only(true)
                .no_proxy()
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(30))
                .user_agent(concat!(
                    "RWKVVibeCode/",
                    env!("CARGO_PKG_VERSION"),
                    " model-installer"
                ))
                .build()
                .expect("model download client"),
        )
    }
    pub fn new(spec: ModelSpec, client: Client) -> Self {
        Self {
            spec,
            gate: tokio::sync::Mutex::new(()),
            active: Mutex::new(None),
            verified: Mutex::new(None),
            client,
        }
    }
    pub fn cancel(&self) {
        if let Some(token) = self.active.lock().unwrap().as_ref() {
            token.cancel();
        }
    }
    pub async fn installed_model(&self, directory: &Path) -> Result<PathBuf, String> {
        let path = directory.join(&self.spec.filename);
        if let Ok(current) = fingerprint(&path) {
            if self.verified.lock().unwrap().as_ref() == Some(&current) {
                return Ok(path);
            }
        }
        if verify(path.clone(), self.spec.clone(), CancellationToken::new()).await? {
            *self.verified.lock().unwrap() = Some(fingerprint(&path)?);
            Ok(path)
        } else {
            Err("本地模型尚未安装或校验未通过，请先完成模型下载".into())
        }
    }
    pub async fn prepare(
        &self,
        directory: PathBuf,
        existing: Vec<PathBuf>,
        notify: Notify,
    ) -> Result<PathBuf, String> {
        let _gate = self.gate.lock().await;
        let token = CancellationToken::new();
        *self.active.lock().unwrap() = Some(token.clone());
        let result = self
            .prepare_inner(&directory, &existing, &notify, &token)
            .await;
        *self.active.lock().unwrap() = None;
        if let Err(error) = &result {
            emit(
                &self.spec,
                &notify,
                if token.is_cancelled() {
                    "paused"
                } else {
                    "error"
                },
                partial_length(&directory, &self.spec).await,
                "",
                error,
                0.0,
            );
        }
        result
    }
    async fn prepare_inner(
        &self,
        directory: &Path,
        existing: &[PathBuf],
        notify: &Notify,
        token: &CancellationToken,
    ) -> Result<PathBuf, String> {
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(io_error)?;
        // OS file lock is released on crash/exit; no stale PID lock or concurrent
        // app instance can append to the same partial download.
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(".install.lock"))
            .map_err(io_error)?;
        lock.try_lock()
            .map_err(|_| "另一个应用实例正在准备模型，请稍后重试".to_string())?;
        let path = directory.join(&self.spec.filename);
        let part = directory.join(format!("{}.part", self.spec.filename));
        emit(
            &self.spec,
            notify,
            "checking",
            0,
            "",
            "正在检查本地模型",
            0.0,
        );
        if self.installed_model(directory).await.is_ok() {
            emit(
                &self.spec,
                notify,
                "ready",
                self.spec.bytes,
                "",
                "本地模型已就绪",
                0.0,
            );
            return Ok(path);
        }
        check_cancel(token)?;
        for candidate in existing {
            if candidate == &path || !candidate.is_file() {
                continue;
            }
            emit(
                &self.spec,
                notify,
                "checking",
                0,
                "已有本地文件",
                "正在校验已有模型",
                0.0,
            );
            if verify(candidate.clone(), self.spec.clone(), token.clone()).await? {
                emit(
                    &self.spec,
                    notify,
                    "importing",
                    0,
                    "已有本地文件",
                    "正在复用已有模型，无需重新下载",
                    0.0,
                );
                tokio::fs::copy(candidate, &part).await.map_err(io_error)?;
                if self.finish(&part, &path, notify, token).await? {
                    return Ok(path);
                }
            }
        }
        if partial_length(directory, &self.spec).await > self.spec.bytes {
            tokio::fs::File::create(&part).await.map_err(io_error)?;
        }
        if partial_length(directory, &self.spec).await == self.spec.bytes
            && self.finish(&part, &path, notify, token).await?
        {
            return Ok(path);
        }
        let mut errors = Vec::new();
        for source in &self.spec.sources {
            for _ in 0..2 {
                check_cancel(token)?;
                match self.transfer(&part, source, notify, token).await {
                    Ok(()) => {
                        if self.finish(&part, &path, notify, token).await? {
                            return Ok(path);
                        } else {
                            errors.push(format!("{}：SHA-256 校验不匹配", source.name));
                            break;
                        }
                    }
                    Err(error) => {
                        check_cancel(token)?;
                        errors.push(format!("{}：{error}", source.name));
                    }
                }
            }
        }
        Err(format!(
            "模型下载暂未完成，可重试并继续下载。{}",
            errors.last().cloned().unwrap_or_default()
        ))
    }
    async fn finish(
        &self,
        part: &Path,
        path: &Path,
        notify: &Notify,
        token: &CancellationToken,
    ) -> Result<bool, String> {
        emit(
            &self.spec,
            notify,
            "verifying",
            self.spec.bytes,
            "",
            "正在校验模型完整性",
            0.0,
        );
        if !verify(part.into(), self.spec.clone(), token.clone()).await? {
            // Never promote a corrupt payload or a proxy HTML error page.
            tokio::fs::File::create(part).await.map_err(io_error)?;
            return Ok(false);
        }
        check_cancel(token)?;
        tokio::fs::rename(part, path).await.map_err(io_error)?;
        *self.verified.lock().unwrap() = Some(fingerprint(path)?);
        emit(
            &self.spec,
            notify,
            "ready",
            self.spec.bytes,
            "",
            "本地模型已就绪",
            0.0,
        );
        Ok(true)
    }
    async fn transfer(
        &self,
        part: &Path,
        source: &Source,
        notify: &Notify,
        token: &CancellationToken,
    ) -> Result<(), String> {
        let mut offset = tokio::fs::metadata(part)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        emit(
            &self.spec,
            notify,
            "downloading",
            offset,
            &source.name,
            "正在连接下载源",
            0.0,
        );
        let response = tokio::select! {
            _=token.cancelled()=>return Err("模型准备已暂停".into()),
            response=self.client.get(&source.url).header(header::RANGE,format!("bytes={offset}-{}",self.spec.bytes-1)).header(header::ACCEPT_ENCODING,"identity").send()=>response.map_err(|e|format!("连接失败：{e}"))?,
        };
        let status = response.status();
        if status == StatusCode::PARTIAL_CONTENT {
            validate_range(
                response
                    .headers()
                    .get(header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok()),
                offset,
                self.spec.bytes,
            )?;
        } else if status == StatusCode::OK {
            // A source may ignore Range. Restart with that full response rather
            // than silently appending an entire file to an existing prefix.
            if response.content_length() != Some(self.spec.bytes) {
                return Err("下载源返回的文件大小不正确".into());
            }
            offset = 0;
        } else {
            return Err(format!("下载源返回 HTTP {status}"));
        }
        if response
            .headers()
            .get(header::CONTENT_ENCODING)
            .is_some_and(|v| v != "identity")
        {
            return Err("下载源返回了不支持的压缩内容".into());
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(offset > 0)
            .truncate(offset == 0)
            .open(part)
            .await
            .map_err(io_error)?;
        let initial = offset;
        let started = Instant::now();
        let mut last = Instant::now() - Duration::from_secs(1);
        let mut stream = response.bytes_stream();
        loop {
            let next = tokio::select! {
                _=token.cancelled()=>{file.flush().await.map_err(io_error)?;return Err("模型准备已暂停".into())},
                next=stream.next()=>next,
            };
            let Some(chunk) = next else { break };
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(error) => {
                    // Finish pending file writes before the next attempt reads the offset.
                    file.flush().await.map_err(io_error)?;
                    return Err(format!("网络中断：{error}"));
                }
            };
            if offset + bytes.len() as u64 > self.spec.bytes {
                file.flush().await.map_err(io_error)?;
                return Err("下载数据超过模型预期大小".into());
            }
            file.write_all(&bytes).await.map_err(io_error)?;
            offset += bytes.len() as u64;
            if last.elapsed() >= Duration::from_millis(200) || offset == self.spec.bytes {
                emit(
                    &self.spec,
                    notify,
                    "downloading",
                    offset,
                    &source.name,
                    "正在下载模型",
                    (offset - initial) as f64 / started.elapsed().as_secs_f64().max(0.001),
                );
                last = Instant::now();
            }
        }
        file.flush().await.map_err(io_error)?;
        file.sync_all().await.map_err(io_error)?;
        if offset != self.spec.bytes {
            return Err("下载连接提前结束，将保留断点继续下载".into());
        }
        Ok(())
    }
}
async fn partial_length(directory: &Path, spec: &ModelSpec) -> u64 {
    tokio::fs::metadata(directory.join(format!("{}.part", spec.filename)))
        .await
        .map(|m| m.len())
        .unwrap_or(0)
}
fn validate_range(value: Option<&str>, offset: u64, total: u64) -> Result<(), String> {
    let values = value
        .and_then(|s| s.strip_prefix("bytes "))
        .and_then(|s| s.split_once('/'))
        .and_then(|(range, total)| {
            range
                .split_once('-')
                .map(|(start, end)| (start, end, total))
        });
    let Some((start, end, size)) = values else {
        return Err("下载源缺少正确的断点续传信息".into());
    };
    let (Ok(start), Ok(end), Ok(size)) = (
        start.parse::<u64>(),
        end.parse::<u64>(),
        size.parse::<u64>(),
    ) else {
        return Err("下载源的范围信息无效".into());
    };
    if start != offset || size != total || end < start || end >= total {
        return Err("下载源返回了错误的文件范围".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("rwkv-install-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
    fn spec(bytes: &[u8], urls: Vec<String>) -> ModelSpec {
        ModelSpec {
            filename: "model.gguf".into(),
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            sources: urls
                .into_iter()
                .enumerate()
                .map(|(i, url)| Source {
                    name: format!("source-{i}"),
                    url,
                })
                .collect(),
        }
    }
    fn notify() -> Notify {
        Arc::new(|_| {})
    }
    enum Reply {
        Range,
        Full,
        BadRange,
        Corrupt,
        Slow,
    }
    fn server(
        data: Vec<u8>,
        plans: Vec<Reply>,
    ) -> (String, Arc<Mutex<Vec<u64>>>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let offsets = Arc::new(Mutex::new(Vec::new()));
        let recorded = offsets.clone();
        let handle = std::thread::spawn(move || {
            for plan in plans {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buf = [0; 1024];
                while !request.windows(4).any(|s| s == b"\r\n\r\n") {
                    let n = socket.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }
                let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                let start: u64 = request
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("range: bytes=")
                            .and_then(|v| v.split('-').next())
                            .map(|n| n.parse().unwrap())
                    })
                    .unwrap();
                recorded.lock().unwrap().push(start);
                let total = data.len();
                let actual = if matches!(plan, Reply::Full | Reply::BadRange | Reply::Corrupt) {
                    0
                } else {
                    start as usize
                };
                let payload = if matches!(plan, Reply::Corrupt) {
                    vec![b'X'; total]
                } else {
                    data[actual..].to_vec()
                };
                let header = if matches!(plan, Reply::Full | Reply::Corrupt) {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        payload.len()
                    )
                } else {
                    format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",payload.len(),actual,total-1,total)
                };
                socket.write_all(header.as_bytes()).unwrap();
                if matches!(plan, Reply::Slow) {
                    for chunk in payload.chunks(4096) {
                        if socket.write_all(chunk).is_err() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                } else {
                    let _ = socket.write_all(&payload);
                }
            }
        });
        (url, offsets, handle)
    }
    fn client() -> Client {
        Client::builder().no_proxy().build().unwrap()
    }
    #[tokio::test]
    async fn resumes_exact_range_then_reuses_cache_offline() {
        let data = b"GGUF0123456789abcdef".to_vec();
        let (url, offsets, thread) = server(data.clone(), vec![Reply::Range]);
        let store = ModelStore::new(spec(&data, vec![url]), client());
        let root = root();
        std::fs::write(root.join("model.gguf.part"), &data[..7]).unwrap();
        let path = store.prepare(root.clone(), vec![], notify()).await.unwrap();
        thread.join().unwrap();
        assert_eq!(*offsets.lock().unwrap(), vec![7]);
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert!(!root.join("model.gguf.part").exists());
        let fresh = ModelStore::new(store.spec.clone(), client());
        assert_eq!(
            fresh.prepare(root.clone(), vec![], notify()).await.unwrap(),
            path
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn ignored_range_restarts_instead_of_appending_duplicate_bytes() {
        let data = b"GGUF complete file".to_vec();
        let (url, _, thread) = server(data.clone(), vec![Reply::Full]);
        let store = ModelStore::new(spec(&data, vec![url]), client());
        let root = root();
        std::fs::write(root.join("model.gguf.part"), b"wrong").unwrap();
        let path = store.prepare(root.clone(), vec![], notify()).await.unwrap();
        thread.join().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), data);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn bad_ranges_do_not_corrupt_the_prefix_and_fail_over() {
        let data = b"GGUF range safe file".to_vec();
        let (url, offsets, thread) = server(
            data.clone(),
            vec![Reply::BadRange, Reply::BadRange, Reply::Range],
        );
        let store = ModelStore::new(spec(&data, vec![url.clone(), url]), client());
        let root = root();
        std::fs::write(root.join("model.gguf.part"), &data[..6]).unwrap();
        let path = store.prepare(root.clone(), vec![], notify()).await.unwrap();
        thread.join().unwrap();
        assert_eq!(*offsets.lock().unwrap(), vec![6, 6, 6]);
        assert_eq!(std::fs::read(path).unwrap(), data);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn hash_mismatch_is_never_installed_and_next_source_can_recover() {
        let data = b"GGUF original data".to_vec();
        let (url, offsets, thread) = server(data.clone(), vec![Reply::Corrupt, Reply::Range]);
        let store = ModelStore::new(spec(&data, vec![url.clone(), url]), client());
        let root = root();
        let path = store.prepare(root.clone(), vec![], notify()).await.unwrap();
        thread.join().unwrap();
        assert_eq!(*offsets.lock().unwrap(), vec![0, 0]);
        assert_eq!(std::fs::read(path).unwrap(), data);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn pause_keeps_a_resumable_partial_file() {
        let data = vec![42; 65536];
        let (url, offsets, thread) = server(data.clone(), vec![Reply::Slow, Reply::Range]);
        let store = Arc::new(ModelStore::new(spec(&data, vec![url]), client()));
        let root = root();
        let cancel = store.clone();
        let result = store
            .prepare(
                root.clone(),
                vec![],
                Arc::new(move |p| {
                    if p.phase == "downloading" && p.downloaded_bytes > 0 {
                        cancel.cancel();
                    }
                }),
            )
            .await;
        assert!(result.is_err());
        let partial = std::fs::metadata(root.join("model.gguf.part"))
            .unwrap()
            .len();
        assert!(partial > 0 && partial < data.len() as u64);
        assert!(!root.join("model.gguf").exists());
        let path = store.prepare(root.clone(), vec![], notify()).await.unwrap();
        thread.join().unwrap();
        assert_eq!(offsets.lock().unwrap()[1], partial);
        assert_eq!(std::fs::read(path).unwrap(), data);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn existing_local_model_is_imported_without_network() {
        let data = b"GGUF existing local model";
        let original = root();
        let source = original.join("existing.gguf");
        std::fs::write(&source, data).unwrap();
        let root = root();
        let store = ModelStore::new(
            spec(data, vec!["http://127.0.0.1:1/offline".into()]),
            client(),
        );
        let installed = store
            .prepare(root.clone(), vec![source.clone()], notify())
            .await
            .unwrap();
        assert_eq!(std::fs::read(installed).unwrap(), data);
        assert!(source.exists());
        std::fs::remove_dir_all(original).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn validates_range_start_size_and_end() {
        assert!(validate_range(Some("bytes 4-9/10"), 4, 10).is_ok());
        for value in [
            None,
            Some("bytes 0-9/10"),
            Some("bytes 4-9/11"),
            Some("bytes 4-20/10"),
        ] {
            assert!(validate_range(value, 4, 10).is_err());
        }
    }

    #[tokio::test]
    #[ignore = "downloads the real 1.34 GB production model; run explicitly"]
    async fn real_domestic_download_and_cache_reuse() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../local-model-results/download-sources/real-model-cache");
        let mut production = ModelSpec::production();
        production
            .sources
            .retain(|source| source.url.starts_with("https://modelscope.cn/"));
        let store = ModelStore::new(
            production,
            Client::builder()
                .user_agent(concat!(
                    "RWKVVibeCode/",
                    env!("CARGO_PKG_VERSION"),
                    " model-installer"
                ))
                .https_only(true)
                .no_proxy()
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
        );
        let started = Instant::now();
        let milestones = Arc::new(Mutex::new(Vec::<Progress>::new()));
        let observed = milestones.clone();
        let percent = Arc::new(Mutex::new(0u64));
        let path = store
            .prepare(
                directory.clone(),
                vec![],
                Arc::new(move |p| {
                    let n = p.downloaded_bytes * 100 / p.total_bytes;
                    let mut last = percent.lock().unwrap();
                    if p.phase != "downloading" || n >= *last + 10 {
                        eprintln!(
                            "MODEL {} {}% {:.1} MB/s {}",
                            p.phase,
                            n,
                            p.speed_bps / 1048576.0,
                            p.source
                        );
                        observed.lock().unwrap().push(p);
                        *last = n;
                    }
                }),
            )
            .await
            .unwrap();
        assert!(
            verify(path.clone(), store.spec.clone(), CancellationToken::new())
                .await
                .unwrap()
        );
        let downloaded_seconds = started.elapsed().as_secs_f64();
        let cached = ModelStore::new(
            ModelSpec {
                sources: vec![],
                ..store.spec.clone()
            },
            client(),
        );
        cached.prepare(directory, vec![], notify()).await.unwrap();
        let report = serde_json::json!({"path":path,"bytes":store.spec.bytes,"sha256":store.spec.sha256,"source":"ModelScope China only","downloadSeconds":downloaded_seconds,"cacheReuseWithoutNetwork":true,"milestones":*milestones.lock().unwrap()});
        let report_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../local-model-results/download-sources/real-install-validation.json");
        std::fs::write(report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
