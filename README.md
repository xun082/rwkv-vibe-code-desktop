# RWKV-Vibe-Code

用 Albatross 和 RWKV-7 并行生成单文件 HTML。默认一批生成八个方案，显存紧张时根据 Albatross 上报的容量降低默认并发。预览随输出更新并缓慢下滚，手动滚动后暂停跟随，滚动停留在当前预览内。可查看源码、编辑已完成的 HTML，也可从列表或详情返回首页后继续查看当前方案。

- 输一条需求时，每路独立采样。支持 2、4、6、8、10、12、14、16 路，并可选「自动」。自动档根据服务报告的 GPU 批处理容量选择偶数档位。
- 多条需求用单独一行 `---` 分隔。每批最多 16 条且不超过所选并发数，每条最多 2000 字。
- 「停止」关闭当前批次的流，并在取得本批请求 ID 后向 Albatross 取消该请求。已返回的源码可以继续查看。服务在首帧前预填充时，只能先关闭客户端连接。
- 每个方案的输出上限为 60,000 tokens。解码使用 Temperature 1、Top P 0.5、Top K 500、Presence Penalty 1、Count Penalty 0.1 和 Penalty Decay 0.99。

界面、中文示例和网页生成提示词参考 [RWKV Vibe Code](https://github.com/RWKV-Vibe/rwkv-vibe-code/tree/189b74463dfbf98d793be8d92dd82ec51996d927)。

## 在 Windows 和 Linux 使用

在 x64 电脑上安装桌面包。Windows 使用 `-setup.exe`。Ubuntu 22.04 或更新版本可使用 `.deb`，兼容的 Linux 桌面也可运行 `.AppImage`。本机推理需要支持 CUDA 12.9 版 Albatross 的 NVIDIA 显卡和驱动。

打开应用，点「安装模型」。应用自动下载、校验 Albatross 运行时和 RWKV-7 G1l 1.5B 权重，然后启动本机推理。首次下载需要联网，Windows 约 3.67 GB，Linux 约 4.04 GB。下载可以暂停，下次从已下载的部分继续。安装后重开应用会自动加载模型，离线也可生成。

运行时与权重保存在当前用户的应用数据目录 `studio.moment.localpage`。Windows 的目录位于 `%LOCALAPPDATA%`，Linux 位于 `$XDG_DATA_HOME`，默认为 `~/.local/share`。Albatross 仅监听本机回环地址，关闭应用时停止推理进程。启动故障的详情记录在应用数据目录的 `albatross.log`。

下载来源及版本见 [RWKV Lightning CUDA v1.8.15](https://github.com/Alic-Li/rwkv_lightning_cuda/releases/tag/v1.8.15) 和 [RWKV-7 G1l 1.5B](https://huggingface.co/BlinkDL/rwkv7-g1/blob/11e3c50128e479898eee79c48831a95cf3e21d91/rwkv7-g1l-1.5b-20261031-ctx34304.pth)。

从 v0.2.7 起，应用启动及运行期间会检查更新。发现新版本时，点右上角的更新按钮，应用会下载并校验签名，安装后重新打开。Debian 安装的应用可能显示系统管理员授权窗口。v0.2.5 及更早版本的用户需要先用新的安装包升级一次。

## 本地开发与打包

开发需要 Node.js 24、pnpm 11.25.0 和 Rustup。Rust 版本由 `rust-toolchain.toml` 固定。macOS 开发需要 Xcode Command Line Tools，Windows 开发需要 Visual Studio C++ Build Tools（x64）和 WebView2。

Ubuntu 22.04 x64 开发和打包依赖如下。

```bash
sudo apt-get update
sudo apt-get install build-essential file libwebkit2gtk-4.1-dev libxdo-dev \
  libayatana-appindicator3-dev librsvg2-dev libfuse2
```

```bash
pnpm install --frozen-lockfile
pnpm desktop:dev
```

macOS 可以开发和预览界面。需要联调时，macOS 的调试构建可读取 `ALBATROSS_URL` 和可选的 `ALBATROSS_API_KEY`，连接开发用的 Albatross。浏览器用 `pnpm dev` 预览，桌面逻辑用 `pnpm desktop:dev` 调试。

- Windows x64 用 `pnpm desktop:build --bundles nsis` 生成 NSIS 安装包。
- Linux x64 用 `NO_STRIP=1 pnpm desktop:build` 生成 `.deb` 和 `.AppImage`。

构建产物位于 `src-tauri/target/release/bundle/`。GitHub Actions 验证 Windows 和 Linux 安装包能启动，并发布安装包及 SHA-256 校验文件。

## 流式生成与验证

一次生成向 Albatross 的 `POST /v1/batch/completions` 提交完整的 `contents`，GPU 服务在同一条 SSE 流中返回按索引标记的 HTML 增量和各路结束事件。首段延迟由桌面端测量。生成中依据本批请求的输出 token 计数采样显示「实时约 … TPS」，批次完成后显示 Albatross 报告的解码 TPS 和 token 总数。预览使用内置 Tailwind 编译器，网页中的 Unsplash 图片仍需网络。

```bash
pnpm test
pnpm build
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

联调实际 Albatross 时，可以运行以下检查。测试发送四条不同需求，核对索引、各路结束事件和完整 HTML。只验证传输时可附加 `--max-tokens 128 --transport-only`。

```bash
pnpm test:concurrency --concurrency 4
```

协议参考 [原生接口说明](https://github.com/Alic-Li/rwkv_lightning_cuda/blob/v1.8.15/docs/http-api.md#batch-completions)。
