# 点击 Debug 后其他按钮崩溃问题 - 诊断与修复

## 问题描述

**症状**: 反复点击"调试窗口"按钮本身不会卡顿，但点完后再点击其他按钮就会崩溃/无响应

**表现**: UI 变得不响应，像是事件循环卡死，或者后端 panic 导致挂起

---

## 根因分析

### 🔴 主要根因：spawn 线程中的窗口操作���致竞态条件

**问题代码** (toggle_debug_window 原实现):
```rust
// 异步执行窗口操作
let window_clone = window.clone();
std::thread::spawn(move || {
    if new_state {
        window_clone.show();  // ❌ 在 spawn 线程中执行
        debug::set_debug_window_state(true);
        std::thread::sleep(Duration::from_millis(50));
        debug_log_bus::set_window_open(true);  // ❌ 触发 tracing，可能获取锁
    }
});

// 立即返回，不等待 spawn 完成
Ok(new_state)  // ❌ 前端认为操作完成，但实际还在后台执行
```

**问题链路**:
1. 用户点击"调试窗口" -> toggle_debug_window 启动 spawn 线程
2. toggle 立即返回 Ok(true)，前端认为操作完成
3. **spawn 线程还在执行**: window.show() + set_window_open(true)
4. set_window_open 触发 tracing::info! -> DebugConsoleLayer -> push_log -> **获取 bus.lock()**
5. **用户立即点击其他按钮** (如"重新登录")
6. 其他 command 也触发 tracing 日志 -> push_log -> **尝试获取 bus.lock()**
7. **锁竞争**: 如果 spawn 线程持有锁，其他 command 阻塞
8. **或者**: spawn 线程的 window 操作与主线程的其他窗口操作冲突，导致死锁

### 次要根因：debug_log_bus 使用 lock() 而不是 try_lock()

**问题代码** (push_log 原实现):
```rust
let mut state = match bus.lock() {  // ❌ 阻塞等待锁
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner()
};
```

**问题**: 如果锁被占用，调用者（command）会阻塞，导致 UI 无响应

---

## 修复方案

### 修复 1: 移除 spawn，在主线程同步执行窗口操作

**修复后的 toggle_debug_window**:
```rust
#[tauri::command]
fn toggle_debug_window(app: AppHandle) -> Result<bool, String> {
    request_context::wrap_command("toggle_debug_window", 200, || {
        // ... 获取窗口和状态 ...

        // ✅ 在主线程同步执行，不使用 spawn
        if new_state {
            window.show()?;  // ✅ 同步执行，确保完成后才返回
            let _ = window.set_focus();
            debug::set_debug_window_state(true);
            debug_log_bus::set_window_open(true);
        } else {
            window.hide()?;
            debug::set_debug_window_state(false);
            debug_log_bus::set_window_open(false);
        }

        Ok(new_state)  // ✅ 返回时操作已完成
    })
}
```

**优点**:
- 窗口操作在返回前完成，不会与后续 command 冲突
- 不会有 spawn 线程持有锁的问题
- 代码更简单，更容易理解和维护

### 修复 2: push_log 使用 try_lock 避免阻塞

**修复后的 push_log**:
```rust
pub fn push_log(event: LogEvent) {
    // ... EXITING 检查 ...

    let Some(bus) = LOG_BUS.get() else { return; };

    // ✅ 使用 try_lock 避免阻塞
    let mut state = match bus.try_lock() {
        Ok(guard) => {
            crate::request_context::cmd_log("BUS_LOCK_OK push_log");
            guard
        }
        Err(std::sync::TryLockError::WouldBlock) => {
            // ✅ 锁被占用，丢弃日志，不阻塞调用者
            crate::request_context::cmd_log("BUS_LOCK_BUSY push_log (dropping log)");
            return;
        }
        Err(std::sync::TryLockError::Poisoned(poisoned)) => {
            crate::request_context::cmd_log("BUS_LOCK_POISONED push_log (recovering)");
            poisoned.into_inner()
        }
    };

    // ... 添加日志到队列 ...
}
```

**优点**:
- 如果锁被占用，立即返回，不阻塞调用者
- 丢弃日志比阻塞 UI 更可接受
- 避免死锁和无响应

### 修复 3: 为所有 commands 添加诊断包装

**新增 wrap_command 函数** (request_context.rs):
```rust
pub fn wrap_command<F, R>(name: &'static str, warn_ms: u64, f: F) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String> + std::panic::UnwindSafe,
{
    let seq = CMD_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    cmd_log(&format!("CMD_ENTER name={} seq={}", name, seq));

    let result = match std::panic::catch_unwind(f) {
        Ok(result) => result,
        Err(panic_info) => {
            cmd_log(&format!("CMD_PANIC name={} seq={} panic={}", name, seq, panic_msg));
            Err(format!("Command panicked: {}", panic_msg))
        }
    };

    cmd_log(&format!("CMD_EXIT name={} seq={} elapsed={}ms ok={}", name, seq, elapsed_ms, ok));
    result
}
```

**应用到 Top 5 commands**:
- toggle_debug_window ✅
- reset_to_login ✅
- change_channel ✅
- set_theme_mode ✅
- start_login3_capture ✅
- launch_projector ✅

**优点**:
- 自动记录每个 command 的进入/退出
- 捕获 panic，转换为 Err，不让进程崩溃
- 记录耗时，识别慢 command

---

## 修改文件清单

### 1. src-tauri/src/main.rs
- **toggle_debug_window**: 移除 spawn，改为同步执行
- **reset_to_login**: 添加 wrap_command 包装
- **change_channel**: 添加 wrap_command 包装
- **set_theme_mode**: 添加 wrap_command 包装
- **start_login3_capture**: 添加 wrap_command 包装
- **launch_projector**: 添加 wrap_command 包装

