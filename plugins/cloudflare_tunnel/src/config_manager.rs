use crate::models::{ServiceRule, TunnelRecord};
use std::fs;
use std::path::{Path, PathBuf};

pub fn plugin_data_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        PathBuf::from(appdata)
            .join("com.agiletiles.app")
            .join("plugin-data")
            .join("cloudflare_tunnel")
    } else {
        PathBuf::from(".agile-data")
            .join("plugin-data")
            .join("cloudflare_tunnel")
    }
}

pub fn tunnel_json_path() -> PathBuf {
    plugin_data_dir().join("tunnel.json")
}

pub fn credentials_path() -> PathBuf {
    plugin_data_dir().join("credentials.json")
}

pub fn config_yaml_path() -> PathBuf {
    plugin_data_dir().join("config.yaml")
}

pub fn all_config_yaml_path() -> PathBuf {
    plugin_data_dir().join("all_config.yaml")
}

/// 规范化本地服务地址，支持纯端口号（如 6694）、:6694 或 http://127.0.0.1:<port>
pub fn normalize_service_address(service: &str) -> Result<(String, u16), String> {
    let trimmed = service.trim();
    if trimmed.is_empty() {
        return Err("服务地址不能为空".to_string());
    }

    // 允许纯端口：6694
    if let Ok(port) = trimmed.parse::<u16>() {
        if port == 0 {
            return Err("端口号不能为 0".to_string());
        }
        return Ok((format!("http://127.0.0.1:{port}"), port));
    }

    // 允许 :6694
    if let Some(port_str) = trimmed.strip_prefix(':') {
        let port: u16 = port_str
            .trim()
            .parse()
            .map_err(|_| "端口号无效".to_string())?;
        if port == 0 {
            return Err("端口号不能为 0".to_string());
        }
        return Ok((format!("http://127.0.0.1:{port}"), port));
    }

    let without_proto = if let Some(rest) = trimmed.strip_prefix("http://") {
        rest
    } else if trimmed.starts_with("https://")
        || trimmed.starts_with("file://")
        || trimmed.starts_with("\\\\")
    {
        return Err("仅支持本地 HTTP 协议服务（如 http://127.0.0.1:端口）".to_string());
    } else {
        trimmed
    };

    let (host, port_str) = without_proto
        .split_once(':')
        .ok_or_else(|| "地址格式需为 端口号、127.0.0.1:端口 或 localhost:端口".to_string())?;

    let host = host.trim().to_lowercase();
    if host != "127.0.0.1" && host != "localhost" {
        return Err("服务地址必须指向本地环回地址（127.0.0.1 或 localhost）".to_string());
    }

    let port: u16 = port_str
        .trim()
        .trim_end_matches('/')
        .parse()
        .map_err(|_| "端口号无效，需为 1-65535 的整数".to_string())?;

    if port == 0 {
        return Err("端口号不能为 0".to_string());
    }

    Ok((format!("http://127.0.0.1:{port}"), port))
}

pub fn extract_port(service: &str) -> u16 {
    normalize_service_address(service)
        .map(|(_, port)| port)
        .unwrap_or(0)
}

pub fn full_hostname(subdomain: &str, domain: &str) -> String {
    let sub = subdomain.trim().trim_matches('.');
    let dom = domain.trim().trim_matches('.');
    if sub.is_empty() {
        return dom.to_string();
    }
    if sub.ends_with(dom) {
        sub.to_string()
    } else {
        format!("{sub}.{dom}")
    }
}

pub fn generate_config_yaml(
    tunnel_id: &str,
    credentials_abs_path: &Path,
    domain: &str,
    services: &[ServiceRule],
    only_enabled: bool,
) -> Result<String, String> {
    let cred_path_str = credentials_abs_path.to_string_lossy().replace('\\', "/");
    let mut yaml = String::new();
    yaml.push_str(&format!("tunnel: {tunnel_id}\n"));
    yaml.push_str(&format!("credentials-file: \"{cred_path_str}\"\n\n"));
    yaml.push_str("ingress:\n");

    for s in services {
        if only_enabled && !s.enabled {
            continue;
        }
        let (norm_service, _) = normalize_service_address(&s.service)?;
        let host = full_hostname(&s.hostname, domain);
        yaml.push_str(&format!("  - hostname: {host}\n"));
        yaml.push_str(&format!("    service: {norm_service}\n"));
    }

    // 强制追加 404 兜底规则，防止未授权暴露
    yaml.push_str("  - service: http_status:404\n");
    Ok(yaml)
}

pub fn has_tunnel() -> bool {
    tunnel_json_path().exists()
}

