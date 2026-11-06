//! 连接目录自动回填（决定三）：把 ZCode `provider_config.json` 里的 `api.baseUrl` 与模型
//! 清单补进**已存连接**，让归因（[`crate::attribution`]）能落到具体连接名。
//!
//! 为什么需要它：归因靠「端点事实」认连接——provider 规则的 baseUrl 命中连接的 `base_url`，
//! 或模型唯一落在连接的 `models_json` 里。但已存连接往往既没 `base_url` 也没有模型清单
//! （本机 `monitor:native:zcode:main` 就是这样），于是 `new-provider` / `zai-api-2` /
//! `deepseek` 这些 provider 永远只能显示平台名。
//!
//! ## 触发时机：**ZCode 采集成功后**（不是每帧轮询）
//!
//! 两个既有路径里选了采集：`native_sources::scan_sources` 在 ZCode 库采集之后调用
//! [`backfill_after_zcode_scan`]。
//!
//! - 采集是离散事件（启动一轮 + 文件监听变化驱动），天然低频，不新增定时器；
//! - 另一条候选路径「连接目录读取时发现缺字段就补」在渲染热路径上：灵动岛收缩态每帧都
//!   读连接目录 / 实时快照，在那里写库等于把写操作塞进读路径，还会让归因缓存
//!   （`attribution` 的连接签名整表作废）每次自愈都抖一次，正是要避免的「每帧轮询」；
//! - ZCode 刚采集完时，`provider_config.json` 与用量记录都在「刚变过」的语境里，
//!   在同一轮里补齐最省事，也最容易对上账。
//!
//! ## 三条硬约束
//!
//! 1. **只补不改**：`base_url` 已有值就不覆盖；模型清单只做并集（去重，保留已有顺序）。
//! 2. **只在能唯一确定时写**：provider → 连接必须是 1:1。同一平台有多条候选连接不写；
//!    反过来同一个连接被多个 provider 同时指向也不写——那种情况下无论写谁的地址都是猜，
//!    写进去等于给归因制造「看起来唯一」的假结论（本机 `new-provider` 三兄弟共用同一
//!    baseUrl，正是这种情况，所以它们仍落平台）。
//! 3. **不碰凭据**：`provider_config.json` 的 `apiKey` 一律不读不存；`base_url` 规范化时
//!    丢掉 `user:pass@`、查询串与片段（见 [`crate::attribution::sanitize_base_url`]）；
//!    日志、诊断与导出里不出现地址原文，只出现 providerId、连接 id 与计数。
//!
//! ## 可追溯 / 可回滚 / 不覆盖用户改动
//!
//! 写进 `connections.catalog_auto`（JSON，纯增列、可空）：哪些字段是自动补的
//! （`baseUrlSource` / `modelsSource` = `auto:zcode`）、补进去的地址规范化键、自动加进去的
//! 模型（`autoModels`）、以及用户删掉过、不许再补回来的模型（`revokedModels`）。
//!
//! - 用户手改了 `base_url`：下一轮发现当前值与该键不一致 → 来源标记作废，之后不再声称是
//!   自动补的（值本身照旧不动，因为「已有值不覆盖」）。
//! - 用户从 `models_json` 删掉了自动补的模型：记进 `revokedModels`，不补回去（不跟用户抢）。
//! - 回滚：见 [`rollback`]——清空自动补的 `base_url`、从 `models_json` 移除 `autoModels`，
//!   并置 `disabled`，这一轮之后该连接不再被自动回填。

use crate::attribution::{normalize_base_url, normalize_model, sanitize_base_url, ProviderRule, ProviderRules};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// 自动回填的来源标记：只标识「这个字段是自动补的」，不含地址与凭据。
const SOURCE_ZCODE: &str = "auto:zcode";

/// ZCode 的 provider 规则只描述 ZCode 自己打哪个端点，所以只有 `zcode` 平台的连接可能是
/// 它的目标。别的平台的连接（Claude / Codex / 自定义中转）不参与回填。
const ZCODE_PLATFORM: &str = "zcode";

/// `connections.catalog_auto` 的内容。字段名用 camelCase 落 JSON（与前端 / 文档一致）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct AutoCatalog {
    /// `base_url` 的写入者；`auto:zcode` 表示本模块补的，缺省表示用户自己设的
    pub base_url_source: Option<String>,
    /// 补进去的地址的规范化键（host + 端口 + 路径，**不含凭据**），用来判断之后是否被改过
    pub base_url_normalized: Option<String>,
    /// `models_json` 的写入者；同上
    pub models_source: Option<String>,
    /// 本模块加进 `models_json` 的模型（回滚时按这个移除）
    pub auto_models: Vec<String>,
    /// 用户手动删掉过的自动模型：不再补回去
    pub revoked_models: Vec<String>,
    /// 贡献过数据的 providerId（不含地址），用于追溯
    pub providers: Vec<String>,
    pub filled_ms: i64,
    /// 回滚后置位：本连接不再自动回填
    pub disabled: bool,
}

