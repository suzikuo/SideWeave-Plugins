//! Tencent Cloud OCR `RecognizeAgent` service.
//!
//! Tencent Cloud RecognizeAgent client used when the user triggers text
//! extraction from the screenshot toolbar or editor.

use base64::Engine as _;
use hmac::{Hmac, Mac};
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use reqwest::blocking::Client;
use reqwest::Url;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const DEFAULT_ENDPOINT: &str = "https://ocr.tencentcloudapi.com";
pub const API_VERSION: &str = "2018-11-19";
pub const ACTION: &str = "RecognizeAgent";
const SERVICE: &str = "ocr";
const CONTENT_TYPE: &str = "application/json";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(50);
const MAX_IMAGE_BASE64_BYTES: usize = 10 * 1024 * 1024;

/// Tencent Cloud SecretId/SecretKey credentials.
///
/// `token` is optional and is used for temporary credentials.  The service
/// never persists these values; callers should source them from the host
/// secret service when wiring this module into the plugin.
#[derive(Clone)]
pub struct TencentCloudCredentials {
    pub secret_id: String,
    pub secret_key: String,
    pub token: Option<String>,
}

impl TencentCloudCredentials {
    pub fn new(secret_id: impl Into<String>, secret_key: impl Into<String>) -> Self {
        Self {
            secret_id: secret_id.into(),
            secret_key: secret_key.into(),
            token: None,
        }
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        let token = token.into().trim().to_owned();
        self.token = (!token.is_empty()).then_some(token);
        self
    }
}

impl fmt::Debug for TencentCloudCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TencentCloudCredentials")
            .field("secret_id", &self.secret_id)
            .field("secret_key", &"***")
            .field("token", &self.token.as_ref().map(|_| "***"))
            .finish()
    }
}

/// Query modes documented by `RecognizeAgent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecognizeAgentQueryType {
    /// Recognize and return the full text.
    #[default]
    FullText,
    /// Judge whether the image matches one query.
    Judge,
    /// Classify the image into one of two to five queries.
    Classify,
    /// Summarize image content related to one query.
    Summarize,
    /// Extract values according to `SchemaItems`.
    Extract,
}

impl RecognizeAgentQueryType {
    pub const fn value(self) -> u8 {
        match self {
            Self::FullText => 0,
            Self::Judge => 1,
            Self::Classify => 2,
            Self::Summarize => 3,
            Self::Extract => 4,
        }
    }
}

impl Serialize for RecognizeAgentQueryType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(self.value())
    }
}

/// A custom field definition for `QueryType=4` information extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaItem {
    #[serde(rename = "KeyName")]
    pub key_name: String,
    #[serde(rename = "KeyType")]
    pub key_type: u8,
    #[serde(rename = "KeyPrompt", skip_serializing_if = "Option::is_none", default)]
    pub key_prompt: Option<String>,
    #[serde(rename = "SubItems", skip_serializing_if = "Vec::is_empty", default)]
    pub sub_items: Vec<SchemaItem>,
}

impl SchemaItem {
    pub fn new(key_name: impl Into<String>, key_type: u8) -> Self {
        Self {
            key_name: key_name.into(),
            key_type,
            key_prompt: None,
            sub_items: Vec::new(),
        }
    }

    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.key_prompt = Some(prompt.into());
        self
    }

    pub fn with_sub_items(mut self, sub_items: Vec<SchemaItem>) -> Self {
        self.sub_items = sub_items;
        self
    }
}

/// Body for Tencent Cloud's `RecognizeAgent` action.
#[derive(Debug, Clone, Serialize)]
pub struct RecognizeAgentRequest {
    #[serde(rename = "ImageUrl", skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(rename = "ImageBase64", skip_serializing_if = "Option::is_none")]
    pub image_base64: Option<String>,
    #[serde(rename = "PdfPageNumber", skip_serializing_if = "Option::is_none")]
    pub pdf_page_number: Option<u32>,
    #[serde(rename = "SelectModel", skip_serializing_if = "Option::is_none")]
    pub select_model: Option<u8>,
    #[serde(rename = "QueryType")]
    pub query_type: RecognizeAgentQueryType,
    #[serde(rename = "SchemaItems", skip_serializing_if = "Vec::is_empty")]
    pub schema_items: Vec<SchemaItem>,
    #[serde(rename = "Query", skip_serializing_if = "Vec::is_empty")]
    pub query: Vec<String>,
    #[serde(rename = "EnableCoord", skip_serializing_if = "Option::is_none")]
    pub enable_coord: Option<bool>,
}

