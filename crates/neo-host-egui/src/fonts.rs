//! 中文字体：egui 默认字体**不含 CJK 字形**，不装就是满屏豆腐块。
//!
//! # 这个缺陷是怎么发现的（以及为什么非发现不可）
//!
//! 界面全部验收（编译、开窗、进程存活、驱动层回归测试）**全绿**，但第一次真机
//! 截图就看到：`NEO` / `Default` / `WorkspaceWrite` 这些拉丁文字正常，**所有中文
//! 都是 `□`**。egui 自带字体只覆盖 latin + cyrillic（官方文档明写），而本项目
//! 的界面文案是中文 —— 也就是说，**不装中文字体 = 界面不可用**，而这一点任何
//! 编译期与单元测试都不可能发现。截图是唯一能发现它的手段。
//!
//! # 为什么不把字体打包进仓库
//!
//! 候选字体都是 20 MB 量级（`Hiragino Sans GB.ttc` 23 MB、`STHeiti` 55 MB）。
//! 放进仓库既让仓库膨胀，也牵扯字体授权（系统字体的再分发通常不允许）。
//! 所以按**平台候选列表**在运行时从系统加载：系统里有就用，没有就如实报告。
//!
//! # 为什么"没有中文字体"要能被调用方知道
//!
//! 静默失败的表现是"界面能开但看不懂"，比直接报错更难排查。所以
//! [`install`] 返回是否装上了，调用方据此在状态行给出提示。

use egui::{FontData, FontFamily};

/// 一个候选字体：路径 + 集合内的 face 索引。
///
/// `.ttc` 是**字体集合**（一个文件含多个 face），索引 0 通常是 Regular 字重。
/// egui 的 `FontData.index` 正是为这种情况准备的。
struct Candidate {
    path: &'static str,
    index: u32,
    /// 人可读的名字，用于诊断输出。
    name: &'static str,
}

/// 按平台给候选：**用系统自带的字体，不从网络拉**（避免引入网络依赖与不确定性）。
///
/// 顺序按"UI 观感"排：黑体/无衬线优于宋体（衬线在界面上偏"文档"）。
fn candidates() -> &'static [Candidate] {
    #[cfg(target_os = "macos")]
    {
        &[
            // macOS 中文界面标准字体（W3 = Regular）
            Candidate { path: "/System/Library/Fonts/Hiragino Sans GB.ttc", index: 0, name: "Hiragino Sans GB" },
            Candidate { path: "/System/Library/Fonts/PingFang.ttc", index: 0, name: "PingFang SC" },
            Candidate { path: "/System/Library/Fonts/STHeiti Light.ttc", index: 0, name: "STHeiti Light" },
            Candidate { path: "/System/Library/Fonts/STHeiti Medium.ttc", index: 0, name: "STHeiti Medium" },
        ]
    }
    #[cfg(target_os = "windows")]
    {
        &[
            // 微软雅黑：Windows 中文界面标准字体
            Candidate { path: "C:/Windows/Fonts/msyh.ttc", index: 0, name: "Microsoft YaHei" },
            Candidate { path: "C:/Windows/Fonts/msyh.ttf", index: 0, name: "Microsoft YaHei (ttf)" },
            Candidate { path: "C:/Windows/Fonts/simhei.ttf", index: 0, name: "SimHei" },
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &[
            // 各发行版路径不一，覆盖最常见的几处
            Candidate { path: "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", index: 0, name: "Noto Sans CJK" },
            Candidate { path: "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", index: 0, name: "Noto Sans CJK" },
            Candidate { path: "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", index: 0, name: "WenQuanYi Zen Hei" },
            Candidate { path: "/usr/share/fonts/wenquanyi/wqy-zenhei/wqy-zenhei.ttc", index: 0, name: "WenQuanYi Zen Hei" },
        ]
    }
}

