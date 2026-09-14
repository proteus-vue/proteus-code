#!/usr/bin/env bash
# 检测 shell 脚本里"变量后紧跟非 ASCII 字符"的写法。
#
# 为什么需要它：macOS 自带 bash 3.2 在解析 `"发布 $V（...）"` 时，会把全角
# 括号的字节也算进变量名，于是报 `V）: unbound variable`。而 `bash -n` 查不出
# （语法合法），只有真跑才炸。本项目已因此复发 **4 次**（publish-npm.sh、
# install.sh、verify.sh、release.yml），故固化成门禁。
#
# 修法：一律用花括号界定 —— `${V}（`。
#
# 扫描范围：scripts/*.sh 与 .github/workflows/*.yml 里的 shell 段。
# 用法：bash scripts/check_shell_multibyte.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import re, sys, pathlib
try:
    import yaml
except ImportError:
    yaml = None

# $VAR 或 ${VAR} 之外的裸 $VAR，其后紧跟非 ASCII 字节
pat = re.compile(r'\$([A-Za-z_][A-Za-z0-9_]*)([^\x00-\x7f])')
violations = []

def scan(text, label):
    for i, line in enumerate(text.splitlines(), 1):
        stripped = line.lstrip()
        # 跳过整行注释（文档里的示例不会被 shell 展开）
        if stripped.startswith("#"):
            continue
        for m in pat.finditer(line):
            col = m.start() + 1
            violations.append((label, i, col, m.group(0), line.strip()))

for sh in sorted(pathlib.Path("scripts").glob("*.sh")):
    scan(sh.read_text(encoding="utf-8"), str(sh))

for wf in sorted(pathlib.Path(".github/workflows").glob("*.yml")):
    text = wf.read_text(encoding="utf-8")
    if yaml is None:
        scan(text, str(wf))
        continue
    data = yaml.safe_load(text)
    for jname, job in ((data.get("jobs") or {})).items():
        for step in (job.get("steps") or []):
            run = step.get("run")
            if run:
                scan(run, f"{wf}:{jname}/{step.get('name', '?')}")

if violations:
    print(f"❌ 发现 {len(violations)} 处「变量后紧跟非 ASCII」—— bash 3.2 会把多字节字节吃进变量名：")
    print()
    for label, line, col, frag, src in violations:
        print(f"  {label}:{line}:{col}  {frag}")
        print(f"      {src[:100]}")
    print()
    print("  修法：用花括号界定变量名 —— 在变量名两侧加大括号，再紧跟中文标点。")
    sys.exit(1)

print("✅ 未发现「变量后紧跟非 ASCII」的写法")
PY
