# opencode 交互范式对照（parity spec）

> **这份文档的用途**：把"对齐 opencode/mimo"从**主观感觉**变成**可逐条核对的事实**。
> 之前我们靠用户逐个截图报问题（"打地鼠"），根因是**没有一份写下来的范式**。
> 本文从 opencode 源码提取设计语言与交互契约，作为后续所有界面工作的依据。
>
> **来源**：`sst/opencode` 的 `packages/tui`（本地缓存于
> `.cache/ai-external/github/opencode-dev`，由
> `docs/ai-efficiency-rules/scripts/cache_fetch.sh` 拉取）。
> 引用处标注文件路径，便于复核。
>
> ⚠️ **`.cache/` 不入库**（外部资源缓存），所以**换一台电脑必须重拉**，
> 否则本文所有路径都指不到东西。一条命令搞定：
> ```bash
> bash scripts/fetch-refs.sh
> PROXY=http://127.0.0.1:7897 bash scripts/fetch-refs.sh   # github 直连不通时
> ```
> 细节与 MiMo 的特殊情况见 `PROJECT_MEMORY.md` §4.53。

---

## 1. 视觉语言：层级来自**底色阶梯 + 单边条**，不是四边框

### 1.1 底色是**精确的亮度阶梯**（最关键的一条）

opencode 默认主题（`theme/assets/opencode.json`）的层级不是随意取色，而是
一条 `#0a0a0a → #141414 → #1e1e1e` 的阶梯：

| token | darkStep | 值 | 用途 |
|---|---|---|---|
| `background` | step1 | `#0a0a0a` | 页面底 |
| `backgroundPanel` | step2 | `#141414` | **面板**（对话框、权限提示、侧栏卡片） |
| `backgroundElement` | step3 | `#1e1e1e` | **元素**（底部选项条、悬停态、页脚） |
| `backgroundMenu` | step3 附近 | — | 未选中项背景 |
| `border` | step7 | `#484848` | 常规边界 |
| `borderActive` | step8 | `#606060` | 激活边界 / 滚动条 |
| `textMuted` | step11 | `#808080` | 次要文字 |
| `text` | step12 | `#eeeeee` | 正文 |

**规则**：越靠前的层级越暗，逐级 +1 step。层级感来自这个**渐变阶梯**，
不来自加更多边框。我们目前的 `bg_panel/bg_surface/bg_selected` 方向对，
但缺 **element** 这一档、且取值没有按固定阶梯走。

### 1.2 面板用**单边竖条**，不是四边框

`ui/border.ts`：
```ts
export const SplitBorder = {
  border: ["left", "right"],           // 只画左右（很多处只用 left）
  customBorderChars: { vertical: "┃" },
}
```
`routes/session/permission.tsx` 的面板：
```tsx
<box backgroundColor={theme.backgroundPanel}
     border={["left"]} borderColor={theme.warning}
     customBorderChars={SplitBorder.customBorderChars}>
```
即：**面板底色 + 左侧一条彩色竖条 `┃`**。没有 `╭╮╰╯` 四角框。

**为什么这样更好**：四角框把内容"关"在一个盒子里，视觉噪音大且吃宽度；
底色 + 单边条既表达"这是独立一层"，又不抢内容。我们目前的审批卡片是四角框
（用户原话"简单划线、没有层级边界感"），应当改成这个形态。

### 1.3 底部**选项条**是独立的一条带

`permission.tsx` 的选项区：
```tsx
<box backgroundColor={theme.backgroundElement}   // 比面板再亮一档
     flexDirection="row" gap={1} paddingLeft={2} paddingRight={3}
     paddingTop={1} paddingBottom={1}
     justifyContent="space-between">
  <For each={keys}>{(option) => (
    <box paddingLeft={1} paddingRight={1}
         backgroundColor={option === selected ? theme.warning : theme.backgroundMenu}>
      <text fg={option === selected ? selectedForeground(theme, warning) : theme.textMuted}>
        {options[option]}
      </text>
    </box>
  )}</For>
  ...
```
**规则**：
- 选项条有**自己的底色**（element，比面板亮一档）→ 形成"内容区 / 操作区"的分层。
- 选项是**横向药丸**，不是竖排列表。
- **选中项 = 整块填充背景色**（warning/primary）+ 反色文字；未选中 = menu 底色 + muted 文字。
- 右侧对齐放**键位提示**（`⇆ select`、`enter confirm`）。

