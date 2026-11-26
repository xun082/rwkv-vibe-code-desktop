mod albatross;
mod app_update;
mod batch;
mod concurrency;
mod local_runtime;
mod metrics;
mod model_store;
mod prompt;
mod runtime_store;
mod sse;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_QUEUE: usize = 3 * concurrency::MAX;

#[derive(Clone)]
struct EngineConnection {
    endpoint: albatross::Endpoint,
    server: albatross::Server,
    permits: Arc<Semaphore>,
    concurrency: usize,
}

struct Engine {
    process: Option<local_runtime::Process>,
    connection: Option<EngineConnection>,
    error: Option<String>,
}

impl Engine {
    fn reap(&mut self) {
        if let Some(error) = self
            .process
            .as_mut()
            .and_then(|process| process.ensure_running().err())
        {
            self.process = None;
            self.connection = None;
            self.error = Some(error);
        }
    }
}

struct Inner {
    engine: Mutex<Engine>,
    jobs: Mutex<HashMap<String, CancellationToken>>,
    lifecycle: tokio::sync::Mutex<()>,
    client: reqwest::Client,
    models: model_store::ModelStore,
    preparation: Mutex<Option<CancellationToken>>,
    shutdown: CancellationToken,
    dev_endpoint: Option<Result<albatross::Endpoint, String>>,
}

#[derive(Clone)]
struct AppState(Arc<Inner>);

impl AppState {
    fn new() -> Self {
        Self(Arc::new(Inner {
            engine: Mutex::new(Engine {
                process: None,
                connection: None,
                error: None,
            }),
            jobs: Mutex::new(HashMap::new()),
            lifecycle: tokio::sync::Mutex::new(()),
            client: albatross::configure_client(),
            models: model_store::ModelStore::production(),
            preparation: Mutex::new(None),
            shutdown: CancellationToken::new(),
            dev_endpoint: dev_endpoint(),
        }))
    }

    fn shutdown(&self) {
        self.0.shutdown.cancel();
        let mut engine = self.0.engine.lock().unwrap();
        engine.connection = None;
        engine.process = None;
    }

    fn status(&self, directory: Option<&Path>) -> EngineStatus {
        let local_supported = local_runtime::supported();
        let model_installed = local_supported
            && directory.is_some_and(|path| self.0.models.appears_installed(&path.join("models")));
        let runtime_installed = local_supported
            && directory.is_some_and(|path| runtime_store::installed(path).is_some());
        let mut engine = self.0.engine.lock().unwrap();
        engine.reap();
        let connection = engine.connection.as_ref();
        EngineStatus {
            ready: connection.is_some(),
            concurrency: connection.map_or(concurrency::DEFAULT, |state| state.concurrency),
            pending: self.0.jobs.lock().unwrap().len(),
            model: connection.map_or_else(String::new, |state| state.server.model().into()),
            error: engine.error.clone(),
            local_supported,
            can_prepare: local_supported || self.0.dev_endpoint.is_some(),
            model_installed,
            runtime_installed,
            hardware: concurrency::HardwareProfile::from_capacity(
                connection.map(|state| state.server.recommended_capacity()),
            ),
        }
    }

    async fn prepare_local(
        &self,
        directory: &Path,
        existing: Vec<PathBuf>,
        notify: model_store::Notify,
        cancel: CancellationToken,
    ) -> Result<(), String> {
        tokio::task::spawn_blocking(local_runtime::check_gpu)
            .await
            .map_err(|_| "无法检查 NVIDIA 显卡")??;
        runtime_store::prepare(directory, &cancel, &notify).await?;
        self.0
            .models
            .prepare_with_cancel(directory.join("models"), existing, notify, cancel)
            .await?;
        Ok(())
    }

