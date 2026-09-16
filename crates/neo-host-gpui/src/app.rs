//! 窗口与视图：把事件流画成界面。
//!
//! # 结构（照 egui 版那条分界，但绘制层换成了 gpui）
//!
//! - **数据**：`neo_driver::transcript::Transcript`（共享，不依赖任何 GUI 库，
//!   可在无窗口环境单测）
//! - **绘制**：本模块。它是全仓唯一用 gpui 画 NEO 界面的地方。
//!
//! # 两处必须照做的细节
//!
//! 1. **窗口首层必须是 `Root`**：否则 `open_dialog` / 通知 / tooltip 会 panic
//!    或静默失效（gpui-component 的 `Root::update` 有 `expect`）。
//! 2. **响应式重绘靠唤醒钩子**：gpui 不出帧就不重绘，所以事件到达时必须
//!    `cx.notify()` —— 否则表现为"模型答完了，屏幕上什么都没变"。

use std::sync::Arc;

use neo_driver::transcript::{mode_label, next_mode, Block, Transcript};
use neo_driver::KernelHandle;
use neo_protocol::{Decision, EventMsg, ExecMode, Op};
use neo_text::Tone;
use neo_ui::neo_color;
use neo_ui_kit::component::{
    button::Button, h_flex, v_flex, Root,
};
use neo_ui_kit::component::scroll::ScrollableElement as _;
use neo_ui_kit::gpui::{div, prelude::*, px, Context, Entity, IntoElement, Render, Window};

/// 一条待审批。
#[derive(Clone)]
struct PendingApproval {
    id: String,
    detail: String,
    kind: String,
}

/// 界面状态（Entity）。
pub struct NeoView {
    handle: KernelHandle,
    transcript: Transcript,
    input: String,
    mode: ExecMode,
    model: String,
    /// 待审批（非 None 时输入被阻塞 —— ZCode 语义：权限门暂停当前任务）。
    pending: Option<PendingApproval>,
    /// 状态行提示
    notice: Option<String>,
    /// 工作区/模式说明
    status: String,
}

impl NeoView {
    fn new(handle: KernelHandle, status: String, mode: ExecMode, model: String) -> Self {
        Self {
            handle,
            transcript: Transcript::new(),
            input: String::new(),
            mode,
            model,
            pending: None,
            notice: None,
            status,
        }
    }

    /// 取回已到达的事件并更新模型。返回是否有变化（决定要不要重绘）。
    fn pump(&mut self) -> bool {
        let events = self.handle.drain();
        if events.is_empty() {
            return false;
        }
        for ev in &events {
            match ev {
                // 审批：进入阻塞态
                EventMsg::ApprovalRequest { id, detail, kind } => {
                    self.pending = Some(PendingApproval {
                        id: id.clone(),
                        detail: detail.clone(),
                        kind: kind.clone(),
                    });
                }
                // 模型切换：状态行跟着变（内核只在这些时刻告诉我们）
                EventMsg::ModelSwitched { model, .. } => self.model = model.clone(),
                _ => {}
            }
        }
        self.transcript.push_batch(&events);
        true
    }

    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        // 审批未决时不接受新任务（避免把下一步排进队列）
        if text.is_empty() || self.pending.is_some() {
            return;
        }
        self.input.clear();
        let refs = neo_protocol::parse_refs(&text);
        // BeginTurn 而不是 UserTurn：后者一次跑完整轮（界面会卡住）。
        // 推进由每帧的 Pump 完成。
        self.handle.send(Op::BeginTurn { text, refs });
        self.transcript.running = true;
    }

    /// 逐帧推进一轮里的**一步**。
    fn pump_step(&self) {
        self.handle.send(Op::Pump);
    }

    fn approve(&mut self, decision: Decision) {
        let Some(p) = self.pending.take() else {
            return;
        };
        // `ApproveStep` 而不是 `Approve`：后者会一次跑完剩余往返（界面又冻）
        self.handle.send(Op::ApproveStep {
            id: p.id,
            decision,
            reason: None,
        });
        self.transcript.running = true;
    }

    fn cycle_mode(&mut self) {
        let next = next_mode(self.mode);
        self.handle.send(Op::ConfigureSession {
            patch: neo_protocol::SessionPatch {
                exec_mode: Some(next),
                ..Default::default()
            },
        });
        self.mode = next;
        // 模式变化**没有内核事件**，宿主必须自记 —— 否则状态行显示旧档位
        self.notice = Some(format!("已切换到 {} 模式", mode_label(next)));
    }
}