我们的审批选项是竖排 `❯` + 文字 —— 这是**列表**范式，不是**对话框**范式。

---

## 2. 对话框范式

### 2.1 覆盖层 + 遮罩

`ui/dialog.tsx`：
```tsx
<box position="absolute" zIndex={3000}
     width={dimensions().width} height={dimensions().height}
     alignItems="center" paddingTop={dimensions().height / 4}
     backgroundColor={RGBA.fromInts(0, 0, 0, 150)}>      // ← 半透明遮罩
  <box width={width()} maxWidth={dimensions().width - 2}
       backgroundColor={theme.backgroundPanel}>
```
- **遮罩**：全屏 150/255 黑，把背景内容压暗 —— 这是"模态"的关键视觉信号。
- **固定宽度档**：`medium=60` / `large=88` / `xlarge=116`（不是"屏幕宽度减 8"）。
- **垂直位置**：`paddingTop = 高度 / 4`（约上 1/4 处，不是垂直居中）。
- 点击遮罩关闭；点击内容不关闭。

### 2.2 三条键位约定

`ui/dialog-confirm.tsx` / `permission.tsx`：
- `←/→`（及 `h`/`l`）在选项间移动（**横向**）。
- `enter` 确认当前选项。
- `esc` **等于**"最后一个选项"的语义（确认框里 esc = cancel；
  权限里 esc = reject）。**esc 从不"什么都不做"**。

我们现在 esc 是"收起卡片但不作答"——与 opencode 不一致，用户会觉得"按了没反应"。

---

## 3. 权限（审批）的完整契约

`routes/session/permission.tsx` 的三段式：

### 3.1 主段（permission）
```
┃ △ Permission required                    ← 标题行：warning 色三角 + 标题
┃   → Edit src/foo.ts                      ← 图标 + 具体动作（按权限类型不同）
┃   <diff / 命令 / 路径>                    ← 具体内容
┃ ────────────────────────────────────────
┃ [Allow once] [Allow always] [Reject]     ← 药丸；选中=填充
┃                        ⇆ select  enter confirm
```
**每个权限类型有专属图标与标题**（同一文件里 `info()` 逐个定义）：
| 权限 | 图标 | 标题 |
|---|---|---|
| edit | `→` | `Edit <path>` |
| read | `→` | `Read <path>` |
| glob | `✱` | `Glob "<pattern>"` |
| grep | `✱` | `Grep "<pattern>"` |
| list | `→` | `List <dir>` |
| bash | `#` | `Shell command` + 正文 `$ <cmd>` |
| task | `#` | `<Type> Task` + `◉ <desc>` |
| webfetch | `%` | `WebFetch <url>` |
| websearch | `◈` | `<Provider> "<query>"` |
| external_directory | `←` | `Access external directory <dir>` |
| doom_loop | `⟳` | `Continue after repeated failures` |
| 其它 | `⚙` | `Call tool <name>` |

### 3.2 "总是允许"是**第二段确认**，并列出具体范围
```
Always allow
This will allow the following patterns until OpenCode is restarted
- <pattern 1>
- <pattern 2>
[Confirm] [Cancel]
```
**这一条很重要**：`always` 不直接生效，而是先**展示"将被放行的具体范围"**再确认。
我们目前是直接生效 —— 用户不知道自己到底授权了什么。
另外语义是 **"直到重启"**，不是永久。

### 3.3 拒绝可以**带理由**
`RejectPrompt` 让用户填一句 message，随 `reply: "reject"` 一起回给模型 ——
模型因此知道"为什么被拒"，可以换个做法而不是重试同一件事。

---

## 4. 差距清单（我们 vs opencode）

