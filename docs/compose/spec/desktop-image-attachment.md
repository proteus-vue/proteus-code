---
feature: desktop-image-attachment
status: in-progress
updated: 2026-09-24
branch: main
commits: 
---

# Desktop Image Attachment

## Report

## [S1] Problem
桌面输入区没有图片入口（粘贴 / 拖放 / 文件选择）。内核已有 `view_image(path)`：读本地图 → `ImageAttachment` → `UserImage` → provider `image_url`（IA-42）。用户无法把图片送进会话；PRODUCT-IA IA-10 / P2 图片附件仍为 ⬜。

关键约束：`turn/start` 的 `@path` 走 `resolve_refs` → `cat` 注入**文本**，对二进制图会得到乱码/超界，**不能**用 `@` 当图片通道。图片必须：**落盘路径 → 模型调用 `view_image`**。

## [S2] Design
- **入口（已选）**：Composer **粘贴 + 拖放 + 按钮选图**（`image/*` 或 png/jpg/webp/gif）。
- **落盘**：写入 `{workspace}/.neo/attachments/`（或无 workspace 时 `$NEO_HOME/attachments/`），文件名 `img-<ts>.<ext>`；**上限 1MiB**（与 `ViewImageTool::MAX_IMAGE_BYTES` 一致），超限拒绝并提示。
- **状态**：`imagePicks: { path, name, bytes }[]`；chip 展示可删（对齐 webPicks 的 attach-chip，多文件 `+N`）。
- **发送**：`send()` 将每个 path 写入正文为 **非 `@` 的明确指令**（避免 `resolve_refs` cat）：  
  `请先 view_image 查看：/abs/or/ws-relative/path` + 用户文本。多图多行。发送后清空 `imagePicks`。
- **内核小改（T1）**：`resolve_refs` 对 `RefKind::File` 若路径为图片扩展名（png/jpg/jpeg/webp/gif）→ **不 cat**，summary 改为「🖼 图片路径已提供，请调用 view_image(path)」，`block` 不塞二进制。这样用户手打 `@img.png` 也不污染上下文。
- **事件**：桌面处理 `image_attached` → 转录显示一行路径状态（可选缩略图用 `<img src="file://…">` 仅本地；无则纯文本）。有界、不 base64 进 transcript。
- **错误**：非图片 MIME / >1MiB / 写失败 → composer 错误提示，不静默。

## [S3] Out of Scope
- 不改 `view_image` 语义与 1MiB 上限。
- 不做多图网格编辑、OCR、云端上传。
- 不做 `windowsSandbox` / 账号相关。
- 不做 `thread/attachment` 侧车 UI（那是会话元数据，不是模型上下文）。

## Tasks
- [x] T1: 内核 `resolve_refs` 图片扩展名跳过 cat，summary 提示 view_image — acceptance: 单测：`@a.png` 不注入二进制 body，summary 含 view_image (covers: S2)
- [x] T2: 桌面粘贴/拖放/选图 → 落盘 + `imagePicks` chip（可删、超限报错） — acceptance: 三种入口都能出 chip；>1MiB 被拒 (covers: S2; depends: T1)
- [x] T3: `send()` 合并 imagePicks 为非 @ 的 view_image 指令并清空 — acceptance: 发送后消息含 path+view_image 提示，chips 清空 (covers: S2; depends: T2)
- [x] T4: 样式 + PRODUCT-IA 回填 + `tsc`/`build`/效率审计 — acceptance: chip 与 attach-chip 一致；IA-10/P2 勾选；门禁绿 (covers: S2; depends: T3)
