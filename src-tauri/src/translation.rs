use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

use crate::config::Settings;

static HTTP_CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationResult {
    pub original: String,
    pub translated: String,
    pub source_language: String,
    pub target_language: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

fn build_translation_prompts(
    text: &str,
    source_language: &str,
    target_language: &str,
) -> (String, String) {
    if source_language == "auto" {
        (
            format!(
                "Detect the input language and translate it accurately into {target_language}. Return only the translation, with no explanation."
            ),
            text.to_string(),
        )
    } else {
        (
            "Translate the input accurately. Return only the translation, with no explanation."
                .to_string(),
            format!("Translate {source_language} to {target_language}: {text}"),
        )
    }
}

pub fn translate_text(
    text: &str,
    source_language: &str,
    target_language: &str,
) -> Result<TranslationResult, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("No text to translate.".to_string());
    }

    let settings = Settings::load().unwrap_or_else(|_| Settings::default());
    let base_url = std::env::var("OPENAI_BASE_URL")
        .or_else(|_| {
            if settings.base_url.trim().is_empty() {
                Err(std::env::VarError::NotPresent)
            } else {
                Ok(settings.base_url.clone())
            }
        })
        .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());

    let use_local_model = base_url.contains("localhost") || base_url.contains("127.0.0.1");
    let api_key = std::env::var("OPENAI_API_KEY")
        .or_else(|_| {
            if settings.api_key.trim().is_empty() {
                if use_local_model {
                    Ok("ollama".to_string())
                } else {
                    Err(std::env::VarError::NotPresent)
                }
            } else {
                Ok(settings.api_key.clone())
            }
        })
        .map_err(|_| "OPENAI_API_KEY is not set. For local Ollama use a placeholder like 'ollama' or set OPENAI_API_KEY in your environment.".to_string())?;

    let model_name = if settings.model.trim().is_empty() {
        if use_local_model {
            "qwen2.5:3b".to_string()
        } else {
            "gpt-4o-mini".to_string()
        }
    } else {
        settings.model.clone()
    };

    let (system_prompt, user_prompt) =
        build_translation_prompts(trimmed, source_language, target_language);

    let client = HTTP_CLIENT.get_or_init(reqwest::blocking::Client::new);
    let response = client
        .post(format!("{}/chat/completions", base_url.trim_end_matches('/')))
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": model_name,
            "max_tokens": 64,
            "temperature": 0,
            "messages": [
                {
                    "role": "system",
                    "content": system_prompt
                },
                {
                    "role": "user",
                    "content": user_prompt
                }
            ]
        }))
        .send()
        .map_err(|err| format!("Translation request failed: {err}"))?;

    if !response.status().is_success() {
        let error_body = response
            .text()
            .unwrap_or_else(|_| "unknown error".to_string());
        return Err(format!("Translation API error: {error_body}"));
    }

    let api_response: OpenAiResponse = response
        .json()
        .map_err(|err| format!("Failed to parse translation response: {err}"))?;

    let translated = api_response
        .choices
        .into_iter()
        .find_map(|choice| match choice.message.content {
            serde_json::Value::String(value) => Some(value),
            serde_json::Value::Array(items) => Some(
                items
                    .into_iter()
                    .filter_map(|item| {
                        item.get("text")
                            .and_then(|value| value.as_str())
                            .map(str::to_owned)
                    })
                    .collect::<Vec<String>>()
                    .join(""),
            ),
            _ => None,
        })
        .unwrap_or_else(|| trimmed.to_string());

    Ok(TranslationResult {
        original: trimmed.to_string(),
        translated: translated.trim().to_string(),
        source_language: source_language.to_string(),
        target_language: target_language.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{translate_text, build_translation_prompts};

    #[test]
    fn auto_source_asks_model_to_detect_language() {
        let (system_prompt, user_prompt) = build_translation_prompts("bonjour", "auto", "zh-Hant");

        assert!(system_prompt.contains("Detect the input language"));
        assert!(system_prompt.contains("zh-Hant"));
        assert_eq!(user_prompt, "bonjour");
    }

    #[test]
    fn rejects_empty_input() {
        let result = translate_text("   ", "en", "zh-Hant");
        assert!(result.is_err());
    }
}
