export type EmptyIllustrationKind =
  | "library"
  | "graph"
  | "inbox"
  | "insights"
  | "generic";

/**
 * 空状态插画集：与 DESIGN-SYSTEM.md 对齐的轻量 SVG 图形。
 * 全部使用 CSS token 取色，自动适配明暗主题；amber 保留给「经验/沉淀」语义。
 */
export function EmptyIllustration({
  kind,
  className,
}: {
  kind: EmptyIllustrationKind;
  className?: string;
}) {
  const stroke = "var(--border-prominent)";
  const fill = "var(--bg-tertiary)";
  const accent = "var(--accent)";
  const amber = "var(--amber)";

  const common = {
    width: 140,
    height: 96,
    viewBox: "0 0 140 96",
    fill: "none",
    className,
    "aria-hidden": true as const,
  };

  switch (kind) {
    case "library":
      // 三张层叠的 Skill 卡片，最上层带 accent 标题条
      return (
        <svg {...common}>
          <rect x="30" y="30" width="64" height="44" rx="6" fill={fill} stroke={stroke} transform="rotate(-6 62 52)" />
          <rect x="40" y="26" width="64" height="44" rx="6" fill={fill} stroke={stroke} transform="rotate(3 72 48)" />
          <rect x="44" y="24" width="64" height="44" rx="6" fill="var(--bg-elevated)" stroke={stroke} />
          <rect x="52" y="32" width="28" height="5" rx="2.5" fill={accent} opacity="0.85" />
          <rect x="52" y="43" width="46" height="3.5" rx="1.75" fill={stroke} />
          <rect x="52" y="51" width="38" height="3.5" rx="1.75" fill={stroke} />
        </svg>
      );
    case "graph":
      // 待连接的知识节点
      return (
        <svg {...common}>
          <path d="M42 60 L70 34 L100 52 M70 34 L70 72" stroke={stroke} strokeWidth="1.5" strokeDasharray="4 4" />
          <circle cx="42" cy="60" r="9" fill={fill} stroke={stroke} />
          <circle cx="100" cy="52" r="9" fill={fill} stroke={stroke} />
          <circle cx="70" cy="72" r="7" fill={fill} stroke={stroke} />
          <circle cx="70" cy="34" r="11" fill="var(--bg-elevated)" stroke={accent} strokeWidth="1.5" />
          <circle cx="70" cy="34" r="4" fill={accent} opacity="0.85" />
        </svg>
      );
    case "inbox":
      // 收件托盘 + 即将落下的经验之星（amber）
      return (
        <svg {...common}>
          <path
            d="M34 56 h20 l6 8 h20 l6 -8 h20 v22 a6 6 0 0 1 -6 6 h-60 a6 6 0 0 1 -6 -6 z"
            fill={fill}
            stroke={stroke}
          />
          <path d="M34 56 L44 36 h52 l10 20" stroke={stroke} strokeWidth="1.5" fill="none" />
          <path
            d="M70 14 l2.6 6.2 6.4 0.8 -4.8 4.4 1.3 6.4 -5.5 -3.4 -5.5 3.4 1.3 -6.4 -4.8 -4.4 6.4 -0.8 z"
            fill={amber}
            opacity="0.9"
          />
          <circle cx="94" cy="26" r="2" fill={amber} opacity="0.5" />
          <circle cx="48" cy="24" r="1.5" fill={amber} opacity="0.4" />
        </svg>
      );
    case "insights":
      // 尚无数据的统计图
      return (
        <svg {...common}>
          <path d="M36 76 h68" stroke={stroke} strokeWidth="1.5" />
          <rect x="44" y="58" width="10" height="18" rx="2" fill={fill} stroke={stroke} />
          <rect x="62" y="46" width="10" height="30" rx="2" fill={fill} stroke={stroke} />
          <rect x="80" y="52" width="10" height="24" rx="2" fill={fill} stroke={stroke} />
          <path d="M42 40 C 58 26, 78 34, 100 22" stroke={accent} strokeWidth="1.5" strokeDasharray="4 4" fill="none" />
          <circle cx="100" cy="22" r="3" fill={accent} />
        </svg>
      );
    case "generic":
    default:
      // 虚线圆 + 中心加号：等待第一份内容
      return (
        <svg {...common}>
          <circle cx="70" cy="48" r="26" fill={fill} stroke={stroke} strokeDasharray="5 5" />
          <path d="M70 38 v20 M60 48 h20" stroke={accent} strokeWidth="2" strokeLinecap="round" opacity="0.85" />
          <circle cx="104" cy="28" r="2" fill={stroke} />
          <circle cx="38" cy="68" r="2" fill={stroke} />
        </svg>
      );
  }
}
