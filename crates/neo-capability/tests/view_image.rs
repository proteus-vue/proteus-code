//! `view_image`：读图 → 元数据 + 可注入模型的 ImageAttachment。
//!
//! 用 1×1 PNG 字节头（真实文件）验证 MIME/上限/缺参，不引图片解码库。

use neo_capability::ViewImageTool;
use neo_core::{Tool, ToolCtx};
use neo_protocol::SandboxMode;
use serde_json::json;
use std::path::PathBuf;

/// 最小合法 1×1 红点 PNG（69 字节）。
fn tiny_png() -> Vec<u8> {
    fn chunk(tag: &[u8], data: &[u8]) -> Vec<u8> {
        let mut c = Vec::new();
        c.extend_from_slice(&(data.len() as u32).to_be_bytes());
        c.extend_from_slice(tag);
        c.extend_from_slice(data);
        let mut crc_input = tag.to_vec();
        crc_input.extend_from_slice(data);
        let mut crc = 0xFFFF_FFFFu32;
        for b in &crc_input {
            crc ^= *b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        c.extend_from_slice(&(!crc).to_be_bytes());
        c
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]));
    // IDAT: zlib 红像素（filter0 + RGB）—— 非零即可，解码不在本测范围
    out.extend(chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0xff, 0xff]));
    out.extend(chunk(b"IEND", &[]));
    out
}

fn tmpdir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("neo-view-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn ctx_for(ws: &std::path::Path) -> ToolCtx<'_> {
    struct Null;
    impl neo_core::SandboxBackend for Null {
        fn supports(&self, _m: SandboxMode) -> bool {
            true
        }
        fn write_file(
            &self,
            _m: SandboxMode,
            _p: &std::path::Path,
            _c: &str,
        ) -> neo_core::FileOutcome {
            neo_core::FileOutcome::Written { bytes: 0 }
        }
        fn execute(
            &self,
            _m: SandboxMode,
            _c: &str,
            _l: usize,
        ) -> neo_core::SandboxOutcome {
            neo_core::SandboxOutcome::Ran {
                stdout: String::new(),
                truncated: false,
            }
        }
    }
    // 测试用泄漏沙箱（与 kernel_loop 同款夹具手法）
    let sb: &'static Null = Box::leak(Box::new(Null));
    ToolCtx {
        sandbox: sb,
        mode: SandboxMode::WorkspaceWrite,
        cwd: ws,
        max_output_bytes: 64 * 1024,
    }
}

#[test]
fn view_image_is_in_defaults_and_reads_png_metadata() {
    use neo_core::{CallKind, ToolRegistry};
    let mut reg = ToolRegistry::new();
    neo_capability::register_defaults(&mut reg);
    let t = reg.get("view_image").expect("view_image 应在默认工具集");
    assert_eq!(t.call_kind(&json!({"path": "a.png"})), CallKind::Read);

    let ws = tmpdir("png");
    std::fs::write(ws.join("a.png"), tiny_png()).unwrap();
    let out = t.execute(&json!({"path": "a.png"}), &ctx_for(&ws));
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("mime=image/png"), "{}", out.stdout);
    assert!(out.stdout.contains("path="), "{}", out.stdout);

    let img = t
        .image_result(&json!({"path": "a.png"}), &out)
        .expect("成功执行应能取到图片");
    assert_eq!(img.mime, "image/png");
    assert!(img.bytes > 0);
    assert!(!img.data_base64.is_empty());
    assert!(img.path.ends_with("a.png"));

    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn view_image_rejects_missing_path_and_oversize() {
    let ws = tmpdir("bad");
    let out = ViewImageTool.execute(&json!({}), &ctx_for(&ws));
    assert_eq!(out.exit_code, -1);
    assert!(out.stderr.contains("path"), "{}", out.stderr);

    // 超上限：>1MiB
    let big = vec![0u8; 1024 * 1024 + 1];
    std::fs::write(ws.join("big.png"), big).unwrap();
    let out = ViewImageTool.execute(&json!({"path": "big.png"}), &ctx_for(&ws));
    assert_eq!(out.exit_code, -1);
    assert!(out.stderr.contains("过大"), "{}", out.stderr);

    let _ = std::fs::remove_dir_all(&ws);
}
