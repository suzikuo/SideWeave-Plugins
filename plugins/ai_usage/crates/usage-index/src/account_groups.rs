use crate::{Index, ORIGIN_SQL};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS source_account(id INTEGER PRIMARY KEY, tool TEXT NOT NULL, origin TEXT NOT NULL, account_id TEXT NOT NULL, UNIQUE(tool,origin,account_id));
CREATE TABLE IF NOT EXISTS account_group(id TEXT PRIMARY KEY, tool TEXT NOT NULL, name TEXT NOT NULL, quota_identity_id INTEGER);
CREATE TABLE IF NOT EXISTS account_group_link(identity_id INTEGER PRIMARY KEY, group_id TEXT NOT NULL);
CREATE VIEW IF NOT EXISTS account_group_member AS SELECT l.group_id,s.* FROM account_group_link l JOIN source_account s ON s.id=l.identity_id;
CREATE INDEX IF NOT EXISTS account_group_link_group ON account_group_link(group_id);
";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceAccount {
    pub id: i64,
    pub tool: String,
    pub origin: String,
    pub account_id: String,
    pub requests: u64,
    pub sessions: u64,
    pub last_used_ms: i64,
    pub group_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountGroup {
    pub id: String,
    pub tool: String,
    pub name: String,
    pub quota_identity_id: Option<i64>,
    pub members: Vec<i64>,
}

