//! Multi-engine translation service inspired by SnowAir's `FreeWebTranslate.cpp`:
//! supports Youdao, Microsoft (Bing), Google, and Baidu with automatic fallback.

use crate::model::OcrRegionBox;
use md5::{Digest, Md5};
use reqwest::blocking::Client;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CHUNK_CHAR_LIMIT: usize = 450;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranslateProvider {
    Youdao,
    Microsoft,
    Google,
    Baidu,
}

impl TranslateProvider {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "microsoft" | "bing" => Self::Microsoft,
            "google" => Self::Google,
            "baidu" => Self::Baidu,
            _ => Self::Youdao,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Youdao => "有道翻译",
            Self::Microsoft => "微软翻译",
            Self::Google => "谷歌翻译",
            Self::Baidu => "百度翻译",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateResponse {
    pub text: String,
    pub provider_used: String,
}

#[derive(Clone, Default)]
struct BingCreds {
    host: String,
    key: String,
    token: String,
    ig: String,
    iid: String,
}

static BING_CACHE: Mutex<Option<BingCreds>> = Mutex::new(None);

pub fn translate_to_chinese(text: &str) -> Result<String, String> {
    translate_text(text, "auto", "zh-CN", TranslateProvider::Youdao).map(|r| r.text)
}

pub fn translate_text(
    text: &str,
    source_lang: &str,
    target_lang: &str,
    preferred: TranslateProvider,
) -> Result<TranslateResponse, String> {
    if text.trim().is_empty() {
        return Err("没有可翻译的文本".into());
    }
    let sl = normalize_lang(source_lang);
    let tl = {
        let n = normalize_lang(target_lang);
        if n == "auto" { "zh-CN" } else { n }
    };

    let order = fallback_order(preferred);
    let mut last_err = String::new();
    for provider in order {
        let res = match provider {
            TranslateProvider::Youdao => translate_youdao(text, sl, tl),
            TranslateProvider::Microsoft => translate_microsoft(text, sl, tl),
            TranslateProvider::Google => translate_google(text, sl, tl),
            TranslateProvider::Baidu => translate_baidu(text, sl, tl),
        };
        match res {
            Ok(out) if !out.trim().is_empty() => {
                return Ok(TranslateResponse {
                    text: out,
                    provider_used: provider.label().to_string(),
                });
            }
            Ok(_) => last_err = format!("{}返回了空译文", provider.label()),
            Err(err) => last_err = err,
        }
    }
    Err(if last_err.is_empty() {
        "所有翻译引擎均请求失败，请检查网络连接".into()
    } else {
        last_err
    })
}

/// Translate OCR blocks in batched chunks while preserving 1-to-1 line alignment for in-place rendering.
pub fn translate_blocks(
    blocks: &[OcrRegionBox],
    target_lang: &str,
    preferred: TranslateProvider,
) -> Result<(Vec<OcrRegionBox>, String), String> {
    if blocks.is_empty() {
        return Err("未识别到可翻译的文字区域".into());
    }
    let joined = blocks
        .iter()
        .map(|b| b.text.replace('\n', " "))
        .collect::<Vec<_>>()
        .join("\n");
    let response = translate_text(&joined, "auto", target_lang, preferred)?;
    let translated_lines: Vec<String> = response
        .text
        .lines()
        .map(|s| s.trim().to_string())
        .collect();

    let mut out = blocks.to_vec();
    if translated_lines.len() == out.len() {
        for (block, line) in out.iter_mut().zip(translated_lines.into_iter()) {
            block.translated = Some(line);
        }
    } else {
        // Fallback: translate individually for short counts or distribute cleanly
        if out.len() <= 12 {
            for block in &mut out {
                if let Ok(single) = translate_text(&block.text, "auto", target_lang, preferred) {
                    block.translated = Some(single.text.trim().to_string());
                } else {
                    block.translated = Some(block.text.clone());
                }
            }
        } else {
            for (i, block) in out.iter_mut().enumerate() {
                block.translated = translated_lines
                    .get(i)
                    .cloned()
                    .or_else(|| Some(block.text.clone()));
            }
        }
    }
    Ok((out, response.provider_used))
}

fn fallback_order(preferred: TranslateProvider) -> Vec<TranslateProvider> {
    let all = [
        TranslateProvider::Youdao,
        TranslateProvider::Microsoft,
        TranslateProvider::Google,
        TranslateProvider::Baidu,
    ];
    let mut list = vec![preferred];
    for p in all {
        if p != preferred {
            list.push(p);
        }
    }
    list
}

fn normalize_lang(lang: &str) -> &'static str {
    let l = lang.trim();
    if l.is_empty() || l.eq_ignore_ascii_case("auto") {
        return "auto";
    }
    let lower = l.to_ascii_lowercase();
    if lower.contains("tw") || lower.contains("hant") || l.contains('繁') {
        return "zh-TW";
    }
    if lower.starts_with("zh") || l.contains('中') || l.contains('简') {
        return "zh-CN";
    }
    if lower.starts_with("en") || l.contains('英') {
        return "en";
    }
    if lower.starts_with("ja") || lower.starts_with("jp") || l.contains('日') {
        return "ja";
    }
    if lower.starts_with("ko") || l.contains('韩') {
        return "ko";
    }
    if lower.starts_with("fr") {
        return "fr";
    }
    if lower.starts_with("de") {
        return "de";
    }
    if lower.starts_with("es") {
        return "es";
    }
    if lower.starts_with("ru") {
        return "ru";
    }
    "zh-CN"
}

