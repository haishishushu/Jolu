//! 模块 F · Skills 管理（画板 74–85）：按供应商扫描本机 Skill 目录，查看、新建 / 编辑、导入、复制、启停与移除。
//!
//! 目录规则按官方文档核对（Context7）：
//! - Claude Code：个人 `~/.claude/skills/<name>/SKILL.md`（尊重 `CLAUDE_CONFIG_DIR`），项目 `<项目>/.claude/skills/`；
//!   `skills/synced/` 是从 claude.ai 同步来的，只读。
//! - Codex：用户 `~/.agents/skills` 与旧位置 `$CODEX_HOME/skills`，系统 `$CODEX_HOME/skills/.system`（只读），
//!   项目 `<项目>/.agents/skills` 与 `<项目>/.codex/skills`。
//! 其他工具没有可靠依据，前端标「待接入」。
//!
//! 启停只作用于选中的这一份副本：
//! - Claude Code 的 `skillOverrides` 按名称生效、不能按路径区分，会连带同名副本，所以 Jolu 不写它；
//!   停用时把这一份移到应用数据目录的停用区 `skills-disabled/`（工具不会扫描），`skills.json` 记录原路径，
//!   启用时检查原位置没有冲突后移回。用户自己写的 `skillOverrides: "off"` 只读显示为「由工具配置停用」。
//! - Codex 的 `[[skills.config]]` 支持 `path` 选择器，按 SKILL.md 路径精确停用；用户写的 `name` 选择器同样只读显示。
//!
//! 项目范围：「目录设置」手动添加的项目 + 会话记录里 Claude Code / Codex 会话的项目目录（cwd），合并去重；
//! 自动带出的项目可以忽略。会话目录结果缓存 10 分钟，「扫描目录」时强制刷新。
//!
//! 文件安全：写入、覆盖、移动与移除前先备份到应用数据目录 `skill-backups/`；目标已存在时报冲突，不静默覆盖；
//! 操作的路径必须是已知根目录的直接子目录（或停用区里登记过的副本），符号链接安装项只读，复制时不跟随符号链接。

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// 支持扫描与写入的工具；其余供应商在前端标「待接入」
pub const PLATFORMS: [&str; 2] = ["claude", "codex"];
const SKILL_FILE: &str = "SKILL.md";
const MAX_SKILL_BYTES: u64 = 1024 * 1024;
const MAX_PREVIEW_BYTES: u64 = 512 * 1024;
const MAX_FILES: usize = 200;
/// 编辑保存时发现文件已被其他程序改动，前端据此显示画板 84 的「检测到外部修改」
pub const EXTERNAL_CHANGE: &str = "external_change";
const TOOL_DISABLED: &str = "由工具配置停用：这条配置按名称生效，会影响所有同名副本；Jolu 不改动你的配置，请在配置文件中修改";
const SESSION_TTL: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Global,
    Project,
    /// Codex 内置 Skills，只读
    System,
    /// 从 claude.ai 同步，只读
    Synced,
}

#[derive(Clone, Debug)]
struct Root {
    platform: &'static str,
    scope: Scope,
    project: Option<PathBuf>,
    path: PathBuf,
    readonly: bool,
    /// 扫描时跳过的子目录（它们是单独的根目录）
    skip: &'static [&'static str],
    /// 停用区里登记的副本
    parked: bool,
}

#[derive(Clone, Debug)]
pub struct Homes {
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub user: PathBuf,
}

fn homes() -> Result<Homes, String> {
    // 隔离验收环境不碰真实用户目录
    if let Some(dir) = crate::validation::data_dir() {
        let base = dir.join("skills-home");
        return Ok(Homes { claude: base.join(".claude"), codex: base.join(".codex"), user: base });
    }
    let user = crate::creds::home().ok_or("未找到用户主目录")?;
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| user.join(".claude"));
    let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| user.join(".codex"));
    Ok(Homes { claude, codex, user })
}

fn static_platform(platform: &str) -> &'static str {
    if platform == "claude" { "claude" } else { "codex" }
}

fn roots(h: &Homes, platform: &str, projects: &[PathBuf]) -> Vec<Root> {
    let root = |scope, project: Option<&PathBuf>, path: PathBuf, readonly, skip| Root {
        platform: static_platform(platform),
        scope,
        project: project.cloned(),
        path,
        readonly,
        skip,
        parked: false,
    };
    let mut out = Vec::new();
    match platform {
        "claude" => {
            out.push(root(Scope::Global, None, h.claude.join("skills"), false, &["synced"]));
            out.push(root(Scope::Synced, None, h.claude.join("skills").join("synced"), true, &[]));
            for p in projects {
                out.push(root(Scope::Project, Some(p), p.join(".claude").join("skills"), false, &[]));
            }
        }
        "codex" => {
            out.push(root(Scope::Global, None, h.user.join(".agents").join("skills"), false, &[]));
            out.push(root(Scope::Global, None, h.codex.join("skills"), false, &[".system"]));
            out.push(root(Scope::System, None, h.codex.join("skills").join(".system"), true, &[]));
            for p in projects {
                out.push(root(Scope::Project, Some(p), p.join(".agents").join("skills"), false, &[]));
                out.push(root(Scope::Project, Some(p), p.join(".codex").join("skills"), false, &[]));
            }
        }
        _ => {}
    }
    out
}

/// 新建、导入、复制的目标目录：每个范围取首选位置（Codex 用户级用 `~/.agents/skills`，`$CODEX_HOME/skills` 已标记为旧位置）
fn write_root(h: &Homes, platform: &str, scope: Scope, project: Option<&Path>) -> Result<Root, String> {
    let project = match scope {
        Scope::Global => None,
        Scope::Project => Some(project.ok_or("请选择项目目录")?.to_path_buf()),
        _ => return Err("只读范围不能写入".into()),
    };
    let list = roots(h, platform, project.as_slice());
    list.into_iter()
        .find(|r| r.scope == scope && !r.readonly)
        .ok_or_else(|| format!("{platform} 暂不支持写入 Skills"))
}

/* ───────────────────────── 路径比较 ───────────────────────── */

/// 统一 `/` 与 `\`、去掉末尾分隔符；Windows 不区分大小写
pub fn norm_key(path: &str) -> String {
    let unified = path.replace('\\', "/");
    let trimmed = unified.trim_end_matches('/');
    if cfg!(windows) { trimmed.to_lowercase() } else { trimmed.to_string() }
}

fn same_path(a: &str, b: &Path) -> bool {
    norm_key(a) == norm_key(&b.to_string_lossy())
}

/* ───────────────────────── 配置（应用数据目录 skills.json） ───────────────────────── */

/// Jolu 停用区里的一份副本
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DisabledEntry {
    pub platform: String,
    pub scope: Scope,
    pub project: Option<String>,
    pub name: String,
    /// 停用前所在的目录，启用时移回这里
    pub original: String,
    /// 停用区里的目录
    pub stored: String,
    pub disabled_at: i64,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// 「目录设置」里手动添加的项目目录
    #[serde(default)]
    pub projects: Vec<String>,
    /// 用户忽略的、从会话记录自动带出的项目目录
    #[serde(default)]
    pub ignored_projects: Vec<String>,
    /// 通过 Jolu 导入的安装项，来源列显示「导入」
    #[serde(default)]
    pub imported: Vec<String>,
    /// 停用区登记
    #[serde(default)]
    pub disabled: Vec<DisabledEntry>,
    /// 会话记录带出的项目目录（运行时填入，不落盘）
    #[serde(skip)]
    pub session_projects: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProjectInfo {
    pub path: String,
    /// manual：手动添加；session：来自会话记录
    pub source: &'static str,
    pub ignored: bool,
    /// 含有 Claude Code / Codex 的项目级 Skills 目录
    pub claude: bool,
    pub codex: bool,
}

impl Config {
    fn ignored(&self, path: &str) -> bool {
        self.ignored_projects.iter().any(|p| norm_key(p) == norm_key(path))
    }

    /// 手动 + 会话记录的项目，合并去重；跳过不存在的目录和用户主目录（其 Skills 目录就是全局目录）
    pub fn all_projects(&self, h: &Homes) -> Vec<ProjectInfo> {
        let home = norm_key(&h.user.to_string_lossy());
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let candidates = self.projects.iter().map(|p| (p, "manual")).chain(self.session_projects.iter().map(|p| (p, "session")));
        for (path, source) in candidates {
            let key = norm_key(path);
            let dir = Path::new(path);
            if key.is_empty() || key == home || !dir.is_absolute() || !dir.is_dir() || !seen.insert(key) {
                continue;
            }
            out.push(ProjectInfo {
                path: path.clone(),
                source,
                ignored: source == "session" && self.ignored(path),
                claude: dir.join(".claude").join("skills").is_dir(),
                codex: dir.join(".agents").join("skills").is_dir() || dir.join(".codex").join("skills").is_dir(),
            });
        }
        out
    }

    /// 参与扫描与作为写入目标的项目（不含已忽略的）
    fn project_paths(&self, h: &Homes) -> Vec<PathBuf> {
        self.all_projects(h).into_iter().filter(|p| !p.ignored).map(|p| PathBuf::from(p.path)).collect()
    }
}

fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    crate::data_paths(app).map(|(dir, _)| dir)
}

