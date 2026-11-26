use crate::AppState;
use serde::Serialize;
use tauri::{ipc::Channel, AppHandle, State};

#[derive(Serialize)]
pub(crate) struct Status {
    supported: bool,
    version: String,
}

#[derive(Serialize)]
pub(crate) struct Available {
    version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Progress {
    phase: &'static str,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

#[tauri::command]
pub(crate) fn app_update_status(app: AppHandle) -> Status {
    Status {
        supported: supported(),
        version: app.package_info().version.to_string(),
    }
}

#[tauri::command]
pub(crate) async fn check_app_update(app: AppHandle) -> Result<Option<Available>, String> {
    #[cfg(any(windows, target_os = "linux"))]
    if supported() {
        return Ok(platform::check(&app).await?.map(|update| Available {
            version: update.version,
        }));
    }
    let _ = app;
    Ok(None)
}

#[tauri::command]
pub(crate) async fn install_app_update(
    app: AppHandle,
    state: State<'_, AppState>,
    version: String,
    on_event: Channel<Progress>,
) -> Result<(), String> {
    #[cfg(any(windows, target_os = "linux"))]
    if supported() {
        return platform::install(app, state.inner(), &version, on_event).await;
    }
    let _ = (app, state, version, on_event);
    Err("当前安装方式不支持应用内更新".into())
}

fn supported() -> bool {
    use tauri::utils::{config::BundleType, platform::bundle_type};

    match bundle_type() {
        Some(BundleType::Nsis) => cfg!(windows),
        Some(BundleType::Deb) => cfg!(target_os = "linux"),
        Some(BundleType::AppImage) => {
            cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some()
        }
        _ => false,
    }
}

#[cfg(any(windows, target_os = "linux"))]
mod platform {
    use super::*;
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };
    use tauri_plugin_updater::{Error, Update, UpdaterExt};
    use tokio::sync::{Mutex, MutexGuard};

    static INSTALL: Mutex<()> = Mutex::const_new(());

    pub(super) async fn check(app: &AppHandle) -> Result<Option<Update>, String> {
        app.updater_builder()
            .timeout(Duration::from_secs(12))
            .configure_client(|client| client.https_only(true))
            .build()
            .map_err(check_error)?
            .check()
            .await
            .map_err(check_error)
    }

    fn check_error(error: Error) -> String {
        eprintln!("Update check: {error}");
        "无法检查更新，请检查网络后重试".into()
    }

    fn apply_error(error: Error) -> String {
        eprintln!("Update installation: {error}");
        match error {
            Error::Minisign(_)
            | Error::Base64(_)
            | Error::SignatureUtf8(_)
            | Error::MissingSignedVersion
            | Error::SignedVersionMismatch { .. } => "更新包签名校验失败，请重新检查更新".into(),
            Error::AuthenticationFailed => "系统授权未完成，应用仍可继续使用".into(),
            Error::PackageInstallFailed | Error::DebInstallFailed => {
                "系统未能安装更新。请确认已授权，并在其他软件安装完成后重试".into()
            }
            Error::Reqwest(_) | Error::Network(_) => "下载更新失败，请检查网络后重试".into(),
            Error::PreviousAppNotRestored(path) => {
                format!("安装失败，原版本保存在 {}", path.to_string_lossy())
            }
            _ => "安装更新失败，请检查应用所在位置的写入权限后重试".into(),
        }
    }

    fn idle(state: &AppState) -> Result<MutexGuard<'_, ()>, String> {
        let busy = "请等当前生成或模型准备完成后再安装更新";
        let lock = state.0.lifecycle.try_lock().map_err(|_| busy)?;
        if !state.0.jobs.lock().unwrap().is_empty() {
            return Err(busy.into());
        }
        Ok(lock)
    }

    pub(super) async fn install(
        app: AppHandle,
        state: &AppState,
        version: &str,
        on_event: Channel<Progress>,
    ) -> Result<(), String> {
        let _single_install = INSTALL.try_lock().map_err(|_| "正在安装更新")?;
        drop(idle(state)?);
        let mut update = check(&app).await?.ok_or("当前已经是最新版本")?;
        if update.version != version {
            return Err("检测到更新的版本，请重新检查更新".into());
        }
        if update.download_url.scheme() != "https" {
            return Err("更新包地址不安全，请重新检查更新".into());
        }
        update.timeout = Some(Duration::from_secs(300));
        let progress = |phase, downloaded_bytes, total_bytes| {
            let _ = on_event.send(Progress {
                phase,
                downloaded_bytes,
                total_bytes,
            });
        };
        progress("downloading", 0, None);
        let downloaded = AtomicU64::new(0);
        let mut last_report = Instant::now();
        let transfer = update.download(
            |bytes, total| {
                let received = downloaded.fetch_add(bytes as u64, Ordering::Relaxed) + bytes as u64;
                if last_report.elapsed() >= Duration::from_millis(100) {
                    progress("downloading", received, total);
                    last_report = Instant::now();
                }
            },
            || {
                let received = downloaded.load(Ordering::Relaxed);
                progress("verifying", received, Some(received));
            },
        );
        let archive = tokio::select! {
            _ = state.0.shutdown.cancelled() => return Err("应用已退出".into()),
            result = transfer => result.map_err(apply_error)?,
        };
        let _idle = idle(state)?;
        if state.0.shutdown.is_cancelled() {
            return Err("应用已退出".into());
        }
        let received = downloaded.load(Ordering::Relaxed);
        progress("installing", received, Some(received));
        #[cfg(windows)]
        {
            // The NSIS installer takes over and exits the old process itself.
            let mut engine = state.0.engine.lock().unwrap();
            engine.connection = None;
            engine.process = None;
        }
        tokio::task::spawn_blocking(move || update.install(archive))
            .await
            .map_err(|_| "系统安装器意外退出")?
            .map_err(apply_error)?;
        // Windows normally exits inside install(). Linux restarts the installed bundle.
        state.shutdown();
        app.request_restart();
        Ok(())
    }
}
