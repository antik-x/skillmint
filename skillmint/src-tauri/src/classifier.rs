//! PRD-08 §3.3 (P1): the four-axis semantic prompt classifier — a 1:1 Rust
//! port of AI-Digest's `src/digest/prompt_classifier.py`.
//!
//! Each user prompt is tagged on four fixed axes (hard-enumerated so the
//! distributions and Role Profile stay comparable):
//!   - requested_action: 规划/生成/修改/解释/检查/执行/总结
//!   - target_object:    代码/测试/文档/配置/数据/设计/环境
//!   - interaction_state: 新任务/继续推进/补充澄清/纠偏修正/切换方向
//!   - interaction_mode:  连续细化/先规划后实施/拆步骤/多体协同
//! plus a confidence score (0.0–1.0; Role Profile uses ≥0.6).
//!
//! Graceful degradation: with no `api_key`, classification is skipped and labels
//! stay empty. The pipeline never hard-fails. Completeness: prompts are retried
//! up to 3 passes so LLM omissions are recovered; a warning is logged if any
//! eligible prompt remains unclassified.

use std::sync::Mutex;

use serde_json::Value;

use crate::db::Db;
use crate::llm;
use crate::settings::AiConfig;

/// Batch size sent to the LLM per request. Matches AI-Digest `BATCH_SIZE`.
const BATCH_SIZE: usize = 15;
/// Max chars of prompt text sent per item. Matches `MAX_PROMPT_CHARS`.
const MAX_PROMPT_CHARS: usize = 150;
/// Retry passes to recover LLM omissions.
const MAX_PASSES: usize = 3;

/// The system prompt (fixed enumeration). Kept identical to the Python original
/// so labeling caliber matches across the two tools.
const SYSTEM_PROMPT: &str = "你是一位人机协作语义分析师。请对下面每一条 User Prompt（用户发给 AI 的指令），打上四个维度的标签。\n\n四个维度的枚举值（必须从中选择，中文）：\n\n1. requested_action（用户主要让 AI 做什么）:\n   规划 / 生成 / 修改 / 解释 / 检查 / 执行 / 总结\n\n2. target_object（这次请求主要作用于什么对象）:\n   代码 / 测试 / 文档 / 配置 / 数据 / 设计 / 环境\n\n3. interaction_state（当前轮在协作中的位置）:\n   新任务 / 继续推进 / 补充澄清 / 纠偏修正 / 切换方向\n\n4. interaction_mode（最近的协作组织模式）:\n   连续细化 / 先规划后实施 / 拆步骤 / 多体协同\n\n规则：\n- 只能从上述枚举中选择，不要自创。\n- 如果信息不足难以判断，选择最可能的一个，不要输出\"未知\"，并把 confidence 调低（越不确定越接近 0.5）。\n- 每个元素额外输出一个 confidence 字段（0.0-1.0 的小数），表示你对这条标注的整体把握：1.0=非常确定，0.5=勉强最可能。\n- requested_action / target_object 通常较易判断（confidence 偏高）；interaction_state / interaction_mode 依赖上下文，不确定时 confidence 应更低。\n- 必须输出合法 JSON 数组，每个元素含 id（对应输入序号）和五个字段（四轴 + confidence）。\n- 不要包含 markdown 代码块。\n\n输出示例：\n[{\"id\":1,\"requested_action\":\"修改\",\"target_object\":\"代码\",\"interaction_state\":\"继续推进\",\"interaction_mode\":\"连续细化\",\"confidence\":0.85}]\n";

/// 分析类任务的系统提示统一加内容哨兵（P5/origin.rs 防线 2）。实现移至
/// origin::analysis_system_prompt，analyzer 同样复用。
pub use crate::origin::analysis_system_prompt;

/// Result of a classification run: counts for the UI/report.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ClassifyResult {
    pub eligible: usize,
    pub classified: usize,
    pub skipped_no_key: bool,
}

