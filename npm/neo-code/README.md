# neo-code (npm)

以 npm 方式安装 [NEO](https://github.com/proteus-vue/proteus-code) 的命令行 `neo`。

```bash
npm install -g neo-code
neo --help
```

## 它是怎么工作的

本包**不含二进制**，只装与你平台匹配的那个可选依赖：

| 平台 | 平台包 |
|---|---|
| macOS (Apple Silicon) | `neo-code-darwin-arm64` |
| macOS (Intel) | `neo-code-darwin-x64` |
| Linux x86_64 | `neo-code-linux-x64` |

npm 依据平台包的 `os` / `cpu` 字段自动只装匹配项。**安装期不执行脚本、
不联网**（不走 postinstall 下载），离线与内网环境同样可用。

## 平台与功能边界

- **Linux 包不含桌面窗口宿主**：桌面窗口需要 `libwebkit2gtk`，预编译产物
  刻意不带它，以免纯终端用户被迫安装整套 webkit。需要 Linux 桌面窗口请：
  ```bash
  cargo install --git https://github.com/proteus-vue/proteus-code -p neo-code-cli --locked
  ```
- 其它平台（Linux arm64、Windows、musl）暂无预编译产物，`neo` 会给出提示。
- **沙箱**：macOS 使用系统 Seatbelt；Linux / Windows 的受限档位
  **fail-closed**（拒绝执行），不会降级放行。

## 许可

MIT。