    async fn open_engine(
        &self,
        app: &AppHandle,
    ) -> Result<
        (
            albatross::Endpoint,
            albatross::Server,
            Option<local_runtime::Process>,
        ),
        String,
    > {
        if let Some(endpoint) = &self.0.dev_endpoint {
            let endpoint = endpoint.clone()?;
            let server = self.probe(&endpoint).await?;
            return Ok((endpoint, server, None));
        }
        tokio::task::spawn_blocking(local_runtime::check_gpu)
            .await
            .map_err(|_| "无法检查 NVIDIA 显卡")??;
        let directory = app_data(app)?;
        let runtime = runtime_store::installed(&directory).ok_or("请先安装本地模型和 Albatross")?;
        let model = self
            .0
            .models
            .installed_model(&directory.join("models"), &self.0.shutdown)
            .await?;
        if self.0.shutdown.is_cancelled() {
            return Err("应用已退出".into());
        }
        let (mut process, endpoint) = local_runtime::Process::launch(&runtime, &model, &directory)?;
        let server = process
            .wait_until_ready(&endpoint, &self.0.client, &self.0.shutdown)
            .await?;
        Ok((endpoint, server, Some(process)))
    }

    async fn probe(&self, endpoint: &albatross::Endpoint) -> Result<albatross::Server, String> {
        tokio::select! {
            _ = self.0.shutdown.cancelled() => Err("应用已退出".into()),
            result = endpoint.probe(&self.0.client) => result,
        }
    }
}

fn dev_endpoint() -> Option<Result<albatross::Endpoint, String>> {
    #[cfg(all(debug_assertions, target_os = "macos"))]
    {
        let address = std::env::var("ALBATROSS_URL").ok()?;
        let key = std::env::var("ALBATROSS_API_KEY").ok();
        Some(albatross::Endpoint::parse(&address, key.as_deref()))
    }
    #[cfg(not(all(debug_assertions, target_os = "macos")))]
    None
}

fn app_data(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map_err(|error| format!("无法定位应用数据目录：{error}"))
}