| # | 项 | opencode | 我们 | 状态 |
|---|---|---|---|---|
| P1 | 底色阶梯 | step1/2/3 固定阶梯 | ✅ backdrop/panel/element/menu/selected 五档（真机验证输出 5 种背景序列） | ✅ 已改 |
| P2 | 面板形态 | 底色 + 左竖条 `┃` | ✅ 审批面板与侧栏均用左竖条（不再是四角框） | ✅ 已改 |
| P3 | 模态 | 遮罩 + 上 1/4 定位 | ✅ 遮罩压暗 + 上 1/4（宽度档未按 60/88/116，用屏宽-6/上限 88） | 🟡 基本对齐 |
| P4 | 选项 | 横向药丸、选中整块填充 | ✅ `允许一次 once / 总是允许 always / 拒绝 reject`，选中整块填充 | ✅ 已改 |
| P5 | 选项条 | 独立底色带(element) | ✅ 底部 element 底色带 | ✅ 已改 |
| P6 | esc 语义 | = 最后一个选项（reject） | ✅ esc = 拒绝 | ✅ 已改 |
| P7 | 权限标题 | 按类型给图标+具体描述 | ✅ `△ 需要审批` + `→ 编辑 <path>` / `# Shell 命令` + `$ <cmd>` | ✅ 已改 |
| P8 | always 语义 | 二次确认 + 列出范围 + "直到重启" | ✅ 确认段列出内核判定的**类别**（`ApprovalRequest.kind` 单一事实源），文案"在 Neo 重启之前，这类调用将不再询问"；esc/ctrl+c 取消回选择段 | ✅ 已改 |
| P9 | 拒绝带理由 | 支持 | ✅ 明确选拒绝（回车/`n`）进理由段，随 `Op::Approve{reason}` 回给模型（进 stderr，模型可见）；esc/ctrl+c = 不带理由快速拒绝 | ✅ 已改 |
| P10 | 键位提示 | 底部右侧常驻 | ✅ `⇆ 选择 enter 确认 esc 拒绝` | ✅ 已改 |
| P11 | 左右键选择 | `←/→` 在选项间移动 | ✅ 已支持（含 h/l） | ✅ 已改 |
| P12 | **ctrl+c 语义（分层）** | 权限框内 = **拒绝**（`app.exit` 被重绑为 Reject）；运行中 = 中断；空闲 = 退出 | ✅ 三段对齐 | ✅ 已改 |
| P13 | 底纹（背景） | 无装饰性底纹；模态用平铺遮罩 | ✅ 模态时关掉星场、铺统一遮罩 | ✅ 已改 |
| P14 | 设置页结构 | 左侧**竖排分类列**导航，右侧只显示该分类的行 | ✅ 左列分类（←→/点击切换），右列只渲染当前分类；光标与分类是两个独立坐标空间 | ✅ 已改 |
| — | 侧栏 | panel 底 + element 悬停 | panel 底 + 竖线 | 🟡 部分 |
| — | 输入框 | 见 `component/prompt/index.tsx` | 已有边框+模型行 | 🟡 部分 |

---

## 4.1 ctrl+c 的分层语义（重要）

opencode 的 `app.exit` 默认绑 `ctrl+c,ctrl+d,<leader>q`
（`config/keybind.ts:48`），但在**权限对话框内被重绑定**为
`Reject permission`（`routes/session/permission.tsx` 里 `name: "app.exit"`
的 `run()` 调 `onSelect(escapeKey)`）。所以：

| 场景 | ctrl+c 的含义 |
|---|---|
| 权限/对话框打开 | **拒绝**（不是退出） |
| 正在运行（回合中） | 中断当前回合（回到空闲） |
| 空闲 | 退出应用 |

我们此前一律 `Key::Quit => break`（直接退出），于是用户在审批框上按
ctrl+c 想取消，**整个会话消失**（用户报的"突然退出会话"）。
现已按上表对齐。

## 5. 落地顺序（按"影响面 × 被复用度"排）

1. **P1 + P2 + P5**：先把**底色阶梯 + 面板/选项条原语**做进渲染层。
   这三条是所有界面的地基 —— 改完设置页、审批、侧栏一起受益。
2. **P3 + P4 + P6 + P10**：对话框原语（遮罩/宽度档/药丸/esc/提示）。
3. **P7 + P8 + P9**：审批内容与三段式语义。
4. 之后才是各页面细节（会话列表、模型选择、MCP…均复用同一套原语）。

> **纪律**：新界面必须复用原语（面板/选项条/对话框），不得再手画
> `╭╮╰╯`。这条能防止"每个页面各长一副样子"——那正是打地鼠的温床。

---

## 6. 复核方式

- 源码：`.cache/ai-external/github/opencode-dev/packages/tui/src/`
  - `ui/border.ts`、`ui/dialog.tsx`、`ui/dialog-confirm.tsx`
  - `routes/session/permission.tsx`（审批完整范式）
  - `theme/assets/opencode.json`（底色阶梯取值）
- 真机对照：本机装有 MiMo（`mimo`），可 pty 抓帧对比观感。