impl Default for RecognizeAgentRequest {
    fn default() -> Self {
        Self {
            image_url: None,
            image_base64: None,
            pdf_page_number: None,
            select_model: None,
            query_type: RecognizeAgentQueryType::FullText,
            schema_items: Vec::new(),
            query: Vec::new(),
            enable_coord: None,
        }
    }
}

impl RecognizeAgentRequest {
    pub fn with_image_url(mut self, image_url: impl Into<String>) -> Self {
        self.image_url = Some(image_url.into());
        self
    }

    pub fn with_image_base64(mut self, image_base64: impl Into<String>) -> Self {
        self.image_base64 = Some(image_base64.into());
        self
    }

    pub fn with_query(mut self, query: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.query = query.into_iter().map(Into::into).collect();
        self
    }

    pub fn validate(&self) -> Result<(), TencentOcrError> {
        let has_url = self
            .image_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_base64 = self
            .image_base64
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        if !has_url && !has_base64 {
            return Err(TencentOcrError::InvalidRequest(
                "ImageUrl 和 ImageBase64 至少需要提供一个".into(),
            ));
        }
        if let Some(image_base64) = self.image_base64.as_deref() {
            if image_base64.len() > MAX_IMAGE_BASE64_BYTES {
                return Err(TencentOcrError::InvalidRequest(format!(
                    "ImageBase64 超过腾讯云 10 MiB 限制（当前 {} 字节）",
                    image_base64.len()
                )));
            }
        }
        if self.select_model.is_some_and(|model| model > 1) {
            return Err(TencentOcrError::InvalidRequest(
                "SelectModel 只支持 0 或 1".into(),
            ));
        }
        if self.pdf_page_number == Some(0) {
            return Err(TencentOcrError::InvalidRequest(
                "PdfPageNumber 必须大于 0".into(),
            ));
        }
        if self.enable_coord == Some(true)
            && self.query_type != RecognizeAgentQueryType::Extract
        {
            return Err(TencentOcrError::InvalidRequest(
                "EnableCoord 仅支持 QueryType=4 信息提取".into(),
            ));
        }

        match self.query_type {
            RecognizeAgentQueryType::Judge | RecognizeAgentQueryType::Summarize
                if self.query.len() != 1 =>
            {
                return Err(TencentOcrError::InvalidRequest(
                    "QueryType=1/3 时 Query 必须恰好包含一个提示词".into(),
                ));
            }
            RecognizeAgentQueryType::Classify if !(2..=5).contains(&self.query.len()) => {
                return Err(TencentOcrError::InvalidRequest(
                    "QueryType=2 时 Query 必须包含 2 到 5 个分类项".into(),
                ));
            }
            RecognizeAgentQueryType::Extract if self.schema_items.is_empty() => {
                return Err(TencentOcrError::InvalidRequest(
                    "QueryType=4 时 SchemaItems 不能为空".into(),
                ));
            }
            _ => {}
        }
        if self.query_type == RecognizeAgentQueryType::Extract {
            validate_schema_items(&self.schema_items, 1)?;
        }
        Ok(())
    }
}

/// Parsed successful response.  `raw` keeps the complete Tencent response,
/// while `response` is the value under the top-level `Response` key.
#[derive(Debug, Clone)]
pub struct RecognizeAgentResult {
    pub request_id: Option<String>,
    pub response: Value,
    pub raw: Value,
}

impl RecognizeAgentResult {
    /// Extract common text fields from full-text and reasoning responses.
    /// Callers needing structured extraction should use `response` or `raw`.
    pub fn text(&self) -> Option<String> {
        let mut values = Vec::new();
        collect_text_values(&self.response, &mut values);
        (!values.is_empty()).then(|| values.join("\n"))
    }
}

