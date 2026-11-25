use crate::{albatross, runtime_store::Runtime};
use std::{
    fs::File,
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub fn supported() -> bool {
    cfg!(all(
        target_arch = "x86_64",
        any(target_os = "windows", target_os = "linux")
    ))
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "windows", target_os = "linux")
))]
pub fn check_gpu() -> Result<(), String> {
    use libloading::Library;
    use std::ffi::{c_int, c_uint};

    let check = || -> Option<c_int> {
        // CUDA's driver ABI is provided by the installed NVIDIA graphics driver.
        unsafe {
            #[cfg(target_os = "windows")]
            let driver = Library::from(
                libloading::os::windows::Library::load_with_flags(
                    "nvcuda.dll",
                    libloading::os::windows::LOAD_LIBRARY_SEARCH_SYSTEM32,
                )
                .ok()?,
            );
            #[cfg(target_os = "linux")]
            let driver = Library::new("libcuda.so.1").ok()?;

            let init = driver
                .get::<unsafe extern "system" fn(c_uint) -> c_int>(b"cuInit\0")
                .ok()?;
            let count = driver
                .get::<unsafe extern "system" fn(*mut c_int) -> c_int>(b"cuDeviceGetCount\0")
                .ok()?;
            let version = driver
                .get::<unsafe extern "system" fn(*mut c_int) -> c_int>(b"cuDriverGetVersion\0")
                .ok()?;
            let (mut devices, mut installed_version) = (0, 0);
            (init(0) == 0
                && count(&mut devices) == 0
                && devices > 0
                && version(&mut installed_version) == 0)
                .then_some(installed_version)
        }
    };
    match check() {
        Some(version) if version >= 12000 => Ok(()),
        Some(_) => Err("NVIDIA 驱动版本过旧。更新显卡驱动后重试。".into()),
        None => Err("未检测到可用的 NVIDIA CUDA 显卡。请检查显卡和 NVIDIA 驱动后重试。".into()),
    }
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "windows", target_os = "linux")
)))]
pub fn check_gpu() -> Result<(), String> {
    Err("本机推理支持配有 NVIDIA 显卡的 Windows 和 Linux x64".into())
}

pub struct Process {
    child: Child,
    _lock: File,
}

impl Process {
    pub fn launch(
        runtime: &Runtime,
        model: &Path,
        data: &Path,
    ) -> Result<(Self, albatross::Endpoint), String> {
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data.join("albatross-session.lock"))
            .map_err(|error| format!("无法打开 Albatross 本机数据目录：{error}"))?;
        lock.try_lock()
            .map_err(|_| "另一个 RWKV-Vibe-Code 窗口正在使用 Albatross，请关闭后重试")?;
        let port = TcpListener::bind(("127.0.0.1", 0))
            .and_then(|listener| listener.local_addr())
            .map_err(|error| format!("无法分配 Albatross 本机端口：{error}"))?
            .port();
        let password = Uuid::new_v4().simple().to_string();
        let endpoint =
            albatross::Endpoint::parse(&format!("http://127.0.0.1:{port}"), Some(&password))?;
        let log = File::create(data.join("albatross.log"))
            .map_err(|error| format!("无法写入 Albatross 启动日志：{error}"))?;
        let stderr = log.try_clone().map_err(|error| error.to_string())?;

        // Relative arguments also work when a Windows user's profile path is
        // Unicode and upstream parses arguments through its native C runtime.
        let model_argument = model.strip_prefix(data).unwrap_or(model);
        let vocabulary_argument = runtime
            .vocabulary
            .strip_prefix(data)
            .unwrap_or(&runtime.vocabulary);
        let mut command = Command::new(&runtime.executable);
        command
            .current_dir(data)
            .arg("--model-path")
            .arg(model_argument)
            .arg("--vocab-path")
            .arg(vocabulary_argument)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .args([
                "--chunk-size",
                "128",
                "--chunk-load",
                "--password",
                &password,
            ])
            .arg("--state-db-path")
            .arg("albatross-state.sqlite")
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(stderr);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        #[cfg(target_os = "linux")]
        if let Some(appdir) = std::env::var_os("APPDIR") {
            // AppImage sets a search path for the desktop client which can shadow Albatross libraries.
            let path = std::env::var_os("LD_LIBRARY_PATH").unwrap_or_default();
            let inherited = std::env::split_paths(&path).filter(|path| {
                !path.as_os_str().is_empty() && !path.starts_with(Path::new(&appdir))
            });
            let paths = std::iter::once(runtime.directory.join("lib")).chain(inherited);
            command.env(
                "LD_LIBRARY_PATH",
                std::env::join_paths(paths).map_err(|error| error.to_string())?,
            );
        }
        let child = command.spawn().map_err(|error| {
            format!("无法启动 Albatross。请检查 NVIDIA 驱动和运行时安装。{error}")
        })?;
        Ok((Self { child, _lock: lock }, endpoint))
    }

    pub fn ensure_running(&mut self) -> Result<(), String> {
        match self.child.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(status)) => Err(format!(
                "Albatross 已退出（{status}）。请检查 NVIDIA 驱动和显存。详情见应用数据目录的 albatross.log。"
            )),
            Err(error) => Err(format!("无法检查 Albatross 运行状态：{error}")),
        }
    }

    pub async fn wait_until_ready(
        &mut self,
        endpoint: &albatross::Endpoint,
        client: &reqwest::Client,
        cancel: &CancellationToken,
    ) -> Result<albatross::Server, String> {
        let startup = async {
            loop {
                self.ensure_running()?;
                if let Ok(server) = endpoint.probe(client).await {
                    self.ensure_running()?;
                    return Ok(server);
                }
                tokio::time::sleep(Duration::from_millis(350)).await;
            }
        };
        tokio::select! {
            _ = cancel.cancelled() => Err("应用已退出".into()),
            result = tokio::time::timeout(Duration::from_secs(180), startup) => result
                .map_err(|_| "Albatross 加载模型超时。请检查显卡状态，详情见应用数据目录的 albatross.log。")?,
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[tokio::test]
    async fn launches_one_session_with_local_paths_and_stops_on_drop() {
        let root = std::env::temp_dir().join(format!("Albatross 测试 {}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let binary = root.join("fixture");
        fs::write(
            &binary,
            "#!/bin/sh\nprintf '%s\\n' \"$@\"\nexec /bin/sleep 60\n",
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = Runtime {
            executable: binary,
            vocabulary: root.join("词表.txt"),
            #[cfg(target_os = "linux")]
            directory: root.clone(),
        };
        let model = root.join("模型.pth");
        let (mut process, endpoint) = Process::launch(&runtime, &model, &root).unwrap();
        assert!(Process::launch(&runtime, &model, &root).is_err());
        let log = root.join("albatross.log");
        let output = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let output = fs::read_to_string(&log).unwrap();
                if output.ends_with("albatross-state.sqlite\n") {
                    break output;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(output.contains("--model-path\n模型.pth\n--vocab-path\n词表.txt\n"));
        assert!(output.contains("--host\n127.0.0.1\n--port\n"));
        assert!(output.contains("--chunk-load\n--password\n"));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(process
            .wait_until_ready(&endpoint, &albatross::configure_client(), &cancel)
            .await
            .is_err());
        drop(process);
        let (replacement, _) = Process::launch(&runtime, &model, &root).unwrap();
        drop(replacement);
        fs::remove_dir_all(root).unwrap();
    }
}
