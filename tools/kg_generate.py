#!/usr/bin/env python3
"""
kg_generate.py — 用 kimi CLI（LLM）从 SKILL.md 抽取知识图谱数据并写入 SkillMint 的 SQLite。

决策：方案 A（DECISIONS-kimi-kg-generation.md）。
- 独立脚本，不改 Rust 后端；app 的 get_knowledge_graph / KnowledgeGraph.tsx 无需改动。
- 流程：遍历 center_repo/*/SKILL.md → kimi -p 抽取 {nodes,edges} → 解析 JSON → 清旧 KG → upsert 新数据。
- node id 哈希与 Rust make_node 一致：sha256(label+":"+type) 前 8 字节 hex。

用法：
    python3 tools/kg_generate.py                 # 默认数据库 + center_repo
    python3 tools/kg_generate.py --dry-run       # 只抽取不写库，打印统计
    python3 tools/kg_generate.py --skill lark-mail  # 只处理单个 skill
    python3 tools/kg_generate.py --limit 5       # 只处理前 5 个（调试）
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

# --- 配置 -------------------------------------------------------------------

DEFAULT_DB = Path.home() / "Library/Application Support/com.skillmint/skillmint.db"
DEFAULT_REPO = Path.home() / ".skillmint/repo"
KIMI_BIN = shutil.which("kimi") or os.path.expandvars("$HOME/.kimi-code/bin/kimi")
KIMI_TIMEOUT = 120  # 单个 skill 抽取超时（秒）

# 节点类型白名单（与 kg_nodes CHECK 约束一致）
VALID_TYPES = {"concept", "scenario", "practice", "action", "object"}
# 关系类型白名单（语义化；kg_edges.relation 是自由 TEXT，但前端/推荐逻辑认这些）
VALID_RELATIONS = {"similar", "related", "generalizes", "depends_on", "composes", "conflicts_with"}

EXTRACT_PROMPT = """你是一个知识图谱抽取助手。请阅读下面的 SKILL.md（一个 AI Agent 技能说明文档），抽取其中的知识图谱结构。

要求：
1. 抽取「概念节点」(concept)：文档中定义的核心概念/术语/对象。每个节点需要 label（中英文均可，简短）、type、description（一句话）。
   type 只能是这五个之一：concept（抽象概念）、scenario（使用场景）、practice（实践/做法）、object（实体对象）、action（操作/动作）。
2. 抽取「关系」(edges)：概念之间存在的关系。from/to 用节点的 label 引用（必须是你上面抽出的某个节点 label）。
   relation 只能是：similar（相似）、related（相关）、generalizes（泛化/包含）、depends_on（依赖）、composes（组成）、conflicts_with（冲突）。
3. 只抽取文档中真正出现/强暗示的概念，不要编造。每个 skill 抽取 3-15 个节点为宜。
4. 严格只返回一个 JSON 对象，不要任何解释、前后缀。格式：
{"nodes":[{"label":"...","type":"concept","description":"..."}],"edges":[{"from":"...","to":"...","relation":"similar","weight":0.8,"reason":"..."}]}

