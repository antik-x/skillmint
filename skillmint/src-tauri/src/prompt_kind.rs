//! Prompt 净化（P4/E2-S2.1.4）：区分"用户真实输入"与"系统/Skill 注入文本"。
//!
//! 背景：采集到的 prompt_text 中约 30-40% 是 harness 注入的系统提醒、命令脚手架
//! 和 Skill 基座文本（真机实测 2026-09-07：19,053 条中 5,432 条为 TodoWrite 提醒、
//! 2,141 条带 system-reminder 标签）。这些脏数据会污染重复模式检测、沉淀建议和
//! 周报复盘——飞轮总结的必须是用户的真实输入（docs/product-epics.md E3）。
//!
//! 规则按 Q2 决议硬编码（协议性、稳定）；采集时打标（prompt_kind 列，保留原文
//! 以便证据回溯），检测器查询时过滤，无标记的老数据按规则兜底排除。

/// prompt_kind 列取值。
pub const PROMPT_KIND_USER: &str = "user";
pub const PROMPT_KIND_NON_USER: &str = "non_user";

/// 判定为"非用户输入"的行首特征（trim 后匹配）。协议性注入文本，勿随意增删。
/// v2（P6）：扩充 harness 注入/命令脚手架/连续会话锚点 + SkillMint 自身分析
/// 提示词特征（净化上线前经各 CLI 普通会话泄漏进库的老分析 prompt）。
const NON_USER_PREFIXES: &[&str] = &[
    "<system-reminder>",
    "<local-command-caveat>",
    "<system_notification>",
    "<command-name>",
    "<command-args>",
    // 会话目标 system-reminder 的裸文本变体。
    "Continue working toward the active session goal",
    // Skill 包基座文本（SKILL.md 加载提示）。
    "Base directory for this skill:",
    // P6：harness 注入 / CLI 事件通知。
    "[Request interrupted by user",
    // bash 工具回显（<bash-stdout> 包裹形态，真机榜单 124 次实例）。
    "<bash-stdout>",
    "<bash-input>",
    "<bash-stderr>",
    // 工具调用回显 / Codex 会话历史注入（真机榜单实例）。
    "Called the Read tool",
    "The following is the Codex agent history",
    // Stop hook 通知（真机榜单 159 次实例）。
    "A session-scoped Stop hook is now active",
    "[SYSTEM NOTIFICATION",
    "(Bash completed with no output)",
    "DO NOT respond",
    "The following task has finished",
    "This session is being continued from a previous conversation",
    "Continue from where you left off",
    // P6：slash 命令脚手架（CLI 把 /cmd 展开成 "/cmd cmd <…>" 形态）。
    "/model ",
    "/compact ",
    "/exit ",
    // P6：SkillMint 自身分析提示词（老会话泄漏；新调用已带 [skillmint-analysis] 哨兵）。
    "[skillmint-analysis]",
    "你是一位人机协作语义分析师",
    "你是一位专业的工程效率分析师",
    "你是一位工程效率分析师",
    "你是一位工程经验沉淀助手",
    // P6：用户环境中其它工具的分析提示词（真机实例 2026-09-07）。
    "你是一个知识图谱抽取助手",
    "# Claude Command:",
];

/// 判定为"非用户输入"的包含特征（任意位置出现即算，用于被截断/包裹的变体）。
/// TodoWrite 提醒的真实文本为 "The TodoWrite tool hasn't been used recently..."
///（撇号有 ' ' ' 多种写法且实测出现过，故只锚定稳定前缀片段 "The TodoWrite tool hasn"）。
const NON_USER_CONTAINS: &[&str] = &[
    "<system-reminder>",
    "<local-command-caveat>",
    "<system_notification>",
    "<command-name>",
    "<bash-stdout>",
    "<bash-input>",
    "The TodoWrite tool hasn",
    "[skillmint-analysis]",
];

/// 图片占位符（多模态消息在文本里的展开形式）。
const IMAGE_PLACEHOLDER_PREFIX: &str = "[Image:";

/// 该 prompt 是否为非用户输入（系统注入 / Skill 脚手架 / 占位符）。
/// 空文本视为非输入。
pub fn is_non_user_prompt(prompt_text: &str) -> bool {
    let t = prompt_text.trim();
    if t.is_empty() {
        return true;
    }
    if t.starts_with(IMAGE_PLACEHOLDER_PREFIX) {
        return true;
    }
    // P6/v4 机制级泛化：以 XML/HTML 标签形态开头（<system-reminder>、<bash-stdout>、
    // <in-app-browser-context>… 整个家族都是 harness 注入）。用户真实输入极少以
    // 标签开头（粘贴 HTML 片段的场景会被误标——可接受的保守取舍，见 Q2 决议）。
    if starts_with_xml_tag(t) {
        return true;
    }
    if NON_USER_PREFIXES.iter().any(|p| t.starts_with(p)) {
        return true;
    }
    NON_USER_CONTAINS.iter().any(|p| t.contains(p))
}

/// 是否以 XML/HTML 标签形态开头："<" + 首字母 + 合法标签字符（字母/数字/-/_/:）+
/// 空白或 ">"。如 "<system-reminder>"、"<bash-input>pwd"。
fn starts_with_xml_tag(t: &str) -> bool {
    let mut chars = t.chars();
    if chars.next() != Some('<') {
        return false;
    }
    let mut seen_alpha = false;
    for c in chars {
        match c {
            c if c.is_ascii_alphabetic() => seen_alpha = true,
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':' => {}
            '>' | ' ' | '\t' | '\n' | '\r' => return seen_alpha,
            _ => return false,
        }
    }
    false
}