fn build_client() -> Result<Client, String> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("无法创建 HTTP 客户端：{e}"))
}

// ---------------- Youdao ----------------
fn youdao_lang(lang: &str) -> &str {
    match lang {
        "auto" => "Auto",
        "zh-CN" | "zh-TW" => "zh-CHS",
        "en" => "en",
        "ja" => "ja",
        "ko" => "ko",
        "fr" => "fr",
        "de" => "de",
        "es" => "es",
        "ru" => "ru",
        _ => "Auto",
    }
}

fn translate_youdao(text: &str, sl: &str, tl: &str) -> Result<String, String> {
    let client = build_client()?;
    let mut parts = Vec::new();
    for chunk in split_chunks(text, CHUNK_CHAR_LIMIT) {
        let resp = client
            .post("https://aidemo.youdao.com/trans")
            .form(&[
                ("q", chunk.as_str()),
                ("from", youdao_lang(sl)),
                ("to", youdao_lang(tl)),
            ])
            .send()
            .map_err(|_| "有道翻译请求失败".to_string())?;
        if !resp.status().is_success() {
            return Err(format!("有道翻译失败（HTTP {}）", resp.status()));
        }
        let body: Value = resp
            .json()
            .map_err(|e| format!("有道翻译响应解析失败：{e}"))?;
        let trans = body
            .get("translation")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "有道翻译未返回有效译文".to_string())?;
        parts.push(trans.to_string());
    }
    Ok(parts.join("\n"))
}

// ---------------- Google ----------------
fn translate_google(text: &str, sl: &str, tl: &str) -> Result<String, String> {
    let client = build_client()?;
    let mut out = Vec::new();
    for chunk in split_chunks(text, 360) {
        let mut url = Url::parse("https://translate.googleapis.com/translate_a/single")
            .map_err(|e| format!("Google 翻译地址无效：{e}"))?;
        url.query_pairs_mut()
            .append_pair("client", "gtx")
            .append_pair("sl", sl)
            .append_pair("tl", tl)
            .append_pair("dt", "t")
            .append_pair("q", &chunk);
        let resp = client
            .get(url)
            .send()
            .map_err(|_| "Google 翻译请求失败".to_string())?;
        if !resp.status().is_success() {
            return Err(format!("Google 翻译失败（HTTP {}）", resp.status()));
        }
        let body: Value = resp
            .json()
            .map_err(|e| format!("Google 翻译响应解析失败：{e}"))?;
        let segs = body
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_array)
            .ok_or_else(|| "Google 翻译没有返回译文".to_string())?;
        let mut piece = String::new();
        for seg in segs {
            if let Some(s) = seg.as_array().and_then(|a| a.first()).and_then(Value::as_str) {
                piece.push_str(s);
            }
        }
        if !piece.trim().is_empty() {
            out.push(piece.trim().to_string());
        }
    }
    if out.is_empty() {
        return Err("Google 翻译返回了空译文".into());
    }
    Ok(out.join("\n"))
}

