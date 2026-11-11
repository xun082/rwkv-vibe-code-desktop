mod batch;
mod metrics;
mod model_store;
mod prompt;
mod sse;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, Manager, State};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_QUEUE: usize = 12;
const MAX_CONCURRENCY: usize = 4;

#[derive(Clone)]
struct EngineConnection {
    base: String,
    key: String,
    permits: Arc<Semaphore>,
    template: Option<prompt::ChatTemplate>,
}
struct Runtime {
    child: Child,
    connection: EngineConnection,
    concurrency: usize,
    ready: bool,
}
struct Inner {
    runtime: Mutex<Option<Runtime>>,
    jobs: Mutex<HashMap<String, CancellationToken>>,
    lifecycle: tokio::sync::Mutex<()>,
    client: reqwest::Client,
    last_error: Mutex<Option<String>>,
    models: model_store::ModelStore,
}
#[derive(Clone)]
struct AppState(Arc<Inner>);
impl AppState {
    fn new() -> Self {
        Self(Arc::new(Inner {
            runtime: Mutex::new(None),
            jobs: Mutex::new(HashMap::new()),
            lifecycle: tokio::sync::Mutex::new(()),
            client: reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(600))
                .build()
                .expect("HTTP client"),
            last_error: Mutex::new(None),
            models: model_store::ModelStore::production(),
        }))
    }
    fn shutdown(&self) {
        self.0.models.cancel();
        self.stop_runtime();
    }
    fn stop_runtime(&self) {
        for token in self.0.jobs.lock().unwrap().values() {
            token.cancel();
        }
        if let Some(mut runtime) = self.0.runtime.lock().unwrap().take() {
            runtime.connection.permits.close();
            let _ = runtime.child.kill();
            let _ = runtime.child.wait();
        }
    }
    fn status(&self) -> EngineStatus {
        let mut guard = self.0.runtime.lock().unwrap();
        if let Some(runtime) = guard.as_mut() {
            if let Ok(Some(code)) = runtime.child.try_wait() {
                runtime.connection.permits.close();
                for token in self.0.jobs.lock().unwrap().values() {
                    token.cancel();
                }
                *self.0.last_error.lock().unwrap() =
                    Some(format!("模型进程已退出（{code}），请重新启动"));
                *guard = None;
            }
        }
        EngineStatus {
            ready: guard.as_ref().is_some_and(|r| r.ready),
            concurrency: guard.as_ref().map_or(MAX_CONCURRENCY, |r| r.concurrency),
            pending: self.0.jobs.lock().unwrap().len(),
            model: self.0.models.spec.filename.clone(),
            error: self.0.last_error.lock().unwrap().clone(),
        }
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

fn resources(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources"))
    } else {
        app.path().resource_dir().map_err(|e| e.to_string())
    }
}

fn model_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models"))
}

#[tauri::command]
async fn prepare_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    on_event: Channel<model_store::Progress>,
) -> Result<(), String> {
    let directory = model_directory(&app)?;
    let mut existing = Vec::new();
    if let Ok(home) = app.path().home_dir() {
        existing.push(home.join("Models/rwkv").join(&state.0.models.spec.filename));
    }
    // Reuse a development/old bundled copy if available, without depending on it.
    existing.push(
        resources(&app)?
            .join("models")
            .join(&state.0.models.spec.filename),
    );
    state
        .0
        .models
        .prepare(
            directory,
            existing,
            Arc::new(move |progress| {
                let _ = on_event.send(progress);
            }),
        )
        .await
        .map(|_| ())
}

#[tauri::command]
fn cancel_model_preparation(state: State<'_, AppState>) {
    state.0.models.cancel();
}

#[tauri::command]
fn engine_status(state: State<'_, AppState>) -> EngineStatus {
    state.status()
}