/// 装中文字体；返回装上时的字体名。
///
/// `None` 表示**没找到任何中文字体** —— 调用方应当提示用户（否则界面是豆腐块）。
pub fn install(ctx: &egui::Context) -> Option<&'static str> {
    let (data, name, index) = load_first()?;

    let mut font = FontData::from_owned(data);
    font.index = index;

    // 两个 family 都要装：正文用 Proportional，代码/工具输出用 Monospace。
    // 只装一个的话，另一半界面照样是豆腐块。
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        ctx.add_font(egui::epaint::text::FontInsert {
            name: format!("cjk-{name}-{}", family_name(&family)),
            data: font.clone(),
            families: vec![egui::epaint::text::InsertFontFamily {
                family,
                // Fallback：拉丁字符仍用 egui 自带字体（中文字体里的拉丁字形
                // 未必比 egui 的好看），缺字时才回退到中文字体。
                priority: egui::epaint::text::FontPriority::Lowest,
            }],
        });
    }
    Some(name)
}

fn family_name(f: &FontFamily) -> &'static str {
    match f {
        FontFamily::Proportional => "prop",
        FontFamily::Monospace => "mono",
        _ => "other",
    }
}

/// 找第一个**存在且能读**的候选字体，返回 (字节, 名字, face 索引)。
///
/// 只读一次：字体文件 20 MB 量级，重复读会白花时间与内存。
fn load_first() -> Option<(Vec<u8>, &'static str, u32)> {
    for c in candidates() {
        if let Ok(bytes) = std::fs::read(c.path) {
            // 空文件或不存在的路径读出来会是 Err，但也防一手"文件存在却是空的"
            if !bytes.is_empty() {
                return Some((bytes, c.name, c.index));
            }
        }
    }
    None
}

/// **自检**：装好字体后，界面里的中文是否**真的能画出来**。
///
/// # 为什么需要这条（本阶段最该带走的一课）
///
/// "中文是豆腐块"这个缺陷躲过了**所有**机器检查：编译通过、开窗成功、进程存活、
/// 驱动层回归测试全绿 —— 因为它不是逻辑错误，而是**字体缺字形**。
/// 唯一发现它的手段是真机截图。
///
/// 但"只能靠人眼"不是可接受的终局：egui 的 `Fonts::has_glyph()` 能直接回答
/// "这个字体有这些字的字形吗"。于是把"能不能显示中文"变成**可机器判定**的断言 ——
/// 一个真实的**界面可用性门禁**，而不是又一次"看起来没问题"。
///
/// 做法：`Context::run_ui` 不需要窗口，无头环境也能跑一帧装好字体，
/// 然后查字形表。失败时给出**具体缺哪个字**，便于定位。
pub fn verify_cjk_renderable(ctx: &egui::Context) -> Result<(), String> {
    // 跑一帧（无窗口）：字体在 pass 里才生效，不跑帧查不到
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(320.0, 200.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        ui.label("中文字形自检");
    });
    // 必须显式消费纹理增量：epaint 在析构时会 panic 提醒"有未处理的 delta"
    // （它假定调用方是渲染后端，会把这些 delta 上传成 GPU 纹理）。
    // 这里是自检、不渲染，所以主动 clear 掉 —— 这正是那条 panic 提示的建议
    // （"If you want to drop this intentionally call `clear` before dropping"）。
    out.textures_delta.clear();

    // 界面里**真正会出现**的字：状态词 / 面板标题 / 输入提示 / 按钮。
    // 不查生僻字：那只说明字体覆盖不全，不代表界面不可用。
    let probe = "就绪目标未设定输入任务发送开始暂停恢复清除允许拒绝审批沙箱文件编辑";
    let mut missing = Vec::new();
    // has_glyph 需要 &mut（首次查询会懒加载字形），故用 fonts_mut
    ctx.fonts_mut(|f| {
        for ch in probe.chars() {
            let has = [
                egui::FontId::proportional(14.0),
                egui::FontId::monospace(13.0),
            ]
            .into_iter()
            .any(|id| f.has_glyph(&id, ch));
            if !has {
                missing.push(ch);
            }
        }
    });

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "这些中文字没有字形（界面会显示成方块）：{}",
            missing.iter().collect::<String>()
        ))
    }
}

/// 供诊断：本次会选中哪个字体（不读文件内容，只看候选是否存在）。
///
/// 与 [`install`] 分开是为了能**不加载 20 MB 字体**就测出"这台机器上有没有"，
/// 让测试可以断言候选表的形状而不依赖具体机器装了哪些字体。
pub fn first_existing_candidate() -> Option<&'static str> {
    candidates()
        .iter()
        .find(|c| std::path::Path::new(c.path).is_file())
        .map(|c| c.name)
}

