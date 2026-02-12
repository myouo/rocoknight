# 命令分析 - Top 5 高频按钮命令

## 从 toolbar.html 和代码推断的按钮映射

### 用户可见按钮（toolbar.html）
1. **重新登录** -> `reset_to_login`
2. **更换频道** -> `change_channel`
3. **调试窗口** -> `toggle_debug_window`
4. **切换主题** -> `set_theme_mode`

### Top 5 最可能被点击的 Commands（按概率排序）

1. **reset_to_login** (概率: 30%)
   - 用户最常用的功能
   - 涉及复杂的状态重置、窗口操作、日志记录
   - 有多个阶段（stop_projector, reset_state, show_login, navigate, emit_status）

2. **change_channel** (概率: 25%)
   - 用户频繁使用
   - 涉及 projector 重启、状态验证
   - 有多个阶段和日志记录

3. **set_theme_mode** (概率: 20%)
   - 简单但会触发 apply_theme_to_app
   - 会对所有 webview 执行 eval（可能阻塞）

4. **start_login3_capture** (概率: 15%)
   - 登录流程自动触发
   - 涉及 webview 操作和状态管理

5. **launch_projector** (概率: 10%)
   - 登录成功后触发
   - 涉及进程启动、窗口嵌入、复杂的错误处理

## 所有 Tauri Commands 列表

从 invoke_handler 提取：
- set_login_bounds
- show_login_webview
- hide_login_webview
- get_theme_mode
- set_theme_mode ✅
- start_login3_capture ✅
- stop_login3_capture
- launch_projector ✅
- resize_projector
- stop_projector
- restart_projector
- change_channel ✅
- reset_to_login ✅
- toggle_debug_window ✅
- debug_log
- get_debug_stats
- debug_get_recent_logs

## 预期的问题触发链路

### 场景 1: toggle_debug -> reset_to_login
1. 用户点击"调试窗口"
2. toggle_debug_window spawn 线程执行 window.show()
3. spawn 线程调用 debug_log_bus::set_window_open(true)
4. set_window_open 触发 tracing::info!
5. **用户立即点击"重新登录"**
6. reset_to_login 触发大量 tracing 日志（每个阶段都有）
7. **如果 spawn 线程还在执行，可能导致锁竞争或 emit 阻塞**

### 场景 2: toggle_debug -> change_channel
1. 用户点击"调试窗口"
2. spawn 线程执行中...
3. **用户立即点击"更换频道"**
4. change_channel 触发 tracing 日志
5. **阻塞在日志记录或 emit 操作**

### 场景 3: toggle_debug -> set_theme_mode
1. 用户点击"调试窗口"
2. spawn 线程执行中...
3. **用户立即点击"切换主题"**
4. set_theme_mode 调用 apply_theme_to_app
5. apply_theme_to_app 对所有 webview 执行 eval
6. **如果 debug window 正在 show/hide 中，eval 可能阻塞**

## 最可能的根因（按概率排序）

### A) spawn 线程中的 window 操作阻塞主线程 (40%)
- window.show()/hide() 在 spawn 线程中执行
- 这些操作可能需要主线程配合
- 如果主线程正在执行其他 command，形成死锁

### B) debug_log_bus 锁竞争 (30%)
- spawn 线程调用 set_window_open -> tracing::info! -> push_log -> bus.lock()
- 其他 command 也触发 tracing -> push_log -> bus.lock()
- 锁竞争导致阻塞

### C) emit 操作阻塞 (20%)
- flush_loop 正在 emit_batch
- emit 等待前端响应
- 其他 command 的日志无法进入队列

### D) window.eval 阻塞 (5%)
- apply_theme_to_app 调用 webview.eval
- 如果 debug window 正在操作中，eval 可能阻塞

### E) panic 被吞掉 (5%)
- spawn 线程 panic 但没有被记录
- 导致状态异常
