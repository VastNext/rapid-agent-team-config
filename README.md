# Rapid Agent Team Configurator

跨平台、轻量级、无 Node.js 运行时依赖的 **Rapid Agent Team** 桌面图形化配置与安装向导工具（基于 Rust + wry/tao + 原生前端技术构建）。当前修复阶段的 Release 暂只发布 Windows x64，待功能稳定后恢复多平台发布。

> Windows 版本当前未配置 Authenticode 代码签名证书，首次下载运行时可能出现 SmartScreen“未知发布者”提示。请从本仓库 Release 下载并核对同目录 `.sha256` 文件；这不是应用运行时读取凭据或执行不受信任脚本造成的提示。后续配置签名证书后再消除该系统提示。

---

## 🌟 核心功能

1. **环境与配置跨平台自动探测**：
   - 自动扫描全局 OpenCode 配置目录（`~/.config/opencode` 或 `%APPDATA%\opencode`）及当前项目目录（`.opencode`）。
   - 兼容解析标准 `opencode.json` 与带注释/尾随逗号的 `opencode.jsonc`。
   - **严格安全边界**：仅提取已定义的 Provider 及其 Models 标识（如 `openai/gpt-4o`、`zhipu/glm-4`），**绝对不读取、展示或篡改任何 API Key / 凭据**。

2. **Rapid Dev Team 9 成员全景状态监控**：
   - 队长/总调度：`rapid-dev-team`
   - 侦察兵：`rapid-scout`
   - 智谱构建者：`rapid-builder-glm-zhipu`
   - GLM Coding 构建者：`rapid-builder-glm-go`
   - DeepSeek 构建者：`rapid-builder-deepseek-go`
   - 商汤日日新构建者：`rapid-builder-deepseek-sensenova`
   - 前端与交互：`rapid-ui`
   - 质量评审员：`rapid-reviewer`
   - 方案架构师：`rapid-architect`
   - 完整展示文件路径、来源（全局/项目）、当前挂载模型及角色模式（primary/subagent）。

3. **可视化模型搜索与分配**：
   - 支持多 Provider 分组折叠与实时模型搜索过滤。
   - 提供「智能预设匹配」一键根据角色专长分配最适模型。

4. **事务化安全写入与 Diff 预览**：
   - 生成变更前详细字段级 Diff 比较列表。
   - **自动备份**：写入前自动备份待修改文件至 `.backups/rapid-team-<timestamp>/`。
   - **并发防护**：文件修改时间戳（mtime）冲突校验。
   - **无损替换**：仅精准修改 Frontmatter 的 `model:` 字段，100% 保持 Prompt 正文、注释和换行符（CRLF/LF）字节一致。
   - **原子写入与回滚**：临时文件写入 + 原子重命名，遇异常立即全量回滚。

5. **未安装/缺失成员安装向导**：
   - 支持从本地 ZIP 安装包解压（内置严格 **Zip Slip** 目录穿越防护与 manifest 校验）。
   - 支持从本地源码文件夹一键同步复制。
   - 支持在线查询 GitHub Releases 固定版本、代理配置/测试、直链解析与下载。

---

## 🚀 编译与本地开发

### 依赖项
- [Rust 1.75+](https://rustup.rs/)
- Windows: 默认自带 WebView2 运行时；若使用 MSVC target 需要 C++ Build Tools。
- Linux: `libwebkit2gtk-4.1-dev` `libgtk-3-dev`

### 常用命令

```bash
# 格式化检查
cargo fmt -- --check

# 代码检查
cargo check

# 运行集成与单元测试
cargo test

# 本地调试运行
cargo run

# 构建发布单文件可执行程序
cargo build --release
```

---

## 🔒 安全说明 (Zero Credential Guarantee)

本软件遵循极简零信任安全设计：
1. 不会发起除用户显式触发的 GitHub 版本查询外的任何未经许可外网请求。
2. 不会读取 OpenCode 配置文件中的任何 `apiKey`、`token`、`secret`、`password` 等敏感键。
3. 应用内代理设置仅作用于内部局部下载器客户端，不修改系统环境变量。
4. 解压逻辑对 ZIP 包内每一个条目进行路径规范化防穿透检测。

---

## 📄 开源许可证

MIT License (c) 2026 VastNext