fn load_config(dir: &Path) -> Config {
    fs::read_to_string(dir.join("skills.json"))
        .ok()
        .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

fn save_config(dir: &Path, cfg: &Config) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    write_atomic(&dir.join("skills.json"), text.as_bytes())
}

/* ───────────────────────── 会话记录里的项目目录 ───────────────────────── */

static SESSION_PROJECTS: LazyLock<Mutex<Option<(Instant, Vec<String>)>>> = LazyLock::new(Default::default);

/// 复用会话线程的源文件定位与 cwd 读取（自带按修改时间失效的缓存）
fn read_session_projects(conn: &rusqlite::Connection) -> Vec<String> {
    let files = crate::session_threads::source_files(conn).unwrap_or_default();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in files.values() {
        if let Some(cwd) = crate::session_threads::read_cwd(path) {
            if seen.insert(norm_key(&cwd)) {
                out.push(cwd);
            }
        }
    }
    out
}

async fn session_projects(pool: Arc<crate::ReadPool>, force: bool) -> Result<Vec<String>, String> {
    if !force {
        if let Ok(guard) = SESSION_PROJECTS.lock() {
            if let Some((at, list)) = guard.as_ref() {
                if at.elapsed() < SESSION_TTL {
                    return Ok(list.clone());
                }
            }
        }
    }
    let list = crate::with_reader(pool, |conn| Ok(read_session_projects(conn))).await?;
    if let Ok(mut guard) = SESSION_PROJECTS.lock() {
        *guard = Some((Instant::now(), list.clone()));
    }
    Ok(list)
}

/* ───────────────────────── SKILL.md frontmatter ───────────────────────── */

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Front {
    pub name: Option<String>,
    pub description: Option<String>,
}

fn unquote(value: &str) -> String {
    let v = value.trim();
    if v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\''))) {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

/// 只解析顶层 `key: value`（含 `>` / `|` 多行写法），够判断 name 与 description；其余字段原样保留在文件里
pub fn parse_front(text: &str) -> Result<Front, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err("SKILL.md 缺少 frontmatter".into());
    }
    let mut body = Vec::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        body.push(line);
    }
    if !closed {
        return Err("SKILL.md 的 frontmatter 没有结束标记".into());
    }
    let mut front = Front::default();
    let mut i = 0;
    while i < body.len() {
        let line = body[i];
        i += 1;
        if line.starts_with([' ', '\t', '#']) || line.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let key = key.trim();
        let raw = value.trim();
        let value = if matches!(raw, ">" | "|" | ">-" | "|-" | ">+" | "|+") {
            let mut parts = Vec::new();
            while i < body.len() && (body[i].starts_with([' ', '\t']) || body[i].trim().is_empty()) {
                parts.push(body[i].trim());
                i += 1;
            }
            let sep = if raw.starts_with('>') { " " } else { "\n" };
            parts.join(sep).trim().to_string()
        } else {
            unquote(raw)
        };
        let value = (!value.is_empty()).then_some(value);
        match key {
            "name" => front.name = value,
            "description" => front.description = value,
            _ => {}
        }
    }
    Ok(front)
}

/// 格式问题；None 表示有效。Codex 要求 name 与 description，Claude Code 缺 name 时用目录名
fn validate_front(platform: &str, front: &Result<Front, String>) -> Option<String> {
    match front {
        Err(reason) => Some(reason.clone()),
        Ok(f) if f.description.is_none() => Some("SKILL.md 缺少 description".into()),
        Ok(f) if platform == "codex" && f.name.is_none() => Some("SKILL.md 缺少 name".into()),
        Ok(_) => None,
    }
}

