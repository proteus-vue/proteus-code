//! 软件光栅化器 + PNG 导出 —— 让自绘表面**有像素可比对**。
//!
//! # 它解决什么（视觉回归基线缺的那一半）
//!
//! 方案 Phase 3 的 DoD 要求"每个 P1 组件 × 亮暗主题 × 2 档 DPI 的截图基线"。
//! 但真截图的路径在 CI 里走不通：要窗口、要 GPU、要显示服务器。而
//! [`crate::HeadlessBackend`] 只导出 SVG —— **SVG 能看，不能比**（没有像素）。
//!
//! 本模块补上那一半：把中立场景**真的画成像素**。于是"这一块自绘表面有没有
//! 被改样"变成一次字节比较。
//!
//! # 为什么可以自己写（而不是引一个 2D 引擎或 `png` crate）
//!
//! 因为**要画的东西少且形状固定**：场景里只有"填充/描边矩形（可圆角）+ 裁剪"，
//! 四个真实消费者**一个都不发文字**（文字留在 GPU 宿主的 element 树里 ——
//! 这正是渲染缝的分工，见 crate 头部）。所以不需要字体、不需要路径、不需要
//! 抗锯齿库，几十行就够。
//!
//! 由此换来三条：
//! - **不引入依赖**：本 crate 属可开源集，加一个依赖要过许可证守卫、要重新生成
//!   `THIRD-PARTY-LICENSES.md`、还要在清单里论证。这里的需要小到不值得。
//! - **跨平台逐字节一致**：光栅化是我们自己算的，没有 GPU 驱动差异、没有字体
//!   差异。于是基线可以**精确比较**，不需要"差几个像素也算过"的容差 —— 而容差
//!   正是视觉回归测试变松、最后没人看的原因。
//! - **能在任何环境跑**：无 GPU、无窗口、无显示服务器（CI 容器里就能跑）。
//!
//! # 诚实边界
//!
//! - **画不出文字**：与 [`crate::GpuiBackend`] 同一条边界（都需要字体上下文）。
//!   场景里有文字指令时**如实记进 [`Raster::unsupported_text`]**，不静默丢弃 ——
//!   "基线上看不到"必须能被解释。当前四个消费者都不发文字，所以基线是完整的；
//!   将来谁发了，那个计数会说话。
//! - **它不是第三个生产后端**：真机上画画的仍是 GPU 后端。它只服务"给像素"。
//! - **描边按 gpui 的语义（向内）**：SVG 的 `stroke` 是骑在边界上（向外一半）。
//!   两者对同一场景会差半个线宽。当前**没有任何消费者发描边**，所以它不在基线
//!   路径上；这里按真机看到的语义（gpui 的 `border`）实现，并把这个分歧记在此处
//!   —— 谁将来加描边，得先把两边对齐。
//!
//! # 与另两个后端的关系
//!
//! | | `GpuiBackend` | `HeadlessBackend` | 本模块 |
//! |---|---|---|---|
//! | 产物 | gpui 绘制参数 | 显示列表 + SVG | **RGBA 像素 + PNG** |
//! | 需要 GPU | 是 | 否 | 否 |
//! | 文字 | 画不出 | 画得出 | **画不出**（无字体） |
//! | 用途 | 真机渲染 | 跨后端契约比对 | **视觉回归基线** |

use crate::scene::{Color, Op, Rect, Scene};

/// 每轴子采样数（覆盖率精度 = `S²` 级）。
///
/// # 为什么用超采样而不是解析式覆盖率
///
/// 解析式（精确算矩形与圆的面积交）更快，但代码量大得多，而这里的场景都很小。
/// 超采样是**显然正确**的那一种：像素的覆盖率 = 落在图形内的子采样点占比。
/// 对一个"用来发现样式漂移"的基线来说，"显然正确"比"快"重要。
///
/// 4 已经够：偏差只在圆角那几十个像素上体现，而它的量化是 1/16 ≈ 6% 的 alpha ——
/// 足以让"圆角没了/半径变了"这类改动变色，而不会因为半个像素的浮点差异假红。
const SAMPLES_PER_AXIS: u32 = 4;

/// 一张光栅化的结果（RGBA8，行优先，不预乘 alpha）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` 字节。
    pub pixels: Vec<u8>,
    /// 场景里**画不出**的文字指令数（见模块头部的诚实边界）。
    pub unsupported_text: usize,
}

