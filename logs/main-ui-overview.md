主界面的组成和切换逻辑主要由 `src/App.tsx` 决定，样式由 `src/styles.css` 决定，入口挂载由 `src/main.tsx` 决定。具体对应关系如下：

1. 入口与挂载  
`src/main.tsx` 负责把 `<App />` 挂到 `#root`，并包了一层 `RootErrorBoundary`。这决定了“主界面”的根组件就是 `App`。

2. 顶部工具栏（Toolbar）  
`src/App.tsx` 里的 `<div className="app-toolbar">…</div>` 定义了“重新登录 / 更换频道 / 切换主题 / RocoKnight”这些按钮与布局。样式在 `src/styles.css` 的 `.app-toolbar`、`.app-toolbar__btn` 等。

3. 状态栏（Status Bar）  
`src/components/StatusBar.tsx` 定义 UI 结构、按钮与状态标签。  
`src/App.tsx` 中 `<StatusBar … />` 决定显示逻辑与事件绑定。  
样式主要使用 Tailwind class（如 `flex`, `border-b`, `bg-steel/70`）和 `clsx` 的动态类。

4. 主内容区（Login / Game 视图切换）  
切换逻辑在 `src/App.tsx`：  
`const isGameView = status === "Running" || status === "Launching" || status === "FoundValue";`  
然后根据 `isGameView` 渲染 `<GameView />` 或 `<LoginView />`。  
视图结构分别在：  
- `src/components/LoginView.tsx`  
- `src/components/GameView.tsx`  
它们内部结构相同（`.game-stage`、`.stage-grid`、`.stage-inner`、`.aspect-frame`），只是内容文字不同。  
布局/背景/边框由 `src/styles.css` 的 `.game-stage`、`.stage-grid`、`.aspect-frame` 决定。

5. 与 Tauri 交互影响界面状态  
`src/App.tsx` 中的 `listen("status_changed", ...)` 和 `invoke(...)` 会驱动 `status` 变化，从而影响主界面显示的视图与状态栏文案。  
这决定了“主界面现在处于登录态还是游戏态”。

如果你想更细地回答“主界面包含哪些功能区”，我可以进一步结合状态流和 Tauri 后端命令把每个区块的业务含义标出来。
