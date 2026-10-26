use rusqlite::params;
use serde::Serialize;
use usage_core::{PricingMap, QuotaOrigin, QuotaView, TokenCounts};
use usage_index::{AccountFilter, Index};

#[derive(Clone, Debug, Serialize)]
pub(super) struct Estimate {
    pub key: String,
    pub used_cost_usd: Option<f64>,
    pub total_cost_usd: Option<f64>,
    pub remaining_cost_usd: Option<f64>,
    pub reason: Option<String>,
}

impl Estimate {
    fn unavailable(quota: &QuotaView, reason: &str) -> Self {
        Self { key: super::quota_notification_identity(quota), used_cost_usd: None,
            total_cost_usd: None, remaining_cost_usd: None, reason: Some(reason.into()) }
    }
}

pub(super) fn collect(index: &Index, quotas: &[QuotaView], selection: &str, prices: &PricingMap) -> Vec<Estimate> {
    let active_codex = if quotas.iter().any(|q| q.tool == "codex") { usage_quota::codex_account_id() } else { None };
    let accounts = if selection == "all" && quotas.iter().any(|q| q.tool == "claude") { index.accounts().ok() } else { None };
    quotas.iter().map(|quota| {
        if quota.origin != QuotaOrigin::Probe || !matches!(quota.window_minutes, 300 | 10_080) {
            return Estimate::unavailable(quota, "仅估算实时订阅的 5 小时／周额度窗口");
        }
        if !matches!(quota.tool.as_str(), "codex" | "claude") {
            return Estimate::unavailable(quota, "此工具的额度与模型费用暂无法可靠对应");
        }
        let account = match AccountFilter::from_selection(selection) {
            AccountFilter::Account { tool, id } if tool == quota.tool => Some(id),
            AccountFilter::All if quota.tool == "codex" => active_codex.clone(),
            AccountFilter::All => accounts.as_ref().and_then(|accounts| {
                let mut matches = accounts.iter().filter(|a| a.tool == quota.tool);
                let first = matches.next()?;
                if matches.next().is_some() { None } else { Some(first.account_id.clone().unwrap_or_default()) }
            }),
            AccountFilter::Unknown(tool) if tool == quota.tool => Some(String::new()),
            _ => None,
        };
        if quota.tool == "codex" {
            let bound = account.as_deref().map(usage_quota::cached_codex_for_account).unwrap_or_default();
            if !bound.iter().any(|row| row.window_minutes == quota.window_minutes
                && row.resets_at_ms == quota.resets_at_ms && row.used_percent == quota.used_percent
                && row.sampled_at_ms.abs_diff(quota.sampled_at_ms) <= 1_000) {
                return Estimate::unavailable(quota, "请刷新当前 Codex 账号订阅后再估算");
            }
        }
        match account {
            Some(account) => calculate(index, quota, &account, prices, usage_core::report::now_ms())
                .unwrap_or_else(|_| Estimate::unavailable(quota, "暂时无法读取窗口内费用")),
            None => Estimate::unavailable(quota, "无法匹配订阅账号，暂不估算"),
        }
    }).collect()
}

