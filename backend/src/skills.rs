//! 模块 F · Skills 管理（画板 74–85）：按供应商扫描本机 Skill 目录，查看、新建 / 编辑、导入、复制、启停与移除。
//!
//! 目录规则按官方文档核对（Context7）：
//! - Claude Code：个人 `~/.claude/skills/<name>/SKILL.md`（尊重 `CLAUDE_CONFIG_DIR`），项目 `<项目>/.claude/skills/`；
//!   `skills/synced/` 是从 claude.ai 同步来的，只读。
//! - Codex：用户 `~/.agents/skills` 与旧位置 `$CODEX_HOME/skills`，系统 `$CODEX_HOME/skills/.system`（只读），
//!   项目 `<项目>/.agents/skills` 与 `<项目>/.codex/skills`。
//! 其他工具没有可靠依据，前端标「待接入」。各工具的目录、启停方式与写入能力集中在 `SPECS` 规则表（D-19）。
//!
//! 「更换全局目录」（D-18）只改 Jolu 扫描和写入的工具主目录（存进 `skills.json` 的 `global_override`），
//! 不改工具的环境变量与配置文件位置；Codex 的 `~/.agents/skills` 不受 `CODEX_HOME` 影响，照常扫描。
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
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

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

/* ───────────────────────── 工具规则表（D-19） ───────────────────────── */

/// 启停方式
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Toggle {
    /// 把这一份移到 Jolu 停用区（Claude Code）；同时只读识别 settings.json 里用户写的 `skillOverrides`
    Park,
    /// Codex `config.toml` 的 `[[skills.config]]`，按 SKILL.md 路径停用
    CodexToml,
    /// 任务 4.2：`~/.gemini/settings.json` 的 `skills.disabled[]`
    #[allow(dead_code)]
    GeminiSettings,
    /// 任务 4.2：`opencode.json` 的 `permission.skill.<名称> = "deny"`
    #[allow(dead_code)]
    OpenCodePermission,
    /// 不提供启停
    #[allow(dead_code)]
    None,
}

/// 一个工具的 Skills 目录规则。路径用 `/` 分隔，拼接时逐段展开（不混用分隔符）。
///
/// 约定：
/// - `global[0]` 是新建、导入、复制到全局时的目标目录；
/// - `global` 的最后一项是工具自己的主目录：`home_env` 设了环境变量时为 `$ENV/<子目录>`，否则为用户目录下的默认位置，
///   默认位置去掉末尾的 `/<子目录>` 就是工具配置所在目录（如 `~/.claude/settings.json`、`~/.codex/config.toml`）；
///   「更换全局目录」（D-18）替换的也是这一项，前面的项（如 Codex 的 `~/.agents/skills`）不受影响；
/// - `readonly_subdirs` 是主目录下由工具托管的只读子目录，单独作为根目录扫描，扫描主目录时跳过。
pub struct ToolSpec {
    pub id: &'static str,
    /// 给用户看的名称（报错文案用）
    pub name: &'static str,
    /// 相对用户目录，如 ".claude/skills"
    pub global: &'static [&'static str],
    /// 相对项目根，如 ".claude/skills"
    pub project: &'static [&'static str],
    /// (环境变量, 其下子目录)，如 ("CLAUDE_CONFIG_DIR", "skills")
    pub home_env: Option<(&'static str, &'static str)>,
    pub toggle: Toggle,
    pub writable: bool,
    /// SKILL.md 必须写 name（Codex）；否则缺 name 时用目录名
    pub require_name: bool,
    /// 目录规则已按官方文档核实
    pub verified: bool,
    pub readonly_subdirs: &'static [(&'static str, Scope)],
}

/// 已接入的工具；其余供应商在前端标「待接入」（任务 4.2 继续添加）
pub const SPECS: &[ToolSpec] = &[
    ToolSpec {
        id: "claude",
        name: "Claude Code",
        global: &[".claude/skills"],
        project: &[".claude/skills"],
        home_env: Some(("CLAUDE_CONFIG_DIR", "skills")),
        toggle: Toggle::Park,
        writable: true,
        require_name: false,
        verified: true,
        readonly_subdirs: &[("synced", Scope::Synced)],
    },
    ToolSpec {
        id: "codex",
        name: "Codex",
        // `~/.agents/skills` 是首选位置，`$CODEX_HOME/skills` 是旧位置；`~/.agents/skills` 不受 CODEX_HOME 影响
        global: &[".agents/skills", ".codex/skills"],
        project: &[".agents/skills", ".codex/skills"],
        home_env: Some(("CODEX_HOME", "skills")),
        toggle: Toggle::CodexToml,
        writable: true,
        require_name: true,
        verified: true,
        readonly_subdirs: &[(".system", Scope::System)],
    },
    ToolSpec { id:"gemini", name:"Gemini", global:&[".agents/skills",".gemini/skills"], project:&[".gemini/skills"], home_env:None, toggle:Toggle::GeminiSettings, writable:true, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"opencode", name:"OpenCode", global:&[".claude/skills",".agents/skills",".config/opencode/skills"], project:&[".opencode/skills",".claude/skills",".agents/skills"], home_env:None, toggle:Toggle::OpenCodePermission, writable:true, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"qoder", name:"Qoder", global:&[".qoder-cn/skills",".qoder/skills"], project:&[".qoder/skills"], home_env:None, toggle:Toggle::None, writable:true, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"trae", name:"Trae", global:&[".trae/skills",".trae-cn/skills"], project:&[".trae/skills"], home_env:None, toggle:Toggle::None, writable:true, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"zcode", name:"ZCode", global:&[".zcode/skills"], project:&[".zcode/skills"], home_env:None, toggle:Toggle::None, writable:true, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"workbuddy", name:"WorkBuddy", global:&[".codebuddy/skills"], project:&[".codebuddy/skills"], home_env:None, toggle:Toggle::None, writable:false, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"custom", name:"自定义", global:&[], project:&[], home_env:None, toggle:Toggle::None, writable:false, require_name:false, verified:false, readonly_subdirs:&[] },
];

pub fn spec(id: &str) -> Option<&'static ToolSpec> {
    SPECS.iter().find(|s| s.id == id)
}

/// 按 `/` 逐段拼到 `base` 下
fn rel(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').filter(|s| !s.is_empty()).fold(base.to_path_buf(), |path, seg| path.join(seg))
}

/// `home_env` 的环境变量名与未设置时的默认目录（相对用户目录）：`global` 最后一项去掉 `/<子目录>`
fn env_default(spec: &ToolSpec) -> Option<(&'static str, &'static str)> {
    let (name, sub) = spec.home_env?;
    let own = spec.global.last()?;
    own.strip_suffix(sub)?.strip_suffix('/').map(|dir| (name, dir))
}

#[derive(Clone, Debug)]
struct Root {
    platform: &'static str,
    scope: Scope,
    project: Option<PathBuf>,
    path: PathBuf,
    readonly: bool,
    /// 扫描时跳过的子目录（它们是单独的根目录）
    skip: Vec<&'static str>,
    /// 停用区里登记的副本
    parked: bool,
    /// 工具主目录（可在 Jolu 里更换）
    own: bool,
}

#[derive(Clone, Debug)]
pub struct Homes {
    pub user: PathBuf,
    /// `home_env` 的环境变量 → 工具配置所在目录（环境变量的值，或用户目录下的默认位置）
    pub env: HashMap<String, PathBuf>,
}

fn homes_from(user: PathBuf, var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Homes {
    let mut env = HashMap::new();
    for spec in SPECS {
        if let Some((name, default)) = env_default(spec) {
            let dir = var(name).map(PathBuf::from).unwrap_or_else(|| rel(&user, default));
            env.insert(name.to_string(), dir);
        }
    }
    Homes { user, env }
}

/// 隔离验收环境：用户目录换成 `<JOLU_TEST_DIR>/skills-home`，不读环境变量
fn isolated_homes(test_dir: &Path) -> Homes {
    homes_from(test_dir.join("skills-home"), |_| None)
}

fn homes() -> Result<Homes, String> {
    // 隔离验收环境不碰真实用户目录，也不读环境变量
    if let Some(dir) = crate::validation::data_dir() {
        return Ok(isolated_homes(&dir));
    }
    let user = crate::creds::home().ok_or("未找到用户主目录")?;
    Ok(homes_from(user, |name| std::env::var_os(name)))
}

/// 工具配置所在目录（如 `~/.claude`、`$CODEX_HOME`）；不受「更换全局目录」影响
fn tool_home(h: &Homes, spec: &ToolSpec) -> PathBuf {
    if let Some(dir) = spec.home_env.and_then(|(name, _)| h.env.get(name)) {
        return dir.clone();
    }
    let own = spec.global.last().copied().unwrap_or_default();
    rel(&h.user, own.rsplit_once('/').map_or(own, |(dir, _)| dir))
}

/// 工具主目录的默认位置（没有在 Jolu 里更换时）
fn default_global(h: &Homes, spec: &ToolSpec) -> PathBuf {
    match spec.home_env {
        Some((_, sub)) if env_default(spec).is_some() => rel(&tool_home(h, spec), sub),
        _ => rel(&h.user, spec.global.last().copied().unwrap_or_default()),
    }
}

/// 工具主目录：「更换全局目录」设置过时用它，否则用默认位置
fn own_global(h: &Homes, cfg: &Config, spec: &ToolSpec) -> PathBuf {
    cfg.global_override.get(spec.id).map(PathBuf::from).unwrap_or_else(|| default_global(h, spec))
}

/// 已保存的覆盖路径失效（不是绝对路径或目录已不存在）时返回它：这时主目录只读，不会被写入时静默重建
fn broken_override<'a>(cfg: &'a Config, spec: &ToolSpec) -> Option<&'a str> {
    let path = cfg.global_override.get(spec.id)?;
    let dir = Path::new(path);
    (!dir.is_absolute() || has_parent_component(dir) || !dir.is_dir()).then_some(path.as_str())
}

fn broken_override_reason(path: &str) -> String {
    format!("已更换的目录不存在：{path}")
}

fn roots(h: &Homes, cfg: &Config, platform: &str, projects: &[PathBuf]) -> Vec<Root> {
    let Some(spec) = spec(platform) else { return Vec::new() };
    let root = |scope, project: Option<&PathBuf>, path: PathBuf, readonly, skip: Vec<&'static str>, own| Root {
        platform: spec.id,
        scope,
        project: project.cloned(),
        path,
        readonly,
        skip,
        parked: false,
        own,
    };
    if platform == "custom" { return cfg.custom_dirs.iter().map(|p|root(Scope::Global,None,PathBuf::from(p),true,vec![],false)).collect(); }
    let mut out = Vec::new();
    let last = spec.global.len().saturating_sub(1);
    for (i, dir) in spec.global.iter().enumerate() {
        if i < last {
            out.push(root(Scope::Global, None, rel(&h.user, dir), !spec.writable || matches!(platform,"gemini"|"opencode"), Vec::new(), false));
            continue;
        }
        let path = own_global(h, cfg, spec);
        let skip = spec.readonly_subdirs.iter().map(|(sub, _)| *sub).collect();
        let readonly = !spec.writable || broken_override(cfg, spec).is_some();
        out.push(root(Scope::Global, None, path.clone(), readonly, skip, true));
        for (sub, scope) in spec.readonly_subdirs {
            out.push(root(*scope, None, rel(&path, sub), true, Vec::new(), false));
        }
    }
    for p in projects {
        for dir in spec.project {
            let shared = platform == "opencode" && *dir != ".opencode/skills";
            out.push(root(Scope::Project, Some(p), rel(p, dir), !spec.writable || shared, Vec::new(), false));
        }
    }
    out
}

/// 新建、导入、复制的目标目录：每个范围取首选位置（Codex 用户级用 `~/.agents/skills`，`$CODEX_HOME/skills` 已标记为旧位置）
fn write_root(h: &Homes, cfg: &Config, platform: &str, scope: Scope, project: Option<&Path>) -> Result<Root, String> {
    let project = match scope {
        Scope::Global => None,
        Scope::Project => Some(project.ok_or("请选择项目目录")?.to_path_buf()),
        _ => return Err("只读范围不能写入".into()),
    };
    let list = roots(h, cfg, platform, project.as_slice());
    list.into_iter().find(|r| r.scope == scope && !r.readonly).ok_or_else(|| {
        match (scope, spec(platform).and_then(|s| broken_override(cfg, s))) {
            (Scope::Global, Some(path)) => broken_override_reason(path),
            _ => format!("{platform} 暂不支持写入 Skills"),
        }
    })
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
    /// 「更换全局目录」（D-18）：工具 id → Jolu 扫描与写入的全局目录，替换工具主目录；不改工具本身的环境变量
    #[serde(default)]
    pub global_override: HashMap<String, String>,
    /// 任务 4.2：「自定义」扫描目录（只扫描，不提供启停）
    #[serde(default)]
    pub custom_dirs: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProjectInfo {
    pub path: String,
    /// manual：手动添加；session：来自会话记录
    pub source: &'static str,
    pub ignored: bool,
    /// 含有项目级 Skills 目录的工具 id（按 `SPECS` 顺序）
    pub tools: Vec<&'static str>,
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
                tools: SPECS.iter().filter(|s| s.project.iter().any(|d| rel(dir, d).is_dir())).map(|s| s.id).collect(),
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

type FileSignature = (u64, Option<std::time::SystemTime>);

/// 读不懂的 skills.json：配置目录 → (文件大小与修改时间, 拒绝保存时的说明)。
/// 读取时登记并备份一次；文件修好后下次读取即清除。登记期间所有保存都被拒绝，避免用空配置覆盖掉停用登记
static BROKEN_CONFIG: LazyLock<Mutex<HashMap<String, (FileSignature, String)>>> = LazyLock::new(Default::default);

fn file_signature(path: &Path) -> FileSignature {
    fs::metadata(path).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None))
}

