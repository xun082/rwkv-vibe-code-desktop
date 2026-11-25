//! Install and verify the pinned RWKV-7 checkpoint in the application data directory.
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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
    if !m.is_file() {
        return Err("模型路径不是文件".into());
    }
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
    checked: Mutex<Option<(Fingerprint, bool)>>,
    client: Client,
}
impl ModelStore {
    pub fn production() -> Self {
        Self::new(
            ModelSpec::production(),
            Client::builder()
                .https_only(true)
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
            checked: Mutex::new(None),
            client,
        }
    }
    pub fn appears_installed(&self, directory: &Path) -> bool {
        let Ok(current) = fingerprint(&directory.join(&self.spec.filename)) else {
            return false;
        };
        current.bytes == self.spec.bytes
            && self
                .checked
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|(previous, valid)| {
                    previous.path != current.path || (*valid && *previous == current)
                })
    }
    pub async fn installed_model(
        &self,
        directory: &Path,
        token: &CancellationToken,
    ) -> Result<PathBuf, String> {
        self.find_installed(directory, token)
            .await?
            .ok_or_else(|| "本地模型尚未安装或校验未通过，请先完成模型下载".into())
    }
    async fn find_installed(
        &self,
        directory: &Path,
        token: &CancellationToken,
    ) -> Result<Option<PathBuf>, String> {
        check_cancel(token)?;
        let path = directory.join(&self.spec.filename);
        let current = fingerprint(&path).ok();
        if let Some((previous, valid)) = self.checked.lock().unwrap().as_ref() {
            if current.as_ref() == Some(previous) {
                return Ok(valid.then_some(path));
            }
        }
        let valid = verify(path.clone(), self.spec.clone(), token.clone()).await?;
        check_cancel(token)?;
        if let Ok(after) = fingerprint(&path) {
            if current.as_ref() == Some(&after) {
                *self.checked.lock().unwrap() = Some((after, valid));
                return Ok(valid.then_some(path));
            }
        }
        Ok(None)
    }
    #[cfg(test)]
    async fn prepare(
        &self,
        directory: PathBuf,
        existing: Vec<PathBuf>,
        notify: Notify,
    ) -> Result<PathBuf, String> {
        self.prepare_with_cancel(directory, existing, notify, CancellationToken::new())
            .await
    }
    pub async fn prepare_with_cancel(
        &self,
        directory: PathBuf,
        existing: Vec<PathBuf>,
        notify: Notify,
        token: CancellationToken,
    ) -> Result<PathBuf, String> {
        let result = async {
            let _gate = tokio::select! {
                _ = token.cancelled() => return Err("模型准备已暂停".into()),
                gate = self.gate.lock() => gate,
            };
            check_cancel(&token)?;
            self.prepare_inner(&directory, &existing, &notify, &token)
                .await
        }
        .await;
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
        if let Some(installed) = self.find_installed(directory, token).await? {
            emit(
                &self.spec,
                notify,
                "ready",
                self.spec.bytes,
                "",
                "本地模型已就绪",
                0.0,
            );
            return Ok(installed);
        }
        check_cancel(token)?;
        for candidate in existing {
            if candidate == &path || candidate == &part || !candidate.is_file() {
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
                copy_partial(candidate, &part, token).await?;
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
        *self.checked.lock().unwrap() = Some((fingerprint(path)?, true));
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
            response=self.client.get(&source.url).header(header::RANGE,format!("bytes={offset}-{}",self.spec.bytes-1)).header(header::ACCEPT_ENCODING,"identity").send()=>response.map_err(|error|format!("连接失败：{}",error.without_url()))?,
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
                    return Err(format!("网络中断：{}", error.without_url()));
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
async fn copy_partial(
    source: &Path,
    destination: &Path,
    token: &CancellationToken,
) -> Result<(), String> {
    check_cancel(token)?;
    let mut input = tokio::fs::File::open(source)
        .await
        .map_err(|e| format!("无法读取已有模型：{e}"))?;
    let mut output = tokio::fs::File::create(destination)
        .await
        .map_err(io_error)?;
    let mut buffer = vec![0; 4 * 1024 * 1024];
    while !token.is_cancelled() {
        let len = input
            .read(&mut buffer)
            .await
            .map_err(|e| format!("无法读取已有模型：{e}"))?;
        if len == 0 {
            break;
        }
        output.write_all(&buffer[..len]).await.map_err(io_error)?;
    }
    output.flush().await.map_err(io_error)?;
    check_cancel(token)?;
    output.sync_all().await.map_err(io_error)
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
            filename: "model.pth".into(),
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
        Interrupted,
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
                } else if matches!(plan, Reply::Interrupted) {
                    socket.write_all(&payload[..payload.len() / 2]).unwrap();
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
        let data = b"PTH0123456789abcdef".to_vec();
        let (url, offsets, thread) = server(data.clone(), vec![Reply::Range]);
        let store = ModelStore::new(spec(&data, vec![url]), client());
        let root = root();
        std::fs::write(root.join("model.pth.part"), &data[..7]).unwrap();
        let path = store.prepare(root.clone(), vec![], notify()).await.unwrap();
        thread.join().unwrap();
        assert_eq!(*offsets.lock().unwrap(), vec![7]);
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert!(!root.join("model.pth.part").exists());
        let fresh = ModelStore::new(store.spec.clone(), client());
        assert_eq!(
            fresh.prepare(root.clone(), vec![], notify()).await.unwrap(),
            path
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn ignored_range_restarts_instead_of_appending_duplicate_bytes() {
        let data = b"PTH complete file".to_vec();
        let (url, _, thread) = server(data.clone(), vec![Reply::Full]);
        let store = ModelStore::new(spec(&data, vec![url]), client());
        let root = root();
        std::fs::write(root.join("model.pth.part"), b"wrong").unwrap();
        let path = store.prepare(root.clone(), vec![], notify()).await.unwrap();
        thread.join().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), data);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn bad_ranges_do_not_corrupt_the_prefix_and_fail_over() {
        let data = b"PTH range safe file".to_vec();
        let (url, offsets, thread) = server(
            data.clone(),
            vec![Reply::BadRange, Reply::BadRange, Reply::Range],
        );
        let store = ModelStore::new(spec(&data, vec![url.clone(), url]), client());
        let root = root();
        std::fs::write(root.join("model.pth.part"), &data[..6]).unwrap();
        let path = store.prepare(root.clone(), vec![], notify()).await.unwrap();
        thread.join().unwrap();
        assert_eq!(*offsets.lock().unwrap(), vec![6, 6, 6]);
        assert_eq!(std::fs::read(path).unwrap(), data);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn interrupted_download_retries_from_the_saved_offset() {
        let data = b"PTH original checkpoint bytes".to_vec();
        let (url, offsets, thread) = server(data.clone(), vec![Reply::Interrupted, Reply::Range]);
        let store = ModelStore::new(spec(&data, vec![url]), client());
        let root = root();
        let path = store.prepare(root.clone(), vec![], notify()).await.unwrap();
        thread.join().unwrap();
        assert_eq!(*offsets.lock().unwrap(), vec![0, (data.len() / 2) as u64]);
        assert_eq!(std::fs::read(path).unwrap(), data);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn hash_mismatch_is_never_installed_and_next_source_can_recover() {
        let data = b"PTH original data".to_vec();
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
        let store = ModelStore::new(spec(&data, vec![url]), client());
        let root = root();
        let token = CancellationToken::new();
        let cancel = token.clone();
        let result = store
            .prepare_with_cancel(
                root.clone(),
                vec![],
                Arc::new(move |p| {
                    if p.phase == "downloading" && p.downloaded_bytes > 0 {
                        cancel.cancel();
                    }
                }),
                token,
            )
            .await;
        assert!(result.is_err());
        let partial = std::fs::metadata(root.join("model.pth.part"))
            .unwrap()
            .len();
        assert!(partial > 0 && partial < data.len() as u64);
        assert!(!root.join("model.pth").exists());
        let path = store.prepare(root.clone(), vec![], notify()).await.unwrap();
        thread.join().unwrap();
        assert_eq!(offsets.lock().unwrap()[1], partial);
        assert_eq!(std::fs::read(path).unwrap(), data);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn existing_local_model_is_imported_without_network() {
        let data = b"PTH existing local model";
        let original = root();
        let source = original.join("existing.pth");
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
    #[tokio::test]
    async fn shared_cancel_reports_paused_without_removing_complete_partial_file() {
        let data = b"PTH downloaded before cancellation";
        let root = root();
        let path = root.join("model.pth.part");
        std::fs::write(&path, data).unwrap();
        let store = ModelStore::new(spec(data, vec![]), client());
        let token = CancellationToken::new();
        let cancel = token.clone();
        let progress = Arc::new(Mutex::new(Vec::new()));
        let recorded = progress.clone();
        let result = store
            .prepare_with_cancel(
                root.clone(),
                vec![],
                Arc::new(move |p| {
                    if p.phase == "verifying" {
                        cancel.cancel();
                    }
                    recorded.lock().unwrap().push(p.phase);
                }),
                token,
            )
            .await;
        assert!(result.is_err());
        assert_eq!(progress.lock().unwrap().last(), Some(&"paused"));
        assert_eq!(std::fs::read(path).unwrap(), data);
        assert!(!store.appears_installed(&root));
        assert_eq!(
            std::fs::read(store.prepare(root.clone(), vec![], notify()).await.unwrap()).unwrap(),
            data
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn cached_model_is_rechecked_after_its_file_revision_changes() {
        let data = b"PTH original bytes";
        let root = root();
        let path = root.join("model.pth");
        let store = ModelStore::new(spec(data, vec![]), client());
        let token = CancellationToken::new();
        assert!(!store.appears_installed(&root));
        std::fs::write(&path, data).unwrap();
        assert!(store.appears_installed(&root));
        assert_eq!(store.installed_model(&root, &token).await.unwrap(), path);
        let changed = File::options().write(true).open(&path).unwrap();
        (&changed).write_all(&vec![b'X'; data.len()]).unwrap();
        changed
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(3600)),
            )
            .unwrap();
        assert!(!store.appears_installed(&root));
        assert!(store.installed_model(&root, &token).await.is_err());
        drop(changed);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn rejected_installed_model_is_available_for_repair_in_the_same_session() {
        let data = b"PTH approved data";
        let root = root();
        let path = root.join("model.pth");
        let store = ModelStore::new(spec(data, vec![]), client());
        std::fs::write(path, vec![b'X'; data.len()]).unwrap();
        assert!(store.appears_installed(&root));
        assert!(store
            .installed_model(&root, &CancellationToken::new())
            .await
            .is_err());
        assert!(!store.appears_installed(&root));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn production_manifest_pins_the_original_albatross_checkpoint() {
        let spec = ModelSpec::production();
        assert_eq!(spec.filename, "rwkv7-g1l-1.5b-20261031-ctx34304.pth");
        assert_eq!(spec.bytes, 3_055_444_605);
        assert_eq!(
            spec.sha256,
            "b0753acefe6c9ea0b4e2e9d60a3c2faa76d712a1d607db187301aae13dadff41"
        );
        assert!(spec
            .sources
            .iter()
            .all(|source| source.url.starts_with("https://")));
        assert!(spec
            .sources
            .iter()
            .any(|source| source.url.contains("modelscope.cn/")));
        assert!(spec
            .sources
            .iter()
            .any(|source| source.url.contains("huggingface.co/")));
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
    #[ignore = "downloads the 3.06 GB production checkpoint for a live installer test"]
    async fn prepare_production_model_for_smoke_test() {
        let directory = PathBuf::from(
            std::env::var_os("RWKV_SMOKE_MODEL_DIR").expect("set RWKV_SMOKE_MODEL_DIR"),
        );
        let store = ModelStore::production();
        let path = store
            .prepare(
                directory,
                vec![],
                Arc::new(|p| {
                    if p.phase != "downloading" || p.downloaded_bytes == 0 {
                        eprintln!("MODEL {} {} {}", p.phase, p.source, p.message);
                    }
                }),
            )
            .await
            .unwrap();
        assert_eq!(std::fs::metadata(path).unwrap().len(), store.spec.bytes);
    }
}