// ---------------- Microsoft Bing ----------------
fn bing_lang(lang: &str) -> &str {
    match lang {
        "auto" => "auto-detect",
        "zh-CN" => "zh-Hans",
        "zh-TW" => "zh-Hant",
        other => other,
    }
}

fn find_quoted_after(web: &str, marker: &str, sep: char) -> Option<String> {
    let mut from = 0usize;
    while let Some(rel) = web[from..].find(marker) {
        let mut idx = from + rel + marker.len();
        let bytes = web.as_bytes();
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx < bytes.len() && bytes[idx] == sep as u8 {
            idx += 1;
            while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
                idx += 1;
            }
            if idx < bytes.len() && bytes[idx] == b'"' {
                let start = idx + 1;
                if let Some(end_rel) = web[start..].find('"') {
                    if end_rel > 0 {
                        return Some(web[start..start + end_rel].to_string());
                    }
                }
            }
        }
        from = idx.saturating_add(1).min(web.len());
    }
    None
}

fn read_bing_abuse_helper(web: &str) -> Option<(String, String)> {
    let marker = "params_AbusePreventionHelper";
    let mut from = 0usize;
    while let Some(rel) = web[from..].find(marker) {
        let mut idx = from + rel + marker.len();
        let bytes = web.as_bytes();
        let skip_ws = |p: &mut usize| {
            while *p < bytes.len() && bytes[*p].is_ascii_whitespace() {
                *p += 1;
            }
        };
        skip_ws(&mut idx);
        if idx < bytes.len() && bytes[idx] == b'=' {
            idx += 1;
            skip_ws(&mut idx);
        }
        if idx < bytes.len() && bytes[idx] == b'[' {
            idx += 1;
            skip_ws(&mut idx);
            let d_start = idx;
            while idx < bytes.len() && bytes[idx].is_ascii_digit() {
                idx += 1;
            }
            if idx > d_start {
                let key = web[d_start..idx].to_string();
                skip_ws(&mut idx);
                if idx < bytes.len() && bytes[idx] == b',' {
                    idx += 1;
                    skip_ws(&mut idx);
                    if idx < bytes.len() && bytes[idx] == b'"' {
                        let t_start = idx + 1;
                        if let Some(end_rel) = web[t_start..].find('"') {
                            if end_rel > 0 {
                                return Some((key, web[t_start..t_start + end_rel].to_string()));
                            }
                        }
                    }
                }
            }
        }
        from = idx.saturating_add(1).min(web.len());
    }
    None
}

fn fetch_bing_creds(client: &Client, base: &str) -> Result<BingCreds, String> {
    let resp = client
        .get(format!("{base}/translator"))
        .header("Accept-Language", "zh-CN,zh;q=0.9,en-US;q=0.8")
        .send()
        .map_err(|_| "微软翻译页面请求失败".to_string())?;
    let final_url = resp.url().clone();
    let host = format!(
        "{}://{}",
        final_url.scheme(),
        final_url.host_str().unwrap_or("cn.bing.com")
    );
    let body = resp.text().map_err(|_| "读取微软翻译页面失败".to_string())?;
    let (key, token) =
        read_bing_abuse_helper(&body).ok_or_else(|| "解析微软翻译凭证失败".to_string())?;
    let ig = find_quoted_after(&body, "IG", ':')
        .ok_or_else(|| "解析微软翻译 IG 失败".to_string())?;
    let iid = find_quoted_after(&body, "data-iid", '=')
        .or_else(|| find_quoted_after(&body, "IID", ':'))
        .ok_or_else(|| "解析微软翻译 IID 失败".to_string())?;
    Ok(BingCreds {
        host,
        key,
        token,
        ig,
        iid,
    })
}