fn parse_config(path: &Path) -> Result<Config, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e.to_string()),
    };
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok(Config::default());
    }
    serde_json::from_str(text).map_err(|e| e.to_string())
}

/// 损坏时先把原文件复制为 `skills.json.broken-<时间戳>`（同一份损坏内容只备份一次），本次按空配置继续读取扫描
fn mark_broken(key: String, path: &Path) {
    let signature = file_signature(path);
    let Ok(mut broken) = BROKEN_CONFIG.lock() else { return };
    if broken.get(&key).is_some_and(|(known, _)| *known == signature) {
        return;
    }
    let backup = path.with_file_name(format!("skills.json.broken-{}", stamp()));
    let message = match fs::copy(path, &backup) {
        Ok(_) => format!("Skills 配置文件损坏，已备份为 {}，请检查后重试", backup.display()),
        Err(e) => format!("Skills 配置文件损坏且无法备份（{e}），请检查 {} 后重试", path.display()),
    };
    broken.insert(key, (signature, message));
}

fn load_config(dir: &Path) -> Config {
    let path = dir.join("skills.json");
    let key = norm_key(&dir.to_string_lossy());
    match parse_config(&path) {
        Ok(cfg) => {
            if let Ok(mut broken) = BROKEN_CONFIG.lock() {
                broken.remove(&key);
            }
            cfg
        }
        Err(error) => {
            if let Err(io) = fs::read(&path) {
                if let Ok(mut broken) = BROKEN_CONFIG.lock() { broken.insert(key, (file_signature(&path), format!("无法读取 Skills 配置：{io}（{error}）；未修改原文件"))); }
                return Config::default();
            }
            mark_broken(key, &path);
            Config::default()
        }
    }
}

/// skills.json 损坏时拒绝一切写入
fn config_guard(dir: &Path) -> Result<(), String> {
    let key = norm_key(&dir.to_string_lossy());
    match BROKEN_CONFIG.lock() {
        Ok(broken) => broken.get(&key).map_or(Ok(()), |(_, message)| Err(message.clone())),
        Err(_) => Ok(()),
    }
}