#[tauri::command]
async fn start_engine(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    concurrency: usize,
) -> Result<EngineStatus, String> {
    if !(1..=MAX_CONCURRENCY).contains(&concurrency) {
        return Err("本机支持 1–4 路并发".into());
    }
    let _lock = state.0.lifecycle.lock().await;
    let status = state.status();
    // A warm four-slot engine can serve a one-prompt batch without reloading
    // 1.3 GB of model weights just because the UI selects one result.
    if status.ready && status.concurrency >= concurrency {
        return Ok(status);
    }
    if !state.0.jobs.lock().unwrap().is_empty() {
        return Err("请等待任务完成或停止任务，再切换并发数量".into());
    }
    state.stop_runtime();
    *state.0.last_error.lock().unwrap() = None;
    let res = resources(&app)?;
    let server = res.join("runtime/llama-server");
    if !server.is_file() {
        return Err("缺少内置推理运行时，请重新安装应用".into());
    }
    let model = state
        .0
        .models
        .installed_model(&model_directory(&app)?)
        .await?;
    let port = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| e.to_string())?
        .local_addr()
        .map_err(|e| e.to_string())?
        .port();
    let key = Uuid::new_v4().to_string();
    let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    let log = File::create(log_dir.join("llama-server.log")).map_err(|e| e.to_string())?;
    let err_log = log.try_clone().map_err(|e| e.to_string())?;
    let child = Command::new(&server)
        .args(["-m"])
        .arg(&model)
        .args([
            "-ngl",
            "99",
            "-c",
            &(16384 * concurrency).to_string(),
            "-np",
            &concurrency.to_string(),
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--alias",
            "rwkv-local",
            "--jinja",
            "--reasoning",
            "off",
            "--cache-ram",
            "0",
            "--slots",
        ])
        .env("LLAMA_API_KEY", &key)
        .env_remove("GGML_BACKEND_PATH")
        .current_dir(res.join("runtime"))
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log))
        .spawn()
        .map_err(|e| format!("无法启动推理引擎：{e}"))?;
    let connection = EngineConnection {
        base: format!("http://127.0.0.1:{port}"),
        key,
        permits: Arc::new(Semaphore::new(concurrency)),
        template: None,
    };
    *state.0.runtime.lock().unwrap() = Some(Runtime {
        child,
        connection: connection.clone(),
        concurrency,
        ready: false,
    });
    for _ in 0..180 {
        state.status();
        if state.0.runtime.lock().unwrap().is_none() {
            return Err("模型进程启动失败，请查看 llama-server.log".into());
        }
        let response = state
            .0
            .client
            .get(format!("{}/props", connection.base))
            .bearer_auth(&connection.key)
            .timeout(Duration::from_secs(1))
            .send()
            .await;
        if response.is_ok_and(|r| r.status().is_success()) {
            // Format role tokens once during warmup, without inference. Every
            // later generation is ONE /completion POST containing prompt[].
            let template_result = async {
                let response = state
                    .0
                    .client
                    .post(format!("{}/apply-template", connection.base))
                    .bearer_auth(&connection.key)
                    .timeout(Duration::from_secs(10))
                    .json(&prompt::template_request())
                    .send()
                    .await
                    .map_err(|e| e.to_string())?
                    .error_for_status()
                    .map_err(|e| e.to_string())?;
                let value: Value = response.json().await.map_err(|e| e.to_string())?;
                prompt::ChatTemplate::from_rendered(&format!(
                    "{}{}",
                    value["prompt"].as_str().ok_or("缺少对话模板")?,
                    prompt::HTML_PREFILL
                ))
            }
            .await;
            let template = match template_result {
                Ok(template) => template,
                Err(error) => {
                    state.stop_runtime();
                    return Err(format!("无法准备批量提示词：{error}"));
                }
            };
            if let Some(runtime) = state.0.runtime.lock().unwrap().as_mut() {
                runtime.connection.template = Some(template);
                runtime.ready = true;
            }
            let status = state.status();
            if status.ready {
                return Ok(status);
            }
            return Err("模型进程启动失败，请查看 llama-server.log".into());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    state.stop_runtime();
    Err(format!(
        "模型启动超时，请检查 {}",
        log_dir.join("llama-server.log").display()
    ))
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
    if !(1..=MAX_CONCURRENCY).contains(&requests.len()) {
        return Err("一次批量请求支持 1–4 条 prompt".into());
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
        let guard = state.0.runtime.lock().unwrap();
        let runtime = guard
            .as_ref()
            .filter(|r| r.ready)
            .ok_or("请先启动本地模型")?;
        if requests.len() > runtime.concurrency {
            return Err("批量 prompt 数超过当前模型并发数量".into());
        }
        runtime.connection.clone()
    };
    let token = CancellationToken::new();
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
        // One HTTP stream is cancelled as a batch, matching the single Stop control.
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
                        .unwrap_or_else(|| "批量流提前结束".into()),
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
        _ = token.cancelled() => return Err("已取消".into()),
        permit = connection.permits.clone().acquire_many_owned(requests.len() as u32) => permit.map_err(|_| "引擎已停止")?,
    };
    let started = Instant::now();
    let template = connection.template.as_ref().ok_or("批量对话模板尚未就绪")?;
    let prompts: Vec<String> = requests
        .iter()
        .map(|request| template.render(request.prompt.trim()))
        .collect();
    for request in requests {
        channel
            .send(GenerationEvent::Started {
                id: request.id.clone(),
            })
            .map_err(|_| "界面连接已关闭")?;
    }
    // A single inference POST with prompt[]; llama-server schedules every item
    // into its continuous batch and returns one interleaved, indexed SSE stream.
    let request = state.0.client.post(format!("{}/completion", connection.base))
        .bearer_auth(&connection.key)
        .json(&json!({"prompt":prompts,"stream":true,"timings_per_token":true,"return_progress":true,"n_predict":12000,"stop":["</html>"],"temperature":1.0,"top_p":0.5,"top_k":60,"min_p":0.0,"repeat_penalty":1.0,"frequency_penalty":0.1,"presence_penalty":1.0,"dry_multiplier":0.8,"dry_allowed_length":48,"dry_penalty_last_n":2048,"dry_sequence_breakers":["<",">"],"seed":-1}));
    let response = tokio::select! {
        _ = token.cancelled() => return Err("已取消".into()),
        response = request.send() => response.map_err(|e| format!("无法连接本地模型：{e}"))?,
    };
    if !response.status().is_success() {
        let code = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "批量模型请求失败（{code}）：{}",
            body.chars().take(500).collect::<String>()
        ));
    }
    for request in requests {
        channel
            .send(GenerationEvent::Delta {
                id: request.id.clone(),
                text: prompt::HTML_PREFILL.into(),
            })
            .map_err(|_| "界面连接已关闭")?;
    }
    let mut stream = response.bytes_stream();
    let mut decoder = sse::SseDecoder::default();
    let mut batch = batch::BatchStream::new(requests.len());
    let mut last_metrics = vec![None::<Instant>; requests.len()];
    loop {
        let next = tokio::select! { _ = token.cancelled() => return Err("已取消".into()), next = stream.next() => next };
        let Some(chunk) = next else {
            break;
        };
        for event in decoder.push(&chunk.map_err(|e| format!("生成连接中断：{e}"))?)? {
            if event == "[DONE]" {
                continue;
            }
            let value: Value =
                serde_json::from_str(&event).map_err(|e| format!("无法解析批量结果：{e}"))?;
            let Some(update) = batch.push(&value, started.elapsed().as_millis() as u64)? else {
                continue;
            };
            let id = &requests[update.index].id;
            if let Some(stats) = update.stats.filter(|_| {
                update.terminal.is_some()
                    || last_metrics[update.index]
                        .is_none_or(|last| last.elapsed() >= Duration::from_millis(100))
            }) {
                last_metrics[update.index] = Some(Instant::now());
                channel
                    .send(GenerationEvent::Metrics {
                        id: id.clone(),
                        stats,
                    })
                    .map_err(|_| "界面连接已关闭")?;
            }
            if let Some(text) = update.delta {
                channel
                    .send(GenerationEvent::Delta {
                        id: id.clone(),
                        text,
                    })
                    .map_err(|_| "界面连接已关闭")?;
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
                channel.send(event).map_err(|_| "界面连接已关闭")?;
            }
            if batch.all_finished() {
                return Ok(());
            }
        }
    }
    Err("批量生成连接提前结束，请重试".into())
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
    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            engine_status,
            prepare_model,
            cancel_model_preparation,
            start_engine,
            generate_pages,
            cancel_generation
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
    async fn four_prompts_use_one_http_post_and_one_channel_with_independent_results() {
        use std::io::{Read, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut received = Vec::new();
            let mut chunk = [0u8; 4096];
            let header_end = loop {
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                received.extend_from_slice(&chunk[..n]);
                if let Some(end) = received.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let header = String::from_utf8_lossy(&received[..header_end]);
            assert!(header.starts_with("POST /completion HTTP/1.1"));
            let length: usize = header
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse().unwrap())
                })
                .unwrap();
            while received.len() - header_end < length {
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                received.extend_from_slice(&chunk[..n]);
            }
            let payload: Value =
                serde_json::from_slice(&received[header_end..header_end + length]).unwrap();
            let prompts = payload["prompt"].as_array().unwrap();
            assert_eq!(prompts.len(), 4);
            assert!(prompts[0].as_str().unwrap().contains("咖啡馆的六款饮品"));
            assert!(prompts[1].as_str().unwrap().contains("酒店的四种客房"));
            assert!(prompts[2].as_str().unwrap().contains("纪念馆的六部作品"));
            assert!(prompts[3].as_str().unwrap().contains("SaaS 的三档定价"));
            let mut events = Vec::new();
            for index in [3, 1, 2, 0] {
                events.push(json!({"index":index,"content":format!(" class=\"bg-white\"><h1>page-{index}</h1></body>"),"timings":{"predicted_n":30+index,"predicted_per_second":40.0}}));
            }
            for index in [2, 0, 3, 1] {
                events.push(json!({"index":index,"stop":true,"stop_type":"word","stopping_word":"</html>","timings":{"predicted_n":40+index,"predicted_per_second":40.0}}));
            }
            let body: String = events
                .iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            listener.set_nonblocking(true).unwrap();
            assert!(
                listener.accept().is_err(),
                "a batch must not create a second inference connection"
            );
        });
        let template = prompt::ChatTemplate::from_rendered(&format!(
            "User: {}\n\nAssistant: {}",
            prompt::TEMPLATE_MARKER,
            prompt::HTML_PREFILL
        ))
        .unwrap();
        let connection = EngineConnection {
            base: format!("http://{address}"),
            key: "test".into(),
            permits: Arc::new(Semaphore::new(4)),
            template: Some(template),
        };
        let requests: Vec<PageRequest> = [
            "咖啡馆的六款饮品",
            "酒店的四种客房",
            "纪念馆的六部作品",
            "SaaS 的三档定价",
        ]
        .iter()
        .enumerate()
        .map(|(i, prompt)| PageRequest {
            id: format!("lane-{i}"),
            prompt: (*prompt).into(),
        })
        .collect();
        let messages = Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured = messages.clone();
        let channel = Channel::new(move |message| {
            if let tauri::ipc::InvokeResponseBody::Json(text) = message {
                captured
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(&text).unwrap());
            }
            Ok(())
        });
        run_batch(
            &AppState::new(),
            &connection,
            &requests,
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
                .contains(&format!("page-{index}")));
            assert_eq!(event["incomplete"], false);
            let last_metrics = messages
                .iter()
                .rev()
                .find(|event| event["type"] == "metrics" && event["id"] == format!("lane-{index}"))
                .unwrap();
            assert_eq!(last_metrics["stats"]["outputTokens"], 40 + index);
        }
    }
}
