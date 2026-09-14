#!/usr/bin/env node
'use strict';

// neo 的 npm 入口 —— 只负责"找到本平台的真二进制并原样转发"。
//
// 为什么不是一个下载器：本包把二进制放在**按平台拆分的可选依赖**里
// （neo-code-<os>-<arch>，各自带 os/cpu 字段，npm 只装匹配的那一个）。
// 好处是**安装期不执行任何脚本、不联网**（postinstall 下载正是供应链
// 注入的常见入口，本项目在安全上刻意避开），离线/内网也照常工作。
//
// 这个 wrapper 自身不含业务逻辑：参数、退出码、信号都原样透传。

const { spawnSync } = require('node:child_process');
const path = require('node:path');
const fs = require('node:fs');

// 平台 → 平台包名。新增平台时这里与 release.yml 的产物矩阵要一起改。
const PKG_BY_PLATFORM = {
  'darwin-arm64': 'neo-code-darwin-arm64',
  'darwin-x64': 'neo-code-darwin-x64',
  'linux-x64': 'neo-code-linux-x64',
};

function fail(message) {
  process.stderr.write(`neo: ${message}\n`);
  process.exit(1);
}

const key = `${process.platform}-${process.arch}`;
const pkgName = PKG_BY_PLATFORM[key];

if (!pkgName) {
  fail(
    `本平台（${key}）没有预编译产物。\n` +
      `  可选方案：\n` +
      `    • 安装脚本（macOS / Linux x64）：https://github.com/proteus-vue/proteus-code#安装\n` +
      `    • 用 Rust 工具链：\n` +
      `      cargo install --git https://github.com/proteus-vue/proteus-code -p neo-code-cli --locked`
  );
}

let pkgJsonPath;
try {
  // 用 require.resolve 而非猜路径：这样 npm 的扁平化/pnpm 的符号链接都成立。
  pkgJsonPath = require.resolve(`${pkgName}/package.json`);
} catch {
  fail(
    `缺少平台包 ${pkgName}。常见原因：安装时跳过了可选依赖\n` +
      `（--no-optional / --ignore-scripts / 离线且未预取），或该平台包发布失败。\n` +
      `  可重装试试：npm i -g neo-code`
  );
}

const binary = path.join(path.dirname(pkgJsonPath), 'bin', 'neo');
if (!fs.existsSync(binary)) {
  fail(`平台包内找不到可执行文件：${binary}`);
}

// npm 解包不保证保留可执行位 —— 幂等补上；失败不致命（可能是只读挂载）。
try {
  fs.chmodSync(binary, 0o755);
} catch {
  /* 交给 spawn 报错 */
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });

if (result.error) {
  fail(`无法启动 ${binary}：${result.error.message}`);
}
// 被信号终止时 status 为 null —— 按惯例以 1 退出，避免被当成成功。
process.exit(result.status === null ? 1 : result.status);
