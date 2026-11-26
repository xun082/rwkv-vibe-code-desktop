#!/usr/bin/env python3
"""Exercise the real Albatross native batch endpoint with independent HTML prompts."""

import argparse
import ipaddress
import json
import math
import os
import re
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from html.parser import HTMLParser
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
STATUS_PATH = "/v1/server/status"
BATCH_PATH = "/v1/batch/completions"
MAX_EVENT_BYTES = 2 * 1024 * 1024
MAX_HTML_BYTES = 8 * 1024 * 1024
MAX_OUTPUT_TOKENS = 60_000
TOPICS = (
    "咖啡馆", "精品酒店", "纪念馆", "摄影展", "花店", "糕点店", "图书馆", "艺术馆",
    "唱片店", "旅行社", "植物园", "独立书店", "露营地", "陶艺工作室", "茶馆", "社区市集",
)
MARKER_PATTERN = re.compile(r"RWKV-CHECK-(?:0[1-9]|1[0-6])(?!\d)")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, *args):
        return None


class Client:
    def __init__(self, base: str, key: str) -> None:
        url = urllib.parse.urlsplit(base)
        if (
            url.scheme not in {"http", "https"}
            or not url.hostname
            or url.username is not None
            or url.query
            or url.fragment
        ):
            raise ValueError("ALBATROSS_URL must be an HTTP(S) base URL without credentials, query or fragment")
        if url.scheme == "http" and url.hostname != "localhost":
            try:
                loopback = ipaddress.ip_address(url.hostname).is_loopback
            except ValueError:
                loopback = False
            if not loopback:
                raise ValueError("Use HTTPS for remote hosts or connect to a loopback SSH tunnel")
        self.base = base.rstrip("/")
        self.headers = {"Authorization": f"Bearer {key}"} if key else {}
        self.key = key
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())

    def request(self, path: str, *, body=None, timeout: float = 15):
        headers = dict(self.headers)
        data = None
        if body is not None:
            headers.update({"Content-Type": "application/json", "Accept": "text/event-stream"})
            data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        request = urllib.request.Request(self.base + path, data=data, headers=headers)
        try:
            return self.opener.open(request, timeout=timeout)
        except urllib.error.HTTPError as error:
            with error:
                detail = error.read(512).decode("utf-8", errors="replace").strip()
            if self.key:
                detail = detail.replace(self.key, "[redacted]")
            raise RuntimeError(f"{path} returned HTTP {error.code}: {detail}") from None

    def status(self) -> dict:
        with self.request(STATUS_PATH) as response:
            status = json.load(response)
        if not isinstance(status, dict):
            raise RuntimeError("Albatross returned an invalid status object")
        return status


