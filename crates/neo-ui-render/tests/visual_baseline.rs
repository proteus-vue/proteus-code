//! 视觉回归基线 —— **把"自绘表面有没有被改样"变成一次字节比较**。
//!
//! # 它补的是 Phase 3 里"截图基线"缺的那一半
//!
//! 方案的 DoD 要求「每个 P1 组件 × 亮暗主题 × 2 档 DPI 的截图基线」。
//! 真截图的路径在 CI 里走不通（要窗口、要 GPU、要显示服务器），而
//! `HeadlessBackend` 只导出 SVG —— **SVG 能看，不能比**（没有像素）。
//!
//! 所以 `raster` 把中立场景真的画成像素，这里拿它建基线。
//!
//! # 它守的是什么（说清"有牙齿"在哪）
//!
//! 基线测试最容易变成"盖章"：只要生成一次、之后永远通过。所以本文件里有
//! **两类测试**：
//!
//! 1. **基线本身**：`baselines_match` —— 与 `tests/baselines/*.png` 逐字节比较。
//! 2. **牙齿自证**：`the_baseline_can_actually_fail` —— 故意改一个颜色，
//!    断言基线**真的会红**。没有这条，第一类测试可能是"永远通过"的（比如
//!    比较函数写错、路径找不到而静默跳过）。
//!
//! 这正是本仓对守卫的一贯要求：**抓不住反例的套件没有牙齿**。
//!
//! # 刷新基线
//!
//! ```text
//! cargo test -p neo-ui-render --test visual_baseline -- --ignored refresh
//! ```
//!
//! `refresh` 被标了 `#[ignore]`：它**写文件**，必须是人显式要的动作。
//! 让它在普通测试里跑，等于"每次改样式都自动更新基线" —— 那基线就永远绿，
//! 也就永远不会告诉任何人"样式变了"。
//!
//! # 为什么用真消费者的场景（不用手搓的玩具场景）
//!
//! 与 `conformance.rs` 同一个理由：手搓一个只有两个矩形的场景，两个后端
//! "都很容易通过"，而真实消费者（diff 底带、用量条、分段进度、变更条）里
//! 才有对齐、夹取、比例这些容易错的地方。四个场景全部取自线上代码。

use neo_text::palette::NEO;
use neo_text::Tone;
use neo_ui_render::raster::{rasterize, Raster};
use neo_ui_render::{
    change_gutter, diff_backdrop, segmented_progress, usage_bars, Color, DiffBand, GutterMark, Op,
    ProgressSegment, Scene, UsageBar,
};

/// 画布与 DPI 档位。
///
/// 两档：`1.0`（基准）与 `2.0`（Retina）。**DPI 那一维的意义**：线宽、圆角、
/// 1px 的分隔线都在缩放后才会暴露"会不会消失/糊掉"。方案要的是四档
/// （100/125/150/200%），这里先做整数倍两档 —— 分数倍缩放需要更细的
/// 像素对齐语义（真机上也还没验过，见缺口表），不假装覆盖。
const DPI: &[(f32, &str)] = &[(1.0, "1x"), (2.0, "2x")];

/// 一个基线用例：名字 + 逻辑尺寸 + 场景 + 底色。
struct Case {
    name: &'static str,
    w: f32,
    h: f32,
    scene: Scene,
    bg: Color,
}

