//! 星场背景 —— 全屏点阵装饰（对标 MiMo Code 的星空首页）
//!
//! # 为什么必须是确定性的
//!
//! TUI 每帧重画整屏。若用 `rand`，每帧星位都变 —— 用户看到的是满屏噪点
//! 在闪，比没有背景还糟。这里改成**位置哈希**：同一 `(row, col, cols)`
//! 永远得到同一个结果，星位在多帧之间完全静止。
//!
//! # 它不参与任何语义
//!
//! 星场是纯装饰：不进 `Fact`、不影响 T6 宿主等价、NO_COLOR 下自动消失。
//! 把它放在独立模块，就是为了让"装饰"与"语义"的边界一眼可见。

/// 一个背景格子：`None` = 空白，`Some((字符, 是否更亮))` = 星。
///
/// 只使用**半角**字符（`·`/`+`），避免宽字符让整行错位 ——
/// 星场错位一列，整屏排版就全歪了。
pub fn cell(row: usize, col: usize, cols: usize) -> Option<(char, bool)> {
    // splitmix64 变体：便宜、雪崩性好，够当装饰用随机源
    let mut h = (row as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (col as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (cols as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;

    const TOTAL: u64 = 1000;
    match h % TOTAL {
        // 亮度分两档：少数"亮点"、多数"暗点"。密度约 0.9%：
        // 再密就变成噪点，会跟正文抢注意力（背景的本分是退后）。
        0..=1 => Some(('+', true)),
        2..=8 => Some(('·', false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_deterministic_across_calls() {
        // 同一格调两次必须一致，否则每帧重画会闪
        for (r, c) in [(0, 0), (3, 17), (40, 199), (7, 42)] {
            assert_eq!(cell(r, c, 200), cell(r, c, 200), "位置 ({r},{c}) 不稳定");
        }
    }

    #[test]
    fn density_is_low_but_nonzero() {
        // 太密 = 噪点；全空 = 白做。量化一次，避免"感觉差不多"
        let (cols, rows) = (200usize, 60usize);
        let n = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (r, c)))
            .filter(|(r, c)| cell(*r, *c, cols).is_some())
            .count();
        let ratio = n as f64 / (cols * rows) as f64;
        assert!(ratio > 0.002 && ratio < 0.03, "星密度 {ratio:.4} 超出合理区间");
    }

    #[test]
    fn only_uses_narrow_characters() {
        // 宽字符会让整行错位；星场必须只用半角
        let chars: Vec<char> = (0..80)
            .flat_map(|r| (0..200).map(move |c| (r, c)))
            .filter_map(|(r, c)| cell(r, c, 200).map(|v| v.0))
            .collect();
        for ch in chars {
            assert_eq!(crate::width::char_width(ch), 1, "{ch:?} 不是半角，会撑歪排版");
        }
    }
}