/// 该 prompt 是否为用户真实输入。
pub fn is_user_prompt(prompt_text: &str) -> bool {
    !is_non_user_prompt(prompt_text)
}

/// 检测器行级过滤：优先信任入库时的标记；老数据（kind 为空）按规则兜底。
pub fn is_user_row(kind: &str, prompt_text: &str) -> bool {
    if kind == PROMPT_KIND_USER {
        return true;
    }
    if kind == PROMPT_KIND_NON_USER {
        return false;
    }
    is_user_prompt(prompt_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_system_reminders_as_non_user() {
        assert!(is_non_user_prompt(
            "<system-reminder>\nThe TodoWrite tool hasn't been used recently...\n</system-reminder>"
        ));
        assert!(is_non_user_prompt(
            "The TodoWrite tool hasn't been recently. If you're working on tasks..."
        ));
        assert!(is_non_user_prompt(
            "Continue working toward the active session goal. The objective below..."
        ));
        assert!(is_non_user_prompt(
            "some prefix\n<system-reminder>wrapped</system-reminder>"
        ));
    }

    #[test]
    fn classifies_skill_and_caveat_as_non_user() {
        assert!(is_non_user_prompt(
            "Base directory for this skill: /Users/x/.claude/skills/foo"
        ));
        assert!(is_non_user_prompt(
            "<local-command-caveat>Caveat: The messages below were generated"
        ));
    }

    #[test]
    fn classifies_placeholders_and_empty_as_non_user() {
        assert!(is_non_user_prompt(
            "[Image: original 1320x2856, displayed at 924x2000]"
        ));
        assert!(is_non_user_prompt("   \n  "));
    }

    #[test]
    fn keeps_real_user_input() {
        assert!(is_user_prompt("重构认证模块，注意向后兼容"));
        assert!(is_user_prompt("fix the nav regression on the settings page"));
        // 用户引用了系统提醒字样但不是以它开头的注入 —— 保留为用户输入。
        assert!(is_user_prompt("how do I suppress the system-reminder noise?"));
        // 零价值但真实的输入仍是 user（由高价值路径的最小信息量门槛挡，不撒谎）。
        assert!(is_user_prompt("hello"));
        assert!(is_user_prompt("你好"));
    }

    #[test]
    fn v2_rules_covers_harness_noise() {
        // P6 扩充：CLI 事件/中断/工具回显类注入。
        assert!(is_non_user_prompt("[Request interrupted by user]"));
        assert!(is_non_user_prompt("(Bash completed with no output)"));
        assert!(is_non_user_prompt(
            "<bash-stdout>(Bash completed with no output)</bash-stdout>"
        ));
        assert!(is_non_user_prompt("<bash-input>pwd</bash-input>"));
        assert!(is_non_user_prompt("[Request interrupted by user for tool use]"));
        assert!(is_non_user_prompt(
            "Called the Read tool with the following input: {\"file_path\": ...}"
        ));
        assert!(is_non_user_prompt(
            "The following is the Codex agent history added since your last turn"
        ));
        assert!(is_non_user_prompt(
            "A session-scoped Stop hook is now active with conditions"
        ));
        // v4 机制级泛化：任意标签形态开头。
        assert!(is_non_user_prompt(
            "<in-app-browser-context source=\"ambient-ui-state\">…"
        ));
        assert!(!is_non_user_prompt("写一个 <div> 居中的组件"));  // 文字开头不受影响
        assert!(is_non_user_prompt(
            "[SYSTEM NOTIFICATION - NOT USER INPUT] This is an automated background-task event"
        ));
        assert!(is_non_user_prompt(
            "<system_notification>\nThe following task has finished..."
        ));
        assert!(is_non_user_prompt("DO NOT respond"));
        assert!(is_non_user_prompt(
            "The following task has finished. If you were already aware, ignore this notification"
        ));
        assert!(is_non_user_prompt(
            "This session is being continued from a previous conversation that ran out of context."
        ));
        assert!(is_non_user_prompt("Continue from where you left off."));
        // slash 命令脚手架。
        assert!(is_non_user_prompt("/model model </command>"));
        assert!(is_non_user_prompt("/compact compact <"));
        assert!(is_non_user_prompt("/exit exit </command>"));
        // 命令/工具 XML 标签（contains 命中）。
        assert!(is_non_user_prompt("<command-name>/model</command-name>"));
        // SkillMint 自身分析提示词（老会话泄漏 + 新哨兵）。
        assert!(is_non_user_prompt(
            "你是一位人机协作语义分析师。请对下面每一条 User Prompt 打标签"
        ));
        assert!(is_non_user_prompt(
            "[skillmint-analysis]\n你是一位专业的工程效率分析师……"
        ));
        assert!(is_non_user_prompt(
            "你是一个知识图谱抽取助手。请阅读下面的 SKILL.md，抽取其中的知识图谱结构。"
        ));
        assert!(is_non_user_prompt(
            "# Claude Command: Commit (Git-only) 该命令在不依赖任何包管理器的前提下"
        ));
    }
}
