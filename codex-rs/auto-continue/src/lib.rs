//! # AutoContinue 扩展 (ACX) 核心库
//!
//! 该 crate 提供 AutoContinue 的核心状态机逻辑，不依赖任何 Codex 特定类型。
//! 可以被 Codex TUI 或其他 CLI 工具集成使用。
//!
//! ## 核心组件
//!
//! - `AutoContinueManager`: 状态机，管理自动继续/重试的完整生命周期
//! - `AcxAction`: 状态机的输出动作（无操作 / 发送继续 / 已停止）
//! - `TurnOutcome`: 一次 Turn 的结果（完成 / 失败 / 中断）
//! - `AcxErrorKind`: 错误分类（含 `is_retryable()` 方法）
//! - `AcxConfig`: 运行时配置
//! - `PromptSource`: 提示词来源
//! - Hook 系统: 可扩展的中断钩子

pub mod config;
pub mod hook;
pub mod prompt;

use anyhow::Result;

pub use config::AcxConfig;
pub use hook::{
    CommandHook, DurationHook, ErrorHook, HookVerdict, RoundHook, StopHook, StopHookManager,
    TimeHook, parse_stop_when,
};
pub use prompt::PromptSource;

/// 指数退避的最大延迟秒数上限
///
/// 当连续失败时，延迟会指数增长（翻倍），但不会超过此上限。
const MAX_BACKOFF_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// AcxAction — 状态机输出动作
// ---------------------------------------------------------------------------

/// ACX 状态机的输出动作
///
/// 表示状态机在处理事件后建议上层执行的动作。
#[derive(Debug, Clone)]
pub enum AcxAction {
    /// 无需动作
    ///
    /// 当前不需要发送任何内容（如计时器未触发、已取消等）。
    None,

    /// 发送继续提示词
    ///
    /// 计时器触发后，解析提示词成功，上层应将此字符串发送到 CLI。
    SendContinue(String),

    /// ACX 已停止
    ///
    /// 因某种原因停止了自动继续功能，附带停止原因描述。
    Stopped {
        /// 停止原因的人类可读描述
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// AcxAppEvent — 应用事件
// ---------------------------------------------------------------------------

/// ACX 应用事件
///
/// 通过 Codex 的 `AppEvent` 通道传递的 ACX 内部事件。
/// 上层只需在 `AppEvent` 中保留一个 `AcxEvent(AcxAppEvent)` 变体，
/// 所有路由逻辑由 ACX 桥接层处理。
#[derive(Debug)]
pub enum AcxAppEvent {
    /// 计时器触发，附带解析好的提示词
    TimerFired { text: String },
    /// 用户请求停止自动继续（/acx-stop）
    StopRequested,
}

// ---------------------------------------------------------------------------
// TurnOutcome — Turn 结果
// ---------------------------------------------------------------------------

/// 一次 Turn 的结果
///
/// 由上层调用者在每次 Turn 完成后提供给 `AutoContinueManager`，
/// 用于决定下一步的行为（继续 / 重试 / 停止）。
#[derive(Debug, Clone)]
pub enum TurnOutcome {
    /// Turn 正常完成
    ///
    /// CLI 工具成功完成了本次操作，可以继续下一轮。
    Completed,

    /// Turn 失败
    ///
    /// CLI 工具在本次操作中遇到错误。
    /// `kind` 字段描述错误类型，通过 `is_retryable()` 判断是否可重试。
    Failed {
        /// 错误类型分类
        kind: AcxErrorKind,
    },

    /// Turn 被中断
    ///
    /// 用户主动中断了操作（如按下 Ctrl+C、输入 /stop 等）。
    /// 中断后 ACX 应立即停止。
    Interrupted,
}

// ---------------------------------------------------------------------------
// AcxErrorKind — 错误分类
// ---------------------------------------------------------------------------

/// ACX 错误分类枚举
///
/// 将 CLI 工具可能遇到的错误分为可重试和不可重试两大类。
/// 通过 `is_retryable()` 方法查询是否值得自动重试。
#[derive(Debug, Clone)]
pub enum AcxErrorKind {
    // --- 可重试错误 ---

    /// API 速率限制（429 Too Many Requests）
    ///
    /// 通常等待一段时间后即可恢复，适合指数退避重试。
    RateLimit,

    /// 服务器过载（503 Service Unavailable 等）
    ///
    /// 服务器暂时无法处理请求，等待后可能恢复。
    ServerOverloaded,

    /// 连接失败（网络中断、DNS 解析失败等）
    ///
    /// 网络问题通常是暂时性的，重试可能成功。
    ConnectionFailed,

    /// 服务器内部错误（500 Internal Server Error）
    ///
    /// 服务端偶发错误，重试可能成功。
    InternalServer,

    /// 尝试次数过多
    ///
    /// 虽然归类为可重试，但通常意味着需要更长的等待时间。
    TooManyAttempts,

    // --- 不可重试错误 ---

    /// 上下文超出限制（Token 数超限）
    ///
    /// 需要用户调整输入，重试不会有帮助。
    ContextExceeded,

    /// 认证/授权失败（401/403）
    ///
    /// 需要用户检查 API Key 或权限，重试不会有帮助。
    Unauthorized,

    /// 其他未分类错误
    ///
    /// 包含错误描述文本，默认为不可重试。
    Other(String),
}

impl AcxErrorKind {
    /// 判断此错误类型是否值得自动重试
    ///
    /// 可重试的错误通常是暂时性的（网络、速率限制、服务器负载），
    /// 等待一段时间后重试可能成功。
    ///
    /// 不可重试的错误是永久性的（认证失败、上下文超限），
    /// 重试不会改变结果，应该停止并提示用户。
    ///
    /// # 返回值
    /// - `true`: 可重试（RateLimit、ServerOverloaded、ConnectionFailed、InternalServer、TooManyAttempts）
    /// - `false`: 不可重试（ContextExceeded、Unauthorized、Other）
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            AcxErrorKind::RateLimit
                | AcxErrorKind::ServerOverloaded
                | AcxErrorKind::ConnectionFailed
                | AcxErrorKind::InternalServer
                | AcxErrorKind::TooManyAttempts
        )
    }
}

