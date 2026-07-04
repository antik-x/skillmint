export interface HumanizedError {
  /** Short user-facing Chinese message. */
  message: string;
  /** Original technical detail for debugging; may be empty. */
  detail: string;
  /** Suggested recovery action; optional. */
  action?: string;
}

function extractRaw(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

const RULES: { pattern: RegExp; message: string; action?: string }[] = [
  {
    pattern: /404.*unknown command|unknown command|command not found/i,
    message: "此功能在当前环境不可用。",
    action: "检查更新或稍后再试",
  },
  {
    pattern: /failed to fetch|networkerror|network error|fetch failed|无法连接|连接被拒绝|connection refused/i,
    message: "无法连接后端服务，请重试。",
    action: "确认 mock_server 或 Tauri 应用正在运行",
  },
  {
    pattern: /permission denied|access is denied|eacces/i,
    message: "没有权限访问该路径或文件。",
    action: "检查目录权限",
  },
  {
    pattern: /no such file|file not found|enoent/i,
    message: "找不到指定的文件或目录。",
    action: "检查路径是否正确",
  },
  {
    pattern: /timeout|timed out/i,
    message: "无法连接后端服务，请重试。",
    action: "稍后重试",
  },
  {
    pattern: /invalid|parse error|unexpected token/i,
    message: "数据格式不正确，无法解析。",
    action: "检查输入或联系支持",
  },
  // SPEC-F6 T3: 高频核心路径的恢复路径补全。
  // 同步失败：写 Agent 目录时出错（权限/链接/目录不存在）。
  {
    pattern: /sync.*(fail|error)|symlink|create.*link|write.*(agent|directory|target)/i,
    message: "同步到 Agent 目录失败。",
    action: "检查目录权限，或在 Skill 详情重试同步",
  },
  // 采集源被占用/锁定（如 Claude Code 正在写入）。
  {
    pattern: /(locked|being used|in use|busy|resource busy|另一进程|占用)/i,
    message: "数据源暂时无法读取。",
    action: "Agent 正在使用中，稍后会自动重试",
  },
  // Skill 重名冲突。
  {
    pattern: /(already exists|duplicate|name.*taken|重名|已存在)/i,
    message: "存在同名 Skill。",
    action: "换一个名称，或先删除/恢复同名 Skill",
  },
  // 导入空目录 / 缺 SKILL.md。
  {
    pattern: /(no skill|empty|skill\.md.*not found|未发现.*SKILL|没有.*规则文件)/i,
    message: "所选目录下没有发现可导入的 Skill。",
    action: "确认所选 Agent 目录下有规则文件",
  },
];

/**
 * Convert an unknown error into a user-friendly Chinese message while
 * preserving the original technical detail for debugging.
 */
export function humanizeError(
  err: unknown,
  context?: { context?: string; action?: string }
): HumanizedError {
  const raw = extractRaw(err);
  const prefix = context?.context ? `${context.context}失败：` : "";

  for (const rule of RULES) {
    if (rule.pattern.test(raw)) {
      return {
        message: `${prefix}${rule.message}`,
        detail: raw,
        action: context?.action ?? rule.action,
      };
    }
  }

  return {
    message: `${prefix}操作失败，请重试。`,
    detail: raw,
    action: context?.action,
  };
}