/// 把一行带色调的片段渲染成一个元素（复用 `neo-text` 的 Markdown 解析）。
///
/// 用 `StyledText::with_highlights` 而不是逐片段 `div().child()`：
/// 前者把它当**一行文字**参与排版（换行、基线正确），后者是并排的盒子，
/// 中英混排时会各占各的宽度、断行位置全错。
fn styled_line(spans: &[(String, Tone)]) -> impl IntoElement {
    let text: String = spans.iter().map(|(t, _)| t.as_str()).collect();
    let mut highlights = Vec::new();
    let mut offset = 0usize;
    for (seg, tone) in spans {
        let len = seg.len();
        if len > 0 {
            highlights.push((
                offset..offset + len,
                neo_ui_kit::gpui::HighlightStyle {
                    color: Some(neo_color(*tone).into()),
                    ..Default::default()
                },
            ));
        }
        offset += len;
    }
    div().child(
        neo_ui_kit::gpui::StyledText::new(text).with_highlights(highlights),
    )
}

/// 转录区：把 `Block` 画出来。
fn transcript_view(blocks: &[Block]) -> impl IntoElement {
    let mut col = v_flex().gap_1().p_3();
    for b in blocks {
        match b {
            Block::User(t) => {
                col = col.child(div().text_color(neo_color(Tone::Accent)).child(format!("┃ {t}")));
            }
            Block::Assistant(t) => {
                // Markdown：走共享解析器出块，逐行渲染（换行交给布局引擎）
                for line in neo_text::markdown::blocks(t) {
                    col = col.child(styled_line(&line));
                }
            }
            Block::Reasoning(t) => {
                col = col.child(
                    div()
                        .text_color(neo_color(Tone::Muted))
                        .child(format!("▾ 思考（{} 字）：{t}", t.chars().count())),
                );
            }
            Block::Tool(c) => {
                let state = if !c.done {
                    "执行中"
                } else if c.exit_code == Some(0) {
                    "完成"
                } else {
                    "失败"
                };
                let tone = if !c.done {
                    Tone::Info
                } else if c.exit_code == Some(0) {
                    Tone::Success
                } else {
                    Tone::Error
                };
                col = col.child(h_flex().gap_2().child(
                    div()
                        .text_color(neo_color(Tone::Primary))
                        .child(format!("▸ {}", c.name)),
                ).child(div().text_color(neo_color(tone)).child(state)));
                if !c.args.is_empty() {
                    col = col.child(
                        div().text_color(neo_color(Tone::Muted)).child(c.args.clone()),
                    );
                }
                let body = if c.stderr.is_empty() { &c.stdout } else { &c.stderr };
                if !body.trim().is_empty() {
                    col = col.child(
                        div()
                            .text_color(neo_color(if c.stderr.is_empty() {
                                Tone::Text
                            } else {
                                Tone::Error
                            }))
                            .child(body.clone()),
                    );
                }
                if c.truncated {
                    col = col.child(
                        div()
                            .text_color(neo_color(Tone::Warning))
                            .child("（输出已截断）"),
                    );
                }
            }
            Block::Diff { path, diff } => {
                col = col.child(
                    div()
                        .text_color(neo_color(Tone::Info))
                        .child(format!("改动 {path}")),
                );
                for line in diff.lines() {
                    let tone = match neo_driver::transcript::diff_line_kind(line) {
                        neo_driver::transcript::DiffLineKind::Add => Tone::Success,
                        neo_driver::transcript::DiffLineKind::Del => Tone::Error,
                        neo_driver::transcript::DiffLineKind::Hunk => Tone::Info,
                        neo_driver::transcript::DiffLineKind::Meta => Tone::Muted,
                        _ => Tone::Text,
                    };
                    col = col.child(div().text_color(neo_color(tone)).child(line.to_string()));
                }
            }
            Block::TurnSummary { input_tokens, output_tokens } => {
                col = col.child(
                    div()
                        .text_color(neo_color(Tone::Muted))
                        .child(format!("· 本轮完成（{input_tokens} in / {output_tokens} out）")),
                );
            }
            Block::Files(files) => {
                for (p, add, del) in files {
                    col = col.child(
                        div()
                            .text_color(neo_color(Tone::Muted))
                            .child(format!("  {p} +{add} -{del}")),
                    );
                }
            }
            Block::Todos(items) => {
                for it in items {
                    let (mark, tone) = match it.status {
                        neo_protocol::TodoStatus::Completed => ("✓", Tone::Success),
                        neo_protocol::TodoStatus::InProgress => ("▸", Tone::Info),
                        neo_protocol::TodoStatus::Pending => ("·", Tone::Muted),
                    };
                    col = col.child(
                        div()
                            .text_color(neo_color(tone))
                            .child(format!("{mark} {}", it.content)),
                    );
                }
            }
            Block::Notice { text, tone } => {
                col = col.child(div().text_color(neo_color(*tone)).child(text.clone()));
            }
        }
    }
    col
}

