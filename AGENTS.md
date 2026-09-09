# Rapid Agent Team Configurator — 架构与开发规范

## 项目愿景与范围
本软件是独立的轻量跨平台桌面配置工具，负责在 OpenCode 体系下发现、安装并配置 Rapid Agent Team 的模型槽位及相关文件。

## 关键安全与设计准则

1. **绝对不碰凭据**：绝不读取、解析、缓存、展示或写入任何 API Key、Token、Secret、Password 或 Session Cookie。
2. **精准无损替换**：仅针对目标 Agent 的 Markdown Frontmatter `model:` 键进行精确修改，其余文本与换行符保持字节一致性，杜绝内存泄漏（避免使用 `Box::leak`）。
3. **Zip Slip 安全防护**：解压任何离线 ZIP 包必须对解压路径进行规范化校验，严防 `../` 路径穿越。
4. **事务与原子写入**：
   - 写入前自动将涉及到的全部目标文件备份到 `.backups/rapid-team-<timestamp>/`。
   - 检查预览生成后的文件最后修改时间戳（mtime），防止并发外部篡改覆盖。
   - 使用临时文件写入 + 原子重命名（rename）。
   - 发生任何写入异常立刻自动还原备份。
5. **代理隔离**：应用内设置的 HTTP/HTTPS/SOCKS5 代理仅作为在线下载客户端的局部连接参数，不污染全局环境变量。
6. **无 Node.js 运行时**：前端全部采用现代原生的 HTML5 / CSS3 / ES2022 JavaScript 构建，直接内嵌进 Rust 二进制中。
7. **自动化工作流与跨平台编译**：提供 GitHub Actions 覆盖 Windows/macOS/Linux 的多平台测试与 Release 自动构建流水线。
