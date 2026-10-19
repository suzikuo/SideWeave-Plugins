use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FileShareConfig {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub view_directory: String,
    pub receive_directory: String,
    pub token: String,
    pub port: u16,
    pub tunnel_hostname: String,
    pub lan_enabled: bool,
    pub allowed_ips: Vec<String>,
    pub allow_view: bool,
    pub allow_upload: bool,
    pub require_login: bool,
    pub media_preview_limit_kbps: u32,
    pub max_upload_mb: u32,
    pub max_total_upload_mb: u32,
}

impl Default for FileShareConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: false,
            view_directory: String::new(),
            receive_directory: String::new(),
            token: String::new(),
            port: 12026,
            tunnel_hostname: String::new(),
            lan_enabled: false,
            allowed_ips: Vec::new(),
            allow_view: true,
            allow_upload: false,
            require_login: false,
            media_preview_limit_kbps: 512,
            max_upload_mb: 100,
            max_total_upload_mb: 2048,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileShareView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub view_directory: String,
    pub receive_directory: String,
    pub token: String,
    pub local_url: String,
    pub local_running: bool,
    pub port: u16,
    pub tunnel_hostname: String,
    pub cloudflare_url: String,
    pub lan_enabled: bool,
    pub lan_running: bool,
    pub lan_host: String,
    pub lan_share_url: String,
    pub allow_view: bool,
    pub allow_upload: bool,
    pub require_login: bool,
    pub media_preview_limit_kbps: u32,
    pub max_upload_mb: u32,
    pub max_total_upload_mb: u32,
    pub allowed_ips: Vec<String>,
    pub access_logs: Vec<crate::file_share::FileShareAccessLog>,
}
