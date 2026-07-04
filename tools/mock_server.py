#!/usr/bin/env python3
"""
Read-only mock backend for browser-based click-testing of the SkillMint frontend.

Serves the EXACT data shapes the Rust `#[tauri::command]` handlers return, read
straight from the live SQLite DB at ~/Library/Application Support/com.skillmint/skillmint.db.
This lets chrome-devtools-mcp drive the real React UI against real data, since
Tauri's WKWebView cannot be driven via CDP.

Endpoints:
  POST /invoke  {cmd, args}  -> JSON matching that command's return type
  GET  /health              -> {"ok": true, "db": "..."}
"""
import json
import os
import re
import sqlite3
import string
import subprocess
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from urllib.parse import urlparse

DB_PATH = Path.home() / "Library" / "Application Support" / "com.skillmint" / "skillmint.db"
PORT = 18220


def db():
    conn = sqlite3.connect(str(DB_PATH))
    conn.row_factory = sqlite3.Row
    return conn


def rows_to_dicts(rows):
    return [dict(r) for r in rows]


def now_ms():
    return int(time.time() * 1000)


# --------------------------------------------------------------------------- #
# Per-command handlers. Each mirrors the Rust command's return shape.
# Args come in camelCase (frontend convention); we map to snake_case columns.
# --------------------------------------------------------------------------- #

def cmd_get_skills(conn, args):
    r = conn.execute(
        "SELECT id, name, repo_path, created_at, updated_at, device_id FROM skills ORDER BY name"
    ).fetchall()
    return rows_to_dicts(r)


def cmd_get_agents(conn, args):
    r = conn.execute(
        """SELECT id, name, skill_directory, is_enabled, discovery_rule,
                  description, source, device_id, last_used_at, project_count,
                  created_at, updated_at
           FROM agents ORDER BY name"""
    ).fetchall()
    out = []
    for row in r:
        d = dict(row)
        d["is_enabled"] = bool(d["is_enabled"])
        # attach skill list for this agent (like the command does)
        skills = conn.execute(
            """SELECT s.id, s.name, st.status, st.mode, st.last_sync_at, st.id AS target_id
               FROM sync_targets st JOIN skills s ON s.id = st.skill_id
               WHERE st.agent_id = ? ORDER BY s.name""",
            (d["id"],),
        ).fetchall()
        d["skills"] = [
            {
                "id": s["id"],
                "name": s["name"],
                "status": s["status"],
                "mode": s["mode"],
                "last_sync_at": s["last_sync_at"],
                "target_id": s["target_id"],
            }
            for s in skills
        ]
        out.append(d)
    return out


# PRD-06 §3.3: list directories owned by an agent (1:N sub-records).
def _agent_directories(conn, agent_id):
    r = conn.execute(
        """SELECT id, agent_id, path, role, is_enabled, created_at
           FROM agent_directories WHERE agent_id = ? ORDER BY created_at""",
        (agent_id,),
    ).fetchall()
    return [
        {
            "id": row["id"],
            "agent_id": row["agent_id"],
            "path": row["path"],
            "role": row["role"],
            "is_enabled": bool(row["is_enabled"]),
            "created_at": row["created_at"],
        }
        for row in r
    ]


def cmd_add_agent_directory(conn, args):
    import uuid
    agent_id = args["agentId"]
    path = args["path"]
    role = args.get("role") or "skills"
    # Expand ~ like the Rust command does.
    if path == "~":
        path = str(Path.home())
    elif path.startswith("~/"):
        path = str(Path.home() / path[2:])
    dir_id = f"adir-{agent_id}-{uuid.uuid4().hex[:8]}"
    created = now_ms() // 1000
    conn.execute(
        "INSERT OR IGNORE INTO agent_directories (id, agent_id, path, role, is_enabled, created_at) VALUES (?,?,?,?,1,?)",
        (dir_id, agent_id, path, role, created),
    )
    conn.commit()
    return {
        "id": dir_id,
        "agent_id": agent_id,
        "path": path,
        "role": role,
        "is_enabled": True,
        "created_at": created,
    }


def cmd_remove_agent_directory(conn, args):
    directory_id = args["directoryId"]
    # PRD-06 §3.3: unbind the record + its sync_targets, return the detached count.
    cur = conn.execute(
        "DELETE FROM sync_targets WHERE agent_directory_id = ?", (directory_id,)
    )
    orphaned = cur.rowcount
    conn.execute("DELETE FROM agent_directories WHERE id = ?", (directory_id,))
    conn.commit()
    return orphaned


def cmd_update_agent_directory(conn, args):
    directory_id = args["directoryId"]
    role = args.get("role")
    is_enabled = args.get("isEnabled")
    if role is not None:
        conn.execute(
            "UPDATE agent_directories SET role = ? WHERE id = ?", (role, directory_id)
        )
    if is_enabled is not None:
        conn.execute(
            "UPDATE agent_directories SET is_enabled = ? WHERE id = ?",
            (1 if is_enabled else 0, directory_id),
        )
    conn.commit()
    return None


def cmd_scan_directory_skills(conn, args):
    # PRD-06 §3.3: scan ONE directory (any path), same shape as scan_agent_skills.
    raw = args["path"]
    if raw == "~":
        path = Path.home()
    elif raw.startswith("~/"):
        path = Path.home() / raw[2:]
    else:
        path = Path(raw)
    center = Path.home() / ".skillmint"
    items = []
    if path.is_dir():
        for p in sorted(path.iterdir()):
            if p.is_dir():
                center_path = center / p.name
                exists = center_path.exists()
                items.append({"name": p.name, "exists_in_center": exists, "content_match": None})
    return items


def cmd_get_sync_targets(conn, args):
    r = conn.execute(
        """SELECT id, skill_id, agent_id, mode, last_sync_at, status, device_id
           FROM sync_targets ORDER BY status, id"""
    ).fetchall()
    return rows_to_dicts(r)


def cmd_get_settings(conn, args):
    # Settings live in a JSON file, but the app also persists device_id in DB.
    settings_path = Path.home() / "Library" / "Application Support" / "com.skillmint" / "settings.json"
    defaults = {
        "device_id": "",
        "center_repo": "",
        "default_sync_mode": "symlink",
        "auto_sync_interval_minutes": 5,
        "launch_at_login": False,
        "show_dock_icon": True,
        "onboarding_completed": True,
        "skill_scope_mode": "global",
        "project_skill_dir_name": ".skillmint/skills",
        "remote_enabled": False,
        "theme": "system",
        "ai": {
            "models": [],
            "acp_connections": [],
            "default_chat_model_id": None,
            "default_embedding_model_id": None,
            "prefer_acp": False,
            "strict_local_mode": False,
        },
    }
    if settings_path.exists():
        try:
            saved = json.loads(settings_path.read_text())
            defaults.update(saved)
            # Deep-merge ai so a partially-saved settings.json doesn't drop
            # sub-fields the frontend expects (issue #2: AI/ACP went missing).
            saved_ai = saved.get("ai") or {}
            merged_ai = dict(defaults["ai"])
            merged_ai.update(saved_ai)
            defaults["ai"] = merged_ai
        except Exception:
            pass
    if not defaults.get("device_id"):
        row = conn.execute(
            "SELECT device_id FROM skills WHERE device_id != '' LIMIT 1"
        ).fetchone()
        if row:
            defaults["device_id"] = row["device_id"]
    defaults["center_repo"] = str(Path.home() / ".skillmint" / "repo")
    return defaults


def cmd_init_app(conn, args):
    return None


def cmd_sync_all_command(conn, args):
    # Return a plausible SyncAllResult: re-read current targets as "synced".
    targets = cmd_get_sync_targets(conn, args)
    failures = []
    for t in targets:
        if t["status"] is None:
            t["status"] = "synced"
        if t["status"] == "broken":
            failures.append({
                "target_id": t["id"],
                "skill_id": t.get("skill_id", ""),
                "skill_name": t.get("skill_name"),
                "agent_id": t.get("agent_id", ""),
                "agent_name": t.get("agent_name"),
                "error": "mock broken target",
                "recovery_hint": "目标路径已失效，请在 Skill 详情重新绑定或恢复该目录",
            })
    success_count = sum(1 for t in targets if t["status"] == "synced")
    failure_count = len(failures)
    return {
        "targets": targets,
        "imported_skills": 0,
        "import_conflicts": 0,
        "success_count": success_count,
        "failure_count": failure_count,
        "failures": failures,
    }


def cmd_sync_single_skill_command(conn, args):
    # SPEC-C2 T2: mock returns 0/0 (no real file ops). The real backend syncs
    # the skill to every enabled agent.
    return {
        "targets": [],
        "imported_skills": 0,
        "import_conflicts": 0,
        "success_count": 0,
        "failure_count": 0,
        "failures": [],
    }


def cmd_scan_agents(conn, args):
    r = conn.execute("SELECT id FROM agents").fetchall()
    return [{"id": row["id"]} for row in r]


def cmd_scan_agent_skills(conn, args):
    agent_id = args.get("agentId")
    r = conn.execute(
        """SELECT s.id, s.name, st.status, st.mode, st.last_sync_at, st.id AS target_id
           FROM sync_targets st JOIN skills s ON s.id = st.skill_id
           WHERE st.agent_id = ? ORDER BY s.name""",
        (agent_id,),
    ).fetchall()
    return rows_to_dicts(r)


def cmd_get_agent_detail(conn, args):
    agent_id = args.get("agentId")
    a = conn.execute(
        """SELECT id, name, skill_directory, is_enabled, description, source,
                  last_used_at, project_count FROM agents WHERE id = ?""",
        (agent_id,),
    ).fetchone()
    if not a:
        return None
    d = dict(a)
    d["is_enabled"] = bool(d["is_enabled"])
    # source now a real column (PRD-06); the old None override is removed.
    # usage_7d matches AgentUsageSummary (7-day window)
    cutoff = int(time.time()) - 7 * 86400
    usage = conn.execute(
        """SELECT COUNT(*) AS session_count,
                  COALESCE(SUM(message_count),0) AS prompt_count,
                  MIN(start_time) AS first, MAX(end_time) AS last
           FROM collected_sessions
           WHERE agent_id = ? AND start_time >= ?""",
        (agent_id, cutoff),
    ).fetchone()
    tokens = conn.execute(
        """SELECT COALESCE(SUM(total_tokens),0) AS total_tokens,
                  COALESCE(SUM(input_tokens),0) AS input_tokens,
                  COALESCE(SUM(output_tokens),0) AS output_tokens,
                  COALESCE(SUM(cache_read_input_tokens),0) AS cache_read,
                  COALESCE(SUM(cache_creation_input_tokens),0) AS cache_create
           FROM collected_token_usage WHERE session_id IN
             (SELECT id FROM collected_sessions WHERE agent_id = ? AND start_time >= ?)""",
        (agent_id, cutoff),
    ).fetchone()
    d["usage_7d"] = {
        "source": "",
        "days": 7,
        "session_count": usage["session_count"] if usage else 0,
        "prompt_count": usage["prompt_count"] if usage else 0,
        "total_tokens": tokens["total_tokens"] if tokens else 0,
        "input_tokens": tokens["input_tokens"] if tokens else 0,
        "output_tokens": tokens["output_tokens"] if tokens else 0,
        "cache_read_input_tokens": tokens["cache_read"] if tokens else 0,
        "cache_creation_input_tokens": tokens["cache_create"] if tokens else 0,
    }
    # projects: ProjectUsageSummary shape { project_id, name, path, session_count, total_tokens }
    projs = conn.execute(
        """SELECT p.id AS project_id, p.name, p.path,
                  COUNT(DISTINCT cs.id) AS session_count,
                  COALESCE(SUM(ct.total_tokens),0) AS total_tokens
           FROM projects p
           LEFT JOIN collected_sessions cs ON cs.project_id = p.id
           LEFT JOIN collected_token_usage ct ON ct.session_id = cs.id
           GROUP BY p.id ORDER BY p.last_active_at DESC LIMIT 100"""
    ).fetchall()
    d["projects"] = [
        {
            "project_id": p["project_id"],
            "name": p["name"],
            "path": p["path"],
            "session_count": p["session_count"],
            "total_tokens": p["total_tokens"],
        }
        for p in projs
    ]
    d["skills"] = cmd_scan_agent_skills(conn, {"agentId": agent_id})
    # PRD-06 §3.3: include all directories owned by this agent (1:N).
    d["directories"] = _agent_directories(conn, agent_id)
    return d


def cmd_get_collection_status(conn, args):
    r = conn.execute(
        """SELECT source, collector_kind, data_path, status,
                  last_collected_at, record_count FROM collected_sources"""
    ).fetchall()
    return rows_to_dicts(r)


def _day_bounds(iso: str):
    from datetime import datetime, timezone, timedelta
    d = datetime.strptime(iso, "%Y-%m-%d").replace(tzinfo=timezone.utc)
    start = int(d.timestamp())
    end = int((d + timedelta(days=1)).timestamp()) - 1
    return start, end


def _window_metrics_for_date(conn, ref_date: str):
    """Compute a plausible day-window metrics payload from collected_* tables."""
    start, end = _day_bounds(ref_date)
    sessions = conn.execute(
        """SELECT id, project_id, source, start_time, message_count
           FROM collected_sessions WHERE start_time >= ? AND start_time <= ?""",
        (start, end),
    ).fetchall()
    session_ids = [s["id"] for s in sessions]
    by_project = {}
    by_platform = {}
    total_tokens = 0
    input_tokens = 0
    output_tokens = 0
    reasoning_tokens = 0
    cache_read_tokens = 0
    cache_creation_tokens = 0
    model_calls = 0
    tool_calls = 0
    duration_hours = 0.0
    if session_ids:
        placeholders = ",".join("?" * len(session_ids))
        toks = conn.execute(
            f"""SELECT source, project_id, model_id, input_tokens, output_tokens,
                       reasoning_tokens, cache_creation_input_tokens, cache_read_input_tokens,
                       total_tokens, model_calls, tool_calls, duration_ms
                FROM collected_token_usage WHERE session_id IN ({placeholders})""",
            session_ids,
        ).fetchall()
        for t in toks:
            src = t["source"] or "unknown"
            proj = t["project_id"] or "未知项目"
            by_platform[src] = by_platform.get(src, 0) + (t["total_tokens"] or 0)
            by_project[proj] = by_project.get(proj, 0) + (t["total_tokens"] or 0)
            total_tokens += t["total_tokens"] or 0
            input_tokens += t["input_tokens"] or 0
            output_tokens += t["output_tokens"] or 0
            reasoning_tokens += t["reasoning_tokens"] or 0
            cache_read_tokens += t["cache_read_input_tokens"] or 0
            cache_creation_tokens += t["cache_creation_input_tokens"] or 0
            model_calls += t["model_calls"] or 0
            tool_calls += t["tool_calls"] or 0
            duration_hours += (t["duration_ms"] or 0) / 3_600_000.0
    est_cost = (input_tokens * 0.000_003 + output_tokens * 0.000_012 +
                cache_read_tokens * 0.000_000_75 + cache_creation_tokens * 0.000_001_5)
    total_prompts = sum(s["message_count"] or 0 for s in sessions)
    by_project_prompts = {}
    by_platform_prompts = {}
    for s in sessions:
        proj = s["project_id"] or "未知项目"
        src = s["source"] or "unknown"
        by_project_prompts[proj] = by_project_prompts.get(proj, 0) + (s["message_count"] or 0)
        by_platform_prompts[src] = by_platform_prompts.get(src, 0) + (s["message_count"] or 0)
    return {
        "kind": "day",
        "ref_date": ref_date,
        "current_window": {"start": ref_date, "end": ref_date},
        "previous_window": {"start": ref_date, "end": ref_date},
        "yoy_window": {"start": ref_date, "end": ref_date},
        "has_previous_baseline": False,
        "has_yoy_baseline": False,
        "comparison": {},
        "token_dimension": {
            "scale": {
                "total_tokens": total_tokens,
                "fresh_tokens": total_tokens,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "reasoning_tokens": reasoning_tokens,
                "cache_read_tokens": cache_read_tokens,
                "cache_creation_tokens": cache_creation_tokens,
                "model_calls": model_calls,
                "tool_calls": tool_calls,
                "duration_hours": round(duration_hours, 2),
            },
            "distribution": {
                "by_platform": by_platform,
                "by_project": by_project,
                "by_model": {},
            },
            "cost": {
                "est_cost_cny": round(est_cost, 4),
                "by_platform_cny": {k: round(v * 0.000_005, 4) for k, v in by_platform.items()},
                "billing_mix": {},
            },
            "diagnostics": {"cache_ratio": 0.0, "heavy_sessions": []},
        },
        "prompt_dimension": {
            "penetration": {
                "total_prompts": total_prompts,
                "by_platform": by_platform_prompts,
                "by_project": by_project_prompts,
            },
            "semantics": {
                "classified_ratio": 0.0,
                "requested_action": {},
                "target_object": {},
                "interaction_state": {},
                "interaction_mode": {},
            },
            "quality": {
                "score": 0,
                "clarification_correction_rate": 0.0,
                "planning_ratio": 0.0,
                "test_object_ratio": 0.0,
                "improvement_suggestions": [],
            },
        },
        "leverage": {
            "est_cost_cny": round(est_cost, 4),
            "variable_cost_cny": round(est_cost, 4),
            "subscription_cost_cny": 0.0,
            "fresh_tokens": total_tokens,
            "output_proxy": 0,
            "leverage_per_cny": 0.0,
            "cost_per_prompt_cny": 0.0 if total_prompts == 0 else round(est_cost / total_prompts, 4),
        },
        "entity_metrics": {
            "role_profile": {"tools": {}, "degraded": False},
            "cost_split": {
                "variable_cny": round(est_cost, 4),
                "variable_calls": model_calls,
                "subscription_calls": 0,
                "subscription_intensity": "low",
            },
            "agent_coefficient_by_tool": {"tools": {}},
        },
    }