/// 全部基线用例。
///
/// **底色的选择有讲究**：宿主实际是在面板底（`bg_panel`）上铺这些自绘表面的，
/// 所以基线用 `bg_panel` 而不是纯黑 —— 半透明的底带（diff 那套 alpha=32/76）
/// 与底色的合成结果正是真机上看到的样子。用纯黑会让"alpha 调错"这类改动
/// 的像素差变小，从而更容易漏过。
fn cases() -> Vec<Case> {
    let panel = {
        let (r, g, b) = NEO.bg_panel;
        Color::rgb(r, g, b)
    };
    vec![
        Case {
            name: "diff_backdrop",
            w: 480.0,
            h: 140.0,
            // 一段典型的审批预览：hunk 头 → 上下文 → 删除 → 新增 → 折叠标记
            scene: diff_backdrop(
                &[
                    DiffBand::Header,
                    DiffBand::Hunk,
                    DiffBand::Context,
                    DiffBand::Del,
                    DiffBand::Del,
                    DiffBand::Add,
                    DiffBand::Add,
                    DiffBand::Fold,
                ],
                480.0,
                17.5,
            ),
            bg: panel,
        },
        Case {
            name: "usage_bars",
            w: 320.0,
            h: 80.0,
            scene: usage_bars(
                &[
                    UsageBar::new(1200, 340),
                    UsageBar::new(400, 90),
                    UsageBar::new(9000, 2100),
                    UsageBar::new(50, 10),
                ],
                320.0,
                80.0,
                Color::from_tone(&NEO, Tone::Primary),
                Color::from_tone(&NEO, Tone::Accent),
                4.0,
            ),
            bg: panel,
        },
        Case {
            name: "segmented_progress",
            w: 400.0,
            h: 24.0,
            scene: segmented_progress(
                &[
                    ProgressSegment::new(5, 5),
                    ProgressSegment::new(3, 5),
                    ProgressSegment::new(0, 5),
                    ProgressSegment::new(2, 5),
                ],
                400.0,
                12.0,
                Color::from_tone(&NEO, Tone::Primary),
                Color::from_tone(&NEO, Tone::Border),
                3.0,
            ),
            bg: panel,
        },
        Case {
            name: "change_gutter",
            w: 16.0,
            h: 200.0,
            // 20 行里几处改动（含相邻两行 —— 考"相邻的块会不会粘成一块"）
            scene: change_gutter(
                &{
                    let mut m = vec![GutterMark::Plain; 20];
                    m[2] = GutterMark::Add;
                    m[3] = GutterMark::Add;
                    m[9] = GutterMark::Del;
                    m[15] = GutterMark::Add;
                    m[16] = GutterMark::Add;
                    m
                },
                8.0,
                200.0,
            ),
            bg: panel,
        },
    ]
}

fn render(case: &Case, scale: f32) -> Raster {
    rasterize(&case.scene, case.w, case.h, scale, case.bg)
}

fn baseline_path(case: &Case, dpi: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/baselines")
        .join(format!("{}.{}.png", case.name, dpi))
}

/// **基线比对**：每个用例 × 每档 DPI。
///
/// 逐字节比较，**没有容差** —— 光栅化是我们自己算的（无 GPU 驱动差异、无字体
/// 差异、无浮点平台差异），所以"差一个字节"就是真的变了。这比"允许差 N 个
/// 像素"强得多：容差正是视觉回归测试慢慢失效的原因。
#[test]
fn baselines_match() {
    let mut failures: Vec<String> = Vec::new();

    for case in cases() {
        for (scale, dpi) in DPI {
            let got = render(&case, *scale);
            let path = baseline_path(&case, dpi);
            let expected = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    failures.push(format!(
                        "{}@{}：读不到基线 {}（{e}）—— 首次生成用 --ignored refresh",
                        case.name,
                        dpi,
                        path.display()
                    ));
                    continue;
                }
            };
            let mine = got.to_png();
            if mine != expected {
                failures.push(format!(
                    "{}@{}：渲染结果与基线不同（基线 {} 字节，本次 {} 字节，画布 {}×{}）\
                     —— 若这是有意的样式改动，跑 --ignored refresh 更新基线并在提交信息里说明",
                    case.name,
                    dpi,
                    expected.len(),
                    mine.len(),
                    got.width,
                    got.height
                ));
            }
        }
    }

    assert!(failures.is_empty(), "视觉回归基线不匹配：\n  {}", failures.join("\n  "));
}

/// **牙齿自证**：基线真的会红。
///
/// 做法：拿一个真实用例，改它一个**语义上确实可见**的东西（删掉一条底带），
/// 断言渲染结果与基线不同。没有这条，`baselines_match` 可能因为"比较逻辑写错"
/// 或"路径找不到而静默跳过"而永远通过 —— 那种套件比没有更糟（它给人一种
/// "有回归保护"的错觉）。
#[test]
fn the_baseline_can_actually_fail() {
    let case = cases().into_iter().find(|c| c.name == "diff_backdrop").expect("有该用例");
    let original = render(&case, 1.0).to_png();

    // 改一处：把一段"删除行"改成"上下文行"（底带少两条）
    let mutated_scene = diff_backdrop(
        &[
            DiffBand::Header,
            DiffBand::Hunk,
            DiffBand::Context,
            DiffBand::Context, // 原本是 Del（有红底）
            DiffBand::Context, // 原本是 Del
            DiffBand::Add,
            DiffBand::Add,
            DiffBand::Fold,
        ],
        480.0,
        17.5,
    );
    let mutated = rasterize(&mutated_scene, case.w, case.h, 1.0, case.bg).to_png();

    assert_ne!(
        original, mutated,
        "改了两条底带却渲染出相同像素 —— 基线没有牙齿（比较或渲染有问题）"
    );
    // 顺带确认基线文件本身与"原始场景"一致（否则上面这条比较是空转）
    let path = baseline_path(&case, "1x");
    if let Ok(expected) = std::fs::read(&path) {
        assert_eq!(
            original,
            expected,
            "基线 {} 与当前渲染不一致 —— 先跑 baselines_match 看是不是有意的改动",
            path.display()
        );
    }
}