/// 名称规则：小写字母、数字与连字符，1–64 位，不以连字符开头或结尾
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn skill_name(dir: &Path) -> String {
    read_limited(&dir.join(SKILL_FILE), MAX_SKILL_BYTES)
        .and_then(|(t, _)| parse_front(&t))
        .ok()
        .and_then(|f| f.name)
        .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/* ───────────────────────── 扫描 ───────────────────────── */

#[derive(Serialize, Clone, Debug)]
pub struct SkillItem {
    pub platform: String,
    /// frontmatter 的 name，缺失时为目录名
    pub name: String,
    pub dir_name: String,
    pub description: Option<String>,
    pub scope: Scope,
    /// 项目范围的项目目录
    pub project: Option<String>,
    /// local / git / project / imported / synced / system
    pub source: &'static str,
    pub enabled: bool,
    /// enabled / disabled / conflict / invalid
    pub status: &'static str,
    pub issue: Option<String>,
    /// 当前所在目录；停用区里的副本为停用区路径
    pub path: String,
    /// 停用区里的副本停用前所在目录
    pub original_path: Option<String>,
    /// parked：Jolu 停用区；path：Codex 按路径停用；tool：工具配置按名称停用（只读）
    pub disabled_by: Option<&'static str>,
    pub readonly: bool,
    pub symlink: bool,
    pub modified_ms: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScanError {
    pub path: String,
    pub reason: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct RootInfo {
    pub platform: String,
    pub scope: Scope,
    pub project: Option<String>,
    pub path: String,
    pub exists: bool,
    pub readonly: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct PlatformSummary {
    pub id: String,
    /// 至少有一个 Skills 根目录存在
    pub detected: bool,
    pub count: usize,
}

#[derive(Serialize, Debug)]
pub struct Overview {
    pub platforms: Vec<PlatformSummary>,
    pub items: Vec<SkillItem>,
    pub roots: Vec<RootInfo>,
    pub projects: Vec<ProjectInfo>,
    pub errors: Vec<ScanError>,
    pub scanned_at: i64,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn modified_ms(path: &Path) -> Option<i64> {
    let time = fs::metadata(path).ok()?.modified().ok()?;
    Some(time.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64)
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

fn read_limited(path: &Path, limit: u64) -> Result<(String, bool), String> {
    let file = fs::File::open(path).map_err(|e| format!("无法读取 {}：{e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > limit;
    bytes.truncate(limit as usize);
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
}

struct Raw {
    root: Root,
    dir: PathBuf,
    dir_name: String,
    front: Result<Front, String>,
    original: Option<String>,
}

fn scan_root(root: &Root, out: &mut Vec<Raw>, errors: &mut Vec<ScanError>) {
    if !root.path.is_dir() {
        return;
    }
    let entries = match fs::read_dir(&root.path) {
        Ok(entries) => entries,
        Err(e) => {
            errors.push(ScanError { path: root.path.display().to_string(), reason: e.to_string() });
            return;
        }
    };
    let mut found: Vec<Raw> = Vec::new();
    for entry in entries.flatten() {
        let dir_name = entry.file_name().to_string_lossy().into_owned();
        if dir_name.starts_with('.') || root.skip.contains(&dir_name.as_str()) {
            continue;
        }
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let file = dir.join(SKILL_FILE);
        if !file.is_file() {
            continue;
        }
        let front = read_limited(&file, MAX_SKILL_BYTES).and_then(|(text, _)| parse_front(&text));
        found.push(Raw { root: root.clone(), dir, dir_name, front, original: None });
    }
    found.sort_by(|a, b| a.dir_name.cmp(&b.dir_name));
    out.extend(found);
}

/* ───────────────────────── 工具配置（只读识别） ───────────────────────── */

fn read_json_object(path: &Path) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(Default::default()),
        Ok(text) => match serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')) {
            Ok(serde_json::Value::Object(map)) => Ok(map),
            _ => Err(format!("{} 不是有效的 JSON 对象", path.display())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(e) => Err(format!("无法读取 {}：{e}", path.display())),
    }
}

/// 某个 settings 文件里这个名称的 skillOverrides 值
fn claude_override(path: &Path, name: &str) -> Option<String> {
    read_json_object(path).ok()?.get("skillOverrides")?.get(name)?.as_str().map(String::from)
}

/// 用户自己写的 `skillOverrides: "off"`：按设置优先级取第一个写了这个名称的文件
/// （项目 settings.local.json > 项目 settings.json > 用户 settings.json）
fn claude_tool_off(h: &Homes, root: &Root, name: &str) -> bool {
    let mut files = Vec::new();
    if let (Scope::Project, Some(p)) = (root.scope, &root.project) {
        files.push(p.join(".claude").join("settings.local.json"));
        files.push(p.join(".claude").join("settings.json"));
    }
    files.push(h.claude.join("settings.json"));
    files.iter().find_map(|f| claude_override(f, name)).as_deref() == Some("off")
}

fn codex_config_path(h: &Homes) -> PathBuf {
    h.codex.join("config.toml")
}

/// Codex config.toml 里 enabled = false 的覆盖：(path 选择器, name 选择器)
fn codex_disabled(path: &Path) -> (Vec<String>, Vec<String>) {
    let Ok(text) = fs::read_to_string(path) else { return (vec![], vec![]) };
    let Ok(value) = text.parse::<toml::Table>() else { return (vec![], vec![]) };
    let mut paths = Vec::new();
    let mut names = Vec::new();
    let entries = value.get("skills").and_then(|s| s.get("config")).and_then(|c| c.as_array()).cloned().unwrap_or_default();
    for entry in entries {
        if entry.get("enabled").and_then(|v| v.as_bool()) != Some(false) {
            continue;
        }
        if let Some(p) = entry.get("path").and_then(|v| v.as_str()) {
            paths.push(p.to_string());
        }
        if let Some(n) = entry.get("name").and_then(|v| v.as_str()) {
            names.push(n.to_string());
        }
    }
    (paths, names)
}

/* ───────────────────────── 启停 ───────────────────────── */

fn has_symlink_inside(dir: &Path) -> bool {
    fs::read_dir(dir).map(|entries| {
        entries.flatten().any(|e| match e.file_type() {
            Ok(t) if t.is_symlink() => true,
            Ok(t) if t.is_dir() => has_symlink_inside(&e.path()),
            _ => false,
        })
    }).unwrap_or(false)
}

/// 移动目录：同盘直接改名；跨盘先完整复制并校验再删原目录。目录里有符号链接时拒绝跨盘移动，避免丢失链接
fn move_dir(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        return Err(format!("目标已存在：{}", to.display()));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建目录：{e}"))?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if has_symlink_inside(from) {
        return Err("目录里有符号链接，不能跨磁盘移动；未做任何修改".into());
    }
    if let Err(e) = copy_dir(from, to) {
        let _ = fs::remove_dir_all(to);
        return Err(format!("移动失败，原目录保持不变：{e}"));
    }
    if !to.join(SKILL_FILE).is_file() {
        let _ = fs::remove_dir_all(to);
        return Err("移动后校验失败，原目录保持不变".into());
    }
    fs::remove_dir_all(from).map_err(|e| format!("已复制到 {}，但原目录删除失败：{e}", to.display()))
}

fn remove_empty(dir: Option<&Path>) {
    if let Some(dir) = dir {
        let _ = fs::remove_dir(dir);
    }
}

/// Claude 停用：把这一份移到停用区，记录原路径
fn park(cfg: &mut Config, root: &Root, dir: &Path, name: &str, data: &Path) -> Result<String, String> {
    ensure_writable(root, dir)?;
    let dir_name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    backup_dir_tree(dir, root.platform, &data.join("skill-backups").join("disabled"))?;
    let stored = data.join("skills-disabled").join(format!("{}-{}", stamp(), root.platform)).join(&dir_name);
    move_dir(dir, &stored)?;
    cfg.disabled.push(DisabledEntry {
        platform: root.platform.into(),
        scope: root.scope,
        project: root.project.as_ref().map(|p| p.display().to_string()),
        name: name.into(),
        original: dir.display().to_string(),
        stored: stored.display().to_string(),
        disabled_at: now_ms(),
    });
    if let Err(e) = save_config(data, cfg) {
        cfg.disabled.pop();
        let _ = move_dir(&stored, dir);
        remove_empty(stored.parent());
        return Err(format!("记录停用状态失败，已恢复原位置：{e}"));
    }
    Ok(stored.display().to_string())
}

/// Claude 启用：从停用区移回原位置；原位置已有目录时报冲突
fn unpark(cfg: &mut Config, dir: &Path, data: &Path) -> Result<String, String> {
    let index = cfg.disabled.iter().position(|e| same_path(&e.stored, dir)).ok_or("停用区里没有这份 Skill 的登记")?;
    let entry = cfg.disabled[index].clone();
    let original = PathBuf::from(&entry.original);
    if original.exists() {
        return Err(format!("原位置已有同名目录，未启用：{}", entry.original));
    }
    move_dir(dir, &original)?;
    cfg.disabled.remove(index);
    if let Err(e) = save_config(data, cfg) {
        cfg.disabled.insert(index, entry);
        let _ = move_dir(&original, dir);
        return Err(format!("记录启用状态失败，已放回停用区：{e}"));
    }
    remove_empty(dir.parent());
    Ok(entry.original)
}

/// 启停这一份副本，返回启停后的目录
fn set_enabled_impl(h: &Homes, cfg: &mut Config, root: &Root, dir: &Path, name: &str, enabled: bool, data: &Path) -> Result<String, String> {
    match root.platform {
        "claude" => match (enabled, root.parked) {
            (true, true) => unpark(cfg, dir, data),
            (false, false) => park(cfg, root, dir, name, data),
            (true, false) if claude_tool_off(h, root, name) => Err(TOOL_DISABLED.into()),
            _ => Ok(dir.display().to_string()),
        },
        "codex" => {
            let path = codex_config_path(h);
            let (_, names) = codex_disabled(&path);
            if enabled && names.iter().any(|n| n == name) {
                return Err(TOOL_DISABLED.into());
            }
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(format!("无法读取 config.toml：{e}")),
            };
            let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| "config.toml 格式无效，未修改")?;
            let skill_file = dir.join(SKILL_FILE);
            let skills = doc
                .entry("skills")
                .or_insert_with(|| {
                    let mut t = toml_edit::Table::new();
                    t.set_implicit(true);
                    toml_edit::Item::Table(t)
                })
                .as_table_like_mut()
                .ok_or("config.toml 的 skills 不是表，未修改")?;
            let config = skills.entry("config").or_insert(toml_edit::Item::ArrayOfTables(Default::default()));
            let array = config.as_array_of_tables_mut().ok_or("config.toml 的 skills.config 写法不受支持，请手动修改")?;
            // 只动指向这一份 SKILL.md 的 path 条目；name 条目是用户写的，保持原样
            let mut keep = toml_edit::ArrayOfTables::new();
            for table in array.iter() {
                if table.get("path").and_then(|v| v.as_str()).is_some_and(|p| same_path(p, &skill_file)) {
                    continue;
                }
                keep.push(table.clone());
            }
            if !enabled {
                let mut entry = toml_edit::Table::new();
                entry["path"] = toml_edit::value(skill_file.to_string_lossy().into_owned());
                entry["enabled"] = toml_edit::value(false);
                keep.push(entry);
            }
            *array = keep;
            backup_file(&path, &data.join("skill-backups").join("config"))?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            write_atomic(&path, doc.to_string().as_bytes())?;
            Ok(dir.display().to_string())
        }
        other => Err(format!("{other} 暂不支持启停 Skills")),
    }
}

/* ───────────────────────── 汇总 ───────────────────────── */

fn source_of(raw: &Raw, imported: &[String]) -> &'static str {
    let home = raw.original.as_ref().map(PathBuf::from).unwrap_or_else(|| raw.dir.clone());
    match raw.root.scope {
        Scope::System => "system",
        Scope::Synced => "synced",
        _ if imported.iter().any(|p| same_path(p, &home)) => "imported",
        _ if raw.dir.join(".git").exists() => "git",
        Scope::Project => "project",
        Scope::Global => "local",
    }
}

fn collect(h: &Homes, cfg: &Config, platform: &str, errors: &mut Vec<ScanError>) -> (Vec<SkillItem>, Vec<RootInfo>) {
    let projects = cfg.project_paths(h);
    // 没有对应 Skills 目录的项目不进入扫描结果
    let list: Vec<Root> = roots(h, platform, &projects).into_iter().filter(|r| r.scope != Scope::Project || r.path.is_dir()).collect();
    let mut raws = Vec::new();
    for root in &list {
        scan_root(root, &mut raws, errors);
    }
    for entry in cfg.disabled.iter().filter(|e| e.platform == platform) {
        let dir = PathBuf::from(&entry.stored);
        if !dir.join(SKILL_FILE).is_file() {
            errors.push(ScanError { path: entry.original.clone(), reason: format!("停用区里找不到这份 Skill：{}", entry.stored) });
            continue;
        }
        let root = Root {
            platform: static_platform(platform),
            scope: entry.scope,
            project: entry.project.as_ref().map(PathBuf::from),
            path: dir.parent().map(Path::to_path_buf).unwrap_or_default(),
            readonly: false,
            skip: &[],
            parked: true,
        };
        let dir_name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let front = read_limited(&dir.join(SKILL_FILE), MAX_SKILL_BYTES).and_then(|(t, _)| parse_front(&t));
        raws.push(Raw { root, dir, dir_name, front, original: Some(entry.original.clone()) });
    }
    let infos = list
        .iter()
        .map(|r| RootInfo {
            platform: r.platform.into(),
            scope: r.scope,
            project: r.project.as_ref().map(|p| p.display().to_string()),
            path: r.path.display().to_string(),
            exists: r.path.is_dir(),
            readonly: r.readonly,
        })
        .collect();
    let codex = if platform == "codex" { Some(codex_disabled(&codex_config_path(h))) } else { None };
    let mut seen: Vec<(String, Scope)> = Vec::new();
    let mut items = Vec::new();
    for raw in &raws {
        let name = raw.front.as_ref().ok().and_then(|f| f.name.clone()).unwrap_or_else(|| raw.dir_name.clone());
        let disabled_by: Option<&'static str> = if raw.root.parked {
            Some("parked")
        } else {
            match &codex {
                Some((paths, names)) => {
                    let file = raw.dir.join(SKILL_FILE);
                    if names.contains(&name) {
                        Some("tool")
                    } else if paths.iter().any(|p| same_path(p, &file)) {
                        Some("path")
                    } else {
                        None
                    }
                }
                None => claude_tool_off(h, &raw.root, &name).then_some("tool"),
            }
        };
        let enabled = disabled_by.is_none();
        let invalid = validate_front(platform, &raw.front);
        // 停用区里的副本不会被工具加载，不参与同名判断
        let conflict = if raw.root.parked {
            None
        } else {
            let earlier = seen.iter().find(|(n, _)| *n == name).map(|(_, s)| *s);
            seen.push((name.clone(), raw.root.scope));
            earlier.map(|scope| {
                if scope != Scope::Project && raw.root.scope == Scope::Project {
                    "同名条目：项目与全局同时存在".to_string()
                } else {
                    "同名条目：多个目录同时存在".to_string()
                }
            })
        };
        let (status, issue) = match (invalid, conflict) {
            (Some(reason), _) => ("invalid", Some(reason)),
            (None, Some(reason)) => ("conflict", Some(reason)),
            (None, None) if disabled_by == Some("tool") => ("disabled", Some(TOOL_DISABLED.split('；').next().unwrap_or(TOOL_DISABLED).to_string())),
            (None, None) => (if enabled { "enabled" } else { "disabled" }, None),
        };
        items.push(SkillItem {
            platform: platform.into(),
            name,
            dir_name: raw.dir_name.clone(),
            description: raw.front.as_ref().ok().and_then(|f| f.description.clone()),
            scope: raw.root.scope,
            project: raw.root.project.as_ref().map(|p| p.display().to_string()),
            source: source_of(raw, &cfg.imported),
            enabled,
            status,
            issue,
            path: raw.dir.display().to_string(),
            original_path: raw.original.clone(),
            disabled_by,
            readonly: raw.root.readonly || is_symlink(&raw.dir),
            symlink: is_symlink(&raw.dir),
            modified_ms: modified_ms(&raw.dir.join(SKILL_FILE)),
        });
    }
    (items, infos)
}

pub fn overview_with(h: &Homes, cfg: &Config, platform: &str) -> Result<Overview, String> {
    if platform != "all" && !PLATFORMS.contains(&platform) {
        return Err(format!("{platform} 暂未接入 Skills 管理"));
    }
    let mut errors = Vec::new();
    let mut platforms = Vec::new();
    let mut items = Vec::new();
    let mut roots_out = Vec::new();
    for p in PLATFORMS {
        let (list, infos) = collect(h, cfg, p, &mut errors);
        platforms.push(PlatformSummary { id: p.into(), detected: infos.iter().any(|r| r.exists), count: list.len() });
        if platform == "all" || platform == p {
            items.extend(list);
            roots_out.extend(infos);
        }
    }
    Ok(Overview { platforms, items, roots: roots_out, projects: cfg.all_projects(h), errors, scanned_at: now_ms() })
}

/* ───────────────────────── 路径定位 ───────────────────────── */

fn has_parent_component(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::ParentDir))
}

