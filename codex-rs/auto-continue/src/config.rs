//! # ACX 配置模块 (config.rs)
//!
//! 定义 AutoContinue 扩展 (ACX) 的运行时配置结构体 `AcxConfig`。
//! 该配置由上层调用者（如 Codex TUI）在启动时构造并传入 `AutoContinueManager`。

use crate::prompt::PromptSource;

/// ACX 运行时配置
///
/// 包含控制 AutoContinue 行为的所有参数。
/// 由上层调用者构造，传入 `AutoContinueManager::new()` 进行初始化。
///
/// ## 字段说明
/// - `enabled`: 是否启用 ACX 功能
/// - `prompt`: 提示词来源（静态文本 / 文件 / IO文件 / 管道命令）
/// - `delay_seconds`: 计时器触发前的延迟秒数（默认 15 秒）
/// - `stop_whens`: 预设中断条件规格字符串列表（如 `<round=5>`）
/// - `stop_hooks`: 自定义中断钩子命令列表
#[derive(Debug, Clone)]
pub struct AcxConfig {
    /// 是否启用 ACX 功能
    ///
    /// 当设为 `false` 时，`AutoContinueManager` 不会启动计时器或发送提示词。
    pub enabled: bool,

    /// 提示词来源
    ///
    /// 决定每次需要发送继续提示词时，从哪里获取提示词内容。
    /// 支持静态文本、文件、IO文件（动态读取）和管道命令四种模式。
    pub prompt: PromptSource,

    /// 计时器延迟秒数（默认 15 秒）
    ///
    /// Turn 完成后，等待此秒数再发送继续提示词。
    /// 给用户留出手动干预的时间窗口。
    pub delay_seconds: u64,

    /// 预设中断条件规格字符串列表
    ///
    /// 每个字符串会被 `parse_stop_when()` 解析为对应的 `StopHook` 实例。
    /// 支持的格式：`<round=N>`, `<error>`, `<time=DATETIME>`, `<duration=N>`
    pub stop_whens: Vec<String>,

    /// 自定义中断钩子命令列表
    ///
    /// 每个字符串是一个 shell 命令，会被 spawn 为长驻子进程。
    /// 子进程退出或输出 "0" 时触发停止。
    pub stop_hooks: Vec<String>,
}

impl Default for AcxConfig {
    /// 创建默认配置
    ///
    /// 默认值：
    /// - `enabled`: true（启用）
    /// - `prompt`: PromptSource::Static("Continue")
    /// - `delay_seconds`: 15
    /// - `stop_whens`: 空列表
    /// - `stop_hooks`: 空列表
    fn default() -> Self {
        Self {
            enabled: true,
            prompt: PromptSource::default(),
            delay_seconds: 15,
            stop_whens: Vec::new(),
            stop_hooks: Vec::new(),
        }
    }
}