impl EngineConnection {
    fn new(
        endpoint: albatross::Endpoint,
        server: albatross::Server,
        concurrency: usize,
    ) -> Result<Self, String> {
        if concurrency > server.hard_limit() {
            return Err(format!(
                "当前 GPU 最多支持 {} 路，请降低并发数量",
                server.hard_limit()
            ));
        }
        Ok(Self {
            endpoint,
            server,
            permits: Arc::new(Semaphore::new(concurrency)),
            concurrency,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineStatus {
    ready: bool,
    concurrency: usize,
    pending: usize,
    model: String,
    error: Option<String>,
    local_supported: bool,
    can_prepare: bool,
    model_installed: bool,
    runtime_installed: bool,
    hardware: concurrency::HardwareProfile,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum GenerationEvent {
    Queued {
        id: String,
    },
    Started {
        id: String,
    },
    Metrics {
        id: String,
        stats: metrics::InferenceMetrics,
    },
    BatchMetrics {
        id: String,
        stats: metrics::InferenceMetrics,
    },
    Delta {
        id: String,
        text: String,
    },
    Done {
        id: String,
        html: String,
        incomplete: bool,
        seconds: f64,
    },
    Cancelled {
        id: String,
    },
    Error {
        id: String,
        message: String,
    },
}

#[tauri::command]
fn engine_status(app: AppHandle, state: State<'_, AppState>) -> EngineStatus {
    state.status(app_data(&app).ok().as_deref())
}

#[tauri::command]
async fn prepare_model(
    app: AppHandle,
    state: State<'_, AppState>,
    on_event: Channel<model_store::Progress>,
) -> Result<(), String> {
    if !local_runtime::supported() {
        return Err("本机推理支持配有 NVIDIA 显卡的 Windows 和 Linux x64".into());
    }
    let _lock = state.0.lifecycle.lock().await;
    {
        let mut engine = state.0.engine.lock().unwrap();
        engine.reap();
        if engine.connection.is_some() {
            return Ok(());
        }
    }
    let directory = app_data(&app)?;
    let existing = app
        .path()
        .home_dir()
        .ok()
        .map(|home| {
            home.join("Models")
                .join("rwkv")
                .join(&state.0.models.spec.filename)
        })
        .into_iter()
        .collect();
    let notify: model_store::Notify = Arc::new(move |progress| {
        let _ = on_event.send(progress);
    });
    let cancel = state.0.shutdown.child_token();
    *state.0.preparation.lock().unwrap() = Some(cancel.clone());
    let result = state
        .prepare_local(&directory, existing, notify, cancel)
        .await;
    *state.0.preparation.lock().unwrap() = None;
    result
}

#[tauri::command]
fn cancel_model_preparation(state: State<'_, AppState>) {
    if let Some(cancel) = state.0.preparation.lock().unwrap().as_ref() {
        cancel.cancel();
    }
}

#[tauri::command]
async fn start_engine(
    app: AppHandle,
    state: State<'_, AppState>,
    concurrency: usize,
) -> Result<EngineStatus, String> {
    if !concurrency::is_supported(concurrency) {
        return Err(format!(
            "请选择 {}–{} 路的偶数并发档位",
            concurrency::MIN,
            concurrency::MAX
        ));
    }
    let _lock = state.0.lifecycle.lock().await;
    if !state.0.jobs.lock().unwrap().is_empty() {
        return Err("请等待任务完成或停止任务，再切换并发数量".into());
    }
    let existing = {
        let mut engine = state.0.engine.lock().unwrap();
        engine.reap();
        engine.connection.clone()
    };
    let result = if let Some(connection) = &existing {
        state
            .probe(&connection.endpoint)
            .await
            .map(|server| (connection.endpoint.clone(), server, None))
    } else {
        state.open_engine(&app).await
    };
    {
        let mut engine = state.0.engine.lock().unwrap();
        if state.0.shutdown.is_cancelled() {
            return Err("应用已退出".into());
        }
        match result {
            Ok((endpoint, server, process)) => {
                engine.connection = Some(EngineConnection::new(endpoint, server, concurrency)?);
                if process.is_some() {
                    engine.process = process;
                }
                engine.error = None;
            }
            Err(error) => {
                engine.process = None;
                engine.connection = None;
                engine.error = Some(error.clone());
                return Err(error);
            }
        }
    }
    Ok(state.status(app_data(&app).ok().as_deref()))
}

#[tauri::command]
fn cancel_generation(state: State<'_, AppState>, ids: Vec<String>) {
    let jobs = state.0.jobs.lock().unwrap();
    for token in ids.iter().filter_map(|id| jobs.get(id)) {
        token.cancel();
    }
}

#[derive(Deserialize)]
struct PageRequest {
    id: String,
    prompt: String,
}

#[tauri::command]
async fn generate_pages(
    state: State<'_, AppState>,
    requests: Vec<PageRequest>,
    on_event: Channel<GenerationEvent>,
) -> Result<(), String> {
    if !(concurrency::MIN..=concurrency::MAX).contains(&requests.len()) {
        return Err(format!(
            "一次批量请求支持 {}–{} 条需求",
            concurrency::MIN,
            concurrency::MAX
        ));
    }
    let mut ids = HashSet::new();
    for request in &requests {
        if Uuid::parse_str(&request.id).is_err() || !ids.insert(request.id.clone()) {
            return Err("无效或重复的任务 ID".into());
        }
        if request.prompt.trim().is_empty() || request.prompt.chars().count() > 2000 {
            return Err("每条网页需求需为 1–2000 字".into());
        }
    }
    let _lock = state.0.lifecycle.lock().await;
    let connection = {
        let mut engine = state.0.engine.lock().unwrap();
        engine.reap();
        let connection = engine.connection.as_ref().ok_or("请先安装并启动本地模型")?;
        if requests.len() > connection.concurrency {
            return Err("批量需求数超过当前 Albatross 并发档位".into());
        }
        connection.clone()
    };
    let token = state.0.shutdown.child_token();
    {
        let mut jobs = state.0.jobs.lock().unwrap();
        if jobs.len() + requests.len() > MAX_QUEUE {
            return Err("任务队列已满，请稍后重试".into());
        }
        if requests
            .iter()
            .any(|request| jobs.contains_key(&request.id))
        {
            return Err("任务 ID 已存在".into());
        }
        // One native Albatross stream is cancelled as a batch, matching the single Stop control.
        for request in &requests {
            jobs.insert(request.id.clone(), token.clone());
        }
    }
    for request in &requests {
        if on_event
            .send(GenerationEvent::Queued {
                id: request.id.clone(),
            })
            .is_err()
        {
            let mut jobs = state.0.jobs.lock().unwrap();
            for request in &requests {
                jobs.remove(&request.id);
            }
            return Err("界面连接已关闭".into());
        }
    }
    let owned = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let result = run_batch(&owned, &connection, &requests, &token, &on_event).await;
        for request in &requests {
            // Finished lanes were already removed and must never be overwritten
            // if a later lane fails, is cancelled, or loses its connection.
            if owned.0.jobs.lock().unwrap().remove(&request.id).is_none() {
                continue;
            }
            let event = if token.is_cancelled() {
                GenerationEvent::Cancelled {
                    id: request.id.clone(),
                }
            } else {
                GenerationEvent::Error {
                    id: request.id.clone(),
                    message: result
                        .as_ref()
                        .err()
                        .cloned()
                        .unwrap_or_else(|| "Albatross 批量流提前结束".into()),
                }
            };
            let _ = on_event.send(event);
        }
    });
    Ok(())
}

async fn run_batch(
    state: &AppState,
    connection: &EngineConnection,
    requests: &[PageRequest],
    token: &CancellationToken,
    channel: &Channel<GenerationEvent>,
) -> Result<(), String> {
    let _permit = tokio::select! {
        biased;
        _ = token.cancelled() => return Err("已取消".into()),
        permit = connection.permits.clone().acquire_many_owned(requests.len() as u32) =>
            permit.map_err(|_| "Albatross 引擎已停止")?,
    };
    let started = Instant::now();
    let prompts = requests
        .iter()
        .map(|request| prompt::render(&request.prompt))
        .collect();
    for request in requests {
        send(
            channel,
            GenerationEvent::Started {
                id: request.id.clone(),
            },
        )?;
    }

    let request =
        connection
            .endpoint
            .batch_request(&state.0.client, connection.server.model(), prompts);
    let response = tokio::select! {
        biased;
        _ = token.cancelled() => return Err("已取消".into()),
        result = request.send() => result.map_err(|_| "无法连接 Albatross 批量服务")?,
    };
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err("Albatross 拒绝生成请求，请检查 API 密钥".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "Albatross 批量请求失败（HTTP {}）",
            response.status().as_u16()
        ));
    }

    let report_metrics = |stats| {
        send(
            channel,
            GenerationEvent::BatchMetrics {
                id: requests[0].id.clone(),
                stats,
            },
        )
    };
    let mut batch = batch::BatchStream::new(requests.len());
    let (result, final_snapshot) = {
        let (send_id, request_id) = tokio::sync::oneshot::channel();
        let stream = read_batch(
            state, requests, token, channel, response, started, &mut batch, send_id,
        );
        tokio::pin!(stream);
        let observe = observe_batch_metrics(
            &connection.endpoint,
            &state.0.client,
            request_id,
            started,
            report_metrics,
        );
        // Dropping the observer at stream completion also drops any in-flight
        // status request. A slow status check never holds up the SSE reader.
        tokio::select! {
            biased;
            result = &mut stream => (result, None),
            observed = observe => match observed {
                Ok(snapshot) => (stream.await, snapshot),
                Err(error) => (Err(error), None),
            },
        }
    };

    if let Some(id) = batch.request_id() {
        if result.is_err() {
            // The upstream empty-ID stop operation targets unrelated requests.
            connection.endpoint.stop(&state.0.client, id).await;
        } else {
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let final_metrics = match final_snapshot {
                Some(mut stats) => {
                    stats.elapsed_ms = elapsed_ms;
                    Some(stats)
                }
                None => {
                    connection
                        .endpoint
                        .final_metrics(&state.0.client, id, elapsed_ms)
                        .await
                }
            };
            if let Some(stats) = final_metrics {
                report_metrics(stats)?;
            }
        }
    }
    result
}

async fn observe_batch_metrics(
    endpoint: &albatross::Endpoint,
    client: &reqwest::Client,
    request_id: tokio::sync::oneshot::Receiver<String>,
    started: Instant,
    report: impl Fn(metrics::InferenceMetrics) -> Result<(), String>,
) -> Result<Option<metrics::InferenceMetrics>, String> {
    let Ok(id) = request_id.await else {
        return Ok(None);
    };
    let mut counter = metrics::StreamRate::default();
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let sent = Instant::now();
        let Some(status) = endpoint.metrics_status(client).await else {
            continue;
        };
        let sampled_at = sent + sent.elapsed() / 2;
        let elapsed = sampled_at.duration_since(started);
        if let Some(stats) = metrics::InferenceMetrics::from_finished_request(
            &status["last_request"],
            &id,
            elapsed.as_millis() as u64,
        ) {
            return Ok(Some(stats));
        }
        if let Some(stats) = counter.sample(&status, &id, sampled_at, elapsed) {
            report(stats)?;
        }
    }
}