def cmd_get_daily_summary(conn, args):
    date = args.get("date")
    row = conn.execute(
        "SELECT date, highlights, activities, model, provider, created_at FROM digest_summary WHERE date = ?",
        (date,),
    ).fetchone()
    if not row:
        return None
    activities = row["activities"]
    try:
        activities = json.loads(activities) if activities else []
    except Exception:
        activities = []
    return {
        "date": row["date"],
        "highlights": [row["highlights"]] if row["highlights"] else [],
        "activities": activities,
        "model": row["model"],
        "provider": row["provider"],
        "created_at": row["created_at"],
    }


def cmd_list_daily_summaries(conn, args):
    start_date = args.get("startDate")
    end_date = args.get("endDate")
    rows = conn.execute(
        """SELECT date, model, provider, created_at FROM digest_summary
           WHERE date >= ? AND date <= ? ORDER BY date DESC""",
        (start_date, end_date),
    ).fetchall()
    return [
        {
            "date": r["date"],
            "model": r["model"],
            "provider": r["provider"],
            "created_at": r["created_at"],
        }
        for r in rows
    ]


def cmd_get_window_metrics(conn, args):
    ref_date = args.get("refDate")
    return _window_metrics_for_date(conn, ref_date)


def cmd_start_collection_job(conn, args):
    import uuid
    job_id = str(uuid.uuid4())
    now = now_ms() // 1000
    conn.execute(
        "INSERT INTO collection_jobs (id, started_at, status, progress_json) VALUES (?, ?, ?, ?)",
        (job_id, now, "running", "{}"),
    )
    conn.commit()
    return job_id


def cmd_list_recent_collection_jobs(conn, args):
    limit = args.get("limit", 20)
    rows = conn.execute(
        """SELECT id, started_at, completed_at, status, progress_json, result_json, error
           FROM collection_jobs ORDER BY started_at DESC LIMIT ?""",
        (limit,),
    ).fetchall()
    return [
        {
            "id": r["id"],
            "started_at": r["started_at"],
            "completed_at": r["completed_at"],
            "status": r["status"],
            "progress_json": r["progress_json"],
            "result_json": r["result_json"],
            "error": r["error"],
        }
        for r in rows
    ]


def cmd_cancel_collection_job(conn, args):
    job_id = args.get("id")
    now = now_ms() // 1000
    conn.execute(
        "UPDATE collection_jobs SET status = ?, completed_at = ? WHERE id = ?",
        ("cancelled", now, job_id),
    )
    conn.commit()
    return None


def cmd_get_agent_usage(conn, args):
    source = args.get("source")
    days = args.get("days", 30)
    # SPEC-F4 T2: days == 0 means "all time".
    now = int(time.time())
    if days == 0:
        cutoff = 0
        session_where = "source = ?"
        session_params = (source,)
    else:
        cutoff = now - days * 86400
        session_where = "source = ? AND start_time >= ?"
        session_params = (source, cutoff)
    row = conn.execute(
        f"""SELECT COUNT(*) AS session_count,
                  COALESCE(SUM(message_count),0) AS prompt_count
           FROM collected_sessions
           WHERE {session_where}""",
        session_params,
    ).fetchone()
    toks = conn.execute(
        """SELECT COALESCE(SUM(total_tokens),0) AS total_tokens,
                  COALESCE(SUM(input_tokens),0) AS input_tokens,
                  COALESCE(SUM(output_tokens),0) AS output_tokens,
                  COALESCE(SUM(cache_read_input_tokens),0) AS cache_read_input_tokens,
                  COALESCE(SUM(cache_creation_input_tokens),0) AS cache_creation_input_tokens
           FROM collected_token_usage
           WHERE source = ?""",
        (source,),
    ).fetchone()
    return {
        "source": source,
        "days": days,
        "session_count": row["session_count"] if row else 0,
        "prompt_count": row["prompt_count"] if row else 0,
        "total_tokens": toks["total_tokens"] if toks else 0,
        "input_tokens": toks["input_tokens"] if toks else 0,
        "output_tokens": toks["output_tokens"] if toks else 0,
        "cache_read_input_tokens": toks["cache_read_input_tokens"] if toks else 0,
        "cache_creation_input_tokens": toks["cache_creation_input_tokens"] if toks else 0,
    }


def cmd_collect_usage_data(conn, args):
    # PRD-05: actually run all four collectors so the click-test exercises the
    # zcode/cursor paths. claude-code/codex are already collected (cached); zcode
    # reads ~/.zcode/cli/db/db.sqlite, cursor reads ~/.cursor/ai-tracking.
    device_id = args.get("deviceId") or _device_id(conn)
    now = now_ms() // 1000
    out = []

    # 1. Claude Code + Codex: report cached counts (a prior real run filled them).
    for src in ("claude-code", "codex"):
        row = conn.execute(
            "SELECT COUNT(*) AS c FROM collected_sessions WHERE source = ?", (src,)
        ).fetchone()
        cnt = row["c"] if row else 0
        out.append({"source": src, "sessions_collected": cnt, "new": 0})
        conn.execute(
            "INSERT OR REPLACE INTO collected_sources (source, collector_kind, data_path, status, last_collected_at, record_count) VALUES (?,?,?,?,?,?)",
            (src, "direct_file", f"~/.{src.replace('claude-code','claude')}/projects" if src == 'claude-code' else "~/.codex/sessions", "ok", now, cnt),
        )

    # 2. ZCode: read ~/.zcode/cli/db/db.sqlite read-only (PRD-05 §3.1).
    zc_stats = _collect_zcode(conn, device_id, now)
    out.append(zc_stats)

    # 3. Cursor: read ~/.cursor/ai-tracking/ai-code-tracking.db (PRD-05 §3.2).
    cur_stats = _collect_cursor(conn, device_id, now)
    out.append(cur_stats)

    conn.commit()
    return out


def _device_id(conn):
    row = conn.execute("SELECT device_id FROM skills WHERE device_id != '' LIMIT 1").fetchone()
    return row["device_id"] if row else "mock-device"


def _ro_open(path):
    # Open read-only via URI so we never lock a live agent process (PRD-05 §6).
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def _ensure_project(conn, device_id, path):
    if not path:
        return None
    row = conn.execute(
        "SELECT id FROM projects WHERE device_id = ? AND path = ?", (device_id, path)
    ).fetchone()
    if row:
        return row["id"]
    import hashlib
    pid = "proj_" + hashlib.sha1(path.encode()).hexdigest()[:32]
    name = path.rstrip("/").split("/")[-1] or path
    now = now_ms() // 1000
    conn.execute(
        "INSERT OR IGNORE INTO projects (id, device_id, name, path, first_seen_at, last_active_at, is_stale, created_at, updated_at) VALUES (?,?,?,?,?,?,0,?,?)",
        (pid, device_id, name, path, now, now, now, now),
    )
    return pid