fn save_config(dir: &Path, cfg: &Config) -> Result<(), String> {
    config_guard(dir)?;
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

/// 格式问题；None 表示有效。都要求 description；`require_name` 的工具（Codex）还要求 name，其余缺 name 时用目录名
fn validate_front(platform: &str, front: &Result<Front, String>) -> Option<String> {
    match front {
        Err(reason) => Some(reason.clone()),
        Ok(f) if f.description.is_none() => Some("SKILL.md 缺少 description".into()),
        Ok(f) if f.name.is_none() && spec(platform).is_some_and(|s| s.require_name) => Some("SKILL.md 缺少 name".into()),
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
    /// 工具主目录，可在「扫描目录设置」里更换（D-18）
    pub overridable: bool,
    /// 已更换为其他目录
    pub overridden: bool,
    /// 已更换时的默认位置，「恢复默认」后回到这里
    pub default_path: Option<String>,
    /// 停用区里原位置在这个目录下的 Skill 数；更换目录后它们启用时仍回到这里
    pub parked_here: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct PlatformSummary {
    pub id: String,
    /// 至少有一个 Skills 根目录存在
    pub detected: bool,
    pub count: usize,
    /// 目录规则已按官方文档核实
    pub verified: bool,
    /// 新建、导入、复制到全局时的目标目录；工具不支持写入时为 None
    pub global_dir: Option<String>,
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
fn claude_tool_off(h: &Homes, spec: &ToolSpec, root: &Root, name: &str) -> bool {
    let mut files = Vec::new();
    if let (Scope::Project, Some(p)) = (root.scope, &root.project) {
        files.push(p.join(".claude").join("settings.local.json"));
        files.push(p.join(".claude").join("settings.json"));
    }
    files.push(tool_home(h, spec).join("settings.json"));
    files.iter().find_map(|f| claude_override(f, name)).as_deref() == Some("off")
}

fn codex_config_path(h: &Homes, spec: &ToolSpec) -> PathBuf {
    tool_home(h, spec).join("config.toml")
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

// 名称级开关会作用于同名副本；遇到无法解释的配置拒绝改写。
fn json_skill_path(h: &Homes, spec: &ToolSpec, root: &Root) -> PathBuf {
    match (spec.toggle, root.project.as_ref()) {
        (Toggle::GeminiSettings, Some(p)) => p.join(".gemini/settings.json"),
        (Toggle::OpenCodePermission, Some(p)) => p.join("opencode.json"),
        (Toggle::GeminiSettings, None) => tool_home(h,spec).join("settings.json"),
        _ => tool_home(h,spec).join("opencode.json"),
    }
}
fn read_skill_json(path: &Path) -> Result<serde_json::Value,String> {
    // 不把 JSONC 当作空配置覆盖；保留原文件让用户在原工具处理。
    if path.with_extension("jsonc").exists() { return Err("检测到 JSONC 配置，请在原工具中管理 Skills 启停".into()); }
    match fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(s.trim_start_matches('\u{feff}')).map_err(|_|format!("{} 格式无法解析，未修改",path.display())),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(e) => Err(format!("无法读取 {}：{e}",path.display())),
    }
}
fn json_skill_off(h:&Homes, spec:&ToolSpec, root:&Root, name:&str) -> Result<bool,String> {
    let path=json_skill_path(h,spec,root);
    let v=read_skill_json(&path)?;
    let local=match spec.toggle {
        Toggle::GeminiSettings => v.pointer("/skills/enabled")==Some(&serde_json::json!(false)) || v.pointer("/skills/disabled").and_then(|x|x.as_array()).is_some_and(|a|a.iter().any(|n|n.as_str()==Some(name))),
        Toggle::OpenCodePermission => {
            let p=v.pointer("/permission/skill");
            if p.and_then(|p|p.as_object()).is_some_and(|m|m.keys().any(|k|k!="*"&&(k.contains('*')||k.contains('?')))){return Err("存在无法静态判定的 Skills 通配符规则，请在原工具核对".into())}
            let rule=p.and_then(|p|p.get(name)).or_else(||p.and_then(|p|p.get("*"))).or(p.filter(|p|p.is_string())).and_then(|p|p.as_str());
            if rule.is_none() && root.project.is_some(){let mut global=root.clone();global.project=None;return json_skill_off(h,spec,&global,name)}
            rule.is_some_and(|p|p!="allow")
        }, _=>false
    };
    if !local && root.project.is_some() && spec.toggle==Toggle::GeminiSettings {
        let mut global=root.clone();global.project=None; return json_skill_off(h,spec,&global,name);
    }
    Ok(local)
}
fn set_json_skill(h:&Homes,spec:&ToolSpec,root:&Root,name:&str,enabled:bool,data:&Path)->Result<(),String>{
    let path=json_skill_path(h,spec,root); let mut v=read_skill_json(&path)?;
    let object=v.as_object_mut().ok_or("配置根节点必须是对象，未修改")?;
    if spec.toggle==Toggle::GeminiSettings {
        let skills=object.entry("skills").or_insert_with(||serde_json::json!({})).as_object_mut().ok_or("skills 格式无效")?;
        if enabled && skills.get("enabled")==Some(&serde_json::json!(false)){return Err("Skills 总开关已关闭，请在原工具中启用".into())}
        let disabled=skills.entry("disabled").or_insert_with(||serde_json::json!([])).as_array_mut().ok_or("skills.disabled 必须是数组")?;
        disabled.retain(|v|v.as_str()!=Some(name)); if !enabled {disabled.push(serde_json::json!(name));}
        if enabled && root.project.is_some(){let mut global=root.clone();global.project=None;if json_skill_off(h,spec,&global,name)?{return Err("全局配置已停用此 Skill，请先在全局范围启用".into())}}
    }else{
        let permission=object.entry("permission").or_insert_with(||serde_json::json!({})).as_object_mut().ok_or("permission 为全局规则，请在原工具中修改")?;
        let skills=permission.entry("skill").or_insert_with(||serde_json::json!({})).as_object_mut().ok_or("permission.skill 为全局规则，请在原工具中修改")?;
        if skills.keys().any(|k|k!=name&&(k.contains('*')||k.contains('?'))) { return Err("存在通配符权限规则，请在原工具中管理，避免改变规则优先级".into()); }
        if enabled {
            if root.project.is_some(){let mut global=root.clone();global.project=None;if json_skill_off(h,spec,&global,name)?{return Err("全局权限仍限制此 Skill，请在原工具中调整".into())}}
            skills.remove(name);
        }else{skills.insert(name.into(),serde_json::json!("deny"));}
    }
    backup_file(&path,&data.join("skill-backups/config"))?;
    if let Some(parent)=path.parent(){fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
    write_atomic(&path,serde_json::to_string_pretty(&v).map_err(|e|e.to_string())?.as_bytes())
}

/// 启停这一份副本，返回启停后的目录
fn set_enabled_impl(h: &Homes, cfg: &mut Config, root: &Root, dir: &Path, name: &str, enabled: bool, data: &Path) -> Result<String, String> {
    let spec = spec(root.platform).ok_or_else(|| format!("{} 暂不支持启停 Skills", root.platform))?;
    match spec.toggle {
        Toggle::Park => match (enabled, root.parked) {
            (true, true) => unpark(cfg, dir, data),
            (false, false) => park(cfg, root, dir, name, data),
            (true, false) if claude_tool_off(h, spec, root, name) => Err(TOOL_DISABLED.into()),
            _ => Ok(dir.display().to_string()),
        },
        Toggle::CodexToml => {
            let path = codex_config_path(h, spec);
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
        Toggle::GeminiSettings | Toggle::OpenCodePermission => { set_json_skill(h,spec,root,name,enabled,data)?; Ok(dir.display().to_string()) },
        Toggle::None => Err(format!("{} 的启停由原工具管理", spec.name)),
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

fn collect(h: &Homes, cfg: &Config, spec: &'static ToolSpec, errors: &mut Vec<ScanError>) -> (Vec<SkillItem>, Vec<RootInfo>) {
    let platform = spec.id;
    let projects = cfg.project_paths(h);
    // 没有对应 Skills 目录的项目不进入扫描结果
    let list: Vec<Root> = roots(h, cfg, platform, &projects).into_iter().filter(|r| r.scope != Scope::Project || r.path.is_dir()).collect();
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
            platform,
            scope: entry.scope,
            project: entry.project.as_ref().map(PathBuf::from),
            path: dir.parent().map(Path::to_path_buf).unwrap_or_default(),
            readonly: false,
            skip: Vec::new(),
            parked: true,
            own: false,
        };
        let dir_name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let front = read_limited(&dir.join(SKILL_FILE), MAX_SKILL_BYTES).and_then(|(t, _)| parse_front(&t));
        raws.push(Raw { root, dir, dir_name, front, original: Some(entry.original.clone()) });
    }
    if let Some(path) = broken_override(cfg, spec) {
        errors.push(ScanError { path: path.to_string(), reason: broken_override_reason(path) });
    }
    let overridden = cfg.global_override.contains_key(platform);
    let infos = list
        .iter()
        .map(|r| RootInfo {
            platform: r.platform.into(),
            scope: r.scope,
            project: r.project.as_ref().map(|p| p.display().to_string()),
            path: r.path.display().to_string(),
            exists: r.path.is_dir(),
            readonly: r.readonly,
            overridable: r.own,
            overridden: overridden && r.own,
            default_path: (overridden && r.own).then(|| default_global(h, spec).display().to_string()),
            parked_here: cfg
                .disabled
                .iter()
                .filter(|e| e.platform == platform && Path::new(&e.stored).join(SKILL_FILE).is_file())
                .filter(|e| Path::new(&e.original).parent().is_some_and(|p| same_path(&p.to_string_lossy(), &r.path)))
                .count(),
        })
        .collect();
    let codex = (spec.toggle == Toggle::CodexToml).then(|| codex_disabled(&codex_config_path(h, spec)));
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
                None if matches!(spec.toggle,Toggle::GeminiSettings|Toggle::OpenCodePermission) => match json_skill_off(h,spec,&raw.root,&name) { Ok(true)=>Some("name"),Ok(false)=>None,Err(e)=>{errors.push(ScanError{path:raw.dir.display().to_string(),reason:e});Some("tool")} },
                None => (spec.toggle == Toggle::Park && claude_tool_off(h, spec, &raw.root, &name)).then_some("tool"),
            }
        };
        let enabled = disabled_by.is_none() && spec.toggle != Toggle::None;
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
            (None, None) if spec.toggle == Toggle::None => ("unknown", Some("启停由原工具管理，Jolu 仅确认文件已安装".into())),
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
    if platform != "all" && spec(platform).is_none() {
        return Err(format!("{platform} 暂未接入 Skills 管理"));
    }
    let mut errors = Vec::new();
    let mut platforms = Vec::new();
    let mut items = Vec::new();
    let mut roots_out = Vec::new();
    for s in SPECS {
        let (list, infos) = collect(h, cfg, s, &mut errors);
        let global_dir = write_root(h, cfg, s.id, Scope::Global, None).ok().map(|r| r.path.display().to_string());
        platforms.push(PlatformSummary { id: s.id.into(), detected: infos.iter().any(|r| r.exists), count: list.len(), verified: s.verified, global_dir });
        if platform == "all" || platform == s.id {
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
            platform: spec(&entry.platform).ok_or("路径无效")?.id,
            scope: entry.scope,
            project: entry.project.as_ref().map(PathBuf::from),
            path: parent.to_path_buf(),
            readonly: false,
            skip: Vec::new(),
            parked: true,
            own: false,
        };
        return Ok((root, dir));
    }
    let parent_c = fs::canonicalize(parent).map_err(|_| "Skill 所在目录不存在")?;
    let projects = cfg.project_paths(h);
    for s in SPECS {
        for root in roots(h, cfg, s.id, &projects) {
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
    for entry in fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| format!("读取 {} 失败：{e}", from.display()))?;
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

/// 先完整复制来源，再交换安装目录；复制或交换失败时保留原安装项。
fn overwrite_dir(source: &Path, dest: &Path, platform: &str, backup_root: &Path) -> Result<(), String> {
    let source_c = fs::canonicalize(source).map_err(|e| e.to_string())?;
    let dest_c = fs::canonicalize(dest).map_err(|e| e.to_string())?;
    if source_c.starts_with(&dest_c) || dest_c.starts_with(&source_c) {
        return Err("来源与目标目录相同或互相包含，不能覆盖导入".into());
    }
    // 放在目标同一目录，两个 rename 都不会跨磁盘；create_dir 避免复用已有暂存区。
    let staging = dest.parent().ok_or("目标目录无效")?.join(format!(".jolu-import-{}", stamp()));
    fs::create_dir(&staging).map_err(|e| format!("无法创建导入暂存目录：{e}"))?;
    let incoming = staging.join("incoming");
    let previous = staging.join("previous");
    let prepared = copy_dir(source, &incoming)
        .and_then(|_| backup_dir_tree(dest, platform, backup_root));
    let backup = match prepared {
        Ok(backup) => backup,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("导入准备失败，原目录保持不变：{error}"));
        }
    };
    if let Err(error) = fs::rename(dest, &previous) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("无法替换目标，原目录保持不变：{error}"));
    }
    if let Err(error) = fs::rename(&incoming, dest) {
        if let Err(rollback) = fs::rename(&previous, dest) {
            return Err(format!("覆盖失败：{error}；恢复失败：{rollback}；原目录保留在 {}，备份在 {}", previous.display(), backup.display()));
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("覆盖失败，已恢复原目录：{error}"));
    }
    fs::remove_dir_all(&staging).map_err(|e| format!("已覆盖导入，但清理暂存目录 {} 失败：{e}；原内容备份在 {}", staging.display(), backup.display()))
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
    let (items, _) = collect(h, cfg, spec(root.platform).ok_or("路径无效")?, &mut errors);
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
    if !spec(&input.platform).is_some_and(|s| s.writable) {
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
    let root = write_root(h, cfg, &input.platform, input.scope, project.as_deref())?;
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
    let root = write_root(h, cfg, platform, scope, project.as_deref())?;
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
    let root = write_root(h, cfg, platform, scope, project.as_deref())?;
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
                None if parked_copy(cfg, platform, &root.path.join(&name)).is_some() => ("conflict", Some("目标已有同名 Skill（已停用）".to_string())),
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

/// 停用区里原位置是 `target` 的那一份（有多份时取最近停用的）
fn parked_copy(cfg: &Config, platform: &str, target: &Path) -> Option<PathBuf> {
    cfg.disabled
        .iter()
        .rev()
        .filter(|e| e.platform == platform && same_path(&e.original, target))
        .map(|e| PathBuf::from(&e.stored))
        .find(|stored| stored.join(SKILL_FILE).is_file())
}

#[allow(clippy::too_many_arguments)]
pub fn import_with(h: &Homes, cfg: &mut Config, source: &str, platform: &str, scope: Scope, project: Option<&str>, choices: Vec<ImportChoice>, enable: bool, data: &Path) -> Result<ImportResult, String> {
    let scan = import_scan_with(h, cfg, source, platform, scope, project)?;
    let project_path = check_project(h, cfg, project)?;
    let root = write_root(h, cfg, platform, scope, project_path.as_deref())?;
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
            ("import", "ok") => fs::create_dir_all(&root.path)
                .map_err(|e| e.to_string())
                .and_then(|_| copy_dir(Path::new(&item.path), &target))
                .map(|_| (target.clone(), true)),
            // 覆盖写到同名 Skill 当前所在的位置（原目录，或 Claude 停用区里的那一份），启停状态保持不变
            ("overwrite", "conflict") => (|| {
                let dest = if target.exists() { target.clone() } else { parked_copy(cfg, platform, &target).ok_or("目标已不存在，请重新扫描")? };
                ensure_writable(&root, &dest)?;
                overwrite_dir(Path::new(&item.path), &dest, platform, &data.join("skill-backups").join("overwritten"))?;
                Ok((dest, false))
            })(),
            ("import", "conflict") => Err("目标已有同名 Skill，请选择跳过或覆盖".into()),
            _ => Err("不支持的导入方式".into()),
        };
        match outcome {
            Ok((dest, is_new)) => {
                // 来源按原位置登记：停用区里的副本也按原位置判断来源
                if !cfg.imported.iter().any(|p| same_path(p, &target)) {
                    cfg.imported.push(target.display().to_string());
                }
                let mut final_path = dest.display().to_string();
                // 「导入后启用」只作用于新导入的 Skill
                if is_new && !enable {
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

/* ───────────────────────── 从 Git 仓库导入（D-16） ───────────────────────── */
//
// 用本机 git 浅克隆到应用临时目录下的 `jolu-git-*`，再复用上面的导入扫描与冲突检查；
// 导入完成或取消后由前端调用清理命令删除临时目录。只接受不带用户名 / 密码的 `https://` 地址，
// 禁止终端与凭据管理器弹窗（`GIT_TERMINAL_PROMPT=0`、`GCM_INTERACTIVE=never`），需要登录的仓库直接报错。

pub const GIT_MISSING: &str = "未找到 git，请先安装 Git";
const GIT_PREFIX: &str = "jolu-git-";
const GIT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Serialize, Debug)]
pub struct GitImportScan {
    /// 临时克隆目录，导入完成或取消后交给 `skills_import_git_cleanup`
    pub checkout_dir: String,
    /// `scan.source` 是实际导入来源：`<checkout_dir>/<仓库名>`，或仓库不是单个 Skill 时其中的 `skills/` 子目录
    pub scan: ImportScan,
}

/// 只接受 `https://`，不能带用户名或密码，不能有空白或控制字符
pub fn validate_git_url(url: &str) -> Result<(), String> {
    let rest = url.strip_prefix("https://").ok_or("仓库地址只支持 https:// 开头")?;
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("仓库地址不能包含空格".into());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err("仓库地址不能包含用户名或密码".into());
    }
    if authority.is_empty() || authority.starts_with(':') {
        return Err("仓库地址缺少主机名".into());
    }
    Ok(())
}

/// 分支名：不能以 `-` 开头（防止被当成 git 参数），不能有空白、控制字符或 `..`
pub fn validate_git_branch(branch: &str) -> Result<(), String> {
    let bad = branch.is_empty()
        || branch.len() > 200
        || branch.starts_with('-')
        || branch.contains("..")
        || branch.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\'));
    if bad { Err("分支名无效".into()) } else { Ok(()) }
}

/// 应用临时目录 `<系统临时目录>/jolu`，不存在时创建
fn git_temp_root() -> PathBuf {
    let dir = std::env::temp_dir().join("jolu");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// 克隆目录里存放仓库的子目录名：URL 最后一段去掉 `.git`；只保留 `[A-Za-z0-9._-]`，其他字符换成 `_`。
/// 仓库本身就是一个 Skill 时它就是 Skill 的目录名，不符合命名规则的由导入扫描标为「目录名不符合 Skill 名称规则」
fn repo_dir_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default().trim_end_matches('/');
    let last = path.rsplit('/').next().unwrap_or_default();
    let last = last.strip_suffix(".git").unwrap_or(last);
    let name: String = last.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
    if name.is_empty() || name.chars().all(|c| c == '.') { "_".into() } else { name }
}

/// 删除目录树；git 会把对象文件设为只读，Windows 上要先去掉只读属性再删
fn remove_tree(dir: &Path) -> Result<(), String> {
    fn clear_readonly(dir: &Path) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                clear_readonly(&entry.path());
            } else if kind.is_file() {
                if let Ok(meta) = entry.metadata() {
                    let mut perms = meta.permissions();
                    if perms.readonly() {
                        #[allow(clippy::permissions_set_readonly_false)]
                        perms.set_readonly(false);
                        let _ = fs::set_permissions(entry.path(), perms);
                    }
                }
            }
        }
    }
    if fs::remove_dir_all(dir).is_ok() || !dir.exists() {
        return Ok(());
    }
    clear_readonly(dir);
    fs::remove_dir_all(dir).map_err(|e| format!("无法删除临时目录 {}：{e}", dir.display()))
}

/// 只删除 `temp_root` 下直接以 `jolu-git-` 开头的目录；已经不存在时视为成功
fn cleanup_checkout(temp_root: &Path, checkout_dir: &str) -> Result<(), String> {
    const REFUSE: &str = "只能清理 Jolu 拉取 Git 仓库时创建的临时目录";
    let dir = PathBuf::from(checkout_dir);
    if !dir.is_absolute() || has_parent_component(&dir) {
        return Err(REFUSE.into());
    }
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !name.starts_with(GIT_PREFIX) {
        return Err(REFUSE.into());
    }
    let parent = dir.parent().ok_or(REFUSE)?;
    let same_root = match (fs::canonicalize(parent), fs::canonicalize(temp_root)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same_root {
        return Err(REFUSE.into());
    }
    match fs::symlink_metadata(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
        // 符号链接 / junction 指向别处，不跟随删除
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => Err(REFUSE.into()),
        Ok(_) => remove_tree(&dir),
    }
}

/// 结束 git 及其子进程（git-remote-https 等）
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// 把 git 的报错翻成用户能处理的说明，附上原文最后一行
fn git_error(stderr: &str) -> String {
    let last = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).next_back().unwrap_or("git 没有给出原因");
    let lower = stderr.to_lowercase();
    let hint = if lower.contains("remote branch") && lower.contains("not found") {
        "仓库里没有这个分支"
    } else if lower.contains("repository not found") || lower.contains("authentication") || lower.contains("could not read username") || lower.contains("terminal prompts disabled") {
        "无法访问仓库：地址不存在，或仓库需要登录"
    } else if lower.contains("could not resolve host") || lower.contains("failed to connect") || lower.contains("timed out") {
        "无法连接到仓库地址，请检查网络或 Git 的代理设置"
    } else {
        "拉取失败"
    };
    format!("{hint}（{last}）")
}

/// `git clone --depth 1 [--branch B] -- <url> <dest>`，隐藏窗口、禁止凭据弹窗，超时后结束进程树
fn clone_repo(program: &str, url: &str, branch: Option<&str>, dest: &Path, timeout: Duration) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(program);
    // 只允许 https 传输；符号链接按普通文件检出（导入复制时本来也不跟随符号链接）
    command.args(["-c", "protocol.allow=never", "-c", "protocol.https.allow=always", "-c", "core.symlinks=false", "clone", "--depth", "1", "--quiet"]);
    if let Some(branch) = branch {
        command.args(["--branch", branch]);
    }
    command
        .arg("--")
        .arg(url)
        .arg(dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(GIT_MISSING.into()),
        Err(e) => return Err(format!("无法启动 git：{e}")),
    };
    let stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = stderr {
            let _ = pipe.read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                kill_tree(&mut child);
                return Err(format!("拉取超时（{} 秒），已取消", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                kill_tree(&mut child);
                return Err(format!("等待 git 结束失败：{e}"));
            }
        }
    };
    let stderr = reader.join().unwrap_or_default();
    if status.success() { Ok(()) } else { Err(git_error(&stderr)) }
}

fn has_import_candidates(dir: &Path) -> bool {
    dir.join(SKILL_FILE).is_file()
        || fs::read_dir(dir).map(|entries| entries.flatten().any(|e| e.path().join(SKILL_FILE).is_file())).unwrap_or(false)
}

