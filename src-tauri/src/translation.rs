use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use crate::config::Settings;

static HTTP_CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
static TRANSLATION_CACHE: OnceLock<Mutex<HashMap<TranslationCacheKey, TranslationResult>>> =
    OnceLock::new();

#[derive(Debug, Hash, PartialEq, Eq)]
struct TranslationCacheKey {
    base_url: String,
    model: String,
    api_key_fingerprint: u64,
    source_language: String,
    target_language: String,
    text: String,
    context: String,
}

fn language_name(language: &str) -> &str {
    match language {
        "auto" => "the detected source language",
        "en" => "English",
        "zh-Hant" => "Traditional Chinese",
        "zh-Hans" => "Simplified Chinese",
        "ja" => "Japanese",
        "ko" => "Korean",
        "fr" => "French",
        "de" => "German",
        "es" => "Spanish",
        other => other,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationResult {
    pub original: String,
    pub translated: String,
    pub source_language: String,
    pub target_language: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DictionaryEntryFields {
    part_of_speech: String,
    meanings: Vec<DictionaryMeaning>,
    examples: Vec<DictionaryExample>,
    #[serde(default)]
    phrasal_verbs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DictionaryMeaning {
    terms: Vec<String>,
    explanation: String,
}

#[derive(Debug, Deserialize)]
struct DictionaryExample {
    source: String,
    translated: String,
}

impl DictionaryEntryFields {
    fn from_model_output(
        text: &str,
        original: &str,
        target_language: &str,
    ) -> Result<Self, String> {
        let mut fields: Self = serde_json::from_str(text)
            .map_err(|err| format!("Failed to parse structured translation response: {err}"))?;

        if fields.part_of_speech.trim().is_empty()
            || fields.meanings.is_empty()
            || fields.meanings.iter().any(|meaning| {
                meaning.terms.is_empty()
                    || meaning.terms.iter().any(|term| term.trim().is_empty())
                    || meaning.explanation.trim().is_empty()
            })
            || fields
                .phrasal_verbs
                .iter()
                .any(|phrase| phrase.trim().is_empty())
        {
            return Err("Translation model returned incomplete dictionary fields.".to_string());
        }

        if fields.meanings.iter().any(|meaning| {
            meaning.terms.iter().any(|term| same_word(term, original))
                || same_word(&meaning.explanation, original)
        }) {
            return Err(
                "Translation model repeated the target word instead of defining its meaning."
                    .to_string(),
            );
        }

        if fields.meanings.iter().any(|meaning| {
            meaning
                .terms
                .iter()
                .any(|term| contains_prompt_artifact(term))
                || contains_prompt_artifact(&meaning.explanation)
        }) {
            return Err(
                "Translation model copied prompt or validation text into the dictionary entry."
                    .to_string(),
            );
        }

        fields.examples.retain(|example| {
            !example.source.trim().is_empty()
                && !example.translated.trim().is_empty()
                && contains_target_form(&example.source, original)
                && !contains_prompt_artifact(&example.source)
                && !contains_prompt_artifact(&example.translated)
        });

        if matches!(target_language, "zh-Hant" | "zh-Hans")
            && fields
                .meanings
                .iter()
                .any(|meaning| {
                    meaning
                        .terms
                        .iter()
                        .any(|term| !contains_chinese(term))
                        || !contains_chinese(&meaning.explanation)
                })
        {
            return Err(
                "Translation model did not provide Chinese terms and an explanation for each meaning."
                    .to_string(),
            );
        }

        Ok(fields)
    }

    fn format(&self, original: &str, target_language: &str) -> Result<String, String> {
        let mut lines = Vec::new();
        let part_of_speech =
            convert_target_script(&single_line(&self.part_of_speech), target_language)?;
        let part_of_speech = part_of_speech.trim_matches(|ch| ch == '(' || ch == ')');
        lines.push(format!("{original} ({part_of_speech})"));

        let (meaning_label, example_label, phrasal_verb_label) = field_labels(target_language);
        lines.push(meaning_label.to_string());
        for meaning in &self.meanings {
            let terms = meaning
                .terms
                .iter()
                .map(|term| convert_target_script(&single_line(term), target_language))
                .collect::<Result<Vec<_>, _>>()?
                .join("、");
            lines.push(terms);
            lines.push(convert_target_script(
                &single_line(&meaning.explanation),
                target_language,
            )?);
        }

        if !self.examples.is_empty() {
            lines.push(example_label.to_string());
            for example in &self.examples {
                lines.push(ensure_sentence_punctuation(
                    &normalize_sentence_case(&example.source),
                    target_language,
                ));
                lines.push(ensure_sentence_punctuation(
                    &convert_target_script(&example.translated, target_language)?,
                    target_language,
                ));
            }
        }

        if !self.phrasal_verbs.is_empty() {
            lines.push(phrasal_verb_label.to_string());
            lines.extend(self.phrasal_verbs.iter().map(|phrase| single_line(phrase)));
        }
        Ok(lines.join("\n"))
    }
}

fn contains_prompt_artifact(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    [
        "did not provide",
        "failed validation",
        "previous response",
        "return a corrected response",
        "required json schema",
        "translation model",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn convert_target_script(text: &str, target_language: &str) -> Result<String, String> {
    let map_flags = match target_language {
        "zh-Hant" => windows::Win32::Globalization::LCMAP_TRADITIONAL_CHINESE,
        "zh-Hans" => windows::Win32::Globalization::LCMAP_SIMPLIFIED_CHINESE,
        _ => return Ok(text.to_string()),
    };

    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::LPARAM;
        use windows::Win32::Globalization::LCMapStringEx;

        let locale_name: Vec<u16> = if target_language == "zh-Hant" {
            "zh-TW\0".encode_utf16().collect()
        } else {
            "zh-CN\0".encode_utf16().collect()
        };
        let source: Vec<u16> = text.encode_utf16().collect();
        let locale = PCWSTR(locale_name.as_ptr());
        let mapped_length = unsafe {
            LCMapStringEx(
                locale,
                map_flags,
                &source,
                None,
                None,
                None,
                LPARAM(0),
            )
        };
        if mapped_length <= 0 {
            return Err(format!(
                "Failed to normalize Chinese text to the requested script: {}",
                std::io::Error::last_os_error()
            ));
        }

        let mut destination = vec![0u16; mapped_length as usize];
        let written = unsafe {
            LCMapStringEx(
                locale,
                map_flags,
                &source,
                Some(&mut destination),
                None,
                None,
                LPARAM(0),
            )
        };
        if written <= 0 {
            return Err(format!(
                "Failed to normalize Chinese text to the requested script: {}",
                std::io::Error::last_os_error()
            ));
        }

        String::from_utf16(&destination[..written as usize])
            .map_err(|err| format!("Failed to decode normalized Chinese text: {err}"))
    }

    #[cfg(not(windows))]
    {
        let _ = map_flags;
        Ok(text.to_string())
    }
}

fn same_word(candidate: &str, original: &str) -> bool {
    let normalize = |text: &str| {
        text.chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };

    normalize(candidate) == normalize(original)
}

fn contains_target_form(sentence: &str, original: &str) -> bool {
    let target: Vec<String> = lexical_tokens(original);
    if target.is_empty() {
        return false;
    }

    if target.iter().any(|token| token.chars().any(is_cjk_character)) {
        return sentence.to_lowercase().contains(&original.to_lowercase());
    }

    let sentence_tokens = lexical_tokens(sentence);
    sentence_tokens.windows(target.len()).any(|window| {
        window
            .iter()
            .zip(&target)
            .enumerate()
            .all(|(index, (word, base))| {
                if index + 1 == target.len() {
                    is_word_form(word, base)
                } else {
                    word == base
                }
            })
    })
}

fn lexical_tokens(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric() && character != '\'')
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn is_word_form(word: &str, base: &str) -> bool {
    if word == base
        || ["s", "es", "ed", "d", "ing", "er", "ers", "est"]
            .iter()
            .any(|suffix| word == format!("{base}{suffix}"))
    {
        return true;
    }

    if let Some(stem) = base.strip_suffix('e') {
        if word == format!("{stem}d")
            || word == format!("{stem}ing")
            || word == format!("{stem}r")
            || word == format!("{stem}rs")
        {
            return true;
        }
    }

    if let Some(stem) = base.strip_suffix('y') {
        if word == format!("{stem}ies") || word == format!("{stem}ied") {
            return true;
        }
    }

    false
}

fn is_cjk_character(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F
    )
}

fn contains_chinese(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            character as u32,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F
        )
    })
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_sentence_case(text: &str) -> String {
    let sentence = single_line(text);
    let words: Vec<&str> = sentence.split_whitespace().collect();
    let cased_words: Vec<&str> = words
        .iter()
        .copied()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .collect();
    let title_case_count = cased_words
        .iter()
        .filter(|word| is_title_case_word(word))
        .count();

    if cased_words.len() < 3 || title_case_count * 2 < cased_words.len() {
        return sentence;
    }

    words
        .iter()
        .enumerate()
        .map(|(index, word)| {
            if index == 0 || is_all_caps_word(word) || *word == "I" {
                (*word).to_string()
            } else {
                word.to_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_title_case_word(word: &str) -> bool {
    let mut characters = word.chars().filter(|character| character.is_alphabetic());
    characters
        .next()
        .is_some_and(char::is_uppercase)
        && characters.all(char::is_lowercase)
}

fn is_all_caps_word(word: &str) -> bool {
    let characters: Vec<char> = word
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect();
    !characters.is_empty() && characters.iter().all(|character| character.is_uppercase())
}

fn ensure_sentence_punctuation(text: &str, language: &str) -> String {
    let sentence = single_line(text);
    if sentence
        .chars()
        .last()
        .is_some_and(|character| matches!(character, '.' | '?' | '!' | '。' | '？' | '！' | '…'))
    {
        return sentence;
    }

    let punctuation = if matches!(language, "zh-Hant" | "zh-Hans") && contains_chinese(&sentence)
        || matches!(language, "ja" | "ko")
    {
        "。"
    } else {
        "."
    };
    format!("{sentence}{punctuation}")
}

fn field_labels(language: &str) -> (&'static str, &'static str, &'static str) {
    match language {
        "zh-Hans" => ("词意：", "例句：", "短语动词："),
        "en" => ("Meaning:", "Examples:", "Phrasal verbs:"),
        "ja" => ("語義：", "例文：", "句動詞："),
        "ko" => ("뜻:", "예문:", "구동사:"),
        _ => ("詞意:", "例句:", "片語動詞:"),
    }
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

#[derive(Debug, Deserialize)]
struct ExampleResponse {
    examples: Vec<DictionaryExample>,
}

fn response_content(api_response: OpenAiResponse) -> Result<String, String> {
    api_response
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
        .ok_or_else(|| "Translation model returned no translation.".to_string())
}

fn parse_fallback_example(text: &str, original: &str) -> Result<Option<DictionaryExample>, String> {
    let response: ExampleResponse = serde_json::from_str(text)
        .map_err(|err| format!("Failed to parse fallback example response: {err}"))?;

    Ok(response.examples.into_iter().find(|example| {
        !example.source.trim().is_empty()
            && !example.translated.trim().is_empty()
            && contains_target_form(&example.source, original)
            && !contains_prompt_artifact(&example.source)
            && !contains_prompt_artifact(&example.translated)
    }))
}

pub(crate) fn is_diagnostic_context(context: &str) -> bool {
    let context = context.trim();
    let normalized = context.to_ascii_lowercase();
    let diagnostic_markers = [
        "[quicktranslate]",
        "ocr selected",
        "confidence",
        "cargo ",
        "watching ",
        "for changes",
        "c:\\users\\",
        "sre-tauri",
        "debuginfo",
        "finished 'dev' profile",
        "compiling ",
        "warning:",
        "error:",
        "--",
    ];

    diagnostic_markers
        .iter()
        .any(|marker| normalized.contains(marker))
}

fn usable_dictionary_context(context: &str) -> &str {
    if is_diagnostic_context(context) {
        ""
    } else {
        context.trim()
    }
}

fn request_chat_completion(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    max_tokens: u32,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    let response = client
        .post(format!(
            "{}/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "temperature": 0,
            "response_format": { "type": "json_object" },
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_prompt }
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
    response_content(api_response)
}

fn request_fallback_example(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    original: &str,
    context: &str,
    source_language: &str,
    target_language: &str,
) -> Result<Option<DictionaryExample>, String> {
    let source = language_name(source_language);
    let target = language_name(target_language);
    let system_prompt = format!(
        "Return exactly one valid JSON object with an examples array containing one natural, complete example in {source} and its translation in {target}: {{\"examples\":[{{\"source\":\"sentence\",\"translated\":\"translation\"}}]}}. The source sentence MUST use the exact target word or a valid inflected form. Do not copy the context, use an unrelated name or acronym, or return a fragment. Capitalize normally and include punctuation."
    );
    let user_prompt = format!("Target word: {original}\nContext for meaning only: {context}");

    let mut last_error = None;
    for attempt in 0..2 {
        let retry_prompt = if attempt == 0 {
            user_prompt.clone()
        } else {
            format!(
                "{user_prompt}\nYour previous response did not contain a valid complete sentence using the target word. Try again and check that the source sentence contains '{original}'."
            )
        };
        let content = request_chat_completion(
            client,
            base_url,
            api_key,
            model,
            160,
            &system_prompt,
            &retry_prompt,
        )?;
        match parse_fallback_example(&content, original) {
            Ok(Some(example)) => return Ok(Some(example)),
            Ok(None) => {
                last_error = Some(format!(
                    "Fallback example did not contain a valid sentence using '{original}'."
                ));
            }
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| "Fallback example generation failed.".to_string()))
}

fn build_translation_prompts(
    text: &str,
    context: &str,
    source_language: &str,
    target_language: &str,
) -> (String, String) {
    let source = language_name(source_language);
    let target = language_name(target_language);
    let context = usable_dictionary_context(context);
    let user_prompt = if context.is_empty() {
        format!("Target word: {text}")
    } else {
        format!("Context: {context}\nTarget word: {text}")
    };

    let system_prompt = format!(
        "You are a careful bilingual dictionary translator. Return data for the exact Target word from {source} into {target}; do not format the final entry. Use context only to choose relevant meanings. Context may contain OCR mistakes or terminal/compiler output; ignore it when it looks like logs, commands, flags, or diagnostics, and never replace the Target word. Do not provide pronunciation or phonetic transcription. Give the part of speech (including singular/plural where relevant). Include every useful, distinct common meaning with no fixed count; do not pad with rare, duplicate, or invented meanings. For each meaning, provide several concise, natural {target} equivalent terms and one clear explanatory sentence in {target}. Do not provide English definitions or put English in the meaning fields. For Chinese targets, use only the requested Chinese writing system in all meanings and example translations: Traditional Chinese for zh-Hant, Simplified Chinese for zh-Hans; never mix the other script into those fields. Do not number the Chinese equivalent terms; the application displays each meaning without a numeric prefix. For example, for 'condemn', return terms '譴責、指責', followed by '通常指出於道義而嚴厲批評某人或某事。' A meaning is not valid if it merely repeats the Target word; for 'training', provide terms such as '訓練、培訓' and a sentence explaining the process of learning skills for a job or activity. Give one or more natural, complete grammatical example sentences in {source}; every source example MUST contain the exact Target word or a valid inflected form of that word. Do not invent acronyms, abbreviations, named people, or unrelated entities in examples; never use an unexplained all-caps string such as 'RBBB'. Do not copy isolated labels or fragments from context as examples. Pair every example with its translation in {target}. Use normal sentence case: capitalize the first word and proper nouns only, never capitalize every word as if it were a title. Preserve correct spelling and include appropriate punctuation, including terminal punctuation. Only include phrasal verbs when the target word genuinely has common phrasal verbs; otherwise return an empty array. Do not include tone/connotation or lemma/base-form fields. Do not invent data. Respond with exactly one valid JSON object, no markdown or extra text, using exactly this schema and field names: {{\"part_of_speech\":\"string without parentheses\",\"meanings\":[{{\"terms\":[\"concise equivalent term in {target}\"],\"explanation\":\"one-sentence explanation in {target}\"}}],\"examples\":[{{\"source\":\"complete grammatical example in normal sentence case containing the Target word, with punctuation\",\"translated\":\"translation in {target}, with punctuation\"}}],\"phrasal_verbs\":[\"string\"]}}. Every key is required; phrasal_verbs may be empty when none exist."
    );

    (system_prompt, user_prompt)
}

fn build_correction_prompts(
    text: &str,
    context: &str,
    source_language: &str,
    target_language: &str,
) -> (String, String) {
    let source = language_name(source_language);
    let target = language_name(target_language);
    let context = usable_dictionary_context(context);
    let system_prompt = format!(
        "You are a bilingual dictionary. Define the exact target word, not the surrounding text. Write part_of_speech in {source}. Write every meaning term and explanation only in {target}; do not put English in those fields. Return one JSON object with exactly these fields: {{\"part_of_speech\":\"noun\",\"meanings\":[{{\"terms\":[\"equivalent term\"],\"explanation\":\"one clear sentence explaining this meaning\"}}],\"examples\":[{{\"source\":\"complete sentence containing the target word\",\"translated\":\"sentence in {target}\"}}],\"phrasal_verbs\":[]}}. Include common meanings only. Examples must be complete {source} sentences containing the exact target word or a valid inflection, paired with a {target} translation. Use an empty phrasal_verbs array when there are none. No markdown or extra text."
    );
    let user_prompt = if context.is_empty() {
        format!("Target word: {text}")
    } else {
        format!("Target word: {text}\nContext for meaning only: {context}")
    };

    (system_prompt, user_prompt)
}

#[cfg(test)]
fn format_translation_output(
    text: &str,
    original: &str,
    target_language: &str,
) -> Result<String, String> {
    let fields = DictionaryEntryFields::from_model_output(text, original, target_language)?;
    fields.format(original, target_language)
}

pub fn translate_text(
    text: &str,
    source_language: &str,
    target_language: &str,
) -> Result<TranslationResult, String> {
    translate_text_with_context(text, "", source_language, target_language)
}

pub fn translate_text_with_context(
    text: &str,
    context: &str,
    source_language: &str,
    target_language: &str,
) -> Result<TranslationResult, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("No text to translate.".to_string());
    }
    let context = usable_dictionary_context(context);

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

    let mut api_key_hasher = std::collections::hash_map::DefaultHasher::new();
    api_key.hash(&mut api_key_hasher);
    let cache_key = TranslationCacheKey {
        base_url: base_url.clone(),
        model: model_name.clone(),
        api_key_fingerprint: api_key_hasher.finish(),
        source_language: source_language.to_string(),
        target_language: target_language.to_string(),
        text: trimmed.to_string(),
        context: context.to_string(),
    };
    let cache = TRANSLATION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(result) = cache
        .lock()
        .map_err(|_| "Translation cache is unavailable.".to_string())?
        .get(&cache_key)
        .cloned()
    {
        return Ok(result);
    }

    let (system_prompt, user_prompt) =
        build_translation_prompts(trimmed, context, source_language, target_language);

    let client = HTTP_CLIENT.get_or_init(reqwest::blocking::Client::new);
    let translated = request_chat_completion(
        client,
        &base_url,
        &api_key,
        &model_name,
        768,
        &system_prompt,
        &user_prompt,
    )?;
    let mut fields = match DictionaryEntryFields::from_model_output(
        &translated,
        trimmed,
        target_language,
    ) {
        Ok(fields) => fields,
        Err(first_error) => {
            eprintln!(
                "[QuickTranslate] translation response failed validation: {first_error}; retrying with a concise schema prompt"
            );
            let (correction_system_prompt, correction_user_prompt) =
                build_correction_prompts(trimmed, context, source_language, target_language);
            let retry = request_chat_completion(
                client,
                &base_url,
                &api_key,
                &model_name,
                512,
                &correction_system_prompt,
                &correction_user_prompt,
            )
            .map_err(|retry_error| {
                format!("{first_error}; correction request failed: {retry_error}")
            })?;
            DictionaryEntryFields::from_model_output(&retry, trimmed, target_language).map_err(
                |retry_error| format!("{first_error}; correction response failed: {retry_error}"),
            )?
        }
    };
    if fields.examples.is_empty() {
        match request_fallback_example(
            client,
            &base_url,
            &api_key,
            &model_name,
            trimmed,
            context.trim(),
            source_language,
            target_language,
        ) {
            Ok(Some(example)) => fields.examples.push(example),
            Ok(None) => eprintln!(
                "[QuickTranslate] model could not provide a valid example for '{trimmed}'"
            ),
            Err(err) => eprintln!("[QuickTranslate] fallback example failed: {err}"),
        }
    }
    let translated = fields.format(trimmed, target_language)?;
    if translated.is_empty() {
        return Err("Translation model returned an empty translation.".to_string());
    }

    let result = TranslationResult {
        original: trimmed.to_string(),
        translated,
        source_language: source_language.to_string(),
        target_language: target_language.to_string(),
    };

    let mut cache = cache
        .lock()
        .map_err(|_| "Translation cache is unavailable.".to_string())?;
    if cache.len() >= 256 {
        cache.clear();
    }
    cache.insert(cache_key, result.clone());

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{
        build_correction_prompts, build_translation_prompts, format_translation_output,
        is_diagnostic_context, normalize_sentence_case, parse_fallback_example, translate_text,
        usable_dictionary_context,
    };

    #[test]
    fn auto_source_asks_model_to_detect_language() {
        let (system_prompt, user_prompt) =
            build_translation_prompts("bonjour", "", "auto", "zh-Hant");

        assert!(system_prompt.contains("detected source language"));
        assert!(system_prompt.contains("Traditional Chinese"));
        assert!(system_prompt.contains("exactly one valid JSON object"));
        assert!(!system_prompt.contains("\"pronunciation\""));
        assert!(system_prompt.contains("\"meanings\""));
        assert!(system_prompt.contains("\"examples\""));
        assert!(system_prompt.contains("\"phrasal_verbs\""));
        assert!(system_prompt.contains("no fixed count"));
        assert!(system_prompt.contains("Do not provide pronunciation or phonetic transcription"));
        assert!(system_prompt.contains("Do not number the Chinese equivalent terms"));
        assert!(system_prompt.contains("never capitalize every word as if it were a title"));
        assert!(system_prompt.contains("A meaning is not valid if it merely repeats the Target word"));
        assert!(system_prompt.contains("for 'training'"));
        assert!(system_prompt.contains("several concise, natural Traditional Chinese equivalent terms"));
        assert!(system_prompt.contains("one clear explanatory sentence in Traditional Chinese"));
        assert!(system_prompt.contains("Do not provide English definitions"));
        assert!(system_prompt.contains("otherwise return an empty array"));
        assert!(system_prompt.contains("Do not include tone/connotation or lemma"));
        assert!(system_prompt.contains("Do not invent"));
        assert!(system_prompt.contains("terminal/compiler output"));
        assert_eq!(user_prompt, "Target word: bonjour");
    }

    #[test]
    fn ignores_terminal_diagnostics_when_building_translation_context() {
        assert!(is_diagnostic_context(
            "OCR selected 'GOSSIP' (confidence 96.3); context: GOSSIP x flea"
        ));
        assert!(is_diagnostic_context(
            "Info Watching C:\\Users\\ethan\\Downloads\\QuickTranslate\\src-tauri for changes..."
        ));
        assert!(is_diagnostic_context(
            "use the option '--future-incompat-report', or run cargo check"
        ));
        assert!(!is_diagnostic_context("The future looks bright."));
        assert_eq!(
            usable_dictionary_context(
                "[QuickTranslate] OCR selected 'future'; context: '--future-incompat-report'"
            ),
            ""
        );
        assert_eq!(
            usable_dictionary_context("The future looks bright."),
            "The future looks bright."
        );
        let (_, user_prompt) = build_translation_prompts(
            "Downloads",
            "Info Watching C:\\Users\\ethan\\Downloads\\QuickTranslate\\src-tauri for changes...",
            "en",
            "zh-Hant",
        );
        assert_eq!(user_prompt, "Target word: Downloads");
    }

    #[test]
    fn correction_prompt_uses_a_short_schema_and_preserves_target_word() {
        let (system_prompt, user_prompt) =
            build_correction_prompts("Friends", "", "en", "zh-Hant");

        assert!(system_prompt.contains("every meaning term and explanation only in Traditional Chinese"));
        assert!(system_prompt.contains("\"meanings\""));
        assert!(system_prompt.contains("\"examples\""));
        assert!(!system_prompt.contains("did not provide Chinese terms"));
        assert_eq!(user_prompt, "Target word: Friends");
    }

    #[test]
    fn correction_prompt_drops_noisy_context() {
        let (_, user_prompt) = build_correction_prompts(
            "Downloads",
            "Info Watching C:\\Users\\ethan\\Downloads\\QuickTranslate\\src-tauri for changes...",
            "en",
            "zh-Hant",
        );

        assert_eq!(user_prompt, "Target word: Downloads");
    }

    #[test]
    fn formats_chinese_terms_and_sentence_explanation_with_multiple_examples() {
        let json = r#"{
            "part_of_speech": "verb",
            "meanings": [
                {"terms": ["譴責", "指責"], "explanation": "通常指出於道義而嚴厲批評某人或某事。"}
            ],
            "examples": [
                {"source": "The terrorist action has been condemned as an act of barbarism and cowardice.", "translated": "人們譴責這一恐怖行徑是野蠻和懦弱的行爲。"},
                {"source": "The film was condemned for its sexism.", "translated": "這部電影因其性別歧視而受到譴責。"}
            ],
            "phrasal_verbs": []
        }"#;

        assert_eq!(
            format_translation_output(json, "condemn", "zh-Hant").unwrap(),
            "condemn (verb)\n詞意:\n譴責、指責\n通常指出於道義而嚴厲批評某人或某事。\n例句:\nThe terrorist action has been condemned as an act of barbarism and cowardice.\n人們譴責這一恐怖行徑是野蠻和懦弱的行爲。\nThe film was condemned for its sexism.\n這部電影因其性別歧視而受到譴責。"
        );
    }

    #[test]
    fn skips_examples_that_do_not_contain_the_target_word_without_failing_translation() {
        let json = r#"{
            "part_of_speech": "noun",
            "meanings": [{
                "terms": ["流言", "閒話"],
                "explanation": "未經證實而在人群中流傳的消息。"
            }],
            "examples": [{
                "source": "RBBB always spread rumors among the staff.",
                "translated": "RBBB總是在員工之間散播謠言。"
            }],
            "phrasal_verbs": []
        }"#;

        let output = format_translation_output(json, "gossip", "zh-Hant").unwrap();

        assert!(output.contains("流言、閒話"));
        assert!(!output.contains("RBBB"));
        assert!(!output.contains("例句:"));
    }

    #[test]
    fn fallback_example_accepts_a_sentence_that_uses_the_target_word() {
        let response = r#"{"examples":[{"source":"People often gossip about celebrities.","translated":"人們常常談論名人的八卦。"}]}"#;

        let example = parse_fallback_example(response, "gossip")
            .unwrap()
            .unwrap();

        assert_eq!(example.source, "People often gossip about celebrities.");
        assert_eq!(example.translated, "人們常常談論名人的八卦。");
    }

    #[test]
    fn fallback_example_rejects_unrelated_or_missing_examples() {
        let response =
            r#"{"examples":[{"source":"RBBB always spread rumors among the staff.","translated":"員工之間散播謠言。"}]}"#;

        assert!(parse_fallback_example(response, "gossip")
            .unwrap()
            .is_none());
        assert!(parse_fallback_example(r#"{"examples":[]}"#, "gossip")
            .unwrap()
            .is_none());
    }

    #[test]
    fn fallback_example_rejects_copied_validation_text() {
        let response = r#"{"examples":[{"source":"It did not provide Chinese terms and an explanation.","translated":"未提供中文詞語及解釋。"}]}"#;

        assert!(parse_fallback_example(response, "Chinese")
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_dictionary_meanings_that_copy_validation_text() {
        let json = r#"{
            "part_of_speech": "verb",
            "meanings": [{
                "terms": ["忽略"],
                "explanation": "The model did not provide Chinese terms and an explanation."
            }],
            "examples": [],
            "phrasal_verbs": []
        }"#;

        assert!(format_translation_output(json, "Chinese", "zh-Hant")
            .unwrap_err()
            .contains("copied prompt or validation text"));
    }

    #[test]
    fn accepts_target_word_inflection_in_example() {
        let json = r#"{
            "part_of_speech": "verb",
            "meanings": [{
                "terms": ["譴責"],
                "explanation": "嚴厲指出某事或某人的錯誤。"
            }],
            "examples": [{
                "source": "The film was condemned for its sexism.",
                "translated": "這部電影因其性別歧視而受到譴責。"
            }],
            "phrasal_verbs": []
        }"#;

        assert!(format_translation_output(json, "condemn", "zh-Hant").is_ok());
    }

    #[test]
    fn includes_phrasal_verb_section_only_when_returned() {
        let json = r#"{
            "part_of_speech": "noun",
            "meanings": [
                {"terms": ["煽情主義", "聳人聽聞"], "explanation": "透過誇大或刺激性的方式呈現新聞，以引起大眾注意。"}
            ],
            "examples": [
                {"source": "The newspaper has been accused of sensationalism in its coverage of the murders.", "translated": "報紙被指責在報道凶殺案時故意聳人聽聞。"}
            ],
            "phrasal_verbs": []
        }"#;
        let output = format_translation_output(json, "sensationalism", "zh-Hant").unwrap();

        assert!(output.starts_with("sensationalism (noun)\n詞意:\n"));
        assert!(!output.contains('/'));
        assert!(!output.contains("片語動詞:"));
        assert!(!output.contains("語氣"));
        assert!(!output.contains("原形"));
    }

    #[test]
    fn renders_phrasal_verbs_when_present() {
        let json = r#"{
            "part_of_speech": "verb",
            "meanings": [{
                "terms": ["譴責"],
                "explanation": "嚴厲指出某事或某人的錯誤。"
            }],
            "examples": [{"source": "They condemned the decision.", "translated": "他們譴責這項決定。"}],
            "phrasal_verbs": ["condemn someone to (do) something", "condemn someone to something"]
        }"#;

        let output = format_translation_output(json, "condemn", "zh-Hant").unwrap();

        assert!(output.ends_with(
            "片語動詞:\ncondemn someone to (do) something\ncondemn someone to something"
        ));
    }

    #[test]
    fn includes_context_without_changing_the_target_word() {
        let (system_prompt, user_prompt) =
            build_translation_prompts("terminal", "Open the terminal now", "en", "zh-Hant");

        assert!(system_prompt.contains("never replace the Target word"));
        assert_eq!(
            user_prompt,
            "Context: Open the terminal now\nTarget word: terminal"
        );
    }

    #[test]
    fn prompt_does_not_request_pronunciation() {
        let (system_prompt, user_prompt) = build_translation_prompts(
            "between",
            "bond that's between us two",
            "en",
            "zh-Hant",
        );

        assert!(system_prompt.contains("Do not provide pronunciation or phonetic transcription"));
        assert!(!system_prompt.contains("KK"));
        assert_eq!(
            user_prompt,
            "Context: bond that's between us two\nTarget word: between"
        );
    }

    #[test]
    fn rejects_invalid_or_incomplete_structured_output() {
        assert!(format_translation_output("not json", "word", "zh-Hant").is_err());
        assert!(format_translation_output(
            r#"{"part_of_speech":"noun","meanings":[],"examples":[{"source":"An example.","translated":"例句。"}],"phrasal_verbs":[]}"#,
            "word",
            "zh-Hant"
        )
        .is_err());
    }

    #[test]
    fn rejects_a_meaning_that_only_repeats_the_target_word() {
        let json = r#"{
            "part_of_speech": "noun",
            "meanings": [{"terms": ["training"], "explanation": "training"}],
            "examples": [{"source": "Training starts today.", "translated": "今天開始訓練。"}],
            "phrasal_verbs": []
        }"#;

        let result = format_translation_output(json, "training", "zh-Hant");

        assert_eq!(
            result.unwrap_err(),
            "Translation model repeated the target word instead of defining its meaning."
        );
    }

    #[test]
    fn accepts_an_explanatory_definition_for_the_target_word() {
        let json = r#"{
            "part_of_speech": "noun",
            "meanings": [{
                "terms": ["訓練", "培訓"],
                "explanation": "學習從事特定工作或活動所需技能的過程。"
            }],
            "examples": [{"source": "Training starts today.", "translated": "今天開始訓練。"}],
            "phrasal_verbs": []
        }"#;

        let result = format_translation_output(json, "training", "zh-Hant").unwrap();

        assert!(result.contains("訓練、培訓"));
        assert!(result.contains("學習從事特定工作"));
    }

    #[test]
    fn requires_chinese_terms_and_explanation_for_each_meaning() {
        let json = r#"{
            "part_of_speech": "noun",
            "meanings": [{"terms": ["training"], "explanation": "training"}],
            "examples": [{"source": "Training starts today.", "translated": "今天開始訓練。"}],
            "phrasal_verbs": []
        }"#;

        assert!(format_translation_output(json, "training", "zh-Hant").is_err());
    }

    #[test]
    fn formats_chinese_terms_then_sentence_explanation_on_next_line() {
        let json = r#"{
            "part_of_speech": "noun",
            "meanings": [{
                "terms": ["訓練", "培訓"],
                "explanation": "學習從事特定工作或活動所需技能的過程。"
            }],
            "examples": [{"source": "Training starts today.", "translated": "今天開始訓練。"}],
            "phrasal_verbs": []
        }"#;

        let output = format_translation_output(json, "training", "zh-Hant").unwrap();
        let lines: Vec<&str> = output.lines().collect();
        let meaning_index = lines.iter().position(|line| *line == "詞意:").unwrap();

        assert_eq!(lines[meaning_index + 1], "訓練、培訓");
        assert_eq!(lines[meaning_index + 2], "學習從事特定工作或活動所需技能的過程。");
    }

    #[test]
    fn normalizes_simplified_model_text_to_traditional_chinese() {
        let json = r#"{
            "part_of_speech": "名词",
            "meanings": [{
                "terms": ["训练", "培训"],
                "explanation": "这是一种学习技能的过程。"
            }],
            "examples": [{"source": "Training helps.", "translated": "这项训练很有效"}],
            "phrasal_verbs": []
        }"#;

        let output = format_translation_output(json, "training", "zh-Hant").unwrap();

        assert!(output.starts_with("training (名詞)\n"));
        assert!(output.contains("訓練、培訓\n這是一種學習技能的過程。"));
        assert!(output.contains("這項訓練很有效。"));
        assert!(!output.contains("训练"));
        assert!(!output.contains("培训"));
        assert!(!output.contains("名词"));
        assert!(!output.contains("这"));
    }

    #[test]
    fn normalizes_traditional_model_text_to_simplified_chinese() {
        let json = r#"{
            "part_of_speech": "名詞",
            "meanings": [{
                "terms": ["訓練", "培訓"],
                "explanation": "這是一種學習技能的過程。"
            }],
            "examples": [{"source": "Training helps.", "translated": "這項訓練很有效。"}],
            "phrasal_verbs": []
        }"#;

        let output = format_translation_output(json, "training", "zh-Hans").unwrap();

        assert!(output.starts_with("training (名词)\n"));
        assert!(output.contains("训练、培训\n这是一种学习技能的过程。"));
        assert!(output.contains("这项训练很有效。"));
        assert!(!output.contains("訓練"));
        assert!(!output.contains("這"));
    }

    #[test]
    fn adds_sentence_punctuation_when_examples_omit_it() {
        let json = r#"{
            "part_of_speech": "noun",
            "meanings": [{
                "terms": ["訓練"],
                "explanation": "學習技能的過程。"
            }],
            "examples": [{"source": "Training starts today", "translated": "今天開始訓練"}],
            "phrasal_verbs": []
        }"#;

        let output = format_translation_output(json, "training", "zh-Hant").unwrap();

        assert!(output.contains("Training starts today.\n今天開始訓練。"));
    }

    #[test]
    fn normalizes_title_case_examples_to_sentence_case() {
        assert_eq!(
            normalize_sentence_case("Primary Tier Required Augment Setup"),
            "Primary tier required augment setup"
        );
        assert_eq!(
            normalize_sentence_case("The NASA Team Uses Augment"),
            "The NASA team uses augment"
        );
        assert_eq!(
            normalize_sentence_case("The Team Requires Augment"),
            "The team requires augment"
        );
    }

    #[test]
    fn rejects_empty_input() {
        let result = translate_text("   ", "en", "zh-Hant");
        assert!(result.is_err());
    }
}