fn translate_microsoft(text: &str, sl: &str, tl: &str) -> Result<String, String> {
    let client = build_client()?;
    let bases = ["https://cn.bing.com", "https://www.bing.com"];
    let mut last_err = String::from("微软翻译失败");
    for base in bases {
        let cached = BING_CACHE.lock().ok().and_then(|g| g.clone());
        let creds = match cached {
            Some(c) => c,
            None => match fetch_bing_creds(&client, base) {
                Ok(c) => {
                    if let Ok(mut g) = BING_CACHE.lock() {
                        *g = Some(c.clone());
                    }
                    c
                }
                Err(e) => {
                    last_err = e;
                    continue;
                }
            },
        };
        let url = format!("{}/ttranslatev3?IG={}&IID={}", creds.host, creds.ig, creds.iid);
        let resp = client
            .post(&url)
            .header("Referer", format!("{}/translator", creds.host))
            .header("Origin", &creds.host)
            .form(&[
                ("text", text),
                ("fromLang", bing_lang(sl)),
                ("to", bing_lang(tl)),
                ("token", creds.token.as_str()),
                ("key", creds.key.as_str()),
            ])
            .send();
        let Ok(resp) = resp else {
            if let Ok(mut g) = BING_CACHE.lock() {
                *g = None;
            }
            last_err = "微软翻译请求失败".into();
            continue;
        };
        let body_text = resp.text().unwrap_or_default();
        if body_text.trim().is_empty() {
            if let Ok(mut g) = BING_CACHE.lock() {
                *g = None;
            }
            last_err = "微软翻译返回空响应".into();
            continue;
        }
        if let Ok(val) = serde_json::from_str::<Value>(&body_text) {
            if let Some(translated) = val
                .as_array()
                .and_then(|a| a.first())
                .and_then(|o| o.get("translations"))
                .and_then(Value::as_array)
                .and_then(|t| t.first())
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
            {
                return Ok(translated.trim().to_string());
            }
        }
        if let Ok(mut g) = BING_CACHE.lock() {
            *g = None;
        }
    }
    Err(last_err)
}

// ---------------- Baidu ----------------
fn baidu_lang(lang: &str) -> &str {
    match lang {
        "auto" => "auto",
        "zh-CN" => "zh",
        "zh-TW" => "cht",
        "ja" => "jp",
        "ko" => "kor",
        "fr" => "fra",
        "es" => "spa",
        other => other,
    }
}

fn translate_baidu(text: &str, sl: &str, tl: &str) -> Result<String, String> {
    let client = build_client()?;
    let app_id = "20240601002067575";
    let secret = "FR7xu9EngQpmqAcvcC27";
    let q: String = text.chars().take(4800).collect();
    let salt = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_else(|_| "123456".into());
    let mut hasher = Md5::new();
    hasher.update(format!("{app_id}{q}{salt}{secret}").as_bytes());
    let sign = hex::encode(hasher.finalize());

    let resp = client
        .post("https://fanyi-api.baidu.com/api/trans/vip/translate")
        .form(&[
            ("q", q.as_str()),
            ("from", baidu_lang(sl)),
            ("to", baidu_lang(tl)),
            ("appid", app_id),
            ("salt", salt.as_str()),
            ("sign", sign.as_str()),
        ])
        .send()
        .map_err(|_| "百度翻译请求失败".to_string())?;
    let body: Value = resp
        .json()
        .map_err(|e| format!("百度翻译响应解析失败：{e}"))?;
    if let Some(code) = body.get("error_code") {
        let msg = body
            .get("error_msg")
            .and_then(Value::as_str)
            .unwrap_or("百度翻译失败");
        return Err(format!("百度翻译错误（{code}）：{msg}"));
    }
    let arr = body
        .get("trans_result")
        .and_then(Value::as_array)
        .ok_or_else(|| "百度翻译没有返回译文".to_string())?;
    let lines: Vec<&str> = arr
        .iter()
        .filter_map(|item| item.get("dst").and_then(Value::as_str))
        .collect();
    if lines.is_empty() {
        return Err("百度翻译返回了空译文".into());
    }
    Ok(lines.join("\n"))
}

fn split_chunks(text: &str, limit: usize) -> Vec<String> {
    let mut remaining: Vec<char> = text.chars().collect();
    let mut chunks = Vec::new();
    while !remaining.is_empty() {
        let split_at = if remaining.len() <= limit {
            remaining.len()
        } else {
            remaining[..limit]
                .iter()
                .rposition(|character| {
                    matches!(
                        character,
                        '\n' | '。' | '！' | '？' | '.' | '!' | '?' | '；' | ';' | ' '
                    )
                })
                .map(|index| index + 1)
                .filter(|index| *index >= limit * 2 / 3)
                .unwrap_or(limit)
        };
        chunks.push(remaining[..split_at].iter().collect());
        remaining.drain(..split_at);
    }
    chunks
}