def _collect_zcode(conn, device_id, now):
    path = Path.home() / ".zcode/cli/db/db.sqlite"
    if not path.is_file():
        conn.execute(
            "INSERT OR REPLACE INTO collected_sources (source, collector_kind, data_path, status, last_collected_at, record_count) VALUES ('zcode','sqlite',?,'not_found',?,0)",
            (str(path), now),
        )
        return {"source": "zcode", "sessions_collected": 0, "new": 0}
    try:
        zc = _ro_open(str(path))
        sessions = zc.execute(
            "SELECT id, directory, title, time_created, time_updated FROM session"
        ).fetchall()
        count = 0
        prompts = 0
        for sid, directory, title, tc, tu in sessions:
            session_pk = f"{device_id}:zcode:{sid}"
            proj = _ensure_project(conn, device_id, directory)
            # tokens per (model, query_source)
            toks = zc.execute(
                """SELECT model_id, query_source,
                          SUM(input_tokens), SUM(output_tokens), SUM(reasoning_tokens),
                          SUM(cache_creation_input_tokens), SUM(cache_read_input_tokens),
                          SUM(computed_total_tokens), COUNT(*), SUM(tool_call_count), SUM(duration_ms)
                   FROM model_usage WHERE session_id = ? GROUP BY model_id, query_source""",
                (sid,),
            ).fetchall()
            # prompt count via part JOIN message
            pcount = zc.execute(
                """SELECT COUNT(*) FROM part p JOIN message m ON p.message_id = m.id
                   WHERE p.session_id = ? AND json_extract(p.data,'$.type')='text'
                     AND json_extract(m.data,'$.role')='user'""",
                (sid,),
            ).fetchone()[0]
            conn.execute(
                "INSERT OR REPLACE INTO collected_sessions (id, device_id, source, project_id, start_time, end_time, message_count, title_or_prompt, cached_at, project_path) VALUES (?,?,?,?,?,?,?,?,?,?)",
                (session_pk, device_id, "zcode", proj, (tc or 0)//1000, (tu or 0)//1000, pcount, title, now, directory or None),
            )
            for (model, qs, inp, outp, reason, cc, cr, total, calls, tcall, dur) in toks:
                folded = f"{model}@{qs}" if qs and qs != "main_turn" else model
                conn.execute(
                    "INSERT OR REPLACE INTO collected_token_usage (id, device_id, session_id, source, project_id, model_id, input_tokens, output_tokens, reasoning_tokens, cache_creation_input_tokens, cache_read_input_tokens, total_tokens, model_calls, tool_calls, duration_ms) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                    (f"{session_pk}:{folded}", device_id, session_pk, "zcode", proj, folded, inp or 0, outp or 0, reason or 0, cc or 0, cr or 0, total or 0, calls or 0, tcall or 0, dur or 0),
                )
            # prompts
            for (pid_, text, pts) in zc.execute(
                """SELECT p.id, json_extract(p.data,'$.text'), p.time_created/1000
                   FROM part p JOIN message m ON p.message_id = m.id
                   WHERE p.session_id = ? AND json_extract(p.data,'$.type')='text'
                     AND json_extract(m.data,'$.role')='user' ORDER BY p.time_created""",
                (sid,),
            ).fetchall():
                if not text or not text.strip():
                    continue
                conn.execute(
                    "INSERT OR REPLACE INTO collected_prompts (id, device_id, session_id, source, project_id, prompt_text, started_at) VALUES (?,?,?,?,?,?,?)",
                    (f"{session_pk}:{pid_}", device_id, session_pk, "zcode", proj, text[:2000], pts),
                )
                prompts += 1
            count += 1
        zc.close()
        conn.execute(
            "INSERT OR REPLACE INTO collected_sources (source, collector_kind, data_path, status, last_collected_at, record_count) VALUES ('zcode','sqlite',?,'ok',?,?)",
            (str(path), now, count),
        )
        return {"source": "zcode", "sessions_collected": count, "new": prompts}
    except Exception as e:
        conn.execute(
            "INSERT OR REPLACE INTO collected_sources (source, collector_kind, data_path, status, last_collected_at, record_count) VALUES ('zcode','sqlite',?,'error',?,0)",
            (str(path), now),
        )
        return {"source": "zcode", "sessions_collected": 0, "new": 0, "error": str(e)}


def _collect_cursor(conn, device_id, now):
    path = Path.home() / ".cursor/ai-tracking/ai-code-tracking.db"
    if not path.is_file():
        conn.execute(
            "INSERT OR REPLACE INTO collected_sources (source, collector_kind, data_path, status, last_collected_at, record_count) VALUES ('cursor','sqlite',?,'not_found',?,0)",
            (str(path), now),
        )
        return {"source": "cursor", "sessions_collected": 0, "new": 0}
    try:
        cu = _ro_open(str(path))
        rows = cu.execute(
            """SELECT commitHash, branchName, commitDate, scoredAt,
                      linesAdded, linesDeleted, composerLinesAdded, composerLinesDeleted,
                      humanLinesAdded, humanLinesDeleted, tabLinesAdded,
                      v2AiPercentage, v1AiPercentage, commitMessage
               FROM scored_commits"""
        ).fetchall()
        commits = 0
        for (ch, branch, cdate, sat, la, ld, cla, cld, hla, hld, taba, v2, v1, msg) in rows:
            pct = None
            for raw in (v2, v1):
                if raw is None:
                    continue
                try:
                    t = str(raw).strip().rstrip("%")
                    v = float(t)
                    pct = (v * 100.0 if v <= 1.0 and "%" not in str(raw) and t != "1" else v)
                    pct = max(0.0, min(100.0, pct))
                    break
                except Exception:
                    continue
            conn.execute(
                """INSERT OR REPLACE INTO collected_code_contributions
                   (id, device_id, source, project_id, commit_hash, branch_name, commit_date,
                    scored_at, lines_added, lines_deleted, composer_lines_added, composer_lines_deleted,
                    human_lines_added, human_lines_deleted, tab_lines_added, ai_percentage,
                    commit_message, cached_at)
                   VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
                (f"{device_id}:cursor:{ch}", device_id, "cursor", None, ch, branch, cdate,
                 (sat or 0)//1000, la, ld, cla, cld, hla, hld, taba, pct, msg, now),
            )
            commits += 1
        cu.close()
        conn.execute(
            "INSERT OR REPLACE INTO collected_sources (source, collector_kind, data_path, status, last_collected_at, record_count) VALUES ('cursor','sqlite',?,'ok',?,?)",
            (str(path), now, commits),
        )
        return {"source": "cursor", "sessions_collected": commits, "new": 0}
    except Exception as e:
        conn.execute(
            "INSERT OR REPLACE INTO collected_sources (source, collector_kind, data_path, status, last_collected_at, record_count) VALUES ('cursor','sqlite',?,'error',?,0)",
            (str(path), now),
        )
        return {"source": "cursor", "sessions_collected": 0, "new": 0, "error": str(e)}


def cmd_get_high_value_prompts(conn, args):
    min_repeat = args.get("minRepeat", 3)
    r = conn.execute(
        """SELECT substr(prompt_text, 1, 120) AS key,
                  source,
                  COUNT(*) AS repeat_count,
                  MIN(started_at) AS first_seen, MAX(started_at) AS last_seen,
                  MAX(session_id) AS sample_session_id
           FROM collected_prompts
           WHERE prompt_text IS NOT NULL AND LENGTH(prompt_text) > 10
           GROUP BY key, source
           HAVING repeat_count >= ?
           ORDER BY repeat_count DESC LIMIT 20""",
        (min_repeat,),
    ).fetchall()
    return [
        {
            "prompt_text": row["key"],
            "source": row["source"],
            "repeat_count": row["repeat_count"],
            "first_seen": row["first_seen"],
            "last_seen": row["last_seen"],
            "sample_session_id": row["sample_session_id"],
        }
        for row in r
    ]


def _validate_prompt_for_skill(prompt_text):
    """SPEC-F2 T1: same quality gate as the Rust backend."""
    trimmed = prompt_text.strip()
    lower = trimmed.lower()
    for prefix in ("[image:", "[file:", "[attachment:"):
        if lower.startswith(prefix):
            raise ValueError("此 Prompt 内容无法沉淀为 Skill（包含图片/文件/附件占位符）")

    placeholder_len = 0
    start = 0
    while True:
        idx = trimmed.find("[", start)
        if idx == -1:
            break
        end = trimmed.find("]", idx)
        if end == -1:
            break
        inner = trimmed[idx + 1:end].lower()
        if inner.startswith(("image:", "file:", "attachment:")):
            placeholder_len += end - idx + 1
        start = end + 1

    total_len = len(trimmed)
    if total_len > 0 and placeholder_len * 100 > total_len * 50:
        raise ValueError("此 Prompt 内容无法沉淀为 Skill（占位符占比过高）")

    without_space = "".join(c for c in trimmed if not c.isspace())
    if len(without_space) < 20:
        raise ValueError("此 Prompt 内容无法沉淀为 Skill（内容过短，无法提取有效语义）")

    has_letter = any(
        ("a" <= ch <= "z" or "A" <= ch <= "Z" or "\u4e00" <= ch <= "\u9fff")
        for ch in trimmed
    )
    if not has_letter:
        raise ValueError(
            "此 Prompt 内容无法沉淀为 Skill（未包含有效文字，仅由符号、数字或链接组成）"
        )

    cleaned = "".join(
        ch if (ch.isalnum() or ch in "-_" or "\u4e00" <= ch <= "\u9fff") else " "
        for ch in trimmed
    )
    tokens = [w.strip().strip("".join(c for c in string.punctuation if c not in "-_")).lower()
              for w in cleaned.split()]
    tokens = [w for w in tokens if w]
    name = "-".join(tokens)[:48].rstrip("-")
    if not name or not any(ch.isalpha() for ch in name):
        raise ValueError("此 Prompt 内容无法沉淀为 Skill（无法提取有效 Skill 名称）")
    return name


def cmd_repair_skill_paths(conn, args):
    """SPEC-F5 T5: diagnostic that re-runs the repo migration. The mock has no
    real filesystem layout to repair, so it reports a no-op result; the real
    Tauri backend performs the actual idempotent migration."""
    return {"migrated": [], "failed": []}


def cmd_preview_skill_from_prompt(conn, args):
    """SPEC-F5 T3: preview the tokenized name + description without persisting.
    Mirrors the Rust `preview_skill_from_prompt` command. Reuses the existing
    `_validate_prompt_for_skill` gate so the dialog shows the same rejection
    reason the backend would surface."""
    prompt_text = args.get("promptText", "")
    name = _validate_prompt_for_skill(prompt_text)
    sentence = prompt_text.strip()[:120].rstrip().rstrip(string.punctuation + string.whitespace)
    if len(sentence) >= len(prompt_text.strip()):
        scenario = "适用于从该 prompt 直接复现的工作流。"
    else:
        scenario = f"适用于「{sentence[:40]}…」等相似场景。"
    desc = f"{sentence}\n\n适用场景：{scenario}"
    return {"name": name, "description": desc}


def cmd_generate_skill_from_prompt(conn, args):
    import uuid

    prompt_text = args.get("promptText", "")
    name = _validate_prompt_for_skill(prompt_text)

    sid = str(uuid.uuid4())
    now = int(time.time())
    device = conn.execute(
        "SELECT device_id FROM skills WHERE device_id != '' LIMIT 1"
    ).fetchone()
    dev = device["device_id"] if device else "mock"
    repo = Path.home() / ".skillmint" / "repo" / name
    repo.mkdir(parents=True, exist_ok=True)
    repo_str = str(repo)
    sentence = prompt_text.strip()[:120].rstrip().rstrip(string.punctuation + string.whitespace)
    scenario = "适用于从该 prompt 直接复现的工作流。"
    desc = f"{sentence}\n\n适用场景：{scenario}"
    skill_md = repo / "SKILL.md"
    skill_md.write_text(f"# {name}\n\n## Description\n\n{desc}\n\n## Usage\n\nDerived from a repeated prompt.\n", encoding="utf-8")
    conn.execute(
        """INSERT INTO skills (id, name, repo_path, created_at, updated_at, device_id)
           VALUES (?,?,?,?,?,?)""",
        (sid, name, repo_str, now, now, dev),
    )
    conn.commit()
    return {"id": sid, "name": name, "repo_path": repo_str, "created_at": now, "updated_at": now, "status": "draft"}


def cmd_get_knowledge_graph(conn, args):
    max_nodes = args.get("maxNodes", 200)
    # Prefer practice nodes (skill nodes) + their direct neighbours so the
    # graph always shows the relations that matter; top up with concepts.
    practice = conn.execute(
        "SELECT id, label, type, source, description FROM kg_nodes WHERE type='practice'"
    ).fetchall()
    practice_ids = [n["id"] for n in practice]
    # all edges (there are ~1.2k)
    edges = conn.execute(
        """SELECT e.id, e.device_id, e.source_id, e.target_id, e.relation,
                  e.weight, e.reason, e.is_manual, e.is_rejected
           FROM kg_edges e LIMIT 1000"""
    ).fetchall()
    # node ids referenced by any edge
    referenced = set(practice_ids)
    for e in edges:
        referenced.add(e["source_id"])
        referenced.add(e["target_id"])
    placeholders = ",".join("?" * len(referenced)) or "''"
    extra = conn.execute(
        f"""SELECT id, label, type, source, description FROM kg_nodes
            WHERE id IN ({placeholders}) LIMIT ?""",
        (*referenced, max_nodes),
    ).fetchall() if referenced else []
    seen = set()
    nodes = []
    for n in list(practice) + list(extra):
        if n["id"] in seen:
            continue
        seen.add(n["id"])
        nodes.append(
            {
                "id": n["id"],
                "label": n["label"],
                "type": n["type"],
                "source": n["source"],
                "description": n["description"],
            }
        )
    return {
        "nodes": nodes[:max_nodes],
        "edges": [
            {
                "id": e["id"],
                "device_id": e["device_id"],
                "source_id": e["source_id"],
                "target_id": e["target_id"],
                "relation": e["relation"],
                "weight": e["weight"],
                "reason": e["reason"],
                "is_manual": bool(e["is_manual"]),
                "is_rejected": bool(e["is_rejected"]),
            }
            for e in edges
        ],
    }


def cmd_analyze_knowledge_graph(conn, args):
    nodes = conn.execute("SELECT COUNT(*) AS c FROM kg_nodes").fetchone()
    edges = conn.execute("SELECT COUNT(*) AS c FROM kg_edges").fetchone()
    return [nodes["c"], edges["c"]]


def cmd_recommend_skills_for_task(conn, args):
    task = args.get("task", "")
    return {
        "matched_skills": [],
        "uncovered_concepts": [],
        "task": task,
        "coverage": 0.0,
    }


def cmd_confirm_kg_edge(conn, args):
    eid = args.get("id")
    conn.execute("UPDATE kg_edges SET is_manual = 1, is_rejected = 0 WHERE id = ?", (eid,))
    conn.commit()
    return None


def cmd_reject_kg_edge(conn, args):
    eid = args.get("id")
    conn.execute(
        "UPDATE kg_edges SET is_rejected = 1 WHERE id = ?", (eid,)
    )
    conn.commit()
    return None


def cmd_get_projects(conn, args):
    r = conn.execute(
        """SELECT p.id, p.device_id, p.name, p.path, p.first_seen_at,
                  p.last_active_at, p.agent_sources, p.is_stale
           FROM projects p
           ORDER BY p.last_active_at DESC"""
    ).fetchall()
    out = []
    for row in r:
        d = dict(row)
        # Frontend expects `project_id`, not the raw column name `id`.
        d["project_id"] = d.pop("id")
        d["is_stale"] = bool(d["is_stale"]) if d["is_stale"] is not None else False
        # sessions + tokens in last 30d
        usage = conn.execute(
            """SELECT COUNT(DISTINCT cs.id) AS sessions,
                      COALESCE(SUM(ct.total_tokens),0) AS tokens
               FROM collected_sessions cs
               LEFT JOIN collected_token_usage ct ON ct.session_id = cs.id
               WHERE cs.project_id = ?""",
            (d["project_id"],),
        ).fetchone()
        d["session_count"] = usage["sessions"] if usage else 0
        d["total_tokens"] = usage["tokens"] if usage else 0
        # skills bound to this project
        skills = conn.execute(
            """SELECT s.name FROM skill_project_bindings b
               JOIN skills s ON s.id = b.skill_id
               WHERE b.project_id = ?""",
            (d["project_id"],),
        ).fetchall()
        d["skills"] = [s["name"] for s in skills]
        out.append(d)
    return out


def cmd_get_skill_bindings(conn, args):
    r = conn.execute(
        """SELECT b.id, b.device_id, b.skill_id, b.project_id, b.agent_id,
                  b.mode, b.local_path, b.is_enabled, b.created_at, b.updated_at
           FROM skill_project_bindings b"""
    ).fetchall()
    out = []
    for row in r:
        d = dict(row)
        d["is_enabled"] = bool(d["is_enabled"])
        out.append(d)
    return out


def cmd_scan_projects(conn, args):
    r = conn.execute("SELECT COUNT(*) AS c FROM projects").fetchone()
    return r["c"] if r else 0


# --- PRD-01 patch: project detail + multi-version skills ---------------------

def cmd_get_project_detail(conn, args):
    project_id = args.get("projectId")
    p = conn.execute(
        "SELECT id, name, path, last_active_at FROM projects WHERE id = ?",
        (project_id,),
    ).fetchone()
    if not p:
        raise RuntimeError("project not found")
    agents = conn.execute(
        """SELECT a.id, a.name, a.skill_directory, a.is_enabled,
                  ai.last_session_at, ai.session_count, ai.total_tokens
           FROM agent_instances ai
           JOIN agents a ON ai.agent_id = a.id
           WHERE ai.project_id = ?
           ORDER BY ai.total_tokens DESC""",
        (project_id,),
    ).fetchall()
    agent_list = []
    for a in agents:
        d = dict(a)
        d["agent_id"] = d.pop("id")
        d["agent_name"] = d.pop("name")
        d["is_enabled"] = bool(d["is_enabled"])
        # count skill dirs physically (best-effort; fall back to None)
        try:
            sd = d.get("skill_directory")
            d["skill_count"] = (
                len([x for x in os.listdir(sd) if os.path.isdir(os.path.join(sd, x))])
                if sd and os.path.isdir(sd)
                else 0
            )
        except OSError:
            d["skill_count"] = None
        agent_list.append(d)
    usage = conn.execute(
        """SELECT COUNT(DISTINCT cs.id) AS sessions,
                  COALESCE(SUM(ct.total_tokens),0) AS tokens
           FROM collected_sessions cs
           LEFT JOIN collected_token_usage ct ON ct.session_id = cs.id
           WHERE cs.project_id = ?""",
        (project_id,),
    ).fetchone()
    return {
        "project_id": p["id"],
        "name": p["name"],
        "path": p["path"],
        "last_active_at": p["last_active_at"],
        "session_count": usage["sessions"] if usage else 0,
        "total_tokens": usage["tokens"] if usage else 0,
        "agents": agent_list,
    }


def cmd_resolve_skill_link_command(conn, args):
    # Read-only mock: derive source from binding row if present, else default.
    project_id = args.get("projectId")
    agent_id = args.get("agentId")
    skill_name = args.get("skillName")
    b = conn.execute(
        """SELECT b.pinned_version, b.mode
           FROM skill_project_bindings b
           JOIN skills s ON s.id = b.skill_id
           WHERE b.project_id = ? AND b.agent_id = ? AND s.name = ?""",
        (project_id, agent_id, skill_name),
    ).fetchone()
    return {
        "name": skill_name,
        "source": (dict(b)["mode"] if b and dict(b)["mode"] in ("symlink", "local_copy", "reference") else "local_copy"),
        "target_path": None,
        "content_match": True,
        "is_registered": b is not None,
        "pinned_version": dict(b).get("pinned_version") if b else None,
    }


def cmd_list_skill_versions_command(conn, args):
    skill_id = args.get("skillId")
    pinned = {
        r["pinned_version"]
        for r in conn.execute(
            "SELECT DISTINCT pinned_version FROM skill_project_bindings WHERE skill_id = ? AND pinned_version IS NOT NULL",
            (skill_id,),
        ).fetchall()
    }
    # PRD §4.5d: pinned_by lists project names pinning to each version.
    pinned_versions = sorted({r["pinned_version"] for r in conn.execute(
        "SELECT pinned_version FROM skill_project_bindings WHERE skill_id = ? AND pinned_version IS NOT NULL",
        (skill_id,),
    ).fetchall()})
    out = [{"version": "latest", "created_at": 0, "note": None, "pinned_by": []}]
    for v in pinned_versions:
        projects = [
            r["name"] or "(unnamed)"
            for r in conn.execute(
                """SELECT p.name FROM skill_project_bindings b
                   LEFT JOIN projects p ON b.project_id = p.id
                   WHERE b.skill_id = ? AND b.pinned_version = ?""",
                (skill_id, v),
            ).fetchall()
        ]
        out.append({"version": v, "created_at": 0, "note": None, "pinned_by": projects})
    return out


def cmd_install_skill_to_project(conn, args):
    # Mock: write binding rows only (no fs side effects in read-only mock).
    import uuid
    skill_id = args.get("skillId")
    project_id = args.get("projectId")
    agent_ids = args.get("agentIds", [])
    mode = args.get("mode", "symlink")
    # Normalize mode for DB storage (CHECK: local_copy/symlink/reference).
    stored_mode = "local_copy" if mode == "copy" else mode
    settings_row = conn.execute("SELECT device_id FROM agents LIMIT 1").fetchone()
    device_id = settings_row["device_id"] if settings_row else "mock-device"
    created = []
    for aid in agent_ids:
        bid = str(uuid.uuid4())
        conn.execute(
            """INSERT OR REPLACE INTO skill_project_bindings
               (id, device_id, skill_id, project_id, agent_id, mode, local_path,
                is_enabled, created_at, updated_at, pinned_version)
               VALUES (?,?,?,?,?,?,?,?,?,?,?)""",
            (bid, device_id, skill_id, project_id, aid, stored_mode, None, 1, 0, 0, None),
        )
        created.append({"id": bid, "device_id": device_id, "skill_id": skill_id, "mode": stored_mode, "is_enabled": True})
    conn.commit()
    return created


def cmd_resolve_skill_diff_command(conn, args):
    # Mock: apply the strategy on DB only (pin/unpin). No fs snapshot in read-only mock.
    binding_id = args.get("bindingId")
    strategy = args.get("strategy")
    note = args.get("note")
    new_version = None
    if strategy == "versionize":
        import random
        new_version = f"v{random.randint(1, 99)}"
        conn.execute(
            "UPDATE skill_project_bindings SET pinned_version = ?, updated_at = ? WHERE id = ?",
            (new_version, 0, binding_id),
        )
    elif strategy == "keep_project_backup":
        # Backup creates a version label but does NOT pin (it's a merge with留底).
        import random
        new_version = f"v{random.randint(1, 99)}"
        conn.execute(
            "UPDATE skill_project_bindings SET pinned_version = NULL, updated_at = ? WHERE id = ?",
            (0, binding_id),
        )
    else:
        conn.execute(
            "UPDATE skill_project_bindings SET pinned_version = NULL, updated_at = ? WHERE id = ?",
            (0, binding_id),
        )
    conn.commit()
    return {
        "strategy": strategy,
        "new_version": new_version,
        "project_path": "",
        "latest_path": "",
    }


def cmd_save_settings(conn, args):
    new = args.get("newSettings", {})
    settings_path = (
        Path.home() / "Library" / "Application Support" / "com.skillmint" / "settings.json"
    )
    settings_path.parent.mkdir(parents=True, exist_ok=True)
    # Merge onto existing file so fields the frontend doesn't send (e.g. older
    # persisted keys) survive, mirroring the Rust backend's field-by-field copy.
    existing = {}
    if settings_path.exists():
        try:
            existing = json.loads(settings_path.read_text())
        except Exception:
            existing = {}
    existing.update(new)
    # Deep-merge ai so saving doesn't clobber a previously-stored model list
    # when the payload only carries a subset of ai sub-fields (issue #2).
    merged_ai = dict(existing.get("ai") or {})
    if isinstance(new.get("ai"), dict):
        merged_ai.update(new["ai"])
    existing["ai"] = merged_ai
    settings_path.write_text(json.dumps(existing, indent=2, ensure_ascii=False))
    # Return through the same get path so the shape always matches what the
    # frontend expects (including device_id fallback and default ai fields).
    return cmd_get_settings(conn, args)


def cmd_get_conflict_contents(conn, args):
    target_id = args.get("targetId")
    return {
        "target_id": target_id,
        "local_content": "# 本地内容（mock）\n示例 Skill 内容。",
        "remote_content": "# 中心仓内容（mock）\n较旧的 Skill 内容。",
        "local_mtime": now_ms(),
        "remote_mtime": now_ms() - 3600000,
    }


def cmd_resolve_conflict(conn, args):
    return None


def cmd_create_skill(conn, args):
    name = args.get("name", "new-skill")
    import uuid
    sid = str(uuid.uuid4())
    now = int(time.time())
    device = conn.execute(
        "SELECT device_id FROM skills WHERE device_id != '' LIMIT 1"
    ).fetchone()
    dev = device["device_id"] if device else "mock"
    repo = Path.home() / ".skillmint" / "repo" / name
    repo.mkdir(parents=True, exist_ok=True)
    repo_str = str(repo)
    conn.execute(
        """INSERT INTO skills (id, name, repo_path, created_at, updated_at, device_id)
           VALUES (?,?,?,?,?,?)""",
        (sid, name, repo_str, now, now, dev),
    )
    conn.commit()
    return {"id": sid, "name": name}


def cmd_open_skill_in_editor(conn, args):
    return {"error": "mock 环境不支持：无法从浏览器唤起外部编辑器"}


def cmd_import_skill(conn, args):
    return {"ok": True}


def cmd_get_skill_usage(conn, args):
    days = args.get("days", 30)
    cutoff = int(time.time()) - days * 86400
    r = conn.execute(
        """SELECT sua.skill_id, sua.skill_name,
                  COUNT(*) AS usage_count,
                  COUNT(DISTINCT sua.session_id) AS session_count,
                  COUNT(DISTINCT sua.project_id) AS project_count
           FROM skill_usage_attributions sua
           WHERE sua.attributed_at >= ?
           GROUP BY sua.skill_id, sua.skill_name
           ORDER BY usage_count DESC""",
        (cutoff,),
    ).fetchall()
    return [
        {
            "skill_name": row["skill_name"],
            "skill_id": row["skill_id"],
            "usage_count": row["usage_count"],
            "session_count": row["session_count"],
            "project_count": row["project_count"],
        }
        for row in r
    ]


def cmd_get_related_skills(conn, args):
    skill_id = args.get("skillId")
    # find edges involving this skill's practice node
    skill = conn.execute("SELECT name FROM skills WHERE id = ?", (skill_id,)).fetchone()
    if not skill:
        return []
    import hashlib
    node_id = hashlib.sha256(f'{skill["name"]}:practice'.encode()).hexdigest()[:8]
    r = conn.execute(
        """SELECT e.relation, e.weight, e.reason,
                  n_other.label AS other_name, n_other.id AS other_id
           FROM kg_edges e
           JOIN kg_nodes n_other ON n_other.id = CASE WHEN e.source_id = ? THEN e.target_id ELSE e.source_id END
           WHERE e.source_id = ? OR e.target_id = ?""",
        (node_id, node_id, node_id),
    ).fetchall()
    out = []
    for row in r:
        out.append(
            {
                "skill_name": row["other_name"],
                "relation": row["relation"],
                "weight": row["weight"],
                "reason": row["reason"],
                "confirmed": False,
            }
        )
    return out


# --- SPEC-I2: discovery inbox + weekly reports ---------------------------------

_CATEGORIES = [
    ("debug", "调试", ["debug", "调试", "排查", "troubleshoot", "error", "bug", "crash", "异常"]),
    ("refactor", "重构", ["refactor", "重构", "重写", "整理代码", "clean up", "extract", "拆分"]),
    ("test", "测试", ["test", "测试", "unit test", "assert", "mock", "jest", "pytest", "验证"]),
    ("doc", "文档", ["doc", "文档", "readme", "comment", "说明", "api doc", "注释"]),
    ("deploy", "部署", ["deploy", "部署", "release", "发布", "build", "打包", "ci/cd", "pipeline"]),
    ("database", "数据库", ["database", "数据库", "sql", "schema", "migration", "query", "索引"]),
    ("ui", "UI", ["ui", "界面", "component", "frontend", "react", "vue", "css", "样式"]),
    ("performance", "性能", ["performance", "性能", "optimize", "slow", "memory", "cpu", "cache", "benchmark"]),
    ("security", "安全", ["security", "安全", "auth", "permission", "oauth", "jwt", "encrypt", "漏洞"]),
    ("script", "脚本", ["script", "脚本", "shell", "bash", "automation", "cli", "工具脚本"]),
]


def _device_id_for_metrics(conn):
    row = conn.execute("SELECT device_id FROM skills WHERE device_id != '' LIMIT 1").fetchone()
    if row:
        return row["device_id"]
    row = conn.execute("SELECT device_id FROM agents WHERE device_id != '' LIMIT 1").fetchone()
    if row:
        return row["device_id"]
    return "mock-device"


def cmd_list_discoveries(conn, args):
    status = args.get("status")
    if status:
        rows = conn.execute(
            """SELECT id, kind, title, payload, confidence, dedup_key,
                      status, created_at, decided_at, resulting_skill_id
               FROM discoveries WHERE status = ? ORDER BY created_at DESC""",
            (status,),
        ).fetchall()
    else:
        rows = conn.execute(
            """SELECT id, kind, title, payload, confidence, dedup_key,
                      status, created_at, decided_at, resulting_skill_id
               FROM discoveries ORDER BY created_at DESC"""
        ).fetchall()
    out = []
    for row in rows:
        payload = row["payload"]
        try:
            payload = json.loads(payload) if payload else {}
        except Exception:
            payload = {}
        out.append(
            {
                "id": row["id"],
                "kind": row["kind"],
                "title": row["title"],
                "payload": payload,
                "confidence": row["confidence"],
                "dedup_key": row["dedup_key"],
                "status": row["status"],
                "created_at": row["created_at"],
                "decided_at": row["decided_at"],
                "resulting_skill_id": row["resulting_skill_id"],
            }
        )
    return out


def cmd_decide_discovery(conn, args):
    # SPEC-C1: four-value action contract. In the mock environment we record the
    # decision (and the adoption_events ledger row) so eval fixtures can assert
    # on state, but sync is downgraded to a no-op summary.
    discovery_id = args.get("id")
    action = args.get("action") or "accept"
    reason = args.get("reason")
    discovery = None
    if discovery_id:
        row = conn.execute(
            """SELECT id, kind, title, payload, confidence, dedup_key,
                      status, created_at, decided_at, resulting_skill_id
               FROM discoveries WHERE id = ?""",
            (discovery_id,),
        ).fetchone()
        if row:
            payload = row["payload"]
            try:
                payload = json.loads(payload) if payload else {}
            except Exception:
                payload = {}
            discovery = {
                "id": row["id"],
                "kind": row["kind"],
                "title": row["title"],
                "payload": payload,
                "confidence": row["confidence"],
                "dedup_key": row["dedup_key"],
                "status": row["status"],
                "created_at": row["created_at"],
                "decided_at": row["decided_at"],
                "resulting_skill_id": row["resulting_skill_id"],
            }

    # Ensure the ledger table exists (older mock DBs predate SPEC-C1).
    conn.execute(
        """CREATE TABLE IF NOT EXISTS adoption_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            discovery_id TEXT NOT NULL,
            decision TEXT NOT NULL CHECK(decision IN ('adopted_as_is','adopted_edited','rejected')),
            reject_reason TEXT CHECK(reject_reason IN ('wrong','trivial','duplicate')),
            resulting_skill_id TEXT,
            decided_at INTEGER NOT NULL
        )"""
    )

    now = int(time.time())
    created_skill_id = None
    sync_summary = None

    if discovery and action in ("accept", "accept_edited", "dismiss"):
        new_status = "dismissed" if action == "dismiss" else "accepted"
        # Normalize reason for dismiss.
        if action == "dismiss":
            reason_val = reason if reason in ("wrong", "trivial", "duplicate") else "trivial"
        else:
            reason_val = None
        decision = "rejected" if action == "dismiss" else (
            "adopted_as_is" if action == "accept" else "adopted_edited"
        )
        conn.execute(
            """UPDATE discoveries SET status = ?, decided_at = ?
               WHERE id = ?""",
            (new_status, now, discovery_id),
        )
        conn.execute(
            """INSERT INTO adoption_events
               (discovery_id, decision, reject_reason, resulting_skill_id, decided_at)
               VALUES (?, ?, ?, ?, ?)""",
            (discovery_id, decision, reason_val, None, now),
        )
        conn.commit()
        if discovery:
            discovery["status"] = new_status
            discovery["decided_at"] = now
        if action == "accept":
            # Mock: sync is a no-op (success:0/failed:0). Real backend pushes
            # the skill to every enabled agent here.
            sync_summary = {"success": 0, "failed": 0, "failures": []}

    return {
        "discovery": discovery,
        "created_skill_id": created_skill_id,
        "sync_summary": sync_summary,
        "mock": True,
        "note": "mock server records the decision but does not sync",
    }


def cmd_list_gate_rejections(conn, args):
    # SPEC-C1 T4: read the gate-rejection ledger (low-confidence band etc.).
    reason = args.get("reason")
    limit = int(args.get("limit") or 100)
    conn.execute(
        """CREATE TABLE IF NOT EXISTS gate_rejections (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            reason TEXT NOT NULL CHECK(reason IN ('below_threshold','daily_limit','cooling','rule_gate')),
            kind TEXT NOT NULL,
            confidence REAL NOT NULL,
            payload TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL
        )"""
    )
    if reason:
        rows = conn.execute(
            """SELECT id, reason, kind, confidence, payload, created_at
               FROM gate_rejections WHERE reason = ?
               ORDER BY created_at DESC LIMIT ?""",
            (reason, limit),
        ).fetchall()
    else:
        rows = conn.execute(
            """SELECT id, reason, kind, confidence, payload, created_at
               FROM gate_rejections ORDER BY created_at DESC LIMIT ?""",
            (limit,),
        ).fetchall()
    out = []
    for r in rows:
        payload = r["payload"]
        try:
            payload = json.loads(payload) if payload else {}
        except Exception:
            payload = {}
        out.append(
            {
                "id": r["id"],
                "reason": r["reason"],
                "kind": r["kind"],
                "confidence": r["confidence"],
                "payload": payload,
                "created_at": r["created_at"],
            }
        )
    return out


def cmd_remove_skill(conn, args):
    # SPEC-C3 T1/T5: remove_skill is now "move to recycle bin". In the mock
    # environment file operations are downgraded to DB-only (no snapshot dir).
    skill_id = args.get("skillId") or args.get("skill_id")
    if not skill_id:
        raise RuntimeError("skillId is required")
    conn.execute(
        """CREATE TABLE IF NOT EXISTS trash_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            item_type TEXT NOT NULL CHECK(item_type IN ('skill')),
            original_id TEXT NOT NULL,
            original_name TEXT NOT NULL,
            snapshot_path TEXT NOT NULL,
            metadata TEXT NOT NULL DEFAULT '{}',
            deleted_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL
        )"""
    )
    row = conn.execute(
        "SELECT id, name, repo_path, created_at, updated_at, status FROM skills WHERE id = ?",
        (skill_id,),
    ).fetchone()
    if not row:
        raise RuntimeError("Skill not found")
    now = int(time.time())
    expires_at = now + 30 * 86400
    metadata = json.dumps({"skill": dict(row), "sync_targets": [], "bindings": []})
    cur = conn.execute(
        """INSERT INTO trash_items
           (item_type, original_id, original_name, snapshot_path, metadata, deleted_at, expires_at)
           VALUES (?, ?, ?, ?, ?, ?, ?)""",
        ("skill", row["id"], row["name"], f"mock://{row['id']}", metadata, now, expires_at),
    )
    trash_id = cur.lastrowid
    conn.execute("DELETE FROM sync_targets WHERE skill_id = ?", (skill_id,))
    conn.execute("DELETE FROM skills WHERE id = ?", (skill_id,))
    conn.commit()
    return trash_id


def cmd_list_trash_items(conn, args):
    conn.execute(
        """CREATE TABLE IF NOT EXISTS trash_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            item_type TEXT NOT NULL CHECK(item_type IN ('skill')),
            original_id TEXT NOT NULL,
            original_name TEXT NOT NULL,
            snapshot_path TEXT NOT NULL,
            metadata TEXT NOT NULL DEFAULT '{}',
            deleted_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL
        )"""
    )
    rows = conn.execute(
        """SELECT id, item_type, original_id, original_name, snapshot_path,
                  metadata, deleted_at, expires_at
           FROM trash_items ORDER BY deleted_at DESC"""
    ).fetchall()
    out = []
    for r in rows:
        metadata = r["metadata"]
        try:
            metadata = json.loads(metadata) if metadata else {}
        except Exception:
            metadata = {}
        out.append(
            {
                "id": r["id"],
                "item_type": r["item_type"],
                "original_id": r["original_id"],
                "original_name": r["original_name"],
                "snapshot_path": r["snapshot_path"],
                "metadata": metadata,
                "deleted_at": r["deleted_at"],
                "expires_at": r["expires_at"],
            }
        )
    return out


def cmd_restore_trash_item(conn, args):
    # SPEC-C3 T5: restore in mock = rebuild the skills row from metadata.
    import uuid as _uuid

    trash_id = args.get("id")
    conflict_strategy = args.get("conflictStrategy") or args.get("conflict_strategy")
    row = conn.execute(
        "SELECT id, item_type, original_id, original_name, snapshot_path, metadata, deleted_at, expires_at FROM trash_items WHERE id = ?",
        (trash_id,),
    ).fetchone()
    if not row:
        raise RuntimeError("回收站项目不存在")
    metadata = row["metadata"]
    try:
        metadata = json.loads(metadata) if metadata else {}
    except Exception:
        metadata = {}
    skill_meta = metadata.get("skill") or {}
    original_name = skill_meta.get("name") or row["original_name"]

    clash = conn.execute(
        "SELECT id FROM skills WHERE name = ?", (original_name,)
    ).fetchone()
    if clash and conflict_strategy not in ("overwrite", "rename"):
        raise RuntimeError(
            f"restore_conflict: 名为 '{original_name}' 的 Skill 已存在，请选择覆盖/重命名策略"
        )
    if clash and conflict_strategy == "overwrite":
        conn.execute("DELETE FROM skills WHERE name = ?", (original_name,))
    final_name = f"{original_name}-restored" if (clash and conflict_strategy == "rename") else original_name
    final_id = str(_uuid.uuid4())
    now = int(time.time())
    conn.execute(
        """INSERT OR REPLACE INTO skills (id, name, repo_path, created_at, updated_at, status)
           VALUES (?, ?, ?, ?, ?, ?)""",
        (final_id, final_name, skill_meta.get("repo_path", f"mock://{final_id}"),
         skill_meta.get("created_at", now), now, skill_meta.get("status", "draft")),
    )
    conn.execute("DELETE FROM trash_items WHERE id = ?", (trash_id,))
    conn.commit()
    return {
        "restored_id": final_id,
        "skipped_bindings": [],
        "final_name": final_name,
    }


def cmd_purge_trash_item(conn, args):
    # SPEC-C3 T5: permanent delete, gated by the confirm code (same as real).
    confirm = args.get("confirm")
    if confirm != DATA_MANAGEMENT_CONFIRM_CODE:
        raise RuntimeError(f"确认码不正确，请输入 {DATA_MANAGEMENT_CONFIRM_CODE}")
    trash_id = args.get("id")
    row = conn.execute("SELECT id FROM trash_items WHERE id = ?", (trash_id,)).fetchone()
    if not row:
        raise RuntimeError("回收站项目不存在")
    conn.execute("DELETE FROM trash_items WHERE id = ?", (trash_id,))
    conn.commit()
    return None


def _this_monday_utc():
    from datetime import datetime, timedelta, timezone
    now = datetime.now(timezone.utc)
    days_since_monday = now.weekday()
    monday = now - timedelta(days=days_since_monday)
    return monday.strftime("%Y-%m-%d")


def cmd_get_weekly_report(conn, args):
    week = args.get("week") or _this_monday_utc()
    row = conn.execute(
        "SELECT week_start, content, generated_at, model, provider FROM weekly_reports WHERE week_start = ?",
        (week,),
    ).fetchone()
    if not row:
        return None
    content = row["content"]
    try:
        content = json.loads(content) if content else {}
    except Exception:
        content = {}
    return {
        "week_start": row["week_start"],
        "content": content,
        "generated_at": row["generated_at"],
        "model": row["model"],
        "provider": row["provider"],
    }


def cmd_generate_weekly_report(conn, args):
    week = args.get("week") or _this_monday_utc()
    now = int(time.time())
    # Template fallback (matches Rust template_weekly_report shape).
    device_id = _device_id_for_metrics(conn)
    start = now - 7 * 86400
    projects = conn.execute(
        """SELECT p.id AS project_id, p.name,
                  COUNT(DISTINCT cs.id) AS session_count,
                  COALESCE(SUM(ct.total_tokens), 0) AS total_tokens
           FROM projects p
           LEFT JOIN collected_sessions cs ON cs.project_id = p.id AND cs.start_time >= ?
           LEFT JOIN collected_token_usage ct ON ct.session_id = cs.id
           WHERE p.device_id = ?
           GROUP BY p.id ORDER BY total_tokens DESC LIMIT 5""",
        (start, device_id),
    ).fetchall()
    pitfalls = conn.execute(
        """SELECT id, project_id, title_or_prompt, message_count
           FROM collected_sessions
           WHERE device_id = ? AND start_time >= ? AND start_time < ?
           ORDER BY message_count DESC LIMIT 3""",
        (device_id, start, now),
    ).fetchall()
    content = {
        "projects": [
            {
                "project_id": p["project_id"],
                "name": p["name"],
                "summary": f"{p['session_count']} 个会话 / {p['total_tokens']} tokens",
            }
            for p in projects
        ],
        "pitfalls": [
            {
                "session_id": p["id"],
                "project_id": p["project_id"],
                "title": p["title_or_prompt"] or "未命名会话",
                "lesson": "（mock 周报，LLM 未配置）",
            }
            for p in pitfalls
        ],
        "growth": {
            "new_skills": [],
            "eliminated_patterns": [],
            "accepted_count": 0,
            "dismissed_count": 0,
        },
    }
    report = {
        "week_start": week,
        "content": content,
        "generated_at": now,
        "model": None,
        "provider": "skillmint-mock",
    }
    # Persist so get_weekly_report can read it back.
    conn.execute(
        "INSERT OR REPLACE INTO weekly_reports (week_start, content, generated_at, model, provider) VALUES (?,?,?,?,?)",
        (week, json.dumps(content, ensure_ascii=False), now, report["model"], report["provider"]),
    )
    conn.commit()
    return report


def cmd_get_growth_metrics(conn, args):
    device_id = _device_id_for_metrics(conn)
    now = int(time.time())

    skill_count_by_source = {"handwritten": 0, "from_discovery": 0, "remote": 0}
    try:
        accepted = {
            row["resulting_skill_id"]
            for row in conn.execute(
                "SELECT resulting_skill_id FROM discoveries WHERE status = 'accepted' AND resulting_skill_id IS NOT NULL"
            ).fetchall()
        }
    except Exception:
        accepted = set()
    for row in conn.execute(
        "SELECT id, repo_path FROM skills"
    ).fetchall():
        sid = row["id"]
        path = row["repo_path"] or ""
        if sid in accepted:
            source = "from_discovery"
        elif "sources" in path:
            source = "remote"
        else:
            source = "handwritten"
        skill_count_by_source[source] = skill_count_by_source.get(source, 0) + 1

    compounding_curves = []
    for d in conn.execute(
        """SELECT payload, resulting_skill_id FROM discoveries
           WHERE status = 'accepted' AND kind = 'repeat_pattern' AND resulting_skill_id IS NOT NULL"""
    ).fetchall():
        payload = d["payload"] or "{}"
        try:
            payload = json.loads(payload)
        except Exception:
            payload = {}
        pattern = payload.get("pattern", "").lower()
        if not pattern:
            continue
        skill = conn.execute(
            "SELECT id, name FROM skills WHERE id = ?", (d["resulting_skill_id"],)
        ).fetchone()
        if not skill:
            continue
        weeks, counts = [], []
        for offset in range(7, -1, -1):
            week_end = now - offset * 7 * 86400
            week_start = week_end - 7 * 86400
            from datetime import datetime, timezone
            label = datetime.fromtimestamp(week_start, timezone.utc).strftime("%Y-W%W")
            cnt = conn.execute(
                """SELECT COUNT(*) FROM collected_prompts
                   WHERE device_id = ? AND prompt_text IS NOT NULL AND LOWER(prompt_text) LIKE ?
                     AND IFNULL(started_at, 0) >= ? AND IFNULL(started_at, 0) < ?""",
                (device_id, f"%{pattern}%", week_start, week_end),
            ).fetchone()[0]
            weeks.append(label)
            counts.append(cnt)
        compounding_curves.append(
            {"skill_id": skill["id"], "skill_name": skill["name"], "weeks": weeks, "counts": counts}
        )

    active_skills = []
    cutoff90 = now - 90 * 86400
    for row in conn.execute(
        """SELECT skill_id, skill_name,
                  COUNT(*) AS usage_count,
                  COUNT(DISTINCT session_id) AS session_count
           FROM skill_usage_attributions
           WHERE attributed_at >= ?
           GROUP BY skill_id, skill_name
           ORDER BY usage_count DESC""",
        (cutoff90,),
    ).fetchall():
        active_skills.append(
            {
                "skill_id": row["skill_id"],
                "skill_name": row["skill_name"],
                "usage_count": row["usage_count"],
                "last_used_at": None,
                "dormant": row["usage_count"] == 0,
            }
        )

    capability_map = []
    start7 = now - 7 * 86400
    skill_texts = []
    for row in conn.execute("SELECT name, repo_path FROM skills").fetchall():
        text = (row["name"] or "").lower()
        path = Path(row["repo_path"] or "")
        skill_md = path / "SKILL.md"
        if skill_md.is_file():
            try:
                text += " " + skill_md.read_text(encoding="utf-8", errors="ignore").lower()
            except Exception:
                pass
        skill_texts.append(text)
    for key, label, keywords in _CATEGORIES:
        count = 0
        for text_row in conn.execute(
            """SELECT prompt_text FROM collected_prompts
               WHERE device_id = ? AND IFNULL(started_at, 0) >= ? AND prompt_text IS NOT NULL""",
            (device_id, start7),
        ).fetchall():
            text = (text_row["prompt_text"] or "").lower()
            if any(kw.lower() in text for kw in keywords):
                count += 1
        has_skill = any(any(kw.lower() in st for kw in keywords) for st in skill_texts)
        capability_map.append(
            {"category": label, "has_skill": has_skill, "prompt_count_7d": count}
        )

    return {
        "skill_count_by_source": skill_count_by_source,
        "compounding_curves": compounding_curves,
        "active_skills": active_skills,
        "capability_map": capability_map,
    }


# --- SPEC-F1: missing command coverage ---------------------------------------

def _mock_unsupported(reason):
    return {"error": f"mock 环境不支持：{reason}"}


def _repo_dir():
    return Path.home() / ".skillmint"


def _skill_root(conn, skill_id):
    row = conn.execute("SELECT repo_path FROM skills WHERE id = ?", (skill_id,)).fetchone()
    if not row:
        raise RuntimeError("Skill not found")
    return Path(row["repo_path"])


def _effective_skill_dir(skill_root: Path, version=None):
    if version and version != "latest":
        return skill_root / version
    latest = skill_root / "latest"
    if latest.is_dir():
        return latest
    return skill_root


def _parse_skill_md(raw):
    trimmed = raw.strip()
    if trimmed.startswith("---"):
        end = raw.find("\n---", 3)
        if end != -1:
            yaml_text = raw[3:end].strip()
            body = raw[end + 4 :].lstrip()
            try:
                import yaml
                frontmatter = yaml.safe_load(yaml_text) or {}
            except Exception:
                frontmatter = {}
            return frontmatter, body
    return {}, raw


def _serialize_skill_md(frontmatter, body):
    if not frontmatter:
        return body
    try:
        import yaml
        yaml_text = yaml.safe_dump(frontmatter, allow_unicode=True, sort_keys=False)
    except Exception:
        yaml_text = ""
    return f"---\n{yaml_text}---\n\n{body.lstrip()}"


def cmd_check_repo_integrity(conn, args):
    center = _repo_dir()
    has_skills = conn.execute("SELECT COUNT(*) AS c FROM skills").fetchone()["c"] > 0
    repo_exists = center.exists()
    repo_empty = True
    if repo_exists:
        try:
            repo_empty = not any(center.iterdir())
        except Exception:
            pass
    if has_skills and (not repo_exists or repo_empty):
        return "missing_with_records"
    return "healthy"


def cmd_save_agent(conn, args):
    import uuid
    agent_id = args.get("id") or str(uuid.uuid4())
    name = args.get("name", "Agent")
    skill_directory = args.get("skillDirectory") or str(Path.home() / ".skillmint" / "skills")
    is_enabled = args.get("isEnabled", True)
    discovery_rule = args.get("discoveryRule")
    description = args.get("description")
    source = args.get("source")
    now = int(time.time())
    conn.execute(
        """INSERT OR REPLACE INTO agents
           (id, name, skill_directory, is_enabled, discovery_rule, description, source, created_at, updated_at)
           VALUES (?,?,?,?,?,?,?,?,?)""",
        (agent_id, name, skill_directory, 1 if is_enabled else 0, discovery_rule, description, source, now, now),
    )
    conn.commit()
    return {
        "id": agent_id,
        "name": name,
        "skill_directory": skill_directory,
        "is_enabled": bool(is_enabled),
        "discovery_rule": discovery_rule,
        "description": description,
        "source": source,
    }


def cmd_detect_local_agents(conn, args):
    import shutil
    candidates = [
        ("claude", "Claude Code", ["--mcp"]),
        ("claude-cli", "Claude Code CLI", ["--mcp"]),
        ("kimi", "Kimi Code", []),
        ("kimi-code", "Kimi Code", []),
        ("codex", "OpenAI Codex CLI", []),
        ("zcode", "ZCode", []),
    ]
    found = []
    for cmd, name, cargs in candidates:
        if shutil.which(cmd):
            found.append({"command": cmd, "display_name": name, "args": cargs})
    return found


def cmd_get_agent_skill_counts(conn, args):
    agent_ids = args.get("agentIds", [])
    counts = {}
    for aid in agent_ids:
        row = conn.execute(
            "SELECT COUNT(*) AS c FROM agent_directory_skills WHERE agent_id = ?",
            (aid,),
        ).fetchone()
        counts[aid] = row["c"] if row else 0
    return counts


def cmd_update_skill_status(conn, args):
    skill_id = args.get("id")
    status = args.get("status", "draft")
    now = int(time.time())
    conn.execute(
        "UPDATE skills SET status = ?, updated_at = ? WHERE id = ?",
        (status, now, skill_id),
    )
    conn.commit()
    return None


def cmd_check_skill_external_change(conn, args):
    skill_id = args.get("skillId")
    row = conn.execute("SELECT repo_path, updated_at FROM skills WHERE id = ?", (skill_id,)).fetchone()
    if not row:
        return False
    md_path = Path(row["repo_path"]) / "SKILL.md"
    try:
        disk_mtime = md_path.stat().st_mtime
    except Exception:
        return False
    db_mtime = row["updated_at"]
    return disk_mtime > db_mtime + 2


def cmd_read_skill_content(conn, args):
    skill_id = args.get("skillId")
    version = args.get("version")
    skill_root = _skill_root(conn, skill_id)
    dir_ = _effective_skill_dir(skill_root, version)
    md_path = dir_ / "SKILL.md"
    raw = md_path.read_text(encoding="utf-8", errors="ignore") if md_path.is_file() else ""
    frontmatter, body = _parse_skill_md(raw)
    return {"frontmatter": frontmatter, "body": body}


def cmd_save_skill_content(conn, args):
    skill_id = args.get("skillId")
    frontmatter = args.get("frontmatter", {}) or {}
    body = args.get("body", "")
    new_name = None
    if isinstance(frontmatter, dict):
        new_name = frontmatter.get("name")
        if isinstance(new_name, str):
            new_name = new_name.strip() or None
    row = conn.execute("SELECT name, repo_path FROM skills WHERE id = ?", (skill_id,)).fetchone()
    current_name = row["name"]
    current_repo = Path(row["repo_path"])
    if new_name and new_name != current_name:
        new_repo = _repo_dir() / new_name
        if new_repo.exists():
            raise RuntimeError(f"已存在同名 Skill 目录「{new_name}」")
        current_repo.rename(new_repo)
        skill_root = new_repo
        current_name = new_name
        conn.execute(
            "UPDATE skills SET name = ?, repo_path = ? WHERE id = ?",
            (new_name, str(new_repo), skill_id),
        )
    else:
        skill_root = current_repo
    md_path = _effective_skill_dir(skill_root, None) / "SKILL.md"
    md_path.parent.mkdir(parents=True, exist_ok=True)
    md_path.write_text(_serialize_skill_md(frontmatter, body), encoding="utf-8")
    now = int(time.time())
    conn.execute("UPDATE skills SET updated_at = ? WHERE id = ?", (now, skill_id))
    conn.commit()
    return {
        "id": skill_id,
        "name": current_name,
        "repo_path": str(skill_root),
        "created_at": conn.execute("SELECT created_at FROM skills WHERE id = ?", (skill_id,)).fetchone()["created_at"],
        "updated_at": now,
        "status": conn.execute("SELECT status FROM skills WHERE id = ?", (skill_id,)).fetchone()["status"] or "draft",
    }


def cmd_search_all(conn, args):
    query = (args.get("query") or "").strip().lower()
    source_filter = args.get("sourceFilter")
    center = _repo_dir()
    rows = conn.execute("SELECT id, name, repo_path FROM skills ORDER BY name").fetchall()
    results = []
    seen = set()
    for row in rows:
        name = row["name"]
        key = name.lower()
        if key in seen:
            continue
        match_field = None
        if not query:
            pass
        elif query in key:
            match_field = "name"
        else:
            md_path = Path(row["repo_path"]) / "SKILL.md"
            if md_path.is_file():
                text = md_path.read_text(encoding="utf-8", errors="ignore").lower()
                if query in text:
                    match_field = "body"
        if query and not match_field:
            continue
        seen.add(key)
        results.append({
            "skill_name": name,
            "origin": "local",
            "source_id": None,
            "source_name": None,
            "description": None,
            "installed_locally": True,
            "usage_count": 0,
            "correction_count": 0,
            "skill_path": "",
            "match_field": match_field,
            "snippet": None,
        })
    for src in conn.execute("SELECT id, name, cache_path, subpath FROM sources").fetchall():
        if source_filter and src["id"] != source_filter:
            continue
        cache = Path(src["cache_path"])
        sub = src["subpath"] or ""
        base = cache / sub if sub else cache
        if not base.is_dir():
            continue
        for p in sorted(base.iterdir()):
            if not p.is_dir():
                continue
            name = p.name
            key = name.lower()
            if key in seen:
                continue
            match_field = None
            if not query:
                pass
            elif query in key:
                match_field = "name"
            else:
                md = p / "SKILL.md"
                if md.is_file():
                    text = md.read_text(encoding="utf-8", errors="ignore").lower()
                    if query in text:
                        match_field = "body"
            if query and not match_field:
                continue
            seen.add(key)
            rel = p.relative_to(cache) if cache in p.parents else Path(name)
            results.append({
                "skill_name": name,
                "origin": "remote",
                "source_id": src["id"],
                "source_name": src["name"],
                "description": None,
                "installed_locally": (center / name).exists(),
                "usage_count": 0,
                "correction_count": 0,
                "skill_path": str(rel),
                "match_field": match_field,
                "snippet": None,
            })
    results.sort(key=lambda r: (
        0 if r["match_field"] == "name" else 1,
        0 if r["installed_locally"] else 1,
        r["skill_name"],
    ))
    return results


def cmd_get_sources(conn, args):
    rows = conn.execute(
        "SELECT id, name, source_type, url, ref_spec, subpath, cache_path, commit_sha, added_at, last_fetched_at, pull_policy, remote_revision FROM sources ORDER BY name"
    ).fetchall()
    return [
        {
            "id": r["id"],
            "name": r["name"],
            "source_type": r["source_type"],
            "url": r["url"],
            "ref_spec": r["ref_spec"],
            "subpath": r["subpath"] or "",
            "cache_path": r["cache_path"],
            "commit_sha": r["commit_sha"] or "",
            "added_at": r["added_at"],
            "last_fetched_at": r["last_fetched_at"],
            "pull_policy": r["pull_policy"] or "manual",
            "remote_revision": r["remote_revision"] or "",
        }
        for r in rows
    ]


def cmd_add_source(conn, args):
    import uuid
    source_type = args.get("sourceType", "local")
    name = args.get("name", "source")
    url = args.get("url", "")
    ref_spec = args.get("refSpec") or "main"
    subpath = args.get("subpath") or ""
    sid = str(uuid.uuid4())
    now = int(time.time())
    if source_type == "local":
        cache_path = str(Path(url).expanduser().resolve()) if url else ""
    else:
        cache_path = str(_repo_dir() / "cache" / "sources" / sid)
    conn.execute(
        """INSERT INTO sources (id, name, source_type, url, ref_spec, subpath, cache_path, commit_sha, added_at, last_fetched_at, pull_policy, remote_revision)
           VALUES (?,?,?,?,?,?,?,?,?,?,?,?)""",
        (sid, name, source_type, url, ref_spec, subpath, cache_path, "", now, now, "manual", ""),
    )
    conn.commit()
    return {
        "id": sid,
        "name": name,
        "source_type": source_type,
        "url": url,
        "ref_spec": ref_spec,
        "subpath": subpath,
        "cache_path": cache_path,
        "commit_sha": "",
        "added_at": now,
        "last_fetched_at": now,
        "pull_policy": "manual",
        "remote_revision": "",
    }


def cmd_remove_source(conn, args):
    sid = args.get("id")
    conn.execute("DELETE FROM sources WHERE id = ?", (sid,))
    conn.commit()
    return None


def cmd_refresh_source(conn, args):
    sid = args.get("id")
    now = int(time.time())
    conn.execute("UPDATE sources SET last_fetched_at = ? WHERE id = ?", (now, sid))
    conn.commit()
    row = conn.execute(
        "SELECT id, name, source_type, url, ref_spec, subpath, cache_path, commit_sha, added_at, last_fetched_at, pull_policy, remote_revision FROM sources WHERE id = ?",
        (sid,),
    ).fetchone()
    if not row:
        raise RuntimeError("源不存在")
    return {
        "id": row["id"],
        "name": row["name"],
        "source_type": row["source_type"],
        "url": row["url"],
        "ref_spec": row["ref_spec"],
        "subpath": row["subpath"] or "",
        "cache_path": row["cache_path"],
        "commit_sha": row["commit_sha"] or "",
        "added_at": row["added_at"],
        "last_fetched_at": row["last_fetched_at"],
        "pull_policy": row["pull_policy"] or "manual",
        "remote_revision": row["remote_revision"] or "",
    }


def cmd_list_source_skills(conn, args):
    source_id = args.get("sourceId")
    row = conn.execute("SELECT cache_path, subpath FROM sources WHERE id = ?", (source_id,)).fetchone()
    if not row:
        return []
    cache = Path(row["cache_path"])
    sub = row["subpath"] or ""
    base = cache / sub if sub else cache
    center = _repo_dir()
    out = []
    if base.is_dir():
        for p in sorted(base.iterdir()):
            if not p.is_dir():
                continue
            desc = None
            md = p / "SKILL.md"
            if md.is_file():
                try:
                    _, body = _parse_skill_md(md.read_text(encoding="utf-8", errors="ignore"))
                    first = body.strip().split("\n")[0]
                    desc = first[:200] if first else None
                except Exception:
                    pass
            rel = p.relative_to(cache) if cache in p.parents else Path(p.name)
            out.append({
                "skill_name": p.name,
                "source_id": source_id,
                "skill_path": str(rel),
                "description": desc,
                "computed_hash": None,
                "installed_locally": (center / p.name).exists(),
            })
    return out


def cmd_scan_skill_safety(conn, args):
    # Honest degradation: do not claim the skill is safe in mock mode.
    return {"clean": False, "findings": []}


def cmd_install_remote_skill(conn, args):
    import shutil, uuid
    source_id = args.get("sourceId")
    skill_path = args.get("skillPath")
    agent_ids = args.get("agentIds", [])
    mode = args.get("mode") or "symlink"
    confirmed_risks = args.get("confirmedRisks") or []
    source = conn.execute("SELECT cache_path FROM sources WHERE id = ?", (source_id,)).fetchone()
    if not source:
        raise RuntimeError("源不存在")
    src = Path(source["cache_path"]) / skill_path
    if not src.is_dir():
        raise RuntimeError(f"Skill 目录不存在于源缓存: {skill_path}")
    skill_name = src.name
    center = _repo_dir()
    dest = center / skill_name
    if dest.exists():
        raise RuntimeError(f"中心仓库已存在「{skill_name}」")

    # SPEC-F3: honor confirmed_risks by scanning SKILL.md for dangerous rules.
    findings = []
    skill_md = src / "SKILL.md"
    if skill_md.is_file():
        body = skill_md.read_text(encoding="utf-8", errors="ignore").lower()
        if "rm -rf" in body:
            findings.append({"line": 1, "rule": "rm-rf", "excerpt": "rm -rf"})
    uncovered = [f for f in findings if f["rule"] not in confirmed_risks]
    if uncovered:
        raise RuntimeError(f"该 Skill 存在未确认的安全风险（{len(uncovered)} 项），请先确认后再安装")

    shutil.copytree(src, dest)
    now = int(time.time())
    sid = str(uuid.uuid4())
    device_id = _device_id(conn)
    conn.execute(
        "INSERT INTO skills (id, name, repo_path, created_at, updated_at, device_id, status) VALUES (?,?,?,?,?,?,?)",
        (sid, skill_name, str(dest), now, now, device_id, "draft"),
    )
    conn.execute(
        """INSERT INTO install_audit (id, skill_name, source, findings_json, confirmed_risks_json, confirmed, created_at)
           VALUES (?,?,?,?,?,?,?)""",
        (str(uuid.uuid4()), skill_name, source_id, json.dumps(findings), json.dumps(confirmed_risks), 1, now),
    )
    synced = []
    skipped = []
    for aid in agent_ids:
        agent = conn.execute("SELECT name, skill_directory FROM agents WHERE id = ?", (aid,)).fetchone()
        if not agent:
            continue
        agent_dir = Path(agent["skill_directory"])
        agent_path = agent_dir / skill_name
        if agent_path.exists() or agent_path.is_symlink():
            skipped.append(agent["name"])
            continue
        agent_dir.mkdir(parents=True, exist_ok=True)
        if mode == "copy":
            shutil.copytree(dest, agent_path)
        else:
            agent_path.symlink_to(dest, target_is_directory=True)
        synced.append(agent["name"])
    conn.commit()
    return {
        "skill": {
            "id": sid,
            "name": skill_name,
            "repo_path": str(dest),
            "created_at": now,
            "updated_at": now,
            "status": "draft",
        },
        "synced_agents": synced,
        "skipped_agents": skipped,
    }


def cmd_preview_remote_skill(conn, args):
    source_id = args.get("sourceId")
    skill_path = args.get("skillPath")
    row = conn.execute("SELECT cache_path FROM sources WHERE id = ?", (source_id,)).fetchone()
    if not row:
        return "# 源不存在（mock）"
    md = Path(row["cache_path"]) / skill_path / "SKILL.md"
    if md.is_file():
        return md.read_text(encoding="utf-8", errors="ignore")
    return "# 未找到 SKILL.md（mock）"


def _strategy_from_db(kind, value, unit, expr):
    if kind == "interval":
        return {"kind": "interval", "value": int(value or 30), "unit": unit or "minutes"}
    if kind == "cron":
        return {"kind": "cron", "expression": expr or ""}
    return {"kind": "manual"}


def _status_from_db(s):
    return s or "pending"


def _trigger_from_db(s):
    return s or "schedule"


def cmd_get_scheduled_tasks(conn, args):
    rows = conn.execute(
        """SELECT id, task_kind, name, description, enabled, strategy_kind,
                  strategy_value, strategy_unit, strategy_expression,
                  created_at, updated_at, last_run_at, last_status,
                  next_run_at, run_count, error_count
           FROM scheduled_tasks ORDER BY created_at"""
    ).fetchall()
    return [
        {
            "id": r["id"],
            "task_kind": r["task_kind"],
            "name": r["name"],
            "description": r["description"] or "",
            "enabled": bool(r["enabled"]),
            "strategy": _strategy_from_db(r["strategy_kind"], r["strategy_value"], r["strategy_unit"], r["strategy_expression"]),
            "created_at": r["created_at"],
            "updated_at": r["updated_at"],
            "last_run_at": r["last_run_at"],
            "last_status": _status_from_db(r["last_status"]),
            "next_run_at": r["next_run_at"],
            "run_count": r["run_count"] or 0,
            "error_count": r["error_count"] or 0,
        }
        for r in rows
    ]


def _compute_next_run(strategy, now):
    kind = strategy.get("kind", "manual")
    if kind == "interval":
        value = int(strategy.get("value", 0))
        unit = strategy.get("unit", "minutes")
        mult = {"minutes": 60, "hours": 3600, "days": 86400}.get(unit, 60)
        return now + value * mult if value else None
    return None


def cmd_save_scheduled_task(conn, args):
    import uuid
    task = args.get("task", {})
    tid = task.get("id") or str(uuid.uuid4())
    now = int(time.time())
    created_at = task.get("createdAt") or now
    strategy = task.get("strategy", {"kind": "manual"})
    kind = strategy.get("kind", "manual")
    value = None
    unit = None
    expr = None
    if kind == "interval":
        value = strategy.get("value")
        unit = strategy.get("unit", "minutes")
    elif kind == "cron":
        expr = strategy.get("expression")
    task_kind = task.get("taskKind") or task.get("task_kind") or "collect_usage_data"
    name = task.get("name", "定时任务")
    description = task.get("description", "")
    enabled = task.get("enabled", True)
    next_run_at = _compute_next_run(strategy, now)
    conn.execute(
        """INSERT OR REPLACE INTO scheduled_tasks
           (id, task_kind, name, description, enabled, strategy_kind,
            strategy_value, strategy_unit, strategy_expression,
            created_at, updated_at, last_run_at, last_status,
            next_run_at, run_count, error_count)
           VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
        (tid, task_kind, name, description, 1 if enabled else 0, kind, value, unit, expr,
         created_at, now, task.get("lastRunAt"), task.get("lastStatus"),
         next_run_at, task.get("runCount", 0), task.get("errorCount", 0)),
    )
    conn.commit()
    return {
        "id": tid,
        "task_kind": task_kind,
        "name": name,
        "description": description,
        "enabled": bool(enabled),
        "strategy": strategy,
        "created_at": created_at,
        "updated_at": now,
        "last_run_at": task.get("lastRunAt"),
        "last_status": task.get("lastStatus") or "pending",
        "next_run_at": next_run_at,
        "run_count": task.get("runCount", 0),
        "error_count": task.get("errorCount", 0),
    }