/// 校验 `path` 是某个已知根目录的直接子目录，或停用区里登记过的副本，返回所属根目录
fn locate(h: &Homes, cfg: &Config, path: &str) -> Result<(Root, PathBuf), String> {
    let dir = PathBuf::from(path);
    if !dir.is_absolute() || has_parent_component(&dir) {
        return Err("路径无效".into());
    }
    let parent = dir.parent().ok_or("路径无效")?;
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.is_empty() || name.starts_with('.') {
        return Err("路径无效".into());
    }
    if let Some(entry) = cfg.disabled.iter().find(|e| same_path(&e.stored, &dir)) {
        if !dir.join(SKILL_FILE).is_file() {
            return Err("停用区里找不到这份 Skill".into());
        }
        let root = Root {
            platform: static_platform(&entry.platform),
            scope: entry.scope,
            project: entry.project.as_ref().map(PathBuf::from),
            path: parent.to_path_buf(),
            readonly: false,
            skip: &[],
            parked: true,
        };
        return Ok((root, dir));
    }
    let parent_c = fs::canonicalize(parent).map_err(|_| "Skill 所在目录不存在")?;
    let projects = cfg.project_paths(h);
    for platform in PLATFORMS {
        for root in roots(h, platform, &projects) {
            let Ok(root_c) = fs::canonicalize(&root.path) else { continue };
            if root_c == parent_c && !root.skip.contains(&name.as_str()) {
                if !dir.join(SKILL_FILE).is_file() {
                    return Err("目录里没有 SKILL.md".into());
                }
                return Ok((root, dir));
            }
        }
    }
    Err("不在已配置的 Skills 目录中".into())
}

fn ensure_writable(root: &Root, dir: &Path) -> Result<(), String> {
    if root.readonly {
        return Err("这是工具托管的只读安装项，可复制为自定义后再修改".into());
    }
    if is_symlink(dir) {
        return Err("符号链接安装项只读，请到原位置修改".into());
    }
    Ok(())
}

fn safe_rel(rel: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(rel);
    if rel.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("文件路径无效".into());
    }
    Ok(p)
}

/* ───────────────────────── 文件工具 ───────────────────────── */

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.jolu-{nonce}.tmp"));
    let result = (|| {
        let mut file = fs::OpenOptions::new().create_new(true).write(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}

fn stamp() -> String {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() % 1000).unwrap_or(0);
    format!("{}{nonce:03}", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f"))
}

fn backup_file(path: &Path, backup_dir: &Path) -> Result<Option<PathBuf>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    fs::create_dir_all(backup_dir).map_err(|e| e.to_string())?;
    let target = backup_dir.join(format!("{}-{name}", stamp()));
    fs::copy(path, &target).map_err(|e| format!("备份失败，未修改：{e}"))?;
    Ok(Some(target))
}

/// 递归复制，不跟随符号链接
fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target).map_err(|e| format!("复制 {} 失败：{e}", entry.path().display()))?;
        }
    }
    Ok(())
}

fn backup_dir_tree(dir: &Path, platform: &str, backup_root: &Path) -> Result<PathBuf, String> {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let target = backup_root.join(format!("{}-{platform}-{name}", stamp()));
    copy_dir(dir, &target).map_err(|e| format!("备份失败，未修改：{e}"))?;
    Ok(target)
}

/* ───────────────────────── 详情 ───────────────────────── */

#[derive(Serialize, Debug)]
pub struct FileEntry {
    pub rel: String,
    pub dir: bool,
    pub depth: u8,
    pub size: Option<u64>,
}

#[derive(Serialize, Debug)]
pub struct Detail {
    pub item: SkillItem,
    pub files: Vec<FileEntry>,
    pub files_truncated: bool,
    pub content: String,
    pub truncated: bool,
}

