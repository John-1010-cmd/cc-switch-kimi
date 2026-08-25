<div align="center">

# CC Switch Kimi

### Kimi Code / Claude Code / Codex 的供应商切换与用量监控工具

[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](../../releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)

[English](README.md) | 中文 | [Changelog](CHANGELOG.md)

</div>

[cc-switch](https://github.com/farion1231/cc-switch) 的个人 fork，围绕 **Kimi Code** 重建：将 Kimi Code 作为与 Claude Code、Codex 并列的一等 app 管理，把本地会话用量接入既有用量管线，并刻意裁剪为精简功能集。

## 核心能力

- **供应商管理** — Kimi Code（`~/.kimi-code/config.toml`）、Claude Code、Codex 的供应商一键新建/切换/删除。内置七家 Kimi Code 预设：Kimi For Coding、GLM、MiniMax、Claude、Codex、DeepSeek、小米 MiMo。CLI 与 VSCode 扩展共享配置目录，切换即时生效。
- **用量统计** — 增量解析 `~/.kimi-code/sessions/**/wire.jsonl`，把 Kimi Code 会话（CLI + VSCode 扩展）接入本地用量数据库：token 四分量（输入/输出/缓存读/缓存写）、首字延迟、流式时长、USD 成本，仪表盘准实时刷新。
- **实时托盘（macOS）** — 菜单栏标题常驻显示额度窗口与生成速度，如 `7D 42% · 5H 87% · 31 t/s`，会话进行中秒级刷新；轻量模式（仅托盘、无主窗口）下持续工作。
- **MCP 管理** — 统一的 MCP 服务器列表，同步进各 app 配置（`~/.kimi-code/mcp.json`、Claude、Codex）。
- **代理与故障转移** — 本地网关接管路由、多端点故障转移、全局代理支持。

## 相对上游的裁剪

对四大能力之外的模块做了物理删除（非功能开关）：gemini / grokbuild / opencode / openclaw / hermes / claudeDesktop 六个 app、会话浏览器、工作区、提示词与 Skills 管理、WebDAV/S3 备份、测速、深链、配置档案，以及上游的赞助与 R2 镜像发布基础设施。上游 `migration.rs` 未改动，已有 `~/.cc-switch` 配置目录与数据库可正常打开，被裁 app 的残留数据会被安全忽略。

## 构建与开发

环境要求：Node.js 18+、pnpm 8+、Rust 1.85+、Tauri CLI 2.8+。

```bash
pnpm install

pnpm dev                # 开发模式（热更新）
pnpm typecheck          # TS 类型检查
pnpm test:unit          # 前端测试（vitest）
pnpm build              # 通过 tauri 构建安装包

cd src-tauri
cargo test              # 后端测试
cargo clippy            # 静态检查
```

## 发布与自动更新

推送 `v*` 标签后由 `.github/workflows/release.yml` 构建并发布到本仓库的 GitHub Releases。应用内置更新检查指向本仓库 `releases/latest/download/latest.json`，并使用 `src-tauri/tauri.conf.json` 中烧录的 minisign 公钥校验产物。

签名私钥**不入库**：

- 私钥在本地用 `tauri signer generate` 生成，保存在仓库目录之外。
- 本地构建需要产出更新产物时，将环境变量指向私钥：

  ```bash
  export TAURI_SIGNING_PRIVATE_KEY=~/.tauri-keys/cc-switch-kimi.key
  pnpm build
  ```

- CI 发布需在本仓库 Settings → Secrets and variables → Actions 配置 `TAURI_SIGNING_PRIVATE_KEY`（可选 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）。

## 署名

Fork 自 [farion1231/cc-switch](https://github.com/farion1231/cc-switch)（v3.19.1）。原始设计与绝大部分代码的功劳属于上游作者与贡献者；本 fork 的改动（Kimi Code 集成、用量管线、预设、裁剪）见 `CHANGELOG.md`。

## 许可证

MIT — 见 [LICENSE](LICENSE)。保留上游项目的 MIT 许可证与署名义务。