/// 候选表是否为空 —— 空表说明这个平台**完全没有**可选字体，
/// 那是配置遗漏，不是"这台机器没装"。
pub fn has_candidates() -> bool {
    !candidates().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_has_a_candidate_list() {
        // 空候选表 = 目标平台被漏掉 = 该平台上界面必定是豆腐块
        assert!(
            has_candidates(),
            "本平台没有配置任何中文字体候选 —— 界面会全是豆腐块"
        );
    }

    #[test]
    fn candidate_entries_are_absolute_paths_with_a_display_name() {
        for c in candidates() {
            assert!(
                c.path.starts_with('/') || c.path.contains(":/"),
                "字体路径必须是绝对的（相对路径会随工作目录变化）：{}",
                c.path
            );
            assert!(!c.name.is_empty(), "候选 {} 缺少显示名", c.path);
        }
    }

    /// 字体加载**不该**假定系统里一定有 —— 缺字体是正常情况，不是异常。
    #[test]
    fn loading_is_graceful_when_no_font_is_present() {
        // 只要不 panic 就算通过：本机装了 Hiragino，但 CI（Linux 容器）多半没装
        let found = first_existing_candidate();
        if let Some(name) = found {
            assert!(!name.is_empty());
        }
        // 无论找到与否，install 的契约是返回 Option，绝不 panic
    }

    /// **本机验收**：macOS 上必须能找到中文字体。
    ///
    /// 这条在 macOS 上失败意味着"用户看到豆腐块"，所以在这台机器上它必须是硬的。
    /// 在其它平台跳过 —— 那些平台的字体部署不在本项目的控制内，由
    /// `first_existing_candidate` 的 Option 语义覆盖。
    #[test]
    #[cfg(target_os = "macos")]
    fn macos_always_has_a_cjk_font() {
        assert!(
            first_existing_candidate().is_some(),
            "macOS 上找不到任何中文字体候选：{:?} —— 界面会全是豆腐块",
            candidates().iter().map(|c| c.path).collect::<Vec<_>>()
        );
    }
    /// **本项目的界面可用性门禁**：装完字体后，中文必须真的能画出来。
    ///
    /// 这条测试存在的全部意义是"让截图才能发现的问题变成机器能发现的"。
    /// 它跑在无头环境（`run_ui` 不需要窗口），所以 CI 也能跑。
    ///
    /// 反例验证过：不调用 `install` 时这条**必然失败**（见下一条测试），
    /// 说明它测的是真实渲染能力，不是"函数被调用过"。
    #[test]
    fn cjk_text_is_actually_renderable_after_install() {
        let ctx = egui::Context::default();
        let installed = install(&ctx);
        if installed.is_none() {
            // 本机没有任何候选字体：这不是界面 bug，是环境缺字体。
            // 报出来但不失败 —— 失败会让"没装中文字体的 CI 容器"红掉，
            // 而那与代码质量无关。macOS 上有单独的硬断言（见下）。
            eprintln!("⚠ 本机无中文字体候选，跳过渲染自检");
            return;
        }
        if let Err(e) = verify_cjk_renderable(&ctx) {
            panic!("装上字体后中文仍不可渲染：{e}");
        }
    }

    /// 反例：**不装**字体时，中文必定不可渲染。
    ///
    /// 这条证明上面那个自检有牙齿 —— 它确实能区分"装了"与"没装"，
    /// 而不是永远返回 Ok 的摆设。
    #[test]
    fn without_install_cjk_is_not_renderable() {
        let ctx = egui::Context::default();
        // 刻意不调 install：egui 自带字体只有 latin + cyrillic
        let result = verify_cjk_renderable(&ctx);
        assert!(
            result.is_err(),
            "egui 默认字体不含 CJK，未装中文字体时自检必须报错（否则该自检没有牙齿）"
        );
        let msg = result.unwrap_err();
        assert!(msg.contains("方块"), "错误信息要说明后果：{msg}");
    }
}