impl Raster {
    /// 取某个像素的 RGBA（越界返回 `None`）。
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        Some([self.pixels[i], self.pixels[i + 1], self.pixels[i + 2], self.pixels[i + 3]])
    }

    /// 编码为 PNG（RGBA8、无隔行、过滤器恒 0）。
    ///
    /// # 为什么 PNG 编码是手写的（约 60 行）
    ///
    /// 同模块头部：不为这点需要引入依赖。这里要写的是 PNG 的**最小合法子集**
    /// —— 真彩 + alpha、8 位、无隔行、每行过滤器恒 0（不做预测编码）。
    ///
    /// # 为什么要认真做压缩（实测数据推翻了"随便编编就行"）
    ///
    /// 最初用的是 deflate **存储块**（不压缩），理由是"基线图是纯色矩形，
    /// 文件不大"。实测**错了**：`sips` 生成的基线里 `diff_backdrop@2x` 是
    /// 1 MB，八张合计 **2064 KB**。而同样的图用真 deflate 压到 **7 KB**
    /// —— 0.3%。差 300 倍的东西，不能靠"反正能跑"糊过去（它要进 git 历史，
    /// 每次改样式都会重写一遍）。
    ///
    /// 于是实现**固定 Huffman + LZ77** 的 deflate：
    /// - 固定 Huffman 表是 RFC 1951 规定的，不必存进流里；
    /// - LZ77 用朴素哈希链找匹配（窗口 32K）—— 慢一点无所谓，这些图很小，
    ///   而"生成基线"是低频动作。
    ///
    /// **不用动态 Huffman**：那要建树 + 两遍编码，复杂度翻倍，而这些图的
    /// 收益已经拿到（0.3% vs 100%）。
    pub fn to_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        // PNG 签名
        out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

        // IHDR：宽、高、位深 8、颜色类型 6（RGBA）、压缩 0、过滤 0、隔行 0
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&self.width.to_be_bytes());
        ihdr.extend_from_slice(&self.height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        push_chunk(&mut out, b"IHDR", &ihdr);

        // 原始数据：每行前置一个过滤器字节（0 = None）
        let stride = (self.width as usize) * 4;
        let mut raw = Vec::with_capacity((stride + 1) * self.height as usize);
        for y in 0..self.height as usize {
            raw.push(0u8);
            raw.extend_from_slice(&self.pixels[y * stride..(y + 1) * stride]);
        }

        // IDAT：zlib 容器 + deflate（固定 Huffman + LZ77，见 `to_png` 的说明）
        let mut z = Vec::with_capacity(raw.len() / 4 + 64);
        z.extend_from_slice(&[0x78, 0x01]); // CMF/FLG：deflate、32K 窗口
        deflate_fixed(&raw, &mut z);
        z.extend_from_slice(&adler32(&raw).to_be_bytes());
        push_chunk(&mut out, b"IDAT", &z);

        push_chunk(&mut out, b"IEND", &[]);
        out
    }
}

