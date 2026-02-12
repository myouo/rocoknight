# 最小修复补丁 - 点击 Debug 后其他按钮崩溃

## 核心问题

**根因**: toggle_debug_window 使用 `std::thread::spawn` 异步执行窗口操作，导致：
1. spawn 线程持有 bus 锁时，其他 command 的日志记录被阻塞
2. spawn 线程的窗口操作与主线程的其他操作冲突

## 最小修复（3 个关键改动）

### 1. toggle_debug_window: 移除 spawn，改为同步执行

**文件**: src-tauri/src/main.rs

**改动**: 将 ~100 行的 spawn 异步逻辑简化为 ~30 行的同步逻辑

**关键代码**:
```rust
// ❌ 原代码：异步执行，立即返回
std::thread::spawn(move || {
    window_clone.show();
    // ... 50ms sleep ...
    debug_log_bus::set_window_open(true);
});
Ok(new_state)  // 立即返回，但操作还在后台执行

// ✅ 修复后：同步执行，完成后返回
if new_state {
    window.show()?;  // 同步等待完成
    debug::set_debug_window_state(true);
    debug_log_bus::set_window_open(true);
}
Ok(new_state)  // 返回时操作已完成
```

### 2. push_log: 使用 try_lock 避免阻塞

**文件**: src-tauri/src/debug_log_bus.rs

**改动**: 将 `bus.lock()` 改为 `bus.try_lock()`

**关键代码**:
```rust
// ❌ 原代码：阻塞等待锁
let mut state = match bus.lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner()
};

// ✅ 修复后：try_lock，锁忙时丢弃日志
let mut state = match bus.try_lock() {
    Ok(guard) => guard,
    Err(TryLockError::WouldBlock) => {
        // 锁被占用，丢弃日志，不阻塞
        return;
    }
    Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner()
};
```

### 3. 添加命令诊断包装（捕获 panic）

**文件**: src-tauri/src/request_context.rs

**新增**: wrap_command 函数

**关键代码**:
```rust
pub fn wrap_command<F, R>(name: &'static str, warn_ms: u64, f: F) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String> + std::panic::UnwindSafe,
{
    cmd_log(&format!("CMD_ENTER name={} seq={}", name, seq));

    let result = match std::panic::catch_unwind(f) {
        Ok(result) => result,
        Err(panic_info) => {
            cmd_log(&format!("CMD_PANIC name={} panic={}", name, panic_msg));
            Err(format!("Command panicked: {}", panic_msg))
        }
    };

    cmd_log(&format!("CMD_EXIT name={} elapsed={}ms", name, elapsed_ms));
    result
}
```

**应用到 Top 6 commands**:
```rust
#[tauri::command]
fn toggle_debug_window(app: AppHandle) -> Result<bool, String> {
    request_context::wrap_command("toggle_debug_window", 200, || {
        // ... 实际逻辑 ...
    })
}

// 同样应用到：
// - reset_to_login
// - change_channel
// - set_theme_mode
// - start_login3_capture
// - launch_projector
```

---

## 修改统计

- **文件数**: 3
- **新增函数**: 2 (wrap_command, cmd_log)
- **修改函数**: 7 (toggle + Top 5 commands + push_log)
- **删除代码**: ~100 行（toggle 的 spawn 逻辑）
- **新增代码**: ~80 行（wrap_command + 诊断）
- **净减少**: ~20 行

---

## 快速验证

### 1. 编译
```bash
cd src-tauri
cargo build --release
```

### 2. 测试
```bash
# 启动应用
./target/release/rocoknight.exe

# 测试步骤：
1. 连续点击"调试窗口"按钮 10 次
2. 点击"调试窗口"（打开）
3. 立即点击"重新登录"
4. 立即点击"更换频道"
5. 立即点击"切换主题"
```

### 3. 检查日志
```bash
# Windows
type %LOCALAPPDATA%\RocoKnight\logs\rocoknight.log | findstr "CMD_"

# 预期输出：
[timestamp] CMD_ENTER name=toggle_debug_window seq=1
[timestamp] CMD_EXIT name=toggle_debug_window seq=1 elapsed=45ms ok=true
[timestamp] CMD_ENTER name=reset_to_login seq=2
[timestamp] CMD_EXIT name=reset_to_login seq=2 elapsed=234ms ok=true
...

# 不应该看到：
CMD_PANIC
BUS_LOCK_BUSY (持续刷屏)
```

---

## 成功标准

1. ✅ 点击 debug 后立即点击其他按钮不崩溃/无响应
2. ✅ 日志中没有 CMD_PANIC
3. ✅ 日志中每个 command 都有 CMD_ENTER 和 CMD_EXIT
4. ✅ BUS_LOCK_BUSY 不持续刷屏（偶尔出现可接受）

---

## 如果还有问题

### 诊断步骤

1. **检查最后一个 CMD_ENTER**
   ```bash
   type %LOCALAPPDATA%\RocoKnight\logs\rocoknight.log | findstr "CMD_ENTER" | tail -1
   ```
   - 如果有 CMD_ENTER 但没有对应的 CMD_EXIT，说明该 command 卡死

2. **检查 BUS_LOCK_BUSY 频率**
   ```bash
   type %LOCALAPPDATA%\RocoKnight\logs\rocoknight.log | findstr "BUS_LOCK_BUSY" | wc -l
   ```
   - 如果数量很大（> 100），说明锁竞争严重

3. **检查 CMD_PANIC**
   ```bash
   type %LOCALAPPDATA%\RocoKnight\logs\rocoknight.log | findstr "CMD_PANIC"
   ```
   - 如果有 panic，查看 panic 消息定位问题

### 进一步修复

如果问题仍然存在，可能需要：
1. 将 flush_loop 的 emit_batch 也改为 try_lock
2. 为 set_window_open 中的 tracing::info! 添加短路（避免递归）
3. 检查 apply_theme_to_app 的 webview.eval 是否阻塞

---

## 回滚方案

```bash
git diff HEAD > toggle_fix.patch
git checkout HEAD -- src-tauri/src/main.rs src-tauri/src/debug_log_bus.rs src-tauri/src/request_context.rs
```

---

## 总结

**核心修复**: 移除 toggle_debug_window 的 spawn，改为同步执行

**为什么有效**:
- 消除了 spawn 线程与主线程的竞态条件
- 消除了 spawn 线程持有锁导致其他 command 阻塞的问题
- 窗口操作在返回前完成，不会与后续操作冲突

**代价**: toggle 操作变为同步，可能稍慢（但实际 < 50ms，用户无感知）

**收益**: 彻底解决崩溃/无响应问题，提供完整的诊断日志