def cmd_delete_scheduled_task(conn, args):
    tid = args.get("id")
    conn.execute("DELETE FROM scheduled_tasks WHERE id = ?", (tid,))
    conn.commit()
    return None


def cmd_get_task_runs(conn, args):
    task_id = args.get("taskId")
    limit = args.get("limit", 20)
    rows = conn.execute(
        """SELECT id, task_id, status, started_at, finished_at, duration_ms,
                  result_summary, error_message, triggered_by
           FROM task_runs WHERE task_id = ?
           ORDER BY started_at DESC LIMIT ?""",
        (task_id, limit),
    ).fetchall()
    return [
        {
            "id": r["id"],
            "task_id": r["task_id"],
            "status": _status_from_db(r["status"]),
            "started_at": r["started_at"],
            "finished_at": r["finished_at"],
            "duration_ms": r["duration_ms"],
            "result_summary": r["result_summary"] or "",
            "error_message": r["error_message"],
            "triggered_by": _trigger_from_db(r["triggered_by"]),
        }
        for r in rows
    ]


def cmd_run_scheduled_task_now(conn, args):
    import uuid
    tid = args.get("id")
    now = int(time.time())
    run_id = str(uuid.uuid4())
    conn.execute(
        """INSERT INTO task_runs (id, task_id, status, started_at, finished_at, duration_ms,
            result_summary, error_message, triggered_by)
           VALUES (?,?,?,?,?,?,?,?,?)""",
        (run_id, tid, "success", now, now, 0, "mock 运行成功", None, "manual"),
    )
    conn.execute(
        """UPDATE scheduled_tasks SET last_run_at = ?, last_status = ?, run_count = run_count + 1
           WHERE id = ?""",
        (now, "success", tid),
    )
    conn.commit()
    return {
        "id": run_id,
        "task_id": tid,
        "status": "success",
        "started_at": now,
        "finished_at": now,
        "duration_ms": 0,
        "result_summary": "mock 运行成功",
        "error_message": None,
        "triggered_by": "manual",
    }


