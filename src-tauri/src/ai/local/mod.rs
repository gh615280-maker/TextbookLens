mod discovery;
mod http;

use std::time::Duration;

use async_trait::async_trait;
use secrecy::SecretString;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

use crate::{
    ai::{
        error::{AiError, AiErrorKind},
        provider::{AiProvider, ProviderStream, validate_model_id},
    },
    db::providers,
    domain::{
        LocalModelConnectResult, LocalServiceReport, LocalServiceStatus, ProviderKind,
        UnifiedChatRequest, UnifiedRole, ValidationResult,
    },
    errors::AppResult,
};
use discovery::{Installation, local_lm_model, model_key};
use http::{INFERENCE_TIMEOUT, LocalHttp};

const METADATA_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_CONTEXT: u32 = 8_192;
const VISION_CONTEXT: u32 = 16_384;
pub(crate) const LOCAL_IMAGE_LIMITS: crate::domain::ImageLimits = crate::domain::ImageLimits {
    max_images: 1,
    max_encoded_bytes_each: 2 * 1024 * 1024,
    max_total_encoded_bytes: 2 * 1024 * 1024,
    max_dimension_px: 2048,
    max_decoded_pixels_each: 1_048_576,
};

#[derive(Clone)]
pub(crate) struct LocalModel {
    pub id: String,
    pub context: u32,
    pub vision: bool,
    size: u64,
    loaded: bool,
}

pub(crate) struct LocalProvider {
    installation: Installation,
    http: LocalHttp,
    context: u32,
}

impl LocalProvider {
    pub fn new(kind: ProviderKind, port: u16, context: u32) -> Result<Self, AiError> {
        if !kind.is_local() || context == 0 {
            return Err(AiError::invalid_input());
        }
        Ok(Self {
            installation: Installation::detect(kind, Some(port)),
            http: LocalHttp::new(port)?,
            context,
        })
    }

    async fn models(&self, cancel: &CancellationToken) -> Result<Vec<LocalModel>, AiError> {
        self.installation.ensure_running(cancel).await?;
        let mut models = if self.installation.kind == ProviderKind::Ollama {
            let list = self
                .http
                .json("/api/tags", None, cancel, METADATA_TIMEOUT)
                .await?;
            let entries = list["models"]
                .as_array()
                .ok_or_else(AiError::malformed_event)?;
            let loaded = self
                .http
                .json("/api/ps", None, cancel, METADATA_TIMEOUT)
                .await
                .unwrap_or(Value::Null);
            let mut models = Vec::new();
            for entry in entries.iter().take(256) {
                let Some(id) = entry["name"].as_str().filter(|id| valid_local_id(id)) else {
                    continue;
                };
                let info = match self
                    .http
                    .json(
                        "/api/show",
                        Some(json!({"model": id})),
                        cancel,
                        METADATA_TIMEOUT,
                    )
                    .await
                {
                    Ok(info) => info,
                    Err(error) if error.kind() == AiErrorKind::Cancelled => return Err(error),
                    Err(_) => continue,
                };
                if !local_ollama_info(&info) {
                    continue;
                }
                let vision = ollama_has_vision(&info);
                let context = info["model_info"]
                    .as_object()
                    .and_then(|fields| {
                        fields
                            .iter()
                            .find(|(key, _)| key.ends_with(".context_length"))
                    })
                    .and_then(|(_, value)| value.as_u64())
                    .and_then(|value| u32::try_from(value).ok())
                    .unwrap_or(DEFAULT_CONTEXT)
                    .min(if vision {
                        VISION_CONTEXT
                    } else {
                        DEFAULT_CONTEXT
                    });
                if context < 2048 {
                    continue;
                }
                models.push(LocalModel {
                    id: id.to_owned(),
                    context,
                    vision,
                    size: entry["size"].as_u64().unwrap_or(u64::MAX),
                    loaded: loaded["models"].as_array().is_some_and(|models| {
                        models
                            .iter()
                            .any(|model| model["name"] == id || model["model"] == id)
                    }),
                });
            }
            models
        } else {
            let list = self.installation.lmstudio_models(cancel).await?;
            parse_lmstudio_models(&list)?
        };
        models.sort_by(|a, b| {
            b.loaded
                .cmp(&a.loaded)
                .then(a.size.cmp(&b.size))
                .then(a.id.cmp(&b.id))
        });
        models.dedup_by(|a, b| a.id == b.id);
        Ok(models)
    }

