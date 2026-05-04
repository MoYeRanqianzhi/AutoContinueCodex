// [ACX]
//! Codex → ACX 类型转换桥
//!
//! 将 Codex 的 `Turn` / `TurnStatus` / `TurnError` 转换为
//! `codex_auto_continue` crate 使用的 `TurnOutcome` / `AcxErrorKind`。

use codex_app_server_protocol::CodexErrorInfo;
use codex_app_server_protocol::Turn;
use codex_app_server_protocol::TurnStatus;
use codex_auto_continue::AcxErrorKind;
use codex_auto_continue::TurnOutcome;

/// 将 Codex 的 `Turn` 转换为 ACX 的 `TurnOutcome`，
/// 用于让 AutoContinueManager 决定是否自动继续/重试。
pub(crate) fn turn_to_outcome(turn: &Turn) -> TurnOutcome {
    match turn.status {
        // Turn 正常完成
        TurnStatus::Completed => TurnOutcome::Completed,

        // Turn 仍在进行中（不应在 TurnCompleted 通知中出现，按完成处理）
        TurnStatus::InProgress => TurnOutcome::Completed,

        // Turn 被用户中断
        TurnStatus::Interrupted => TurnOutcome::Interrupted,

        // Turn 失败，需要分类错误类型
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

/// 将 Codex 的 `CodexErrorInfo` 映射为 ACX 的 `AcxErrorKind`。
///
/// 映射逻辑：
/// - UsageLimitExceeded → RateLimit（可重试）
/// - ServerOverloaded → ServerOverloaded（可重试）
/// - InternalServerError → InternalServer（可重试）
/// - HttpConnectionFailed / ResponseStreamConnectionFailed / ResponseStreamDisconnected → ConnectionFailed（可重试）
/// - ResponseTooManyFailedAttempts → TooManyAttempts（可重试）
/// - ContextWindowExceeded → ContextExceeded（停止）
/// - Unauthorized → Unauthorized（停止）
/// - 其余 → Other（停止）
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
