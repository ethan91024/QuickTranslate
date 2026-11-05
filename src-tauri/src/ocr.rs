use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use image::ImageEncoder;
use serde::{Deserialize, Serialize};

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn resolve_tesseract_path() -> Result<PathBuf, String> {
    let candidates = [
        PathBuf::from(r"C:\Program Files\Tesseract-OCR\tesseract.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Tesseract-OCR\tesseract.exe"),
    ];

    for path in candidates {
        if path.exists() {
            return Ok(path);
        }
    }

    if let Ok(path_value) = std::env::var("PATH") {
        for entry in path_value.split(';') {
            let candidate = PathBuf::from(entry).join("tesseract.exe");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    Err("Tesseract OCR is not installed or not on PATH.".to_string())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OcrWord {
    pub text: String,
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub confidence: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OcrResult {
    pub raw_text: String,
    pub words: Vec<OcrWord>,
}

pub fn recognize(image: &image::RgbaImage) -> Result<OcrResult, String> {
    let tesseract_path = resolve_tesseract_path()?;

    let mut child = Command::new(&tesseract_path)
        .creation_flags(CREATE_NO_WINDOW)
        .arg("stdin")
        .arg("stdout")
        .arg("--psm")
        .arg("11")
        .arg("tsv")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("Failed to run Tesseract: {err}"))?;

    let mut stdin = child.stdin.take().ok_or_else(|| "Failed to open Tesseract stdin.".to_string())?;
    let mut png_bytes: Vec<u8> = Vec::new();
    {
        let mut cursor = std::io::Cursor::new(&mut png_bytes);
        image::codecs::png::PngEncoder::new(&mut cursor)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ColorType::Rgba8.into(),
            )
            .map_err(|err| format!("Failed to encode image for OCR: {err}"))?;
    }
    stdin
        .write_all(&png_bytes)
        .map_err(|err| format!("Failed to write image to Tesseract stdin: {err}"))?;
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|err| format!("Failed to run Tesseract on image buffer: {err}"))?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Tesseract failed: {error}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut words: Vec<OcrWord> = Vec::new();
    let mut lines = stdout.lines();
    let _header = lines.next();

    for row in lines {
        let fields: Vec<&str> = row.split('\t').collect();
        if fields.len() < 12 {
            continue;
        }

        let text = fields[11].trim();
        if text.is_empty() {
            continue;
        }

        let left = fields[6].parse::<i32>().unwrap_or_default();
        let top = fields[7].parse::<i32>().unwrap_or_default();
        let width = fields[8].parse::<i32>().unwrap_or_default();
        let height = fields[9].parse::<i32>().unwrap_or_default();
        let confidence = fields[10].parse::<f32>().unwrap_or(-1.0);
        if confidence < 0.0 {
            continue;
        }

        words.push(OcrWord {
            text: text.to_string(),
            left,
            top,
            width,
            height,
            confidence,
        });
    }

    let raw_text = words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    Ok(OcrResult { raw_text, words })
}

pub fn recognize_from_file(image_path: &Path) -> Result<OcrResult, String> {
    let tesseract_path = resolve_tesseract_path()?;

    if !Command::new(&tesseract_path)
        .creation_flags(CREATE_NO_WINDOW)
        .arg("--version")
        .output()
        .map_err(|err| format!("Failed to check Tesseract: {err}"))?
        .status
        .success()
    {
        return Err("Tesseract OCR is not available.".to_string());
    }

    let output = Command::new(&tesseract_path)
        .creation_flags(CREATE_NO_WINDOW)
        .arg(image_path)
        .arg("stdout")
        .arg("--psm")
        .arg("11")
        .arg("tsv")
        .output()
        .map_err(|err| format!("Failed to run Tesseract: {err}"))?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Tesseract failed: {error}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut words: Vec<OcrWord> = Vec::new();
    let mut lines = stdout.lines();
    let _header = lines.next();

    for row in lines {
        let fields: Vec<&str> = row.split('\t').collect();
        if fields.len() < 12 {
            continue;
        }

        let text = fields[11].trim();
        if text.is_empty() {
            continue;
        }

        let left = fields[6].parse::<i32>().unwrap_or_default();
        let top = fields[7].parse::<i32>().unwrap_or_default();
        let width = fields[8].parse::<i32>().unwrap_or_default();
        let height = fields[9].parse::<i32>().unwrap_or_default();
        let confidence = fields[10].parse::<f32>().unwrap_or(-1.0);
        if confidence < 0.0 {
            continue;
        }

        words.push(OcrWord {
            text: text.to_string(),
            left,
            top,
            width,
            height,
            confidence,
        });
    }

    let raw_text = words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    Ok(OcrResult { raw_text, words })
}

const MIN_WORD_CONFIDENCE: f32 = 25.0;

pub fn word_index_under_cursor(words: &[OcrWord], cursor_x: i32, cursor_y: i32) -> Option<usize> {
    words
        .iter()
        .enumerate()
        .filter(|(_, word)| word.confidence >= MIN_WORD_CONFIDENCE)
        .filter(|(_, word)| {
            let left = word.left;
            let top = word.top;
            let right = word.left + word.width;
            let bottom = word.top + word.height;
            cursor_x >= left && cursor_x <= right && cursor_y >= top && cursor_y <= bottom
        })
        .max_by(|(_, first), (_, second)| first.confidence.total_cmp(&second.confidence))
        .map(|(index, _)| index)
}

pub fn find_word_under_cursor(words: &[OcrWord], cursor_x: i32, cursor_y: i32) -> Option<String> {
    let word = words.get(word_index_under_cursor(words, cursor_x, cursor_y)?)?;
    word_at_horizontal_position(&word.text, word.left, word.width, cursor_x)
}

pub fn context_for_word(words: &[OcrWord], selected_index: usize) -> Option<String> {
    let selected = words.get(selected_index)?;
    let selected_center_y = selected.top + selected.height / 2;
    let line_tolerance = (selected.height.max(1) as f32 * 0.75).max(8.0) as i32;

    let mut line_words: Vec<(usize, &OcrWord)> = words
        .iter()
        .enumerate()
        .filter(|(_, word)| {
            word.confidence >= MIN_WORD_CONFIDENCE
                && ((word.top + word.height / 2) - selected_center_y).abs() <= line_tolerance
        })
        .collect();
    line_words.sort_by_key(|(_, word)| word.left);

    let selected_position = line_words
        .iter()
        .position(|(index, _)| *index == selected_index)?;
    let start = selected_position.saturating_sub(3);
    let end = (selected_position + 4).min(line_words.len());

    Some(
        line_words[start..end]
            .iter()
            .map(|(_, word)| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn word_at_horizontal_position(text: &str, left: i32, width: i32, cursor_x: i32) -> Option<String> {
    let characters: Vec<char> = text.chars().collect();
    let mut ranges = Vec::new();
    let mut index = 0;

    while index < characters.len() {
        if !characters[index].is_alphanumeric() {
            index += 1;
            continue;
        }

        let start = index;
        while index < characters.len() && characters[index].is_alphanumeric() {
            index += 1;
        }
        ranges.push((start, index));
    }

    if ranges.is_empty() {
        return None;
    }
    if ranges.len() == 1 {
        return Some(characters[ranges[0].0..ranges[0].1].iter().collect());
    }

    let character_index = if width <= 0 {
        0
    } else {
        (((cursor_x - left).clamp(0, width) as f64 / width as f64) * characters.len() as f64)
            .floor() as usize
    }
    .min(characters.len().saturating_sub(1));

    let (start, end) = ranges
        .into_iter()
        .min_by_key(|(start, end)| {
            if character_index < *start {
                *start - character_index
            } else if character_index >= *end {
                character_index - (*end - 1)
            } else {
                0
            }
        })?;

    Some(characters[start..end].iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_word_under_cursor_from_punctuation_joined_ocr_text() {
        let words = [OcrWord {
            text: "busted,agent。breathe".to_string(),
            left: 0,
            top: 0,
            width: 180,
            height: 24,
            confidence: 90.0,
        }];

        assert_eq!(find_word_under_cursor(&words, 80, 12).as_deref(), Some("agent"));
    }

    #[test]
    fn removes_punctuation_from_the_selected_word() {
        let words = [OcrWord {
            text: "(agent),".to_string(),
            left: 0,
            top: 0,
            width: 70,
            height: 24,
            confidence: 90.0,
        }];

        assert_eq!(find_word_under_cursor(&words, 35, 12).as_deref(), Some("agent"));
    }

    #[test]
    fn ignores_ocr_tokens_that_only_contain_punctuation() {
        let words = [OcrWord {
            text: "...，".to_string(),
            left: 0,
            top: 0,
            width: 30,
            height: 24,
            confidence: 90.0,
        }];

        assert_eq!(find_word_under_cursor(&words, 15, 12), None);
    }

    #[test]
    fn does_not_select_a_nearby_word_when_cursor_is_outside_its_bounds() {
        let words = [OcrWord {
            text: "TERMINAL".to_string(),
            left: 100,
            top: 100,
            width: 100,
            height: 24,
            confidence: 90.0,
        }];

        assert_eq!(find_word_under_cursor(&words, 99, 112), None);
    }

    #[test]
    fn ignores_low_confidence_words() {
        let words = [OcrWord {
            text: "TERMINAL".to_string(),
            left: 100,
            top: 100,
            width: 100,
            height: 24,
            confidence: 20.0,
        }];

        assert_eq!(find_word_under_cursor(&words, 120, 112), None);
    }

    #[test]
    fn builds_context_from_nearby_words_on_the_same_line() {
        let words = ["Open", "the", "terminal", "now"]
            .iter()
            .enumerate()
            .map(|(index, text)| OcrWord {
                text: (*text).to_string(),
                left: index as i32 * 50,
                top: 10,
                width: 40,
                height: 20,
                confidence: 90.0,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            context_for_word(&words, 2).as_deref(),
            Some("Open the terminal now")
        );
    }
}