/// 一次回填的报告。只含 id / 原因 / 计数，不含地址与凭据，可以安全地打印或塞进诊断。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub filled: Vec<Fill>,
    pub skipped: Vec<Skip>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fill {
    pub connection_id: String,
    pub provider_id: String,
    /// 本次是否补了 `base_url`
    pub base_url: bool,
    /// 本次新增了几个模型
    pub models_added: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Skip {
    pub provider_id: String,
    /// 机器可读的原因码，见 `reason::*`
    pub reason: &'static str,
}

pub(crate) mod reason {
    /// 该平台下没有可对应的连接
    pub const NO_CANDIDATE: &str = "no-candidate";
    /// 该 provider 能对应到多条连接
    pub const MULTI_CANDIDATE: &str = "multi-candidate";
    /// 这个连接被多个 provider 同时指向：写谁的地址都是猜
    pub const CONTENDED: &str = "contended";
    /// 已经有值、也没有可补的空缺
    pub const NOTHING_TO_FILL: &str = "nothing-to-fill";
    /// 用户回滚过：本连接不再自动回填
    pub const DISABLED: &str = "disabled";
    /// `models_json` 读不出来（不是 JSON 数组）：不覆盖，留给人看
    pub const MODELS_UNREADABLE: &str = "models-unreadable";
    /// 写回时发现连接已被并发改过（乐观锁失败）
    pub const STALE: &str = "stale";
}

impl Report {
    pub(crate) fn is_empty(&self) -> bool {
        self.filled.is_empty() && self.skipped.is_empty()
    }

    pub(crate) fn filled_models(&self) -> usize {
        self.filled.iter().map(|fill| fill.models_added).sum()
    }
}

/// 已存连接的一行（只读需要的列，绝不读 `secret`）。
struct Row {
    id: String,
    base_url: Option<String>,
    models_json: Option<String>,
    auto: AutoCatalog,
}

fn parse_auto(raw: Option<&str>) -> AutoCatalog {
    raw.and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default()
}

/// `models_json` 解析；`Some(..)` 但解析不出来时返回 `Err`（调用方据此不覆盖）。
fn models_of(raw: Option<&str>) -> Result<Vec<String>, &'static str> {
    match raw.map(str::trim).filter(|text| !text.is_empty()) {
        None => Ok(Vec::new()),
        Some(text) => serde_json::from_str::<Vec<String>>(text).map_err(|_| reason::MODELS_UNREADABLE),
    }
}