    async fn prepare_model(&self, id: &str, cancel: &CancellationToken) -> Result<String, AiError> {
        self.prepare_model_for_input(id, cancel, false).await
    }

    async fn prepare_model_for_input(
        &self,
        id: &str,
        cancel: &CancellationToken,
        vision: bool,
    ) -> Result<String, AiError> {
        validate_model_id(id)?;
        if !valid_local_id(id) {
            return Err(AiError::model_not_found());
        }
        self.installation.ensure_running(cancel).await?;
        if self.installation.kind == ProviderKind::Ollama {
            // Recheck the installed model at each use. localhost alone does not imply local inference.
            let info = self
                .http
                .json(
                    "/api/show",
                    Some(json!({"model": id})),
                    cancel,
                    METADATA_TIMEOUT,
                )
                .await?;
            if !local_ollama_info(&info) {
                return Err(AiError::model_not_found());
            }
            if vision && !ollama_has_vision(&info) {
                return Err(AiError::model_not_found());
            }
            Ok(id.to_owned())
        } else {
            self.installation
                .load_lmstudio(id, self.context, cancel)
                .await
        }
    }

    fn body(&self, id: &str, request: &UnifiedChatRequest, stream: bool) -> Value {
        let mut messages = vec![json!({"role": "system", "content": request.system})];
        messages.extend(request.messages.iter().map(|message| {
            json!({
                "role": if message.role == UnifiedRole::User { "user" } else { "assistant" },
                "content": message.content,
            })
        }));
        if self.installation.kind == ProviderKind::Ollama {
            json!({"model": id, "messages": messages, "stream": stream, "think": false,
                "options": {"num_ctx": self.context, "num_predict": request.max_output_tokens}})
        } else {
            json!({"model": id, "messages": messages, "stream": stream, "max_tokens": request.max_output_tokens})
        }
    }

    fn chat_path(&self) -> &'static str {
        if self.installation.kind == ProviderKind::Ollama {
            "/api/chat"
        } else {
            "/v1/chat/completions"
        }
    }

    async fn smoke_test(&self, id: &str, cancel: &CancellationToken) -> Result<(), AiError> {
        let wire_id = self.prepare_model(id, cancel).await?;
        let request = UnifiedChatRequest {
            model: id.to_owned(),
            system: "Reply briefly.".to_owned(),
            messages: vec![crate::domain::UnifiedMessage {
                role: UnifiedRole::User,
                content: "Reply with OK.".to_owned(),
            }],
            max_output_tokens: super::registry::local_output_tokens(self.context),
            expected_language: None,
        };
        let reply = self
            .http
            .json(
                self.chat_path(),
                Some(self.body(&wire_id, &request, false)),
                cancel,
                INFERENCE_TIMEOUT,
            )
            .await?;
        let message = if self.installation.kind == ProviderKind::Ollama {
            if reply["done"] != true || reply["done_reason"] == "length" {
                return Err(AiError::provider_unavailable());
            }
            &reply["message"]
        } else {
            if reply["choices"][0]["finish_reason"] != "stop" {
                return Err(AiError::provider_unavailable());
            }
            &reply["choices"][0]["message"]
        };
        if !message["content"]
            .as_str()
            .is_some_and(|text| !text.trim().is_empty())
        {
            return Err(AiError::provider_unavailable());
        }
        Ok(())
    }
}

