# RWKV Vibe Code Desktop

用本地 RWKV7 G1l 1.5B Q6_K 模型生成单文件 HTML。默认同时生成四个方案，每张预览会随输出更新。点击方案可查看源码和预览，生成结束后可编辑 HTML。

- 输入一条需求，可选择 1–4 个独立采样的方案。
- 输入多条需求，用单独一行 `---` 分隔。每批最多 4 条，每条最多 2000 字。
- 停止会取消当前批次，保留已有输出。推理引擎继续预热，供下一批使用。

界面、中文示例和网页生成提示词参考 [RWKV Vibe Code](https://github.com/RWKV-Vibe/rwkv-vibe-code/tree/189b74463dfbf98d793be8d92dd82ec51996d927)。

## 本地开发

桌面端目前以 macOS Apple Silicon 为目标。开发需要 Node.js 24、pnpm 11.25.0、Python 3、Xcode Command Line Tools 和 Rustup。Rust 版本由 `rust-toolchain.toml` 固定。

打包脚本使用 Homebrew 布局的 llama.cpp **0.9.0（cd959bae9）** 及配套的 GGML 等动态库。默认从 PATH 查找 `llama-server`，也可用 `LLAMA_SERVER_PATH` 指向匹配版本的可执行文件。

```bash
pnpm install --frozen-lockfile
pnpm desktop:dev
```

`pnpm dev` 可单独预览前端。本地推理在 Tauri 桌面窗口内运行。

## 模型与安装包

```bash
pnpm desktop:build
```

生成 `src-tauri/target/release/bundle/macos/织页.app` 和 `src-tauri/target/release/bundle/dmg/织页_0.2.0_aarch64.dmg`。DMG 约 17 MB，包含推理运行时和模型许可。默认配置使用本地测试签名，面向用户分发需配置 Developer ID 签名和公证。

首次启动自动检查并准备约 **1.34 GB** 的模型。应用会复用已安装模型，也可从 `~/Models/rwkv/` 导入。缺失时优先从魔搭国内 CDN 下载，支持暂停、继续和失败换源。模型通过 SHA-256 校验后保存在 `~/Library/Application Support/studio.moment.localpage/models/`，之后可离线推理。预览使用内置 Tailwind 编译器，网页中的 Unsplash 图片仍需网络。

下载地址、固定版本和校验值见 [模型清单](src-tauri/model-manifest.json)。国内主源为 [同作者的 ModelScope 仓库](https://modelscope.cn/models/shoumenchougou/RWKV7-G1l-1.5B-GGUF)，备用顺序为魔搭 API、HF-Mirror、Hugging Face 原站。这个文件经 HF-Mirror 会跳转海外 CDN。

## 流式生成

React 通过 Tauri 调用 Rust 后端。一次生成由一个 llama-server `POST /completion` 请求提交完整的 `prompt` 数组，四槽引擎在同一条 SSE 流中按索引返回结果。Rust 分发各路 HTML、结束状态和引擎报告的 token 计数。React 合并高频更新，预览 iframe 每次处理一个快照并优先渲染最新内容。

每路上下文为 16,384 token，最多生成 12,000 token。状态栏显示合计 TPS、输出 token 和经过时间。预览就绪时间以首个非空正文为准。

## 验证

```bash
pnpm test
pnpm build
pnpm prepare:runtime
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

安装模型后，可运行真实四路流式传输检查。测试使用应用的提示词和打包运行时，结果写入本地 `local-model-results/`。

```bash
pnpm test:concurrency --concurrency 4 --max-tokens 128 --transport-only
```
