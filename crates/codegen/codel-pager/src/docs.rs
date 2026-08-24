//! In-app how-to documentation data (embedded markdown).
//!
//! Single source of truth: two static arrays (`USER_GUIDE`, `REFERENCE_DOCS`)
//! hold every doc. All lookups are zero-allocation; `DocEntry` exists only for
//! backward compatibility with the TUI doc picker.

/// A compile-time document entry. All fields are `&'static str`.
#[derive(Debug)]
pub struct Doc {
    pub filename: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub content: &'static str,
}

/// Owned variant for the TUI doc picker (backward compat).
#[derive(Debug, Clone)]
pub struct DocEntry {
    pub title: String,
    pub description: String,
    /// Embedded markdown content.
    pub content: &'static str,
}

impl From<&Doc> for DocEntry {
    fn from(d: &Doc) -> Self {
        Self {
            title: d.title.into(),
            description: d.description.into(),
            content: d.content,
        }
    }
}

// ── Static doc tables ────────────────────────────────────────────────────────

macro_rules! guide {
    ($file:literal, $title:literal, $desc:literal) => {
        Doc {
            filename: $file,
            title: $title,
            description: $desc,
            content: include_str!(concat!("../docs/user-guide/", $file)),
        }
    };
}

pub static USER_GUIDE: &[Doc] = &[
    guide!(
        "01-快速入门.md",
        "快速入门",
        "安装、首次启动与基本交互"
    ),
    guide!(
        "02-模型配置.md",
        "模型配置",
        "模型与 API 密钥配置"
    ),
    guide!(
        "03-快捷键指南.md",
        "快捷键指南",
        "TUI 全部按键绑定的完整参考"
    ),
    guide!(
        "04-斜杠指令.md",
        "斜杠指令",
        "所有 / 指令，包括目标、研究与工作流管理"
    ),
    guide!(
        "05-系统配置.md",
        "系统配置",
        "config.toml、pager.toml、环境变量与文件位置"
    ),
    guide!(
        "06-主题与外观.md",
        "主题与外观",
        "主题、色彩支持与 pager.toml 自定义"
    ),
    guide!(
        "07-mcp服务器.md",
        "MCP 服务器",
        "通过 MCP 配置外部工具集成"
    ),
    guide!(
        "08-技能扩展.md",
        "技能扩展",
        "创建和使用可复用的提示词包"
    ),
    guide!(
        "09-插件系统.md",
        "插件系统",
        "安装、管理和创建插件包"
    ),
    guide!(
        "10-钩子机制.md",
        "钩子机制",
        "工具使用前后的项目生命周期脚本"
    ),
    guide!(
        "11-自定义模型.md",
        "自定义模型",
        "BYOK、Ollama、OpenAI 兼容端点"
    ),
    guide!(
        "12-项目规范律条.md",
        "项目规范律条 (AGENTS.md)",
        "按目录的指令与优先级规则"
    ),
    guide!(
        "13-长期记忆.md",
        "长期记忆",
        "跨会话知识持久化与检索"
    ),
    guide!(
        "14-无头交互模式.md",
        "无头交互模式",
        "用于自动化和 CI/CD 的非交互式 CLI"
    ),
    guide!(
        "15-智使模式.md",
        "智使模式与 IDE 集成",
        "ACP stdio 传输、WebSocket 中继、SDK 集成"
    ),
    guide!(
        "16-分身智使.md",
        "分身智使",
        "生成具有专业角色的并行子智使"
    ),
    guide!(
        "17-会话管理.md",
        "会话管理",
        "保存、加载、恢复、回退和压缩会话"
    ),
    guide!(
        "18-沙盒安全环境.md",
        "沙盒安全环境",
        "操作系统级文件系统与网络隔离"
    ),
    guide!(
        "19-筹划模式.md",
        "筹划模式",
        "带审批对话框的结构化规划"
    ),
    guide!(
        "20-后台任务.md",
        "后台任务与监控",
        "后台命令、/loop、monitor、scheduler"
    ),
    guide!(
        "21-终端兼容性.md",
        "终端兼容性与故障排除",
        "tmux、Byobu、Zellij、SSH、truecolor、剪贴板与诊断"
    ),
    guide!(
        "22-权限与安全.md",
        "权限与安全",
        "模式、授权顺序、允许/询问/拒绝规则、匹配与钩子"
    ),
    guide!(
        "23-控制面板.md",
        "控制面板",
        "智使控制面板：概览、附着与管理会话"
    ),
    guide!(
        "25-工作流.md",
        "工作流",
        "Rhai 脚本化工作流编排引擎的使用与编写"
    ),
];

/// Non-user-guide reference docs. Separate from USER_GUIDE because they
/// live under `docs/` (not `docs/user-guide/`), are not extracted to disk,
/// and do not follow the NN-*.md managed naming pattern. Bundled via
/// `include_str!` so they are available at runtime without a docs path.
static REFERENCE_DOCS: &[Doc] = &[
    Doc {
        filename: "hooks-and-plugins.md",
        title: "Hooks & Plugins Guide",
        description: "Using hooks, plugins, and marketplace",
        content: include_str!("../docs/hooks-and-plugins.md"),
    },
    Doc {
        filename: "custom-hooks.md",
        title: "Creating Custom Hooks",
        description: "Writing your own hooks and matchers",
        content: include_str!("../docs/custom-hooks.md"),
    },
];

// ── Public API ───────────────────────────────────────────────────────────────