/// 把场景画成像素。
///
/// - `width` / `height`：**逻辑**尺寸（场景坐标的坐标系）。
/// - `scale`：像素密度倍数。`2.0` 即"同一份逻辑布局用两倍像素画"——
///   这正是 DPI 那一维：几何与线宽都按 scale 放大，于是"1 物理像素的线
///   在高分屏上会不会消失"这类问题能被基线抓到。
/// - `background`：底色。场景本身不含背景（宿主在下面铺），但 alpha 合成
///   必须有东西可叠 —— 所以这里显式传入，而不是假设黑或白。
pub fn rasterize(
    scene: &Scene,
    width: f32,
    height: f32,
    scale: f32,
    background: Color,
) -> Raster {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let w = ((width * scale).round().max(0.0)) as u32;
    let h = ((height * scale).round().max(0.0)) as u32;

    let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
    for px in buf.chunks_exact_mut(4) {
        px.copy_from_slice(&[background.r, background.g, background.b, background.a]);
    }

    let mut raster = Raster { width: w, height: h, pixels: buf, unsupported_text: 0 };
    // 裁剪栈：栈顶是**已与所有外层求交**的有效区域（嵌套裁剪等价于交集）。
    let mut clips: Vec<Rect> = Vec::new();

    for op in scene.ops() {
        match op {
            Op::PushClip { rect } => {
                let r = scale_rect(rect, scale);
                let eff = match clips.last() {
                    Some(parent) => intersect(parent, &r),
                    None => r,
                };
                clips.push(eff);
            }
            Op::PopClip => {
                // 多余的 PopClip 静默忽略而不是 panic：渲染路径上的 panic
                // 会整帧空白，比"少一层裁剪"严重得多。场景的平衡性由
                // `Scene::clips_balanced()` 自检（那是调用方的责任）。
                clips.pop();
            }
            Op::FillRect { rect, color, radius } => {
                let r = scale_rect(rect, scale);
                let rad = radius * scale;
                let clip = clips.last().copied();
                fill_rounded_rect(&mut raster, &r, rad, *color, clip.as_ref());
            }
            Op::StrokeRect { rect, color, width, radius } => {
                // 零/负线宽**不画**（与 `GpuiBackend`/`HeadlessBackend` 一致：
                // 那三个后端都把它当作"没有描边"，而不是"1px 描边"）。
                if *width <= 0.0 {
                    continue;
                }
                let r = scale_rect(rect, scale);
                let line = width * scale;
                let rad = radius * scale;
                let clip = clips.last().copied();
                stroke_rect_inside(&mut raster, &r, line, rad, *color, clip.as_ref());
            }
            Op::FillText { .. } => {
                // 画不出（无字体上下文）—— **如实计数**，不静默丢弃。
                raster.unsupported_text += 1;
            }
        }
    }
    raster
}

// ── 几何 ──────────────────────────────────────────────────────────────────

fn scale_rect(r: &Rect, scale: f32) -> Rect {
    Rect::new(
        r.origin.x * scale,
        r.origin.y * scale,
        r.size.w * scale,
        r.size.h * scale,
    )
}