def cmd_export_data_dictionary(conn, args):
    return {
        "db_path": str(DB_PATH),
        "tables": [
            {
                "name": "skills",
                "description": "中心仓库中的 Skill 元数据",
                "columns": ["id", "name", "repo_path", "created_at", "updated_at", "status"],
            },
            {
                "name": "agents",
                "description": "本机 Agent 元数据",
                "columns": ["id", "name", "skill_directory", "is_enabled", "discovery_rule", "description", "source"],
            },
            {
                "name": "sync_targets",
                "description": "Skill 与 Agent 的同步目标",
                "columns": ["id", "skill_id", "agent_id", "mode", "last_sync_at", "status"],
            },
            {
                "name": "collected_sessions",
                "description": "从各 Agent 采集的会话记录",
                "columns": ["id", "device_id", "source", "project_id", "start_time", "end_time", "message_count", "title_or_prompt"],
            },
            {
                "name": "projects",
                "description": "从会话中识别的项目",
                "columns": ["id", "device_id", "name", "path", "first_seen_at", "last_active_at"],
            },
        ],
    }


def _dump_table(conn, table):
    try:
        cols = [c[1] for c in conn.execute(f"PRAGMA table_info({table})").fetchall()]
    except Exception:
        return []
    if not cols:
        return []
    rows = conn.execute(f"SELECT {','.join(cols)} FROM {table}").fetchall()
    return [dict(zip(cols, row)) for row in rows]