fn zcode_rows(conn: &Connection) -> rusqlite::Result<Vec<Row>> {
    let mut stmt = conn.prepare(
        "SELECT id, base_url, models_json, catalog_auto FROM connections WHERE platform = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map([ZCODE_PLATFORM], |r| {
            let raw: Option<String> = r.get(3)?;
            Ok(Row { id: r.get(0)?, base_url: r.get(1)?, models_json: r.get(2)?, auto: parse_auto(raw.as_deref()) })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 每次回填前先对账：自动补进去的 `base_url` 若已经不是当初那个值（用户改过或被清空），
/// 就把来源标记作废——之后不能再声称这个字段是自动补的。值本身一律不动（只补不改）。
///
/// 对账独立于「候选」判断：连接被用户改成别的端点后就不再是任何 provider 的候选，
/// 若把这一步塞进回填分支里，那条连接会一直挂着一个撒谎的 `baseUrlSource`。
fn reconcile(conn: &Connection, rows: &mut [Row]) {
    for row in rows.iter_mut() {
        if row.auto.base_url_source.as_deref() != Some(SOURCE_ZCODE) {
            continue;
        }
        let still_ours = match row.base_url.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
            Some(current) => normalize_base_url(current).as_deref() == row.auto.base_url_normalized.as_deref(),
            None => false,
        };
        if still_ours {
            continue;
        }
        row.auto.base_url_source = None;
        row.auto.base_url_normalized = None;
        row.auto.filled_ms = chrono::Utc::now().timestamp_millis();
        let Ok(auto_json) = serde_json::to_string(&row.auto) else { continue };
        let _ = conn.execute(
            "UPDATE connections SET catalog_auto = ?2 WHERE id = ?1 AND (base_url IS ?3)",
            params![row.id, auto_json, row.base_url],
        );
    }
}

/// 回填的唯一入口（纯库操作，规则由调用方给，便于测试用合成规则）。
///
/// 返回值同时写进进程内诊断槽（[`last_report`]）；调用方不需要它时可以直接忽略。
pub(crate) fn apply(conn: &Connection, rules: &ProviderRules) -> Report {
    let mut report = Report::default();
    if rules.is_empty() {
        return report;
    }
    // 连接读不出来就什么都不做：宁可这轮不补，也不在半截状态下写。
    let Ok(mut rows) = zcode_rows(conn) else { return report };
    if rows.is_empty() {
        // 该平台一条连接都没有：全部按「没有候选」记账，不写任何东西。
        // 不提前 return，是为了让诊断里说得清「为什么这条 provider 没归到连接」。
        let mut ordered: Vec<&String> = rules.keys().collect();
        ordered.sort();
        report.skipped = ordered
            .into_iter()
            .map(|provider_id| Skip { provider_id: provider_id.clone(), reason: reason::NO_CANDIDATE })
            .collect();
        return report;
    }
    // 先对账：自动补的 base_url 若已被人改过，来源标记作废（值本身不动）。
    reconcile(conn, &mut rows);

    // HashMap 迭代顺序不稳定：先按 providerId 排序，报告与写入顺序才是可复现的。
    let mut ordered: Vec<(&String, &ProviderRule)> = rules.iter().collect();
    ordered.sort_by(|left, right| left.0.cmp(right.0));

    // 第一遍：provider → 候选连接（0 / 1 / 多条）。
    let mut plans: Vec<(usize, &str)> = Vec::new();
    for (provider_id, rule) in ordered {
        let want = rule.base_url.as_deref().and_then(normalize_base_url);
        let candidates: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match (row.base_url.as_deref().and_then(normalize_base_url), want.as_deref()) {
                // 连接已经指向别的端点：不可能是这个 provider
                (Some(current), Some(want)) => current == want,
                // 连接没写地址，或规则本身没给地址：都可能，靠下面的唯一性兜住
                _ => true,
            })
            .map(|(index, _)| index)
            .collect();
        match candidates.as_slice() {
            [] => report.skipped.push(Skip { provider_id: provider_id.clone(), reason: reason::NO_CANDIDATE }),
            [index] => plans.push((*index, provider_id.as_str())),
            _ => report.skipped.push(Skip { provider_id: provider_id.clone(), reason: reason::MULTI_CANDIDATE }),
        }
    }

    // 第二遍：同一个连接被多个 provider 指向 → 全部不写（1:1 不成立）。
    let mut claim: std::collections::BTreeMap<usize, Vec<&str>> = std::collections::BTreeMap::new();
    for (index, provider_id) in &plans {
        claim.entry(*index).or_default().push(provider_id);
    }
    let contended: HashSet<usize> =
        claim.iter().filter(|(_, providers)| providers.len() > 1).map(|(index, _)| *index).collect();
    for index in &contended {
        for provider_id in &claim[index] {
            report.skipped.push(Skip { provider_id: (*provider_id).to_string(), reason: reason::CONTENDED });
        }
    }
    plans.retain(|(index, _)| !contended.contains(index));

    // 第三遍：真正写入。
    for (index, provider_id) in plans {
        let row = &rows[index];
        let Some(rule) = rules.get(provider_id) else { continue };
        match fill_one(conn, row, provider_id, rule) {
            Ok(fill) => report.filled.push(fill),
            Err(reason) => report.skipped.push(Skip { provider_id: provider_id.to_string(), reason }),
        }
    }
    report
}