fn list_files(dir: &Path, base: &Path, depth: u8, out: &mut Vec<FileEntry>) -> bool {
    let Ok(entries) = fs::read_dir(dir) else { return false };
    let mut entries: Vec<_> = entries.flatten().filter(|e| e.file_type().map(|t| !t.is_symlink()).unwrap_or(false)).collect();
    entries.sort_by_key(|e| (!e.path().is_dir(), e.file_name()));
    for entry in entries {
        if out.len() >= MAX_FILES {
            return true;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        let is_dir = path.is_dir();
        out.push(FileEntry { rel, dir: is_dir, depth, size: if is_dir { None } else { fs::metadata(&path).ok().map(|m| m.len()) } });
        if is_dir && depth < 3 && list_files(&path, base, depth + 1, out) {
            return true;
        }
    }
    false
}

pub fn detail_with(h: &Homes, cfg: &Config, path: &str) -> Result<Detail, String> {
    let (root, dir) = locate(h, cfg, path)?;
    let mut errors = Vec::new();
    let (items, _) = collect(h, cfg, root.platform, &mut errors);
    let item = items.into_iter().find(|i| same_path(&i.path, &dir)).ok_or("没有找到这个 Skill，可能已被移除")?;
    let (content, truncated) = read_limited(&dir.join(SKILL_FILE), MAX_PREVIEW_BYTES)?;
    let mut files = Vec::new();
    let files_truncated = list_files(&dir, &dir, 0, &mut files);
    Ok(Detail { item, files, files_truncated, content, truncated })
}

#[derive(Serialize, Debug)]
pub struct FileContent {
    pub text: Option<String>,
    pub binary: bool,
    pub size: u64,
    pub truncated: bool,
}

pub fn file_with(h: &Homes, cfg: &Config, path: &str, rel: &str) -> Result<FileContent, String> {
    let (_, dir) = locate(h, cfg, path)?;
    let full = dir.join(safe_rel(rel)?);
    let full_c = fs::canonicalize(&full).map_err(|_| "文件不存在")?;
    let dir_c = fs::canonicalize(&dir).map_err(|e| e.to_string())?;
    if !full_c.starts_with(&dir_c) || is_symlink(&full) || !full_c.is_file() {
        return Err("文件不在此 Skill 目录内".into());
    }
    let size = fs::metadata(&full_c).map(|m| m.len()).unwrap_or(0);
    let file = fs::File::open(&full_c).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_PREVIEW_BYTES + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > MAX_PREVIEW_BYTES;
    bytes.truncate(MAX_PREVIEW_BYTES as usize);
    if bytes.contains(&0) {
        return Ok(FileContent { text: None, binary: true, size, truncated: false });
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(FileContent { text: Some(text), binary: false, size, truncated }),
        Err(e) if truncated => Ok(FileContent { text: Some(String::from_utf8_lossy(&e.into_bytes()).into_owned()), binary: false, size, truncated }),
        Err(_) => Ok(FileContent { text: None, binary: true, size, truncated: false }),
    }
}

/* ───────────────────────── 新建 / 编辑 ───────────────────────── */

#[derive(Deserialize, Debug)]
pub struct SaveInput {
    pub platform: String,
    pub scope: Scope,
    pub project: Option<String>,
    pub name: String,
    pub content: String,
    /// 编辑时为原目录；新建为 None
    pub original_path: Option<String>,
    /// 打开编辑器时 SKILL.md 的修改时间，用于发现外部修改
    pub expected_modified_ms: Option<i64>,
    pub enable: bool,
}

#[derive(Serialize, Debug)]
pub struct SaveResult {
    pub path: String,
    pub modified_ms: Option<i64>,
    pub backup: Option<String>,
}

fn check_project(h: &Homes, cfg: &Config, project: Option<&str>) -> Result<Option<PathBuf>, String> {
    match project {
        None => Ok(None),
        Some(p) => cfg
            .project_paths(h)
            .into_iter()
            .find(|known| same_path(p, known))
            .map(Some)
            .ok_or_else(|| "项目目录不在会话记录或「目录设置」里".into()),
    }
}

pub fn save_with(h: &Homes, cfg: &mut Config, input: SaveInput, data: &Path) -> Result<SaveResult, String> {
    if !PLATFORMS.contains(&input.platform.as_str()) {
        return Err(format!("{} 暂不支持写入 Skills", input.platform));
    }
    if !valid_name(&input.name) {
        return Err("名称只能使用小写字母、数字和连字符，最长 64 位".into());
    }
    if input.content.len() as u64 > MAX_SKILL_BYTES {
        return Err("SKILL.md 超过 1 MB".into());
    }
    let front = parse_front(&input.content);
    if let Some(reason) = validate_front(&input.platform, &front) {
        return Err(reason);
    }
    if front.as_ref().ok().and_then(|f| f.name.as_deref()).is_some_and(|n| n != input.name) {
        return Err("SKILL.md 里的 name 与名称不一致".into());
    }
    if let Some(original) = &input.original_path {
        let (root, dir) = locate(h, cfg, original)?;
        ensure_writable(&root, &dir)?;
        if root.platform != input.platform || dir.file_name().map(|n| n.to_string_lossy() != input.name).unwrap_or(true) {
            return Err("编辑时不能修改名称、供应商或位置；请用「复制到供应商」".into());
        }
        let file = dir.join(SKILL_FILE);
        if let (Some(expected), Some(actual)) = (input.expected_modified_ms, modified_ms(&file)) {
            if (expected - actual).abs() > 1 {
                return Err(EXTERNAL_CHANGE.into());
            }
        }
        let backup = backup_file(&file, &data.join("skill-backups").join("files"))?;
        write_atomic(&file, input.content.as_bytes())?;
        return Ok(SaveResult { path: dir.display().to_string(), modified_ms: modified_ms(&file), backup: backup.map(|p| p.display().to_string()) });
    }
    let project = check_project(h, cfg, input.project.as_deref())?;
    let root = write_root(h, &input.platform, input.scope, project.as_deref())?;
    let dir = root.path.join(&input.name);
    if dir.exists() {
        return Err(format!("目标已有同名 Skill：{}", dir.display()));
    }
    fs::create_dir_all(&root.path).map_err(|e| format!("无法创建目录：{e}"))?;
    fs::create_dir(&dir).map_err(|e| format!("无法创建目录：{e}"))?;
    let file = dir.join(SKILL_FILE);
    write_atomic(&file, input.content.as_bytes())?;
    let mut path = dir.display().to_string();
    if !input.enable {
        path = set_enabled_impl(h, cfg, &root, &dir, &input.name, false, data)?;
    }
    Ok(SaveResult { modified_ms: modified_ms(&Path::new(&path).join(SKILL_FILE)), path, backup: None })
}

/* ───────────────────────── 复制 / 移除 ───────────────────────── */

#[derive(Serialize, Debug)]
pub struct CopyResult {
    pub target: String,
    /// 仅检查时为 false
    pub copied: bool,
    pub conflict: bool,
    pub issue: Option<String>,
}

pub fn copy_with(h: &Homes, cfg: &Config, path: &str, platform: &str, scope: Scope, project: Option<&str>, dry_run: bool) -> Result<CopyResult, String> {
    let (_, dir) = locate(h, cfg, path)?;
    let project = check_project(h, cfg, project)?;
    let root = write_root(h, platform, scope, project.as_deref())?;
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let target = root.path.join(&name);
    let front = read_limited(&dir.join(SKILL_FILE), MAX_SKILL_BYTES).and_then(|(t, _)| parse_front(&t));
    let issue = validate_front(platform, &front).map(|r| format!("目标工具不接受：{r}"));
    let conflict = target.exists();
    if same_path(&target.to_string_lossy(), &dir) {
        return Err("目标与源目录相同".into());
    }
    if dry_run || conflict || issue.is_some() {
        return Ok(CopyResult { target: target.display().to_string(), copied: false, conflict, issue });
    }
    copy_dir(&dir, &target)?;
    Ok(CopyResult { target: target.display().to_string(), copied: true, conflict: false, issue: None })
}

pub fn remove_with(h: &Homes, cfg: &mut Config, path: &str, data: &Path) -> Result<String, String> {
    let (root, dir) = locate(h, cfg, path)?;
    ensure_writable(&root, &dir)?;
    let backup = backup_dir_tree(&dir, root.platform, &data.join("skill-backups").join("removed"))?;
    fs::remove_dir_all(&dir).map_err(|e| format!("移除失败（备份已保存在 {}）：{e}", backup.display()))?;
    if root.parked {
        cfg.disabled.retain(|e| !same_path(&e.stored, &dir));
        save_config(data, cfg)?;
        remove_empty(dir.parent());
    }
    Ok(backup.display().to_string())
}

/* ───────────────────────── 导入 ───────────────────────── */

#[derive(Serialize, Debug, Clone)]
pub struct ImportItem {
    pub name: String,
    pub path: String,
    pub description: Option<String>,
    /// ok / conflict / invalid
    pub status: &'static str,
    pub issue: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct ImportScan {
    pub source: String,
    pub target: String,
    pub items: Vec<ImportItem>,
}

pub fn import_scan_with(h: &Homes, cfg: &Config, source: &str, platform: &str, scope: Scope, project: Option<&str>) -> Result<ImportScan, String> {
    let src = PathBuf::from(source);
    if !src.is_absolute() || has_parent_component(&src) || !src.is_dir() {
        return Err("来源目录不存在".into());
    }
    let project = check_project(h, cfg, project)?;
    let root = write_root(h, platform, scope, project.as_deref())?;
    let mut candidates = Vec::new();
    if src.join(SKILL_FILE).is_file() {
        candidates.push(src.clone());
    } else {
        let mut dirs: Vec<PathBuf> = fs::read_dir(&src)
            .map_err(|e| format!("无法读取来源目录：{e}"))?
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .filter(|p| p.join(SKILL_FILE).is_file())
            .collect();
        dirs.sort();
        candidates.extend(dirs.into_iter().take(MAX_FILES));
    }
    let items = candidates
        .into_iter()
        .map(|dir| {
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let front = read_limited(&dir.join(SKILL_FILE), MAX_SKILL_BYTES).and_then(|(t, _)| parse_front(&t));
            let invalid = validate_front(platform, &front).or_else(|| (!valid_name(&name)).then(|| "目录名不符合 Skill 名称规则".to_string()));
            let (status, issue) = match invalid {
                Some(reason) => ("invalid", Some(format!("{reason}，需先修复"))),
                None if root.path.join(&name).exists() => ("conflict", Some("目标已有同名 Skill".to_string())),
                None => ("ok", None),
            };
            ImportItem { name, path: dir.display().to_string(), description: front.ok().and_then(|f| f.description), status, issue }
        })
        .collect();
    Ok(ImportScan { source: src.display().to_string(), target: root.path.display().to_string(), items })
}

#[derive(Deserialize, Debug)]
pub struct ImportChoice {
    pub name: String,
    /// import / overwrite / skip
    pub action: String,
}

#[derive(Serialize, Debug, Default)]
pub struct ImportResult {
    pub imported: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<ScanError>,
}

#[allow(clippy::too_many_arguments)]
pub fn import_with(h: &Homes, cfg: &mut Config, source: &str, platform: &str, scope: Scope, project: Option<&str>, choices: Vec<ImportChoice>, enable: bool, data: &Path) -> Result<ImportResult, String> {
    let scan = import_scan_with(h, cfg, source, platform, scope, project)?;
    let project_path = check_project(h, cfg, project)?;
    let root = write_root(h, platform, scope, project_path.as_deref())?;
    let mut result = ImportResult::default();
    for choice in choices {
        let Some(item) = scan.items.iter().find(|i| i.name == choice.name) else {
            result.failed.push(ScanError { path: choice.name, reason: "来源中没有这个 Skill".into() });
            continue;
        };
        let target = root.path.join(&item.name);
        let outcome = match (choice.action.as_str(), item.status) {
            ("skip", _) => {
                result.skipped.push(item.name.clone());
                continue;
            }
            (_, "invalid") => Err(item.issue.clone().unwrap_or_else(|| "格式异常".into())),
            ("import", "ok") => fs::create_dir_all(&root.path).map_err(|e| e.to_string()).and_then(|_| copy_dir(Path::new(&item.path), &target)),
            ("overwrite", "conflict") => (|| {
                ensure_writable(&root, &target)?;
                backup_dir_tree(&target, platform, &data.join("skill-backups").join("overwritten"))?;
                fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
                copy_dir(Path::new(&item.path), &target)
            })(),
            ("import", "conflict") => Err("目标已有同名 Skill，请选择跳过或覆盖".into()),
            _ => Err("不支持的导入方式".into()),
        };
        match outcome {
            Ok(()) => {
                let path = target.display().to_string();
                if !cfg.imported.iter().any(|p| same_path(p, &target)) {
                    cfg.imported.push(path.clone());
                }
                let mut final_path = path;
                if !enable {
                    match set_enabled_impl(h, cfg, &root, &target, &skill_name(&target), false, data) {
                        Ok(p) => final_path = p,
                        Err(reason) => result.failed.push(ScanError { path: item.name.clone(), reason: format!("已导入，但未能设为停用：{reason}") }),
                    }
                }
                result.imported.push(final_path);
            }
            Err(reason) => result.failed.push(ScanError { path: item.name.clone(), reason }),
        }
    }
    Ok(result)
}

/* ───────────────────────── 项目目录设置 ───────────────────────── */

pub fn set_projects_with(cfg: &mut Config, projects: Vec<String>) -> Result<Vec<String>, String> {
    let mut list: Vec<String> = Vec::new();
    for p in projects {
        let path = PathBuf::from(p.trim());
        if !path.is_absolute() || has_parent_component(&path) || !path.is_dir() {
            return Err(format!("项目目录不存在：{}", path.display()));
        }
        if !list.iter().any(|known| same_path(known, &path)) {
            list.push(path.display().to_string());
        }
    }
    if list.len() > 50 {
        return Err("最多添加 50 个项目目录".into());
    }
    cfg.projects = list.clone();
    Ok(list)
}

/// 忽略 / 恢复一个从会话记录带出的项目；只改 skills.json，不碰会话数据
pub fn ignore_project_with(cfg: &mut Config, path: &str, ignored: bool) {
    cfg.ignored_projects.retain(|p| norm_key(p) != norm_key(path));
    if ignored {
        cfg.ignored_projects.push(path.to_string());
    }
}

/* ───────────────────────── Tauri 命令 ───────────────────────── */

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())?
}