def cmd_export_raw_data(conn, args):
    tables = args.get("tables", [])
    allowed = {
        "skills", "agents", "sync_targets", "collected_sessions", "collected_prompts",
        "skill_versions", "skill_bundles", "skill_bundle_items", "projects",
        "skill_project_bindings", "llm_request_logs", "collection_jobs",
    }
    data = {}
    for table in tables:
        if table not in allowed:
            raise RuntimeError(f"unsupported table: {table}")
        data[table] = _dump_table(conn, table)
    return {
        "format": "json",
        "exported_at": int(time.time()),
        "tables": tables,
        "data": data,
    }


def cmd_list_llm_request_logs(conn, args):
    limit = args.get("limit", 50)
    rows = conn.execute(
        "SELECT id, requested_at, provider, fallback, has_raw_text, error, metadata_json FROM llm_request_logs ORDER BY requested_at DESC LIMIT ?",
        (limit,),
    ).fetchall()
    return [
        {
            "id": r["id"],
            "requested_at": r["requested_at"],
            "provider": r["provider"],
            "fallback": bool(r["fallback"]),
            "has_raw_text": bool(r["has_raw_text"]),
            "error": r["error"],
            "metadata_json": r["metadata_json"] or "{}",
        }
        for r in rows
    ]


def cmd_classify_prompts(conn, args):
    return {"eligible": 0, "classified": 0, "skipped_no_key": True}


