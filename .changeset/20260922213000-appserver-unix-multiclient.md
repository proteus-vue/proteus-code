---
bump: minor
---

`neo app-server --listen unix://<路径>`：本地多客户端共用一个内核（事件广播、响应回发起连接、审批同看同控、`shutdown` 全局收尾、断开一条不影响另一条）。socket 文件权限即信任边界；TCP 刻意未做（要鉴权，远程客户端走 `neo serve`）。
