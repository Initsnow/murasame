# Murasame

基于 Rust、Live2D 与 Slint 构建的轻量桌面 AI 伴侣。

## 特性

- **Live2D 渲染**：基于 WGPU 硬件加速与 Live2D Native SDK，支持视线追踪、动作过渡与全屏避让。
- **AI Agent 架构**：支持 OpenAI 与 Hermes 协议，集成多模态视觉（屏幕感知）、本地工具调用与 MCP 扩展。
- **语音与口型同步**：集成 Edge-TTS 与 Beatrice 声音转换模型，根据音频实时驱动角色口型。
- **轻量桌面交互**：基于 Slint 的透明无边框窗口、独立字幕与设置面板，低内存与低 GPU 开销。

## 编译与运行

### 环境要求
- Windows 10 / 11 x64
- Rust (2024 Edition / Stable) 与 Visual Studio MSVC 生成工具
- Git LFS

### 步骤
```powershell
# 克隆代码库（含子模块）
git clone --recursive https://github.com/Initsnow/murasame.git
cd murasame

# 拉取 LFS 权重资源
git lfs pull
git -C models/murasame-beatrice-v1 lfs pull

# 编译运行
cargo run --release
```

## 配置

- `config.toml`：全局窗口坐标、渲染帧率、视线追踪与托盘行为。
- `characters/<id>/character.toml`：角色 Live2D 模型、音色配置、AI 接口凭证及动作映射规则。

## 许可证

本项目采用 [GPL-3.0-or-later](LICENSE) 开源。第三方 SDK 与模型资源遵循其原有许可协议。