fn calculate(index: &Index, quota: &QuotaView, account: &str, prices: &PricingMap, now: i64) -> Result<Estimate, String> {
    let unavailable = |reason| Ok(Estimate::unavailable(quota, reason));
    if quota.origin != QuotaOrigin::Probe || !matches!(quota.window_minutes, 300 | 10_080) {
        return unavailable("仅估算实时订阅的 5 小时／周额度窗口");
    }
    if !matches!(quota.tool.as_str(), "codex" | "claude") {
        return unavailable("此工具的额度与模型费用暂无法可靠对应");
    }
    if quota.resets_at_ms <= now || quota.sampled_at_ms <= 0 {
        return unavailable("缺少有效的窗口重置时间或额度采样");
    }
    if !quota.used_percent.is_finite() || quota.used_percent < 1.0 || quota.used_percent > 100.0 {
        return unavailable("已用额度不足 1%，暂不外推满额金额");
    }
    let start = quota.resets_at_ms.saturating_sub(quota.window_minutes.saturating_mul(60_000));
    let until = quota.sampled_at_ms.min(now);
    if until <= start { return unavailable("窗口尚未开始或采样时间不匹配") }
    let label = quota.label.as_deref().unwrap_or_default().to_ascii_lowercase();
    let family = if quota.tool == "claude" && label.contains("sonnet") { "sonnet" }
        else if quota.tool == "claude" && label.contains("opus") { "opus" } else { "" };
    let mut statement = index.conn().prepare(
        "SELECT e.model, COUNT(*), COALESCE(SUM(e.in_tok),0), COALESCE(SUM(e.cc_tok),0), \
                COALESCE(SUM(e.cr_tok),0), COALESCE(SUM(e.out_tok),0) \
         FROM event e LEFT JOIN event_extra extra ON extra.event_id = e.id \
         WHERE e.tool=?1 AND e.ts_ms>=?2 AND e.ts_ms<=?3 AND e.source NOT LIKE 'linux:%' \
           AND COALESCE(extra.account_id,'')=?4 \
           AND (?5='' OR LOWER(COALESCE(e.model,'')) LIKE '%' || ?5 || '%') \
         GROUP BY e.model"
    ).map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![quota.tool, start, until, account, family], |row| Ok((
        row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?,
        TokenCounts { input: row.get(2)?, cache_creation: row.get(3)?, cache_read: row.get(4)?,
            output: row.get(5)?, ..Default::default() }
    ))).map_err(|error| error.to_string())?;
    let mut used_cost = 0.0;
    let mut requests = 0;
    for row in rows {
        let (model, count, tokens) = row.map_err(|error| error.to_string())?;
        requests += count;
        if tokens.total() <= 0.0 { continue }
        let Some(cost) = prices.cost(model.as_deref(), &tokens) else {
            return unavailable("窗口内有模型缺少价格，暂不估算满额金额");
        };
        used_cost += cost;
    }
    if requests == 0 || used_cost <= 0.0 || !used_cost.is_finite() {
        return unavailable("此账号在该窗口内暂无可定价的本地用量");
    }
    let total = used_cost / (quota.used_percent / 100.0);
    Ok(Estimate { key: super::quota_notification_identity(quota), used_cost_usd: Some(used_cost),
        total_cost_usd: Some(total), remaining_cost_usd: Some((total - used_cost).max(0.0)), reason: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    const NOW: i64 = 1_790_000_000_000;

    fn quota(minutes: i64) -> QuotaView {
        QuotaView { tool: "codex".into(), used_percent: 25.0, window_minutes: minutes,
            resets_at_ms: NOW + 60_000, sampled_at_ms: NOW, label: None, id: None, origin: QuotaOrigin::Probe }
    }

    fn insert(index: &Index, at: i64, account: &str, source: &str, model: &str, tokens: f64) {
        index.conn().execute("INSERT INTO event(tool,ts_ms,session,model,meter,in_tok,source) VALUES('codex',?1,'s',?2,'tokens',?3,?4)",
            params![at, model, tokens, source]).unwrap();
        let id = index.conn().last_insert_rowid();
        index.conn().execute("INSERT INTO event_extra(event_id,account_id) VALUES(?1,?2)", params![id, account]).unwrap();
    }

    #[test]
    fn window_estimates_exclude_other_accounts_remote_rows_and_post_sample_usage() {
        let index = Index::open_in_memory().unwrap();
        let prices = PricingMap::default();
        insert(&index, NOW - 60_000, "a", "local", "gpt-5", 1_000_000.0);
        insert(&index, NOW - 6 * 3_600_000, "a", "local", "gpt-5", 2_000_000.0);
        insert(&index, NOW - 1_000, "b", "local", "gpt-5", 9_000_000.0);
        insert(&index, NOW - 1_000, "a", "linux:remote:file", "gpt-5", 9_000_000.0);
        insert(&index, NOW + 1, "a", "local", "gpt-5", 9_000_000.0);
        let five = calculate(&index, &quota(300), "a", &prices, NOW).unwrap();
        let week = calculate(&index, &quota(10_080), "a", &prices, NOW).unwrap();
        let cost = prices.cost(Some("gpt-5"), &TokenCounts { input: 1_000_000.0, ..Default::default() }).unwrap();
        assert_eq!(five.used_cost_usd, Some(cost));
        assert_eq!(five.total_cost_usd, Some(cost * 4.0));
        assert_eq!(five.remaining_cost_usd, Some(cost * 3.0));
        assert_eq!(week.total_cost_usd, Some(cost * 12.0));
    }

    #[test]
    fn missing_prices_or_near_zero_usage_do_not_produce_misleading_totals() {
        let index = Index::open_in_memory().unwrap();
        insert(&index, NOW - 1, "a", "local", "unknown-missing-price", 100.0);
        let prices = PricingMap::default();
        assert!(calculate(&index, &quota(300), "a", &prices, NOW).unwrap().total_cost_usd.is_none());
        let mut q = quota(300);
        q.used_percent = 0.0;
        assert!(calculate(&index, &q, "a", &prices, NOW).unwrap().reason.unwrap().contains("1%"));
        q.resets_at_ms = 0;
        assert!(calculate(&index, &q, "a", &prices, NOW).unwrap().total_cost_usd.is_none());
    }
}