impl Index {
    pub fn stored_source_accounts(&self) -> Result<Vec<SourceAccount>, String> {
        let mut stmt=self.conn().prepare("SELECT s.id,s.tool,s.origin,s.account_id,l.group_id FROM source_account s LEFT JOIN account_group_link l ON l.identity_id=s.id ORDER BY s.id").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceAccount {
                    id: r.get(0)?,
                    tool: r.get(1)?,
                    origin: r.get(2)?,
                    account_id: r.get(3)?,
                    group_id: r.get(4)?,
                    requests: 0,
                    sessions: 0,
                    last_used_ms: 0,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn source_accounts(&self) -> Result<Vec<SourceAccount>, String> {
        self.conn().execute(&format!("INSERT OR IGNORE INTO source_account(tool,origin,account_id)
            SELECT DISTINCT e.tool, {ORIGIN_SQL}, extra.account_id FROM event e JOIN event_extra extra ON extra.event_id=e.id
            WHERE NULLIF(TRIM(extra.account_id),'') IS NOT NULL"), []).map_err(|e| e.to_string())?;
        let sql = format!("WITH counts AS (SELECT e.tool,{ORIGIN_SQL} AS origin,extra.account_id,COUNT(*) AS n,COUNT(DISTINCT e.session) AS sessions,MAX(e.ts_ms) AS last_used
            FROM event e JOIN event_extra extra ON extra.event_id=e.id WHERE NULLIF(TRIM(extra.account_id),'') IS NOT NULL GROUP BY e.tool,origin,extra.account_id)
            SELECT s.id,s.tool,s.origin,s.account_id,COALESCE(c.n,0),COALESCE(c.sessions,0),COALESCE(c.last_used,0),l.group_id
            FROM source_account s LEFT JOIN account_group_link l ON l.identity_id=s.id
            LEFT JOIN counts c ON c.tool=s.tool AND c.origin=s.origin AND c.account_id=s.account_id ORDER BY s.tool,s.origin,s.account_id");
        let mut stmt = self.conn().prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceAccount {
                    id: r.get(0)?,
                    tool: r.get(1)?,
                    origin: r.get(2)?,
                    account_id: r.get(3)?,
                    requests: r.get::<_, i64>(4)?.max(0) as u64,
                    sessions: r.get::<_, i64>(5)?.max(0) as u64,
                    last_used_ms: r.get(6)?,
                    group_id: r.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn account_groups(&self) -> Result<Vec<AccountGroup>, String> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id,tool,name,quota_identity_id FROM account_group ORDER BY name,id")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AccountGroup {
                    id: r.get(0)?,
                    tool: r.get(1)?,
                    name: r.get(2)?,
                    quota_identity_id: r.get(3)?,
                    members: Vec::new(),
                })
            })
            .map_err(|e| e.to_string())?;
        let mut groups = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for group in &mut groups {
            let mut stmt=self.conn().prepare("SELECT identity_id FROM account_group_link WHERE group_id=?1 ORDER BY identity_id").map_err(|e|e.to_string())?;
            group.members = stmt
                .query_map([&group.id], |r| r.get(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
        }
        Ok(groups)
    }
    pub fn save_account_group(&self, group: &AccountGroup) -> Result<(), String> {
        let name = group.name.trim();
        if name.is_empty()
            || name.chars().count() > 80
            || group.id.is_empty()
            || group.id.len() > 100
            || group.members.is_empty()
        {
            return Err("请填写名称并选择至少一个已识别账号".into());
        }
        let unique: std::collections::HashSet<_> = group.members.iter().collect();
        if unique.len() != group.members.len() {
            return Err("账号成员不能重复".into());
        }
        if group
            .quota_identity_id
            .is_some_and(|id| !group.members.contains(&id))
        {
            return Err("配额来源必须是当前组成员".into());
        }
        let tx = self
            .conn()
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let old: Option<String> = tx
            .query_row(
                "SELECT tool FROM account_group WHERE id=?1",
                [&group.id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if old.as_deref().is_some_and(|tool| tool != group.tool) {
            return Err("统一账号不能更换工具".into());
        }
        for id in &group.members {
            let tool: Option<String> = tx
                .query_row("SELECT tool FROM source_account WHERE id=?1", [id], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(|e| e.to_string())?;
            if tool.as_deref() != Some(group.tool.as_str()) {
                return Err("成员不存在或属于其他工具".into());
            }
            let owner: Option<String> = tx
                .query_row(
                    "SELECT group_id FROM account_group_link WHERE identity_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if owner.as_deref().is_some_and(|owner| owner != group.id) {
                return Err("该来源账号已加入其他统一账号，请先移除".into());
            }
        }
        tx.execute("INSERT INTO account_group(id,tool,name,quota_identity_id) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET name=excluded.name,quota_identity_id=excluded.quota_identity_id",
            params![group.id,group.tool,name,group.quota_identity_id]).map_err(|e|e.to_string())?;
        tx.execute(
            "DELETE FROM account_group_link WHERE group_id=?1",
            [&group.id],
        )
        .map_err(|e| e.to_string())?;
        for id in &group.members {
            tx.execute(
                "INSERT INTO account_group_link VALUES(?1,?2)",
                params![id, group.id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn delete_account_group(&self, id: &str) -> Result<(), String> {
        let tx = self
            .conn()
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM account_group_link WHERE group_id=?1", [id])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM account_group WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
}

/// All identifiers are internal SQL; the selected group remains a bound parameter.
pub fn group_clause(kind: &str, key: &str, tool: &str, account: &str, origin: &str) -> String {
    format!("({kind}='group' AND EXISTS(SELECT 1 FROM account_group_member gm WHERE gm.group_id={key} AND gm.tool={tool} AND gm.origin=({origin}) AND gm.account_id={account}))")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn add(index: &Index, tool: &str, source: &str, account: Option<&str>, at: i64) {
        index.conn().execute("INSERT INTO event(tool,ts_ms,session,session_title,project,model,meter,in_tok,source) VALUES(?1,?2,'session','Title','/project','gpt-5','tokens',100,?3)",params![tool,at,source]).unwrap();
        let id = index.conn().last_insert_rowid();
        index
            .conn()
            .execute(
                "INSERT INTO event_extra(event_id,account_id) VALUES(?1,?2)",
                params![id, account],
            )
            .unwrap();
    }
    #[test]
    fn grouping_is_source_scoped_reversible_and_preserves_usage() {
        let mut index = Index::open_in_memory().unwrap();
        let now = usage_core::report::now_ms();
        add(&index, "codex", "native", Some("a"), now - 1000);
        add(&index, "codex", "linux:wsl:file", Some("a"), now - 1000);
        add(
            &index,
            "codex",
            "linux:unrelated:file",
            Some("a"),
            now - 1000,
        );
        add(&index, "codex", "native", None, now - 1000);
        add(&index, "claude", "native", Some("a"), now - 1000);
        let identities = index.source_accounts().unwrap();
        assert_eq!(
            identities.len(),
            4,
            "unknown accounts are not selectable identities"
        );
        let members: Vec<_> = identities
            .iter()
            .filter(|i| i.tool == "codex" && i.origin != "unrelated")
            .map(|i| i.id)
            .collect();
        let mut group = AccountGroup {
            id: "group-a".into(),
            tool: "codex".into(),
            name: "Unified".into(),
            quota_identity_id: Some(members[0]),
            members,
        };
        index.save_account_group(&group).unwrap();
        let plan = usage_core::AggregatePlan::build(now, 20);
        let prices = usage_core::PricingMap::default();
        let mut options = usage_core::ReportOptions::new(&prices);
        options.now_ms = now;
        let facts = index
            .report_facts_for_account(
                &plan,
                &usage_core::MachineScope::All,
                &crate::AccountFilter::Group(group.id.clone()),
            )
            .unwrap();
        let report = usage_core::summarize_facts(&facts, &options);
        assert_eq!(report.day.summary.requests, 2);
        assert_eq!(report.recent_sessions.len(), 1);
        assert_eq!(
            report.recent_sessions[0].project.as_deref(),
            Some("/project")
        );
        group.members.pop();
        group.quota_identity_id = None;
        index.save_account_group(&group).unwrap();
        let facts = index
            .report_facts_for_account(
                &plan,
                &usage_core::MachineScope::All,
                &crate::AccountFilter::Group(group.id.clone()),
            )
            .unwrap();
        assert_eq!(
            usage_core::summarize_facts(&facts, &options)
                .day
                .summary
                .requests,
            1
        );
        index.delete_account_group(&group.id).unwrap();
        let facts = index
            .report_facts_for_account(
                &plan,
                &usage_core::MachineScope::All,
                &crate::AccountFilter::Group(group.id.clone()),
            )
            .unwrap();
        assert_eq!(
            usage_core::summarize_facts(&facts, &options)
                .day
                .summary
                .requests,
            0
        );
        assert_eq!(index.event_count().unwrap(), 5);
        assert_eq!(
            index
                .conn()
                .query_row("SELECT SUM(in_tok) FROM event", [], |r| r.get::<_, f64>(0))
                .unwrap(),
            500.0
        );
    }
    #[test]
    fn invalid_members_and_conflicting_groups_roll_back_atomically_and_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        {
            let index = Index::open(&path).unwrap();
            add(&index, "codex", "local", Some("a"), 1);
            add(&index, "claude", "local", Some("b"), 1);
            let identities = index.source_accounts().unwrap();
            let codex = identities.iter().find(|i| i.tool == "codex").unwrap().id;
            let claude = identities.iter().find(|i| i.tool == "claude").unwrap().id;
            let group = AccountGroup {
                id: "a".into(),
                tool: "codex".into(),
                name: "Name".into(),
                quota_identity_id: Some(codex),
                members: vec![codex],
            };
            index.save_account_group(&group).unwrap();
            for bad in [
                AccountGroup {
                    id: "b".into(),
                    ..group.clone()
                },
                AccountGroup {
                    members: vec![codex, claude],
                    ..group.clone()
                },
                AccountGroup {
                    members: vec![codex, codex],
                    ..group.clone()
                },
                AccountGroup {
                    quota_identity_id: Some(999),
                    ..group.clone()
                },
                AccountGroup {
                    members: vec![999],
                    quota_identity_id: None,
                    ..group.clone()
                },
            ] {
                assert!(index.save_account_group(&bad).is_err());
                assert_eq!(index.account_groups().unwrap()[0].members, vec![codex]);
            }
        }
        let index = Index::open(&path).unwrap();
        assert_eq!(index.account_groups().unwrap()[0].name, "Name");
        assert_eq!(index.event_count().unwrap(), 2);
    }
}