/// 审批对话框（模态）：三档 Allow / Always / Reject。
fn approval_dialog(view: &NeoView, cx: &mut Context<NeoView>) -> impl IntoElement {
    let p = view.pending.clone().expect("调用方保证有 pending");
    let v1 = cx.entity().clone();
    let v2 = cx.entity().clone();
    let v3 = cx.entity().clone();
    v_flex()
        .gap_3()
        .p_4()
        .child(div().child("需要审批"))
        .child(div().child(p.detail.clone()))
        .child(
            div()
                .text_color(neo_color(Tone::Muted))
                .child(format!("类别：{}", p.kind)),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("allow").label("允许").on_click(move |_, _, cx| {
                        v1.update(cx, |this, _| this.approve(Decision::Allow));
                    }),
                )
                .child(
                    Button::new("always").label("总是允许").on_click(move |_, _, cx| {
                        v2.update(cx, |this, _| this.approve(Decision::AllowAlways));
                    }),
                )
                .child(
                    Button::new("reject").label("拒绝").on_click(move |_, _, cx| {
                        v3.update(cx, |this, _| this.approve(Decision::Deny));
                    }),
                ),
        )
}

impl Render for NeoView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 1) 收事件（非阻塞）
        let changed = self.pump();
        // 2) 若有变化，把界面标记为需要重绘 —— 响应式宿主的核心一步
        if changed {
            cx.notify();
        }
        // 3) 轮次进行中：推进一步。
        //
        // 不需要"每帧排一帧"的动画机制：推进本身会产生事件批，
        // 驱动的唤醒钩子（见 `run`）会再触发一次重绘 —— 事件不断则循环自持。
        // 内核忙（暂无事件）时界面停住是对的，那时也没有新内容可画。
        if self.transcript.running && self.pending.is_none() {
            self.pump_step();
        }

        let blocks = self.transcript.blocks.clone();
        let running = self.transcript.running;
        let mode = self.mode;
        let model = self.model.clone();
        let status = self.status.clone();
        let notice = self.notice.clone();
        let total = (self.transcript.total_in, self.transcript.total_out);
        let input = self.input.clone();
        let blocked = self.pending.is_some();

        let view_entity = cx.entity().clone();
        let view_for_submit = cx.entity().clone();

        let mut root = v_flex()
            .size_full()
            // ⚠️ 用 base_bg() 而**不是** neo_color(Tone::None)：后者兜底到正文色（近白），
            // 当底色用会画出白底白字（真机截图抓到的）
            .bg(neo_ui::base_bg())
            .text_color(neo_color(Tone::Text))
            // ── 状态行 ──
            .child(
                h_flex()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .text_color(neo_color(Tone::Accent))
                            .child("NEO"),
                    )
                    .child(div().text_color(neo_color(Tone::Muted)).child(status))
                    .child(
                        div()
                            .text_color(neo_color(if matches!(
                                mode,
                                ExecMode::AutoEdit | ExecMode::FullAccess
                            ) {
                                Tone::Warning
                            } else {
                                Tone::Info
                            }))
                            .child(mode_label(mode)),
                    )
                    .child(
                        div()
                            .text_color(neo_color(Tone::Muted))
                            .child(format!("模型：{model}")),
                    )
                    // 高风险档位常驻提示（ZCode 语义：风险状态不能只在切档时弹一次）
                    .child(if matches!(mode, ExecMode::AutoEdit | ExecMode::FullAccess) {
                        div()
                            .text_color(neo_color(Tone::Warning))
                            .child("⚠ 写操作可能不经确认")
                    } else {
                        div()
                    })
                    .child(
                        div()
                            .text_color(neo_color(Tone::Muted))
                            .child(format!("{} in / {} out", total.0, total.1)),
                    )
                    .child(if let Some(n) = notice {
                        div().text_color(neo_color(Tone::Success)).child(n)
                    } else {
                        div()
                    })
                    .child(
                        div()
                            .text_color(neo_color(if self.pending.is_some() {
                                Tone::Warning
                            } else if running {
                                Tone::Info
                            } else {
                                Tone::Muted
                            }))
                            .child(if self.pending.is_some() {
                                "待审批"
                            } else if running {
                                "运行中"
                            } else {
                                "就绪"
                            }),
                    ),
            )
            // ── 转录区 ──
            .child(
                div()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(transcript_view(&blocks)),
            );

        // ── 审批对话框（模态覆盖）──
        if let Some(_) = self.pending.clone() {
            root = root.child(approval_dialog(self, cx));
        }

        // ── 输入区（审批未决时阻塞）──
        root.child(
            h_flex()
                .gap_2()
                .px_3()
                .py_2()
                .child(
                    div()
                        .flex_1()
                        .text_color(if blocked {
                            neo_color(Tone::Muted)
                        } else {
                            neo_color(Tone::Text)
                        })
                        .child(if blocked {
                            "待审批：请先在上方选择（避免把下一步排进队列）".to_string()
                        } else if input.is_empty() {
                            "输入任务后回车提交（@文件 / $技能 可用）".to_string()
                        } else {
                            input.clone()
                        }),
                )
                .child(Button::new("send").label("发送").on_click(move |_, _, cx| {
                    view_for_submit.update(cx, |this, _| this.submit());
                })),
        )
        // 模式切换：`Shift+Tab` 循环（对齐 ZCode）
        .on_key_down(move |ev, _window, cx| {
            if ev.keystroke.modifiers.shift && ev.keystroke.key == "tab" {
                view_entity.update(cx, |this, _| this.cycle_mode());
            }
        })
    }
}