pub fn load_tunnel() -> Result<TunnelRecord, String> {
    let json_path = tunnel_json_path();
    if !json_path.exists() {
        return Err("尚未配置 Tunnel".to_string());
    }
    let content =
        fs::read_to_string(&json_path).map_err(|e| format!("读取 tunnel.json 失败: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("解析 tunnel.json 失败: {e}"))
}

pub fn save_tunnel(record: &TunnelRecord) -> Result<(), String> {
    save_tunnel_record_only(record)?;

    let cred_path = credentials_path();

    // 1. 生成实际由 cloudflared 运行加载的 config.yaml（仅包含启用服务）
    let active_yaml = generate_config_yaml(
        &record.tunnel_id,
        &cred_path,
        &record.domain,
        &record.services,
        true,
    )?;
    let yaml_path = config_yaml_path();
    fs::write(&yaml_path, active_yaml).map_err(|e| format!("保存 config.yaml 失败: {e}"))?;

    // 2. 同时生成包含所有配置的 all_config.yaml（便于对比与备份）
    let all_yaml = generate_config_yaml(
        &record.tunnel_id,
        &cred_path,
        &record.domain,
        &record.services,
        false,
    )?;
    let all_yaml_path = all_config_yaml_path();
    fs::write(&all_yaml_path, all_yaml).map_err(|e| format!("保存 all_config.yaml 失败: {e}"))?;

    Ok(())
}

pub fn save_tunnel_record_only(record: &TunnelRecord) -> Result<(), String> {
    let dir = plugin_data_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {e}"))?;

    let json_path = tunnel_json_path();
    let content = serde_json::to_string_pretty(record).map_err(|e| format!("序列化失败: {e}"))?;
    fs::write(&json_path, content).map_err(|e| format!("保存 tunnel.json 失败: {e}"))?;
    Ok(())
}

pub fn delete_tunnel_data() -> Result<(), String> {
    let dir = plugin_data_dir();
    if dir.exists() {
        let _ = fs::remove_file(tunnel_json_path());
        let _ = fs::remove_file(config_yaml_path());
        let _ = fs::remove_file(all_config_yaml_path());
        let _ = fs::remove_file(credentials_path());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_service() {
        assert_eq!(
            normalize_service_address("6694").unwrap(),
            ("http://127.0.0.1:6694".to_string(), 6694)
        );
        assert_eq!(
            normalize_service_address("127.0.0.1:16888").unwrap(),
            ("http://127.0.0.1:16888".to_string(), 16888)
        );
        assert_eq!(
            normalize_service_address("http://localhost:22343/").unwrap(),
            ("http://127.0.0.1:22343".to_string(), 22343)
        );
        assert!(normalize_service_address("http://google.com").is_err());
        assert!(normalize_service_address("file://test").is_err());
    }

    #[test]
    fn test_full_hostname() {
        assert_eq!(
            full_hostname("tunnel6694", "zikuosu.ccwu.cc"),
            "tunnel6694.zikuosu.ccwu.cc"
        );
        assert_eq!(
            full_hostname("tunnel6694.zikuosu.ccwu.cc", "zikuosu.ccwu.cc"),
            "tunnel6694.zikuosu.ccwu.cc"
        );
    }

    #[test]
    fn test_generate_config_yaml() {
        let services = vec![
            ServiceRule {
                id: "1".into(),
                name: "API".into(),
                hostname: "tunnel6694".into(),
                service: "http://127.0.0.1:6694".into(),
                enabled: true,
            },
            ServiceRule {
                id: "2".into(),
                name: "Web".into(),
                hostname: "tunnel16888".into(),
                service: "16888".into(),
                enabled: false,
            },
        ];
        let cred_path = Path::new("C:/test/cred.json");

        // active 模式仅包含 enabled: true 的服务
        let active_yaml = generate_config_yaml(
            "test-tunnel-id",
            cred_path,
            "zikuosu.ccwu.cc",
            &services,
            true,
        )
        .unwrap();
        assert!(active_yaml.contains("hostname: tunnel6694.zikuosu.ccwu.cc"));
        assert!(!active_yaml.contains("hostname: tunnel16888.zikuosu.ccwu.cc"));
        assert!(active_yaml.ends_with("- service: http_status:404\n"));

        // all 模式包含全部服务
        let all_yaml = generate_config_yaml(
            "test-tunnel-id",
            cred_path,
            "zikuosu.ccwu.cc",
            &services,
            false,
        )
        .unwrap();
        assert!(all_yaml.contains("hostname: tunnel6694.zikuosu.ccwu.cc"));
        assert!(all_yaml.contains("hostname: tunnel16888.zikuosu.ccwu.cc"));
    }
}