class HeadingText(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.inside_heading = False
        self.text = []

    def handle_starttag(self, tag, attrs):
        if tag == "h1":
            self.inside_heading = True

    def handle_endtag(self, tag):
        if tag == "h1":
            self.inside_heading = False

    def handle_data(self, data):
        if self.inside_heading:
            self.text.append(data)


@dataclass
class Lane:
    marker: str
    fragments: list[str] = field(default_factory=list)
    size: int = 0
    first_delta_ms: float | None = None
    finished_ms: float | None = None
    finish_reason: str | None = None
    matched_stop: str | None = None

    def observe(self, choice: dict, elapsed_ms: float) -> None:
        if self.finish_reason is not None:
            raise RuntimeError(f"Received a choice after lane {self.marker} finished")
        delta = choice.get("delta", {})
        if not isinstance(delta, dict) or not isinstance(delta.get("content", ""), str):
            raise RuntimeError(f"Invalid text delta for lane {self.marker}")
        content = delta.get("content", "")
        if content:
            if self.first_delta_ms is None:
                self.first_delta_ms = elapsed_ms
            self.size += len(content.encode("utf-8"))
            if self.size > MAX_HTML_BYTES:
                raise RuntimeError(f"Lane {self.marker} exceeded the HTML size limit")
            self.fragments.append(content)

        reason = choice.get("finish_reason")
        if reason is not None:
            if reason not in ("stop", "length"):
                raise RuntimeError(f"Unexpected finish reason for lane {self.marker}: {reason!r}")
            self.finish_reason = reason
            self.finished_ms = elapsed_ms
            self.matched_stop = choice.get("matched_stop")

    def summary(self, prefill: str, transport_only: bool) -> dict:
        text = "".join(self.fragments)
        if not text.strip() or self.finish_reason is None:
            raise RuntimeError(f"Lane {self.marker} did not return both output and a finish reason")
        close = re.search(r"</html\s*>", text, re.IGNORECASE)
        if self.matched_stop in ("</html>", "</HTML>") and not close:
            raise RuntimeError(f"Lane {self.marker} lost the inclusive HTML stop delimiter")
        foreign = set(MARKER_PATTERN.findall(text)) - {self.marker}
        if foreign:
            raise RuntimeError(f"Lane {self.marker} contains another prompt's marker: {sorted(foreign)}")
        if not transport_only:
            visible = HeadingText()
            visible.feed(prefill + text)
            visible.close()
            if self.finish_reason != "stop" or not close or self.marker not in "".join(visible.text):
                raise RuntimeError(f"Lane {self.marker} did not finish a complete HTML page with its own heading")
        return {
            "marker": self.marker,
            "firstDeltaMs": round(self.first_delta_ms, 2),
            "finishedMs": round(self.finished_ms, 2),
            "finishReason": self.finish_reason,
            "matchedStop": self.matched_stop,
            "closedHtml": close is not None,
        }


def events(response):
    pending = []
    size = 0
    while line := response.readline(MAX_EVENT_BYTES + 1):
        size += len(line)
        if size > MAX_EVENT_BYTES:
            raise RuntimeError("Albatross sent an oversized SSE event")
        line = line.rstrip(b"\r\n")
        if not line:
            if pending:
                yield b"\n".join(pending).decode("utf-8")
            pending = []
            size = 0
        elif line.startswith(b"data:"):
            pending.append(line[5:].removeprefix(b" "))
    if pending:
        raise RuntimeError("Albatross disconnected inside an SSE event")


def make_prompts(lanes: list[Lane]):
    source = (ROOT / "src-tauri" / "src" / "prompt.rs").read_text(encoding="utf-8")

    def constant(name):
        match = re.search(r'pub const ' + name + r': &str = r#"(.*?)"#;', source, re.S)
        if match is None:
            raise RuntimeError(f"Missing application prompt constant {name}")
        return match.group(1)

    prefix, instruction, prefill = (constant(name) for name in ("WEBPAGE_PREFIX", "STREAMING_INSTRUCTION", "HTML_PREFILL"))
    prompts = []
    for lane, topic in zip(lanes, TOPICS):
        text = f"创建{topic}官网。首个可见标题使用 <h1>{lane.marker} {topic}</h1>，页面至少有简介与联系方式，最后关闭 HTML。"
        prompts.append(f"System: {instruction}\n\nUser: {prefix} {text}\n\nAssistant: <think></think>\n{prefill}")
    return prompts, prefill


def measure_batch(client: Client, model: str, lanes: list[Lane], prompts: list[str], max_tokens: int, timeout: float):
    payload = {
        "model": model, "contents": prompts, "stream": True, "max_tokens": max_tokens,
        "stop": ["</html>", "</HTML>", "\n\nUser:", "<EOD>"], "chunk_size": 8, "metrics": True,
        "temperature": 1.0, "top_k": 500, "top_p": 0.5,
        "alpha_presence": 1.0, "alpha_frequency": 0.1, "alpha_decay": 0.99,
    }
    request_id = None
    done = False
    started = time.monotonic()
    with client.request(BATCH_PATH, body=payload, timeout=timeout) as response:
        if response.headers.get_content_type() != "text/event-stream":
            raise RuntimeError("Albatross batch endpoint did not return an SSE stream")
        for event in events(response):
            if event == "[DONE]":
                done = True
                break
            payload = json.loads(event)
            if not isinstance(payload, dict):
                raise RuntimeError("Albatross returned a non-object SSE event")
            if "error" in payload:
                raise RuntimeError(f"Albatross rejected the batch: {payload['error']}")
            identifier = payload.get("id")
            if not isinstance(identifier, str) or not identifier or request_id not in (None, identifier):
                raise RuntimeError("Albatross sent a missing or inconsistent request ID")
            request_id = identifier
            choices = payload.get("choices")
            if not isinstance(choices, list) or not choices:
                raise RuntimeError("Albatross SSE event is missing indexed choices")
            elapsed_ms = (time.monotonic() - started) * 1000
            for choice in choices:
                index = choice.get("index") if isinstance(choice, dict) else None
                if type(index) is not int or not 0 <= index < len(lanes):
                    raise RuntimeError(f"Invalid Albatross lane index: {index!r}")
                lanes[index].observe(choice, elapsed_ms)
    if not done or request_id is None:
        raise RuntimeError("Albatross disconnected before completing the batch with [DONE]")
    return request_id


def server_metrics(client: Client, request_id: str) -> dict | None:
    # The native stream has no token counts. Best-effort read only the matching final batch.
    for _ in range(4):
        try:
            last = client.status().get("last_request")
        except (OSError, RuntimeError, ValueError):
            return None
        if isinstance(last, dict) and last.get("id") == request_id:
            values = {}
            for name in ("prompt_tokens", "generated_tokens", "prefill_speed", "decode_speed"):
                value = last.get(name)
                if type(value) in (int, float) and math.isfinite(value) and value > 0:
                    values[name] = value
            return values or None
        time.sleep(0.1)
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default=os.environ.get("ALBATROSS_URL", "http://127.0.0.1:8000"))
    parser.add_argument("--concurrency", type=int, choices=range(4, 17, 2), default=8)
    parser.add_argument("--max-tokens", type=int, default=MAX_OUTPUT_TOKENS)
    parser.add_argument("--timeout", type=float, default=180, help="Timeout in seconds per read during inference")
    parser.add_argument("--transport-only", action="store_true", help="Require every lane to stream and finish, but allow incomplete pages")
    args = parser.parse_args()
    if not 1 <= args.max_tokens <= MAX_OUTPUT_TOKENS or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error(f"--max-tokens must be 1–{MAX_OUTPUT_TOKENS} and --timeout must be a positive number")
    key = os.environ.get("ALBATROSS_API_KEY", "")
    try:
        client = Client(args.url, key)
        status = client.status()
        engine = status.get("engine_version")
        capabilities = status.get("capabilities")
        model = status.get("model")
        if (
            status.get("status") != "running"
            or engine != "albatross-1.3.0"
            or status.get("api_version") != "1.3"
            or not isinstance(capabilities, dict)
            or not capabilities.get("batch_completion")
            or not capabilities.get("stream")
            or not isinstance(model, dict)
            or not isinstance(model.get("id"), str)
            or not model["id"].strip()
        ):
            raise RuntimeError("No running native Albatross model with batch SSE support was found")
        prefill_queue = status.get("prefill_queue")
        hard_limit = prefill_queue.get("hard_max_bsz") if isinstance(prefill_queue, dict) else None
        if type(hard_limit) is int and 0 < hard_limit < args.concurrency:
            raise RuntimeError(f"Albatross supports at most {hard_limit} lanes on this host")
        lanes = [Lane(f"RWKV-CHECK-{index:02}") for index in range(1, args.concurrency + 1)]
        prompts, prefill = make_prompts(lanes)
        request_id = measure_batch(client, model["id"], lanes, prompts, args.max_tokens, args.timeout)
        summary = [lane.summary(prefill, args.transport_only) for lane in lanes]
        print(json.dumps({
            "engine": engine,
            "model": model["id"],
            "requestId": request_id,
            "inferenceRequests": 1,
            "transportOnly": args.transport_only,
            "lanes": summary,
            "serverBatchMetrics": server_metrics(client, request_id),
        }, indent=2, ensure_ascii=False))
        return 0
    except (OSError, RuntimeError, ValueError) as error:
        message = str(error)
        if key:
            message = message.replace(key, "[redacted]")
        parser.exit(1, f"Albatross smoke test failed: {message}\n")
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