fn intersect(a: &Rect, b: &Rect) -> Rect {
    let x0 = a.origin.x.max(b.origin.x);
    let y0 = a.origin.y.max(b.origin.y);
    let x1 = (a.origin.x + a.size.w).min(b.origin.x + b.size.w);
    let y1 = (a.origin.y + a.size.h).min(b.origin.y + b.size.h);
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// 点是否在圆角矩形内。`radius` 超过半边长时按半边长算 ——
/// SVG 的 `rx` 与 gpui 的 `Corners` 都这么夹取，所以这里跟随。
fn inside_rounded(r: &Rect, radius: f32, x: f32, y: f32) -> bool {
    let (x0, y0) = (r.origin.x, r.origin.y);
    let (x1, y1) = (x0 + r.size.w, y0 + r.size.h);
    if x < x0 || y < y0 || x >= x1 || y >= y1 {
        return false;
    }
    let rad = radius.min(r.size.w / 2.0).min(r.size.h / 2.0);
    if rad <= 0.0 {
        return true;
    }
    // 四个角的圆心
    let cxs = [x0 + rad, x1 - rad];
    let cys = [y0 + rad, y1 - rad];
    for cx in cxs {
        for cy in cys {
            // 只在对应的角区域里做圆判定
            let in_corner_x = if cx < x0 + r.size.w / 2.0 { x < cx } else { x > cx };
            let in_corner_y = if cy < y0 + r.size.h / 2.0 { y < cy } else { y > cy };
            if in_corner_x && in_corner_y {
                let (dx, dy) = (x - cx, y - cy);
                return dx * dx + dy * dy <= rad * rad;
            }
        }
    }
    true
}

/// 覆盖式绘制一个圆角矩形（`op` 语义：新内容直接盖上去）。
///
/// 超采样：每个像素取 `SAMPLES_PER_AXIS²` 个点，覆盖率 = 落在图形内的占比。
/// 与底色做 alpha 合成 —— **不预乘**，与 `Raster::pixels` 的约定一致。
fn fill_rounded_rect(
    raster: &mut Raster,
    rect: &Rect,
    radius: f32,
    color: Color,
    clip: Option<&Rect>,
) {
    if rect.size.w <= 0.0 || rect.size.h <= 0.0 || color.a == 0 {
        return;
    }
    // 像素包围盒：与裁剪区求交后只扫这一块（不全图扫，图可能不小）。
    let (x0, y0, x1, y1) = pixel_bounds(rect, clip);
    let (w, h) = (raster.width, raster.height);
    let n = SAMPLES_PER_AXIS;
    let step = 1.0 / n as f32;
    for py in y0..y1 {
        for px in x0..x1 {
            // 逐子采样点判定，累计覆盖率
            let mut hits = 0u32;
            for sy in 0..n {
                for sx in 0..n {
                    let fx = px as f32 + (sx as f32 + 0.5) * step;
                    let fy = py as f32 + (sy as f32 + 0.5) * step;
                    if inside_rounded(rect, radius, fx, fy) {
                        hits += 1;
                    }
                }
            }
            if hits == 0 {
                continue;
            }
            let coverage = hits as f32 / (n * n) as f32;
            blend(raster, px, py, color, coverage, w, h);
        }
    }
}

/// 描边矩形，**线宽向内**（与 gpui 的 `border` 一致；见模块头部的诚实边界）。
fn stroke_rect_inside(
    raster: &mut Raster,
    rect: &Rect,
    width: f32,
    radius: f32,
    color: Color,
    clip: Option<&Rect>,
) {
    let inner = Rect::new(
        rect.origin.x + width,
        rect.origin.y + width,
        rect.size.w - 2.0 * width,
        rect.size.h - 2.0 * width,
    );
    // 外框内、内框外 —— 内框退化（线宽超过一半）时整个框都是实的
    let inner_radius = (radius - width).max(0.0);
    if rect.size.w <= 0.0 || rect.size.h <= 0.0 || color.a == 0 {
        return;
    }
    let (x0, y0, x1, y1) = pixel_bounds(rect, clip);
    let (w, h) = (raster.width, raster.height);
    let n = SAMPLES_PER_AXIS;
    let step = 1.0 / n as f32;
    let degenerate = inner.size.w <= 0.0 || inner.size.h <= 0.0;
    for py in y0..y1 {
        for px in x0..x1 {
            let mut hits = 0u32;
            for sy in 0..n {
                for sx in 0..n {
                    let fx = px as f32 + (sx as f32 + 0.5) * step;
                    let fy = py as f32 + (sy as f32 + 0.5) * step;
                    let in_outer = inside_rounded(rect, radius, fx, fy);
                    let in_inner = !degenerate && inside_rounded(&inner, inner_radius, fx, fy);
                    if in_outer && !in_inner {
                        hits += 1;
                    }
                }
            }
            if hits == 0 {
                continue;
            }
            let coverage = hits as f32 / (n * n) as f32;
            blend(raster, px, py, color, coverage, w, h);
        }
    }
}

/// 待扫的像素范围：形状包围盒 ∩ 裁剪区 ∩ 画布。
fn pixel_bounds(rect: &Rect, clip: Option<&Rect>) -> (u32, u32, u32, u32) {
    let mut r = *rect;
    if let Some(c) = clip {
        r = intersect(&r, c);
    }
    let x0 = r.origin.x.floor().max(0.0) as u32;
    let y0 = r.origin.y.floor().max(0.0) as u32;
    let x1 = (r.origin.x + r.size.w).ceil().max(0.0) as u32;
    let y1 = (r.origin.y + r.size.h).ceil().max(0.0) as u32;
    (x0, y0, x1, y1)
}

/// 把一个**不透明或半透明**的颜色按覆盖率叠到像素上（source-over）。
fn blend(raster: &mut Raster, x: u32, y: u32, color: Color, coverage: f32, w: u32, h: u32) {
    if x >= w || y >= h {
        return;
    }
    // 有效 alpha = 颜色自身 alpha × 覆盖率
    let a = (color.a as f32 / 255.0) * coverage;
    if a <= 0.0 {
        return;
    }
    let i = ((y * w + x) * 4) as usize;
    let dst = &mut raster.pixels[i..i + 4];
    for (k, src) in [color.r, color.g, color.b].iter().enumerate() {
        let d = dst[k] as f32;
        dst[k] = (*src as f32 * a + d * (1.0 - a)).round() as u8;
    }
    // 目标是不透明底板（background 通常 a=255），所以结果 alpha 保持 255；
    // 若底板本身半透明，按 source-over 合成 alpha。
    let da = dst[3] as f32 / 255.0;
    let out_a = a + da * (1.0 - a);
    dst[3] = (out_a * 255.0).round() as u8;
}

// ── PNG 编码的底层件 ──────────────────────────────────────────────────────

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    // 标准 PNG/zip CRC-32（多项式 0xEDB88320），表在首次调用时构建。
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for b in data {
        c = table[((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + *byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

// ── deflate（固定 Huffman + LZ77）─────────────────────────────────────────
//
// RFC 1951。只做固定 Huffman 表（BTYPE=01）：表是规范规定的，不必存进流；
// 动态表要建树 + 两遍编码，而这些基线图的收益已经拿到了（2064 KB → 7 KB）。

/// 固定 Huffman 的字面量/长度码字表（RFC 1951 §3.2.6）。
///
/// 每个字节值 `b` 的码字：`0..=143` 用 8 位（`0x30 + b`）、`144..=255` 用 9 位
/// （`0x190 + b - 144`）。这里的实现按规范直接算，不预先建表 —— 少一份可能与
/// 规范漂移的常量。
fn fixed_lit_code(sym: u16) -> (u16, u8) {
    if sym <= 143 {
        (0x30 + sym, 8)
    } else if sym <= 255 {
        (0x190 + (sym - 144), 9)
    } else if sym <= 279 {
        (sym - 256, 7)
    } else {
        (0xC0 + (sym - 280), 8)
    }
}

/// **码字要按 MSB-first 写**（Huffman 码在 deflate 里是"打包的位"，
/// 而其它字段是 LSB-first —— 两者搞混会得到"能解但全是乱码"的流）。
fn write_bits_lsb(out: &mut Vec<u8>, bitbuf: &mut u32, nbits: &mut u8, value: u32, n: u8) {
    *bitbuf |= value << *nbits;
    *nbits += n;
    while *nbits >= 8 {
        out.push((*bitbuf & 0xff) as u8);
        *bitbuf >>= 8;
        *nbits -= 8;
    }
}

/// 写一个 Huffman 码：**码字本身高位在前**。
fn write_huff(out: &mut Vec<u8>, bitbuf: &mut u32, nbits: &mut u8, code: u16, len: u8) {
    // 码字在流里是"从最高位开始读"，所以要按 MSB-first 展开成位序列。
    for i in (0..len).rev() {
        let bit = (code >> i) & 1;
        write_bits_lsb(out, bitbuf, nbits, bit as u32, 1);
    }
}

/// 长度码表：`(码 257..285 的编号, 额外位数, 基准长度)`。
const LEN_BASE: &[(u16, u8, u16)] = &[
    (257, 0, 3), (258, 0, 4), (259, 0, 5), (260, 0, 6), (261, 0, 7),
    (262, 0, 8), (263, 0, 9), (264, 0, 10), (265, 1, 11), (266, 1, 13),
    (267, 1, 15), (268, 1, 17), (269, 2, 19), (270, 2, 23), (271, 2, 27),
    (272, 2, 31), (273, 3, 35), (274, 3, 43), (275, 3, 51), (276, 3, 59),
    (277, 4, 67), (278, 4, 83), (279, 4, 99), (280, 4, 115), (281, 5, 131),
    (282, 5, 163), (283, 5, 195), (284, 5, 227), (285, 0, 258),
];

/// 距离码表：`(码编号 0..29, 额外位数, 基准距离)`。
const DIST_BASE: &[(u16, u8, u16)] = &[
    (0, 0, 1), (1, 0, 2), (2, 0, 3), (3, 0, 4), (4, 1, 5), (5, 1, 7),
    (6, 2, 9), (7, 2, 13), (8, 3, 17), (9, 3, 25), (10, 4, 33), (11, 4, 49),
    (12, 5, 65), (13, 5, 97), (14, 6, 129), (15, 6, 193), (16, 7, 257),
    (17, 7, 385), (18, 8, 513), (19, 8, 769), (20, 9, 1025), (21, 9, 1537),
    (22, 10, 2049), (23, 10, 3073), (24, 11, 4097), (25, 11, 6145),
    (26, 12, 8193), (27, 12, 12289), (28, 13, 16385), (29, 13, 24577),
];

/// 找出 `(len, code_id, extra_bits, extra_val)`；`len` 必须 ≥ 3。
fn match_len_code(len: u16) -> (u16, u8, u16) {
    let mut chosen = LEN_BASE[0];
    for &e in LEN_BASE {
        if e.2 <= len {
            chosen = e;
        } else {
            break;
        }
    }
    (chosen.0, chosen.1, len - chosen.2)
}

fn match_dist_code(dist: u16) -> (u16, u8, u16) {
    let mut chosen = DIST_BASE[0];
    for &e in DIST_BASE {
        if e.2 <= dist {
            chosen = e;
        } else {
            break;
        }
    }
    (chosen.0, chosen.1, dist - chosen.2)
}

/// 把 `data` 按固定 Huffman + LZ77 编进 `out`（含结束码与位填充）。
fn deflate_fixed(data: &[u8], out: &mut Vec<u8>) {
    let (mut bitbuf, mut nbits) = (0u32, 0u8);
    // 块头：BFINAL=1（单块）+ BTYPE=01（固定 Huffman）
    write_bits_lsb(out, &mut bitbuf, &mut nbits, 1, 1);
    write_bits_lsb(out, &mut bitbuf, &mut nbits, 1, 2);

    // LZ77：哈希链。窗口 32768（deflate 上限），链长封顶避免病态慢。
    const MIN_MATCH: usize = 3;
    const MAX_MATCH: usize = 258;
    const MAX_CHAIN: usize = 64;
    let mut head = vec![usize::MAX; 1 << 15];
    let mut prev = vec![usize::MAX; data.len().max(1)];

    let hash3 = |d: &[u8], i: usize| -> usize {
        // 三个字节混成一个 15 位哈希
        let a = d[i] as usize;
        let b = d.get(i + 1).copied().unwrap_or(0) as usize;
        let c = d.get(i + 2).copied().unwrap_or(0) as usize;
        ((a << 10) ^ (b << 5) ^ c) & 0x7fff
    };

    let mut i = 0usize;
    while i < data.len() {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        if i + MIN_MATCH <= data.len() {
            let h = hash3(data, i);
            let mut cand = head[h];
            let mut chain = 0;
            let limit = i.saturating_sub(32768);
            while cand != usize::MAX && cand >= limit && chain < MAX_CHAIN {
                // 比长度（不超过窗口/剩余/上限）
                let max = MAX_MATCH.min(data.len() - i);
                let mut l = 0usize;
                while l < max && data[cand + l] == data[i + l] {
                    l += 1;
                }
                if l > best_len && l >= MIN_MATCH {
                    best_len = l;
                    best_dist = i - cand;
                    if l == MAX_MATCH {
                        break;
                    }
                }
                cand = prev[cand];
                chain += 1;
            }
            // 把当前位置挂进链
            prev[i] = head[h];
            head[h] = i;
        }

        if best_len >= MIN_MATCH {
            let (len_sym, extra_bits, extra_val) = match_len_code(best_len as u16);
            let (code, bits) = fixed_lit_code(len_sym);
            write_huff(out, &mut bitbuf, &mut nbits, code, bits);
            if extra_bits > 0 {
                write_bits_lsb(out, &mut bitbuf, &mut nbits, extra_val as u32, extra_bits);
            }
            let (dist_sym, dbits, dval) = match_dist_code(best_dist as u16);
            // 距离码用**它们自己的** 5 位固定码（0..31），同样是 MSB-first
            write_huff(out, &mut bitbuf, &mut nbits, dist_sym, 5);
            if dbits > 0 {
                write_bits_lsb(out, &mut bitbuf, &mut nbits, dval as u32, dbits);
            }
            // 匹配区间内的所有位置都要进哈希链（否则后面找不到这些起点）
            for k in 1..best_len {
                let j = i + k;
                if j + MIN_MATCH <= data.len() {
                    let h = hash3(data, j);
                    prev[j] = head[h];
                    head[h] = j;
                }
            }
            i += best_len;
        } else {
            let (code, bits) = fixed_lit_code(data[i] as u16);
            write_huff(out, &mut bitbuf, &mut nbits, code, bits);
            i += 1;
        }
    }

    // 结束码 256
    let (code, bits) = fixed_lit_code(256);
    write_huff(out, &mut bitbuf, &mut nbits, code, bits);
    // 位填充到字节边界
    if nbits > 0 {
        out.push((bitbuf & 0xff) as u8);
    }
}
