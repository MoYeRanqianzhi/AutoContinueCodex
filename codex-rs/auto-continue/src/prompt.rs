//! # 提示词解析模块 (prompt.rs)
//!
//! 定义 `PromptSource` 枚举及其 `resolve()` 异步方法，
//! 负责从不同来源获取提示词内容。
//!
//! ## 支持的模式
//! - `Static`: 固定文本，直接返回
//! - `File`: 启动时一次性读取文件内容
//! - `Io`: 每次调用时重新读取文件（动态）
//! - `Pipe`: 每次调用时执行命令获取输出（支持 format 标签提取）

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

/// 提示词来源枚举
///
/// 描述提示词内容的获取方式。每种变体对应一种加载策略，
/// 通过 `resolve()` 异步方法统一获取最终的提示词文本。
#[derive(Debug, Clone)]
pub enum PromptSource {
    /// 固定文本提示词
    ///
    /// 内容在构造时确定，`resolve()` 直接返回。
    Static(String),

    /// 文件提示词（启动时一次性读取）
    ///
    /// 在构造 `AcxConfig` 时读取文件内容并存入 `Static`，
    /// 但此变体保留用于语义明确性。
    /// `resolve()` 时读取文件内容。
    File(PathBuf),

    /// IO 文件提示词（每次动态读取）
    ///
    /// 每次 `resolve()` 调用都会重新读取文件内容，
    /// 适用于提示词需要随时间变化的场景。
    Io(PathBuf),

    /// 管道命令提示词（每次执行命令获取）
    ///
    /// 每次 `resolve()` 调用都会执行指定的 shell 命令，
    /// 取 stdout 输出作为提示词。可选的 `format` 字段用于
    /// 从输出中提取特定标签包裹的内容。
    Pipe {
        /// 要执行的 shell 命令字符串
        command: String,
        /// 可选的格式提取标签对 (前缀, 后缀)
        ///
        /// 设置后，从命令输出中查找最后一组 `前缀...后缀` 匹配，
        /// 提取中间内容作为提示词。
        format: Option<(String, String)>,
    },
}

impl Default for PromptSource {
    /// 默认提示词为静态文本 "Continue"
    fn default() -> Self {
        PromptSource::Static("Continue".to_string())
    }
}

impl PromptSource {
    /// 解析提示词，返回最终的提示词文本
    ///
    /// 根据 `PromptSource` 的变体，采用不同的策略获取提示词内容：
    /// - `Static`: 直接返回存储的文本
    /// - `File`: 异步读取文件内容
    /// - `Io`: 异步读取文件内容（每次都重新读取）
    /// - `Pipe`: 异步执行命令，取 stdout 输出，可选 format 标签提取
    ///
    /// # 返回值
    /// 成功返回提示词文本（已 trim），失败返回错误
    ///
    /// # 错误
    /// - 文件读取失败（路径不存在、权限不足等）
    /// - 管道命令执行失败（命令不存在、超时、非零退出码等）
    /// - 管道命令输出为空
    pub async fn resolve(&self) -> Result<String> {
        match self {
            // 静态文本：直接返回克隆
            PromptSource::Static(text) => Ok(text.clone()),

            // 文件模式：异步读取文件内容
            PromptSource::File(path) => {
                let content = tokio::fs::read_to_string(path)
                    .await
                    .with_context(|| format!("读取提示词文件失败: {}", path.display()))?;
                // 标准化换行符并去除首尾空白
                Ok(normalize_and_trim(&content))
            }

            // IO 模式：每次重新异步读取文件内容
            PromptSource::Io(path) => {
                let content = tokio::fs::read_to_string(path)
                    .await
                    .with_context(|| format!("读取提示词IO文件失败: {}", path.display()))?;
                Ok(normalize_and_trim(&content))
            }

            // Pipe 模式：执行命令获取提示词
            PromptSource::Pipe { command, format } => {
                let output = execute_pipe_command(command).await?;

                // 如果配置了格式提取标签，尝试从输出中提取
                if let Some((prefix, suffix)) = format {
                    if let Some(extracted) = extract_format(&output, prefix, suffix) {
                        return Ok(extracted);
                    }
                    tracing::warn!(
                        "管道输出中未找到格式标签 {prefix}...{suffix}，使用完整输出"
                    );
                }

                Ok(output)
            }
        }
    }
}