fn send(channel: &Channel<GenerationEvent>, event: GenerationEvent) -> Result<(), String> {
    channel.send(event).map_err(|_| "界面连接已关闭".into())
}

async fn read_batch(
    state: &AppState,
    requests: &[PageRequest],
    token: &CancellationToken,
    channel: &Channel<GenerationEvent>,
    response: reqwest::Response,
    started: Instant,
    batch: &mut batch::BatchStream,
    request_id: tokio::sync::oneshot::Sender<String>,
) -> Result<(), String> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next());
    if content_type != Some("text/event-stream") {
        return Err("Albatross 未返回 SSE 批量流".into());
    }
    for request in requests {
        send(
            channel,
            GenerationEvent::Delta {
                id: request.id.clone(),
                text: prompt::HTML_PREFILL.into(),
            },
        )?;
    }
    let mut stream = response.bytes_stream();
    let mut decoder = sse::SseDecoder::default();
    let mut request_id = Some(request_id);
    loop {
        let next = tokio::select! {
            biased;
            _ = token.cancelled() => return Err("已取消".into()),
            next = stream.next() => next,
        };
        let Some(chunk) = next else {
            return Err("Albatross 批量流提前断开，请重试".into());
        };
        for data in decoder.push(&chunk.map_err(|_| "Albatross 生成连接中断")?)? {
            if token.is_cancelled() {
                return Err("已取消".into());
            }
            if data == "[DONE]" {
                return if batch.all_finished() {
                    Ok(())
                } else {
                    Err("Albatross 批量流缺少部分方案的结束事件".into())
                };
            }
            let value: Value =
                serde_json::from_str(&data).map_err(|_| "无法解析 Albatross 批量事件")?;
            let updates = batch.push(&value, started.elapsed().as_millis() as u64)?;
            if let Some(id) = batch.request_id() {
                if let Some(sender) = request_id.take() {
                    let _ = sender.send(id.into());
                }
            }
            for update in updates {
                let id = &requests[update.index].id;
                if let Some(stats) = update.stats {
                    send(
                        channel,
                        GenerationEvent::Metrics {
                            id: id.clone(),
                            stats,
                        },
                    )?;
                }
                if let Some(text) = update.delta {
                    send(
                        channel,
                        GenerationEvent::Delta {
                            id: id.clone(),
                            text,
                        },
                    )?;
                }
                if let Some(terminal) = update.terminal {
                    state.0.jobs.lock().unwrap().remove(id);
                    let event = match terminal {
                        Ok((html, incomplete)) => GenerationEvent::Done {
                            id: id.clone(),
                            html,
                            incomplete,
                            seconds: started.elapsed().as_secs_f64(),
                        },
                        Err(message) => GenerationEvent::Error {
                            id: id.clone(),
                            message,
                        },
                    };
                    send(channel, event)?;
                }
            }
        }
    }
}

