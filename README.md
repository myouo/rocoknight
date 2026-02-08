# RocoKnight

RocoKnight 是一个基于 Tauri v2 + WebView2 的登录启动器：先在内置 WebView 完成洛克王国登录，然后从 `login3` 响应中解析 `flashVars`，拼接 `main.swf` 启动 URL，最后在同一主窗口内嵌 Flash Projector 运行游戏。

## 依赖

- Node.js 18+
- Rust 1.74+
- Windows 10/11（需要 WebView2）

## 目录结构

- `static/` 轻量前端静态页（用于主窗口占位，不依赖 React）
- `src-tauri/` Rust + Tauri v2 后端
- `resources/` 放置 `projector.exe`

## 快速开始

1. 安装依赖

```bash
npm install
```

2. 开发模式启动

```bash
npm run dev
```

3. 打包构建

```bash
npm run build
```

## 核心流程

- 主窗口创建时加载登录页：`https://17roco.qq.com/login.html`
- 启动内置 `projector.exe` 并将其窗口嵌入主窗口（Win32 attach）
- 隐藏登录 WebView，进入运行状态

## 安全与日志

- `flashVars` 与 URL 含敏感 token：严禁落盘、严禁写日志
- 仅输出脱敏信息（参数名列表等）
- 如需调试完整 `login3` 响应，可设置环境变量 `ROCO_DEBUG_DUMP_LOGIN3=1`（不建议日常使用）

## 典型问题

- 黑屏但有声音：通常是 WebView 覆盖了 projector。当前已在嵌入后隐藏 WebView 并提升 projector 窗口层级。

