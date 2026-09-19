# 贡献指南

> **一句话**：这个仓库的门禁**比大多数开源项目严**，而每一条都有它所防的
> 具体缺陷（不是风格偏好）。先读这份，能省掉一轮 CI 红。

---

## 1. 先跑一次门禁（两分钟，先知道尺子在哪）

```bash
bash scripts/verify.sh          # 全套：测试 / 架构 / 协议 / 会话 / SPI / UI 分层 / 可提取性 / 许可证 / 无障碍 / 体积 / 效率
bash scripts/measure-perf.sh    # 性能预算（体积 / 空闲出帧 / 冷启动）—— 较慢，改界面时跑
bash scripts/check-keyboard-reach.sh   # 键盘可达（真机走一遍 Tab 顺序）
```

`verify.sh` 会一次跑完 9 组检查并给出结论。**它报的失败都附了"为什么"**，
先读那句再改代码。

### 工具链

用仓里钉的版本（`rust-toolchain.toml`，当前 1.95）：

```bash
rustup show          # 应当显示 active toolchain = 1.95.0（由 rust-toolchain.toml 覆盖）
```

`verify.sh` 会核对 `cargo` 版本，不一致直接判失败并告诉你为什么 ——
**因为它曾经伪装成编译错误**（旧 cargo 解析不了依赖树，报 `edition2024 is required`，
看着像依赖坏了，实为工具链选错）。

macOS 上如果 `xcrun --show-sdk-path` 报"未同意 Xcode 许可"，链接期测试会失败：

```bash
sudo xcodebuild -license accept                       # 方式 a（推荐）
export DEVELOPER_DIR=/Library/Developer/CommandLineTools   # 方式 b（不需要 sudo）
```

`verify.sh` 的预检会把这两种修法直接打出来。

---

## 2. 硬约束（会被门禁拦住，不是建议）

| 约束 | 为什么 | 谁在拦 |
|---|---|---|
| **零 warning** | warning 是未来错误的温床 | `verify.sh` 第 3 段 |
| **零 `unsafe`**（生产代码） | 内核的安全论证依赖它 | 人工 review + CI |
| **依赖只能向下** `L0→L1→L2→L3→L4→L5` | 宿主反向污染内核是最难拆的债 | `check_architecture.py` |
| **每个 SPI ≥2 真实后端 + conformance** | 只有一个实现的抽象是**信仰**，不是设计 | `check_spi_conformance.py` |
| **工具不得绕开沙箱** | `Tool::execute` 拿的是内核注入的 `ToolCtx`，类型上就做不到 | 架构守卫 + review |
| **模型可见即已落日志** | 否则回放与审计失效 | `check_session_schema.py` + review |
| **内存有界** | Rust 消除 UB，**不保证有界** | 内存测试（具体字节数断言） |
| **效率规范** | 固定盲等/重复拉取/无退出轮询会浪费真实时间 | `audit_efficiency.py`（error 级卡门禁） |
| **无障碍四件套** | 自绘控件默认"既不可访问也不可测试" | `check_a11y.py` |

细节与理由见 [`AGENTS.md`](AGENTS.md) 与 [`PROJECT_MEMORY.md`](PROJECT_MEMORY.md)。

---

## 3. 提交前自检（按这个顺序，能省掉来回）

1. **`cargo test --workspace` 绿**。
2. **改动界面 → 跑 `measure-perf.sh`**：它同时查体积预算与空闲出帧。
3. **改动自绘可点元素 → 跑 `check_a11y.py`**，并在真机上跑
   `check-keyboard-reach.sh`（静态属性齐 ≠ 键盘真能到，这条有实例：
   补齐 `tab_index` 后 `focus_next` 仍原地不动，根因是缺 `track_focus`）。
4. **改动渲染缝 → 跑 `cargo test -p neo-ui-render --test visual_baseline`**；
   若样式是**有意**改动，跑 `-- --ignored refresh` 更新基线，**并在提交信息里说明**。
5. **提交信息写"为什么"**：约束、踩过的坑、被推翻的旧判断。不写"改了什么"
   （diff 已经说了）。

### 提交信息的样子