/// 标准化换行符并去除首尾空白
///
/// 将 Windows 换行符 `\r\n` 和旧 Mac 换行符 `\r` 统一转换为 Unix 换行符 `\n`，
/// 然后去除首尾空白字符。
///
/// # 参数
/// - `content`: 原始文件内容
///
/// # 返回值
/// 标准化后的文本
fn normalize_and_trim(content: &str) -> String {
    content
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string()
}

/// 异步执行管道命令并返回 stdout 输出
///
/// 使用 `tokio::process::Command` 异步执行命令，30 秒超时。
///
/// ## 平台差异
/// - Windows: 使用解析后的 args 直接执行（避免 cmd.exe shell 注入）
/// - Unix: 使用解析后的 args 直接执行
///
/// # 参数
/// - `command`: 要执行的命令字符串（支持 shell 风格引号语法）
///
/// # 返回值
/// 成功返回命令输出（已 trim），失败返回错误
///
/// # 错误
/// - 命令字符串解析失败
/// - 命令启动失败
/// - 命令执行超时（30 秒）
/// - 命令返回非零退出码
/// - 命令输出为空
async fn execute_pipe_command(command: &str) -> Result<String> {
    // 解析命令字符串
    let args = parse_command(command)
        .with_context(|| format!("解析管道命令失败: {command}"))?;

    tracing::debug!("执行管道命令: {} (参数: {:?})", args[0], &args[1..]);

    // 构建 tokio::process::Command
    let mut cmd = Command::new(&args[0]);
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // Windows 平台：设置 CREATE_NO_WINDOW 标志
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }

    // 使用 tokio::time::timeout 实现 30 秒超时
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        cmd.output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("管道命令执行超时（30秒）: {command}"))?
    .with_context(|| format!("无法执行管道命令: {command}"))?;

    // 检查退出状态
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output.status.code().unwrap_or(-1);
        bail!("管道命令退出码 {code}，stderr: {}", stderr.trim());
    }

    // 解码 stdout
    let result = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if result.is_empty() {
        bail!("管道命令输出为空");
    }

    Ok(result)
}

/// 从文本中提取最后一组前缀后缀包裹的内容
///
/// 在输出中查找最后一个 `prefix...suffix` 模式，返回中间内容（已 trim）。
/// 用于从管道命令输出中提取特定标签包裹的提示词，过滤多余文本。
///
/// # 参数
/// - `output`: 完整输出文本
/// - `prefix`: 前缀标签（如 `<continue>`）
/// - `suffix`: 后缀标签（如 `</continue>`）
///
/// # 返回值
/// 找到匹配返回 `Some(内容)`（已 trim），未找到返回 `None`
pub fn extract_format(output: &str, prefix: &str, suffix: &str) -> Option<String> {
    // 从后往前查找最后一个 prefix
    let prefix_pos = output.rfind(prefix)?;
    let content_start = prefix_pos + prefix.len();
    let remaining = &output[content_start..];
    // 从 prefix 之后查找第一个 suffix
    let suffix_pos = remaining.find(suffix)?;
    let content = &remaining[..suffix_pos];
    Some(content.trim().to_string())
}