/// 单条连接的回填：只补空、只并集，写完用乐观锁确认没人并发改过。
fn fill_one(conn: &Connection, row: &Row, provider_id: &str, rule: &ProviderRule) -> Result<Fill, &'static str> {
    if row.auto.disabled {
        return Err(reason::DISABLED);
    }
    let mut models = models_of(row.models_json.as_deref())?;
    let mut auto = row.auto.clone();
    let mut changed = false;

    // 1) base_url：只在空的时候补；已有值一律不动（来源对账见 `reconcile`）。
    let mut base_write: Option<(String, String)> = None;
    if let Some((shown, normalized)) = rule.base_url.as_deref().and_then(sanitize_base_url) {
        if row.base_url.as_deref().map(str::trim).filter(|value| !value.is_empty()).is_none() {
            base_write = Some((shown, normalized));
        }
    }
    if let Some((_, normalized)) = &base_write {
        auto.base_url_source = Some(SOURCE_ZCODE.to_string());
        auto.base_url_normalized = Some(normalized.clone());
        changed = true;
    }

    // 2) 模型清单：并集 + 去重。用户删过的自动模型记进 revokedModels，不再补回来。
    let existing: HashSet<String> = models.iter().map(|model| normalize_model(model)).collect();
    let mut revoked = auto.revoked_models.clone();
    for model in &auto.auto_models {
        let key = normalize_model(model);
        if !existing.contains(&key) && !revoked.iter().any(|item| normalize_model(item) == key) {
            revoked.push(model.clone());
            changed = true;
        }
    }
    let mut survivors: Vec<String> =
        auto.auto_models.iter().filter(|model| existing.contains(&normalize_model(model))).cloned().collect();
    let mut added = 0;
    for model in &rule.models {
        let key = normalize_model(model);
        if existing.contains(&key) || revoked.iter().any(|item| normalize_model(item) == key) {
            continue;
        }
        models.push(model.clone());
        survivors.push(model.clone());
        added += 1;
    }
    if added > 0 {
        auto.models_source = Some(SOURCE_ZCODE.to_string());
        changed = true;
    }
    auto.revoked_models = revoked;
    auto.auto_models = survivors;
    if !changed {
        return Err(reason::NOTHING_TO_FILL);
    }
    if !auto.providers.iter().any(|item| item == provider_id) {
        auto.providers.push(provider_id.to_string());
    }

    auto.filled_ms = chrono::Utc::now().timestamp_millis();
    let models_json = serde_json::to_string(&models).map_err(|_| reason::MODELS_UNREADABLE)?;
    let auto_json = serde_json::to_string(&auto).map_err(|_| reason::MODELS_UNREADABLE)?;
    let base_url = match &base_write {
        Some((shown, _)) => Some(shown.clone()),
        None => row.base_url.clone(),
    };
    // 乐观锁：base_url / models_json 任一被并发改过就不写（`IS` 对 NULL 也成立）。
    let changed_rows = conn
        .execute(
            "UPDATE connections SET base_url = ?2, models_json = ?3, catalog_auto = ?4
             WHERE id = ?1 AND (base_url IS ?5) AND (models_json IS ?6)",
            params![row.id, base_url, models_json, auto_json, row.base_url, row.models_json],
        )
        .map_err(|_| reason::STALE)?;
    if changed_rows == 0 {
        return Err(reason::STALE);
    }
    Ok(Fill {
        connection_id: row.id.clone(),
        provider_id: provider_id.to_string(),
        base_url: base_write.is_some(),
        models_added: added,
    })
}

/// 采集成功后的入口（`native_sources::scan_sources` 在 ZCode 库采集之后调用）。
///
/// 隔离验收模式（`validation::isolated()`）下不读真实 ZCode 配置、不写连接：
/// [`crate::attribution::provider_rules`] 本身在隔离模式下就是空表，这里再显式挡一层。
pub(crate) fn backfill_after_zcode_scan(conn: &Connection) -> Report {
    if crate::validation::isolated() {
        return Report::default();
    }
    let rules = crate::attribution::provider_rules();
    let report = apply(conn, &rules);
    if !report.is_empty() {
        // 只报数量与 id，不打印地址：地址可能夹带中转凭据，日志里不出原文。
        let connections: Vec<&str> = report.filled.iter().map(|fill| fill.connection_id.as_str()).collect();
        eprintln!(
            "[连接目录] ZCode 自动回填 {} 条连接（新增模型 {} 个）：{}；跳过 {} 个 provider",
            report.filled.len(),
            report.filled_models(),
            connections.join(", "),
            report.skipped.len()
        );
    }
    remember(&report);
    report
}

/// 进程内诊断槽：最近一次回填的报告（不含地址与凭据），测试与诊断命令可读。
#[allow(dead_code)] // 诊断入口：单元测试读取；后续可挂到诊断命令上
pub(crate) fn last_report() -> Option<Report> {
    slot().lock().ok().and_then(|guard| guard.clone())
}

fn slot() -> &'static Mutex<Option<Report>> {
    static SLOT: OnceLock<Mutex<Option<Report>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn remember(report: &Report) {
    if let Ok(mut guard) = slot().lock() {
        *guard = Some(report.clone());
    }
}

