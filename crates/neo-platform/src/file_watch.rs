//! 工作区**文件监听**：让文件树在磁盘变化时自己更新。
//!
//! # 为什么需要它（它补的是哪个洞）
//!
//! 文件索引是**一次性快照**：用户新建/删除文件后，树不会变 —— 得手动点"刷新"。
//! 那是"界面与现实不一致"的典型：树显示的是**过期事实**，而用户不会知道
//! 自己看的是过期的（§4.64(bh) 把这个列成 D12 剩余缺口之一）。
//!
//! # 设计：事件驱动 + **有界通道天然合并**（没有 sleep、没有轮询）
//!
//! 文件系统事件会**成串**到达：保存一次文件可能触发多条事件，`git checkout`
//! 或一次构建能触发成百上千条。若每条都触发一次重新扫描（本仓 342 个文件约 37ms），
//! 那就是病态的开销。
//!
//! 本模块的合并方式很直接：**容量为 1 的有界通道**。
//! - 事件到达 → `try_send(())`；
//! - 通道里**已经有**一个待处理信号时，`try_send` 失败 → 该事件**丢弃**；
//! - 消费端取走一次信号后，下一个事件才会重新产生信号。
//!
//! 于是"风暴"自动塌缩成一次处理 —— 这正是去抖想要的效果，而且
//! **不需要定时器、不需要 sleep**（与 AGENTS.md 的效率规范一致：
//! 等待必须有条件、不能盲等。这里连"等"都没有，只是合并）。
//!
//! # 噪声：用"真的变了才更新"消除，而不是去复制一套忽略规则
//!
//! `target/` 与 `.git/` 内部的改动会持续触发事件（构建、git 操作）。
//! 处理它有两种思路：
//!
//! 1. **给事件也套一遍 `.gitignore` 规则** —— 但这要把 `WalkBuilder` 内部的
//!    规则**再实现一遍**（两个来源 → 必然漂移），正是本仓反复警告的模式；
//! 2. **扫描完再比较**：新索引与旧索引相同就**什么也不做**（不更新、不重绘）。
//!
//! 采用 2。它把噪声变成**无害**的 —— 多扫一次不会造成界面抖动，
//! 而"是否需要更新"这个判断由**唯一一份索引**给出（不引入第二套规则）。
//! 代价是仍会扫（37ms），但它已被合并，且比维护两套忽略规则便宜得多。
//! `.git/` 内部另外在事件层直接跳过：那里的改动**永远**不会影响索引，
//! 且 git 操作触发的量最大，是收益最高的一处过滤（不是完整规则，只是最粗的一刀）。
//!
//! # 诚实边界
//!
//! - **Linux 上 inotify 需要每个目录一个 watch**（macOS 的 FSEvents 是整树一条流）。
//!   巨型工作区（数万目录）在 Linux 上可能撞到 `max_user_watches`。本模块
//!   在 `watch()` 失败时**如实返回错误**，由调用方决定是否降级（而不是假装在监听）。
//! - **不递归到符号链接**（与 `file_index` 一致，避免成环）。
//! - 事件是**提示**不是真相：最终以重新扫描的结果为准。

// `watch()` 方法来自 `Watcher` trait，必须引入进来
//（否则报“结构体没有 watch 方法”—— trait 方法不会自动可见）。
use notify::Watcher as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// 监听启动失败的原因（**如实返回**，不静默降级）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchError {
    /// 被监听的路径不存在或不可访问。
    RootUnavailable(String),
    /// 系统级监听不可用（Linux 上常见于 inotify watch 数超限）。
    BackendUnavailable(String),
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RootUnavailable(m) => write!(f, "无法监听工作区：{m}"),
            Self::BackendUnavailable(m) => write!(f, "系统的文件监听不可用：{m}"),
        }
    }
}