/// Reusable blocking client for Tencent Cloud OCR.
pub struct TencentOcrService {
    client: Client,
    credentials: TencentCloudCredentials,
    endpoint: Url,
    host: String,
    region: Option<String>,
}

impl TencentOcrService {
    pub fn new(credentials: TencentCloudCredentials) -> Result<Self, TencentOcrError> {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(TencentOcrError::Request)?;
        let endpoint = Url::parse(DEFAULT_ENDPOINT)
            .map_err(|error| TencentOcrError::Configuration(error.to_string()))?;
        let host = host_from_url(&endpoint)?;
        Ok(Self {
            client,
            credentials,
            endpoint,
            host,
            region: None,
        })
    }

    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        let region = region.into();
        self.region = (!region.trim().is_empty()).then_some(region);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, TencentOcrError> {
        self.client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(TencentOcrError::Request)?;
        Ok(self)
    }

    pub fn recognize_agent(
        &self,
        request: &RecognizeAgentRequest,
    ) -> Result<RecognizeAgentResult, TencentOcrError> {
        if self.credentials.secret_id.trim().is_empty()
            || self.credentials.secret_key.trim().is_empty()
        {
            return Err(TencentOcrError::Configuration(
                "Tencent Cloud SecretId/SecretKey 不能为空".into(),
            ));
        }
        request.validate()?;
        let payload = serde_json::to_vec(request)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| TencentOcrError::Configuration(error.to_string()))?
            .as_secs();
        let authorization = build_authorization(&self.credentials, &self.host, timestamp, &payload);

        let mut http_request = self
            .client
            .post(self.endpoint.clone())
            .header("Content-Type", CONTENT_TYPE)
            .header("Host", &self.host)
            .header("X-TC-Action", ACTION)
            .header("X-TC-Version", API_VERSION)
            .header("X-TC-Timestamp", timestamp.to_string())
            .header("Authorization", authorization)
            .body(payload);
        if let Some(region) = self.region.as_deref() {
            http_request = http_request.header("X-TC-Region", region);
        }
        if let Some(token) = self.credentials.token.as_deref() {
            http_request = http_request.header("X-TC-Token", token);
        }

        let response = http_request.send().map_err(TencentOcrError::Request)?;
        let status = response.status();
        let body = response.text().map_err(TencentOcrError::Request)?;
        let raw: Value =
            serde_json::from_str(&body).map_err(|error| TencentOcrError::ResponseDecode {
                status: status.as_u16(),
                message: error.to_string(),
                body: compact(&body),
            })?;
        let response_value =
            raw.get("Response")
                .cloned()
                .ok_or_else(|| TencentOcrError::ResponseDecode {
                    status: status.as_u16(),
                    message: "响应缺少 Response 字段".into(),
                    body: compact(&body),
                })?;
        let request_id = response_value
            .get("RequestId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(error) = response_value.get("Error") {
            return Err(TencentOcrError::Api {
                code: error
                    .get("Code")
                    .and_then(Value::as_str)
                    .unwrap_or("UnknownError")
                    .to_owned(),
                message: error
                    .get("Message")
                    .and_then(Value::as_str)
                    .unwrap_or("腾讯云 OCR 返回业务错误")
                    .to_owned(),
                request_id,
            });
        }
        if !status.is_success() {
            return Err(TencentOcrError::HttpStatus {
                status: status.as_u16(),
                body: compact(&body),
            });
        }
        Ok(RecognizeAgentResult {
            request_id,
            response: response_value,
            raw,
        })
    }

    /// Encode a screenshot's RGBA pixels as PNG and call `RecognizeAgent`.
    pub fn recognize_rgba(
        &self,
        width: usize,
        height: usize,
        rgba: &[u8],
        mut request: RecognizeAgentRequest,
    ) -> Result<RecognizeAgentResult, TencentOcrError> {
        request.image_url = None;
        request.image_base64 = Some(encode_rgba_png_base64(width, height, rgba)?);
        self.recognize_agent(&request)
    }
}

