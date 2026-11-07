# Lab1 实验报告：多道程序与分时多任务（ch3）

## 一、实验目的

本章把内核从 ch2 的「批处理系统」升级为「多道程序 + 分时多任务」：

- **多道程序**：启动时一次性把所有用户程序加载进内存，每个程序占一段独立的地址空间；
- **分时多任务**：通过时钟中断（每 10ms 一次）强制抢占当前任务，切换到下一个任务，实现时间片轮转。

在此基础上，完成 lab1 实验：**实现 `sys_trace` 系统调用**，让用户程序能够追踪和统计系统调用。

## 二、实验内容与要求

实现 `sys_trace` 系统调用（编号 410），支持三种操作模式（由第一个参数 `trace_request` 区分）：

| 操作 | 值 | 参数 | 功能 | 返回值 |
|------|---|------|------|--------|
| Read | 0 | `id` = 地址 | 读出该地址的一个字节 | 字节值；非法返回 -1 |
| Write | 1 | `id` = 地址，`data` = 字节 | 往该地址写一个字节 | 0 |
| Syscall | 2 | `id` = syscall 编号 | 返回该 syscall 的调用次数 | 次数 |

配套要求：内核需要**统计每个系统调用的调用次数**，且计数应**按任务独立**（存在每个任务的 TCB 里）。

## 三、实现思路

### 1. 系统调用计数（按任务）

给任务控制块 `TaskControlBlock` 增加一个字段：

```rust
pub syscall_times: [u32; MAX_SYSCALL_NUM],
```

- 数组下标 = 系统调用编号，元素值 = 该编号系统调用被调用的次数；
- 每个任务有自己的 TCB，因此计数是**按任务独立**的。

在 `syscall()` 函数**分发之前**计数：

```rust
pub fn syscall(syscall_id: usize, args: [usize; 3]) -> isize {
    crate::task::increase_syscall_count(syscall_id);  // 先计数
    match syscall_id { ... }                           // 再分发
}
```

关键点：计数放在 `match` 之前，这样 **`sys_trace` 调用本身也会被计入**（测试程序明确要求 `count_syscall(SYSCALL_TRACE)` 要包含本次调用）。

### 2. 实现 sys_trace 三种模式

```rust
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    match trace_request {
        TRACE_SYSCALL => crate::task::get_syscall_count(id) as isize,  // 统计
        TRACE_READ  => unsafe { *(id as *const u8) as isize },          // 读内存
        TRACE_WRITE => { unsafe { *(id as *mut u8) = data as u8; } 0 }  // 写内存
        _ => -1,
    }
}
```

- **Syscall 模式**：直接读取当前任务 TCB 里的计数并返回；
- **Read/Write 模式**：直接读写用户地址。因为 ch3 还没有地址空间隔离，用户程序的地址就是物理地址，内核可以安全地直接解引用裸指针。

## 四、关键代码与说明

### config.rs —— 新增常量

```rust
pub const MAX_SYSCALL_NUM: usize = 500;  // 最大 syscall 编号（最大的 410，留余量）
```

### task/task.rs —— TCB 增加计数字段

```rust
pub struct TaskControlBlock {
    pub task_status: TaskStatus,
    pub task_cx: TaskContext,
    pub syscall_times: [u32; MAX_SYSCALL_NUM],  // 新增
}
```

### task/mod.rs —— 计数相关的两个函数

```rust
pub fn increase_syscall_count(syscall_id: usize) {
    let mut inner = TASK_MANAGER.inner.exclusive_access();
    let current = inner.current_task;
    inner.tasks[current].syscall_times[syscall_id] += 1;
}

pub fn get_syscall_count(syscall_id: usize) -> u32 {
    let inner = TASK_MANAGER.inner.exclusive_access();
    let current = inner.current_task;
    inner.tasks[current].syscall_times[syscall_id]
}
```

- 通过 `UPSafeCell`（单核安全的 RefCell）访问全局静态 `TASK_MANAGER`；
- 先拷贝出 `current` 再索引，避免同时读写的借用冲突。

### syscall/process.rs —— 实现 sys_trace

见上文「实现思路」部分的代码。

## 五、实验结果

本地运行（`make -C os run BASE=0`，构建完整测试）：

```
string from task trace test
Test trace OK!
Test sleep OK!
Test sleep1 passed!
```

- `Test trace OK!` 表示 `ch3_trace` 测试程序中所有断言通过，说明 `count_syscall`、`trace_read`、`trace_write` 三个功能均正确实现；
- 结尾的 `Panicked ... All applications completed!` 是所有程序跑完后内核故意 panic 关机，属正常现象。

## 六、思考与总结

1. **为什么计数在 match 之前？** 因为 `sys_trace` 自己也是系统调用，`count_syscall(SYSCALL_TRACE)` 需要统计「包括本次在内的所有调用」，所以必须先计数再分发。

2. **为什么 Read/Write 能直接解引用裸指针？** 因为 ch3 还没有地址空间（页表）隔离，用户程序的虚拟地址 = 物理地址，内核可以直接访问。到 ch4 引入地址空间后，这一实现需要改为「先翻译虚拟地址再访问」。

3. **为什么计数要按任务而非全局？** 每个任务的系统调用统计是独立的，测试程序也要求「当前任务」的语义，因此计数存在 TCB 里。