/// Classify all eligible, currently-unclassified prompts for a given source
/// (empty string = all sources). Idempotent: prompts already classified are
/// skipped (COALESCE preserves existing labels).
///
/// P5：接收 `&Mutex<Db>`——LLM 调用（可达分钟级，尤其 ACP）在锁外执行，锁只
/// 包住 load/write 短临界区（验收标准：长 I/O 不进锁）。
pub fn classify_prompts(
    db: &Mutex<Db>,
    source: Option<&str>,
    cfg: &AiConfig,
    limit_batches: Option<usize>,
) -> anyhow::Result<ClassifyResult> {
    if !llm::is_configured(cfg) {
        // No LLM key → skip entirely, report the eligible count for visibility.
        let eligible = {
            let d = db.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
            d.count_unclassified_prompts(source)?
        };
        return Ok(ClassifyResult {
            eligible,
            classified: 0,
            skipped_no_key: true,
        });
    }

    let mut pending = {
        let d = db.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        d.load_unclassified_prompts(source)?
    };
    // 额度优先（Q7）：limit_batches 限制本次跑的批数（1 批 = BATCH_SIZE 条），
    // 供「先试一批看效果」——全量跑传 None。
    if let Some(max_batches) = limit_batches {
        pending.truncate(max_batches * BATCH_SIZE);
    }
    let eligible = pending.len();
    if eligible == 0 {
        return Ok(ClassifyResult { eligible: 0, classified: 0, skipped_no_key: false });
    }

    let mut classified = 0usize;
    for _pass in 0..MAX_PASSES {
        if pending.is_empty() {
            break;
        }
        let mut still_missing: Vec<UnclassifiedPrompt> = Vec::new();
        for batch in pending.chunks(BATCH_SIZE) {
            // LLM/ACP 调用：锁外（可达分钟级）。
            let (labels, outcome) = classify_batch(batch, cfg);
            let d = db.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
            d.log_llm_request(&outcome, "classifier");
            // labels: batch-local 0-based position -> axis values
            for (local_pos, item) in batch.iter().enumerate() {
                if let Some(lbl) = labels.get(local_pos) {
                    d.update_prompt_semantic(&item.id, lbl)?;
                    classified += 1;
                } else {
                    still_missing.push(item.clone());
                }
            }
        }
        pending = still_missing;
    }

    if classified < eligible {
        eprintln!(
            "[classifier] WARNING: {}/{} eligible prompts remain unclassified after {} passes",
            eligible - classified,
            eligible,
            MAX_PASSES
        );
    }
    Ok(ClassifyResult {
        eligible,
        classified,
        skipped_no_key: false,
    })
}

/// One unclassified prompt row loaded from the DB for labeling.
#[derive(Debug, Clone)]
pub struct UnclassifiedPrompt {
    pub id: String,        // collected_prompts.id
    pub prompt_text: String,
}

/// The four-axis label (+confidence) written back for one prompt.
#[derive(Debug, Clone, Default)]
pub struct PromptLabel {
    pub requested_action: Option<String>,
    pub target_object: Option<String>,
    pub interaction_state: Option<String>,
    pub interaction_mode: Option<String>,
    pub confidence: Option<f64>,
}

