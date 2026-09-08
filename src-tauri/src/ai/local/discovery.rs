use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::http::LocalHttp;
use crate::{ai::error::AiError, domain::ProviderKind};

const CLI_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(super) struct Installation {
    pub kind: ProviderKind,
    pub port: u16,
    pub cli: Option<PathBuf>,
}

impl Installation {
    pub fn detect(kind: ProviderKind, saved_port: Option<u16>) -> Self {
        let mut candidates = Vec::new();
        let name = if kind == ProviderKind::Ollama {
            "ollama.exe"
        } else {
            "lms.exe"
        };
        if let Some(local) = std::env::var_os("LOCALAPPDATA")
            && kind == ProviderKind::Ollama
        {
            candidates.push(PathBuf::from(local).join("Programs/Ollama/ollama.exe"));
        }
        if kind == ProviderKind::LmStudio {
            for home in lmstudio_homes() {
                candidates.push(home.join("bin/lms.exe"));
            }
        }
        if let Some(paths) = std::env::var_os("PATH") {
            candidates.extend(
                std::env::split_paths(&paths)
                    .filter(|path| path.is_absolute())
                    .map(|path| path.join(name)),
            );
        }
        let cli = candidates.into_iter().find(|path| path.is_file());
        let port = saved_port.unwrap_or_else(|| {
            if kind == ProviderKind::Ollama {
                std::env::var("OLLAMA_HOST")
                    .ok()
                    .and_then(|host| loopback_port(&host))
                    .unwrap_or(11434)
            } else {
                lmstudio_homes()
                    .iter()
                    .find_map(|home| {
                        read_small_json(&home.join(".internal/http-server-config.json"))
                            .and_then(|value| value["port"].as_u64())
                            .and_then(|port| u16::try_from(port).ok())
                            .filter(|port| *port > 0)
                    })
                    .unwrap_or(1234)
            }
        });
        Self { kind, port, cli }
    }