fn extract_html(text: &str) -> Result<String, String> {
    let lower = text.to_ascii_lowercase();
    let start = lower
        .find("<!doctype html")
        .or_else(|| lower.find("<html"))
        .ok_or("模型未返回完整 HTML。可以查看已生成源码，调整需求后重试")?;
    let end = lower[start..]
        .find("</html>")
        .map_or(text.len(), |i| start + i + 7);
    Ok(text[start..end].trim().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(any(windows, target_os = "linux"))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    builder
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            engine_status,
            prepare_model,
            cancel_model_preparation,
            start_engine,
            generate_pages,
            cancel_generation,
            app_update::app_update_status,
            app_update::check_app_update,
            app_update::install_app_update
        ])
        .build(tauri::generate_context!())
        .expect("failed to build desktop application")
        .run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }
            ) {
                app.state::<AppState>().shutdown();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream},
        time::Duration,
    };

    fn accept(listener: &TcpListener) -> (TcpStream, String, Value) {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(&mut stream);
        let mut headers = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            headers.push_str(&line);
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        };
        (stream, headers, value)
    }

    fn respond(mut stream: TcpStream, status: &str, content_type: &str, body: &str) {
        write!(stream,
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        ).unwrap();
    }

    fn status(last_request: Value) -> Value {
        json!({
            "status":"running","api_version":"1.3","engine_version":"albatross-1.3.0",
            "model":{"id":"rwkv7-g1l-1.5b"},
            "capabilities":{"batch_completion":true,"stream":true,"stop":true},
            "prefill_queue":{"hard_max_bsz":16,"dynamic_max_bsz":12},
            "last_request":last_request
        })
    }

    fn requests() -> Vec<PageRequest> {
        [
            "咖啡馆的六款饮品",
            "酒店的四种客房",
            "纪念馆的六部作品",
            "SaaS 的三档定价",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, prompt)| PageRequest {
            id: format!("lane-{index}"),
            prompt: prompt.into(),
        })
        .collect()
    }

    fn capture(
        stop_after_delta: Option<CancellationToken>,
    ) -> (Channel<GenerationEvent>, Arc<Mutex<Vec<Value>>>) {
        let messages = Arc::new(Mutex::new(Vec::new()));
        let captured = messages.clone();
        let channel = Channel::new(move |message| {
            if let tauri::ipc::InvokeResponseBody::Json(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "delta" && value["text"] == "><h1>停止</h1>" {
                    if let Some(token) = &stop_after_delta {
                        token.cancel();
                    }
                }
                captured.lock().unwrap().push(value);
            }
            Ok(())
        });
        (channel, messages)
    }

    #[test]
    fn extracts_document_without_markdown_or_trailing_explanation() {
        assert_eq!(
            extract_html(
                "Here is your page:\n```html\n<!DOCTYPE html><html>中文</html>\n```\nExplanation"
            )
            .unwrap(),
            "<!DOCTYPE html><html>中文</html>"
        );
        assert!(extract_html("I cannot make a webpage.").is_err());
    }

    #[tokio::test]
    async fn authenticates_and_uses_one_native_batch_with_independent_results() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = albatross::Endpoint::parse(
            &format!("http://{}", listener.local_addr().unwrap()),
            Some("test"),
        )
        .unwrap();
        let server = std::thread::spawn(move || {
            let (stream, headers, _) = accept(&listener);
            assert!(headers.starts_with("GET /v1/server/status HTTP/1.1"));
            respond(
                stream,
                "200 OK",
                "application/json",
                &status(Value::Null).to_string(),
            );

            let (stream, headers, body) = accept(&listener);
            assert!(headers.starts_with("POST /v1/tokens/count HTTP/1.1"));
            assert!(headers.contains("authorization: Bearer test"));
            assert_eq!(body, json!({"text":""}));
            respond(stream, "200 OK", "application/json", r#"{"tokens":0}"#);

            let (stream, headers, body) = accept(&listener);
            assert!(headers.starts_with("POST /v1/batch/completions HTTP/1.1"));
            assert!(headers.contains("authorization: Bearer test"));
            let prompts = body["contents"].as_array().unwrap();
            let originals = requests();
            assert_eq!(prompts.len(), originals.len());
            for (prompt, original) in prompts.iter().zip(&originals) {
                let prompt = prompt.as_str().unwrap();
                assert!(prompt.contains(&original.prompt));
                assert!(prompt.ends_with(prompt::HTML_PREFILL));
            }
            assert_eq!(body["metrics"], true);
            assert_eq!(body["max_tokens"], 60_000);
            assert_eq!(body["temperature"], 1.0);
            assert_eq!(body["top_k"], 500);
            assert_eq!(body["top_p"], 0.5);
            assert_eq!(body["alpha_presence"], 1.0);
            assert_eq!(body["alpha_frequency"], 0.1);
            assert_eq!(body["alpha_decay"], 0.99);
            assert_eq!(body["chunk_size"], 8);
            assert_eq!(body["stop"][0], "</html>");
            assert_eq!(body["stop"][3], "<EOD>");

            let mut body = String::new();
            for index in [3, 1, 2, 0] {
                let delta = json!({"id":"req-42","choices":[{
                    "index":index,"delta":{"content":format!("><h1>网页-{index}</h1></body></html>")}
                }]});
                let finish = json!({"id":"req-42","choices":[{
                    "index":index,"delta":{},"finish_reason":"stop","matched_stop":"</html>"
                }]});
                body.push_str(&format!("data: {delta}\n\ndata: {finish}\n\n"));
            }
            body.push_str("data: [DONE]\n\n");
            respond(stream, "200 OK", "text/event-stream; charset=utf-8", &body);

            let (stream, headers, _) = accept(&listener);
            assert!(headers.starts_with("GET /v1/server/status HTTP/1.1"));
            let exact = status(json!({"id":"req-42","endpoint":"batch.completions",
                "generated_tokens":125,"prompt_tokens":521,"decode_speed":147.2}));
            respond(stream, "200 OK", "application/json", &exact.to_string());
        });

        let state = AppState::new();
        let available = endpoint.probe(&state.0.client).await.unwrap();
        assert_eq!(available.recommended_capacity(), 12);
        let connection = EngineConnection::new(endpoint, available, 4).unwrap();
        let (channel, messages) = capture(None);
        run_batch(
            &state,
            &connection,
            &requests(),
            &CancellationToken::new(),
            &channel,
        )
        .await
        .unwrap();
        server.join().unwrap();

        let messages = messages.lock().unwrap();
        let done: Vec<_> = messages
            .iter()
            .filter(|event| event["type"] == "done")
            .collect();
        assert_eq!(done.len(), 4);
        for index in 0..4 {
            let event = done
                .iter()
                .find(|event| event["id"] == format!("lane-{index}"))
                .unwrap();
            assert!(event["html"]
                .as_str()
                .unwrap()
                .contains(&format!("网页-{index}")));
            assert_eq!(event["incomplete"], false);
        }
        let exact = messages
            .iter()
            .find(|event| event["type"] == "batchMetrics")
            .unwrap();
        assert_eq!(exact["stats"]["outputTokens"], 125);
        assert_eq!(exact["stats"]["tps"], 147.2);
        assert_eq!(exact["stats"]["rateKind"], "decode");
        assert!(messages
            .iter()
            .filter(|event| event["type"] == "metrics")
            .all(|event| event["stats"]["outputTokens"] == 0));
    }

    #[tokio::test]
    async fn delivers_live_batch_rate_before_sse_finish_then_uses_exact_decode_speed() {
        fn send_sse_chunk(stream: &mut TcpStream, data: &str) {
            let payload = format!("data: {data}\n\n");
            write!(stream, "{:x}\r\n{payload}\r\n", payload.len()).unwrap();
            stream.flush().unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint =
            albatross::Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap()), None)
                .unwrap();
        let (live_reported, live_seen) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut sse, headers, _) = accept(&listener);
            assert!(headers.starts_with("POST /v1/batch/completions HTTP/1.1"));
            write!(sse, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
            for (text, tokens) in [("><h1>酒店</h1>", 8), ("<p>介绍</p></body></html>", 24)] {
                let chunk = json!({"id":"req-42","object":"chat.completion.chunk",
                    "choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]});
                send_sse_chunk(&mut sse, &chunk.to_string());
                let (stream, headers, _) = accept(&listener);
                assert!(headers.starts_with("GET /v1/server/status HTTP/1.1"));
                let mut active = status(Value::Null);
                active["active_requests"] = json!([
                    {"id":"req-42","endpoint":"batch.completions","generated_tokens":tokens,
                     "prompt_tokens":120,"prefill_progress":1.0,"decode_speed":0.0},
                    {"id":"other-request","endpoint":"batch.completions","generated_tokens":1000,
                     "prompt_tokens":500,"prefill_progress":1.0,"decode_speed":0.0}
                ]);
                active["active_request"] = active["active_requests"][1].clone();
                respond(stream, "200 OK", "application/json", &active.to_string());
            }
            live_seen.recv_timeout(Duration::from_secs(3)).unwrap();
            let finish = json!({"id":"req-42","choices":[{"index":0,"delta":{},
                "finish_reason":"stop","matched_stop":"</html>"}]});
            send_sse_chunk(&mut sse, &finish.to_string());
            send_sse_chunk(&mut sse, "[DONE]");
            write!(sse, "0\r\n\r\n").unwrap();
            drop(sse);

            // v1.8.15 sends [DONE] before moving the request into last_request.
            let (stream, _, _) = accept(&listener);
            respond(
                stream,
                "200 OK",
                "application/json",
                &status(Value::Null).to_string(),
            );
            let (stream, _, _) = accept(&listener);
            let final_status = status(json!({"id":"req-42","endpoint":"batch.completions",
                "generated_tokens":23,"prompt_tokens":120,"decode_speed":48.5,"finished":42}));
            respond(
                stream,
                "200 OK",
                "application/json",
                &final_status.to_string(),
            );
        });
        let connection = EngineConnection::new(
            endpoint,
            serde_json::from_value(status(Value::Null)).unwrap(),
            4,
        )
        .unwrap();
        let messages = Arc::new(Mutex::new(Vec::new()));
        let captured = messages.clone();
        let channel = Channel::new(move |message| {
            if let tauri::ipc::InvokeResponseBody::Json(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "batchMetrics"
                    && value["stats"]["rateKind"] == "stream"
                    && value["stats"]["tps"].as_f64().is_some_and(|tps| tps > 0.0)
                {
                    live_reported.send(()).unwrap();
                }
                captured.lock().unwrap().push(value);
            }
            Ok(())
        });
        tokio::time::timeout(
            Duration::from_secs(5),
            run_batch(
                &AppState::new(),
                &connection,
                &requests()[..1],
                &CancellationToken::new(),
                &channel,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        server.join().unwrap();
        let messages = messages.lock().unwrap();
        let live = messages
            .iter()
            .position(|event| event["stats"]["outputTokens"] == 24)
            .unwrap();
        let done = messages
            .iter()
            .position(|event| event["type"] == "done")
            .unwrap();
        let final_metrics = messages.last().unwrap();
        assert!(live < done);
        assert_eq!(messages[live]["stats"]["rateKind"], "stream");
        assert_eq!(final_metrics["stats"]["rateKind"], "decode");
        assert_eq!(final_metrics["stats"]["outputTokens"], 23);
        assert_eq!(final_metrics["stats"]["tps"], 48.5);
    }

    #[tokio::test]
    async fn cancel_targets_the_exact_albatross_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = albatross::Endpoint::parse(
            &format!("http://{}", listener.local_addr().unwrap()),
            Some("test"),
        )
        .unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _, _) = accept(&listener);
            let delta = "data: {\"id\":\"req-stop\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"><h1>停止</h1>\"}}]}\n\n";
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{delta}\r\n", delta.len()).unwrap();
            stream.flush().unwrap();
            let (stop, headers, body) = accept(&listener);
            assert!(headers.starts_with("POST /v1/server/stop HTTP/1.1"));
            assert_eq!(body, json!({"request_id":"req-stop"}));
            respond(
                stop,
                "200 OK",
                "application/json",
                r#"{"ok":true,"stopped":true}"#,
            );
        });
        let connection = EngineConnection::new(
            endpoint,
            serde_json::from_value(status(Value::Null)).unwrap(),
            4,
        )
        .unwrap();
        let state = AppState::new();
        let token = CancellationToken::new();
        let (channel, messages) = capture(Some(token.clone()));
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_batch(&state, &connection, &requests(), &token, &channel),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(token.is_cancelled());
        server.join().unwrap();
        assert!(!messages
            .lock()
            .unwrap()
            .iter()
            .any(|event| event["type"] == "done"));
    }

    #[tokio::test]
    async fn rejects_missing_credentials_even_when_public_status_is_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint =
            albatross::Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap()), None)
                .unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _, _) = accept(&listener);
            respond(
                stream,
                "200 OK",
                "application/json",
                &status(Value::Null).to_string(),
            );
            let (stream, headers, _) = accept(&listener);
            assert!(headers.starts_with("POST /v1/tokens/count "));
            respond(
                stream,
                "401 Unauthorized",
                "application/json",
                r#"{"error":"Unauthorized"}"#,
            );
        });
        assert!(
            matches!(endpoint.probe(&albatross::configure_client()).await,
            Err(error) if error.contains("API 密钥"))
        );
        server.join().unwrap();
    }
}
