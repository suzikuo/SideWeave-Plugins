//! Text translation providers used by the screenshot result window.
//!
//! Google public translate_a/single client used by the screenshot result window.

use reqwest::blocking::Client;
use reqwest::Url;
use serde_json::Value;
use std::time::Duration;

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CHUNK_CHAR_LIMIT: usize = 350;

pub fn translate_to_chinese(text: &str) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("没有可翻译的识别结果".into());
    }

    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("无法创建 Google 翻译连接：{error}"))?;
    let mut translated = String::new();

    for chunk in split_chunks(text, CHUNK_CHAR_LIMIT) {
        let mut url = Url::parse(ENDPOINT)
            .map_err(|error| format!("Google 翻译地址无效：{error}"))?;
        url.query_pairs_mut()
            .append_pair("client", "gtx")
            .append_pair("sl", "auto")
            .append_pair("dt", "t")
            .append_pair("tl", "zh-CN")
            .append_pair("q", &chunk);
        let response = client
            .get(url)
            .send()
            .map_err(|_| "Google 翻译请求失败，请检查网络连接".to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("Google 翻译失败（HTTP {status}）"));
        }
        let body: Value = response
            .json()
            .map_err(|error| format!("Google 翻译响应无法解析：{error}"))?;
        translated.push_str(&extract_google_translation(&body)?);
    }
    Ok(translated)
}

fn extract_google_translation(value: &Value) -> Result<String, String> {
    let parts = value
        .as_array()
        .and_then(|items| items.first())
        .and_then(Value::as_array)
        .ok_or_else(|| "Google 翻译没有返回译文".to_string())?;
    let texts: Vec<&str> = parts
        .iter()
        .filter_map(|item| item.as_array()?.first()?.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect();
    if texts.is_empty() {
        return Err("Google 翻译返回了空译文".into());
    }
    Ok(texts.join("\n"))
}

fn split_chunks(text: &str, limit: usize) -> Vec<String> {
    let mut remaining: Vec<char> = text.chars().collect();
    let mut chunks = Vec::new();
    while !remaining.is_empty() {
        let split_at = if remaining.len() <= limit {
            remaining.len()
        } else {
            let boundary = remaining[..limit]
                .iter()
                .rposition(|character| {
                    matches!(character, '\n' | '。' | '！' | '？' | '.' | '!' | '?' | '；' | ';' | ' ')
                })
                .map(|index| index + 1)
                .filter(|index| *index >= limit * 2 / 3)
                .unwrap_or(limit);
            boundary
        };
        chunks.push(remaining[..split_at].iter().collect());
        remaining.drain(..split_at);
    }
    chunks
}