pub fn encode_rgba_png_base64(
    width: usize,
    height: usize,
    rgba: &[u8],
) -> Result<String, TencentOcrError> {
    let width = u32::try_from(width)
        .map_err(|_| TencentOcrError::InvalidRequest("截图宽度超过 PNG 支持范围".into()))?;
    let height = u32::try_from(height)
        .map_err(|_| TencentOcrError::InvalidRequest("截图高度超过 PNG 支持范围".into()))?;
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| TencentOcrError::InvalidRequest("截图尺寸溢出".into()))?;
    if rgba.len() != expected {
        return Err(TencentOcrError::InvalidRequest(format!(
            "RGBA 截图数据长度不匹配：需要 {expected}，实际 {}",
            rgba.len()
        )));
    }
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|error| TencentOcrError::Encode(format!("PNG 编码失败：{error}")))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(png))
}

#[derive(Debug)]
pub enum TencentOcrError {
    Configuration(String),
    InvalidRequest(String),
    Encode(String),
    Request(reqwest::Error),
    ResponseDecode {
        status: u16,
        message: String,
        body: String,
    },
    HttpStatus {
        status: u16,
        body: String,
    },
    Api {
        code: String,
        message: String,
        request_id: Option<String>,
    },
    Json(serde_json::Error),
}

impl fmt::Display for TencentOcrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message) => write!(formatter, "腾讯云 OCR 配置错误：{message}"),
            Self::InvalidRequest(message) => write!(formatter, "腾讯云 OCR 请求无效：{message}"),
            Self::Encode(message) => write!(formatter, "腾讯云 OCR 图片编码失败：{message}"),
            Self::Request(error) => write!(formatter, "腾讯云 OCR 网络请求失败：{error}"),
            Self::ResponseDecode {
                status,
                message,
                body,
            } => write!(
                formatter,
                "腾讯云 OCR 响应解析失败（HTTP {status}）：{message}；响应：{body}"
            ),
            Self::HttpStatus { status, body } => {
                write!(formatter, "腾讯云 OCR 返回 HTTP {status}：{body}")
            }
            Self::Api {
                code,
                message,
                request_id,
            } => write!(
                formatter,
                "腾讯云 OCR 业务错误 {code}：{message}{}",
                request_id
                    .as_deref()
                    .map(|id| format!("（RequestId: {id}）"))
                    .unwrap_or_default()
            ),
            Self::Json(error) => write!(formatter, "腾讯云 OCR JSON 编码失败：{error}"),
        }
    }
}

impl std::error::Error for TencentOcrError {}

impl From<reqwest::Error> for TencentOcrError {
    fn from(error: reqwest::Error) -> Self {
        Self::Request(error)
    }
}

impl From<serde_json::Error> for TencentOcrError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

fn host_from_url(url: &Url) -> Result<String, TencentOcrError> {
    let host = url
        .host_str()
        .ok_or_else(|| TencentOcrError::Configuration("OCR Endpoint 缺少主机名".into()))?;
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

fn build_authorization(
    credentials: &TencentCloudCredentials,
    host: &str,
    timestamp: u64,
    payload: &[u8],
) -> String {
    let date = utc_date(timestamp);
    let hashed_payload = hex::encode(Sha256::digest(payload));
    let canonical_headers = format!("content-type:{CONTENT_TYPE}\nhost:{host}\n");
    let signed_headers = "content-type;host";
    let canonical_request =
        format!("POST\n/\n\n{canonical_headers}\n{signed_headers}\n{hashed_payload}");
    let credential_scope = format!("{date}/{SERVICE}/tc3_request");
    let string_to_sign = format!(
        "TC3-HMAC-SHA256\n{timestamp}\n{credential_scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let secret_date = hmac_sha256(
        format!("TC3{}", credentials.secret_key).as_bytes(),
        date.as_bytes(),
    );
    let secret_service = hmac_sha256(&secret_date, SERVICE.as_bytes());
    let secret_signing = hmac_sha256(&secret_service, b"tc3_request");
    let signature = hex::encode(hmac_sha256(&secret_signing, string_to_sign.as_bytes()));
    format!(
        "TC3-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.secret_id
    )
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts arbitrary key lengths");
    mac.update(message);
    let digest = mac.finalize().into_bytes();
    let mut output = [0u8; 32];
    output.copy_from_slice(&digest);
    output
}

fn utc_date(timestamp: u64) -> String {
    // Gregorian calendar conversion from Unix days; avoids adding a date crate
    // just for TC3's YYYY-MM-DD credential scope.
    let days = (timestamp / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}")
}

fn collect_text_values(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            const TEXT_KEYS: [&str; 4] = ["DetectedText", "Answer", "Text", "KeyValue"];
            for key in TEXT_KEYS {
                if let Some(text) = object.get(key).and_then(Value::as_str) {
                    let text = text.trim();
                    if !text.is_empty() {
                        output.push(text.to_owned());
                    }
                }
            }
            for (key, child) in object {
                if !TEXT_KEYS.contains(&key.as_str()) {
                    collect_text_values(child, output);
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_text_values(child, output);
            }
        }
        _ => {}
    }
}

