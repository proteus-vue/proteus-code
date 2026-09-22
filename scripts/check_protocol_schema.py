#!/usr/bin/env python3
"""协议契约产物守卫：Rust 类型改了，schema/ 没跟上就变红。

为什么要有这道门禁
------------------
`schema/neo-appserver.{schema.json,ts}` 是给**跨进程客户端**（GUI / IDE /
脚本）用的契约事实来源。它由 `schemars` / `ts-rs` 从 Rust 类型生成 ——
若有人改了 `Op` / `EventMsg` / 方法参数却忘了重生成，客户端会拿着过期类型
开发，而 Rust 侧测试全绿（它只看类型，不看产物）。

这正是本仓库反复踩过的坑：**静态检查全绿，而对外契约是坏的**。
所以判据必须是「产物与类型同源重生成后逐字节一致」，不是人眼 review。

用法
----
    python3 scripts/check_protocol_schema.py          # 门禁
    bash scripts/gen_protocol_schema.sh               # 人手更新产物

退出码：0 通过 / 1 产物过期或生成失败 / 2 环境错误。
"""

from __future__ import annotations

import filecmp
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
COMMITTED = ROOT / "crates" / "neo-host-appserver" / "schema"
ARTIFACTS = ("neo-appserver.schema.json", "neo-appserver.ts", "methods.json")


def fail(msg: str) -> int:
    print(f"❌ {msg}")
    return 1


def main() -> int:
    cargo = os.environ.get("CARGO", "cargo")
    if shutil.which(cargo) is None and not Path.home().joinpath(".cargo/bin/cargo").exists():
        print("❌ 找不到 cargo —— 无法重生成协议契约，按失败处理（环境问题）")
        return 2
    if shutil.which(cargo) is None:
        cargo = str(Path.home() / ".cargo/bin/cargo")

    for name in ARTIFACTS:
        if not (COMMITTED / name).is_file():
            return fail(
                f"入库产物缺失：{COMMITTED / name}\n"
                f"  先跑 bash scripts/gen_protocol_schema.sh 生成并提交。"
            )

    tmp = Path(tempfile.mkdtemp(prefix="neo-schema-"))
    try:
        env = os.environ.copy()
        env["NEO_SCHEMA_OUT"] = str(tmp)
        # 只跑写盘那一条用例；其余 schema 单测由 cargo test 覆盖。
        cmd = [
            cargo,
            "test",
            "-p",
            "neo-host-appserver",
            "--features",
            "schema",
            "export_schema",
            "--",
            "--ignored",
            "--nocapture",
        ]
        print(f"  → 重生成到 {tmp}")
        print(f"  $ {' '.join(cmd)}")
        r = subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True, text=True)
        if r.returncode != 0:
            sys.stderr.write(r.stdout)
            sys.stderr.write(r.stderr)
            return fail("重生成失败（见上）—— 门禁无法判定产物是否过期")

        stale = []
        for name in ARTIFACTS:
            new = tmp / name
            old = COMMITTED / name
            if not new.is_file():
                return fail(f"生成器没写出 {name} —— export_schema 被改坏了？")
            if not filecmp.cmp(new, old, shallow=False):
                stale.append(name)

        if stale:
            print("❌ 协议契约产物与 Rust 类型不一致：")
            for name in stale:
                print(f"    {name}")
            print("  修法：bash scripts/gen_protocol_schema.sh  # 重生成并提交")
            print("  （不要手改 schema/ 下的文件 —— 下次生成会被覆盖）")
            return 1

        print(f"✅ 协议契约产物与类型一致（{len(ARTIFACTS)} 个文件逐字节比对）")
        return 0
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