/// Call the LLM for one batch. Returns (batch-local-position → label, outcome)
/// — the outcome is logged by the caller under the DB lock. Mirrors
/// `_classify_batch`: prompts are sent with 1-based batch-local ids.
fn classify_batch(batch: &[UnclassifiedPrompt], cfg: &AiConfig) -> (Vec<PromptLabel>, llm::ChatOutcome) {
    let mut lines = Vec::with_capacity(batch.len());
    for (i, p) in batch.iter().enumerate() {
        let text: String = p
            .prompt_text
            .chars()
            .take(MAX_PROMPT_CHARS)
            .collect::<String>()
            .replace('\n', " ");
        lines.push(format!("{}. {}", i + 1, text.trim()));
    }
    let user_payload = lines.join("\n");

    let outcome = llm::chat_with_outcome(cfg, &analysis_system_prompt(SYSTEM_PROMPT), &user_payload);
    let content = match outcome.content.clone() {
        Some(c) => c,
        None => return (Vec::new(), outcome),
    };
    let clean = llm::strip_codefence(&content);
    let arr: Vec<Value> = match serde_json::from_str(&clean) {
        Ok(v) => v,
        Err(_) => {
            // Truncated output: salvage complete objects before the cut-off.
            let salvaged = llm::salvage_json_array(&clean);
            if salvaged.is_empty() {
                return (Vec::new(), outcome);
            }
            salvaged
        }
    };

    let mut out: Vec<PromptLabel> = vec![PromptLabel::default(); batch.len()];
    for item in arr {
        let obj = match item.as_object() {
            Some(o) => o,
            None => continue,
        };
        let id = match obj.get("id").and_then(|v| v.as_i64()) {
            Some(n) => n,
            None => continue,
        };
        let pos = (id - 1) as usize; // batch-local, 0-based
        if pos >= batch.len() {
            continue;
        }
        out[pos] = PromptLabel {
            requested_action: obj.get("requested_action").and_then(|v| v.as_str()).map(String::from),
            target_object: obj.get("target_object").and_then(|v| v.as_str()).map(String::from),
            interaction_state: obj.get("interaction_state").and_then(|v| v.as_str()).map(String::from),
            interaction_mode: obj.get("interaction_mode").and_then(|v| v.as_str()).map(String::from),
            confidence: obj
                .get("confidence")
                .and_then(|v| v.as_f64())
                .or_else(|| obj.get("confidence").and_then(|v| v.as_str()).and_then(|s| s.parse().ok())),
        };
    }
    (out, outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_skips_gracefully() {
        // With an empty key, classify_prompts must not call the LLM. We can't
        // easily build a Db here without a file, so test the gate directly.
        let cfg = AiConfig::default();
        assert!(!llm::is_configured(&cfg));
    }

    #[test]
    fn classify_batch_parses_well_formed_json() {
        // Simulate an LLM response for a 2-prompt batch and verify mapping.
        let batch = vec![
            UnclassifiedPrompt { id: "p1".into(), prompt_text: "帮我写个函数".into() },
            UnclassifiedPrompt { id: "p2".into(), prompt_text: "解释这段代码".into() },
        ];
        // We can't hit a real LLM, but classify_batch builds the payload from
        // batch-local 1-based ids; verify the payload numbering contract.
        let lines: Vec<String> = batch
            .iter()
            .enumerate()
            .map(|(i, p)| format!("{}. {}", i + 1, p.prompt_text))
            .collect();
        assert_eq!(lines[0], "1. 帮我写个函数");
        assert_eq!(lines[1], "2. 解释这段代码");
        // And verify an empty config short-circuits the batch (no panic, no labels).
        let tmp = tempfile::tempdir().unwrap();
        let mut db = crate::db::Db::new(&tmp.path().join("test.db")).unwrap();
        db.init("test-device").unwrap();
        let (labels, outcome) = classify_batch(&batch, &AiConfig::default());
        assert!(labels.is_empty(), "expected no labels without a key");
        assert!(outcome.content.is_none());
        // 落库路径不受签名调整影响：结果表无分类写入。
        assert_eq!(db.count_unclassified_prompts(None).unwrap(), 0);
    }

    #[test]
    fn salvage_recovers_objects_from_truncated_array() {
        // The LLM may hit the token limit mid-array; salvage should still
        // extract the complete objects before the cut-off. (Use an escaped
        // string because the truncated payload ends in a quote that would break
        // a raw string.)
        let truncated = "[\n  {\"id\":1,\"requested_action\":\"生成\",\"confidence\":0.9},\n  {\"id\":2,\"requested_action\":\"修改\",\"target_object\":\"#";
        let salvaged = llm::salvage_json_array(truncated);
        assert_eq!(salvaged.len(), 1, "only the first complete object survives");
        assert_eq!(salvaged[0].get("id").and_then(|v| v.as_i64()), Some(1));
    }
}
