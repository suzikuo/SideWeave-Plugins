use super::models::{ServiceRule, TunnelRecord};
use std::fs;
use std::path::{Path, PathBuf};
use url::Url;

pub fn plugin_data_dir() -> PathBuf {
    let data_root = if let Some(appdata) = std::env::var_os("APPDATA") {
        PathBuf::from(appdata)
            .join("com.agiletiles.app")
            .join("plugin-data")
    } else {
        PathBuf::from(".agile-data").join("plugin-data")
    };
    let target = data_root.join("dev_connection");
    let legacy = data_root.join("cloudflare_tunnel");
    if !target.join("tunnel.json").exists() && legacy.join("tunnel.json").exists() {
        if fs::create_dir_all(&target).is_ok() {
            for name in [
                "tunnel.json",
                "credentials.json",
                "config.yaml",
                "all_config.yaml",
            ] {
                let source = legacy.join(name);
                let destination = target.join(name);
                if !source.is_file() || destination.exists() {
                    continue;
                }
                if fs::copy(&source, &destination).is_ok() && name.ends_with(".yaml") {
                    if let Ok(contents) = fs::read_to_string(&destination) {
                        let old_credentials = legacy
                            .join("credentials.json")
                            .to_string_lossy()
                            .replace('\\', "/");
                        let new_credentials = target
                            .join("credentials.json")
                            .to_string_lossy()
                            .replace('\\', "/");
                        let updated = contents.replace(&old_credentials, &new_credentials);
                        let _ = fs::write(&destination, updated);
                    }
                }
            }
        }
    }
    target
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

/// 支持本地端口简写，以及本地、局域网或远程 HTTP/HTTPS 源站。
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

    if trimmed.chars().any(char::is_whitespace) || trimmed.contains('\\') {
        return Err("服务地址不能包含空白字符或反斜杠".to_string());
    }
    let address = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let mut url = Url::parse(&address)
        .map_err(|_| "服务地址无效，请输入端口号或完整的 HTTP/HTTPS 地址".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("仅支持 HTTP 或 HTTPS 源站地址".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("源站地址不能包含用户名或密码".to_string());
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err("源站地址只需协议、主机和端口，不能包含路径、查询参数或片段".to_string());
    }
    let port = url.port_or_known_default().ok_or("服务端口无效")?;
    if port == 0 {
        return Err("端口号不能为 0".to_string());
    }
    // 保留旧 HTTP localhost 配置的行为；HTTPS 保留主机名用于证书验证。
    if url.scheme() == "http" && url.host_str() == Some("localhost") {
        url.set_host(Some("127.0.0.1")).map_err(|e| e.to_string())?;
    }
    Ok((url.as_str().trim_end_matches('/').to_string(), port))
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

fn generate_config_yaml(
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
        let host = full_hostname(&s.hostname, domain);
        let (norm_service, _) = normalize_service_address(&s.service)?;
        yaml.push_str(&format!("  - hostname: {host}\n"));
        yaml.push_str(&format!("    service: {norm_service}\n"));
        let origin = Url::parse(&norm_service).map_err(|e| e.to_string())?;
        let remote = !is_loopback_origin(&origin);
        let no_tls_verify = s.no_tls_verify && origin.scheme() == "https";
        if remote || s.disable_chunked_encoding || no_tls_verify {
            yaml.push_str("    originRequest:\n");
            if remote {
                // 远程虚拟主机需要源站 Host，而非 Tunnel 的公网子域名。
                let authority = &origin[url::Position::BeforeHost..url::Position::AfterPort];
                yaml.push_str(&format!(
                    "      httpHostHeader: {}\n",
                    serde_json::to_string(authority).unwrap()
                ));
                if origin.scheme() == "https" {
                    let server_name = match origin.host().unwrap() {
                        url::Host::Domain(domain) => domain.to_string(),
                        url::Host::Ipv4(ip) => ip.to_string(),
                        url::Host::Ipv6(ip) => ip.to_string(),
                    };
                    yaml.push_str(&format!(
                        "      originServerName: {}\n",
                        serde_json::to_string(&server_name).unwrap()
                    ));
                }
            }
            if s.disable_chunked_encoding {
                yaml.push_str("      disableChunkedEncoding: true\n");
            }
            if no_tls_verify {
                yaml.push_str("      noTLSVerify: true\n");
            }
        }
    }

    // 强制追加 404 兜底规则，防止未授权暴露
    yaml.push_str("  - service: http_status:404\n");
    Ok(yaml)
}

pub fn is_loopback_origin(origin: &Url) -> bool {
    match origin.host() {
        Some(url::Host::Domain(host)) => host == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
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
        assert!(normalize_service_address("file://test").is_err());
    }

    #[test]
    fn normalize_remote_origins() {
        for (input, expected, port) in [
            (" https://API.example.com/ ", "https://api.example.com", 443),
            (
                "https://api.example.com:8443",
                "https://api.example.com:8443",
                8443,
            ),
            ("http://192.168.1.10:8080", "http://192.168.1.10:8080", 8080),
            ("api.example.com:8080", "http://api.example.com:8080", 8080),
            ("https://localhost:8443", "https://localhost:8443", 8443),
            ("https://[::1]:8443", "https://[::1]:8443", 8443),
            (":6694", "http://127.0.0.1:6694", 6694),
        ] {
            assert_eq!(
                normalize_service_address(input).unwrap(),
                (expected.into(), port)
            );
            assert_eq!(extract_port(input), port);
        }
    }

    #[test]
    fn reject_invalid_origins() {
        for input in [
            "",
            "0",
            ":0",
            "https://example.com:0",
            "https://example.com:65536",
            "https://",
            "ftp://example.com",
            "file://test",
            "https://user:pass@example.com",
            "https://example.com/api",
            "https://example.com?token=secret",
            "https://example.com#fragment",
            "https://exa mple.com",
            "https://example.com\nignored",
            "https://example.com\\api",
        ] {
            assert!(
                normalize_service_address(input).is_err(),
                "accepted {input:?}"
            );
        }
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
                disable_chunked_encoding: false,
                no_tls_verify: false,
                in_preview: false,
            },
            ServiceRule {
                id: "2".into(),
                name: "Web".into(),
                hostname: "tunnel16888".into(),
                service: "16888".into(),
                enabled: false,
                disable_chunked_encoding: false,
                no_tls_verify: false,
                in_preview: false,
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

    #[test]
    fn remote_yaml_sets_origin_host_and_tls_name() {
        let services = vec![
            ServiceRule {
                hostname: "api".into(),
                service: "https://origin.example.com:8443".into(),
                enabled: true,
                disable_chunked_encoding: true,
                ..Default::default()
            },
            ServiceRule {
                hostname: "local".into(),
                service: "6694".into(),
                enabled: true,
                ..Default::default()
            },
            ServiceRule {
                hostname: "web".into(),
                service: "http://origin.example.com".into(),
                enabled: true,
                ..Default::default()
            },
        ];
        let yaml = generate_config_yaml(
            "test",
            Path::new("cred.json"),
            "example.net",
            &services,
            true,
        )
        .unwrap();
        assert!(yaml.contains("service: https://origin.example.com:8443\n    originRequest:\n      httpHostHeader: \"origin.example.com:8443\"\n      originServerName: \"origin.example.com\"\n      disableChunkedEncoding: true\n"));
        assert!(yaml.contains("service: http://127.0.0.1:6694\n  - hostname:"));
        assert!(yaml.contains("service: http://origin.example.com\n    originRequest:\n      httpHostHeader: \"origin.example.com\"\n  - service: http_status:404\n"));
        assert!(!yaml.contains("noTLSVerify"));
    }

    #[test]
    fn tls_verification_is_opt_in_per_https_origin() {
        let services = vec![
            ServiceRule {
                hostname: "selfsigned".into(),
                service: "https://origin.example.com".into(),
                enabled: true,
                no_tls_verify: true,
                ..Default::default()
            },
            ServiceRule {
                hostname: "trusted".into(),
                service: "https://trusted.example.com".into(),
                enabled: true,
                ..Default::default()
            },
            ServiceRule {
                hostname: "local".into(),
                service: "https://localhost:8443".into(),
                enabled: true,
                no_tls_verify: true,
                ..Default::default()
            },
            ServiceRule {
                hostname: "http".into(),
                service: "http://localhost:8080".into(),
                enabled: true,
                no_tls_verify: true,
                ..Default::default()
            },
        ];
        let yaml = generate_config_yaml(
            "test",
            Path::new("cred.json"),
            "example.net",
            &services,
            true,
        )
        .unwrap();
        let rules: Vec<_> = yaml.split("  - hostname:").skip(1).collect();
        assert!(rules[0]
            .contains("originServerName: \"origin.example.com\"\n      noTLSVerify: true\n"));
        assert!(!rules[1].contains("noTLSVerify"));
        assert!(rules[2].contains(
            "service: https://localhost:8443\n    originRequest:\n      noTLSVerify: true\n"
        ));
        assert!(!rules[3].contains("noTLSVerify"));
    }

    #[test]
    fn old_rules_keep_certificate_verification_and_new_rules_roundtrip() {
        let mut rule: ServiceRule = serde_json::from_value(serde_json::json!({
            "id": "legacy", "name": "API", "hostname": "api", "service": "https://example.com"
        }))
        .unwrap();
        assert!(!rule.no_tls_verify);
        rule.no_tls_verify = true;
        let restored: ServiceRule =
            serde_json::from_str(&serde_json::to_string(&rule).unwrap()).unwrap();
        assert_eq!(restored, rule);
    }
}