SKILL.md 内容：
---
{content}
---"""


# --- 工具函数 ---------------------------------------------------------------

def make_node_id(label: str, node_type: str) -> str:
    """与 Rust make_node 一致：sha256(label + ':' + type) 前 8 字节 hex。"""
    h = hashlib.sha256(f"{label}:{node_type}".encode()).digest()
    return h[:8].hex()


def now_secs() -> int:
    return int(time.time())


def run_kimi_extract(content: str, skill_name: str) -> dict | None:
    """调用 kimi -p 抽取一个 SKILL.md 的图谱，返回 {nodes, edges} 或 None。"""
    # 用 replace 而非 str.format，避免 prompt 里的 JSON 花括号被误解析。
    prompt = EXTRACT_PROMPT.replace("{content}", content[:6000])
    try:
        result = subprocess.run(
            [KIMI_BIN, "-p", prompt, "--output-format", "text"],
            capture_output=True, text=True, timeout=KIMI_TIMEOUT,
        )
    except subprocess.TimeoutExpired:
        print(f"  ⚠ {skill_name}: kimi 超时（{KIMI_TIMEOUT}s），跳过", file=sys.stderr)
        return None
    except Exception as e:
        print(f"  ⚠ {skill_name}: kimi 调用失败 {e}", file=sys.stderr)
        return None

    if result.returncode != 0:
        print(f"  ⚠ {skill_name}: kimi 退出码 {result.returncode}: {result.stderr[:120]}", file=sys.stderr)
        return None

    return parse_kimi_json(result.stdout)


def parse_kimi_json(raw: str) -> dict | None:
    """从 kimi 输出中解析 JSON。kimi 可能把 JSON 包在 ```json 围栏或对话文本里。"""
    # 1. 先尝试找 ```json ... ``` 代码块
    m = re.search(r"```(?:json)?\s*(\{.*?\})\s*```", raw, re.DOTALL)
    if m:
        candidate = m.group(1)
    else:
        # 2. 找第一个 { 到最后一个 } 的片段
        start = raw.find("{")
        end = raw.rfind("}")
        if start == -1 or end == -1 or end <= start:
            return None
        candidate = raw[start:end + 1]
    try:
        data = json.loads(candidate)
    except json.JSONDecodeError:
        return None
    if not isinstance(data, dict) or "nodes" not in data:
        return None
    return data


# --- 数据库写入 -------------------------------------------------------------

def clear_kg(conn: sqlite3.Connection, device_id: str) -> None:
    """清空当前设备的 KG 数据（重新生成前）。保留表结构。"""
    conn.execute("DELETE FROM kg_skill_nodes WHERE device_id = ?", (device_id,))
    conn.execute("DELETE FROM kg_edges WHERE device_id = ?", (device_id,))
    # kg_nodes 是全局的（id=hash），但重生成时清掉避免孤儿。linked 通过 kg_skill_nodes 管理。
    conn.execute("DELETE FROM kg_nodes")
    conn.commit()


def upsert_kg_data(
    conn: sqlite3.Connection,
    device_id: str,
    skill_id: str,
    skill_name: str,
    extracted: dict,
) -> tuple[int, int]:
    """把一个 skill 的抽取结果写入库。返回 (nodes_written, edges_written)。"""
    raw_nodes = extracted.get("nodes", []) or []
    raw_edges = extracted.get("edges", []) or []

    # 建立 label -> node_id 映射（用于 edge 解析）
    label_to_id: dict[str, str] = {}
    nodes_written = 0
    ts = now_secs()

    for n in raw_nodes:
        if not isinstance(n, dict):
            continue
        label = str(n.get("label", "")).strip()
        ntype = str(n.get("type", "concept")).strip()
        if not label or ntype not in VALID_TYPES:
            continue
        # 去重 label（同一 skill 内同名节点只保留第一个）
        if label in label_to_id:
            continue
        node_id = make_node_id(label, ntype)
        label_to_id[label] = node_id
        desc = str(n.get("description", "")).strip() or None
        conn.execute(
            "INSERT OR REPLACE INTO kg_nodes (id, label, type, source, description, created_at, updated_at) "
            "VALUES (?,?,?,?,?,?,?)",
            (node_id, label, ntype, skill_name, desc, ts, ts),
        )
        # 链接 skill → node
        conn.execute(
            "INSERT OR REPLACE INTO kg_skill_nodes (device_id, skill_id, node_id, relevance, created_at) "
            "VALUES (?,?,?,?,?)",
            (device_id, skill_id, node_id, 1.0, ts),
        )
        nodes_written += 1

    edges_written = 0
    for e in raw_edges:
        if not isinstance(e, dict):
            continue
        frm = str(e.get("from", "")).strip()
        to = str(e.get("to", "")).strip()
        rel = str(e.get("relation", "related")).strip()
        if rel not in VALID_RELATIONS:
            rel = "related"
        # from/to 必须是已抽取的节点 label
        src_id = label_to_id.get(frm)
        tgt_id = label_to_id.get(to)
        if not src_id or not tgt_id or src_id == tgt_id:
            continue
        weight = float(e.get("weight", 0.8) or 0.8)
        weight = max(0.0, min(1.0, weight))
        reason = str(e.get("reason", "")).strip() or None
        edge_id = f"{device_id}:{src_id}:{tgt_id}:{rel}"
        conn.execute(
            "INSERT OR REPLACE INTO kg_edges "
            "(id, device_id, source_id, target_id, relation, weight, reason, is_manual, is_rejected, created_at, updated_at) "
            "VALUES (?,?,?,?,?,?,?,?,?,?,?)",
            (edge_id, device_id, src_id, tgt_id, rel, weight, reason, 1, 0, ts, ts),
        )
        edges_written += 1

    return nodes_written, edges_written


# --- 主流程 -----------------------------------------------------------------

def main() -> int:
    ap = argparse.ArgumentParser(description="用 kimi CLI 生成 SkillMint 知识图谱数据")
    ap.add_argument("--db", default=str(DEFAULT_DB), help="SQLite 数据库路径")
    ap.add_argument("--repo", default=str(DEFAULT_REPO), help="center_repo 路径")
    ap.add_argument("--device-id", default=None, help="设备 id（默认从库自动取）")
    ap.add_argument("--skill", default=None, help="只处理指定 skill 名（调试）")
    ap.add_argument("--limit", type=int, default=0, help="只处理前 N 个（0=全部）")
    ap.add_argument("--dry-run", action="store_true", help="只抽取不写库")
    ap.add_argument("--no-clear", action="store_true", help="不先清空旧 KG（追加）")
    args = ap.parse_args()

    db_path = Path(args.db).expanduser()
    repo_path = Path(args.repo).expanduser()
    if not db_path.exists():
        print(f"错误：数据库不存在 {db_path}", file=sys.stderr)
        return 1
    if not repo_path.is_dir():
        print(f"错误：center_repo 不存在 {repo_path}", file=sys.stderr)
        return 1
    if not Path(KIMI_BIN).exists():
        print(f"错误：kimi 未安装 {KIMI_BIN}", file=sys.stderr)
        return 1

    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row

    # 取 device_id
    device_id = args.device_id
    if not device_id:
        row = conn.execute("SELECT DISTINCT device_id FROM skills LIMIT 1").fetchone()
        if not row:
            print("错误：库中无 device_id（skills 表为空）", file=sys.stderr)
            return 1
        device_id = row["device_id"]
    print(f"device_id = {device_id}")

    # 取 skill 列表（id, name）
    if args.skill:
        skills = conn.execute(
            "SELECT id, name FROM skills WHERE name = ?", (args.skill,)
        ).fetchall()
    else:
        skills = conn.execute("SELECT id, name FROM skills ORDER BY name").fetchall()

    if args.limit > 0:
        skills = skills[: args.limit]

    print(f"待处理 skill 数：{len(skills)}")
    if not skills:
        print("无 skill，退出。")
        return 0

    if not args.dry_run and not args.no_clear:
        print("清空旧 KG 数据...")
        clear_kg(conn, device_id)

    total_nodes = total_edges = ok = failed = 0
    for i, sk in enumerate(skills, 1):
        sid, sname = sk["id"], sk["name"]
        md = repo_path / sname / "SKILL.md"
        if not md.exists():
            print(f"[{i}/{len(skills)}] {sname}: 跳过（无 SKILL.md）")
            continue
        content = md.read_text(encoding="utf-8", errors="replace")
        print(f"[{i}/{len(skills)}] {sname}: 抽取中...", end="", flush=True)
        t0 = time.time()
        extracted = run_kimi_extract(content, sname)
        dt = time.time() - t0
        if extracted is None:
            print(f" 失败（{dt:.1f}s）")
            failed += 1
            continue
        nn = len(extracted.get("nodes", []) or [])
        ne = len(extracted.get("edges", []) or [])
        if args.dry_run:
            print(f" dry-run: {nn} 节点, {ne} 边（{dt:.1f}s）")
            total_nodes += nn
            total_edges += ne
        else:
            wn, we = upsert_kg_data(conn, device_id, sid, sname, extracted)
            conn.commit()
            print(f" 写入 {wn} 节点, {we} 边（{dt:.1f}s）")
            total_nodes += wn
            total_edges += we
        ok += 1

    conn.close()
    print(f"\n完成：成功 {ok}/{len(skills)}，失败 {failed}")
    print(f"总计：{total_nodes} 节点, {total_edges} 边")
    if args.dry_run:
        print("（dry-run 模式，未写入数据库）")
    else:
        print("已写入数据库。打开 SkillMint 知识图谱页查看。")
    return 0 if failed == 0 else 2


if __name__ == "__main__":
    sys.exit(main())