/// **刷新基线**（显式动作，见文件头）。
#[test]
#[ignore = "写文件：只有人要更新基线时才跑（--ignored refresh）"]
fn refresh() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/baselines");
    std::fs::create_dir_all(&dir).unwrap();
    for case in cases() {
        for (scale, dpi) in DPI {
            let r = render(&case, *scale);
            let path = baseline_path(&case, dpi);
            std::fs::write(&path, r.to_png()).unwrap();
            eprintln!(
                "写入 {}（{}×{}，{} 字节）",
                path.display(),
                r.width,
                r.height,
                std::fs::metadata(&path).unwrap().len()
            );
        }
    }
}

// ── 光栅化器自身的契约 ────────────────────────────────────────────────────
//
// 这些不依赖基线文件，测的是"画得对不对"的基本事实。它们保证基线不是
// "把某个 bug 的产物固化下来"。

/// 底色被保住：空场景 = 纯底色（也验证了 alpha 合成没把底板搞坏）。
#[test]
fn an_empty_scene_is_exactly_the_background() {
    let bg = Color::rgb(0x20, 0x1e, 0x2a);
    let r = rasterize(&Scene::new(), 8.0, 4.0, 1.0, bg);
    assert_eq!((r.width, r.height), (8, 4));
    for y in 0..4 {
        for x in 0..8 {
            assert_eq!(r.pixel(x, y), Some([0x20, 0x1e, 0x2a, 255]), "({x},{y})");
        }
    }
}

/// 不透明填充：像素就是那个颜色（含边界像素 —— 半开区间 `[x0, x1)`）。
#[test]
fn an_opaque_fill_covers_exactly_its_pixels() {
    let mut s = Scene::new();
    s.fill(neo_ui_render::Rect::new(2.0, 1.0, 3.0, 2.0), Color::rgb(255, 0, 0));
    let r = rasterize(&s, 8.0, 4.0, 1.0, Color::rgb(0, 0, 0));
    // 覆盖区
    for y in 1..3 {
        for x in 2..5 {
            assert_eq!(r.pixel(x, y), Some([255, 0, 0, 255]), "({x},{y}) 应在红块内");
        }
    }
    // 边界外：半开区间，右/下边界那一格**不该**被涂
    assert_eq!(r.pixel(5, 1), Some([0, 0, 0, 255]), "右边界外");
    assert_eq!(r.pixel(2, 3), Some([0, 0, 0, 255]), "下边界外");
    assert_eq!(r.pixel(1, 1), Some([0, 0, 0, 255]), "左边界外");
}

/// **半透明与底色合成**（source-over）：alpha=128 的红压在中灰 (0x80) 上。
///
/// 算式：`255×0.502 + 128×0.498 = 191.75 → 0xC0`（红）；
/// `0×0.502 + 128×0.498 = 63.7 → 0x40`（绿、蓝）。
///
/// 这条是 diff 底带那类"低透明度铺色"的正确性根据。容差 ±8：覆盖率量化
/// （4×4 子采样）在整像素内部应当精确，留一点浮点余地。
#[test]
fn translucent_fill_blends_with_the_background() {
    let mut s = Scene::new();
    s.fill(
        neo_ui_render::Rect::new(0.0, 0.0, 4.0, 4.0),
        Color::rgba(255, 0, 0, 128),
    );
    let r = rasterize(&s, 4.0, 4.0, 1.0, Color::rgb(0x80, 0x80, 0x80));
    let [rr, gg, bb, aa] = r.pixel(2, 2).unwrap();
    assert!(aa == 255, "底板不透明，结果应仍为不透明");
    assert!((rr as i32 - 0xC0).abs() <= 8, "红通道应约 0xC0，实得 {rr:#x}");
    assert!((gg as i32 - 0x40).abs() <= 8, "绿通道应约 0x40，实得 {gg:#x}");
    assert!((bb as i32 - 0x40).abs() <= 8, "蓝通道应约 0x40，实得 {bb:#x}");
}

