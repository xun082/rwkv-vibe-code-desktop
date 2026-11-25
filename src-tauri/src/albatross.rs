//! Native batch API of RWKV Lightning CUDA v1.8.15 (Albatross 1.3).
use crate::metrics::InferenceMetrics;
use reqwest::{header, Client, Method, RequestBuilder, Response, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{net::IpAddr, time::Duration};

const ENGINE_VERSION: &str = "albatross-1.3.0";
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct Endpoint {
    base: Url,
    authorization: Option<header::HeaderValue>,
}

#[derive(Clone, Deserialize)]
pub struct Server {
    status: String,
    engine_version: String,
    api_version: String,
    model: Model,
    capabilities: Capabilities,
    prefill_queue: Capacity,
}

#[derive(Clone, Deserialize)]
struct Model {
    id: String,
}

#[derive(Clone, Deserialize)]
struct Capabilities {
    batch_completion: bool,
    stream: bool,
    stop: bool,
}

#[derive(Clone, Deserialize)]
struct Capacity {
    hard_max_bsz: usize,
    dynamic_max_bsz: usize,
}

impl Server {
    pub fn model(&self) -> &str {
        &self.model.id
    }

    pub fn hard_limit(&self) -> usize {
        self.prefill_queue.hard_max_bsz
    }

    pub fn recommended_capacity(&self) -> usize {
        self.prefill_queue.dynamic_max_bsz.min(self.hard_limit())
    }
}

impl Endpoint {
    pub fn parse(address: &str, api_key: Option<&str>) -> Result<Self, String> {
        let mut base = Url::parse(address.trim()).map_err(|_| "请输入完整的 Albatross 服务地址")?;
        let host = base.host_str().ok_or("Albatross 服务地址缺少主机名")?;
        let loopback = host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if base.scheme() != "https" && !(base.scheme() == "http" && loopback) {
            return Err("请使用 HTTPS 服务地址，或使用 HTTP 连接本机及 SSH 转发端口".into());
        }
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err("服务地址不能含用户名、密码、查询参数或片段".into());
        }
        if !base.path().ends_with('/') {
            base.path_segments_mut()
                .map_err(|_| "无效的服务路径")?
                .push("");
        }
        let authorization = api_key
            .filter(|key| !key.trim().is_empty())
            .map(|key| {
                let mut value = header::HeaderValue::from_str(&format!("Bearer {}", key.trim()))
                    .map_err(|_| "API 密钥包含无效字符")?;
                value.set_sensitive(true);
                Ok::<_, &str>(value)
            })
            .transpose()?;
        Ok(Self {
            base,
            authorization,
        })
    }

    fn request(&self, client: &Client, method: Method, path: &str) -> RequestBuilder {
        let url = self
            .base
            .join(path)
            .expect("static relative Albatross API path");
        let request = client.request(method, url);
        match &self.authorization {
            Some(value) => request.header(header::AUTHORIZATION, value.clone()),
            None => request,
        }
    }

    async fn status(&self, client: &Client, timeout: Duration) -> Result<Response, String> {
        let response = self
            .request(client, Method::GET, "v1/server/status")
            .timeout(timeout)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    "Albatross 服务连接超时"
                } else {
                    "Albatross 推理进程无法响应"
                }
            })?;
        if !response.status().is_success() {
            return Err(format!(
                "Albatross 状态检查失败（HTTP {}）",
                response.status().as_u16()
            ));
        }
        Ok(response)
    }

    pub async fn probe(&self, client: &Client) -> Result<Server, String> {
        let server: Server = self
            .status(client, CONTROL_TIMEOUT)
            .await?
            .json()
            .await
            .map_err(|_| "服务状态格式不兼容，请使用 Lightning CUDA v1.8.15 的固定模型模式")?;
        if server.status != "running"
            || server.engine_version != ENGINE_VERSION
            || server.api_version != "1.3"
            || server.model().trim().is_empty()
            || !server.capabilities.batch_completion
            || !server.capabilities.stream
            || !server.capabilities.stop
            || server.hard_limit() == 0
        {
            return Err(
                "Albatross 批量服务未就绪，请检查 Lightning CUDA 版本和 RWKV-7 模型".into(),
            );
        }
        // Status is public upstream. An empty token count checks auth without GPU inference.
        let auth = self
            .request(client, Method::POST, "v1/tokens/count")
            .timeout(CONTROL_TIMEOUT)
            .json(&json!({"text":""}))
            .send()
            .await
            .map_err(|_| "无法验证 Albatross 服务的访问权限")?;
        if matches!(auth.status().as_u16(), 401 | 403) {
            return Err("Albatross 拒绝访问，请检查 API 密钥".into());
        }
        if !auth.status().is_success() {
            return Err(format!(
                "Albatross 访问验证失败（HTTP {}）",
                auth.status().as_u16()
            ));
        }
        let result: Value = auth
            .json()
            .await
            .map_err(|_| "Albatross token 接口格式不兼容")?;
        if result["tokens"] != 0 {
            return Err("Albatross token 接口格式不兼容".into());
        }
        Ok(server)
    }

    pub fn batch_request(
        &self,
        client: &Client,
        model: &str,
        prompts: Vec<String>,
    ) -> RequestBuilder {
        self.request(client, Method::POST, "v1/batch/completions")
            .header(header::ACCEPT, "text/event-stream")
            .json(&json!({
                "model": model, "contents": prompts, "stream": true, "metrics": true,
                "max_tokens": 12000, "chunk_size": 8, "stop": ["</html>", "</HTML>", "\n\nUser:", "<EOD>"],
                "temperature": 1.0, "top_p": 0.5, "top_k": 60,
                "alpha_presence": 1.0, "alpha_frequency": 0.1, "alpha_decay": 0.99
            }))
    }

    pub async fn stop(&self, client: &Client, id: &str) {
        if id.is_empty() {
            return;
        }
        let _ = self
            .request(client, Method::POST, "v1/server/stop")
            .timeout(Duration::from_secs(2))
            .json(&json!({"request_id":id}))
            .send()
            .await;
    }

    pub async fn final_metrics(
        &self,
        client: &Client,
        id: &str,
        elapsed_ms: u64,
    ) -> Option<InferenceMetrics> {
        // Only the latest server request is retained. Shared-server races can
        // legitimately make metrics unavailable, so match the ID strictly.
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(80)).await;
            }
            let status: Value = self
                .status(client, Duration::from_secs(1))
                .await
                .ok()?
                .json()
                .await
                .ok()?;
            if let Some(stats) =
                InferenceMetrics::from_finished_request(&status["last_request"], id, elapsed_ms)
            {
                return Some(stats);
            }
        }
        None
    }
}

pub fn configure_client() -> Client {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONTROL_TIMEOUT)
        .read_timeout(Duration::from_secs(180))
        .build()
        .expect("Albatross HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_addresses_and_preserves_reverse_proxy_prefix() {
        let endpoint = Endpoint::parse(" https://gpu.example/albatross ", Some("key")).unwrap();
        assert_eq!(endpoint.base.as_str(), "https://gpu.example/albatross/");
        let request = endpoint
            .batch_request(&configure_client(), "rwkv", Vec::new())
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://gpu.example/albatross/v1/batch/completions"
        );
        assert!(request.headers()[header::AUTHORIZATION].is_sensitive());
        assert!(Endpoint::parse("http://127.0.0.1:8000", None).is_ok());
        assert!(Endpoint::parse("http://[::1]:8000", None).is_ok());
        for invalid in [
            "http://gpu.example",
            "ftp://localhost",
            "https://user@gpu.example",
            "https://gpu.example/?token=a",
            "https://gpu.example/#unsafe",
        ] {
            assert!(Endpoint::parse(invalid, None).is_err());
        }
        assert!(Endpoint::parse("http://127.0.0.1", Some("key\r\nother")).is_err());
    }
}
