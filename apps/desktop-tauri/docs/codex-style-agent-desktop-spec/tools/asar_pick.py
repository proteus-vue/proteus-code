#!/usr/bin/env python3
"""从 asar 中按路径片段选择性提取文件（避免整体解包数百 MB）。

用法: python3 asar_pick.py <app.asar> <输出目录> <路径片段1> [片段2 ...]
"""
import json
import os
import struct
import sys


def load(asar_path):
    with open(asar_path, "rb") as f:
        u = struct.unpack("<IIII", f.read(16))
        f.seek(16)
        tree = json.loads(f.read(u[3]).decode("utf-8"))
    return tree["files"], 8 + u[1]


def main():
    asar, out = sys.argv[1], sys.argv[2]
    pats = sys.argv[3:]
    files, base = load(asar)
    hits = []

    def walk(node, prefix):
        for name, item in node.items():
            path = f"{prefix}/{name}" if prefix else name
            if "files" in item:
                walk(item["files"], path)
            elif any(p in path for p in pats):
                hits.append((path, item))

    walk(files, "")
    print(f"命中 {len(hits)} 个文件")

    with open(asar, "rb") as f:
        for path, item in hits:
            if "offset" not in item:
                continue
            dest = os.path.join(out, path)
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            f.seek(int(item["offset"]) + base)
            with open(dest, "wb") as w:
                w.write(f.read(item["size"]))
            print(f"{item['size'] / 1024:9.1f}KB  {path}")


if __name__ == "__main__":
    main()
