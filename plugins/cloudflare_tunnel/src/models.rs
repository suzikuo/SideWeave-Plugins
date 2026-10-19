use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ServiceRule {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub service: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TunnelRecord {
    pub id: String,
    pub name: String,
    pub tunnel_id: String,
    pub domain: String,
    pub created_at: String,
    #[serde(default)]
    pub services: Vec<ServiceRule>,
    #[serde(default)]
    pub file_share: Option<FileShareConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FileShareConfig {
    pub enabled: bool,
    pub directory: String,
    pub hostname: String,
    pub token: String,
    pub port: u16,
    pub lan_enabled: bool,
    pub allowed_ips: Vec<String>,
    pub upload_enabled: bool,
    pub upload_directory: String,
    pub max_upload_mb: u32,
    pub max_total_upload_mb: u32,
}

impl Default for FileShareConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            directory: String::new(),
            hostname: String::new(),
            token: String::new(),
            port: 12026,
            lan_enabled: false,
            allowed_ips: Vec::new(),
            upload_enabled: false,
            upload_directory: String::new(),
            max_upload_mb: 100,
            max_total_upload_mb: 2048,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileShareView {
    pub enabled: bool,
    pub directory: String,
    pub local_url: String,
    pub local_running: bool,
    pub port: u16,
    pub lan_enabled: bool,
    pub lan_running: bool,
    pub lan_host: String,
    pub lan_share_url: String,
    pub cloudflare_share_url: String,
    pub upload_enabled: bool,
    pub upload_directory: String,
    pub max_upload_mb: u32,
    pub max_total_upload_mb: u32,
    pub allowed_ips: Vec<String>,
    pub access_logs: Vec<crate::file_share::FileShareAccessLog>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TunnelState {
    Stopped,
    Starting,
    Running,
    Error,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelSummary {
    pub id: String,
    pub name: String,
    pub tunnel_id: String,
    pub domain: String,
    pub created_at: String,
    pub status: TunnelState,
    pub conns: u32,
    pub service_count: usize,
    pub config_path: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceView {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub full_url: String,
    pub service: String,
    pub port: u16,
    pub local_online: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelDetailView {
    pub tunnel: TunnelRecord,
    pub status: TunnelState,
    pub conns: u32,
    pub services: Vec<ServiceView>,
    pub file_share: Option<FileShareView>,
    pub error_message: Option<String>,
    pub config_path: String,
    pub credentials_path: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct CreateTunnelInput {
    pub name: String,
    pub domain: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ImportTunnelInput {
    pub name: String,
    pub domain: String,
    pub tunnel_id: String,
    pub credentials_path: Option<String>,
    pub credentials_content: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateTunnelInput {
    pub id: String,
    pub name: String,
    pub domain: String,
}