/// 打开窗口并运行到关闭。
pub fn run(
    handle: KernelHandle,
    title: String,
    status: String,
    mode: ExecMode,
    model: String,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
) -> Result<(), String> {
    let view_holder: Arc<std::sync::Mutex<Option<Entity<NeoView>>>> =
        Arc::new(std::sync::Mutex::new(None));

    neo_ui_kit::application()
        .with_assets(neo_ui_kit::assets::Assets)
        .run(move |cx| {
            neo_ui_kit::init(cx);
            // 品牌主题：NEO 的紫（从 `neo-text` 的调色板派生，不写字面量）
            neo_ui::apply_neo_theme(cx);

            let view = cx.new(|_cx| NeoView::new(handle.clone(), status, mode, model));
            *view_holder.lock().unwrap_or_else(|e| e.into_inner()) = Some(view.clone());

            // 唤醒钩子：事件到达 → notify 视图 → 重绘。
            // 这是响应式宿主唯一的重绘触发点（gpui 不出帧就不画）。
            if let Some(wake) = wake {
                wake();
            }

            let view_for_window = view.clone();
            cx.spawn(async move |cx| {
                // 窗口尺寸要算在**主 App 上**（要有 display 信息），而这里拿到的是
                // `AsyncApp`（异步上下文）—— 所以先用 `update` 借一次主 App。
                let bounds = cx.update(|cx| {
                    neo_ui_kit::gpui::Bounds::centered(
                        None,
                        neo_ui_kit::gpui::size(px(1080.), px(720.)),
                        cx,
                    )
                });
                let _ = cx.open_window(
                    neo_ui_kit::gpui::WindowOptions {
                        window_bounds: Some(neo_ui_kit::gpui::WindowBounds::Windowed(bounds)),
                        titlebar: Some(neo_ui_kit::gpui::TitlebarOptions {
                            title: Some(title.clone().into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    move |window, cx| {
                        // ⚠️ 窗口首层必须是 `Root`：否则 dialog/通知/tooltip
                        // 会 panic 或静默失效（见 gpui-component 的 Root::update）
                        let inner = view_for_window.clone();
                        cx.new(|cx| Root::new(inner, window, cx))
                    },
                );
            })
            .detach();
        });

    Ok(())
}

/// 供测试与调用方检查工具参数摘要（转发共享实现，避免宿主各写一套）。
pub use neo_driver::transcript::summarize_args as summarize_tool_args;

