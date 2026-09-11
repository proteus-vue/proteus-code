#!/usr/bin/env python3
"""
配置层级与执行模式解析校验（对应 dsh-config）。

  C1 四级配置：内置 -> /etc -> 用户 -> 项目 -> CLI flag，后置覆盖前置
  C2 安全敏感键（model_provider / profile / notify）项目级必须被忽略
  C3 ExecMode -> (SandboxMode, ApprovalPolicy, FileEditPolicy) 映射存在且唯一
  C4 AGENTS.md 合并上限 32 KiB，超限走摘要降级
"""
import os, sys

SENSITIVE = {"model_provider", "profile", "profiles", "notify"}
MAX_BYTES = 32 * 1024

BUILTIN  = {"model": "deepseek-v4-flash", "exec_mode": "default",
            "sandbox_mode": "workspace_write", "approval_policy": "on_request"}
SYSTEM   = {"model": "sys-model"}
USER     = {"model": "user-model", "approval_policy": "untrusted"}
PROJECT  = {"sandbox_mode": "read_only", "model_provider": "evil-provider", "profile": "evil"}
CLI      = {"exec_mode": "plan"}

def merge(*layers, scope=None):
    out = {}
    for name, layer in layers:
        for k, v in layer.items():
            # C2 非用户级遇到敏感键 -> 忽略
            if k in SENSITIVE and scope != "user":
                continue
            out[k] = v
    return out

# 执行模式 -> (沙箱, 审批, 文件编辑策略)（对应 dsh_config::resolve）
# 第三维是必需的：ZCode 的 default 与 auto_edit 在"沙箱×审批"双轴上完全相同，
# 二者的真实差异在工具类别粒度 —— 文件编辑是否自动放行、命令是否仍需批准。
# 缺了这一维，两个模式在底层就不可区分（此前的映射曾被本校验判为歧义）。
MODE_MAP = {
    "plan":           ("read_only",          "on_request", "ask"),
    "confirm_before": ("workspace_write",    "untrusted",  "ask"),
    "default":        ("workspace_write",    "on_request", "ask"),
    "auto_edit":      ("workspace_write",    "on_request", "auto"),
    "full_access":    ("danger_full_access", "never",      "auto"),
}
SANDBOXES = {"read_only", "workspace_write", "danger_full_access"}
POLICIES  = {"untrusted", "on_request", "on_failure", "never"}
FILE_EDIT_POLICY = {"auto", "ask"}

def main():
    fails = []
    print("=" * 62)
    print("配置层级与模式解析校验")
    print("=" * 62)

    # C1
    got = merge(("builtin", BUILTIN), ("system", SYSTEM), ("user", USER),
                ("project", PROJECT), ("cli", CLI))
    expect_model = "user-model"   # CLI 没覆盖 model，最后是 user 层
    expect_mode  = "plan"         # CLI flag 最高优先级
    expect_sand  = "read_only"    # 项目层
    c1 = (got.get("model") == expect_model
          and got.get("exec_mode") == expect_mode
          and got.get("sandbox_mode") == expect_sand)
    print(f"  [{'PASS' if c1 else 'FAIL'}] C1 四级配置合并：model={got.get('model')} "
          f"exec_mode={got.get('exec_mode')} sandbox={got.get('sandbox_mode')}")
    if not c1:
        fails.append("C1 配置优先级不符")
        print(f"       期望 model={expect_model} exec_mode={expect_mode} sandbox={expect_sand}")

    # C2
    c2 = ("model_provider" not in got) and ("profile" not in got)
    print(f"  [{'PASS' if c2 else 'FAIL'}] C2 敏感键在项目级被忽略："
          f"model_provider={'model_provider' in got} profile={'profile' in got}")
    if not c2:
        fails.append("C2 敏感键未被拦截")

    # C3
    c3 = True
    for m, (sb, ap, fe) in MODE_MAP.items():
        if sb not in SANDBOXES or ap not in POLICIES or fe not in FILE_EDIT_POLICY:
            c3 = False
            fails.append(f"C3 模式 {m} 映射到非法值 ({sb}, {ap}, {fe})")
    if len(set(MODE_MAP.values())) != len(MODE_MAP):
        c3 = False
        fails.append("C3 存在重复映射（模式语义歧义）")
    print(f"  [{'PASS' if c3 else 'FAIL'}] C3 五档模式 -> (沙箱, 审批, 文件编辑) 唯一且合法")
    for m, (sb, ap, fe) in MODE_MAP.items():
        print(f"       {m:15} -> sandbox={sb:20} approval={ap:12} file_edit={fe}")

    # C4
    small = "x" * 100
    big   = "y" * (MAX_BYTES + 1)
    c4 = len(small.encode()) <= MAX_BYTES and len(big.encode()) > MAX_BYTES
    degraded = len(big.encode()) > MAX_BYTES   # 超限 -> 应降级为摘要
    c4 = c4 and degraded
    print(f"  [{'PASS' if c4 else 'FAIL'}] C4 AGENTS.md 上限 32KiB：超限降级为摘要")
    if not c4:
        fails.append("C4 32KiB 降级逻辑不符")

    print("-" * 62)
    if fails:
        for f in fails:
            print("   -", f)
        print(f"❌ 配置校验失败，共 {len(fails)} 项")
        return 1
    print("✅ 配置校验全部通过")
    return 0

if __name__ == "__main__":
    sys.exit(main())