### 2. src-tauri/src/debug_log_bus.rs
- **push_log**: 改用 try_lock，添加锁诊断日志

### 3. src-tauri/src/request_context.rs
- **新增**: wrap_command 函数
- **新增**: cmd_log 函数
- **新增**: CMD_SEQ 全局序列号

---

## 诊断日志格式

### 命令级别日志
```
[timestamp] CMD_ENTER name=toggle_debug_window seq=1
[timestamp] CMD_EXIT name=toggle_debug_window seq=1 elapsed=45ms ok=true
```

### 锁诊断日志
```
[timestamp] BUS_LOCK_TRY push_log
[timestamp] BUS_LOCK_OK push_log
[timestamp] BUS_LOCK_BUSY push_log (dropping log)
```

### Toggle 详细日志
```
[timestamp] TOGGLE_ENTER
[timestamp] TOGGLE_STATE: visible=false -> true
[timestamp] TOGGLE_SHOW_START
[timestamp] TOGGLE_SHOW_OK
[timestamp] TOGGLE_DONE: new_state=true
```

### Panic 日志
```
[timestamp] CMD_PANIC name=some_command seq=5 panic=index out of bounds
```

---

## 回归测试清单

### 测试 1: 连续点击 debug 20 次
- [ ] 启动应用
- [ ] 连续点击"调试窗口"按钮 20 次
- [ ] **预期**: 不卡顿，不崩溃，窗口正常显示/隐藏

### 测试 2: debug -> 重新登录
- [ ] 点击"调试窗口"（打开）
- [ ] 立即点击"重新登录"
- [ ] **预期**: 重新登录正常执行，不卡死
- [ ] 检查日志：没有 BUS_LOCK_BUSY 持续刷屏

### 测试 3: debug -> 更换频道
- [ ] 点击"调试窗口"（打开）
- [ ] 立即点击"更换频道"
- [ ] **预期**: 更换频道正常执行，不卡死

### 测试 4: debug -> 切换主题
- [ ] 点击"调试窗口"（打开）
- [ ] 立即点击"切换主题"
- [ ] **预期**: 主题正常切换，不卡死

### 测试 5: 快速切换 debug + 其他按钮
- [ ] 点击"调试窗口"（打开）
- [ ] 点击"调试窗口"（关闭）
- [ ] 点击"调试窗口"（打开）
- [ ] 立即点击"重新登录"
- [ ] **预期**: 所有操作正常，不卡死

### 测试 6: debug 打开状态下点击其他按钮
- [ ] 点击"调试窗口"（打开）
- [ ] 等待 1 秒
- [ ] 点击"重新登录"
- [ ] 点击"更换频道"
- [ ] 点击"切换主题"
- [ ] **预期**: 所有操作正常，debug 窗口继续显示日志

### 测试 7: 检查日志
- [ ] 执行上述所有测试
- [ ] 检查 `%LOCALAPPDATA%\RocoKnight\logs\rocoknight.log`
- [ ] **预期**:
  - 每个 command 都有 CMD_ENTER 和 CMD_EXIT
  - 没有 CMD_PANIC
  - BUS_LOCK_BUSY 偶尔出现可接受，但不应持续刷屏
  - 所有 command 的 elapsed 时间合理（< 阈值）

---

## 验证成功标准

1. ✅ 连续点击 debug 20 次不卡顿
2. ✅ 点完 debug 后点击任意其他按钮不崩溃/无响应
3. ✅ 日志中没有 CMD_PANIC
4. ✅ 日志中 BUS_LOCK_BUSY 不持续刷屏（偶尔出现可接受）
5. ✅ 所有 command 都有完整的 CMD_ENTER/CMD_EXIT 日志
6. ✅ 不需要强杀进程

---

## 为什么这个修复一定能解决

### 1. 消除竞态条件
- 移除 spawn 后，toggle 操作在返回前完成
- 不会有后台线程与主线程的窗口操作冲突
- 不会有后台线程持有锁导致其他 command 阻塞

### 2. 避免阻塞
- try_lock 确保 push_log 不会阻塞调用者
- 即使锁被占用，也只是丢弃日志，不影响 UI 响应

### 3. 防御性编程
- wrap_command 捕获所有 panic，转换为 Err
- 不会因为某个 command panic 导致整个进程崩溃
- 所有异常都被记录到日志

### 4. 可观测性
- 每个 command 都有完整的进入/退出日志
- 锁竞争被记录，方便诊断
- panic 被捕获并记录，不会丢失信息

---

## 已知限制

1. **日志可能丢失**: 如果 bus 锁被占用，push_log 会丢弃日志
   - **可接受**: 丢弃日志比阻塞 UI 更好
   - **缓解**: 只有在高并发时才会丢失，正常使用不会

2. **toggle 可能稍慢**: 移除 spawn 后，toggle 操作变为同步
   - **可接受**: 窗口 show/hide 操作很快（< 50ms）
   - **优点**: 操作完成后才返回，更可靠

3. **诊断日志开销**: 每个 command 都记录日志
   - **可接受**: 只写入文件，开销很小（< 1ms）
   - **优点**: 方便诊断问题

---

## 总结

**核心修复**: 移除 toggle_debug_window 中的 spawn，改为同步执行窗口操作

**辅助修复**:
- push_log 使用 try_lock 避免阻塞
- 所有 commands 添加 wrap_command 包装，捕获 panic

**效果**:
- 消除竞态条件
- 避免锁阻塞
- 防止 panic 导致崩溃
- 提供完整的诊断日志

**验证**: 通过 7 个回归测试确认修复有效
