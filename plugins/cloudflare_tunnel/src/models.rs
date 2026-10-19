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
    #[serde(default)]
    pub disable_chunked_encoding: bool,
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
    pub disable_chunked_encoding: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelDetailView {
    pub tunnel: TunnelRecord,
    pub status: TunnelState,
    pub conns: u32,
    pub services: Vec<ServiceView>,
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
