---
bump: patch
---

修复 Web 宿主线格式丢字段：`ApprovalRequest` 自带的 `kind`（内核判定的放行范围 read/write/network/interactive）被事件名覆盖 —— 现在完整原始载荷挂在 `payload` 下（与会话日志、app-server 通知同形），顶层 `kind` 恒为事件名；单元变体（如 `shutdown_complete`）的 `kind` 不再是空串。