// ---------------------------------------------------------------------------
// AcxState — 内部状态
// ---------------------------------------------------------------------------

/// ACX 内部状态枚举
///
/// 跟踪 `AutoContinueManager` 当前处于哪个阶段。
/// 状态转换由 `on_turn_completed`、`on_timer_fired_sync`、`cancel_pending`
/// 和 `request_stop` 方法驱动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcxState {
    /// 监控中：等待 Turn 完成
    ///
    /// 初始状态或计时器被取消后的状态。
    /// 在此状态下等待上层调用 `on_turn_completed()`。
    Monitoring,

    /// 等待发送：延迟已记录，等待外部计时器触发
    ///
    /// `on_turn_completed()` 记录了待执行延迟，等待外部 bridge 层启动计时器。
    /// 此状态下上层可以通过 `take_pending_delay()` 获取延迟秒数。
    WaitingToSend,

    /// 已停止：ACX 不再活跃
    ///
    /// 因以下原因之一进入此状态：
    /// - 用户请求停止（`request_stop()`）
    /// - Turn 被中断（`TurnOutcome::Interrupted`）
    /// - 遇到不可重试的错误
    /// - 钩子触发停止
    Stopped,
}

// ---------------------------------------------------------------------------
// AutoContinueManager — 核心状态机
// ---------------------------------------------------------------------------

