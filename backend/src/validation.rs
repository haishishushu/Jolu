//! 隔离验收只在调试构建中启用，正式程序始终使用正常数据目录。
pub fn data_dir() -> Option<std::path::PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(value) = std::env::var_os("JOLU_TEST_DIR") {
        let path = std::path::PathBuf::from(value);
        assert!(path.is_absolute() && path.join(".acceptance-profile").is_file(),
            "验收目录必须是绝对路径且包含 .acceptance-profile 标记");
        return Some(path);
    }
    None
}

pub fn isolated() -> bool { data_dir().is_some() }

#[cfg(all(test, debug_assertions))]
mod tests {
    #[test]
    fn isolated_reads_stay_off_user_files() {
        // 独立进程设置环境变量，避免污染并行运行的其他测试。
        if std::env::var_os("JOLU_ISOLATION_TEST_CHILD").is_none() {
            let root = std::env::temp_dir().join(format!("jolu-isolation-{}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join(".acceptance-profile"), "test").unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "validation::tests::isolated_reads_stay_off_user_files", "--nocapture"])
                .env("JOLU_ISOLATION_TEST_CHILD", "1")
                .env("JOLU_TEST_DIR", &root)
                .env("USERPROFILE", root.join("synthetic-user"))
                .env("HOME", root.join("synthetic-user"))
                .env("CODEX_HOME", root.join("synthetic-user/.codex"))
                .output().unwrap();
            std::fs::remove_dir_all(&root).unwrap();
            assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        let root = super::data_dir().unwrap();
        let error = crate::cli_apply::apply_to_cli("codex", "api", "synthetic-isolated", None, &root.join("backups"), None).err().unwrap();
        assert!(error.contains("隔离验收"));
        assert!(!root.join("backups").exists());
        let source = root.join("synthetic-user/.claude/projects/example/session.jsonl");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "{\"type\":\"user\",\"cwd\":\"private-project\",\"message\":{\"content\":\"private text\"}}\n").unwrap();
        let mut conn = crate::db::open(&root.join("test.db")).unwrap();
        let scan = crate::collector::scan(&mut conn);
        assert_eq!(scan.files_scanned, 0, "托盘使用的全量采集入口不能扫描用户文件");
        assert!(!scan.errors.is_empty(), "隔离拒绝不能报告为成功采集");
        let scan = crate::collector::scan_paths(&mut conn, [source.clone()]);
        assert_eq!(scan.files_scanned, 0);
        assert!(!scan.errors.is_empty());
        conn.execute("INSERT INTO scan_state(path, offset, mtime, parser_session_id) VALUES(?1, 0, 0, 'session')", [source.to_string_lossy().as_ref()]).unwrap();
        assert!(crate::session_threads::source_files(&conn).unwrap().is_empty());
        assert!(crate::session_threads::read_cwd(&source).is_none());
        assert!(crate::session_threads::read_lines(&source).is_err());
        let messages = crate::session_threads::messages(&conn, "claude", "session", None, 20).unwrap();
        assert!(!messages.available);
        assert!(messages.messages.is_empty());
    }
}