/// Find a doc by title (case-insensitive). Returns the static entry.
pub fn find_doc(title: &str) -> Option<&'static Doc> {
    USER_GUIDE
        .iter()
        .chain(REFERENCE_DOCS.iter())
        .find(|d| d.title.eq_ignore_ascii_case(title))
}

/// All doc titles, zero allocation.
pub fn all_titles() -> impl Iterator<Item = &'static str> {
    USER_GUIDE
        .iter()
        .chain(REFERENCE_DOCS.iter())
        .map(|d| d.title)
}

/// Returns the content of a how-to document by exact title match (case-insensitive).
pub fn get_howto_doc(title: &str) -> Option<&'static str> {
    find_doc(title).map(|d| d.content)
}

/// Returns a list of available how-to titles for the model to choose from.
pub fn list_howto_titles() -> Vec<String> {
    all_titles().map(String::from).collect()
}

/// Returns all docs as owned `DocEntry` values for the TUI doc picker.
pub fn default_howto_entries() -> Vec<DocEntry> {
    USER_GUIDE
        .iter()
        .chain(REFERENCE_DOCS.iter())
        .map(DocEntry::from)
        .collect()
}

/// Extract user-guide docs to `<codel_home>/docs/user-guide/`.
///
/// Called from the pager binary startup so the model can read them from disk.
pub fn extract_user_guide_docs(codel_home: &std::path::Path) {
    let docs_dir = codel_home.join("docs").join("user-guide");
    if let Err(e) = std::fs::create_dir_all(&docs_dir) {
        tracing::warn!(error = %e, "Failed to create user-guide docs directory");
        return;
    }
    for doc in USER_GUIDE {
        if let Err(e) = std::fs::write(docs_dir.join(doc.filename), doc.content) {
            tracing::debug!(error = %e, filename = doc.filename, "Failed to extract user-guide doc");
        }
    }
    // Clean up stale managed docs (files removed from USER_GUIDE since last run).
    // Only remove files matching the managed naming pattern (NN-*.md).
    if let Ok(entries) = std::fs::read_dir(&docs_dir) {
        let valid: std::collections::HashSet<&str> =
            USER_GUIDE.iter().map(|d| d.filename).collect();
        for dir_entry in entries.flatten() {
            if let Some(name) = dir_entry.file_name().to_str() {
                let is_managed = name.len() > 3
                    && name.as_bytes()[0].is_ascii_digit()
                    && name.as_bytes()[1].is_ascii_digit()
                    && name.as_bytes()[2] == b'-'
                    && name.ends_with(".md");
                if is_managed
                    && !valid.contains(name)
                    && let Err(e) = std::fs::remove_file(dir_entry.path())
                {
                    tracing::debug!(error = %e, filename = name, "Failed to remove stale user-guide doc");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_guide_entries_are_valid() {
        for doc in USER_GUIDE {
            assert!(!doc.content.is_empty(), "Doc {} is empty", doc.filename);
            assert!(
                !doc.title.is_empty(),
                "Doc {} has empty title",
                doc.filename
            );
            assert!(
                !doc.description.is_empty(),
                "Doc {} has empty description",
                doc.filename
            );
            assert!(
                doc.content.starts_with('#'),
                "Doc {} should start with a markdown header",
                doc.filename
            );
        }
    }

    #[test]
    fn user_guide_entries_have_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for doc in USER_GUIDE {
            assert!(
                seen.insert(doc.filename),
                "Duplicate doc in list: {}",
                doc.filename
            );
        }
    }

    #[test]
    fn default_howto_entries_includes_all_user_guide_docs() {
        let entries = default_howto_entries();
        assert_eq!(entries.len(), USER_GUIDE.len() + REFERENCE_DOCS.len());
        for (i, doc) in USER_GUIDE.iter().enumerate() {
            assert_eq!(entries[i].title, doc.title, "Entry {} title mismatch", i);
        }
    }

    #[test]
    fn find_doc_is_case_insensitive() {
        let doc = find_doc("getting started").expect("should find Getting Started");
        assert_eq!(doc.title, "Getting Started");
        assert!(find_doc("nonexistent guide").is_none());
    }

    #[test]
    fn all_titles_covers_both_tables() {
        let titles: Vec<_> = all_titles().collect();
        assert_eq!(titles.len(), USER_GUIDE.len() + REFERENCE_DOCS.len());
    }

    #[test]
    fn get_howto_doc_delegates_to_find_doc() {
        assert!(get_howto_doc("Getting Started").is_some());
        assert!(get_howto_doc("Hooks & Plugins Guide").is_some());
        assert!(get_howto_doc("no such doc").is_none());
    }

    #[test]
    fn list_howto_titles_returns_all() {
        let titles = list_howto_titles();
        assert_eq!(titles.len(), USER_GUIDE.len() + REFERENCE_DOCS.len());
    }

    #[test]
    fn extract_writes_docs_and_cleans_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let docs_dir = tmp.path().join("docs").join("user-guide");

        std::fs::create_dir_all(&docs_dir).unwrap();
        std::fs::write(docs_dir.join("99-removed.md"), "stale").unwrap();
        std::fs::write(docs_dir.join("notes.md"), "user notes").unwrap();

        extract_user_guide_docs(tmp.path());

        for doc in USER_GUIDE {
            let path = docs_dir.join(doc.filename);
            assert!(path.exists(), "Expected doc {} to exist", doc.filename);
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                doc.content,
                "Content mismatch for {}",
                doc.filename
            );
        }
        assert!(
            !docs_dir.join("99-removed.md").exists(),
            "Stale doc should be cleaned up"
        );
        assert!(
            docs_dir.join("notes.md").exists(),
            "User file should not be deleted"
        );
    }
}