/// AutoContinue 管理器（核心状态机）
///
/// 管理自动继续/重试的完整生命周期。通过以下方法与上层交互：
///
/// - `on_turn_completed(outcome)`: Turn 完成时调用，记录待执行延迟
/// - `take_pending_delay()`: 获取延迟秒数（一次性消费），用于外部启动计时器
/// - `on_timer_fired_sync()`: 计时器触发后调用，更新内部状态
/// - `cancel_pending()`: 取消正在等待的计时器（用户手动输入时）
/// - `request_stop()`: 请求停止 ACX（用户命令 /acx-stop）
/// - `is_waiting_to_send()`: 检查是否在等待发送状态
/// - `config()`: 获取配置引用（用于外部解析提示词）
///
/// ## 典型使用模式
///
/// ```ignore
/// let mut acx = AutoContinueManager::new(config)?;
///
/// // Turn 完成后
/// acx.on_turn_completed(outcome);
///
/// // 检查是否需要启动计时器
/// if let Some(delay) = acx.take_pending_delay() {
///     // 在外部 bridge 层启动计时器任务
///     tokio::spawn(async move {
///         tokio::time::sleep(Duration::from_secs(delay)).await;
///         // 发送 AcxAppEvent::TimerFired { text } 到 AppEvent 通道
///     });
/// }
/// ```
pub struct AutoContinueManager {
    /// 运行时配置
    config: AcxConfig,

    /// 当前状态
    state: AcxState,

    /// 已完成的轮次计数
    round_count: u64,

    /// 中断钩子管理器
    hook_manager: StopHookManager,

    /// 待执行的延迟秒数
    ///
    /// 当 `on_turn_completed()` 决定需要继续时，记录延迟秒数。
    /// 外部 bridge 层通过 `take_pending_delay()` 获取并启动计时器任务。
    /// `None` 表示当前没有待启动的计时器。
    pending_delay_secs: Option<u64>,

    /// 当前延迟秒数
    ///
    /// 成功时使用基础延迟（`config.delay_seconds`），
    /// 失败时指数退避（翻倍，上限 `MAX_BACKOFF_SECS`）。
    current_delay: u64,

    /// 连续失败次数
    ///
    /// 用于计算指数退避延迟。成功时重置为 0。
    consecutive_failures: u64,
}

impl AutoContinueManager {
    /// 创建新的 AutoContinueManager
    ///
    /// 解析配置中的 `stop_whens` 和 `stop_hooks`，注册到钩子管理器。
    ///
    /// # 参数
    /// - `config`: ACX 运行时配置
    ///
    /// # 返回值
    /// 成功返回管理器实例，失败返回错误（如钩子解析失败、命令钩子启动失败）
    ///
    /// # 错误
    /// - `stop_whens` 中包含无法解析的规格字符串
    /// - `stop_hooks` 中的命令无法启动
    pub fn new(config: AcxConfig) -> Result<Self> {
        let mut hook_manager = StopHookManager::new();

        // 解析并注册预设中断条件
        for spec in &config.stop_whens {
            let hook = parse_stop_when(spec)?;
            hook_manager.add(hook);
        }

        // 解析并注册自定义命令钩子
        for cmd in &config.stop_hooks {
            let hook = CommandHook::new(cmd)?;
            hook_manager.add(Box::new(hook));
        }

        let delay = config.delay_seconds;

        Ok(Self {
            config,
            state: AcxState::Monitoring,
            round_count: 0,
            hook_manager,
            pending_delay_secs: None,
            current_delay: delay,
            consecutive_failures: 0,
        })
    }

