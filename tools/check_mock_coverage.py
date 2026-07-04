#!/usr/bin/env python3
"""
Self-check: ensure every frontend `invoke('...')` / `invoke<T>('...')` has a
matching `cmd_<name>` handler in tools/mock_server.py.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FE_DIR = ROOT / "skillmint" / "src"
MOCK_FILE = ROOT / "tools" / "mock_server.py"


def frontend_commands():
    """Collect command names used in frontend invoke() calls."""
    names = set()
    # rg is faster and respects .gitignore; fall back to pathlib if unavailable.
    invoke_re = re.compile(r"invoke\s*(?:<[^()]*?>)?\s*\(\s*['\"]([a-z_]+)['\"]")
    def _is_source(path: Path) -> bool:
        if path.suffix not in {".ts", ".tsx", ".js", ".jsx"}:
            return False
        stem = path.name.lower()
        return ".test." not in stem and ".spec." not in stem

    try:
        raw = subprocess.check_output(
            [
                "rg",
                "-U",  # multiline so generic type on one line and command name on the next match
                "-o",
                invoke_re.pattern,
                "-r",
                "$1",
                "-g", "!*.test.*",
                "-g", "!*.spec.*",
                str(FE_DIR),
                "--no-filename",
            ],
            text=True,
            stderr=subprocess.DEVNULL,
        )
        for line in raw.splitlines():
            line = line.strip()
            if line:
                names.add(line)
    except Exception:
        pass
    for path in FE_DIR.rglob("*"):
        if not _is_source(path):
            continue
        text = path.read_text(encoding="utf-8", errors="ignore")
        for m in invoke_re.finditer(text):
            names.add(m.group(1))
    return names


def mock_commands():
    """Collect cmd_* handler names registered in mock_server.py."""
    names = set()
    text = MOCK_FILE.read_text(encoding="utf-8")
    for m in re.finditer(r"def\s+cmd_([a-z_]+)\s*\(", text):
        names.add(m.group(1))
    return names


def main():
    fe = frontend_commands()
    mock = mock_commands()
    missing = sorted(fe - mock)
    total = len(fe)
    covered = total - len(missing)
    print(f"frontend invoke commands: {total}")
    print(f"mock handlers: {len(mock)}")
    print(f"command coverage: {covered}/{total}")
    if missing:
        print("missing handlers:")
        for name in missing:
            print(f"  - {name}")
        return 1
    print("all frontend commands covered")
    return 0


if __name__ == "__main__":
    sys.exit(main())