/// 应用数据目录 + 合并了会话项目的配置
async fn context(app: &tauri::AppHandle, db: &State<'_, crate::DbRead>, force: bool) -> Result<(PathBuf, Config), String> {
    let dir = data_dir(app)?;
    let sessions = session_projects(db.0.clone(), force).await?;
    let mut cfg = load_config(&dir);
    cfg.session_projects = sessions;
    Ok((dir, cfg))
}

/// 列出 Skills。`platform` 为 all / claude / codex；`force` 时重新读取会话记录里的项目目录。
/// 模块 E 的「Skills（可选）」也可用它列出某个工具的可用项
#[tauri::command]
pub async fn skills_overview(app: tauri::AppHandle, db: State<'_, crate::DbRead>, platform: String, force: bool) -> Result<Overview, String> {
    let (_, cfg) = context(&app, &db, force).await?;
    blocking(move || overview_with(&homes()?, &cfg, &platform)).await
}

#[tauri::command]
pub async fn skill_detail(app: tauri::AppHandle, db: State<'_, crate::DbRead>, path: String) -> Result<Detail, String> {
    let (_, cfg) = context(&app, &db, false).await?;
    blocking(move || detail_with(&homes()?, &cfg, &path)).await
}

#[tauri::command]
pub async fn skill_file(app: tauri::AppHandle, db: State<'_, crate::DbRead>, path: String, rel: String) -> Result<FileContent, String> {
    let (_, cfg) = context(&app, &db, false).await?;
    blocking(move || file_with(&homes()?, &cfg, &path, &rel)).await
}

/// 启停这一份副本，返回启停后的目录（Claude 停用后位于停用区）
#[tauri::command]
pub async fn skill_set_enabled(app: tauri::AppHandle, db: State<'_, crate::DbRead>, path: String, enabled: bool) -> Result<String, String> {
    let (dir, mut cfg) = context(&app, &db, false).await?;
    blocking(move || {
        let h = homes()?;
        let (root, skill_dir) = locate(&h, &cfg, &path)?;
        let name = skill_name(&skill_dir);
        set_enabled_impl(&h, &mut cfg, &root, &skill_dir, &name, enabled, &dir)
    })
    .await
}

#[tauri::command]
pub async fn skill_save(app: tauri::AppHandle, db: State<'_, crate::DbRead>, input: SaveInput) -> Result<SaveResult, String> {
    let (dir, mut cfg) = context(&app, &db, false).await?;
    blocking(move || save_with(&homes()?, &mut cfg, input, &dir)).await
}

#[tauri::command]
pub async fn skill_copy(app: tauri::AppHandle, db: State<'_, crate::DbRead>, path: String, platform: String, scope: Scope, project: Option<String>, dry_run: bool) -> Result<CopyResult, String> {
    let (_, cfg) = context(&app, &db, false).await?;
    blocking(move || copy_with(&homes()?, &cfg, &path, &platform, scope, project.as_deref(), dry_run)).await
}