    /// Turn 完成时调用
    ///
    /// 根据 Turn 结果决定下一步行为：
    ///
    /// 1. **Interrupted** → 立即停止
    /// 2. **Failed + 不可重试** → 停止
    /// 3. **检查钩子** → 任意钩子触发 → 停止
    /// 4. **计算延迟**：
    ///    - 成功：使用基础延迟（`config.delay_seconds`）
    ///    - 失败（可重试）：指数退避（翻倍，上限 300 秒）
    /// 5. **启动计时器**
    /// 6. **更新连续失败计数**：成功时重置为 0，失败时 +1
    ///
    /// # 参数
    /// - `outcome`: 本次 Turn 的结果
    pub fn on_turn_completed(&mut self, outcome: TurnOutcome) {
        // 已停止状态下忽略
        if self.state == AcxState::Stopped {
            return;
        }

        // 未启用时忽略
        if !self.config.enabled {
            return;
        }

        // 1. 中断 → 立即停止
        if matches!(outcome, TurnOutcome::Interrupted) {
            self.state = AcxState::Stopped;
            self.pending_delay_secs = None;
            tracing::info!("[ACX] Turn 被中断，停止自动继续");
            return;
        }

        // 2. 不可重试错误 → 停止
        if let TurnOutcome::Failed { ref kind } = outcome {
            if !kind.is_retryable() {
                self.state = AcxState::Stopped;
                self.pending_delay_secs = None;
                tracing::info!("[ACX] 遇到不可重试错误 ({kind:?})，停止自动继续");
                return;
            }
        }

        // 3. 检查钩子
        if self.hook_manager.should_stop(self.round_count, &outcome) {
            self.state = AcxState::Stopped;
            self.pending_delay_secs = None;
            return;
        }

        // 4. 计算延迟
        let delay = match outcome {
            TurnOutcome::Completed => {
                // 成功：使用基础延迟
                self.config.delay_seconds
            }
            TurnOutcome::Failed { .. } => {
                // 失败（可重试）：指数退避
                let backoff = self.config.delay_seconds * 2u64.saturating_pow(
                    self.consecutive_failures.min(32) as u32
                );
                backoff.min(MAX_BACKOFF_SECS)
            }
            TurnOutcome::Interrupted => unreachable!(),
        };
        self.current_delay = delay;

        // 5. 记录待执行延迟（供外部 bridge 层查询）
        self.pending_delay_secs = Some(delay);
        self.state = AcxState::WaitingToSend;

        // 6. 更新连续失败计数和轮次
        match outcome {
            TurnOutcome::Completed => {
                self.consecutive_failures = 0;
            }
            TurnOutcome::Failed { .. } => {
                self.consecutive_failures += 1;
            }
            TurnOutcome::Interrupted => unreachable!(),
        }

        self.round_count += 1;

        tracing::debug!(
            "[ACX] 第{}轮，{}秒后自动继续 (连续失败: {})",
            self.round_count,
            self.current_delay,
            self.consecutive_failures
        );
    }

    /// 获取待执行的延迟秒数
    ///
    /// 当 `on_turn_completed()` 决定需要继续时，此方法返回等待延迟。
    /// 外部 bridge 层可以据此启动计时器任务。调用后清除延迟（一次性消费）。
    pub fn take_pending_delay(&mut self) -> Option<u64> {
        self.pending_delay_secs.take()
    }

    /// 计时器触发后调用（同步版本）
    ///
    /// 仅更新内部状态（轮次、状态），不执行异步操作。
    /// 提示词解析由外部 bridge 层在 spawned task 中完成。
    pub fn on_timer_fired_sync(&mut self) {
        self.state = AcxState::Monitoring;
        tracing::info!("[ACX] 计时器触发（第{}轮）", self.round_count);
    }

    /// 获取当前状态是否在等待发送
    pub fn is_waiting_to_send(&self) -> bool {
        self.state == AcxState::WaitingToSend
    }

    /// 获取配置引用
    pub fn config(&self) -> &AcxConfig {
        &self.config
    }

    /// 取消正在等待的计时器
    ///
    /// 当用户在计时器等待期间手动输入时调用。
    /// 清除延迟记录，状态回到 `Monitoring`。
    pub fn cancel_pending(&mut self) {
        if self.state == AcxState::WaitingToSend {
            self.pending_delay_secs = None;
            self.state = AcxState::Monitoring;
            tracing::debug!("[ACX] 计时器已取消（用户手动输入）");
        }
    }

    /// 请求停止 ACX
    ///
    /// 由用户命令（如 `/acx-stop`）触发。
    /// 取消延迟记录并设置状态为 `Stopped`。
    pub fn request_stop(&mut self) {
        self.pending_delay_secs = None;
        self.state = AcxState::Stopped;
        tracing::info!("[ACX] 用户请求停止");
    }

    /// 检查 ACX 是否仍然活跃
    ///
    /// # 返回值
    /// - `true`: 状态不是 `Stopped`（仍在运行）
    /// - `false`: 已停止
    pub fn is_active(&self) -> bool {
        self.state != AcxState::Stopped
    }