fn validate_schema_items(items: &[SchemaItem], depth: usize) -> Result<(), TencentOcrError> {
    if depth > 3 {
        return Err(TencentOcrError::InvalidRequest(
            "SchemaItems 最多支持三层嵌套".into(),
        ));
    }
    for item in items {
        if item.key_name.trim().is_empty() {
            return Err(TencentOcrError::InvalidRequest(
                "SchemaItems.KeyName 不能为空".into(),
            ));
        }
        match item.key_type {
            0 if !item.sub_items.is_empty() => {
                return Err(TencentOcrError::InvalidRequest(
                    "KeyType=0 的字段不能配置 SubItems".into(),
                ));
            }
            1 if item.sub_items.is_empty() => {
                return Err(TencentOcrError::InvalidRequest(
                    "KeyType=1 的数组字段需要至少一个 SubItems".into(),
                ));
            }
            1 => validate_schema_items(&item.sub_items, depth + 1)?,
            0 => {}
            _ => {
                return Err(TencentOcrError::InvalidRequest(
                    "SchemaItems.KeyType 只支持 0 或 1".into(),
                ));
            }
        }
    }
    Ok(())
}

fn compact(value: &str) -> String {
    let value = value.replace(['\r', '\n'], " ");
    if value.len() > 500 {
        let prefix: String = value.chars().take(500).collect();
        format!("{prefix}…")
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_authorization, utc_date, RecognizeAgentQueryType, RecognizeAgentRequest,
        TencentCloudCredentials,
    };
    use serde_json::json;

    #[test]
    fn request_uses_tencent_pascal_case_fields() {
        let request = RecognizeAgentRequest::default()
            .with_image_base64("abc")
            .with_query(["invoice"]);
        let value = serde_json::to_value(request).expect("serialize request");
        assert_eq!(value["ImageBase64"], "abc");
        assert_eq!(value["QueryType"], 0);
        assert_eq!(value["Query"][0], "invoice");
    }

    #[test]
    fn query_validation_matches_documented_cardinality() {
        let request = RecognizeAgentRequest {
            query_type: RecognizeAgentQueryType::Classify,
            image_base64: Some("abc".into()),
            ..Default::default()
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn result_text_can_read_common_response_fields() {
        let result = super::RecognizeAgentResult {
            request_id: Some("req".into()),
            response: json!({"Response": [{"TextDetections": [{"DetectedText": "hello"}]}, {"Answer": "world"}]}),
            raw: json!({}),
        };
        assert_eq!(result.text().as_deref(), Some("hello\nworld"));
    }

    #[test]
    fn tc3_scope_date_and_signature_are_deterministic() {
        assert_eq!(utc_date(1_617_081_031), "2021-03-30");
        let credentials = TencentCloudCredentials::new("AKIDEXAMPLE", "secret");
        let first = build_authorization(
            &credentials,
            "ocr.tencentcloudapi.com",
            1_617_081_031,
            b"{}",
        );
        let second = build_authorization(
            &credentials,
            "ocr.tencentcloudapi.com",
            1_617_081_031,
            b"{}",
        );
        assert_eq!(first, second);
        assert!(
            first.starts_with("TC3-HMAC-SHA256 Credential=AKIDEXAMPLE/2021-03-30/ocr/tc3_request")
        );
    }
}
