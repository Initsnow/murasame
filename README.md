# Murasame (丛雨)

[![License: GPL-3.0](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows-0078D6?logo=windows)](https://github.com/Initsnow/murasame)
[![Rust](https://img.shields.io/badge/Rust-2024_Edition-DEA584?logo=rust)](https://www.rust-lang.org/)

**Murasame** 是一个基于 Rust、Live2D 与 Slint UI 构建的高性能、低开销 AI 桌面宠物伴侣。

---

## ✨ 核心特性

- 🌸 **Live2D 原生高性能渲染**  
  基于官方 Live2D Cubism SDK Native (v5-r.5) 与 WGPU 硬件加速管线，支持分层动作混合（Layered Idle Blending）、呼吸微动、动作平滑淡入淡出与可配置超采样抗锯齿（SSAA）。
- 💬 **智能 AI 对话与记忆**  
  支持 OpenAI 兼容接口与 Hermes 协议的流式响应，内置自适应上下文截断与 SQLite 本地聊天记录持久化。
- 🎙️ **个性化语音与口型同步 (Lip-sync)**  
  集成 Edge-TTS 语音合成与本地 Beatrice 声音转换模型，支持基于音频振幅的实时口型驱动。
- 🎭 **情感表达与动作联动 (Cue Mappings)**  
  支持从模型回复中捕获 Cue 动作标签，触发多候选随机动作与表情切换。
- 🖥️ **现代轻量桌面交互**  
  基于 Slint 打造透明无边框窗口、置顶显示、字幕气泡与浮动工具栏、系统托盘常驻及可视化的设置面板。

---

## 🛠️ 技术栈

| 模块 | 技术选型 |
| :--- | :--- |
| **语言与生态** | Rust (2024 Edition), Tokio 异步运行时 |
| **GUI 框架** | [Slint UI](https://slint.dev/) (Femtovg-WGPU 渲染后端) |
| **Live2D 引擎** | Live2D Cubism SDK for Native v5-r.5, WGPU 30 |
| **音频与语音** | Rodio, Symphonia, Edge-TTS, Beatrice (VC 声音转换) |
| **数据与网络** | Reqwest, Async-OpenAI, SQLite (Rusqlite Bundled) |

---

## 🚀 快速开始

### 依赖环境
- **操作系统**: Windows 10/11 x64
- **编译工具**: Rust (Stable) 与 Visual Studio C++ 生成工具 (MSVC 2022)
- **Git LFS**: 用于拉取预训练模型权重

### 获取源码与编译
```powershell
# 1. 克隆代码库（包含子模块）
git clone --recursive https://github.com/Initsnow/murasame.git
cd murasame

# 2. 拉取 Git LFS 大文件资源（模型权重）
git lfs pull
git -C models/murasame-beatrice-v1 lfs pull

# 3. 运行项目
cargo run --release
```

### 辅助调试工具
项目内置多款辅助二进制工具：
- `cargo run --bin voice-tune`: 实时测试并微调 TTS / Beatrice 声音音高与共振峰参数。
- `cargo run --bin live2d-bench`: 评估 Live2D 变形计算与 GPU 渲染延迟性能基准。
- `cargo run --bin capture-motions`: 分析与捕捉模型动作轨迹参数。

---

## ⚙️ 配置说明

- `config.toml`: 全局窗口位置、尺寸、字幕样式与界面托盘配置。
- `characters/<character_id>/character.toml`: 角色专属配置，包括 Live2D 模型路径、TTS/VC 音色参数、AI API 凭证及 Cue 动作映射规则。

---

## 📄 开源许可证

本项目采用 [GNU General Public License v3.0](LICENSE) 开源。
包含的第三方 SDK 与模型资源遵循其各自的授权许可（如 Live2D Proprietary EULA）。
