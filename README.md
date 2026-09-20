# PixLens · 图镜

> 免费、无广告、纯本地的 Windows 桌面看图软件 —— 轻快如镜，纤毫毕现。

- **定位**：对标收费/广告泛滥的看图工具，做免费、轻快、功能实在的看图软件
- **特色**：万张图库流畅浏览、超大图不卡、原生支持 **PSD / PSB** 设计源文件
- **平台**：Windows 10 / 11 x64
- **技术栈**：Tauri 2（Rust 核心 + WebView2 前端，TypeScript + Vite）

## 当前状态

**v1 完成（2026-09-20）**：M1~M7 全部里程碑交付并通过验收（见 `doc/验收记录.md`）——
缩略图墙、Canvas 大图查看器、PSD/PSB 自研解析器、多页 TIFF、HDR 曝光、批量处理、基础编辑、
NSIS 安装器（2.55MB）+ 文件关联 ×12 + 设置页。P1~P8/P11/P12 达标；P9/P10 应用进程达标（WebView2 运行时全树口径架构性超出，根因已归档）。

## 构建与发布

```bash
pnpm install                 # 前端依赖
pnpm tauri dev               # 开发运行
pnpm tauri build             # 产出 NSIS 安装器 target/release/bundle/nsis/
cargo test -p pixlens        # 14 项单元测试
```

验收基准：`PIXLENS_BENCH=<图库> [PIXLENS_BENCH_CLEAR=1] [PIXLENS_BENCH_FULL=1] pixlens.exe`，
测试图库由 `tests/gen-lib`（LIB-S/M/XL/TIFF）与 `tests/gen-psd`（LIB-PSB，含 2GB PSB）生成。

## 文档索引

| 文档 | 内容 |
|---|---|
| [doc/01-需求定义.md](doc/01-需求定义.md) | 定位与原则、功能清单、格式支持矩阵、决策记录 |
| [doc/02-技术选型.md](doc/02-技术选型.md) | 三方案对比、Tauri 决定与理由、依赖清单、**开工前置检查清单** |
| [doc/03-架构设计.md](doc/03-架构设计.md) | 分层架构、模块设计、**PSD/PSB 解析器详设**、协议与缓存设计 |
| [doc/04-性能要求.md](doc/04-性能要求.md) | 性能指标验收表（含测量方法）、基准测试方案 |
| [doc/05-里程碑计划.md](doc/05-里程碑计划.md) | M1~M7 分期计划、每期交付物与验收标准 |

## 后续开发（新会话指引）

在 **D:\work\pixlens** 目录打开 ZCode 会话，建议流程：

1. 通读 `doc/` 全部文档与 `doc/验收记录.md`
2. backlog 项见 `doc/01-需求定义.md` §8（EXIF 面板、RAW、JPEG 无损旋转、双图对比、ICC、跨平台）
3. 每次改动后跑 `cargo test -p pixlens` 与相关 P 指标基准，结果续写 `doc/验收记录.md`

## 关键决策速览

| 决策项 | 结论 |
|---|---|
| 平台 | Windows 10/11 x64 桌面端 |
| 技术栈 | Tauri 2（Rust + WebView2），前端 TS 无框架 |
| 性能目标 | 万张流畅 + 超大图不卡 + 冷启动 < 1s |
| 特色格式 | PSD / PSB（自研 Rust 解析器） |
| 格式范围 | 常见格式 + 多页 TIFF / HDR（RAW 进 backlog） |
| v1 功能 | 缩略图浏览 + 极速大图浏览 + 批量处理 + 基础编辑 |
| 商业模式 | 完全免费、无广告、无内购、不联网 |