/// 裁剪真的裁：矩形只画在裁剪区内。
#[test]
fn clip_confines_the_paint() {
    let mut s = Scene::new();
    s.push(Op::PushClip { rect: neo_ui_render::Rect::new(2.0, 2.0, 2.0, 2.0) });
    s.fill(neo_ui_render::Rect::new(0.0, 0.0, 8.0, 8.0), Color::rgb(0, 255, 0));
    s.push(Op::PopClip);
    let r = rasterize(&s, 8.0, 8.0, 1.0, Color::rgb(0, 0, 0));
    assert_eq!(r.pixel(2, 2), Some([0, 255, 0, 255]), "裁剪区内应被涂");
    assert_eq!(r.pixel(3, 3), Some([0, 255, 0, 255]));
    assert_eq!(r.pixel(1, 1), Some([0, 0, 0, 255]), "裁剪区外不该被涂");
    assert_eq!(r.pixel(4, 4), Some([0, 0, 0, 255]), "裁剪区外不该被涂");
}

/// 嵌套裁剪 = 交集（不是"最后一次赢"）。
#[test]
fn nested_clips_intersect() {
    let mut s = Scene::new();
    s.push(Op::PushClip { rect: neo_ui_render::Rect::new(0.0, 0.0, 4.0, 4.0) });
    s.push(Op::PushClip { rect: neo_ui_render::Rect::new(2.0, 2.0, 4.0, 4.0) });
    s.fill(neo_ui_render::Rect::new(0.0, 0.0, 8.0, 8.0), Color::rgb(0, 0, 255));
    s.push(Op::PopClip);
    s.push(Op::PopClip);
    let r = rasterize(&s, 8.0, 8.0, 1.0, Color::rgb(0, 0, 0));
    assert_eq!(r.pixel(2, 2), Some([0, 0, 255, 255]), "交集内应被涂");
    assert_eq!(r.pixel(1, 1), Some([0, 0, 0, 255]), "只在外层内 → 不该涂");
    assert_eq!(r.pixel(5, 5), Some([0, 0, 0, 255]), "只在内层内 → 不该涂");
}

/// 圆角真的去掉了角上的像素（半径语义有效）。
#[test]
fn rounded_corners_remove_corner_pixels() {
    let mut sharp = Scene::new();
    sharp.fill(neo_ui_render::Rect::new(0.0, 0.0, 16.0, 16.0), Color::rgb(255, 255, 255));
    let mut round = Scene::new();
    round.fill_rounded(
        neo_ui_render::Rect::new(0.0, 0.0, 16.0, 16.0),
        Color::rgb(255, 255, 255),
        6.0,
    );
    let a = rasterize(&sharp, 16.0, 16.0, 1.0, Color::rgb(0, 0, 0));
    let b = rasterize(&round, 16.0, 16.0, 1.0, Color::rgb(0, 0, 0));

    // 角上：直角有、圆角没有
    let corner_sharp = a.pixel(0, 0).unwrap()[0];
    let corner_round = b.pixel(0, 0).unwrap()[0];
    assert!(corner_sharp > 200, "直角的角像素应是实心，实得 {corner_sharp}");
    assert!(corner_round < 60, "圆角的角像素应被去掉，实得 {corner_round}");
    // 中心两者都是实心
    assert!(b.pixel(8, 8).unwrap()[0] > 200, "圆角不该影响中心");
}

/// 零线宽不画（与另两个后端同一口径），非零线宽**向内**画。
#[test]
fn stroke_semantics_are_zero_skips_and_positive_goes_inward() {
    // 零线宽：等同没有
    let mut s = Scene::new();
    s.push(Op::StrokeRect {
        rect: neo_ui_render::Rect::new(1.0, 1.0, 6.0, 6.0),
        color: Color::rgb(255, 255, 255),
        width: 0.0,
        radius: 0.0,
    });
    let r = rasterize(&s, 8.0, 8.0, 1.0, Color::rgb(0, 0, 0));
    assert_eq!(r.pixel(1, 1), Some([0, 0, 0, 255]), "零线宽不该画");

    // 2px 线宽、向内：边框占 [1,3) × [1,3)，中心 4,4 不该被涂
    let mut s2 = Scene::new();
    s2.push(Op::StrokeRect {
        rect: neo_ui_render::Rect::new(1.0, 1.0, 6.0, 6.0),
        color: Color::rgb(255, 255, 255),
        width: 2.0,
        radius: 0.0,
    });
    let r2 = rasterize(&s2, 8.0, 8.0, 1.0, Color::rgb(0, 0, 0));
    assert_eq!(r2.pixel(1, 1), Some([255, 255, 255, 255]), "边框左上角");
    assert_eq!(r2.pixel(2, 2), Some([255, 255, 255, 255]), "2px 宽 → 第 2 格仍在框内");
    assert_eq!(r2.pixel(3, 3), Some([0, 0, 0, 255]), "第 3 格已是内部");
    assert_eq!(r2.pixel(6, 6), Some([255, 255, 255, 255]), "框右下角");
    assert_eq!(r2.pixel(7, 7), Some([0, 0, 0, 255]), "**向外**没有画（这正是与 SVG 的差别）");
}

