// [ACX]
//! ACX (AutoContinue) 与 Codex TUI 的桥接层
//!
//! 所有 ACX 逻辑集中在此文件。上游 Codex 文件中仅保留最少的
//! 单行调用，降低合并冲突风险。

use codex_app_server_protocol::CodexErrorInfo;
use codex_app_server_protocol::Turn;
use codex_app_server_protocol::TurnStatus;
use codex_auto_continue::{
    AcxAppEvent, AcxConfig, AcxErrorKind, AutoContinueManager, TurnOutcome,
};
use tokio::task::JoinHandle;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;

/// ACX 桥接层
///
/// 持有 AutoContinueManager 和计时器任务句柄。
/// 通过 tokio::spawn 在后台等待计时器，触发时通过 AppEvent 通道
/// 发送事件，避免修改 Codex 的 select! 循环。
pub(crate) struct AcxBridge {
    /// 核心状态机
    manager: AutoContinueManager,
    /// 事件发送通道（用于计时器触发后发送 AppEvent）
    app_event_tx: AppEventSender,
    /// 后台计时器任务句柄
    timer_handle: Option<JoinHandle<()>>,
}

impl AcxBridge {
    /// 创建新的 AcxBridge
    pub(crate) fn new(
        config: AcxConfig,
        app_event_tx: AppEventSender,
    ) -> anyhow::Result<Self> {
        let manager = AutoContinueManager::new(config)?;
        Ok(Self {
            manager,
            app_event_tx,
            timer_handle: None,
        })
    }

    /// Turn 完成时调用
    ///
    /// 将 Codex Turn 转为 ACX TurnOutcome，通知状态机，
    /// 如果需要继续则启动后台计时器任务。
    pub(crate) fn on_turn_completed(&mut self, turn: &Turn) {
        let outcome = turn_to_outcome(turn);
        self.manager.on_turn_completed(outcome);
        self.maybe_start_timer();
    }

    /// 取消正在等待的计时器
    pub(crate) fn cancel_pending(&mut self) {
        self.abort_timer();
        self.manager.cancel_pending();
    }

    /// 处理 AcxAppEvent，返回需要提交的提示词文本（如有）
    pub(crate) fn handle_event(&mut self, event: AcxAppEvent) -> AcxEventResult {
        match event {
            AcxAppEvent::TimerFired { text } => {
                // 检查状态：可能在 timer 等待期间被取消
                if self.manager.is_waiting_to_send() {
                    self.manager.on_timer_fired_sync();
                    AcxEventResult::SubmitContinue(text)
                } else {
                    AcxEventResult::None
                }
            }
            AcxAppEvent::StopRequested => {
                self.abort_timer();
                self.manager.request_stop();
                AcxEventResult::Stopped("自动继续已停止，当前轮完成后不再继续".to_string())
            }
        }
    }

    /// 如果状态机决定需要等待，启动后台计时器任务
    fn maybe_start_timer(&mut self) {
        self.abort_timer();
        if let Some(delay_secs) = self.manager.take_pending_delay() {
            let tx = self.app_event_tx.clone();
            let prompt = self.manager.config().prompt.clone();
            self.timer_handle = Some(tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                match prompt.resolve().await {
                    Ok(text) => {
                        tx.send(AppEvent::AcxEvent(AcxAppEvent::TimerFired { text }));
                    }
                    Err(e) => {
                        tracing::warn!("[ACX] 提示词解析失败: {e:#}");
                    }
                }
            }));
        }
    }

    /// 中止后台计时器任务
    fn abort_timer(&mut self) {
        if let Some(handle) = self.timer_handle.take() {
            handle.abort();
        }
    }
}

impl Drop for AcxBridge {
    fn drop(&mut self) {
        self.abort_timer();
    }
}

/// AcxBridge 处理事件的结果
pub(crate) enum AcxEventResult {
    /// 提交继续提示词
    SubmitContinue(String),
    /// ACX 已停止
    Stopped(String),
    /// 无操作
    None,
}

// ---------------------------------------------------------------------------
// 公开的分发函数（供 event_dispatch.rs 的单行委托调用）
// ---------------------------------------------------------------------------

/// 分发 ACX 事件
///
/// event_dispatch.rs 中只需一行：
/// `acx_integration::dispatch_acx_event(&mut self.acx_manager, &mut self.chat_widget, evt);`
pub(crate) fn dispatch_acx_event(
    bridge: &mut Option<AcxBridge>,
    chat_widget: &mut crate::chatwidget::ChatWidget,
    event: AcxAppEvent,
) {
    let Some(bridge) = bridge.as_mut() else {
        return;
    };
    match bridge.handle_event(event) {
        AcxEventResult::SubmitContinue(text) => {
            chat_widget.submit_user_message_as_plain_user_turn(text.into());
        }
        AcxEventResult::Stopped(reason) => {
            chat_widget.add_info_message(format!("[ACX] {reason}"), None);
        }
        AcxEventResult::None => {}
    }
}

// ---------------------------------------------------------------------------
// Codex → ACX 类型转换
// ---------------------------------------------------------------------------

/// 将 Codex 的 Turn 转换为 ACX 的 TurnOutcome
fn turn_to_outcome(turn: &Turn) -> TurnOutcome {
    match turn.status {
        TurnStatus::Completed => TurnOutcome::Completed,
        TurnStatus::InProgress => TurnOutcome::Completed,
        TurnStatus::Interrupted => TurnOutcome::Interrupted,
        TurnStatus::Failed => {
            let kind = turn
                .error
                .as_ref()
                .and_then(|err| err.codex_error_info.as_ref())
                .map(classify_error)
                .unwrap_or_else(|| AcxErrorKind::Other("unknown error".to_string()));
            TurnOutcome::Failed { kind }
        }
    }
}

/// 将 Codex 的 CodexErrorInfo 映射为 ACX 的 AcxErrorKind
fn classify_error(info: &CodexErrorInfo) -> AcxErrorKind {
    match info {
        CodexErrorInfo::UsageLimitExceeded => AcxErrorKind::RateLimit,
        CodexErrorInfo::ServerOverloaded => AcxErrorKind::ServerOverloaded,
        CodexErrorInfo::InternalServerError => AcxErrorKind::InternalServer,
        CodexErrorInfo::HttpConnectionFailed { .. }
        | CodexErrorInfo::ResponseStreamConnectionFailed { .. }
        | CodexErrorInfo::ResponseStreamDisconnected { .. } => AcxErrorKind::ConnectionFailed,
        CodexErrorInfo::ResponseTooManyFailedAttempts { .. } => AcxErrorKind::TooManyAttempts,
        CodexErrorInfo::ContextWindowExceeded => AcxErrorKind::ContextExceeded,
        CodexErrorInfo::Unauthorized => AcxErrorKind::Unauthorized,
        other => AcxErrorKind::Other(format!("{other:?}")),
    }
}
// [/ACX]
