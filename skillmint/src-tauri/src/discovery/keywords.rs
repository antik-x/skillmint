//! SPEC-I2: built-in capability-gap category keyword table.
//!
//! The category table is data-driven so future builds can make it user-editable
//! without changing detector logic.

/// One capability category with its matching keywords.
#[derive(Debug, Clone)]
pub struct Category {
    pub key: &'static str,
    pub label: &'static str,
    pub keywords: &'static [&'static str],
}

/// All built-in categories. Keep labels stable — the UI may render them.
pub fn categories() -> &'static [Category] {
    &[
        Category {
            key: "debug",
            label: "调试",
            keywords: &["debug", "调试", "排查", "troubleshoot", "error", "bug", "crash", "异常"],
        },
        Category {
            key: "refactor",
            label: "重构",
            keywords: &["refactor", "重构", "重写", "整理代码", "clean up", "extract", "拆分"],
        },
        Category {
            key: "test",
            label: "测试",
            keywords: &["test", "测试", "unit test", "assert", "mock", "jest", "pytest", "验证"],
        },
        Category {
            key: "doc",
            label: "文档",
            keywords: &["doc", "文档", "readme", "comment", "说明", "api doc", "注释"],
        },
        Category {
            key: "deploy",
            label: "部署",
            keywords: &["deploy", "部署", "release", "发布", "build", "打包", "ci/cd", "pipeline"],
        },
        Category {
            key: "database",
            label: "数据库",
            keywords: &["database", "数据库", "sql", "schema", "migration", "query", "索引"],
        },
        Category {
            key: "ui",
            label: "UI",
            keywords: &["ui", "界面", "component", "frontend", "react", "vue", "css", "样式"],
        },
        Category {
            key: "performance",
            label: "性能",
            keywords: &["performance", "性能", "optimize", "slow", "memory", "cpu", "cache", "benchmark"],
        },
        Category {
            key: "security",
            label: "安全",
            keywords: &["security", "安全", "auth", "permission", "oauth", "jwt", "encrypt", "漏洞"],
        },
        Category {
            key: "script",
            label: "脚本",
            keywords: &["script", "脚本", "shell", "bash", "automation", "cli", "工具脚本"],
        },
    ]
}

/// Match a prompt against all categories and return the matching keys.
pub fn match_categories(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    categories()
        .iter()
        .filter(|c| c.keywords.iter().any(|k| lower.contains(&k.to_lowercase())))
        .map(|c| c.key)
        .collect()
}

