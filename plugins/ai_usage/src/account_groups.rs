use serde_json::{json, Value};
use std::path::PathBuf;
use usage_index::{
    account_groups::{AccountGroup, SourceAccount},
    Index,
};

pub(super) fn home(index: &Index, identity: &SourceAccount) -> Option<PathBuf> {
    if identity.tool != "codex" {
        return None;
    }
    if identity.origin.is_empty() {
        return std::env::var_os("CODEX_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| usage_core::home_dir().map(|h| h.join(".codex")));
    }
    super::read_data_roots(index)
        .into_iter()
        .find(|root| root.id == identity.origin)
        .map(|root| PathBuf::from(root.path).join(".codex"))
}

pub(super) fn get(index: &Index) -> Result<Value, String> {
    let names = super::read_account_names();
    let identities = index
        .source_accounts()?
        .into_iter()
        .map(|identity| {
            let path = home(index, &identity);
            let available = path.as_ref().is_some_and(|p| {
                usage_quota::codex_account_at(p).as_deref() == Some(identity.account_id.as_str())
            });
            let source_label = if identity.origin.is_empty() {
                "Windows 本机".to_string()
            } else {
                super::read_data_roots(index)
                    .iter()
                    .find(|root| root.id == identity.origin)
                    .map(|root| format!("WSL / 外部目录 · {}", root.path))
                    .unwrap_or_else(|| format!("其他机器 · {}", identity.origin))
            };
            let name = names
                .get(&super::make_account_selection(
                    &identity.tool,
                    Some(&identity.account_id),
                ))
                .or_else(|| names.get(&identity.account_id))
                .cloned();
            let mut value = serde_json::to_value(identity).unwrap();
            value["source_label"] = json!(source_label);
            value["name"] = json!(name);
            value["quota_available"] = json!(available);
            value
        })
        .collect::<Vec<_>>();
    let mut stmt=index.conn().prepare("SELECT e.tool,COUNT(*) FROM event e LEFT JOIN event_extra x ON x.event_id=e.id WHERE NULLIF(TRIM(x.account_id),'') IS NULL GROUP BY e.tool").map_err(|e|e.to_string())?;
    let unknown = stmt
        .query_map([], |r| {
            Ok(json!({"tool":r.get::<_,String>(0)?,"requests":r.get::<_,i64>(1)?}))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(json!({"identities":identities,"groups":index.account_groups()?,"unknown":unknown}))
}

pub(super) fn save(payload: &Value) -> Result<Value, String> {
    let index = super::open_index()?;
    index.source_accounts()?;
    let mut value = payload.clone();
    if value
        .get("id")
        .and_then(Value::as_str)
        .is_none_or(|id| id.is_empty())
    {
        value["id"] = json!(format!(
            "g-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        ));
    }
    let group: AccountGroup =
        serde_json::from_value(value).map_err(|e| format!("账号关联参数无效：{e}"))?;
    index.save_account_group(&group)?;
    get(&index)
}
pub(super) fn delete(payload: &Value) -> Result<Value, String> {
    let index = super::open_index()?;
    index.delete_account_group(
        payload
            .get("id")
            .and_then(Value::as_str)
            .ok_or("缺少统一账号 ID")?,
    )?;
    get(&index)
}

pub(super) fn selected(
    index: &Index,
    selection: &str,
    tool: &str,
) -> Option<(AccountGroup, SourceAccount)> {
    let groups = index.account_groups().ok()?;
    let identities = index.stored_source_accounts().ok()?;
    if let Some(id) = selection.strip_prefix("group:") {
        let group = groups.into_iter().find(|g| g.id == id && g.tool == tool)?;
        let identity = identities
            .into_iter()
            .find(|i| Some(i.id) == group.quota_identity_id)?;
        return Some((group, identity));
    }
    if selection != "all" || tool != "codex" {
        return None;
    }
    let active = usage_quota::codex_account_id();
    let mut matches = groups.into_iter().filter_map(|g| {
        let identity = identities
            .iter()
            .find(|i| Some(i.id) == g.quota_identity_id && i.tool == tool)?
            .clone();
        if let Some(active) = active.as_deref() {
            if !identities
                .iter()
                .any(|i| g.members.contains(&i.id) && i.account_id == active && i.tool == tool)
            {
                return None;
            }
        } else {
            let home = home(index, &identity)?;
            if usage_quota::codex_account_at(&home).as_deref() != Some(identity.account_id.as_str())
            {
                return None;
            }
        }
        Some((g, identity))
    });
    let first = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(first)
    }
}

pub(super) fn quotas(
    index: &Index,
    selection: &str,
    refresh: bool,
    force: bool,
) -> Option<Vec<usage_core::QuotaView>> {
    if selection.starts_with("group:")
        || (selection == "all" && selected(index, selection, "codex").is_some())
    {
        let Some((_, identity)) = selected(index, selection, "codex") else {
            return Some(Vec::new());
        };
        let Some(home) = home(index, &identity) else {
            return Some(Vec::new());
        };
        let mut rows = if selection == "all" {
            if refresh {
                if force {
                    usage_quota::refresh_within(std::time::Duration::from_secs(2))
                } else {
                    usage_quota::collect_within(std::time::Duration::from_secs(2))
                }
            } else {
                usage_quota::cached()
            }
        } else {
            Vec::new()
        };
        rows.retain(|q| q.tool != "codex");
        rows.extend(usage_quota::codex_source_quotas(
            &home,
            &identity.account_id,
            refresh,
            force,
            std::time::Duration::from_secs(2),
        ));
        return Some(rows);
    }
    None
}

pub(super) fn refresh_background() {
    let Ok(index) = super::open_index() else {
        return;
    };
    let Ok(groups) = index.account_groups() else {
        return;
    };
    if groups.is_empty() {
        return;
    }
    let Ok(identities) = index.stored_source_accounts() else {
        return;
    };
    for group in groups {
        if let Some(identity) = identities
            .iter()
            .find(|i| Some(i.id) == group.quota_identity_id)
        {
            if let Some(home) = home(&index, identity) {
                let _ = usage_quota::codex_source_quotas(
                    &home,
                    &identity.account_id,
                    true,
                    true,
                    std::time::Duration::ZERO,
                );
            }
        }
    }
}