#[async_trait]
impl AiProvider for LocalProvider {
    fn kind(&self) -> ProviderKind {
        self.installation.kind.clone()
    }
    async fn validate(
        &self,
        _credential: &SecretString,
        model: &str,
    ) -> Result<ValidationResult, AiError> {
        let cancel = CancellationToken::new();
        self.prepare_model(model, &cancel).await?;
        Ok(ValidationResult {
            model: model.to_owned(),
            context_window_tokens: self.context,
        })
    }
    async fn stream_chat(
        &self,
        _credential: &SecretString,
        request: UnifiedChatRequest,
        cancel: CancellationToken,
    ) -> Result<ProviderStream, AiError> {
        if request.messages.is_empty() || request.max_output_tokens == 0 {
            return Err(AiError::invalid_input());
        }
        let wire_id = self.prepare_model(&request.model, &cancel).await?;
        let response = self
            .http
            .request(
                self.chat_path(),
                Some(self.body(&wire_id, &request, true)),
                &cancel,
                INFERENCE_TIMEOUT,
            )
            .await?;
        Ok(if self.installation.kind == ProviderKind::Ollama {
            http::ollama_stream(response, cancel)
        } else {
            http::lmstudio_stream(response, cancel)
        })
    }

    async fn stream_vision(
        &self,
        _credential: &SecretString,
        request: crate::domain::UnifiedVisionRequest,
        cancel: CancellationToken,
    ) -> AppResult<ProviderStream> {
        if self.installation.kind != ProviderKind::Ollama {
            return Err(crate::errors::AppError::unsupported_provider_capability());
        }
        let book_id = request
            .images
            .first()
            .map(|image| image.meta.book_id)
            .ok_or_else(|| AiError::invalid_input().into_app_error())?;
        crate::ai::multimodal::validate_vision_request(book_id, &request, LOCAL_IMAGE_LIMITS)?;
        if request
            .text
            .messages
            .last()
            .is_none_or(|message| message.role != UnifiedRole::User)
            || request.text.max_output_tokens == 0
        {
            return Err(AiError::invalid_input().into_app_error());
        }
        let wire_id = self
            .prepare_model_for_input(&request.text.model, &cancel, true)
            .await
            .map_err(AiError::into_app_error)?;
        let mut body = self.body(&wire_id, &request.text, true);
        let messages = body["messages"]
            .as_array_mut()
            .ok_or_else(|| AiError::invalid_input().into_app_error())?;
        messages
            .last_mut()
            .ok_or_else(|| AiError::invalid_input().into_app_error())?["images"] = json!(
            request
                .images
                .iter()
                .map(|image| encode_image(image.bytes()))
                .collect::<Vec<_>>()
        );
        let response = self
            .http
            .request(self.chat_path(), Some(body), &cancel, INFERENCE_TIMEOUT)
            .await
            .map_err(AiError::into_app_error)?;
        Ok(http::ollama_stream(response, cancel))
    }
}

