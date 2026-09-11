//! 鼠标事件解析（SGR 扩展模式）
//!
//! # 为什么用 SGR（`ESC[<b;x;yM`）而不是老模式
//!
//! 老的 X10 模式把坐标编码成单个字节（值 = 32 + n），**列号超过 223 就会
//! 溢出**。我们支持宽终端（用户实测用过 260 列），必须用 SGR 扩展模式
//! （`ESC[?1006h`）：坐标是十进制文本，多宽都行，且能区分按下/释放。
//!
//! # 只解析，不判断
//!
//! 这个模块只把字节序列解成 `MouseEvent`；"点在哪个区域、该做什么"
//! 由调用方按布局决定。分开的理由与其它模块一致：解析可以脱离终端单测，
//! 而命中测试依赖布局，属于渲染层的事。

/// 鼠标按键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    /// 滚轮向上（每格）
    WheelUp,
    /// 滚轮向下
    WheelDown,
    /// 其它（后退/前进等），当前不处理
    Other,
}

/// 一次鼠标事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub button: Button,
    /// 是否按下（false = 释放）。滚轮没有释放事件，恒为 true。
    pub pressed: bool,
    /// 列（1 基，与终端一致）
    pub x: usize,
    /// 行（1 基）
    pub y: usize,
    /// 是否按住修饰键（shift/alt/ctrl 任一）
    pub modified: bool,
}

/// 解析 SGR 鼠标序列的**主体**（调用方已剥掉 `ESC[<`）。
///
/// 形如 `0;12;5M`（左键在第 12 列第 5 行按下）或 `64;3;7M`（滚轮上）。
/// 返回 `None` 表示不是合法鼠标序列（调用方应忽略而不是报错 ——
/// 终端可能送来我们不认识的扩展）。
pub fn parse_sgr(body: &str) -> Option<MouseEvent> {
    // 末字符 M = 按下，m = 释放
    let (payload, pressed) = match body.chars().last()? {
        'M' => (&body[..body.len() - 1], true),
        'm' => (&body[..body.len() - 1], false),
        _ => return None,
    };
    let mut it = payload.split(';');
    let code: u32 = it.next()?.parse().ok()?;
    let x: usize = it.next()?.parse().ok()?;
    let y: usize = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None; // 字段多于预期 → 不是我们认识的格式
    }
    if x == 0 || y == 0 {
        return None; // SGR 坐标是 1 基；0 说明是噪声
    }

    // 位含义（xterm 约定）：
    //   低 2 位：按键（0=左,1=中,2=右,3=释放）
    //   第 3 位（4）：shift；第 4 位（8）：alt；第 5 位（16）：ctrl
    //   第 7 位（64）：滚轮
    let button_bits = code & 0b11;
    let modified = code & (4 | 8 | 16) != 0;
    let wheel = code & 64 != 0;

    let button = if wheel {
        if button_bits == 0 {
            Button::WheelUp
        } else {
            Button::WheelDown
        }
    } else {
        match button_bits {
            0 => Button::Left,
            1 => Button::Middle,
            2 => Button::Right,
            _ => Button::Other, // 3 = 仅释放（无具体按键）
        }
    };
    Some(MouseEvent { button, pressed, x, y, modified })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_left_click_press_and_release() {
        let down = parse_sgr("0;12;5M").expect("左键按下应可解析");
        assert_eq!(down.button, Button::Left);
        assert!(down.pressed);
        assert_eq!((down.x, down.y), (12, 5));

        let up = parse_sgr("0;12;5m").expect("左键释放应可解析");
        assert!(!up.pressed, "小写 m 表示释放");
    }

    #[test]
    fn parses_wheel_directions() {
        let up = parse_sgr("64;3;7M").expect("滚轮上");
        assert_eq!(up.button, Button::WheelUp);
        let down = parse_sgr("65;3;7M").expect("滚轮下");
        assert_eq!(down.button, Button::WheelDown);
    }

    #[test]
    fn parses_middle_and_right() {
        assert_eq!(parse_sgr("1;1;1M").unwrap().button, Button::Middle);
        assert_eq!(parse_sgr("2;1;1M").unwrap().button, Button::Right);
    }

    #[test]
    fn detects_modifiers() {
        // 4=shift, 8=alt, 16=ctrl
        for code in [4, 8, 16, 4 | 8 | 16] {
            let e = parse_sgr(&format!("{code};1;1M")).unwrap();
            assert!(e.modified, "code={code} 应标记为含修饰键");
        }
        assert!(!parse_sgr("0;1;1M").unwrap().modified);
    }

    #[test]
    fn handles_wide_columns_that_break_the_old_protocol() {
        // 老 X10 协议在列号 >223 时会溢出；SGR 用十进制文本，多宽都行。
        // 用户实测用过 260 列宽终端，所以这条必须有。
        let e = parse_sgr("0;300;40M").expect("300 列应可解析");
        assert_eq!(e.x, 300);
        let e = parse_sgr("0;1000;1M").expect("1000 列应可解析");
        assert_eq!(e.x, 1000);
    }

    #[test]
    fn rejects_malformed_input_without_panicking() {
        // 终端可能送来我们不认识的扩展；必须安静忽略而不是崩
        for bad in ["", "M", "0;1M", "a;b;cM", "0;1;2;3M", "0;0;5M", "0;5;0M", "0;1;2X", ";;M"] {
            assert!(parse_sgr(bad).is_none(), "{bad:?} 应被拒绝");
        }
    }

    #[test]
    fn unknown_button_bits_are_marked_other() {
        // 3 = "仅释放，无按键信息"；不该被误判成左键
        assert_eq!(parse_sgr("3;1;1m").unwrap().button, Button::Other);
    }

    #[test]
    fn release_events_are_recognised_for_non_wheel() {
        let e = parse_sgr("2;10;10m").unwrap();
        assert_eq!(e.button, Button::Right);
        assert!(!e.pressed);
    }
}