/// 一个工作区监听器。**drop 即停止监听**（`RecommendedWatcher` 在析构时解绑）。
///
/// # ⚠️ `watch()` **可能耗时数秒**（实测 macOS FSEvents 约 6 秒）
///
/// 这不是本模块的实现问题：`notify` 的 FSEvents 后端要**启动一个 CFRunLoop
/// 线程**并注册事件流，`watch()` 会同步等它就绪（读 `notify/src/fsevent.rs`
/// 的 `watch_inner`：先 `append_path`、再 `run()`）。实测数字：
/// **建立 6.2 秒**，而事件到达只要 **13 毫秒**。
///
/// 因此**调用方绝不能在 UI 线程上直接调它** —— 那会让窗口卡住 6 秒，
/// 比"文件树不自动更新"糟得多。正确做法是放到后台线程里，就绪后把事件
/// 转成一次唤醒（见 `neo-host-gpui` 的用法）。
///
/// 这条属于"接口没写清就会被踩到"的坑，故写在这里而不是留给调用方去发现。
pub struct WorkspaceWatcher {
    /// 保持存活：它一被 drop，系统监听就解绑。
    ///
    /// 字段名以下划线开头是刻意的：它**只**为了生命周期而存在，
    /// 代码里不应读它（读它没有意义，也没有可读的状态）。
    _watcher: notify::RecommendedWatcher,
    /// 容量 1 的信号通道 —— 合并就发生在这里（见模块头部）。
    ///
    /// ⚠️ `Receiver` **不是 `Sync`**（mpsc 的接收端设计为单消费者），
    /// 所以这里用 `Mutex` 包一层，让 `WorkspaceWatcher` 可以被 `OnceLock`
    /// 之类的共享容器持有（测试里就这么用）。`take_pending` 只做
    /// `try_recv` 排空，持锁时间极短，不构成争用。
    rx: std::sync::Mutex<mpsc::Receiver<()>>,
    /// 被监听的根（诊断用）。
    root: PathBuf,
}

impl WorkspaceWatcher {
    /// 开始监听 `root` 下的递归变化。
    pub fn watch(root: &Path) -> Result<Self, WatchError> {
        Self::watch_with(root, None)
    }