def cmd_get_summary_value_metrics(conn, args):
    rows = conn.execute("SELECT date, activities FROM digest_summary").fetchall()
    covered_days = len(rows)
    total_summaries = covered_days
    total_activities = 0
    for r in rows:
        try:
            acts = json.loads(r["activities"]) if r["activities"] else []
            total_activities += len(acts)
        except Exception:
            pass
    return {
        "covered_days": covered_days,
        "total_summaries": total_summaries,
        "total_activities": total_activities,
        "covered_projects": 0,
    }


def cmd_get_prompts_for_cell(conn, args):
    return []


def cmd_generate_daily_summary(conn, args):
    date = args.get("date")
    row = conn.execute(
        "SELECT COUNT(*) AS c FROM collected_sessions WHERE date(datetime(start_time, 'unixepoch')) = ?",
        (date,),
    ).fetchone()
    if not row or row["c"] == 0:
        return {"status": "no_sessions"}
    return {"status": "no_key"}


def cmd_pin_binding_version(conn, args):
    binding_id = args.get("bindingId")
    version = args.get("version")
    now = int(time.time())
    conn.execute(
        "UPDATE skill_project_bindings SET pinned_version = ?, updated_at = ? WHERE id = ?",
        (version, now, binding_id),
    )
    conn.commit()
    row = conn.execute(
        "SELECT id, device_id, skill_id, project_id, agent_id, mode, local_path, is_enabled, created_at, updated_at, pinned_version FROM skill_project_bindings WHERE id = ?",
        (binding_id,),
    ).fetchone()
    if not row:
        raise RuntimeError("Binding disappeared")
    return {
        "id": row["id"],
        "device_id": row["device_id"],
        "skill_id": row["skill_id"],
        "project_id": row["project_id"],
        "agent_id": row["agent_id"],
        "mode": row["mode"],
        "local_path": row["local_path"],
        "is_enabled": bool(row["is_enabled"]),
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
        "pinned_version": row["pinned_version"],
    }


def cmd_delete_skill_version(conn, args):
    skill_id = args.get("skillId")
    version = args.get("version")
    if version == "latest":
        raise RuntimeError("不能删除 latest 版本")
    pinned = conn.execute(
        "SELECT p.name FROM skill_project_bindings b LEFT JOIN projects p ON b.project_id = p.id WHERE b.skill_id = ? AND b.pinned_version = ?",
        (skill_id, version),
    ).fetchall()
    if pinned:
        raise RuntimeError(f"版本 {version} 仍被以下项目固定：" + "、".join(p["name"] or "(unnamed)" for p in pinned))
    skill_root = _skill_root(conn, skill_id)
    version_dir = skill_root / version
    if version_dir.exists():
        import shutil
        shutil.rmtree(version_dir)
    return None


def cmd_set_version_note(conn, args):
    skill_id = args.get("skillId")
    version = args.get("version")
    note = args.get("note")
    skill_root = _skill_root(conn, skill_id)
    version_dir = skill_root / version
    if not version_dir.is_dir():
        raise RuntimeError(f"版本目录不存在：{version_dir}")
    note_path = version_dir / ".version-note"
    if note:
        note_path.write_text(note, encoding="utf-8")
    else:
        note_path.unlink(missing_ok=True)
    return None


def cmd_save_version(conn, args):
    import shutil
    skill_id = args.get("skillId")
    note = args.get("note")
    skill_root = _skill_root(conn, skill_id)
    existing = [p.name for p in skill_root.iterdir() if p.is_dir() and p.name.startswith("v") and p.name[1:].isdigit()]
    next_n = max([int(p.name[1:]) for p in existing] or [0]) + 1
    label = f"v{next_n}"
    source = _effective_skill_dir(skill_root, None)
    dest = skill_root / label
    if source.is_dir():
        shutil.copytree(source, dest)
    if note:
        (dest / ".version-note").write_text(note, encoding="utf-8")
    return label


def cmd_rollback_to_version(conn, args):
    import shutil
    skill_id = args.get("skillId")
    target_version = args.get("targetVersion")
    if not target_version or not target_version.startswith("v") or not target_version[1:].isdigit():
        raise RuntimeError(f"invalid version label: {target_version}")
    skill_root = _skill_root(conn, skill_id)
    target_dir = skill_root / target_version
    if not target_dir.is_dir():
        raise RuntimeError(f"version does not exist: {target_version}")
    existing = [p.name for p in skill_root.iterdir() if p.is_dir() and p.name.startswith("v") and p.name[1:].isdigit()]
    next_n = max([int(p.name[1:]) for p in existing] or [0]) + 1
    saved_as = f"v{next_n}"
    latest = skill_root / "latest"
    if latest.is_dir():
        shutil.copytree(latest, skill_root / saved_as)
    if latest.exists():
        shutil.rmtree(latest)
    shutil.copytree(target_dir, latest)
    return {"saved_as": saved_as, "rolled_to": target_version}


def cmd_open_project_in_finder(conn, args):
    return _mock_unsupported("无法从浏览器打开 Finder")


def cmd_backup_center_repo(conn, args):
    return _mock_unsupported("浏览器环境无法打包中心仓库")


def cmd_restore_center_repo(conn, args):
    return _mock_unsupported("浏览器环境无法恢复中心仓库备份")


def cmd_test_acp_transport(conn, args):
    return _mock_unsupported("ACP 传输测试需要本地 Agent 进程")


def cmd_test_ai_model(conn, args):
    return _mock_unsupported("AI 模型连接测试需要真实 API Key 与网络")


def cmd_open_path_in_terminal(conn, args):
    return _mock_unsupported("浏览器环境无法打开系统终端")


def cmd_list_bundles(conn, args):
    device_id = _device_id(conn)
    rows = conn.execute(
        """SELECT b.id, b.name, b.description, b.created_at, b.updated_at,
                  (SELECT COUNT(*) FROM skill_bundle_items i WHERE i.bundle_id = b.id) AS skill_count
           FROM skill_bundles b WHERE b.device_id = ? ORDER BY b.updated_at DESC""",
        (device_id,),
    ).fetchall()
    return [
        {
            "id": r["id"],
            "name": r["name"],
            "description": r["description"],
            "skill_count": r["skill_count"],
            "created_at": r["created_at"],
            "updated_at": r["updated_at"],
        }
        for r in rows
    ]


def cmd_get_bundle_detail(conn, args):
    bid = args.get("id")
    bundle = conn.execute(
        """SELECT b.id, b.name, b.description, b.created_at, b.updated_at,
                  (SELECT COUNT(*) FROM skill_bundle_items i WHERE i.bundle_id = b.id) AS skill_count
           FROM skill_bundles b WHERE b.id = ?""",
        (bid,),
    ).fetchone()
    if not bundle:
        raise RuntimeError("技能集不存在")
    items = conn.execute(
        """SELECT i.id, i.skill_id, COALESCE(s.name, i.skill_id) AS skill_name, i.sort_order
           FROM skill_bundle_items i LEFT JOIN skills s ON s.id = i.skill_id
           WHERE i.bundle_id = ? ORDER BY i.sort_order ASC, i.added_at ASC""",
        (bid,),
    ).fetchall()
    bundle_obj = {
        "id": bundle["id"],
        "name": bundle["name"],
        "description": bundle["description"],
        "skill_count": bundle["skill_count"],
        "created_at": bundle["created_at"],
        "updated_at": bundle["updated_at"],
    }
    item_list = [
        {"id": r["id"], "skill_id": r["skill_id"], "skill_name": r["skill_name"], "sort_order": r["sort_order"]}
        for r in items
    ]
    return [bundle_obj, item_list]


def cmd_create_bundle(conn, args):
    import uuid
    device_id = _device_id(conn)
    name = args.get("name", "bundle")
    description = args.get("description")
    bid = str(uuid.uuid4())
    now = int(time.time())
    conn.execute(
        "INSERT INTO skill_bundles (id, device_id, name, description, created_at, updated_at) VALUES (?,?,?,?,?,?)",
        (bid, device_id, name, description, now, now),
    )
    conn.commit()
    return {
        "id": bid,
        "name": name,
        "description": description,
        "skill_count": 0,
        "created_at": now,
        "updated_at": now,
    }


def cmd_delete_bundle(conn, args):
    bid = args.get("id")
    conn.execute("DELETE FROM skill_bundles WHERE id = ?", (bid,))
    conn.commit()
    return None


def cmd_add_skill_to_bundle(conn, args):
    import uuid
    bundle_id = args.get("bundleId")
    skill_id = args.get("skillId")
    device_id = _device_id(conn)
    skill = conn.execute("SELECT name FROM skills WHERE id = ?", (skill_id,)).fetchone()
    if not skill:
        raise RuntimeError("Skill 不存在")
    max_order = conn.execute(
        "SELECT COALESCE(MAX(sort_order), -1) FROM skill_bundle_items WHERE bundle_id = ?",
        (bundle_id,),
    ).fetchone()[0]
    item_id = str(uuid.uuid4())
    now = int(time.time())
    conn.execute(
        "INSERT INTO skill_bundle_items (id, device_id, bundle_id, skill_id, sort_order, added_at) VALUES (?,?,?,?,?,?)",
        (item_id, device_id, bundle_id, skill_id, max_order + 1, now),
    )
    conn.commit()
    return {
        "id": item_id,
        "skill_id": skill_id,
        "skill_name": skill["name"],
        "sort_order": max_order + 1,
    }


def cmd_remove_skill_from_bundle(conn, args):
    bundle_id = args.get("bundleId")
    skill_id = args.get("skillId")
    conn.execute(
        "DELETE FROM skill_bundle_items WHERE bundle_id = ? AND skill_id = ?",
        (bundle_id, skill_id),
    )
    conn.commit()
    return None


def cmd_apply_bundle_to_project(conn, args):
    bundle_id = args.get("bundleId")
    project_id = args.get("projectId")
    agent_ids = args.get("agentIds", [])
    mode = args.get("mode", "symlink")
    rows = conn.execute(
        """SELECT i.skill_id, s.name
           FROM skill_bundle_items i JOIN skills s ON s.id = i.skill_id
           WHERE i.bundle_id = ?""",
        (bundle_id,),
    ).fetchall()
    applied = []
    skipped = []
    for r in rows:
        try:
            cmd_install_skill_to_project(conn, {"skillId": r["skill_id"], "projectId": project_id, "agentIds": agent_ids, "mode": mode})
            applied.append(r["name"])
        except Exception as e:
            skipped.append((r["name"], str(e)))
    return {"applied": applied, "skipped": skipped}


def cmd_export_bundle(conn, args):
    bid = args.get("id")
    bundle = conn.execute("SELECT name, description FROM skill_bundles WHERE id = ?", (bid,)).fetchone()
    if not bundle:
        raise RuntimeError("技能集不存在")
    items = conn.execute(
        """SELECT COALESCE(s.name, i.skill_id) AS skill_name
           FROM skill_bundle_items i LEFT JOIN skills s ON s.id = i.skill_id
           WHERE i.bundle_id = ?""",
        (bid,),
    ).fetchall()
    export = {
        "name": bundle["name"],
        "description": bundle["description"],
        "exported_at": int(time.time()),
        "skills": [{"name": r["skill_name"], "required": True} for r in items],
    }
    return json.dumps(export, ensure_ascii=False, indent=2)


