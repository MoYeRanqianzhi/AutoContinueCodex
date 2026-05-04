# ACX — AutoContinue + Codex

<p align="center"><code>npm i -g @moyeranqianzhi/acx</code></p>
<p align="center"><strong>ACX</strong> 是 <a href="https://github.com/openai/codex">OpenAI Codex CLI</a> 的增强版本，内嵌了 <a href="https://github.com/MoYeRanqianzhi/AutoContinue">AutoContinue</a> 自动继续/重试功能。</p>
<p align="center"><a href="README.md">English</a></p>

---

## 什么是 ACX？

ACX = **A**uto**C**ontinue + Code**X**

它在 [OpenAI Codex CLI](https://github.com/openai/codex) 的基础上，内嵌了 [AutoContinue](https://github.com/MoYeRanqianzhi/AutoContinue) 的核心逻辑，实现：

- **自动继续**：Turn 完成后自动发送继续提示词，无需人工干预
- **智能重试**：遇到可重试错误（速率限制、服务器过载、连接失败等）时，指数退避自动重试，上限 5 分钟
- **不可重试错误自动停止**：上下文超限、认证失败等永久性错误不会浪费重试
- **可扩展的停止钩子**：支持按轮次、时间、持续时长、自定义命令等条件自动停止
- **`/acx-stop` 命令**：随时手动停止自动继续

所有 Codex CLI 原有功能完整保留。

## 安装

```shell
npm install -g @moyeranqianzhi/acx
```

安装后命令行工具名称仍为 `codex`。

## 使用

```shell
# 启用自动继续模式
codex --acx "你的任务描述"

# 自定义继续提示词和延迟
codex --acx --acx-cp "继续迭代" --acx-delay 10 "重构项目"

# 带停止条件
codex --acx --acx-sw "<round=5>" --acx-sw "<duration=3600>" "优化代码库"

# 运行中手动停止：在 TUI 输入 /acx-stop
```

### ACX 参数

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `--acx` | 启用自动继续模式 | `false` |
| `--acx-cp` | 继续提示词 | `"Continue"` |
| `--acx-cpio` | 继续提示词 IO 文件（每次重新读取） | — |
| `--acx-cpp` | 继续提示词管道命令 | — |
| `--acx-delay` | 基础等待秒数（错误时指数退避） | `15` |
| `--acx-sw` | 预设停止条件（可重复） | — |
| `--acx-sh` | 自定义停止命令钩子（可重复） | — |

### 停止条件示例

```shell
--acx-sw "<round=10>"       # 10 轮后停止
--acx-sw "<error>"           # 任何错误立即停止（不重试）
--acx-sw "<duration=3600>"   # 运行 1 小时后停止
--acx-sw "<time=2026-01-01T08:00:00>"  # 到指定时间停止
```

## 支持平台

| 平台 | 架构 | 状态 |
|------|------|------|
| Linux | x86_64, ARM64 | ✅ |
| macOS | ARM64 (Apple Silicon) | ✅ |
| Windows | x86_64, ARM64 | ✅ |

## 与上游同步

ACX 通过 GitHub Actions 每小时自动同步 [openai/codex](https://github.com/openai/codex) 上游更新。ACX 的修改采用最小插入原则（`// [ACX]` 标记），核心逻辑封装在独立 crate `codex-auto-continue` 中，降低合并冲突风险。

## 致谢

- [OpenAI Codex CLI](https://github.com/openai/codex) — 底层 CLI 框架，Apache-2.0 许可
- [AutoContinue](https://github.com/MoYeRanqianzhi/AutoContinue) — 自动继续/重试的核心逻辑来源

## 许可

本项目基于 [Apache-2.0 License](LICENSE) 许可，与上游 Codex 保持一致。

## 相关文档

- [Codex 官方文档](https://developers.openai.com/codex)
- [AutoContinue 项目](https://github.com/MoYeRanqianzhi/AutoContinue)
- [Contributing](./docs/contributing.md)
- [Installing & building](./docs/install.md)
