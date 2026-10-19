use crate::models::FileShareConfig;
use std::fs;
use std::path::PathBuf;

pub fn plugin_data_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        PathBuf::from(appdata)
            .join("com.agiletiles.app")
            .join("plugin-data")
            .join("file_share")
    } else {
        PathBuf::from(".agile-data")
            .join("plugin-data")
            .join("file_share")
    }
}

fn config_path() -> PathBuf {
    plugin_data_dir().join("shares.json")
}

pub fn load_shares() -> Result<Vec<FileShareConfig>, String> {
    let path = config_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("Unable to read File Share configuration: {error}"))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("Unable to parse File Share configuration: {error}"))
}

pub fn save_shares(shares: &[FileShareConfig]) -> Result<(), String> {
    let dir = plugin_data_dir();
    fs::create_dir_all(&dir)
        .map_err(|error| format!("Unable to create File Share data directory: {error}"))?;
    let content = serde_json::to_string_pretty(shares)
        .map_err(|error| format!("Unable to serialize File Share configuration: {error}"))?;
    fs::write(config_path(), content)
        .map_err(|error| format!("Unable to write File Share configuration: {error}"))?;
    Ok(())
}