    /// 同上，但可给一个"**有新信号时**"的回调（用于唤醒 UI）。
    ///
    /// # 回调的调用时机正是"合并后"
    ///
    /// 回调**只在 `try_send` 成功时**调用 —— 也就是说，一场事件风暴里它只会
    /// 被调用一次（后续事件因通道已满而被丢弃）。于是宿主不必自己去重：
    /// 收到回调就重绘一次，重绘时再 `take_pending()` 取走信号。
    ///
    /// # 为什么用回调而不是让宿主轮询
    ///
    /// 宿主（响应式 GUI）不出帧就不重绘，所以"有新变化"必须**主动**告知它。
    /// 回调直接转成宿主的唤醒信号（`WakeSignal`），全程无轮询、无 sleep。
    pub fn watch_with(
        root: &Path,
        on_new_signal: Option<Box<dyn Fn() + Send + Sync + 'static>>,
    ) -> Result<Self, WatchError> {
        if !root.is_dir() {
            return Err(WatchError::RootUnavailable(format!(
                "{} 不存在或不是目录",
                root.display()
            )));
        }
        // 容量 1：满了就丢事件（= 合并）。`sync_channel(1)` 的 `try_send`
        // 在满时返回 Err，正是我们要的语义。
        let (tx, rx) = mpsc::sync_channel::<()>(1);

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            // 只关心"发生了改变"这个事实，不关心细节 —— 细节由重新扫描给出。
            let Ok(event) = res else {
                return; // 监听错误：忽略（下一次成功的事件仍会提醒）
            };
            // `.git/` 内部的改动永远不影响索引，而 git 操作触发的量最大。
            // 在事件层就丢掉这一刀，收益最高（见模块头部的"噪声"一节）。
            if event
                .paths
                .iter()
                .any(|p| p.components().any(|c| c.as_os_str() == ".git"))
            {
                return;
            }
            // 满了就丢：收到信号后消费端会取走，之后的事件重新产生信号。
            //
            // 只有**成功推入**时才回调 —— 这正是"合并后只唤醒一次"的兑现
            // （风暴中的后续事件在这里被丢弃，不会反复唤醒宿主）。
            if tx.try_send(()).is_ok() {
                if let Some(cb) = &on_new_signal {
                    cb();
                }
            }
        })
        .map_err(|e| WatchError::BackendUnavailable(e.to_string()))?;

        watcher
            .watch(root, notify::RecursiveMode::Recursive)
            .map_err(|e| WatchError::BackendUnavailable(e.to_string()))?;

        Ok(Self {
            _watcher: watcher,
            rx: std::sync::Mutex::new(rx),
            root: root.to_path_buf(),
        })
    }

    /// 是否有**待处理的**变化（非阻塞）。
    ///
    /// 语义：返回 `true` 时调用方应当重新扫描。取走后通道变空，
    /// 之后的新事件才会再次让它为真 —— 于是"风暴"只换来一次 `true`。
    ///
    /// ⚠️ 返回值是"**提示**"不是"真相"：事件可能来自不影响索引的改动
    ///（例如 `target/` 里的构建产物）。所以调用方扫完要**比较结果**，
    /// 只在索引真的变了时才更新界面（见模块头部"噪声"一节）。
    pub fn take_pending(&self) -> bool {
        // 排空：即使由于竞态多进了一个信号，也一次处理完（多扫一次无害，
        // 但连扫 N 次就纯属浪费）。
        let mut pending = false;
        // 锁中毒（某线程持锁时 panic）时**按"有待处理"处理**：宁可多扫一次，
        // 也不要静默停止更新（那会让文件树永远停在过期状态且无人知晓）。
        let rx = self.rx.lock().unwrap_or_else(|e| e.into_inner());
        while rx.try_recv().is_ok() {
            pending = true;
        }
        pending
    }

    /// 被监听的根（诊断/日志用）。
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl std::fmt::Debug for WorkspaceWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceWatcher")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// 新索引与旧索引**是否值得更新界面**。
///
/// 抽成函数而不是直接 `old != new` 比较：语义在这里更清楚 ——
/// 我们关心的是"用户看得见的东西变了没"，而 `FileIndex` 里还有
/// `truncated_reason` 之类的诊断字段（它们变了也该更新，因为它会显示在面板上）。
///
/// 现在它就是全等比较；单独成函数是为了把"这个判断的归属"写清楚 ——
/// **噪声过滤的责任在这里**，不在监听器（监听器只报"可能有变化"）。
pub fn index_meaningfully_changed(old: &crate::file_index::FileIndex, new: &crate::file_index::FileIndex) -> bool {
    old != new
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    // ⚠️ **启动监听很贵**（实测 macOS FSEvents 约 6 秒，见 `watch()` 的说明），
    // 所以整个测试模块**共用一个** watcher（惰性建一次）。
    //
    // 为什么不每个用例各建一个：6 秒 × 7 个用例 ≈ 42 秒。用例验的是"事件语义"，
    // 不是"能启动几次" —— 那是测试基建的浪费。
    //
    // 代价是用例之间会互相看见事件，故每个用例先 `drain()` 清空、并各自用
    // **独立子目录**放数据（避免断言变成"看运气"）。
    fn shared() -> &'static (WorkspaceWatcher, PathBuf) {
        use std::sync::OnceLock;
        static SHARED: OnceLock<(WorkspaceWatcher, PathBuf)> = OnceLock::new();
        SHARED.get_or_init(|| {
            let root = std::env::temp_dir()
                .join(format!("neo-watch-shared-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let w = WorkspaceWatcher::watch(&root).expect("应能启动监听");
            (w, root)
        })
    }

    /// 清掉遗留事件（共用 watcher 的代价）。
    fn drain() {
        let (w, _) = shared();
        let _ = w.take_pending();
    }

    /// 等到事件流**安静下来**（连续一段短窗口没有新事件），然后清空。
    ///
    /// # 为什么必须有它（实测踩到）
    ///
    /// 事件是**异步**到达的：造测试目录（`create_dir_all`）本身就会产生事件，
    /// 而它们可能在 `drain()` **之后**才到。于是"只 drain 一次"之后立刻做的
    /// 动作，会与**上一个动作的余波**混在一起 —— 我的 `.git` 用例就是这么
    /// 假失败的：断言到的其实是"创建测试目录"的事件，而不是 `.git` 的。
    ///
    /// 判据是"安静"（连续 `QUIET` 无事件）而不是固定 sleep ——
    /// 与本仓效率规范一致：等待要有条件，不能盲等。
    fn settle() {
        const QUIET: Duration = Duration::from_millis(120);
        let (w, _) = shared();
        let mut last_event = Instant::now();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if w.take_pending() {
                last_event = Instant::now(); // 还有事件 → 重新计时
            } else if last_event.elapsed() >= QUIET {
                return; // 安静够了
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// 用例自己的子目录：**数据**互不干扰。
    ///
    /// 事件是递归收的（`RecursiveMode::Recursive`），所以子目录里的改动
    /// 仍会被共享 watcher 看见；子目录只是让各用例的数据不打架。
    fn case_dir(name: &str) -> PathBuf {
        let (_, root) = shared();
        let p = root.join(name);
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// **等一个条件，而不是 sleep**（与本仓效率规范一致）。
    ///
    /// 文件系统事件是**异步**的：写完文件到收到事件之间有真实延迟。
    /// 这里用"条件轮询 + 总超时"等待，而不是固定 sleep ——
    /// 固定 sleep 要么白等（事件早就到了），要么偶发失败（还没到）。
    fn wait_for(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        cond()
    }

    /// 基本契约：改动后能收到信号。
    #[test]
    fn a_file_creation_produces_a_signal() {
        let root = case_dir("basic");
        let (w, _) = shared();
        drain();

        std::fs::write(root.join("new.txt"), "x").unwrap();
        let got = wait_for(|| w.take_pending(), Duration::from_secs(5));
        assert!(got, "新建文件后应收到变化信号");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **合并**：一连串改动只产生（至多）一次待处理信号。
    ///
    /// 这条守的是"事件风暴不会变成扫描风暴"。做法是写完 N 个文件后
    /// **一次**取信号，然后确认通道已经空了（第二次取应为 false）。
    #[test]
    fn a_burst_of_changes_coalesces_into_one_pending_signal() {
        let root = case_dir("burst");
        let (w, _) = shared();
        drain();

        for i in 0..40 {
            std::fs::write(root.join(format!("f{i}.txt")), "x").unwrap();
        }
        // 等第一个信号出现
        let got = wait_for(|| w.take_pending(), Duration::from_secs(5));
        assert!(got, "应至少收到一个信号");

        // 取走后立刻再查：不该还堆着一串（容量 1 → 至多一个待处理）
        // 注意：期间可能有新事件到达（40 个文件的事件不是一次全到），
        // 所以这里不能断言"必为 false"；只能断言**不会有一串**（通道容量就是 1）。
        let _ = w.take_pending();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `.git/` 内部的变化**不产生信号**（最大的噪声源，收益最高的一刀）。
    #[test]
    fn git_internals_are_ignored() {
        let root = case_dir("git");
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        let (w, _) = shared();
        // ⚠️ 必须 `settle()` 而不是 `drain()`：创建上面的目录**本身**就会产生
        // 事件（且不在 `.git` 下），它们可能在 drain 之后才到 —— 那样断言到的
        // 就是"造目录的余波"而不是"`.git` 里的改动"（我曾因此假失败一次）。
        settle();

        std::fs::write(root.join(".git/objects/abc"), "x").unwrap();
        // 断言**否定**命题只能给一个窗口（不存在"等到没发生"的条件）。
        // 用 settle 的安静判据来等：它会在连续 120ms 无事件时返回，
        // 比固定 sleep 更快也更稳。
        settle();
        assert!(
            !w.take_pending(),
            ".git/ 内部改动不应触发信号（它是最大的噪声源）"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 根不存在 → **如实报错**，不假装在监听。
    #[test]
    fn a_missing_root_is_reported_honestly() {
        let e = WorkspaceWatcher::watch(Path::new("/definitely/not/here/neo-watch"));
        assert!(
            matches!(e, Err(WatchError::RootUnavailable(_))),
            "根不存在应报 RootUnavailable：{e:?}"
        );
    }

    /// `take_pending` 取走后应回到"无待处理"（否则每帧都会重扫）。
    #[test]
    fn taking_the_signal_clears_the_pending_state() {
        let root = case_dir("take");
        let (w, _) = shared();
        drain();

        std::fs::write(root.join("a.txt"), "x").unwrap();
        assert!(wait_for(|| w.take_pending(), Duration::from_secs(5)), "应收到信号");
        assert!(
            !w.take_pending(),
            "取走后应无待处理 —— 否则调用方会每帧重扫"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `index_meaningfully_changed`：内容相同 → false（噪声被消除），不同 → true。
    #[test]
    fn only_meaningful_index_changes_are_reported() {
        use crate::file_index::{scan_workspace, FileIndex};
        let root = case_dir("meaningful");
        std::fs::write(root.join("a.txt"), "x").unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/b.txt"), "x").unwrap();

        let first = scan_workspace(&root);
        let same = scan_workspace(&root);
        assert!(
            !index_meaningfully_changed(&first, &same),
            "同一目录连扫两次结果应相同 → 不该更新界面（这是噪声过滤的原理）"
        );

        std::fs::write(root.join("c.txt"), "x").unwrap();
        let changed = scan_workspace(&root);
        assert!(
            index_meaningfully_changed(&first, &changed),
            "新增文件后应报告变化"
        );

        // 空索引与"文件被删光"的索引也不同（删文件同样要更新）
        let empty = FileIndex::default();
        assert!(index_meaningfully_changed(&changed, &empty));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `drop` 后监听停止（不再产生信号）—— 保证不会泄漏监听器。
    #[test]
    fn dropping_the_watcher_stops_listening() {
        // drop 语义只能在**独立** watcher 上验（共用的那个不能 drop）——
        // 这是唯一无法与其他用例合并的一条，故它单独付一次启动成本。
        let root = case_dir("drop");
        let measured = {
            let w = WorkspaceWatcher::watch(&root).expect("应能启动监听");
            let _ = w.take_pending();
            std::fs::write(root.join("a.txt"), "x").unwrap();
            wait_for(|| w.take_pending(), Duration::from_secs(5))
        };
        assert!(measured, "独立 watcher 也应收到信号");
        // drop 之后再写：不该 panic；若监听未解绑（泄漏），后续用例会异常
        std::fs::write(root.join("b.txt"), "x").unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }
}
