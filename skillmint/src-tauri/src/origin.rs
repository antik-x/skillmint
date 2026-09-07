//! 数据来源（P5/ACP 防自吞尾）：区分「用户原始数据」与「SkillMint 驱动本地
//! Agent 分析时产生的副产品数据」。
//!
//! 背景：ACP 分析（语义分类、每日摘要）会驱动本机 Agent CLI（Claude Code /
//! Kimi Code）真正执行任务，Agent 会在自己的数据目录（~/.claude/projects 等）
//! 落新的会话文件。若不处理，下一轮采集会把它们当普通用户会话收进来，进入
//! 摘要/周报/评估/收件箱全部管线——分类请求模板反复出现会触发重复模式检测的
//! 垃圾发现，未分类的副产品 prompt 又会被送去分类（递归自吞尾）。
//!
//! 两层防线（Q4/Q6 决议）：
//! 1. 源头隔离：ACP 调用时的 `session/new.cwd` 固定为 `~/.skillmint/acp-workspace`
//!    哨兵目录。Agent 按 cwd 归档会话（Claude Code 尤其如此），副产品会话集中
//!    落在可识别路径；采集端按 project_path 命中哨兵目录打标。相比重定向
//!    CLAUDE_CONFIG_DIR 等 HOME 变量，cwd 哨兵不迁移登录凭据，用户现有登录与
//!    额度零影响。
//! 2. 内容哨兵：所有分析类 prompt 的系统提示头部带 [skillmint-analysis] 标记，
//!    采集端按文本命中兜底打标（防 Agent 未按 cwd 归档的边缘情况）。
//!
//! 打标列 `origin`（'user' | 'skillmint_acp'）落在三张采集表上，所有分析消费
//! 端统一过滤；数据照采照存，可审计可回溯。

/// origin 列取值：用户原始数据。
pub const ORIGIN_USER: &str = "user";
/// origin 列取值：SkillMint ACP 分析的副产品数据。
pub const ORIGIN_SKILLMINT_ACP: &str = "skillmint_acp";

/// 分析类 prompt 的内容哨兵（出现在系统提示首行）。采集端以文本命中兜底打标。
pub const ANALYSIS_SENTINEL: &str = "[skillmint-analysis]";

/// ACP 会话的固定工作目录（session/new.cwd）。副产品会话应集中落在此目录下。
pub fn acp_workspace_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/"))
        .join(".skillmint")
        .join("acp-workspace")
}

/// 路径是否位于 ACP 工作目录内。
pub fn is_workspace_path(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return false;
    }
    std::path::Path::new(trimmed).starts_with(acp_workspace_dir())
}

/// 文本是否携带分析哨兵标记。
pub fn has_analysis_sentinel(text: &str) -> bool {
    text.contains(ANALYSIS_SENTINEL)
}

/// 分析类任务的系统提示统一加内容哨兵（防线 2）：本地 Agent 执行分析时，
/// 该标记会出现在其自产会话里，采集端据此兜底打 origin 标、防止自吞尾。
pub fn analysis_system_prompt(base: &str) -> String {
    format!("{}\n{}", ANALYSIS_SENTINEL, base)
}

/// 会话级来源判定：cwd 命中哨兵目录，或标题携带内容哨兵。
pub fn session_origin(project_path: Option<&str>, title_or_prompt: Option<&str>) -> &'static str {
    if project_path.map(is_workspace_path).unwrap_or(false) {
        return ORIGIN_SKILLMINT_ACP;
    }
    if title_or_prompt.map(has_analysis_sentinel).unwrap_or(false) {
        return ORIGIN_SKILLMINT_ACP;
    }
    ORIGIN_USER
}

/// prompt 级来源判定：继承会话来源；文本命中内容哨兵时升级为副产品。
pub fn prompt_origin(session_origin: &str, prompt_text: Option<&str>) -> &'static str {
    if session_origin == ORIGIN_SKILLMINT_ACP {
        return ORIGIN_SKILLMINT_ACP;
    }
    if prompt_text.map(has_analysis_sentinel).unwrap_or(false) {
        return ORIGIN_SKILLMINT_ACP;
    }
    ORIGIN_USER
}

/// SQL 过滤片段口径：老数据 origin 为 NULL，一律视为 user。
pub const USER_ORIGIN_SQL: &str = "IFNULL(origin, 'user') != 'skillmint_acp'";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_path_detection() {
        let ws = acp_workspace_dir();
        assert!(is_workspace_path(ws.to_str().unwrap()));
        assert!(is_workspace_path(&format!("{}/sub/dir", ws.display())));
        assert!(!is_workspace_path("/Users/someone/dev/myproject"));
        assert!(!is_workspace_path(""));
        // 用户家目录下其他路径不受影响。
        assert!(!is_workspace_path("~/.skillmint/hub"));
    }

    #[test]
    fn session_origin_by_cwd_and_sentinel() {
        let ws = acp_workspace_dir().to_str().unwrap().to_string();
        assert_eq!(session_origin(Some(&ws), None), ORIGIN_SKILLMINT_ACP);
        assert_eq!(session_origin(Some("/tmp/x"), Some(&format!("{} 你好", ANALYSIS_SENTINEL))), ORIGIN_SKILLMINT_ACP);
        assert_eq!(session_origin(Some("/tmp/x"), Some("普通会话")), ORIGIN_USER);
        assert_eq!(session_origin(None, None), ORIGIN_USER);
    }

    #[test]
    fn prompt_origin_inherits_and_upgrades() {
        assert_eq!(prompt_origin(ORIGIN_SKILLMINT_ACP, Some("随便什么")), ORIGIN_SKILLMINT_ACP);
        assert_eq!(prompt_origin(ORIGIN_USER, Some(&format!("x {}", ANALYSIS_SENTINEL))), ORIGIN_SKILLMINT_ACP);
        assert_eq!(prompt_origin(ORIGIN_USER, Some("真实用户输入")), ORIGIN_USER);
    }
}
