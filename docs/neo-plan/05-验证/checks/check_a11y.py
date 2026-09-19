#!/usr/bin/env python3
"""
无障碍守卫 —— 保证「自绘控件对读屏可见、对键盘可达」不是靠自觉。

校验项：
  A1 每个**裸可点元素**（`div().on_click`，非组件库控件）必须有 role + aria_label
  A2 每个裸可点元素必须能进 Tab 顺序（track_focus + tab_index）
  A3 tab_index 必须配 track_focus（否则登记不进顺序表）

# 为什么需要这个门禁

方案 Phase 3 的组件成熟度九宫格（`docs/gpui-方案/docs/01-组件库规格.md` §3）
前两项是：

  1. **键盘可达** —— 纯键盘可完成该组件所有操作
  3. **无障碍** —— 有 role + label + state；读屏能读出名称与当前状态

而本轮审计（§4.64(bp)）实测：gpui 宿主里 **28 个可点元素只有 4 个带 role、
8 个带 aria_label、0 个进 Tab 顺序**。缺口集中在"裸 `div().on_click()`"——
组件库的 `Button` / `Input` 自带 role + track_focus + tab_index，所以那些
不需要我们操心；**自己拼的每一个可点 `div` 都要手动补齐**。

# 为什么 role 必须写在**调用点**（一次实测确认的约束）
#
# 试过把 `role(Button)` 收进 `text_button` 助手函数里（那样 9 个调用点只写一次），
# 但**编译不过**：`role` 来自 `StatefulInteractiveElement`，而那要求元素先有
# `.id()` —— 助手函数返回的 `Div` 还没有 id，调用方之后才 `.id(...)`。
# 所以 role 只能写在调用点。这条记在这里，免得下一个人再试一次同样的收拢。
#
# 为什么必须机器判（而不是 review 时看一眼）

因为**它不补也能用**：鼠标点得动、界面看着正常、测试全绿 —— 缺陷只对
"用键盘或读屏的人"显现，而我们自己不是那类用户。这正是"看起来永远无害"
的另一例，与 `check_ui_layering.py` 要防的是同一种东西。

# 这条守卫的"牙齿"（本轮实测过的）

给文件行补 `role(Button)` + `aria_label` 时发现的**正面副作用**：
它同时把**程序化验证的通道**打开了 —— 有了 role，测试可以用 `AXPress`
语义点击（不需要坐标、不需要窗口在前台，见 PROJECT_MEMORY §4.64(ba)）。
所以补 a11y 是一石二鸟：**既合规，又可测**。

# 诚实边界

- 它只查**静态形态**（属性在不在），查不了"标签文案是否准确描述了行为"——
  那是语义问题，只能靠 review。
- 它只扫 `neo-host-gpui`。TUI 宿主没有 AT 概念（终端里读屏走文本流），
  egui 是冻结的回退通道 —— 给它补 a11y 是纯浪费。
- 它**不检查焦点顺序是否合理**（Tab 走过去是不是按视觉顺序）—— 那要真跑
  起来逐个按 Tab 观察，属于人工验收。
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.environ.get("NEO_ROOT", os.path.join(HERE, "..", "..", "..", ".."))

# 被扫的文件：桌面主宿主的界面单文件（本项目的 GUI 全在这里）
TARGET = os.path.join(ROOT, "crates", "neo-host-gpui", "src", "app.rs")

# ── 组件库控件：它们**自带** role / track_focus / tab_index ───────────────
#
# 依据（读上游源码确认，不是猜）：
#   gpui-component-0.6.1/src/button/button.rs:716  Role::Button
#                                              :751  .track_focus(&focus_handle)
#                                              :752  .tab_index(self.tab_index)
# 所以凡是从组件库构造的控件，都不该被本守卫要求再补一遍。
LIBRARY_WIDGETS = (
    "Button::new",
    "Input::new",
    "Textarea::new",
    "Checkbox::new",
    "Switch::new",
    "Select::new",
    "Slider::new",
    "IconButton::new",
    "DropdownButton::new",
    "Toggle::new",
)

# 每个可点元素向前找多少行算"同一个元素的属性区"。
#
# 取 30：链式构造一个带条件分支的行比较长（实测最长的一处从 `.id()` 到
# `.on_click()` 有 22 行）。这个数字偏大只会放宽检查，不会误报 —— 而误报是
# 守卫最危险的失效方式（见 anti-patterns 里"没有牙齿"的反面：**乱咬**）。
CONTEXT_LINES = 30


def audit(path):
    """返回 (violations, summary)。"""
    with open(path, encoding="utf-8") as f:
        lines = f.read().split("\n")

    violations = []
    checked = 0
    for i, line in enumerate(lines):
        if ".on_click(" not in line:
            continue
        start = max(0, i - CONTEXT_LINES)
        ctx = "\n".join(lines[start : i + 4])

        # 组件库控件：自带 a11y，跳过
        if any(w in ctx for w in LIBRARY_WIDGETS):
            continue

        checked += 1
        has_role = ".role(" in ctx
        has_aria = ".aria_label(" in ctx
        has_tab = ".tab_index(" in ctx or ".tab_stop(" in ctx

        if not has_role or not has_aria:
            missing = []
            if not has_role:
                missing.append("role")
            if not has_aria:
                missing.append("aria_label")
            violations.append(
                (i + 1, f"A1 缺 {' + '.join(missing)} —— 读屏用户不知道这是什么/干什么", line.strip()[:70])
            )
        if not has_tab:
            violations.append(
                (i + 1, "A2 未进 Tab 顺序（缺 tab_index）—— 纯键盘用户到不了这里", line.strip()[:70])
            )
        # A3：`tab_index` 必须配 `track_focus`，否则**登记不进顺序表**。
        #
        # 这是真机实测抓出来的：我一度给 15 处补齐了 `tab_index`，而
        # `focus_next` 连调 5 次焦点一动不动。根因（读上游源码）：
        #
        #     gpui-pre/src/elements/div.rs:2555
        #     if let Some(focus_handle) = &self.tracked_focus_handle {
        #         window.next_frame.tab_stops.insert(focus_handle);
        #     }
        #
        # 登记要求 `tracked_focus_handle`（来自 `track_focus(&handle)`），
        # 而当时生产代码里 `track_focus` 是 **0 处** —— 那 15 个 `tab_index`
        # 全部形同虚设。**属性在、键盘走不到**，而静态检查是绿的。
        if has_tab and ".track_focus(" not in ctx and ".tab_stop(" not in ctx:
            violations.append(
                (
                    i + 1,
                    "A3 有 tab_index 但缺 track_focus —— 元素登记不进 Tab 顺序表，"
                    "`focus_next` 会跳过它（属性在、键盘走不到）",
                    line.strip()[:70],
                )
            )

    return violations, checked


def main():
    if not os.path.isfile(TARGET):
        print(f"❌ 找不到待扫文件：{TARGET}")
        return 1

    violations, checked = audit(TARGET)

    if violations:
        print(f"❌ 无障碍缺口 {len(violations)} 处（扫了 {checked} 个自绘可点元素）：")
        print()
        for line, why, src in violations:
            print(f"  crates/neo-host-gpui/src/app.rs:{line}")
            print(f"      {why}")
            print(f"      {src}")
        print()
        print("  修法：给该元素补上")
        print("      .role(neo_ui_kit::gpui::accesskit::Role::Button)")
        print('      .aria_label("<它做什么>")   ← 带上对象（如“切换到文件 xxx”）')
        print("      .track_focus(&focus_handle).tab_index(<序号>)")
        print("  组件库的 Button/Input 等自带这些，只有**自己拼的 div** 要补。")
        return 1

    print(f"✅ 无障碍守卫通过：{checked} 个自绘可点元素都有 role / aria_label / Tab 入口")
    return 0


if __name__ == "__main__":
    sys.exit(main())