/// 导入来源：仓库本身就是一个 Skill 时用根目录；否则仓库约定的合集目录 `skills/` 里有 Skill 就用它
/// （如 anthropics/skills 根目录还有一个 `template/` 模板，不应混进导入列表）；都没有时用根目录
fn git_import_source(checkout: &Path) -> PathBuf {
    let nested = checkout.join("skills");
    if !checkout.join(SKILL_FILE).is_file() && nested.is_dir() && has_import_candidates(&nested) {
        nested
    } else {
        checkout.to_path_buf()
    }
}

/// 克隆完成后：先删掉 `.git`（不随导入复制进 Skills 目录），再选来源并扫描
fn scan_clone(h: &Homes, cfg: &Config, repo: &Path, platform: &str, scope: Scope, project: Option<&str>) -> Result<ImportScan, String> {
    let git = repo.join(".git");
    match fs::symlink_metadata(&git) {
        Ok(meta) if meta.is_dir() => remove_tree(&git)?,
        Ok(_) => fs::remove_file(&git).map_err(|e| format!("无法删除 {}：{e}", git.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // 拒绝访问等情况无法确认 .git 已不在，中止导入（调用方会删除临时目录）
        Err(e) => return Err(format!("无法确认 .git 已删除：{e}")),
    }
    import_scan_with(h, cfg, &git_import_source(repo).display().to_string(), platform, scope, project)
}

#[allow(clippy::too_many_arguments)]
pub fn import_git_with(h: &Homes, cfg: &Config, temp_root: &Path, program: &str, url: &str, branch: Option<&str>, platform: &str, scope: Scope, project: Option<&str>) -> Result<GitImportScan, String> {
    let url = url.trim();
    validate_git_url(url)?;
    let branch = branch.map(str::trim).filter(|b| !b.is_empty());
    if let Some(b) = branch {
        validate_git_branch(b)?;
    }
    // 先确认目标范围可写，避免白拉一次
    let project_path = check_project(h, cfg, project)?;
    write_root(h, cfg, platform, scope, project_path.as_deref())?;
    fs::create_dir_all(temp_root).map_err(|e| format!("无法创建临时目录：{e}"))?;
    // 克隆到 `jolu-git-<时间戳>/<仓库名>`：仓库本身是 Skill 时目录名就是仓库名，重复导入能命中冲突检查
    let dest = temp_root.join(format!("{GIT_PREFIX}{}", stamp()));
    let repo = dest.join(repo_dir_name(url));
    let result = fs::create_dir_all(&dest)
        .map_err(|e| format!("无法创建临时目录：{e}"))
        .and_then(|_| clone_repo(program, url, branch, &repo, GIT_TIMEOUT))
        .and_then(|_| scan_clone(h, cfg, &repo, platform, scope, project));
    match result {
        Ok(scan) => Ok(GitImportScan { checkout_dir: dest.display().to_string(), scan }),
        Err(reason) => {
            let _ = remove_tree(&dest);
            Err(reason)
        }
    }
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

/// 「更换全局目录」（D-18）：替换工具主目录（`ToolSpec::global` 的最后一项），只改 Jolu 扫描和写入的位置，
/// 不改工具本身的环境变量；`path` 为 None 或与默认位置相同时恢复默认。目录必须已存在
pub fn set_global_dir_with(h: &Homes, cfg: &mut Config, platform: &str, path: Option<&str>) -> Result<(), String> {
    let spec = spec(platform).ok_or_else(|| format!("{platform} 暂未接入 Skills 管理"))?;
    let Some(raw) = path.map(str::trim).filter(|p| !p.is_empty()) else {
        cfg.global_override.remove(spec.id);
        return Ok(());
    };
    let dir = PathBuf::from(raw);
    if !dir.is_absolute() || has_parent_component(&dir) || !dir.is_dir() {
        return Err(format!("目录不存在：{raw}"));
    }
    if same_path(raw, &default_global(h, spec)) {
        cfg.global_override.remove(spec.id);
        return Ok(());
    }
    // 已经是任一工具（含自己）的其他扫描目录、只读子目录或项目 Skills 目录时拒绝：同一目录被扫描两次会标成同名冲突，
    // 被两个工具扫描时 `locate` 先命中排在前面的工具，启停就会走错方式（如在 Codex 列表里停用却按 Claude 移进停用区）
    let projects = cfg.project_paths(h);
    for other in SPECS {
        let taken = roots(h, cfg, other.id, &projects).into_iter().filter(|r| !(other.id == spec.id && r.own)).find(|r| same_path(raw, &r.path));
        if let Some(root) = taken {
            return Err(format!("这个目录已经是 {} 的扫描目录：{}", other.name, root.path.display()));
        }
    }
    cfg.global_override.insert(spec.id.into(), dir.display().to_string());
    Ok(())
}

#[derive(Deserialize, Debug)]
pub struct IgnoreChange {
    pub path: String,
    pub ignored: bool,
}

#[derive(Deserialize, Debug)]
pub struct GlobalChange {
    pub platform: String,
    /// None：恢复默认
    pub path: Option<String>,
}

/// 「扫描目录设置」弹窗的草稿，点「保存并扫描」时一次提交
#[derive(Deserialize, Debug, Default)]
pub struct DirSettings {
    #[serde(default)]
    pub custom_dirs: Option<Vec<String>>,
    /// 手动添加的项目目录（完整列表）
    pub projects: Vec<String>,
    /// 会话记录带出的项目里改了忽略状态的
    #[serde(default)]
    pub ignore: Vec<IgnoreChange>,
    /// 改了全局目录的工具
    #[serde(default)]
    pub globals: Vec<GlobalChange>,
}

/// 在副本上依次应用并校验，全部通过才替换 `cfg`；任何一项失败都不改动配置
pub fn save_dir_settings_with(h: &Homes, cfg: &mut Config, input: DirSettings) -> Result<(), String> {
    let mut next = cfg.clone();
    if let Some(dirs)=input.custom_dirs {
        if dirs.len()>50 {return Err("最多添加 50 个自定义目录".into())}
        let mut valid=Vec::new();for dir in dirs {let p=PathBuf::from(dir.trim());if !p.is_absolute()||has_parent_component(&p)||!p.is_dir(){return Err(format!("自定义目录不存在：{}",p.display()))}if !valid.iter().any(|v:&String|same_path(v,&p)){valid.push(p.display().to_string());}}
        next.custom_dirs=valid;
    }
    set_projects_with(&mut next, input.projects)?;
    for change in input.ignore {
        ignore_project_with(&mut next, &change.path, change.ignored);
    }
    for change in input.globals {
        set_global_dir_with(h, &mut next, &change.platform, change.path.as_deref())?;
    }
    // 再按最终草稿检查，覆盖“先更换全局目录、后加项目”和恢复默认的反向冲突。
    let projects=next.project_paths(h);
    for (platform,path) in &next.global_override {
        for other in SPECS {
            if roots(h,&next,other.id,&projects).iter().any(|r| !(r.platform==platform&&r.own) && same_path(path,&r.path)) {
                return Err(format!("更换的全局目录与 {} 扫描目录重复：{path}",other.name));
            }
        }
    }
    *cfg = next;
    Ok(())
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

/// 会改动文件或配置的命令：skills.json 损坏时在动手之前就拒绝，避免改了文件却记不下停用登记
async fn context_mut(app: &tauri::AppHandle, db: &State<'_, crate::DbRead>) -> Result<(PathBuf, Config), String> {
    let (dir, cfg) = context(app, db, false).await?;
    config_guard(&dir)?;
    Ok((dir, cfg))
}

/// 列出 Skills。`platform` 为 all / claude / codex；`force` 时重新读取会话记录里的项目目录。
/// 模块 E 的「Skills（可选）」也可用它列出某个工具的可用项
#[tauri::command]
pub async fn skills_overview(app: tauri::AppHandle, db: State<'_, crate::DbRead>, platform: String, force: bool) -> Result<Overview, String> {
    let (dir, cfg) = context(&app, &db, force).await?;
    blocking(move || {
        let mut overview = overview_with(&homes()?, &cfg, &platform)?;
        // 配置损坏时照常扫描，同时在扫描错误里说明为什么改动会被拒绝
        if let Err(reason) = config_guard(&dir) {
            overview.errors.insert(0, ScanError { path: dir.join("skills.json").display().to_string(), reason });
        }
        Ok(overview)
    })
    .await
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
    let (dir, mut cfg) = context_mut(&app, &db).await?;
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
    let (dir, mut cfg) = context_mut(&app, &db).await?;
    blocking(move || save_with(&homes()?, &mut cfg, input, &dir)).await
}

#[tauri::command]
pub async fn skill_copy(app: tauri::AppHandle, db: State<'_, crate::DbRead>, path: String, platform: String, scope: Scope, project: Option<String>, dry_run: bool) -> Result<CopyResult, String> {
    let (_, cfg) = if dry_run { context(&app, &db, false).await? } else { context_mut(&app, &db).await? };
    blocking(move || copy_with(&homes()?, &cfg, &path, &platform, scope, project.as_deref(), dry_run)).await
}

#[tauri::command]
pub async fn skill_remove(app: tauri::AppHandle, db: State<'_, crate::DbRead>, path: String) -> Result<String, String> {
    let (dir, mut cfg) = context_mut(&app, &db).await?;
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
    let (dir, mut cfg) = context_mut(&app, &db).await?;
    blocking(move || {
        let result = import_with(&homes()?, &mut cfg, &source, &platform, scope, project.as_deref(), choices, enable, &dir)?;
        save_config(&dir, &cfg)?;
        Ok(result)
    })
    .await
}

/// 从 Git 仓库导入：浅克隆到应用临时目录并扫描；之后用 `scan.source` 调 `skills_import`，结束后调 `skills_import_git_cleanup`
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn skills_import_git(app: tauri::AppHandle, db: State<'_, crate::DbRead>, url: String, branch: Option<String>, platform: String, scope: Scope, project: Option<String>) -> Result<GitImportScan, String> {
    let (_, cfg) = context(&app, &db, false).await?;
    blocking(move || import_git_with(&homes()?, &cfg, &git_temp_root(), "git", &url, branch.as_deref(), &platform, scope, project.as_deref())).await
}

/// 删除 `skills_import_git` 创建的临时克隆目录；只接受应用临时目录下 `jolu-git-` 开头的目录
#[tauri::command]
pub async fn skills_import_git_cleanup(checkout_dir: String) -> Result<(), String> {
    blocking(move || cleanup_checkout(&git_temp_root(), &checkout_dir)).await
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

/// 更换或恢复（`path` 为 None）某个工具的全局目录，返回这个工具更新后的概览。
/// 立即写入，仅供脚本 / 验收使用；界面走 `skills_save_dir_settings`（「保存并扫描」时一次提交）
#[tauri::command]
pub async fn skills_set_global_dir(app: tauri::AppHandle, db: State<'_, crate::DbRead>, platform: String, path: Option<String>) -> Result<Overview, String> {
    let (dir, mut cfg) = context_mut(&app, &db).await?;
    blocking(move || {
        let h = homes()?;
        set_global_dir_with(&h, &mut cfg, &platform, path.as_deref())?;
        save_config(&dir, &cfg)?;
        overview_with(&h, &cfg, &platform)
    })
    .await
}

/// 「扫描目录设置」的「保存并扫描」：项目目录、忽略状态、全局目录一次校验，skills.json 只原子写入一次
#[tauri::command]
pub async fn skills_save_dir_settings(app: tauri::AppHandle, db: State<'_, crate::DbRead>, input: DirSettings) -> Result<(), String> {
    let (dir, mut cfg) = context_mut(&app, &db).await?;
    blocking(move || {
        save_dir_settings_with(&homes()?, &mut cfg, input)?;
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

    #[test]
    fn additional_tools_and_name_based_settings() {
        let base = temp("additional"); let h = homes_at(&base); let mut cfg=Config::default();
        for id in ["gemini","opencode","qoder","trae","zcode","workbuddy"] { assert!(spec(id).is_some(), "{id}"); }
        for id in ["gemini", "opencode"] {
            let r=write_root(&h,&cfg,id,Scope::Global,None).unwrap();
            let dir=skill(&r.path,"demo",&body("demo"));
            set_enabled_impl(&h,&mut cfg,&r,&dir,"demo",false,&base.join("data")).unwrap();
            let (items,_)=collect(&h,&cfg,spec(id).unwrap(),&mut vec![]); assert!(!items[0].enabled);
            set_enabled_impl(&h,&mut cfg,&r,&dir,"demo",true,&base.join("data")).unwrap();
            let (items,_)=collect(&h,&cfg,spec(id).unwrap(),&mut vec![]); assert!(items[0].enabled);
        }
        assert!(write_root(&h,&cfg,"workbuddy",Scope::Global,None).is_err());
    }

    #[test]
    fn opencode_project_inherits_global_permission() {
        let base=temp("opencode-inherit");let h=homes_at(&base);let cfg=Config::default();let project=base.join("project");fs::create_dir_all(&project).unwrap();
        let spec=spec("opencode").unwrap();let global=write_root(&h,&cfg,"opencode",Scope::Global,None).unwrap();
        set_json_skill(&h,spec,&global,"demo",false,&base.join("data")).unwrap();
        let local=write_root(&h,&cfg,"opencode",Scope::Project,Some(&project)).unwrap();
        assert!(json_skill_off(&h,spec,&local,"demo").unwrap());
        assert!(set_json_skill(&h,spec,&local,"demo",true,&base.join("data")).is_err());
    }

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jolu-skills-{tag}-{}-{}", std::process::id(), stamp()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn homes_at(base: &Path) -> Homes {
        homes_from(base.to_path_buf(), |_| None)
    }

    fn claude_home(h: &Homes) -> PathBuf {
        h.env["CLAUDE_CONFIG_DIR"].clone()
    }

    fn codex_home(h: &Homes) -> PathBuf {
        h.env["CODEX_HOME"].clone()
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

    fn fmt_roots(list: &[Root]) -> Vec<String> {
        list.iter()
            .map(|r| format!("{}|{:?}|{}|{}|{}|{:?}|{}", r.platform, r.scope, r.project.as_ref().map(|p| p.display().to_string()).unwrap_or_default(), r.path.display(), r.readonly, r.skip, r.parked))
            .collect()
    }

    /// 重构前 `roots()` 的输出（2026-10-23 用旧实现跑出的固定值）
    fn roots_before() -> Vec<String> {
        [
            r#"claude|Global||C:\h\cc\skills|false|["synced"]|false"#,
            r"claude|Synced||C:\h\cc\skills\synced|true|[]|false",
            r"claude|Project|D:\p1|D:\p1\.claude\skills|false|[]|false",
            r"claude|Project|D:\p2|D:\p2\.claude\skills|false|[]|false",
            r"codex|Global||C:\h\.agents\skills|false|[]|false",
            r#"codex|Global||C:\h\cx\skills|false|[".system"]|false"#,
            r"codex|System||C:\h\cx\skills\.system|true|[]|false",
            r"codex|Project|D:\p1|D:\p1\.agents\skills|false|[]|false",
            r"codex|Project|D:\p1|D:\p1\.codex\skills|false|[]|false",
            r"codex|Project|D:\p2|D:\p2\.agents\skills|false|[]|false",
            r"codex|Project|D:\p2|D:\p2\.codex\skills|false|[]|false",
        ]
        .map(String::from)
        .to_vec()
    }

    #[cfg(windows)]
    #[test]
    fn specs_preserve_claude_codex_roots() {
        let h = homes_from(PathBuf::from(r"C:\h"), |name| match name {
            "CLAUDE_CONFIG_DIR" => Some(r"C:\h\cc".into()),
            "CODEX_HOME" => Some(r"C:\h\cx".into()),
            _ => None,
        });
        let cfg = Config::default();
        let projects = [PathBuf::from(r"D:\p1"), PathBuf::from(r"D:\p2")];
        let mut got = fmt_roots(&roots(&h, &cfg, "claude", &projects));
        got.extend(fmt_roots(&roots(&h, &cfg, "codex", &projects)));
        assert_eq!(got, roots_before());
        // 没设环境变量时用默认位置
        let d = homes_from(PathBuf::from(r"C:\h"), |_| None);
        assert_eq!((claude_home(&d), codex_home(&d)), (PathBuf::from(r"C:\h\.claude"), PathBuf::from(r"C:\h\.codex")));
        let mut got = fmt_roots(&roots(&d, &cfg, "claude", &projects));
        got.extend(fmt_roots(&roots(&d, &cfg, "codex", &projects)));
        // 按旧实现推导：Homes { claude: user/.claude, codex: user/.codex, user }
        let expected_default = [
            r#"claude|Global||C:\h\.claude\skills|false|["synced"]|false"#,
            r"claude|Synced||C:\h\.claude\skills\synced|true|[]|false",
            r"claude|Project|D:\p1|D:\p1\.claude\skills|false|[]|false",
            r"claude|Project|D:\p2|D:\p2\.claude\skills|false|[]|false",
            r"codex|Global||C:\h\.agents\skills|false|[]|false",
            r#"codex|Global||C:\h\.codex\skills|false|[".system"]|false"#,
            r"codex|System||C:\h\.codex\skills\.system|true|[]|false",
            r"codex|Project|D:\p1|D:\p1\.agents\skills|false|[]|false",
            r"codex|Project|D:\p1|D:\p1\.codex\skills|false|[]|false",
            r"codex|Project|D:\p2|D:\p2\.agents\skills|false|[]|false",
            r"codex|Project|D:\p2|D:\p2\.codex\skills|false|[]|false",
        ];
        assert_eq!(got, expected_default);
        // 隔离环境：按旧实现推导 base = <JOLU_TEST_DIR>/skills-home，claude = base/.claude，codex = base/.codex，不读环境变量
        let iso = isolated_homes(Path::new(r"D:\t"));
        assert_eq!(iso.user, PathBuf::from(r"D:\t\skills-home"));
        assert_eq!((claude_home(&iso), codex_home(&iso)), (PathBuf::from(r"D:\t\skills-home\.claude"), PathBuf::from(r"D:\t\skills-home\.codex")));
        let mut got = fmt_roots(&roots(&iso, &cfg, "claude", &[]));
        got.extend(fmt_roots(&roots(&iso, &cfg, "codex", &[])));
        let expected_iso = [
            r#"claude|Global||D:\t\skills-home\.claude\skills|false|["synced"]|false"#,
            r"claude|Synced||D:\t\skills-home\.claude\skills\synced|true|[]|false",
            r"codex|Global||D:\t\skills-home\.agents\skills|false|[]|false",
            r#"codex|Global||D:\t\skills-home\.codex\skills|false|[".system"]|false"#,
            r"codex|System||D:\t\skills-home\.codex\skills\.system|true|[]|false",
        ];
        assert_eq!(got, expected_iso);
        // 启停方式与校验规则
        let (claude, codex) = (spec("claude").unwrap(), spec("codex").unwrap());
        assert_eq!((claude.toggle, claude.require_name, claude.writable), (Toggle::Park, false, true));
        assert_eq!((codex.toggle, codex.require_name, codex.writable), (Toggle::CodexToml, true, true));
        assert!(spec("grok").is_none() && roots(&h, &cfg, "grok", &projects).is_empty());
        // 约定：home_env 控制 global 的最后一项，且这一项以 `/<子目录>` 结尾
        for s in SPECS {
            if s.home_env.is_some() {
                assert!(env_default(s).is_some(), "{} 的 global 最后一项应以 home_env 的子目录结尾", s.id);
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn codex_agents_dir_ignores_codex_home() {
        let h = homes_from(PathBuf::from(r"C:\u"), |name| (name == "CODEX_HOME").then(|| r"E:\codex-home".into()));
        let cfg = Config::default();
        let globals: Vec<_> = roots(&h, &cfg, "codex", &[]).into_iter().filter(|r| r.scope == Scope::Global).map(|r| r.path).collect();
        assert_eq!(globals, vec![PathBuf::from(r"C:\u\.agents\skills"), PathBuf::from(r"E:\codex-home\skills")]);
        assert_eq!(codex_config_path(&h, spec("codex").unwrap()), PathBuf::from(r"E:\codex-home\config.toml"));
        // 新建 / 导入到全局仍写 ~/.agents/skills
        assert_eq!(write_root(&h, &cfg, "codex", Scope::Global, None).unwrap().path, PathBuf::from(r"C:\u\.agents\skills"));
        // Claude 不受 CODEX_HOME 影响
        assert_eq!(claude_home(&h), PathBuf::from(r"C:\u\.claude"));
    }

    #[test]
    fn global_override_replaces_default() {
        let base = temp("override");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let custom = base.join("my-skills");
        skill(&custom, "custom-one", &body("custom-one"));
        skill(&claude_home(&h).join("skills"), "default-one", &body("default-one"));

        let missing = base.join("missing").display().to_string();
        assert!(set_global_dir_with(&h, &mut cfg, "claude", Some(&missing)).unwrap_err().contains("目录不存在"));
        assert!(set_global_dir_with(&h, &mut cfg, "claude", Some("relative/dir")).unwrap_err().contains("目录不存在"));
        assert!(set_global_dir_with(&h, &mut cfg, "grok", Some(&custom.display().to_string())).is_err());
        assert!(cfg.global_override.is_empty());

        set_global_dir_with(&h, &mut cfg, "claude", Some(&custom.display().to_string())).unwrap();
        let list = roots(&h, &cfg, "claude", &[]);
        assert_eq!((list[0].scope, &list[0].path), (Scope::Global, &custom));
        assert_eq!((list[1].scope, &list[1].path), (Scope::Synced, &custom.join("synced")));
        let o = overview_with(&h, &cfg, "claude").unwrap();
        assert_eq!(o.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["custom-one"], "只扫描新目录");
        let info = o.roots.iter().find(|r| r.scope == Scope::Global).unwrap();
        assert!(info.overridable && info.overridden);
        assert_eq!(info.default_path.as_deref(), Some(claude_home(&h).join("skills").display().to_string().as_str()));
        assert_eq!(o.platforms.iter().find(|p| p.id == "claude").unwrap().global_dir.as_deref(), Some(custom.display().to_string().as_str()));
        // 新建写到新目录；工具配置（settings.json）位置不变
        let input = SaveInput { platform: "claude".into(), scope: Scope::Global, project: None, name: "new-one".into(), content: body("new-one"), original_path: None, expected_modified_ms: None, enable: true };
        assert!(Path::new(&save_with(&h, &mut cfg, input, &data).unwrap().path).starts_with(&custom));
        assert!(locate(&h, &cfg, &custom.join("new-one").display().to_string()).is_ok());
        // 存进 skills.json
        save_config(&data, &cfg).unwrap();
        assert_eq!(load_config(&data).global_override.get("claude"), Some(&custom.display().to_string()));

        // 恢复默认；选回默认目录也等同恢复默认
        set_global_dir_with(&h, &mut cfg, "claude", None).unwrap();
        assert_eq!(roots(&h, &cfg, "claude", &[])[0].path, claude_home(&h).join("skills"));
        set_global_dir_with(&h, &mut cfg, "claude", Some(&claude_home(&h).join("skills").display().to_string())).unwrap();
        assert!(cfg.global_override.is_empty());
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let info = o.roots.iter().find(|r| r.scope == Scope::Global).unwrap();
        assert!(info.overridable && !info.overridden && info.default_path.is_none());

        // Codex：更换的是 $CODEX_HOME/skills，~/.agents/skills 照常扫描、仍是写入目标，且不能被选作替换目录
        let agents = base.join(".agents").join("skills");
        fs::create_dir_all(&agents).unwrap();
        assert!(set_global_dir_with(&h, &mut cfg, "codex", Some(&agents.display().to_string())).unwrap_err().contains("已经是"));
        set_global_dir_with(&h, &mut cfg, "codex", Some(&custom.display().to_string())).unwrap();
        let paths: Vec<_> = roots(&h, &cfg, "codex", &[]).into_iter().map(|r| (r.scope, r.path)).collect();
        assert_eq!(paths, vec![(Scope::Global, agents.clone()), (Scope::Global, custom.clone()), (Scope::System, custom.join(".system"))]);
        assert_eq!(write_root(&h, &cfg, "codex", Scope::Global, None).unwrap().path, agents);
        let o = overview_with(&h, &cfg, "codex").unwrap();
        let flags: Vec<_> = o.roots.iter().filter(|r| r.scope == Scope::Global).map(|r| (r.overridable, r.overridden)).collect();
        assert_eq!(flags, vec![(false, false), (true, true)]);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn global_override_rejects_other_tools_and_project_dirs() {
        let base = temp("override-others");
        let h = homes_at(&base);
        let s = |p: &Path| p.display().to_string();
        let project = base.join("proj");
        let project_claude = project.join(".claude").join("skills");
        let agents = base.join(".agents").join("skills");
        let codex_skills = codex_home(&h).join("skills");
        let claude_skills = claude_home(&h).join("skills");
        let custom = base.join("my-skills");
        for d in [&project_claude, &agents, &codex_skills, &claude_skills.join("synced"), &custom] {
            fs::create_dir_all(d).unwrap();
        }
        let mut cfg = Config { projects: vec![s(&project)], ..Default::default() };
        let err = |cfg: &mut Config, platform: &str, path: String| set_global_dir_with(&h, cfg, platform, Some(&path)).unwrap_err();

        // 其他工具的全局目录（两个方向）
        let e = err(&mut cfg, "claude", s(&agents));
        assert!(e.contains("已经是 Codex 的扫描目录"), "{e}");
        assert!(err(&mut cfg, "claude", s(&codex_skills)).contains("已经是 Codex 的扫描目录"));
        assert!(err(&mut cfg, "codex", s(&claude_skills)).contains("已经是 Claude Code 的扫描目录"));
        // 只读子目录
        assert!(err(&mut cfg, "codex", s(&claude_skills.join("synced"))).contains("已经是 Claude Code 的扫描目录"));
        // 项目的 Skills 目录（同一工具与其他工具）
        assert!(err(&mut cfg, "claude", s(&project_claude)).contains("已经是 Claude Code 的扫描目录"));
        assert!(err(&mut cfg, "codex", s(&project_claude)).contains("已经是 Claude Code 的扫描目录"));
        // 大小写与分隔符不同也拦住
        if cfg!(windows) {
            assert!(err(&mut cfg, "claude", s(&agents).replace('\\', "/").to_uppercase()).contains("已经是 Codex 的扫描目录"));
        }
        // 另一个工具已经更换到的目录
        set_global_dir_with(&h, &mut cfg, "claude", Some(&s(&custom))).unwrap();
        assert!(err(&mut cfg, "codex", s(&custom)).contains("已经是 Claude Code 的扫描目录"));
        assert_eq!(cfg.global_override.keys().collect::<Vec<_>>(), vec!["claude"]);
        // 一次保存里新添加的项目也算：同时添加项目并把它的 Skills 目录选作全局目录，整体拒绝
        let other = base.join("other");
        fs::create_dir_all(other.join(".agents").join("skills")).unwrap();
        let input = DirSettings { custom_dirs: None, projects: vec![s(&project), s(&other)], ignore: vec![], globals: vec![GlobalChange { platform: "claude".into(), path: Some(s(&other.join(".agents").join("skills"))) }] };
        assert!(save_dir_settings_with(&h, &mut cfg, input).unwrap_err().contains("已经是 Codex 的扫描目录"));
        assert_eq!(cfg.projects, vec![s(&project)]);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn opencode_shared_skill_roots_are_readonly() {
        let base=temp("opencode-shared");let h=homes_at(&base);let cfg=Config::default();let project=base.join("project");
        let list=roots(&h,&cfg,"opencode",&[project.clone()]);
        for suffix in [".agents/skills",".claude/skills"] {
            assert!(list.iter().any(|r|r.path==base.join(suffix)&&r.readonly));
            assert!(list.iter().any(|r|r.path==project.join(suffix)&&r.readonly));
        }
        assert_eq!(write_root(&h,&cfg,"opencode",Scope::Global,None).unwrap().path,base.join(".config/opencode/skills"));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn broken_saved_override_is_readonly_and_reported() {
        let base = temp("override-broken");
        let h = homes_at(&base);
        let data = base.join("data");
        let gone = base.join("gone");
        let mut cfg = Config::default();
        cfg.global_override.insert("claude".into(), gone.display().to_string());
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let reason = format!("已更换的目录不存在：{}", gone.display());
        assert!(o.errors.iter().any(|e| e.reason == reason), "{:?}", o.errors);
        let g = o.roots.iter().find(|r| r.scope == Scope::Global).unwrap();
        assert!(g.readonly && g.overridden && !g.exists);
        assert_eq!(o.platforms.iter().find(|p| p.id == "claude").unwrap().global_dir, None);
        // 写入报错，不静默重建
        let input = SaveInput { platform: "claude".into(), scope: Scope::Global, project: None, name: "new-one".into(), content: body("new-one"), original_path: None, expected_modified_ms: None, enable: true };
        assert_eq!(save_with(&h, &mut cfg, input, &data).unwrap_err(), reason);
        let source = base.join("downloads");
        skill(&source, "api-review", &body("api-review"));
        assert_eq!(import_scan_with(&h, &cfg, &source.display().to_string(), "claude", Scope::Global, None).unwrap_err(), reason);
        assert!(!gone.exists(), "不重建已更换的目录");
        // 相对路径同样视为失效
        cfg.global_override.insert("claude".into(), "relative/dir".into());
        assert!(overview_with(&h, &cfg, "claude").unwrap().errors.iter().any(|e| e.reason.contains("已更换的目录不存在：relative/dir")));
        // 恢复默认后正常
        set_global_dir_with(&h, &mut cfg, "claude", None).unwrap();
        assert!(overview_with(&h, &cfg, "claude").unwrap().errors.is_empty());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn broken_config_is_backed_up_and_never_overwritten() {
        let data = temp("broken-config");
        let file = data.join("skills.json");
        let broken = "{ \"disabled\": [ 损坏";
        fs::write(&file, broken).unwrap();
        let backups = || {
            let mut list: Vec<PathBuf> = fs::read_dir(&data).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("skills.json.broken-")).map(|e| e.path()).collect();
            list.sort();
            list
        };
        let cfg = load_config(&data);
        assert!(cfg.disabled.is_empty(), "读取扫描照常可用（按空配置）");
        assert_eq!(backups().len(), 1);
        assert_eq!(fs::read_to_string(&backups()[0]).unwrap(), broken, "备份内容与原文件一致");
        let err = save_config(&data, &cfg).unwrap_err();
        assert_eq!(err, format!("Skills 配置文件损坏，已备份为 {}，请检查后重试", backups()[0].display()));
        assert_eq!(fs::read_to_string(&file).unwrap(), broken, "不覆盖损坏的文件");
        // 再读一次不重复备份
        let mut cfg = load_config(&data);
        assert_eq!(backups().len(), 1);
        // 停用（要写停用登记）被拒绝并回滚，目录留在原位
        let h = homes_at(&data);
        let dir = skill(&claude_home(&h).join("skills"), "keep-me", &body("keep-me"));
        let (root, located) = locate(&h, &cfg, &dir.display().to_string()).unwrap();
        assert!(set_enabled_impl(&h, &mut cfg, &root, &located, "keep-me", false, &data).unwrap_err().contains("Skills 配置文件损坏"));
        assert!(dir.join(SKILL_FILE).is_file() && cfg.disabled.is_empty());
        assert!(config_guard(&data).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), broken);
        // 修好之后恢复正常
        fs::write(&file, "{}").unwrap();
        let cfg = load_config(&data);
        assert!(config_guard(&data).is_ok());
        save_config(&data, &cfg).unwrap();
        // 空文件按空配置处理，不算损坏
        fs::write(&file, "  ").unwrap();
        load_config(&data);
        assert!(config_guard(&data).is_ok());
        assert_eq!(backups().len(), 1);
        fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn save_dir_settings_is_all_or_nothing() {
        let base = temp("dir-settings");
        let h = homes_at(&base);
        let project = base.join("proj");
        let session = base.join("from-session");
        let custom = base.join("my-skills");
        for d in [&project, &session, &custom] {
            fs::create_dir_all(d).unwrap();
        }
        let s = |p: &Path| p.display().to_string();
        let mut cfg = Config { session_projects: vec![s(&session)], ..Default::default() };
        let snapshot = |c: &Config| serde_json::to_string(c).unwrap();
        let before = snapshot(&cfg);
        let ignore = || vec![IgnoreChange { path: s(&session), ignored: true }];

        // 全局目录不存在：项目与忽略状态也不写入
        let bad = DirSettings { custom_dirs: None, projects: vec![s(&project)], ignore: ignore(), globals: vec![GlobalChange { platform: "claude".into(), path: Some(s(&base.join("missing"))) }] };
        assert!(save_dir_settings_with(&h, &mut cfg, bad).unwrap_err().contains("目录不存在"));
        assert_eq!(snapshot(&cfg), before, "失败时配置保持原样");
        // 项目目录无效：全局目录也不写入
        let bad = DirSettings { custom_dirs: None, projects: vec![s(&base.join("gone"))], ignore: ignore(), globals: vec![GlobalChange { platform: "claude".into(), path: Some(s(&custom)) }] };
        assert!(save_dir_settings_with(&h, &mut cfg, bad).unwrap_err().contains("项目目录不存在"));
        assert_eq!(snapshot(&cfg), before);

        let good = DirSettings { custom_dirs: None, projects: vec![s(&project)], ignore: ignore(), globals: vec![GlobalChange { platform: "claude".into(), path: Some(s(&custom)) }] };
        save_dir_settings_with(&h, &mut cfg, good).unwrap();
        assert_eq!(cfg.projects, vec![s(&project)]);
        assert_eq!(cfg.ignored_projects, vec![s(&session)]);
        assert_eq!(cfg.global_override.get("claude"), Some(&s(&custom)));
        // 同一次保存里恢复默认，其余设置不变
        let reset = DirSettings { custom_dirs: None, projects: vec![s(&project)], ignore: vec![], globals: vec![GlobalChange { platform: "claude".into(), path: None }] };
        save_dir_settings_with(&h, &mut cfg, reset).unwrap();
        assert!(cfg.global_override.is_empty());
        assert_eq!((cfg.projects.len(), cfg.ignored_projects.len()), (1, 1));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn roots_count_parked_skills_left_in_old_dir() {
        let base = temp("parked-here");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let dir = skill(&claude_home(&h).join("skills"), "old-one", &body("old-one"));
        let (root, located) = locate(&h, &cfg, &dir.display().to_string()).unwrap();
        set_enabled_impl(&h, &mut cfg, &root, &located, "old-one", false, &data).unwrap();
        let global = |cfg: &Config| overview_with(&h, cfg, "claude").unwrap().roots.into_iter().find(|r| r.scope == Scope::Global).unwrap();
        assert_eq!(global(&cfg).parked_here, 1, "停用区里原位置在默认目录的 1 个");
        let custom = base.join("my-skills");
        fs::create_dir_all(&custom).unwrap();
        set_global_dir_with(&h, &mut cfg, "claude", Some(&custom.display().to_string())).unwrap();
        assert_eq!(global(&cfg).parked_here, 0, "新目录下没有停用的");
        // 启用逻辑不变：已停用的仍显示，原位置还是旧目录
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let parked = o.items.iter().find(|i| i.name == "old-one").unwrap();
        assert_eq!((parked.disabled_by, parked.original_path.as_deref()), (Some("parked"), Some(dir.display().to_string().as_str())));
        fs::remove_dir_all(&base).ok();
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
        let global = claude_home(&h).join("skills");
        skill(&global, "frontend-design", &body("frontend-design"));
        skill(&global, "api-contract", "---\nname: api-contract\n---\n");
        skill(&global, "legacy", &body("legacy"));
        skill(&global.join("synced"), "synced-one", &body("synced-one"));
        skill(&project.join(".claude").join("skills"), "frontend-design", &body("frontend-design"));
        let settings = r#"{"skillOverrides":{"legacy":"off"}}"#;
        fs::write(claude_home(&h).join("settings.json"), settings).unwrap();
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
        assert_eq!(fs::read_to_string(claude_home(&h).join("settings.json")).unwrap(), settings);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn claude_disable_parks_only_this_copy() {
        let base = temp("park");
        let h = homes_at(&base);
        let data = base.join("data");
        let project = base.join("proj");
        skill(&claude_home(&h).join("skills"), "docs-writer", &body("docs-writer"));
        let local = skill(&project.join(".claude").join("skills"), "docs-writer", &body("docs-writer"));
        fs::write(local.join("notes.md"), "附带文件").unwrap();
        let mut cfg = Config { projects: vec![project.display().to_string()], ..Default::default() };

        let (root, dir) = locate(&h, &cfg, &local.display().to_string()).unwrap();
        let stored = set_enabled_impl(&h, &mut cfg, &root, &dir, "docs-writer", false, &data).unwrap();
        assert!(!local.exists() && Path::new(&stored).join("notes.md").is_file() && Path::new(&stored).join(SKILL_FILE).is_file());
        assert!(stored.starts_with(&data.join("skills-disabled").display().to_string()));
        assert!(!claude_home(&h).join("settings.json").exists(), "不写 skillOverrides");
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

    fn drive_of(path: &Path) -> Option<String> {
        match path.components().next() {
            Some(std::path::Component::Prefix(p)) => Some(p.as_os_str().to_string_lossy().to_uppercase()),
            _ => None,
        }
    }

    /// Skills 目录在仓库所在盘（D:），停用区在系统临时目录（C:）：停用、启用都要走「复制 + 校验 + 删除」路径，文件内容不变
    #[test]
    fn claude_park_and_unpark_across_volumes() {
        let data = temp("cross-data");
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("jolu-skills-cross-{}-{}", std::process::id(), stamp()));
        if drive_of(&data) == drive_of(&base) {
            eprintln!("未测：临时目录与仓库在同一个盘（{:?}），无法模拟跨盘移动", drive_of(&data));
            fs::remove_dir_all(&data).ok();
            return;
        }
        let h = homes_at(&base);
        let dir = skill(&claude_home(&h).join("skills"), "cross-disk", &body("cross-disk"));
        fs::create_dir_all(dir.join("refs")).unwrap();
        fs::write(dir.join("refs").join("guide.md"), "跨盘附带文件\n第二行").unwrap();
        let bytes = vec![0u8, 1, 2, 254, 255];
        fs::write(dir.join("blob.bin"), &bytes).unwrap();
        let mut cfg = Config::default();

        let (root, located) = locate(&h, &cfg, &dir.display().to_string()).unwrap();
        let stored = set_enabled_impl(&h, &mut cfg, &root, &located, "cross-disk", false, &data).unwrap();
        let stored = PathBuf::from(stored);
        assert_ne!(drive_of(&stored), drive_of(&dir), "停用区应在另一个盘");
        assert!(!dir.exists(), "原目录已删除");
        assert_eq!(fs::read_to_string(stored.join(SKILL_FILE)).unwrap(), body("cross-disk"));
        assert_eq!(fs::read_to_string(stored.join("refs").join("guide.md")).unwrap(), "跨盘附带文件\n第二行");
        assert_eq!(fs::read(stored.join("blob.bin")).unwrap(), bytes);

        let (root, located) = locate(&h, &cfg, &stored.display().to_string()).unwrap();
        let back = set_enabled_impl(&h, &mut cfg, &root, &located, "cross-disk", true, &data).unwrap();
        assert!(same_path(&back, &dir) && !stored.exists());
        assert_eq!(fs::read_to_string(dir.join(SKILL_FILE)).unwrap(), body("cross-disk"));
        assert_eq!(fs::read_to_string(dir.join("refs").join("guide.md")).unwrap(), "跨盘附带文件\n第二行");
        assert_eq!(fs::read(dir.join("blob.bin")).unwrap(), bytes);
        assert!(load_config(&data).disabled.is_empty());
        fs::remove_dir_all(&base).ok();
        fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn codex_toggles_by_path_and_keeps_name_entries() {
        let base = temp("codex");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let a = skill(&base.join(".agents").join("skills"), "test-runner", &body("test-runner"));
        skill(&codex_home(&h).join("skills"), "test-runner", &body("test-runner"));
        skill(&codex_home(&h).join("skills"), "legacy", &body("legacy"));
        fs::create_dir_all(&codex_home(&h)).unwrap();
        fs::write(codex_home(&h).join("config.toml"), "# 我的配置\nmodel = \"gpt-5\"\n\n[[skills.config]]\nname = \"legacy\"\nenabled = false\n").unwrap();
        let (root, dir) = locate(&h, &cfg, &a.display().to_string()).unwrap();
        set_enabled_impl(&h, &mut cfg, &root, &dir, "test-runner", false, &data).unwrap();
        let text = fs::read_to_string(codex_home(&h).join("config.toml")).unwrap();
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
        let text = fs::read_to_string(codex_home(&h).join("config.toml")).unwrap();
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
        assert_eq!((all[0].source, all[0].tools.as_slice()), ("manual", ["codex", "opencode"].as_slice()));
        assert_eq!((all[1].source, all[1].tools.is_empty()), ("session", true));
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
        let dir = skill(&claude_home(&h).join("skills"), "safe", "---\ndescription: d\n---\n");
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
        skill(&claude_home(&h).join("skills"), "frontend-design", &body("frontend-design"));
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

    #[test]
    fn git_url_rules() {
        assert!(validate_git_url("https://github.com/o/r.git").is_ok());
        assert!(validate_git_url("https://github.com/anthropics/skills").is_ok());
        for bad in ["git@github.com:o/r", "http://x", "https://u:p@x/r", "file:///c:/x", "https://user@x/r", "https://", "https://x/r with space", "ssh://x/r"] {
            assert!(validate_git_url(bad).is_err(), "应拒绝 {bad}");
        }
        assert!(validate_git_branch("main").is_ok() && validate_git_branch("release/v1.2").is_ok());
        assert!(validate_git_branch("-x").is_err() && validate_git_branch("a b").is_err() && validate_git_branch("").is_err());
    }

    #[test]
    fn cleanup_only_removes_own_temp() {
        let base = temp("git-cleanup");
        let temp_root = base.join("tmp");
        let own = temp_root.join("jolu-git-20261023-1");
        fs::create_dir_all(own.join(".git").join("objects")).unwrap();
        let pack = own.join(".git").join("objects").join("pack.idx");
        fs::write(&pack, "x").unwrap();
        // git 把对象文件设为只读，清理时也要删得掉
        let mut perms = fs::metadata(&pack).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&pack, perms).unwrap();
        let other = temp_root.join("other-dir");
        fs::create_dir_all(&other).unwrap();
        let outside = base.join("jolu-git-outside");
        fs::create_dir_all(&outside).unwrap();
        for path in [&other, &outside, &temp_root] {
            assert!(cleanup_checkout(&temp_root, &path.display().to_string()).is_err(), "不应删除 {}", path.display());
            assert!(path.is_dir(), "目录应保持不变：{}", path.display());
        }
        assert!(cleanup_checkout(&temp_root, &format!("{}/../jolu-git-outside", other.display())).is_err());
        assert!(cleanup_checkout(&temp_root, "jolu-git-relative").is_err());
        assert!(outside.is_dir());
        // jolu-git-* 是指向外部目录的 junction 时拒绝删除，外部目录保持不变
        #[cfg(windows)]
        {
            let junction = temp_root.join("jolu-git-junction");
            fs::write(outside.join("keep.txt"), "外部文件").unwrap();
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&junction)
                .arg(&outside)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if made {
                assert!(cleanup_checkout(&temp_root, &junction.display().to_string()).is_err(), "junction 不应被清理");
                assert!(junction.exists() && outside.join("keep.txt").is_file());
                fs::remove_dir(&junction).unwrap();
            } else {
                println!("未测：无法创建 junction");
            }
        }
        cleanup_checkout(&temp_root, &own.display().to_string()).unwrap();
        assert!(!own.exists());
        // 已经删掉的再清理一次不报错
        assert!(cleanup_checkout(&temp_root, &own.display().to_string()).is_ok());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn git_missing_reports_install_hint() {
        let base = temp("git-missing");
        let err = clone_repo("jolu-no-such-git-binary", "https://github.com/o/r.git", None, &base.join("jolu-git-x"), Duration::from_secs(5)).unwrap_err();
        assert_eq!(err, GIT_MISSING);
        fs::remove_dir_all(&base).ok();
    }

    /// 联网：`cargo test skills::tests::git_clone_public_repo -- --ignored`
    #[test]
    #[ignore]
    fn git_clone_public_repo() {
        let base = temp("git-net");
        let h = homes_at(&base);
        let temp_root = base.join("tmp");
        let scan = import_git_with(&h, &Config::default(), &temp_root, "git", "https://github.com/anthropics/skills", None, "claude", Scope::Global, None).unwrap();
        assert!(Path::new(&scan.checkout_dir).starts_with(&temp_root));
        assert!(scan.scan.source.replace('\\', "/").ends_with("/skills"), "来源应为仓库里的 skills/：{}", scan.scan.source);
        assert!(scan.scan.items.iter().filter(|i| i.status == "ok").count() > 3);
        let bad = import_git_with(&h, &Config::default(), &temp_root, "git", "https://github.com/anthropics/skills", Some("jolu-no-such-branch"), "claude", Scope::Global, None).unwrap_err();
        assert!(bad.contains("没有这个分支"), "{bad}");
        cleanup_checkout(&temp_root, &scan.checkout_dir).unwrap();
        assert_eq!(fs::read_dir(&temp_root).unwrap().count(), 0, "失败的克隆也应删除临时目录");
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn repo_dir_name_from_url() {
        assert_eq!(repo_dir_name("https://github.com/o/my-skill.git"), "my-skill");
        assert_eq!(repo_dir_name("https://github.com/o/my-skill/"), "my-skill");
        assert_eq!(repo_dir_name("https://x/o/r.git?ref=1#top"), "r");
        assert_eq!(repo_dir_name("https://x/o/My%20Repo"), "My_20Repo");
        assert_eq!(repo_dir_name("https://x/o/.."), "_");
    }

    #[test]
    fn git_single_skill_repo_uses_repo_name_and_drops_git_dir() {
        let base = temp("git-single");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let checkout = base.join("tmp").join("jolu-git-20261023-1");
        let repo = checkout.join(repo_dir_name("https://github.com/o/my-skill.git"));
        fs::create_dir_all(repo.join(".git").join("objects")).unwrap();
        fs::write(repo.join(SKILL_FILE), body("my-skill")).unwrap();
        let pack = repo.join(".git").join("objects").join("pack.idx");
        fs::write(&pack, "x").unwrap();
        let mut perms = fs::metadata(&pack).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&pack, perms).unwrap();

        let scan = scan_clone(&h, &cfg, &repo, "claude", Scope::Global, None).unwrap();
        assert!(!repo.join(".git").exists(), "克隆后先删掉 .git");
        let names: Vec<_> = scan.items.iter().map(|i| (i.name.as_str(), i.status)).collect();
        assert_eq!(names, vec![("my-skill", "ok")], "名称是仓库名，不是 jolu-git-*");
        let choices = vec![ImportChoice { name: "my-skill".into(), action: "import".into() }];
        let result = import_with(&h, &mut cfg, &scan.source, "claude", Scope::Global, None, choices, true, &data).unwrap();
        assert_eq!(result.imported.len(), 1);
        let target = claude_home(&h).join("skills").join("my-skill");
        assert!(target.join(SKILL_FILE).is_file() && !target.join(".git").exists(), "目标目录里没有 .git");
        // 再导入一次命中冲突检查
        let again = scan_clone(&h, &cfg, &repo, "claude", Scope::Global, None).unwrap();
        assert_eq!(again.items[0].status, "conflict");

        // 仓库名不符合 Skill 命名规则时标为异常
        let odd = base.join("tmp").join("jolu-git-20261023-2").join(repo_dir_name("https://github.com/o/My.Skill.git"));
        skill(odd.parent().unwrap(), "My.Skill", &body("my-skill"));
        let scan = scan_clone(&h, &cfg, &odd, "claude", Scope::Global, None).unwrap();
        assert_eq!(scan.items[0].status, "invalid");
        assert!(scan.items[0].issue.as_deref().unwrap().contains("目录名不符合 Skill 名称规则"));
        fs::remove_dir_all(&base).ok();
    }

    fn new_body(name: &str) -> String {
        format!("---\nname: {name}\ndescription: 新版内容\n---\n")
    }

    fn overwrite(name: &str) -> Vec<ImportChoice> {
        vec![ImportChoice { name: name.into(), action: "overwrite".into() }]
    }

    #[test]
    fn overwrite_rejects_its_own_source_without_removing_files() {
        let base = temp("overwrite-self");
        let h = homes_at(&base);
        let mut cfg = Config::default();
        let root = write_root(&h, &cfg, "claude", Scope::Global, None).unwrap();
        let original = body("demo");
        let dir = skill(&root.path, "demo", &original);
        fs::write(dir.join("notes.txt"), "保留附件").unwrap();
        let result = import_with(&h, &mut cfg, &dir.display().to_string(), "claude", Scope::Global, None, overwrite("demo"), true, &base.join("data")).unwrap();
        assert_eq!(result.failed.len(), 1, "不能把安装项覆盖到自身");
        assert!(result.imported.is_empty());
        assert_eq!(fs::read_to_string(dir.join(SKILL_FILE)).unwrap(), original);
        assert_eq!(fs::read_to_string(dir.join("notes.txt")).unwrap(), "保留附件");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn overwrite_rejects_nested_source_without_removing_files() {
        let base = temp("overwrite-nested");
        let h = homes_at(&base);
        let mut cfg = Config::default();
        let root = write_root(&h, &cfg, "claude", Scope::Global, None).unwrap();
        let original = body("demo");
        let dest = skill(&root.path, "demo", &original);
        let replacement = format!("{original}\n新正文");
        let source = skill(&dest.join("nested"), "demo", &replacement);
        let result = import_with(&h, &mut cfg, &source.display().to_string(), "claude", Scope::Global, None, overwrite("demo"), true, &base.join("data")).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert!(result.imported.is_empty());
        assert_eq!(fs::read_to_string(dest.join(SKILL_FILE)).unwrap(), original);
        assert_eq!(fs::read_to_string(source.join(SKILL_FILE)).unwrap(), replacement);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn overwrite_replaces_tree_and_keeps_backup_and_source() {
        let base = temp("overwrite-tree");
        let h = homes_at(&base);
        let mut cfg = Config::default();
        let root = write_root(&h, &cfg, "claude", Scope::Global, None).unwrap();
        let original = body("demo");
        let dest = skill(&root.path, "demo", &original);
        fs::write(dest.join("old.txt"), "旧附件").unwrap();
        let replacement = format!("{original}\n新正文");
        let source = skill(&base.join("source"), "demo", &replacement);
        fs::write(source.join("new.txt"), "新附件").unwrap();
        let data = base.join("data");
        let result = import_with(&h, &mut cfg, &source.display().to_string(), "claude", Scope::Global, None, overwrite("demo"), true, &data).unwrap();
        assert!(result.failed.is_empty());
        assert_eq!(result.imported.len(), 1);
        assert_eq!(fs::read_to_string(dest.join(SKILL_FILE)).unwrap(), replacement);
        assert_eq!(fs::read_to_string(source.join(SKILL_FILE)).unwrap(), replacement);
        assert_eq!(fs::read_to_string(dest.join("new.txt")).unwrap(), "新附件");
        assert!(!dest.join("old.txt").exists());
        let backup = fs::read_dir(data.join("skill-backups/overwritten")).unwrap().next().unwrap().unwrap().path();
        assert_eq!(fs::read_to_string(backup.join(SKILL_FILE)).unwrap(), original);
        assert_eq!(fs::read_to_string(backup.join("old.txt")).unwrap(), "旧附件");
        assert_eq!(fs::read_dir(&root.path).unwrap().count(), 1, "成功后不残留暂存目录");
        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn overwrite_copy_failure_preserves_original_installation() {
        use std::os::windows::fs::OpenOptionsExt;
        let base = temp("overwrite-failed-copy");
        let h = homes_at(&base);
        let mut cfg = Config::default();
        let root = write_root(&h, &cfg, "claude", Scope::Global, None).unwrap();
        let original = format!("{}\n旧正文", body("demo"));
        let dest = skill(&root.path, "demo", &original);
        fs::write(dest.join("notes.txt"), "旧附件").unwrap();
        let source = skill(&base.join("source"), "demo", &format!("{}\n新正文", body("demo")));
        fs::write(source.join("locked.txt"), "读取失败").unwrap();
        let locked = fs::OpenOptions::new().read(true).share_mode(0).open(source.join("locked.txt")).unwrap();
        let result = import_with(&h, &mut cfg, &source.display().to_string(), "claude", Scope::Global, None, overwrite("demo"), true, &base.join("data")).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert!(result.imported.is_empty());
        assert_eq!(fs::read_to_string(dest.join(SKILL_FILE)).unwrap(), original, "复制失败不改变现有正文");
        assert_eq!(fs::read_to_string(dest.join("notes.txt")).unwrap(), "旧附件");
        drop(locked);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn overwrite_keeps_enabled_state() {
        let base = temp("ow-enabled");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let source = base.join("downloads");
        skill(&source, "api-review", &new_body("api-review"));
        let src = source.display().to_string();

        // Claude：启用中的 Skill 被覆盖后仍在原位置启用，「导入后启用」关闭也不停用
        let target = skill(&claude_home(&h).join("skills"), "api-review", &body("api-review"));
        let result = import_with(&h, &mut cfg, &src, "claude", Scope::Global, None, overwrite("api-review"), false, &data).unwrap();
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert!(same_path(&result.imported[0], &target));
        assert_eq!(fs::read_to_string(target.join(SKILL_FILE)).unwrap(), new_body("api-review"), "内容已更新");
        assert!(cfg.disabled.is_empty() && !data.join("skills-disabled").exists(), "没有移进停用区");
        let o = overview_with(&h, &cfg, "claude").unwrap();
        assert_eq!(o.items.iter().find(|i| i.name == "api-review").unwrap().status, "enabled");

        // Codex：启用中的 Skill 被覆盖后 config.toml 不出现停用条目
        let codex_target = skill(&base.join(".agents").join("skills"), "api-review", &body("api-review"));
        let result = import_with(&h, &mut cfg, &src, "codex", Scope::Global, None, overwrite("api-review"), false, &data).unwrap();
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert_eq!(fs::read_to_string(codex_target.join(SKILL_FILE)).unwrap(), new_body("api-review"));
        assert!(!codex_home(&h).join("config.toml").exists(), "不写停用条目");
        let o = overview_with(&h, &cfg, "codex").unwrap();
        assert_eq!(o.items.iter().find(|i| i.name == "api-review").unwrap().disabled_by, None);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn overwrite_keeps_disabled_state() {
        let base = temp("ow-disabled");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let source = base.join("downloads");
        skill(&source, "api-review", &new_body("api-review"));
        let src = source.display().to_string();

        // Claude：停用区里的 Skill 算作同名冲突，覆盖写到停用区里那一份，不在原位置再放一份
        let original = skill(&claude_home(&h).join("skills"), "api-review", &body("api-review"));
        let (root, dir) = locate(&h, &cfg, &original.display().to_string()).unwrap();
        let stored = set_enabled_impl(&h, &mut cfg, &root, &dir, "api-review", false, &data).unwrap();
        let registry = serde_json::to_string(&load_config(&data).disabled).unwrap();
        let scan = import_scan_with(&h, &cfg, &src, "claude", Scope::Global, None).unwrap();
        assert_eq!(scan.items[0].status, "conflict", "停用区里的同名 Skill 也是冲突");
        let result = import_with(&h, &mut cfg, &src, "claude", Scope::Global, None, overwrite("api-review"), true, &data).unwrap();
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert!(same_path(&result.imported[0], Path::new(&stored)));
        assert!(!original.exists(), "原位置没有第二份");
        assert_eq!(fs::read_to_string(Path::new(&stored).join(SKILL_FILE)).unwrap(), new_body("api-review"), "停用区里的内容已更新");
        assert_eq!(serde_json::to_string(&cfg.disabled).unwrap(), registry, "停用登记不变");
        assert!(fs::read_dir(data.join("skill-backups").join("overwritten")).unwrap().count() == 1, "覆盖前已备份");
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let items: Vec<_> = o.items.iter().filter(|i| i.name == "api-review").collect();
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].status, items[0].disabled_by), ("disabled", Some("parked")));
        // 停用区里的同名 Skill 选「导入」也要报冲突，不能在原位置再放一份
        let choices = vec![ImportChoice { name: "api-review".into(), action: "import".into() }];
        let result = import_with(&h, &mut cfg, &src, "claude", Scope::Global, None, choices, true, &data).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert!(!original.exists());

        // Codex：按路径停用的 Skill 被覆盖后仍停用，config.toml 原样
        let codex_target = skill(&base.join(".agents").join("skills"), "api-review", &body("api-review"));
        let (root, dir) = locate(&h, &cfg, &codex_target.display().to_string()).unwrap();
        set_enabled_impl(&h, &mut cfg, &root, &dir, "api-review", false, &data).unwrap();
        let config = fs::read_to_string(codex_home(&h).join("config.toml")).unwrap();
        let result = import_with(&h, &mut cfg, &src, "codex", Scope::Global, None, overwrite("api-review"), true, &data).unwrap();
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert_eq!(fs::read_to_string(codex_target.join(SKILL_FILE)).unwrap(), new_body("api-review"));
        assert_eq!(fs::read_to_string(codex_home(&h).join("config.toml")).unwrap(), config, "config.toml 不变");
        let o = overview_with(&h, &cfg, "codex").unwrap();
        assert_eq!(o.items.iter().find(|i| i.name == "api-review").unwrap().disabled_by, Some("path"));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn enable_after_import_only_for_new() {
        let base = temp("ow-new");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let source = base.join("downloads");
        skill(&source, "fresh-one", &body("fresh-one"));
        skill(&source, "kept-on", &new_body("kept-on"));
        let existing = skill(&claude_home(&h).join("skills"), "kept-on", &body("kept-on"));
        let choices = vec![
            ImportChoice { name: "fresh-one".into(), action: "import".into() },
            ImportChoice { name: "kept-on".into(), action: "overwrite".into() },
        ];
        let result = import_with(&h, &mut cfg, &source.display().to_string(), "claude", Scope::Global, None, choices, false, &data).unwrap();
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let state = |name: &str| o.items.iter().find(|i| i.name == name).map(|i| (i.status, i.disabled_by)).unwrap();
        assert_eq!(state("fresh-one"), ("disabled", Some("parked")), "新导入的按开关停用");
        assert_eq!(state("kept-on"), ("enabled", None), "覆盖的保持原状态");
        assert_eq!(fs::read_to_string(existing.join(SKILL_FILE)).unwrap(), new_body("kept-on"));
        assert_eq!(cfg.disabled.len(), 1);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn scan_clone_stops_when_git_removal_unconfirmed() {
        let base = temp("git-unknown");
        let h = homes_at(&base);
        let cfg = Config::default();
        // 路径里的 NUL 让 symlink_metadata 返回 InvalidInput（不是 NotFound），无法确认 .git 是否还在
        let repo = base.join("bad\0repo");
        let err = scan_clone(&h, &cfg, &repo, "claude", Scope::Global, None).unwrap_err();
        assert!(err.starts_with("无法确认 .git 已删除："), "{err}");
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn git_import_source_prefers_root_then_skills_dir() {
        let base = temp("git-source");
        let flat = base.join("flat");
        skill(&flat, "a-skill", &body("a-skill"));
        assert_eq!(git_import_source(&flat), flat);
        let nested = base.join("nested");
        skill(&nested.join("skills"), "b-skill", &body("b-skill"));
        skill(&nested, "template", &body("template"));
        assert_eq!(git_import_source(&nested), nested.join("skills"));
        let single = base.join("single");
        fs::create_dir_all(&single).unwrap();
        fs::write(single.join(SKILL_FILE), body("single")).unwrap();
        assert_eq!(git_import_source(&single), single);
        let empty = base.join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(git_import_source(&empty), empty);
        fs::remove_dir_all(&base).ok();
    }
}

// 任务 3.3 开始
/* ───────────────────────── 自动化选择 Skills（D-08） ───────────────────────── */

/// 自动化「Skills（可选）」的一个候选项
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SkillOption {
    pub name: String,
    pub description: Option<String>,
    /// global / project / system / synced（与 [`Scope`] 的序列化名一致）
    pub scope: String,
    /// 项目级 Skill 所在的项目目录
    pub project: Option<String>,
}

/// 目标工具会加载的 Skills：全局目录 + `cwd` 下的项目目录。只列已启用、格式有效、名称合规的项，同名只保留第一份。
/// 停用区里的副本不在扫描目录里，自然不出现；工具配置按名称 / 路径停用的项被过滤
pub fn enabled_skills(platform: &str, cwd: Option<&Path>) -> Result<Vec<SkillOption>, String> {
    enabled_skills_with(&homes()?, platform, cwd)
}

pub fn enabled_skills_with(h: &Homes, platform: &str, cwd: Option<&Path>) -> Result<Vec<SkillOption>, String> {
    if !matches!(platform,"claude"|"codex") {return Err("自动化仅支持 Claude Code 与 Codex".into())}
    if spec(platform).is_none() {
        return Err(format!("{platform} 暂未接入 Skills 管理"));
    }
    // 只扫描任务的工作目录：不读 skills.json 里的项目列表与忽略设置，原工具在该目录执行时总会加载其项目 Skills
    let cfg = Config { projects: cwd.map(|p| vec![p.display().to_string()]).unwrap_or_default(), ..Config::default() };
    let overview = overview_with(h, &cfg, platform)?;
    let mut seen = HashSet::new();
    Ok(overview
        .items
        .into_iter()
        .filter(|i| i.enabled && i.status != "invalid" && valid_name(&i.name) && seen.insert(i.name.clone()))
        .map(|i| SkillOption {
            scope: serde_json::to_value(i.scope).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
            name: i.name,
            description: i.description,
            project: i.project,
        })
        .collect())
}

#[cfg(test)]
mod task_3_3_tests {
    use super::*;

    fn skill(root: &Path, name: &str, body: &str) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(SKILL_FILE), body).unwrap();
    }

    #[test]
    fn enabled_skills_lists_global_and_cwd_project() {
        let base = std::env::temp_dir().join(format!("jolu-skills-enabled-{}-{}", std::process::id(), stamp()));
        let h = homes_from(base.clone(), |_| None);
        let claude_home = tool_home(&h, spec("claude").unwrap());
        let global = claude_home.join("skills");
        let body = |n: &str| format!("---\nname: {n}\ndescription: {n} 描述\n---\n");
        skill(&global, "api-review", &body("api-review"));
        skill(&global, "no-desc", "---\nname: no-desc\n---\n");
        skill(&global, "Upper", "---\ndescription: 目录名不合规\n---\n");
        skill(&global, "turned-off", &body("turned-off"));
        fs::write(claude_home.join("settings.json"), r#"{"skillOverrides":{"turned-off":"off"}}"#).unwrap();
        let project = base.join("proj");
        skill(&project.join(".claude").join("skills"), "test-check", &body("test-check"));
        skill(&project.join(".claude").join("skills"), "api-review", &body("api-review"));
        let other = base.join("other");
        skill(&other.join(".claude").join("skills"), "elsewhere", &body("elsewhere"));

        let names = |list: Vec<SkillOption>| list.into_iter().map(|o| (o.name, o.scope, o.project.is_some())).collect::<Vec<_>>();
        assert_eq!(
            names(enabled_skills_with(&h, "claude", Some(&project)).unwrap()),
            vec![("api-review".into(), "global".into(), false), ("test-check".into(), "project".into(), true)],
            "只列已启用、有效、名称合规的；同名只留一份；不扫描其他项目",
        );
        assert_eq!(names(enabled_skills_with(&h, "claude", None).unwrap()), vec![("api-review".into(), "global".into(), false)]);
        let first = enabled_skills_with(&h, "claude", Some(&project)).unwrap().remove(0);
        assert_eq!(first.description.as_deref(), Some("api-review 描述"));
        assert!(enabled_skills_with(&h, "codex", Some(&project)).unwrap().is_empty(), "按工具区分");
        assert!(enabled_skills_with(&h, "all", None).is_err());
        assert!(enabled_skills_with(&h, "gemini", None).is_err());
        fs::remove_dir_all(&base).ok();
    }
}
// 任务 3.3 结束