    /// 获取当前状态的人类可读文本
    ///
    /// 用于 TUI 显示 ACX 的当前状态。
    ///
    /// # 返回值
    /// - `Some(text)`: 有活跃状态时返回状态描述
    /// - `None`: 已停止时返回 None
    pub fn status_text(&self) -> Option<String> {
        match self.state {
            AcxState::Monitoring => {
                Some(format!("[ACX] 监控中 | 第{}轮", self.round_count))
            }
            AcxState::WaitingToSend => {
                Some(format!(
                    "[ACX] {}秒后自动继续 | 第{}轮",
                    self.current_delay, self.round_count
                ))
            }
            AcxState::Stopped => None,
        }
    }
}

// ===========================================================================
// 单元测试
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 创建测试用的默认配置
    fn test_config() -> AcxConfig {
        AcxConfig {
            enabled: true,
            prompt: PromptSource::Static("Continue".to_string()),
            delay_seconds: 15,
            stop_whens: Vec::new(),
            stop_hooks: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // AcxErrorKind 测试
    // -----------------------------------------------------------------------

    /// 测试可重试错误
    #[test]
    fn test_retryable_errors() {
        assert!(AcxErrorKind::RateLimit.is_retryable());
        assert!(AcxErrorKind::ServerOverloaded.is_retryable());
        assert!(AcxErrorKind::ConnectionFailed.is_retryable());
        assert!(AcxErrorKind::InternalServer.is_retryable());
        assert!(AcxErrorKind::TooManyAttempts.is_retryable());
    }

    /// 测试不可重试错误
    #[test]
    fn test_non_retryable_errors() {
        assert!(!AcxErrorKind::ContextExceeded.is_retryable());
        assert!(!AcxErrorKind::Unauthorized.is_retryable());
        assert!(!AcxErrorKind::Other("unknown".to_string()).is_retryable());
    }

    // -----------------------------------------------------------------------
    // 状态机基本测试
    // -----------------------------------------------------------------------

    /// 测试成功 Turn 后记录待执行延迟
    #[test]
    fn test_completed_turn_starts_timer() {
        let config = test_config();
        let mut acx = AutoContinueManager::new(config).unwrap();

        acx.on_turn_completed(TurnOutcome::Completed);

        assert!(acx.is_waiting_to_send());
        assert!(acx.is_active());
        assert_eq!(acx.state, AcxState::WaitingToSend);
        // take_pending_delay 消费延迟值
        let delay = acx.take_pending_delay();
        assert!(delay.is_some());
        assert_eq!(delay.unwrap(), 15); // 默认延迟秒数
        // 再次调用应返回 None（已消费）
        assert!(acx.take_pending_delay().is_none());
    }

    /// 测试成功 Turn 后重置连续失败计数
    #[test]
    fn test_completed_resets_failures() {
        let config = test_config();
        let mut acx = AutoContinueManager::new(config).unwrap();

        // 先产生一些失败
        acx.on_turn_completed(TurnOutcome::Failed {
            kind: AcxErrorKind::RateLimit,
        });
        assert_eq!(acx.consecutive_failures, 1);

        // 成功后重置
        acx.on_turn_completed(TurnOutcome::Completed);
        assert_eq!(acx.consecutive_failures, 0);
    }

    /// 测试可重试失败后记录待执行延迟并增加失败计数
    #[test]
    fn test_retryable_failure_starts_timer() {
        let config = test_config();
        let mut acx = AutoContinueManager::new(config).unwrap();

        acx.on_turn_completed(TurnOutcome::Failed {
            kind: AcxErrorKind::RateLimit,
        });

        assert!(acx.is_waiting_to_send());
        assert!(acx.take_pending_delay().is_some());
        assert!(acx.is_active());
        assert_eq!(acx.consecutive_failures, 1);
    }

    /// 测试不可重试失败后停止
    #[test]
    fn test_non_retryable_failure_stops() {
        let config = test_config();
        let mut acx = AutoContinueManager::new(config).unwrap();

        acx.on_turn_completed(TurnOutcome::Failed {
            kind: AcxErrorKind::ContextExceeded,
        });

        assert!(!acx.is_active());
        assert!(acx.take_pending_delay().is_none());
        assert_eq!(acx.state, AcxState::Stopped);
    }

    /// 测试中断后停止
    #[test]
    fn test_interrupted_stops() {
        let config = test_config();
        let mut acx = AutoContinueManager::new(config).unwrap();

        acx.on_turn_completed(TurnOutcome::Interrupted);

        assert!(!acx.is_active());
        assert_eq!(acx.state, AcxState::Stopped);
    }

    // -----------------------------------------------------------------------
    // 指数退避测试
    // -----------------------------------------------------------------------

    /// 测试指数退避：延迟应翻倍
    #[test]
    fn test_exponential_backoff_doubles() {
        let config = AcxConfig {
            delay_seconds: 10,
            ..test_config()
        };
        let mut acx = AutoContinueManager::new(config).unwrap();

        // 第 1 次失败：delay = 10 * 2^0 = 10
        acx.on_turn_completed(TurnOutcome::Failed {
            kind: AcxErrorKind::RateLimit,
        });
        assert_eq!(acx.current_delay, 10);

        // 第 2 次失败：delay = 10 * 2^1 = 20
        acx.on_turn_completed(TurnOutcome::Failed {
            kind: AcxErrorKind::RateLimit,
        });
        assert_eq!(acx.current_delay, 20);

        // 第 3 次失败：delay = 10 * 2^2 = 40
        acx.on_turn_completed(TurnOutcome::Failed {
            kind: AcxErrorKind::RateLimit,
        });
        assert_eq!(acx.current_delay, 40);
    }

    /// 测试指数退避：上限 300 秒
    #[test]
    fn test_exponential_backoff_max_cap() {
        let config = AcxConfig {
            delay_seconds: 100,
            ..test_config()
        };
        let mut acx = AutoContinueManager::new(config).unwrap();

        // 第 1 次失败：100 * 2^0 = 100
        acx.on_turn_completed(TurnOutcome::Failed {
            kind: AcxErrorKind::ServerOverloaded,
        });
        assert_eq!(acx.current_delay, 100);

        // 第 2 次失败：100 * 2^1 = 200
        acx.on_turn_completed(TurnOutcome::Failed {
            kind: AcxErrorKind::ServerOverloaded,
        });
        assert_eq!(acx.current_delay, 200);

        // 第 3 次失败：100 * 2^2 = 400 → cap 到 300
        acx.on_turn_completed(TurnOutcome::Failed {
            kind: AcxErrorKind::ServerOverloaded,
        });
        assert_eq!(acx.current_delay, MAX_BACKOFF_SECS);
    }

    /// 测试成功后退避延迟重置
    #[test]
    fn test_backoff_resets_on_success() {
        let config = AcxConfig {
            delay_seconds: 10,
            ..test_config()
        };
        let mut acx = AutoContinueManager::new(config).unwrap();

        // 连续失败使延迟增加
        acx.on_turn_completed(TurnOutcome::Failed {
            kind: AcxErrorKind::RateLimit,
        });
        acx.on_turn_completed(TurnOutcome::Failed {
            kind: AcxErrorKind::RateLimit,
        });
        assert_eq!(acx.current_delay, 20);

        // 成功后重置
        acx.on_turn_completed(TurnOutcome::Completed);
        assert_eq!(acx.current_delay, 10);
        assert_eq!(acx.consecutive_failures, 0);
    }

    // -----------------------------------------------------------------------
    // cancel_pending 测试
    // -----------------------------------------------------------------------

    /// 测试 cancel_pending 取消延迟记录
    #[test]
    fn test_cancel_pending() {
        let config = test_config();
        let mut acx = AutoContinueManager::new(config).unwrap();

        acx.on_turn_completed(TurnOutcome::Completed);
        assert!(acx.is_waiting_to_send());

        acx.cancel_pending();
        assert!(acx.take_pending_delay().is_none());
        assert_eq!(acx.state, AcxState::Monitoring);
        assert!(acx.is_active());
    }

    /// 测试在 Monitoring 状态下 cancel_pending 无效果
    #[test]
    fn test_cancel_pending_in_monitoring() {
        let config = test_config();
        let mut acx = AutoContinueManager::new(config).unwrap();

        // 初始状态就是 Monitoring
        acx.cancel_pending();
        assert_eq!(acx.state, AcxState::Monitoring);
        assert!(acx.is_active());
    }

    // -----------------------------------------------------------------------
    // request_stop 测试
    // -----------------------------------------------------------------------

    /// 测试 request_stop 停止 ACX
    #[test]
    fn test_request_stop() {
        let config = test_config();
        let mut acx = AutoContinueManager::new(config).unwrap();

        acx.on_turn_completed(TurnOutcome::Completed);
        acx.request_stop();

        assert!(!acx.is_active());
        assert!(acx.take_pending_delay().is_none());
        assert_eq!(acx.state, AcxState::Stopped);
    }

    /// 测试停止后不再响应 on_turn_completed
    #[test]
    fn test_stopped_ignores_turn() {
        let config = test_config();
        let mut acx = AutoContinueManager::new(config).unwrap();

        acx.request_stop();
        acx.on_turn_completed(TurnOutcome::Completed);

        assert!(!acx.is_active());
        assert!(acx.take_pending_delay().is_none());
    }

    // -----------------------------------------------------------------------
    // on_timer_fired_sync 测试
    // -----------------------------------------------------------------------

    /// 测试 on_timer_fired_sync 更新状态到 Monitoring
    #[test]
    fn test_on_timer_fired_sync() {
        let config = AcxConfig {
            prompt: PromptSource::Static("test prompt".to_string()),
            delay_seconds: 0,
            ..test_config()
        };
        let mut acx = AutoContinueManager::new(config).unwrap();

        acx.on_turn_completed(TurnOutcome::Completed);
        assert!(acx.is_waiting_to_send());

        // 消费延迟值
        let delay = acx.take_pending_delay();
        assert!(delay.is_some());

        // 模拟计时器触发
        acx.on_timer_fired_sync();

        // 状态应回到 Monitoring
        assert_eq!(acx.state, AcxState::Monitoring);
        assert!(!acx.is_waiting_to_send());
    }

    // -----------------------------------------------------------------------
    // status_text 测试
    // -----------------------------------------------------------------------

    /// 测试 status_text 在各状态下的输出
    #[test]
    fn test_status_text() {
        let config = test_config();
        let mut acx = AutoContinueManager::new(config).unwrap();

        // Monitoring 状态
        let text = acx.status_text();
        assert!(text.is_some());
        assert!(text.as_ref().unwrap().contains("监控中"));

        // WaitingToSend 状态
        acx.on_turn_completed(TurnOutcome::Completed);
        let text = acx.status_text();
        assert!(text.is_some());
        assert!(text.as_ref().unwrap().contains("自动继续"));

        // Stopped 状态
        acx.request_stop();
        assert!(acx.status_text().is_none());
    }

    // -----------------------------------------------------------------------
    // 钩子集成测试
    // -----------------------------------------------------------------------

    /// 测试钩子触发停止
    #[test]
    fn test_hook_triggers_stop() {
        let config = AcxConfig {
            stop_whens: vec!["<round=2>".to_string()],
            ..test_config()
        };
        let mut acx = AutoContinueManager::new(config).unwrap();

        // 第 1 轮：继续（round_count=0 时检查，RoundHook(2) 通过）
        acx.on_turn_completed(TurnOutcome::Completed);
        assert!(acx.is_active());

        // 第 2 轮：继续（round_count=1 时检查）
        acx.on_turn_completed(TurnOutcome::Completed);
        assert!(acx.is_active());

        // 第 3 轮：停止（round_count=2 时检查，>=2 触发 Stop）
        acx.on_turn_completed(TurnOutcome::Completed);
        assert!(!acx.is_active());
    }

    // -----------------------------------------------------------------------
    // 禁用模式测试
    // -----------------------------------------------------------------------

    /// 测试 enabled=false 时不启动计时器
    #[test]
    fn test_disabled_mode() {
        let config = AcxConfig {
            enabled: false,
            ..test_config()
        };
        let mut acx = AutoContinueManager::new(config).unwrap();

        acx.on_turn_completed(TurnOutcome::Completed);
        assert!(acx.take_pending_delay().is_none());
        assert_eq!(acx.state, AcxState::Monitoring);
    }
}