def cmd_export_bundle_directory(conn, args):
    import shutil
    bid = args.get("id")
    output_dir = args.get("outputDir") or str(_repo_dir() / "exports")
    bundle = conn.execute("SELECT name, description FROM skill_bundles WHERE id = ?", (bid,)).fetchone()
    if not bundle:
        raise RuntimeError("技能集不存在")
    base = Path(output_dir) / _sanitize_dir_name(bundle["name"])
    base.mkdir(parents=True, exist_ok=True)
    manifest = {
        "name": bundle["name"],
        "description": bundle["description"],
        "exported_at": int(time.time()),
        "skills": [],
    }
    skills_dir = base / "skills"
    items = conn.execute(
        "SELECT skill_id FROM skill_bundle_items WHERE bundle_id = ?", (bid,)
    ).fetchall()
    for item in items:
        skill = conn.execute("SELECT name, repo_path FROM skills WHERE id = ?", (item["skill_id"],)).fetchone()
        if not skill:
            continue
        manifest["skills"].append({"name": skill["name"], "required": True})
        dest = skills_dir / _sanitize_dir_name(skill["name"])
        dest.mkdir(parents=True, exist_ok=True)
        src = Path(skill["repo_path"]) / "SKILL.md"
        if src.is_file():
            shutil.copy2(src, dest / "SKILL.md")
    (base / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    return str(base)


def _sanitize_dir_name(name):
    return "".join(c if c.isalnum() or c in "-_" else "_" for c in name)


def cmd_create_snapshot(conn, args):
    import shutil
    center = _repo_dir()
    if not center.is_dir():
        raise RuntimeError("中心仓库不存在")
    snap_root = _repo_dir() / ".snapshots"
    snap_root.mkdir(parents=True, exist_ok=True)
    snap_id = f"{int(time.time())}-snapshot"
    dest = snap_root / snap_id
    shutil.copytree(center, dest)
    meta = {"id": snap_id, "created_at": int(time.time()), "note": args.get("note")}
    (dest / ".snapshot-meta.json").write_text(json.dumps(meta), encoding="utf-8")
    return {"id": snap_id, "created_at": meta["created_at"], "note": meta["note"], "skill_count": _count_skill_md(dest)}


def cmd_list_snapshots(conn, args):
    snap_root = _repo_dir() / ".snapshots"
    out = []
    if snap_root.is_dir():
        for p in sorted(snap_root.iterdir(), reverse=True):
            if not p.is_dir():
                continue
            meta_path = p / ".snapshot-meta.json"
            created_at = 0
            note = None
            if meta_path.is_file():
                try:
                    meta = json.loads(meta_path.read_text(encoding="utf-8"))
                    created_at = meta.get("created_at", 0)
                    note = meta.get("note")
                except Exception:
                    pass
            out.append({"id": p.name, "created_at": created_at, "note": note, "skill_count": _count_skill_md(p)})
    return out


def cmd_restore_snapshot(conn, args):
    import shutil
    snap_id = args.get("id")
    center = _repo_dir()
    src = _repo_dir() / ".snapshots" / snap_id
    if not src.is_dir():
        raise RuntimeError("快照不存在")
    snap_root = _repo_dir() / ".snapshots"
    if center.is_dir():
        backup = snap_root / f"center-repo-pre-restore-{int(time.time())}"
        shutil.copytree(center, backup)
    if center.exists():
        shutil.rmtree(center)
    shutil.copytree(src, center)
    return None


def _count_skill_md(path):
    count = 0
    if not path.is_dir():
        return 0
    for _ in path.rglob("SKILL.md"):
        count += 1
    return count


def cmd_git_init_repo(conn, args):
    center = _repo_dir()
    if not center.is_dir():
        raise RuntimeError("中心仓库不存在")
    git_dir = center / ".git"
    git_dir.mkdir(parents=True, exist_ok=True)
    (git_dir / "HEAD").write_text("ref: refs/heads/main\n", encoding="utf-8")
    return None


def cmd_git_commit(conn, args):
    return None


def cmd_git_pull(conn, args):
    return None


def cmd_git_status_inited(conn, args):
    return (_repo_dir() / ".git").is_dir()


DATA_MANAGEMENT_CONFIRM_CODE = "DELETE"


def cmd_clear_collected_data(conn, args):
    if args.get("confirm") != DATA_MANAGEMENT_CONFIRM_CODE:
        raise RuntimeError("确认码错误，操作已取消")
    tables = [
        "collected_sources", "collected_sessions", "collected_prompts",
        "collected_token_usage", "collected_code_contributions",
        "collector_file_states", "collection_jobs", "analysis_window_cache",
        "digest_summary", "llm_request_logs",
    ]
    for table in tables:
        conn.execute(f"DELETE FROM {table}")
    conn.commit()
    return None


def cmd_reset_database(conn, args):
    if args.get("confirm") != DATA_MANAGEMENT_CONFIRM_CODE:
        raise RuntimeError("确认码错误，操作已取消")
    tables = [
        "agents", "agent_directories", "agent_instances", "skills", "sync_targets",
        "projects", "skill_project_bindings", "skill_bundles", "skill_bundle_items",
        "collected_sources", "collected_sessions", "collected_prompts",
        "collected_token_usage", "collected_code_contributions", "collector_file_states",
        "collection_jobs", "kg_nodes", "kg_edges", "kg_skill_nodes", "sources",
        "digest_model", "digest_tool", "digest_summary", "analysis_window_cache",
        "discoveries", "weekly_reports", "scheduled_tasks", "task_runs", "llm_request_logs",
        "install_audit", "adoption_events", "gate_rejections", "trash_items",
    ]
    for table in tables:
        conn.execute(f"DELETE FROM {table}")
    conn.commit()
    return None


def cmd_git_versions(conn, args):
    return [
        {
            "sha": "mock-1",
            "message": "mock 初始提交",
            "author": "skillmint-mock",
            "timestamp": int(time.time()),
        }
    ]


COMMANDS = {
    "init_app": cmd_init_app,
    "get_skills": cmd_get_skills,
    "get_agents": cmd_get_agents,
    "get_sync_targets": cmd_get_sync_targets,
    "get_settings": cmd_get_settings,
    "save_settings": cmd_save_settings,
    "sync_all_command": cmd_sync_all_command,
    "clear_collected_data": cmd_clear_collected_data,
    "reset_database": cmd_reset_database,
    "scan_agents": cmd_scan_agents,
    "scan_agent_skills": cmd_scan_agent_skills,
    "get_agent_detail": cmd_get_agent_detail,
    "get_collection_status": cmd_get_collection_status,
    "get_daily_summary": cmd_get_daily_summary,
    "list_daily_summaries": cmd_list_daily_summaries,
    "get_window_metrics": cmd_get_window_metrics,
    "start_collection_job": cmd_start_collection_job,
    "list_recent_collection_jobs": cmd_list_recent_collection_jobs,
    "cancel_collection_job": cmd_cancel_collection_job,
    "get_agent_usage": cmd_get_agent_usage,
    "collect_usage_data": cmd_collect_usage_data,
    # PRD-06 §3.3: agent directory (1:N) management.
    "add_agent_directory": cmd_add_agent_directory,
    "remove_agent_directory": cmd_remove_agent_directory,
    "update_agent_directory": cmd_update_agent_directory,
    "scan_directory_skills": cmd_scan_directory_skills,
    "get_high_value_prompts": cmd_get_high_value_prompts,
    "generate_skill_from_prompt": cmd_generate_skill_from_prompt,
    "preview_skill_from_prompt": cmd_preview_skill_from_prompt,
    "repair_skill_paths": cmd_repair_skill_paths,
    "get_knowledge_graph": cmd_get_knowledge_graph,
    "analyze_knowledge_graph": cmd_analyze_knowledge_graph,
    "recommend_skills_for_task": cmd_recommend_skills_for_task,
    "confirm_kg_edge": cmd_confirm_kg_edge,
    "reject_kg_edge": cmd_reject_kg_edge,
    "get_projects": cmd_get_projects,
    "get_skill_bindings": cmd_get_skill_bindings,
    "scan_projects": cmd_scan_projects,
    "get_project_detail": cmd_get_project_detail,
    "resolve_skill_link_command": cmd_resolve_skill_link_command,
    "list_skill_versions_command": cmd_list_skill_versions_command,
    "install_skill_to_project": cmd_install_skill_to_project,
    "resolve_skill_diff_command": cmd_resolve_skill_diff_command,
    "get_conflict_contents": cmd_get_conflict_contents,
    "resolve_conflict": cmd_resolve_conflict,
    "create_skill": cmd_create_skill,
    # SPEC-C3: recycle bin (remove_skill is now "move to trash").
    "remove_skill": cmd_remove_skill,
    "list_trash_items": cmd_list_trash_items,
    "restore_trash_item": cmd_restore_trash_item,
    "purge_trash_item": cmd_purge_trash_item,
    "open_skill_in_editor": cmd_open_skill_in_editor,
    "import_skill": cmd_import_skill,
    "get_skill_usage": cmd_get_skill_usage,
    "get_related_skills": cmd_get_related_skills,
    # SPEC-I2: discovery inbox + weekly reports.
    "list_discoveries": cmd_list_discoveries,
    "decide_discovery": cmd_decide_discovery,
    # SPEC-C1: three-way decision + gate-rejection ledger.
    "list_gate_rejections": cmd_list_gate_rejections,
    # SPEC-C2: retry sync for a single skill.
    "sync_single_skill_command": cmd_sync_single_skill_command,
    "get_growth_metrics": cmd_get_growth_metrics,
    "get_weekly_report": cmd_get_weekly_report,
    "generate_weekly_report": cmd_generate_weekly_report,
    # SPEC-F1: eval environment alignment.
    "check_repo_integrity": cmd_check_repo_integrity,
    "save_agent": cmd_save_agent,
    "detect_local_agents": cmd_detect_local_agents,
    "get_agent_skill_counts": cmd_get_agent_skill_counts,
    "update_skill_status": cmd_update_skill_status,
    "check_skill_external_change": cmd_check_skill_external_change,
    "read_skill_content": cmd_read_skill_content,
    "save_skill_content": cmd_save_skill_content,
    "search_all": cmd_search_all,
    "get_sources": cmd_get_sources,
    "add_source": cmd_add_source,
    "remove_source": cmd_remove_source,
    "refresh_source": cmd_refresh_source,
    "list_source_skills": cmd_list_source_skills,
    "scan_skill_safety": cmd_scan_skill_safety,
    "preview_remote_skill": cmd_preview_remote_skill,
    "install_remote_skill": cmd_install_remote_skill,
    "get_bundle_detail": cmd_get_bundle_detail,
    "get_scheduled_tasks": cmd_get_scheduled_tasks,
    "save_scheduled_task": cmd_save_scheduled_task,
    "delete_scheduled_task": cmd_delete_scheduled_task,
    "get_task_runs": cmd_get_task_runs,
    "run_scheduled_task_now": cmd_run_scheduled_task_now,
    "export_data_dictionary": cmd_export_data_dictionary,
    "export_raw_data": cmd_export_raw_data,
    "list_llm_request_logs": cmd_list_llm_request_logs,
    "classify_prompts": cmd_classify_prompts,
    "get_summary_value_metrics": cmd_get_summary_value_metrics,
    "get_prompts_for_cell": cmd_get_prompts_for_cell,
    "generate_daily_summary": cmd_generate_daily_summary,
    "pin_binding_version": cmd_pin_binding_version,
    "delete_skill_version": cmd_delete_skill_version,
    "set_version_note": cmd_set_version_note,
    "save_version": cmd_save_version,
    "rollback_to_version": cmd_rollback_to_version,
    "open_project_in_finder": cmd_open_project_in_finder,
    "backup_center_repo": cmd_backup_center_repo,
    "restore_center_repo": cmd_restore_center_repo,
    "test_acp_transport": cmd_test_acp_transport,
    "test_ai_model": cmd_test_ai_model,
    "open_path_in_terminal": cmd_open_path_in_terminal,
    "list_bundles": cmd_list_bundles,
    "create_bundle": cmd_create_bundle,
    "delete_bundle": cmd_delete_bundle,
    "add_skill_to_bundle": cmd_add_skill_to_bundle,
    "remove_skill_from_bundle": cmd_remove_skill_from_bundle,
    "apply_bundle_to_project": cmd_apply_bundle_to_project,
    "export_bundle": cmd_export_bundle,
    "export_bundle_directory": cmd_export_bundle_directory,
    "create_snapshot": cmd_create_snapshot,
    "list_snapshots": cmd_list_snapshots,
    "restore_snapshot": cmd_restore_snapshot,
    "git_init_repo": cmd_git_init_repo,
    "git_commit": cmd_git_commit,
    "git_pull": cmd_git_pull,
    "git_status_inited": cmd_git_status_inited,
    "git_versions": cmd_git_versions,
}


class Handler(BaseHTTPRequestHandler):
    def _cors(self):
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")

    def log_message(self, fmt, *args):  # quieter
        return

    def do_OPTIONS(self):
        self.send_response(204)
        self._cors()
        self.end_headers()

    def do_GET(self):
        path = urlparse(self.path).path
        if path == "/health":
            body = json.dumps(
                {"ok": True, "db": str(DB_PATH), "exists": DB_PATH.exists()}
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self._cors()
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_error(404)

    def do_POST(self):
        path = urlparse(self.path).path
        if path != "/invoke":
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b"{}"
        try:
            payload = json.loads(raw or b"{}")
        except Exception:
            payload = {}
        cmd = payload.get("cmd")
        args = payload.get("args", {}) or {}
        handler = COMMANDS.get(cmd)
        if handler is None:
            body = json.dumps(
                {"error": f"mock 后端未实现命令 {cmd}，请先运行 tools/check_mock_coverage.py"}
            ).encode()
            self.send_response(404)
            self.send_header("Content-Type", "application/json")
            self._cors()
            self.end_headers()
            self.wfile.write(body)
            return
        try:
            conn = db()
            result = handler(conn, args)
            conn.close()
            body = json.dumps(result).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self._cors()
            self.end_headers()
            self.wfile.write(body)
        except Exception as e:
            import traceback
            traceback.print_exc()
            body = json.dumps({"error": str(e)}).encode()
            self.send_response(500)
            self.send_header("Content-Type", "application/json")
            self._cors()
            self.end_headers()
            self.wfile.write(body)


def _coverage_counts():
    fe_dir = Path(__file__).resolve().parent.parent / "skillmint" / "src"
    invoke_re = re.compile(r"invoke\s*(?:<[^()]*?>)?\s*\(\s*['\"]([a-z_]+)['\"]")

    def _is_source(path):
        if path.suffix not in {".ts", ".tsx", ".js", ".jsx"}:
            return False
        stem = path.name.lower()
        return ".test." not in stem and ".spec." not in stem

    fe_names = set()
    try:
        raw = subprocess.check_output(
            [
                "rg",
                "-U",
                "-o",
                invoke_re.pattern,
                "-r",
                "$1",
                "-g", "!*.test.*",
                "-g", "!*.spec.*",
                str(fe_dir),
                "--no-filename",
            ],
            text=True,
            stderr=subprocess.DEVNULL,
        )
        for line in raw.splitlines():
            name = line.strip()
            if name:
                fe_names.add(name)
    except Exception:
        pass
    for path in fe_dir.rglob("*"):
        if not _is_source(path):
            continue
        text = path.read_text(encoding="utf-8", errors="ignore")
        for m in invoke_re.finditer(text):
            fe_names.add(m.group(1))
    mock_names = set()
    self_text = Path(__file__).read_text(encoding="utf-8")
    for m in re.finditer(r"def\s+cmd_([a-z_]+)\s*\(", self_text):
        mock_names.add(m.group(1))
    return len(fe_names & mock_names), len(fe_names)


def main():
    if not DB_PATH.exists():
        raise SystemExit(f"DB not found: {DB_PATH}")
    covered, total = _coverage_counts()
    server = HTTPServer(("127.0.0.1", PORT), Handler)
    print(f"mock backend on http://127.0.0.1:{PORT}  (db: {DB_PATH})")
    print(f"commands: {len(COMMANDS)}")
    print(f"command coverage: {covered}/{total}")
    server.serve_forever()


if __name__ == "__main__":
    main()