/// DPI 缩放：2x 的画布是 2 倍，几何同比放大。
#[test]
fn scaling_doubles_the_canvas_and_the_geometry() {
    let mut s = Scene::new();
    s.fill(neo_ui_render::Rect::new(1.0, 1.0, 2.0, 2.0), Color::rgb(255, 0, 0));
    let r = rasterize(&s, 8.0, 8.0, 2.0, Color::rgb(0, 0, 0));
    assert_eq!((r.width, r.height), (16, 16), "2x 画布应翻倍");
    // 逻辑 (1,1)-(3,3) → 物理 (2,2)-(6,6)
    assert_eq!(r.pixel(2, 2), Some([255, 0, 0, 255]), "物理 (2,2) 在块内");
    assert_eq!(r.pixel(5, 5), Some([255, 0, 0, 255]), "物理 (5,5) 在块内");
    assert_eq!(r.pixel(1, 1), Some([0, 0, 0, 255]), "物理 (1,1) 在块外");
    assert_eq!(r.pixel(6, 6), Some([0, 0, 0, 255]), "右/下边界外");
}

/// **画不出的东西要计数**，不静默丢弃（与另两个后端同一纪律）。
#[test]
fn text_is_counted_as_unsupported() {
    let mut s = Scene::new();
    s.push(Op::FillText {
        text: "你好".into(),
        origin: neo_ui_render::Point::new(0.0, 0.0),
        color: Color::rgb(255, 255, 255),
        size: 12.0,
    });
    s.fill(neo_ui_render::Rect::new(0.0, 0.0, 1.0, 1.0), Color::rgb(255, 255, 255));
    let r = rasterize(&s, 8.0, 8.0, 1.0, Color::rgb(0, 0, 0));
    assert_eq!(r.unsupported_text, 1, "文字画不出，必须计数（否则'基线上没有'无从解释）");
    assert_eq!(r.pixel(0, 0), Some([255, 255, 255, 255]), "矩形仍要画");
}

/// PNG 是**真 PNG**：签名与分块结构合法，且能用系统解码器读回来。
///
/// 这条是"别生成一个只有我们自己认得的格式"的守卫。
#[test]
fn png_output_is_a_valid_png() {
    let r = rasterize(&Scene::new(), 4.0, 3.0, 1.0, Color::rgb(10, 20, 30));
    let png = r.to_png();
    assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a], "PNG 签名");
    // 必有的三块按序出现
    let has = |kind: &[u8]| png.windows(4).any(|w| w == kind);
    assert!(has(b"IHDR") && has(b"IDAT") && has(b"IEND"), "缺块");
    // IHDR 里的宽高应与光栅一致
    let ihdr = png.windows(4).position(|w| w == b"IHDR").unwrap();
    let w = u32::from_be_bytes(png[ihdr + 4..ihdr + 8].try_into().unwrap());
    let h = u32::from_be_bytes(png[ihdr + 8..ihdr + 12].try_into().unwrap());
    assert_eq!((w, h), (4, 3), "IHDR 尺寸不对");

    // 真解码一遍（用系统 `sips`，本机与 macOS CI 都有）——
    // 这一步才是"打不开的图"的守卫；只查签名会漏掉 zlib/CRC 写错的版本。
    if std::process::Command::new("sips").arg("--version").output().is_ok() {
        let tmp = std::env::temp_dir().join(format!("neo-png-check-{}.png", std::process::id()));
        std::fs::write(&tmp, &png).unwrap();
        let out = std::process::Command::new("sips")
            .args(["-g", "pixelWidth", "-g", "pixelHeight"])
            .arg(&tmp)
            .output()
            .expect("能跑 sips");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && text.contains("pixelWidth: 4") && text.contains("pixelHeight: 3"),
            "系统解码器读不回来：{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_file(&tmp);
    }
}

/// 光栅化是**确定性**的：同一场景两次渲染逐字节相同。
///
/// 没有这条，基线会随机红（浮点累加顺序、HashMap 迭代序都可能引入噪声）——
/// 而"偶尔红的测试"最后都会被忽略。
#[test]
fn rasterization_is_deterministic() {
    let case = cases().into_iter().find(|c| c.name == "segmented_progress").unwrap();
    let a = render(&case, 2.0);
    let b = render(&case, 2.0);
    assert_eq!(a.pixels, b.pixels, "同一场景两次渲染必须逐字节相同");
}