#[tauri::command]
pub async fn skill_remove(app: tauri::AppHandle, db: State<'_, crate::DbRead>, path: String) -> Result<String, String> {
    let (dir, mut cfg) = context(&app, &db, false).await?;
    blocking(move || remove_with(&homes()?, &mut cfg, &path, &dir)).await
}

#[tauri::command]
pub async fn skills_import_scan(app: tauri::AppHandle, db: State<'_, crate::DbRead>, source: String, platform: String, scope: Scope, project: Option<String>) -> Result<ImportScan, String> {
    let (_, cfg) = context(&app, &db, false).await?;
    blocking(move || import_scan_with(&homes()?, &cfg, &source, &platform, scope, project.as_deref())).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn skills_import(app: tauri::AppHandle, db: State<'_, crate::DbRead>, source: String, platform: String, scope: Scope, project: Option<String>, choices: Vec<ImportChoice>, enable: bool) -> Result<ImportResult, String> {
    let (dir, mut cfg) = context(&app, &db, false).await?;
    blocking(move || {
        let result = import_with(&homes()?, &mut cfg, &source, &platform, scope, project.as_deref(), choices, enable, &dir)?;
        save_config(&dir, &cfg)?;
        Ok(result)
    })
    .await
}

/// 「目录设置」：保存手动添加的项目目录（只接受已存在的绝对路径）
#[tauri::command]
pub async fn skills_set_projects(app: tauri::AppHandle, projects: Vec<String>) -> Result<Vec<String>, String> {
    let dir = data_dir(&app)?;
    blocking(move || {
        let mut cfg = load_config(&dir);
        let list = set_projects_with(&mut cfg, projects)?;
        save_config(&dir, &cfg)?;
        Ok(list)
    })
    .await
}

/// 「目录设置」：忽略或恢复一个从会话记录带出的项目
#[tauri::command]
pub async fn skills_ignore_project(app: tauri::AppHandle, path: String, ignored: bool) -> Result<(), String> {
    let dir = data_dir(&app)?;
    blocking(move || {
        let mut cfg = load_config(&dir);
        ignore_project_with(&mut cfg, &path, ignored);
        save_config(&dir, &cfg)
    })
    .await
}

#[tauri::command]
pub async fn skills_pick_folder(app: tauri::AppHandle, title: String) -> Result<Option<String>, String> {
    let window = app.get_webview_window(crate::MAIN).ok_or("请从主面板选择目录")?;
    let dialog = app.dialog().file().set_parent(&window).set_title(title);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder()).await.map_err(|e| e.to_string())?;
    match picked {
        None => Ok(None),
        Some(path) => path.into_path().map(|p| Some(p.display().to_string())).map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub async fn skill_open_dir(app: tauri::AppHandle, db: State<'_, crate::DbRead>, path: String) -> Result<(), String> {
    let (_, cfg) = context(&app, &db, false).await?;
    let target = blocking(move || locate(&homes()?, &cfg, &path).map(|(_, d)| d)).await?;
    app.opener().open_path(target.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jolu-skills-{tag}-{}-{}", std::process::id(), stamp()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn homes_at(base: &Path) -> Homes {
        Homes { claude: base.join(".claude"), codex: base.join(".codex"), user: base.to_path_buf() }
    }

    fn skill(root: &Path, name: &str, body: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(SKILL_FILE), body).unwrap();
        dir
    }

    fn body(name: &str) -> String {
        format!("---\nname: {name}\ndescription: {name} 描述\n---\n")
    }

    #[test]
    fn parses_frontmatter_and_validates() {
        let f = parse_front("---\nname: api-review\ndescription: \"核对 API\"\nallowed-tools: Read\n---\n# x").unwrap();
        assert_eq!(f.name.as_deref(), Some("api-review"));
        assert_eq!(f.description.as_deref(), Some("核对 API"));
        let folded = parse_front("---\ndescription: >\n  第一行\n  第二行\n---\n").unwrap();
        assert_eq!(folded.description.as_deref(), Some("第一行 第二行"));
        assert!(parse_front("# 没有 frontmatter").is_err());
        assert!(parse_front("---\nname: a\n").is_err());
        let no_name = Ok(Front { name: None, description: Some("d".into()) });
        assert_eq!(validate_front("claude", &no_name), None);
        assert_eq!(validate_front("codex", &no_name).as_deref(), Some("SKILL.md 缺少 name"));
        assert!(valid_name("api-review") && !valid_name("Api") && !valid_name("-a") && !valid_name("a/b") && !valid_name(""));
    }

    #[test]
    fn scans_conflicts_invalid_and_tool_overrides() {
        let base = temp("scan");
        let h = homes_at(&base);
        let project = base.join("proj");
        let global = h.claude.join("skills");
        skill(&global, "frontend-design", &body("frontend-design"));
        skill(&global, "api-contract", "---\nname: api-contract\n---\n");
        skill(&global, "legacy", &body("legacy"));
        skill(&global.join("synced"), "synced-one", &body("synced-one"));
        skill(&project.join(".claude").join("skills"), "frontend-design", &body("frontend-design"));
        let settings = r#"{"skillOverrides":{"legacy":"off"}}"#;
        fs::write(h.claude.join("settings.json"), settings).unwrap();
        let cfg = Config { projects: vec![project.display().to_string()], ..Default::default() };
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let get = |name: &str, scope: Scope| o.items.iter().find(|i| i.name == name && i.scope == scope).unwrap();
        assert_eq!(get("frontend-design", Scope::Global).status, "enabled");
        let project_item = get("frontend-design", Scope::Project);
        assert_eq!((project_item.status, project_item.issue.as_deref()), ("conflict", Some("同名条目：项目与全局同时存在")));
        assert_eq!(get("api-contract", Scope::Global).status, "invalid");
        let legacy = get("legacy", Scope::Global);
        assert_eq!((legacy.status, legacy.disabled_by, legacy.enabled), ("disabled", Some("tool"), false));
        assert!(legacy.issue.as_deref().unwrap().contains("由工具配置停用"));
        assert!(get("synced-one", Scope::Synced).readonly);
        // 用户写的 skillOverrides 不允许 Jolu 改：启用报错，文件原样
        let mut cfg = cfg;
        let (root, dir) = locate(&h, &cfg, &legacy.path).unwrap();
        assert!(set_enabled_impl(&h, &mut cfg, &root, &dir, "legacy", true, &base.join("data")).unwrap_err().contains("由工具配置停用"));
        assert_eq!(fs::read_to_string(h.claude.join("settings.json")).unwrap(), settings);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn claude_disable_parks_only_this_copy() {
        let base = temp("park");
        let h = homes_at(&base);
        let data = base.join("data");
        let project = base.join("proj");
        skill(&h.claude.join("skills"), "docs-writer", &body("docs-writer"));
        let local = skill(&project.join(".claude").join("skills"), "docs-writer", &body("docs-writer"));
        fs::write(local.join("notes.md"), "附带文件").unwrap();
        let mut cfg = Config { projects: vec![project.display().to_string()], ..Default::default() };

        let (root, dir) = locate(&h, &cfg, &local.display().to_string()).unwrap();
        let stored = set_enabled_impl(&h, &mut cfg, &root, &dir, "docs-writer", false, &data).unwrap();
        assert!(!local.exists() && Path::new(&stored).join("notes.md").is_file() && Path::new(&stored).join(SKILL_FILE).is_file());
        assert!(stored.starts_with(&data.join("skills-disabled").display().to_string()));
        assert!(!h.claude.join("settings.json").exists(), "不写 skillOverrides");
        assert_eq!(load_config(&data).disabled.len(), 1);
        assert!(fs::read_dir(data.join("skill-backups").join("disabled")).unwrap().count() == 1);

        let o = overview_with(&h, &cfg, "claude").unwrap();
        let global_item = o.items.iter().find(|i| i.scope == Scope::Global).unwrap();
        assert_eq!(global_item.status, "enabled", "同名的全局副本不受影响");
        let parked = o.items.iter().find(|i| i.disabled_by == Some("parked")).unwrap();
        assert_eq!((parked.status, parked.original_path.as_deref()), ("disabled", Some(local.display().to_string().as_str())));
        assert!(detail_with(&h, &cfg, &parked.path).is_ok());

        // 原位置被占用时拒绝启用，两边文件都保留
        fs::create_dir_all(&local).unwrap();
        let (root, dir) = locate(&h, &cfg, &stored).unwrap();
        assert!(set_enabled_impl(&h, &mut cfg, &root, &dir, "docs-writer", true, &data).unwrap_err().contains("原位置已有同名目录"));
        assert!(Path::new(&stored).join(SKILL_FILE).is_file());
        fs::remove_dir(&local).unwrap();
        let back = set_enabled_impl(&h, &mut cfg, &root, &dir, "docs-writer", true, &data).unwrap();
        assert!(same_path(&back, &local) && local.join(SKILL_FILE).is_file() && !Path::new(&stored).exists());
        assert!(load_config(&data).disabled.is_empty());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn codex_toggles_by_path_and_keeps_name_entries() {
        let base = temp("codex");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let a = skill(&base.join(".agents").join("skills"), "test-runner", &body("test-runner"));
        skill(&h.codex.join("skills"), "test-runner", &body("test-runner"));
        skill(&h.codex.join("skills"), "legacy", &body("legacy"));
        fs::create_dir_all(&h.codex).unwrap();
        fs::write(h.codex.join("config.toml"), "# 我的配置\nmodel = \"gpt-5\"\n\n[[skills.config]]\nname = \"legacy\"\nenabled = false\n").unwrap();
        let (root, dir) = locate(&h, &cfg, &a.display().to_string()).unwrap();
        set_enabled_impl(&h, &mut cfg, &root, &dir, "test-runner", false, &data).unwrap();
        let text = fs::read_to_string(h.codex.join("config.toml")).unwrap();
        assert!(text.contains("# 我的配置") && text.contains("name = \"legacy\"") && text.contains("enabled = false"));
        let o = overview_with(&h, &cfg, "codex").unwrap();
        let states: Vec<_> = o.items.iter().map(|i| (i.name.as_str(), i.path == a.display().to_string(), i.disabled_by)).collect();
        assert!(states.contains(&("test-runner", true, Some("path"))));
        assert!(states.contains(&("test-runner", false, None)), "另一份同名副本保持启用");
        assert!(states.contains(&("legacy", false, Some("tool"))));
        let legacy = o.items.iter().find(|i| i.name == "legacy").unwrap();
        let (lroot, ldir) = locate(&h, &cfg, &legacy.path).unwrap();
        assert!(set_enabled_impl(&h, &mut cfg, &lroot, &ldir, "legacy", true, &data).is_err());
        set_enabled_impl(&h, &mut cfg, &root, &dir, "test-runner", true, &data).unwrap();
        let text = fs::read_to_string(h.codex.join("config.toml")).unwrap();
        assert!(text.contains("name = \"legacy\"") && !text.contains("test-runner"));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn merges_session_projects_and_ignores() {
        let base = temp("projects");
        let h = homes_at(&base);
        let with_skills = base.join("WithSkills");
        skill(&with_skills.join(".agents").join("skills"), "repo-skill", &body("repo-skill"));
        let plain = base.join("plain");
        fs::create_dir_all(&plain).unwrap();
        let alt = with_skills.display().to_string().replace('\\', "/").to_uppercase();
        let mut cfg = Config {
            projects: vec![with_skills.display().to_string()],
            session_projects: vec![alt, plain.display().to_string(), base.join("gone").display().to_string(), base.display().to_string()],
            ..Default::default()
        };
        let all = cfg.all_projects(&h);
        assert_eq!(all.len(), 2, "大小写与分隔符去重、跳过不存在目录和主目录");
        assert_eq!((all[0].source, all[0].codex, all[0].claude), ("manual", true, false));
        assert_eq!((all[1].source, all[1].codex), ("session", false));
        let o = overview_with(&h, &cfg, "codex").unwrap();
        assert!(o.roots.iter().all(|r| r.scope != Scope::Project || r.exists), "没有 Skills 目录的项目不扫描");
        assert_eq!(o.items.iter().filter(|i| i.scope == Scope::Project).count(), 1);
        // 没有 Skills 目录的会话项目仍可作为新建目标
        let input = SaveInput { platform: "claude".into(), scope: Scope::Project, project: Some(plain.display().to_string()), name: "new-one".into(), content: body("new-one"), original_path: None, expected_modified_ms: None, enable: true };
        assert!(save_with(&h, &mut cfg, input, &base.join("data")).is_ok());
        ignore_project_with(&mut cfg, &plain.display().to_string().to_uppercase(), true);
        let plain_info = cfg.all_projects(&h).into_iter().find(|p| p.source == "session").unwrap();
        assert!(plain_info.ignored);
        assert!(check_project(&h, &cfg, Some(&plain.display().to_string())).is_err());
        ignore_project_with(&mut cfg, &plain.display().to_string(), false);
        assert!(cfg.ignored_projects.is_empty());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn save_detects_conflicts_and_external_changes() {
        let base = temp("save");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let content = "---\nname: api-review\ndescription: 核对\n---\n# API Review\n";
        let input = |original: Option<String>, expected: Option<i64>, body: &str, enable: bool| SaveInput {
            platform: "claude".into(),
            scope: Scope::Global,
            project: None,
            name: "api-review".into(),
            content: body.into(),
            original_path: original,
            expected_modified_ms: expected,
            enable,
        };
        let saved = save_with(&h, &mut cfg, input(None, None, content, true), &data).unwrap();
        assert!(save_with(&h, &mut cfg, input(None, None, content, true), &data).unwrap_err().contains("同名"));
        let edited = save_with(&h, &mut cfg, input(Some(saved.path.clone()), saved.modified_ms, &content.replace("核对", "检查"), true), &data).unwrap();
        assert!(edited.backup.is_some());
        assert_eq!(save_with(&h, &mut cfg, input(Some(saved.path.clone()), Some(1), content, true), &data).unwrap_err(), EXTERNAL_CHANGE);
        assert!(save_with(&h, &mut cfg, input(None, None, "---\nname: other\ndescription: d\n---\n", true), &data).unwrap_err().contains("不一致"));
        let mut bad = input(None, None, content, true);
        bad.scope = Scope::Project;
        bad.project = Some(base.join("unknown").display().to_string());
        assert!(save_with(&h, &mut cfg, bad, &data).unwrap_err().contains("目录设置"));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn rejects_paths_outside_roots() {
        let base = temp("paths");
        let h = homes_at(&base);
        let cfg = Config::default();
        let dir = skill(&h.claude.join("skills"), "safe", "---\ndescription: d\n---\n");
        let outside = skill(&base.join("elsewhere"), "evil", "---\ndescription: d\n---\n");
        assert!(locate(&h, &cfg, &dir.display().to_string()).is_ok());
        assert!(locate(&h, &cfg, &outside.display().to_string()).is_err());
        assert!(locate(&h, &cfg, &format!("{}/../elsewhere/evil", dir.display())).is_err());
        assert!(locate(&h, &cfg, "relative/path").is_err());
        assert!(file_with(&h, &cfg, &dir.display().to_string(), "../../elsewhere/evil/SKILL.md").is_err());
        assert!(file_with(&h, &cfg, &dir.display().to_string(), "SKILL.md").unwrap().text.is_some());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn imports_disabled_into_parking_and_removes_with_backup() {
        let base = temp("import");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let source = base.join("downloads");
        skill(&source, "api-review", &body("api-review"));
        skill(&source, "frontend-design", &body("frontend-design"));
        skill(&source, "quick-test", "---\nname: quick-test\n---\n");
        skill(&h.claude.join("skills"), "frontend-design", &body("frontend-design"));
        let src = source.display().to_string();
        let scan = import_scan_with(&h, &cfg, &src, "claude", Scope::Global, None).unwrap();
        let status: Vec<_> = scan.items.iter().map(|i| (i.name.as_str(), i.status)).collect();
        assert_eq!(status, vec![("api-review", "ok"), ("frontend-design", "conflict"), ("quick-test", "invalid")]);
        let choices = vec![
            ImportChoice { name: "api-review".into(), action: "import".into() },
            ImportChoice { name: "frontend-design".into(), action: "skip".into() },
            ImportChoice { name: "quick-test".into(), action: "import".into() },
        ];
        let result = import_with(&h, &mut cfg, &src, "claude", Scope::Global, None, choices, false, &data).unwrap();
        assert_eq!((result.imported.len(), result.skipped.len(), result.failed.len()), (1, 1, 1));
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let imported = o.items.iter().find(|i| i.name == "api-review").unwrap();
        assert_eq!((imported.source, imported.status, imported.disabled_by), ("imported", "disabled", Some("parked")));
        let backup = remove_with(&h, &mut cfg, &imported.path, &data).unwrap();
        assert!(Path::new(&backup).join(SKILL_FILE).is_file());
        assert!(!Path::new(&imported.path).exists() && cfg.disabled.is_empty());
        let copy = copy_with(&h, &cfg, &o.items.iter().find(|i| i.name == "frontend-design").unwrap().path, "codex", Scope::Global, None, false).unwrap();
        assert!(copy.copied && Path::new(&copy.target).join(SKILL_FILE).is_file());
        fs::remove_dir_all(&base).ok();
    }
}
