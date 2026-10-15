use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

fn default_interface_language() -> String {
    "zh-Hant".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub shortcut: String,
    pub target_language: String,
    pub source_language: String,
    #[serde(default = "default_interface_language")]
    pub interface_language: String,
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn old_settings_without_interface_language_default_to_traditional_chinese() {
        let settings: Settings = serde_json::from_str(
            r#"{"api_key":"","base_url":"","model":"","shortcut":"","target_language":"","source_language":""}"#,
        )
        .unwrap();

        assert_eq!(settings.interface_language, "zh-Hant");
    }
}

impl Settings {
    fn default_path() -> Result<PathBuf, String> {
        let app_data_dir = std::env::var("APPDATA")
            .map(PathBuf::from)
            .or_else(|_| {
                std::env::current_dir().map(|dir| dir.join(".quicktranslate"))
            })
            .map_err(|err| format!("Failed to resolve app data dir: {err}"))?;

        let path = app_data_dir.join("QuickTranslate").join("settings.json");
        Ok(path)
    }

    pub fn default() -> Self {
        let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());

        Self {
            api_key,
            base_url,
            model: "qwen2.5:3b".to_string(),
            shortcut: "Ctrl+Shift+T".to_string(),
            target_language: "zh-Hant".to_string(),
            source_language: "en".to_string(),
            interface_language: "zh-Hant".to_string(),
        }
    }

    pub fn load() -> Result<Self, String> {
        let path = Self::default_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)
            .map_err(|err| format!("Failed to read settings file: {err}"))?;

        if contents.trim().is_empty() {
            return Ok(Self::default());
        }

        let mut settings: Self = serde_json::from_str(&contents)
            .map_err(|err| format!("Failed to parse settings file: {err}"))?;

        if settings.api_key.is_empty() {
            settings.api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
        }
        if settings.base_url.is_empty() {
            settings.base_url = std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
        }
        if settings.model.trim().is_empty() {
            settings.model = "qwen2.5:3b".to_string();
        }
        if settings.shortcut.is_empty() {
            settings.shortcut = "Ctrl+Shift+T".to_string();
        }
        if settings.target_language.is_empty() {
            settings.target_language = "zh-Hant".to_string();
        }
        if settings.source_language.is_empty() {
            settings.source_language = "en".to_string();
        }
        if !matches!(
            settings.interface_language.as_str(),
            "zh-Hant" | "zh-Hans" | "en" | "ja" | "ko"
        ) {
            settings.interface_language = "zh-Hant".to_string();
        }

        Ok(settings)
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::default_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create settings directory: {err}"))?;
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|err| format!("Failed to serialize settings: {err}"))?;
        fs::write(path, json)
            .map_err(|err| format!("Failed to save settings: {err}"))?;
        Ok(())
    }
}
