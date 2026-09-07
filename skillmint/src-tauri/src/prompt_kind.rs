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
const NON_USER_PREFIXES: &[&str] = &[
    "<system-reminder>",
    "<local-command-caveat>",
    // 会话目标 system-reminder 的裸文本变体。
    "Continue working toward the active session goal",
    // Skill 包基座文本（SKILL.md 加载提示）。
    "Base directory for this skill:",
];

/// 判定为"非用户输入"的包含特征（任意位置出现即算，用于被截断/包裹的变体）。
/// TodoWrite 提醒的真实文本为 "The TodoWrite tool hasn't been used recently..."
///（撇号有 ' ' ' 多种写法且实测出现过，故只锚定稳定前缀片段 "The TodoWrite tool hasn"）。
const NON_USER_CONTAINS: &[&str] = &[
    "<system-reminder>",
    "<local-command-caveat>",
    "The TodoWrite tool hasn",
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
    if NON_USER_PREFIXES.iter().any(|p| t.starts_with(p)) {
        return true;
    }
    NON_USER_CONTAINS.iter().any(|p| t.contains(p))
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
    }
}