    pub async fn ensure_running(&self, cancel: &CancellationToken) -> Result<(), AiError> {
        let http = LocalHttp::new(self.port)?;
        let path = if self.kind == ProviderKind::Ollama {
            "/api/tags"
        } else {
            "/lmstudio-greeting"
        };
        if http
            .json(path, None, cancel, Duration::from_secs(2))
            .await
            .is_ok()
        {
            return Ok(());
        }
        let cli = self
            .cli
            .as_ref()
            .ok_or_else(AiError::provider_unavailable)?;
        if self.kind == ProviderKind::Ollama {
            let mut command = hidden_command(cli);
            command
                .arg("serve")
                .env("OLLAMA_HOST", format!("127.0.0.1:{}", self.port))
                .env("OLLAMA_NO_CLOUD", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            // This is a reusable local server, intentionally allowed to outlive this request.
            let mut child = command
                .spawn()
                .map_err(|_| AiError::provider_unavailable())?;
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
        } else {
            run_cli(
                cli,
                &[
                    "server",
                    "start",
                    "--port",
                    &self.port.to_string(),
                    "--bind",
                    "127.0.0.1",
                ],
                cancel,
                CLI_TIMEOUT,
            )
            .await?;
        }
        for _ in 0..30 {
            if http
                .json(path, None, cancel, Duration::from_secs(1))
                .await
                .is_ok()
            {
                return Ok(());
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err(AiError::cancelled()),
                _ = tokio::time::sleep(Duration::from_millis(500)) => {},
            }
        }
        Err(AiError::provider_unavailable())
    }

    pub async fn lmstudio_models(&self, cancel: &CancellationToken) -> Result<Value, AiError> {
        let cli = self
            .cli
            .as_ref()
            .ok_or_else(AiError::provider_unavailable)?;
        cli_json(cli, &["ls", "--llm", "--json"], cancel).await
    }

    pub async fn load_lmstudio(
        &self,
        key: &str,
        context: u32,
        cancel: &CancellationToken,
    ) -> Result<String, AiError> {
        let cli = self
            .cli
            .as_ref()
            .ok_or_else(AiError::provider_unavailable)?;
        let models = self.lmstudio_models(cancel).await?;
        let model = models
            .as_array()
            .and_then(|models| {
                models
                    .iter()
                    .find(|model| local_lm_model(model) && model_key(model) == Some(key))
            })
            .ok_or_else(AiError::model_not_found)?;
        let supports_local = model.get("deviceIdentifier").is_some();
        let loaded = cli_json(cli, &["ps", "--json"], cancel).await?;
        if let Some(model) = loaded.as_array().and_then(|models| {
            models.iter().find(|model| {
                local_lm_model(model)
                    && model_key(model) == Some(key)
                    && model["contextLength"]
                        .as_u64()
                        .is_some_and(|tokens| tokens >= u64::from(context))
            })
        }) && let Some(id) = model["identifier"].as_str()
        {
            return Ok(id.to_owned());
        }
        let digest: String = Sha256::digest(key.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let identifier = format!("textbooklens-{digest}");
        let context = context.to_string();
        let mut args = vec![
            "load",
            key,
            "--identifier",
            &identifier,
            "--context-length",
            &context,
            "--yes",
        ];
        // Modern LM Link-aware CLIs provide --local. Older CLIs only know local models.
        if supports_local {
            args.push("--local");
        }
        run_cli(cli, &args, cancel, super::http::INFERENCE_TIMEOUT).await?;
        let loaded = cli_json(cli, &["ps", "--json"], cancel).await?;
        if !loaded.as_array().is_some_and(|models| {
            models.iter().any(|model| {
                local_lm_model(model)
                    && model["identifier"] == identifier
                    && model_key(model) == Some(key)
            })
        }) {
            return Err(AiError::provider_unavailable());
        }
        Ok(identifier)
    }
}

pub(super) fn model_key(value: &Value) -> Option<&str> {
    value["modelKey"]
        .as_str()
        .or_else(|| value["path"].as_str())
}

pub(super) fn local_lm_model(value: &Value) -> bool {
    value["type"] == "llm" && value.get("deviceIdentifier").is_none_or(Value::is_null)
}

fn lmstudio_homes() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    if let Some(home) = std::env::var_os("LMSTUDIO_HOME") {
        homes.push(PathBuf::from(home));
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let home = PathBuf::from(home);
        homes.push(home.join(".lmstudio"));
        homes.push(home.join(".cache/lm-studio"));
    }
    homes
}

fn read_small_json(path: &Path) -> Option<Value> {
    if std::fs::metadata(path).ok()?.len() > 64 * 1024 {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn loopback_port(host: &str) -> Option<u16> {
    let host = if host.contains("://") {
        host.to_owned()
    } else {
        format!("http://{host}")
    };
    let url = reqwest::Url::parse(&host).ok()?;
    if !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "0.0.0.0")) {
        return None;
    }
    url.port().filter(|port| *port > 0)
}

fn hidden_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    command.stdin(Stdio::null());
    command
}

async fn cli_json(
    path: &Path,
    args: &[&str],
    cancel: &CancellationToken,
) -> Result<Value, AiError> {
    let output = run_cli(path, args, cancel, CLI_TIMEOUT).await?;
    serde_json::from_slice(&output).map_err(|_| AiError::provider_unavailable())
}

async fn run_cli(
    path: &Path,
    args: &[&str],
    cancel: &CancellationToken,
    timeout: Duration,
) -> Result<Vec<u8>, AiError> {
    let mut command = hidden_command(path);
    command
        .args(args)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // No shell, installation commands, model downloads, or remote-host arguments.
    let output = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(AiError::cancelled()),
        result = tokio::time::timeout(timeout, command.output()) => result.map_err(|_| AiError::provider_unavailable())?.map_err(|_| AiError::provider_unavailable())?,
    };
    if !output.status.success() || output.stdout.len() > 4 * 1024 * 1024 {
        return Err(AiError::provider_unavailable());
    }
    Ok(output.stdout)
}