/// 回滚一条连接的自动回填：清空自动补的 `base_url`、从模型清单里移除自动补的模型，
/// 并把该连接标记为 `disabled`（之后的采集不再补回来）。
///
/// 返回一句给人看的说明；不含地址原文。
#[allow(dead_code)] // 回滚入口：单元测试覆盖；用户界面接上之前先留在这里
pub(crate) fn rollback(conn: &Connection, id: &str) -> Result<String, String> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> = conn
        .query_row("SELECT base_url, models_json, catalog_auto FROM connections WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((base_url, models_json, auto_raw)) = row else {
        return Err("连接不存在".into());
    };
    let mut auto = parse_auto(auto_raw.as_deref());
    if auto.auto_models.is_empty() && auto.base_url_source.is_none() {
        return Err("该连接没有自动回填的记录".into());
    }
    let clear_base = auto.base_url_source.as_deref() == Some(SOURCE_ZCODE);
    let mut models = models_of(models_json.as_deref()).unwrap_or_default();
    let auto_keys: HashSet<String> = auto.auto_models.iter().map(|model| normalize_model(model)).collect();
    let before = models.len();
    models.retain(|model| !auto_keys.contains(&normalize_model(model)));
    let removed = before - models.len();
    // 回滚即停用：把这次的自动模型记账进 revokedModels，之后不会再补。
    for model in std::mem::take(&mut auto.auto_models) {
        if !auto.revoked_models.iter().any(|item| normalize_model(item) == normalize_model(&model)) {
            auto.revoked_models.push(model);
        }
    }
    auto.base_url_source = None;
    auto.base_url_normalized = None;
    auto.models_source = None;
    auto.disabled = true;
    auto.filled_ms = chrono::Utc::now().timestamp_millis();

    let auto_json = serde_json::to_string(&auto).map_err(|error| error.to_string())?;
    let models_json = serde_json::to_string(&models).map_err(|error| error.to_string())?;
    let base = if clear_base { None } else { base_url };
    conn.execute(
        "UPDATE connections SET base_url = ?2, models_json = ?3, catalog_auto = ?4 WHERE id = ?1",
        params![id, base, models_json, auto_json],
    )
    .map_err(|error| error.to_string())?;
    Ok(format!(
        "已回滚：{}；移除自动模型 {} 个；该连接不再自动回填",
        if clear_base { "清空自动补的 base_url" } else { "base_url 未动" },
        removed
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn add_connection(conn: &Connection, id: &str, platform: &str, kind: &str, base_url: Option<&str>) {
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,base_url,created_ms) VALUES(?1,?2,?3,?4,?5,1)",
            params![id, platform, kind, format!("连接 {id}"), base_url],
        )
        .unwrap();
    }

    fn rules(items: &[(&str, Option<&str>, &[&str])]) -> ProviderRules {
        items
            .iter()
            .map(|(id, base, models)| {
                (
                    (*id).to_string(),
                    ProviderRule {
                        base_url: base.map(str::to_owned),
                        models: models.iter().map(|model| (*model).to_string()).collect(),
                    },
                )
            })
            .collect()
    }

    fn row(conn: &Connection, id: &str) -> (Option<String>, Option<String>, Option<String>) {
        conn.query_row("SELECT base_url, models_json, catalog_auto FROM connections WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
    }

    /// 唯一候选：补 base_url 与模型清单，并留下可追溯的来源标记。
    #[test]
    fn unique_candidate_fills_base_url_and_models() {
        let conn = db();
        add_connection(&conn, "c-acct", "zcode", "auth", None);
        let report = apply(&conn, &rules(&[("relay", Some("https://user:pw@Relay.Example.test/v1/?token=x"), &["claude-opus-5-5"])]));
        assert_eq!(report.filled.len(), 1, "{report:?}");
        assert!(report.filled[0].base_url);
        assert_eq!(report.filled[0].models_added, 1);

        let (base, models, auto) = row(&conn, "c-acct");
        // 凭据与查询串都被丢掉
        assert_eq!(base.as_deref(), Some("https://relay.example.test/v1"));
        assert_eq!(models.as_deref(), Some(r#"["claude-opus-5-5"]"#));
        let auto = parse_auto(auto.as_deref());
        assert_eq!(auto.base_url_source.as_deref(), Some(SOURCE_ZCODE));
        assert_eq!(auto.models_source.as_deref(), Some(SOURCE_ZCODE));
        assert_eq!(auto.base_url_normalized.as_deref(), Some("relay.example.test/v1"));
        assert_eq!(auto.auto_models, vec!["claude-opus-5-5"]);
        assert_eq!(auto.providers, vec!["relay"]);
        // 报告本身不含地址原文
        assert!(!format!("{report:?}").contains("relay.example.test"));
    }

    /// 只补不改：已有 base_url（与规则同址）不重写，模型清单只做并集（去重、大小写不敏感）。
    #[test]
    fn existing_values_are_never_overwritten_and_models_union() {
        let conn = db();
        add_connection(&conn, "c-acct", "zcode", "auth", Some("https://relay.example.test"));
        conn.execute("UPDATE connections SET models_json='[\"kept-model\"]' WHERE id='c-acct'", []).unwrap();
        let report = apply(&conn, &rules(&[("relay", Some("https://relay.example.test/"), &["new-model", "KEPT-MODEL"])]));
        assert_eq!(report.filled.len(), 1, "{report:?}");
        assert!(!report.filled[0].base_url, "已有地址不重写");
        assert_eq!(report.filled[0].models_added, 1, "大小写不同算同一个模型");

        let (base, models, _) = row(&conn, "c-acct");
        assert_eq!(base.as_deref(), Some("https://relay.example.test"));
        assert_eq!(models.as_deref(), Some(r#"["kept-model","new-model"]"#));

        // 地址指到别处的连接根本不是这个 provider 的候选：一个字都不写
        let other = db();
        add_connection(&other, "c-elsewhere", "zcode", "auth", Some("https://elsewhere.example.test"));
        let report = apply(&other, &rules(&[("relay", Some("https://relay.example.test"), &["m1"])]));
        assert!(report.filled.is_empty(), "{report:?}");
        assert_eq!(report.skipped[0].reason, reason::NO_CANDIDATE);
        assert_eq!(row(&other, "c-elsewhere"), (Some("https://elsewhere.example.test".to_string()), None, None));
    }

    /// 多候选（同一平台两条连接）→ 不写。
    #[test]
    fn several_candidates_write_nothing() {
        let conn = db();
        add_connection(&conn, "c-1", "zcode", "auth", None);
        add_connection(&conn, "c-2", "zcode", "api", None);
        let report = apply(&conn, &rules(&[("relay", Some("https://relay.example.test"), &["m1"])]));
        assert!(report.filled.is_empty(), "多候选不许猜：{report:?}");
        assert_eq!(report.skipped[0].reason, reason::MULTI_CANDIDATE);
        assert_eq!(row(&conn, "c-1").1, None);
        assert_eq!(row(&conn, "c-2").1, None);
    }

    /// 平台无连接 → 不写，只给原因。
    #[test]
    fn no_candidate_writes_nothing() {
        let conn = db();
        add_connection(&conn, "c-claude", "claude", "auth", None);
        let report = apply(&conn, &rules(&[("relay", Some("https://relay.example.test"), &["m1"])]));
        assert!(report.filled.is_empty());
        assert_eq!(report.skipped[0].reason, reason::NO_CANDIDATE);
        // 别的平台的连接一个字都不许动
        assert_eq!(row(&conn, "c-claude"), (None, None, None));
    }

    /// 多个 provider 争同一条连接（同 baseUrl 的三兄弟）→ 全部不写，这正是它们的正确结论。
    #[test]
    fn contended_connection_stays_untouched() {
        let conn = db();
        add_connection(&conn, "c-acct", "zcode", "auth", None);
        let report = apply(
            &conn,
            &rules(&[
                ("new-provider", Some("https://relay.example.test"), &["m1"]),
                ("new-provider-2", Some("https://relay.example.test"), &["m2"]),
            ]),
        );
        assert!(report.filled.is_empty(), "{report:?}");
        assert!(report.skipped.iter().all(|skip| skip.reason == reason::CONTENDED));
        assert_eq!(row(&conn, "c-acct"), (None, None, None));
    }

    /// 用户删掉自动补的模型后不再补回来（不跟用户抢），且改动被记进 revokedModels。
    #[test]
    fn user_removed_models_are_not_restored() {
        let conn = db();
        add_connection(&conn, "c-acct", "zcode", "auth", None);
        apply(&conn, &rules(&[("relay", Some("https://relay.example.test"), &["m1", "m2"])]));
        assert_eq!(row(&conn, "c-acct").1.as_deref(), Some(r#"["m1","m2"]"#));

        // 用户删掉 m1
        conn.execute("UPDATE connections SET models_json='[\"m2\"]' WHERE id='c-acct'", []).unwrap();
        let report = apply(&conn, &rules(&[("relay", Some("https://relay.example.test"), &["m1", "m2"])]));
        assert_eq!(report.filled.len(), 1, "revoked 记账要落库：{report:?}");
        assert_eq!(report.filled[0].models_added, 0);
        assert_eq!(row(&conn, "c-acct").1.as_deref(), Some(r#"["m2"]"#));
        let auto = parse_auto(row(&conn, "c-acct").2.as_deref());
        assert_eq!(auto.revoked_models, vec!["m1"]);
        assert_eq!(auto.auto_models, vec!["m2"]);

        // 再跑一轮：已经没有可改的，安静跳过
        let again = apply(&conn, &rules(&[("relay", Some("https://relay.example.test"), &["m1", "m2"])]));
        assert!(again.filled.is_empty());
        assert_eq!(again.skipped[0].reason, reason::NOTHING_TO_FILL);
    }

    /// 用户手改 base_url 之后来源标记作废：不再声称是自己补的，值也不动。
    #[test]
    fn manual_base_url_edit_drops_provenance() {
        let conn = db();
        add_connection(&conn, "c-acct", "zcode", "auth", None);
        apply(&conn, &rules(&[("relay", Some("https://relay.example.test"), &["m1"])]));
        conn.execute("UPDATE connections SET base_url='https://manual.example.test' WHERE id='c-acct'", []).unwrap();

        let report = apply(&conn, &rules(&[("relay", Some("https://relay.example.test"), &["m1"])]));
        // 地址已指到别处：这条连接不再是候选，但来源标记照样被清掉
        assert!(report.filled.is_empty(), "{report:?}");
        assert_eq!(report.skipped[0].reason, reason::NO_CANDIDATE);
        let (base, models, auto) = row(&conn, "c-acct");
        assert_eq!(base.as_deref(), Some("https://manual.example.test"), "用户改的地址不许被改回去");
        assert_eq!(models.as_deref(), Some(r#"["m1"]"#), "已补的模型留着（只补不改）");
        let auto = parse_auto(auto.as_deref());
        assert_eq!(auto.base_url_source, None);
        assert_eq!(auto.base_url_normalized, None);
    }

    /// 回滚：清空自动补的 base_url、移除自动模型，并停用该连接的自动回填。
    #[test]
    fn rollback_restores_original_state_and_disables_further_fills() {
        let conn = db();
        add_connection(&conn, "c-acct", "zcode", "auth", None);
        conn.execute("UPDATE connections SET models_json='[\"user-model\"]' WHERE id='c-acct'", []).unwrap();
        apply(&conn, &rules(&[("relay", Some("https://relay.example.test"), &["m1"])]));
        assert_eq!(row(&conn, "c-acct").1.as_deref(), Some(r#"["user-model","m1"]"#));

        let note = rollback(&conn, "c-acct").unwrap();
        assert!(note.contains("移除自动模型 1 个"), "{note}");
        let (base, models, auto) = row(&conn, "c-acct");
        assert_eq!(base, None, "自动补的 base_url 要清掉");
        assert_eq!(models.as_deref(), Some(r#"["user-model"]"#), "用户自己的模型留着");
        let auto = parse_auto(auto.as_deref());
        assert!(auto.disabled);
        assert!(auto.auto_models.is_empty());
        assert_eq!(auto.revoked_models, vec!["m1"]);

        // 停用之后，下一轮采集不再补回来
        let report = apply(&conn, &rules(&[("relay", Some("https://relay.example.test"), &["m1"])]));
        assert!(report.filled.is_empty());
        assert_eq!(report.skipped[0].reason, reason::DISABLED);
        let (base_after, models_after, _) = row(&conn, "c-acct");
        assert_eq!(base_after, None);
        assert_eq!(models_after.as_deref(), Some(r#"["user-model"]"#));
    }

    /// 没有自动回填记录时回滚报错，不动任何数据。
    #[test]
    fn rollback_without_auto_records_is_refused() {
        let conn = db();
        add_connection(&conn, "c-acct", "zcode", "auth", Some("https://manual.example.test"));
        assert!(rollback(&conn, "c-acct").unwrap_err().contains("没有自动回填的记录"));
        assert_eq!(row(&conn, "c-acct").0.as_deref(), Some("https://manual.example.test"));
        assert!(rollback(&conn, "c-missing").unwrap_err().contains("连接不存在"));
    }

    /// 已有 base_url / 模型完全对得上（用户自己填了同样的值）：不写、不声称来源。
    #[test]
    fn identical_manual_values_are_left_alone() {
        let conn = db();
        add_connection(&conn, "c-acct", "zcode", "auth", Some("https://relay.example.test/v1"));
        conn.execute("UPDATE connections SET models_json='[\"m1\"]' WHERE id='c-acct'", []).unwrap();
        let report = apply(&conn, &rules(&[("relay", Some("https://relay.example.test/v1/"), &["m1"])]));
        assert!(report.filled.is_empty());
        assert_eq!(report.skipped[0].reason, reason::NOTHING_TO_FILL);
        assert_eq!(row(&conn, "c-acct").2, None, "用户自己填的值不该被我们记账");
    }

    /// 模型清单不是 JSON 数组：不覆盖，报原因留给人看。
    #[test]
    fn unreadable_models_are_not_overwritten() {
        let conn = db();
        add_connection(&conn, "c-acct", "zcode", "auth", None);
        conn.execute("UPDATE connections SET models_json='not-json' WHERE id='c-acct'", []).unwrap();
        let report = apply(&conn, &rules(&[("relay", Some("https://relay.example.test"), &["m1"])]));
        assert!(report.filled.is_empty());
        assert_eq!(report.skipped[0].reason, reason::MODELS_UNREADABLE);
        assert_eq!(row(&conn, "c-acct").0, None, "读不出来就整条不写");
    }

    /// 真库**副本**上的迁移 / 回填对照（绝不写真库）：
    ///
    /// ```text
    /// $env:JOLU_BACKFILL_DB = "$env:TEMP\jolu-copy\usage.db"
    /// cargo test --manifest-path backend/Cargo.toml catalog_backfill::tests::report_copy_backfill -- --ignored --nocapture
    /// ```
    ///
    /// 打印：迁移前后 `catalog_auto` 列的有无 → 回填前后连接行 → 归因结论 → 回滚后状态。
    /// 只打印「有没有地址 / 几个模型 / 来源标记」，不打印地址原文。
    #[test]
    #[ignore]
    fn report_copy_backfill() {
        let Some(path) = std::env::var_os("JOLU_BACKFILL_DB").map(std::path::PathBuf::from) else {
            println!("未设置 JOLU_BACKFILL_DB（指向真库副本），跳过");
            return;
        };
        let columns = |conn: &Connection| -> Vec<String> {
            let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('connections')").unwrap();
            let names: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().flatten().collect();
            names
        };
        // 迁移前：副本里还没有 catalog_auto 列
        let raw = Connection::open(&path).unwrap();
        let before_columns = columns(&raw);
        println!("迁移前：connections 共 {} 列，有 catalog_auto = {}", before_columns.len(), before_columns.iter().any(|c| c == "catalog_auto"));
        drop(raw);

        // 打开即完成迁移（纯增列），顺便把库调成应用正常形态
        let conn = crate::db::open(&path).unwrap();
        let after_columns = columns(&conn);
        println!("迁移后：connections 共 {} 列，有 catalog_auto = {}", after_columns.len(), after_columns.iter().any(|c| c == "catalog_auto"));

        let dump = |conn: &Connection, label: &str| {
            let mut stmt = conn
                .prepare("SELECT id, base_url, models_json, catalog_auto FROM connections WHERE platform = 'zcode' ORDER BY id")
                .unwrap();
            let rows: Vec<(String, Option<String>, Option<String>, Option<String>)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .unwrap()
                .flatten()
                .collect();
            for (id, base, models, auto) in rows {
                let models = models_of(models.as_deref()).unwrap_or_default();
                let auto = parse_auto(auto.as_deref());
                println!(
                    "{label} {id}：有 baseUrl = {}（含 @ = {}）/ 模型 {} 个 / 来源 base={:?} models={:?} / autoModels={:?} / disabled={}",
                    base.is_some(),
                    base.as_deref().is_some_and(|url| url.contains('@')),
                    models.len(),
                    auto.base_url_source,
                    auto.models_source,
                    auto.auto_models.len(),
                    auto.disabled
                );
            }
        };
        dump(&conn, "回填前");

        // 先按真机全量规则跑一遍：7 个 provider 争同一条连接 → 1:1 不成立，全部不写
        let all = crate::attribution::provider_rules();
        let all_report = apply(&conn, &all);
        println!("真机全量规则（{} 个）回填：filled = {} / skipped = {:?}", all.len(), all_report.filled.len(), all_report.skipped);
        dump(&conn, "全量规则后");

        // 再只留一个 provider 演示 1:1 才会写
        let single: ProviderRules =
            all.iter().filter(|(id, _)| id.as_str() == "new-provider").map(|(id, rule)| (id.clone(), rule.clone())).collect();
        let report = apply(&conn, &single);
        println!("单 provider 演示（new-provider）报告：filled = {:?}", report.filled);
        dump(&conn, "回填后");

        let snapshot = crate::attribution::Snapshot::load(&conn).unwrap();
        for provider in ["new-provider", "zai-api-2", "deepseek", "account:zai-start-plan"] {
            let view = snapshot.resolve("zcode", None, Some(provider), None);
            println!("  归因 {provider} → {:?} / {:?}", view.confidence, view.connection_id);
        }

        if let Some(fill) = report.filled.first() {
            println!("回滚：{}", rollback(&conn, &fill.connection_id).unwrap());
            dump(&conn, "回滚后");
            let report = apply(&conn, &single);
            println!("回滚后再跑一轮：filled = {} / skipped = {:?}", report.filled.len(), report.skipped);
        }
    }
}