一份好的提交信息在本仓长这样：

```
fix(ui): 两栏面板纵向溢出 —— h_flex 默认 items_center（潜伏已久的缺陷）

预览本仓 PROJECT_MEMORY.md（5596 行）时暴露：预览栏既向上盖住面板标题、
又向下溢出到主界面上。此前所有验证都只预览短文件，所以它一直存在但从未被看见。

根因（读 gpui 源码确认，不是猜）：
    gpui-base/src/styled.rs:105-107
    fn h_flex() -> Self { self.flex().flex_row().items_center() }

而 flex 行的 align_items 管的正是纵轴 —— 于是两个子栏高度都按内容算、
并上下居中：内容一长就同时向两个方向溢出。
```

要点：**现象 → 根因（带证据/出处）→ 修法 → 影响面**。若某条旧判断被实测推翻，
明说（本仓的 `PROJECT_MEMORY.md` 里这类"实测推翻"记录是最被看重的部分）。

---

## 4. 加东西的时候（最容易踩的三处）

### 新增自绘可点元素

它**必须**四件套齐全，缺一件就是"鼠标能用、键盘与读屏到不了"：

```rust
let h = self.tab_handle(cx, /* 序号 */);   // 焦点句柄（见 app.rs 的 tab_focus 说明）
div()
    .id(/* 稳定 id */)
    .role(accesskit::Role::Button)
    .aria_label(/* 说清"对谁做什么" */)
    .track_focus(&h)        // ⚠️ 少了它，tab_index 登记不进顺序表
    .tab_index(/* 顺序 */)
    .on_click(...)
```

**列表项共用一个句柄**是刻意的：`tab_index` 决定顺序，句柄只需"让人知道这里
可聚焦"。若每项各建句柄，300 行的文件树要按 300 次 Tab 才能走出去。

### 新增 SPI（或改 SPI 契约）

先读 [`docs/spi-first-methodology/`](docs/spi-first-methodology/README.md)。
硬要求：**≥2 个真实后端 + 一份 conformance 用例跑所有后端**。
只有一个实现的抽象层会被 `check_spi_conformance.py` 拒绝 ——
因为"可替换"在没有第二个实现时无法验证。

### 新增产出大输出的路径

必须**受上限约束并如实上报 `truncated`**。截断要"诚实"：报告被截的是什么、
原有多少，而不是静默丢掉。

---

## 5. 两条来自实际事故的调试纪律

**① 反复"不合直觉"时，先读上游源码，别继续调参数。**
本仓多次栽在"猜"上（一次布局问题猜了三轮，读 gpui 源码一次定位）。
依赖的源码在 `~/.cargo/registry/src/*/` 下，可读。

**② 验证方法本身会错，且比代码错更难发现。**
本仓已经记了五次实例：图像坐标与逻辑坐标混用、探针喂了转义输出、
在**被遮挡的窗口**上验 GUI（得到假失败）、用手写时序的测试替身验框架时序、
用 debug 构建测性能。**"我测过了"之前，先问"这个测法能测到什么、不能测到什么"。**

---

## 6. 提 PR 时

- **一个逻辑改动一个 PR**（也是提交的粒度要求）。
- PR 描述里**如实说明没做的事**与已知边界。本仓的文档里"诚实边界"是
  **被高度重视的部分** —— 一个"这部分没验"的说明比一段含糊的"应该没问题"有价值得多。
- 若改了 `PROJECT_MEMORY.md` 里记着的某个判断（尤其是"实测推翻"那条），
  在 PR 里指出并更新它。

---

## 7. 许可

- 本仓代码：**MIT**（见 [LICENSE](LICENSE)）。
- 可开源集（`neo-text` / `neo-ui-kit` / `neo-ui-render` / `neo-ui-behavior` / `neo-ui`）：
  **Apache-2.0**（含明确专利授权），各自目录下有 LICENSE。
- 贡献即表示同意以相应许可发布你的贡献。
- **第三方许可证清单**（`THIRD-PARTY-LICENSES.md`）由脚本生成，
  改动依赖后跑 `python3 scripts/gen_third_party_licenses.py` 更新，
  否则门禁会红。