pub async fn connect_local_models(pool: &SqlitePool) -> AppResult<LocalModelConnectResult> {
    let guard = providers::try_provider_mutation(pool).await?;
    let previous_default: Option<String> =
        sqlx::query_scalar("SELECT default_learning_profile_id FROM app_settings WHERE id = 1")
            .fetch_one(pool)
            .await?;
    let cancel = CancellationToken::new();
    let mut reports = Vec::new();
    let mut connected = Vec::new();
    let mut default_id = None;
    for kind in [ProviderKind::Ollama, ProviderKind::LmStudio] {
        let installation = Installation::detect(kind.clone(), None);
        let port = installation.port;
        let provider = LocalProvider::new(kind.clone(), port, DEFAULT_CONTEXT)
            .map_err(AiError::into_app_error)?;
        let models = match provider.models(&cancel).await {
            Ok(models) => models,
            Err(error) => {
                reports.push(LocalServiceReport {
                    kind,
                    status: if error.kind() == AiErrorKind::InvalidApiKey {
                        LocalServiceStatus::AuthenticationRequired
                    } else if installation.cli.is_none() {
                        LocalServiceStatus::NotInstalled
                    } else {
                        LocalServiceStatus::Unavailable
                    },
                    model_count: 0,
                });
                continue;
            }
        };
        let mut working_model = None;
        let mut authentication_required = false;
        for model in &models {
            let provider = LocalProvider::new(kind.clone(), port, model.context)
                .map_err(AiError::into_app_error)?;
            match provider.smoke_test(&model.id, &cancel).await {
                Ok(()) => {
                    working_model = Some(model.id.clone());
                    break;
                }
                Err(error) if error.kind() == AiErrorKind::InvalidApiKey => {
                    authentication_required = true;
                    break;
                }
                Err(_) => {}
            }
        }
        let Some(working_model) = working_model else {
            reports.push(LocalServiceReport {
                kind,
                status: if authentication_required {
                    LocalServiceStatus::AuthenticationRequired
                } else if models.is_empty() {
                    LocalServiceStatus::NoModels
                } else {
                    LocalServiceStatus::NoUsableModels
                },
                model_count: 0,
            });
            continue;
        };
        let profiles =
            providers::save_local_models(pool, &guard, kind.clone(), port, &models).await?;
        if default_id.is_none() {
            default_id = profiles
                .iter()
                .find(|profile| profile.model_id == working_model)
                .map(|profile| profile.id);
        }
        reports.push(LocalServiceReport {
            kind,
            status: LocalServiceStatus::Connected,
            model_count: profiles.len() as u32,
        });
        connected.extend(profiles);
    }
    if let Some(profile) = connected
        .iter()
        .find(|profile| previous_default.as_deref() == Some(profile.id.to_string().as_str()))
    {
        default_id = Some(profile.id);
    }
    if let Some(id) = default_id {
        providers::select_local_default(pool, &guard, id).await?;
    }
    for profile in &mut connected {
        profile.is_active = Some(profile.id) == default_id;
    }
    Ok(LocalModelConnectResult {
        profiles: connected,
        default_profile_id: default_id,
        services: reports,
    })
}

fn valid_local_id(id: &str) -> bool {
    validate_model_id(id).is_ok()
        && !id.to_ascii_lowercase().contains("cloud")
        && !id.starts_with('-')
}

fn local_ollama_info(value: &Value) -> bool {
    value
        .get("remote_host")
        .is_none_or(|v| v.is_null() || v == "")
        && value
            .get("remote_model")
            .is_none_or(|v| v.is_null() || v == "")
        && value["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|cap| cap == "completion"))
        && value["model_info"].is_object()
}

fn ollama_has_vision(value: &Value) -> bool {
    local_ollama_info(value)
        && value["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|cap| cap == "vision"))
}

fn encode_image(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(char::from(TABLE[usize::from(first >> 2)]));
        output.push(char::from(
            TABLE[usize::from(((first & 3) << 4) | (second >> 4))],
        ));
        output.push(if chunk.len() > 1 {
            char::from(TABLE[usize::from(((second & 15) << 2) | (third >> 6))])
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            char::from(TABLE[usize::from(third & 63)])
        } else {
            '='
        });
    }
    output
}

fn parse_lmstudio_models(list: &Value) -> Result<Vec<LocalModel>, AiError> {
    Ok(list
        .as_array()
        .ok_or_else(AiError::malformed_event)?
        .iter()
        .take(256)
        .filter(|model| local_lm_model(model))
        .filter_map(|model| {
            let id = model_key(model).filter(|id| valid_local_id(id))?;
            let context = model["maxContextLength"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(DEFAULT_CONTEXT)
                .min(DEFAULT_CONTEXT);
            (context >= 2048).then(|| LocalModel {
                id: id.to_owned(),
                context,
                vision: false,
                size: model["sizeBytes"].as_u64().unwrap_or(u64::MAX),
                loaded: false,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests;