/// 解析命令字符串为程序名和参数列表
///
/// 支持 shell 风格的引号语法：
/// - 单引号 `'...'`：内容原样保留
/// - 双引号 `"..."`：支持 `\"` 和 `\\` 转义
/// - 反斜杠：转义下一个字符
/// - 空白分隔参数
///
/// # 参数
/// - `cmd`: 命令字符串
///
/// # 返回值
/// 成功返回参数列表（第一个元素为程序名），失败返回错误
///
/// # 错误
/// - 未闭合的引号
/// - 空命令
pub fn parse_command(cmd: &str) -> Result<Vec<String>> {
    let mut args: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = cmd.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some(c) = chars.next() {
        if in_single_quote {
            // 单引号内：除了闭合引号，一切原样保留
            if c == '\'' {
                in_single_quote = false;
            } else {
                current.push(c);
            }
        } else if in_double_quote {
            // 双引号内：支持 \" 和 \\ 转义
            if c == '"' {
                in_double_quote = false;
            } else if c == '\\' {
                if let Some(&next) = chars.peek() {
                    if next == '"' || next == '\\' {
                        current.push(chars.next().unwrap_or('\\'));
                    } else {
                        current.push(c);
                    }
                } else {
                    current.push(c);
                }
            } else {
                current.push(c);
            }
        } else {
            // 引号外
            match c {
                '\'' => in_single_quote = true,
                '"' => in_double_quote = true,
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                c if c.is_whitespace() => {
                    if !current.is_empty() {
                        args.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(c),
            }
        }
    }

    // 最后一个参数
    if !current.is_empty() {
        args.push(current);
    }

    if in_single_quote || in_double_quote {
        bail!("未闭合的引号");
    }

    if args.is_empty() {
        bail!("空命令");
    }

    Ok(args)
}

// ===========================================================================
// 单元测试
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // PromptSource 默认值测试
    // -----------------------------------------------------------------------

    /// 测试 PromptSource 默认值为 Static("Continue")
    #[test]
    fn test_prompt_source_default() {
        let source = PromptSource::default();
        match source {
            PromptSource::Static(text) => assert_eq!(text, "Continue"),
            _ => panic!("默认 PromptSource 应为 Static"),
        }
    }

    // -----------------------------------------------------------------------
    // PromptSource::Static resolve 测试
    // -----------------------------------------------------------------------

    /// 测试 Static 模式直接返回文本
    #[tokio::test]
    async fn test_static_resolve() {
        let source = PromptSource::Static("hello".to_string());
        let result = source.resolve().await.unwrap();
        assert_eq!(result, "hello");
    }

    // -----------------------------------------------------------------------
    // PromptSource::File resolve 测试
    // -----------------------------------------------------------------------

    /// 测试 File 模式读取文件内容
    #[tokio::test]
    async fn test_file_resolve() {
        // 创建临时文件
        let dir = std::env::temp_dir().join("acx_test_file_resolve");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("prompt.txt");
        std::fs::write(&file_path, "  test prompt  \r\n").unwrap();

        let source = PromptSource::File(file_path.clone());
        let result = source.resolve().await.unwrap();
        assert_eq!(result, "test prompt");

        // 清理
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // extract_format 测试
    // -----------------------------------------------------------------------

    /// 测试基本提取
    #[test]
    fn test_extract_format_basic() {
        let output = "some text <continue>real prompt</continue> more text";
        let result = extract_format(output, "<continue>", "</continue>");
        assert_eq!(result, Some("real prompt".to_string()));
    }

    /// 测试多组匹配取最后一组
    #[test]
    fn test_extract_format_last_match() {
        let output = "<c>first</c> middle <c>second</c> end";
        let result = extract_format(output, "<c>", "</c>");
        assert_eq!(result, Some("second".to_string()));
    }

    /// 测试无匹配返回 None
    #[test]
    fn test_extract_format_no_match() {
        let output = "no tags here";
        let result = extract_format(output, "<c>", "</c>");
        assert_eq!(result, None);
    }

    /// 测试只有前缀没有后缀
    #[test]
    fn test_extract_format_no_suffix() {
        let output = "text <c>content without closing";
        let result = extract_format(output, "<c>", "</c>");
        assert_eq!(result, None);
    }

    /// 测试多行内容
    #[test]
    fn test_extract_format_multiline() {
        let output = "header\n<continue>\nline1\nline2\n</continue>\nfooter";
        let result = extract_format(output, "<continue>", "</continue>");
        assert_eq!(result, Some("line1\nline2".to_string()));
    }

    // -----------------------------------------------------------------------
    // parse_command 测试
    // -----------------------------------------------------------------------

    /// 测试基本解析
    #[test]
    fn test_parse_command_basic() {
        let args = parse_command("echo hello").unwrap();
        assert_eq!(args, vec!["echo", "hello"]);
    }

    /// 测试单引号
    #[test]
    fn test_parse_command_single_quotes() {
        let args = parse_command("codex exec '输出<continue>继续</continue>'").unwrap();
        assert_eq!(args, vec!["codex", "exec", "输出<continue>继续</continue>"]);
    }

    /// 测试双引号
    #[test]
    fn test_parse_command_double_quotes() {
        let args = parse_command(r#"echo "hello world""#).unwrap();
        assert_eq!(args, vec!["echo", "hello world"]);
    }

    /// 测试混合引号和普通参数
    #[test]
    fn test_parse_command_mixed() {
        let args = parse_command("cmd arg1 'arg 2' arg3").unwrap();
        assert_eq!(args, vec!["cmd", "arg1", "arg 2", "arg3"]);
    }

    /// 测试空命令报错
    #[test]
    fn test_parse_command_empty() {
        assert!(parse_command("").is_err());
        assert!(parse_command("   ").is_err());
    }

    /// 测试未闭合引号报错
    #[test]
    fn test_parse_command_unclosed_quote() {
        assert!(parse_command("echo 'hello").is_err());
    }
}
