# Lab3 实验报告：进程管理（ch5）

## 一、实验目的

本章把内核从 ch4 的「多任务」升级为「进程管理」：

- 引入进程的 PID、父子关系、生命周期（fork/exec/exit/waitpid）；
- 引入 **stride 调度**（优先级调度），让进程获得的 CPU 时间正比于 priority。

在此基础上完成 lab3：实现进程管理的系统调用。

## 二、实验内容与要求

实现以下 5 个系统调用：

| 任务 | 要求 |
|------|------|
| `sys_get_time` | 用 `translated_refmut` 把 TimeVal 写到用户地址 |
| `sys_mmap` | 页对齐 + prot 合法 + 不重叠 + 映射 |
| `sys_munmap` | 页对齐 + 解除映射 |
| `sys_spawn` | 直接创建新进程运行指定程序 |
| `sys_set_priority` | 设置优先级 + stride 调度 |

## 三、实现思路

### 1. sys_get_time

用 `translated_refmut` 把内核里的 `TimeVal` 写到用户地址（虚拟地址翻译）。

### 2. sys_mmap / sys_munmap

- mmap：检查页对齐、prot 合法（bit 0/1/2 = R/W/X），转换权限，检查不重叠（`MemorySet::overlap`），然后 `insert_framed_area`。
- munmap：检查页对齐，`remove_area_with_start_vpn` 解除映射。

### 3. sys_spawn

`spawn` 直接 `TaskControlBlock::new(程序)` 创建新进程加载程序，不复制父进程（区别于 fork+exec）。

**关键：要建立父子关系**——设置子进程的 `parent` 指向当前任务，并把子进程加入当前任务的 `children` 列表，否则父进程 `waitpid` 找不到子进程。

### 4. sys_set_priority + stride 调度

stride 调度让进程获得的 CPU 时间正比于 priority：

- 每个进程有 `priority`（>= 2）、`stride = BIG_STRIDE / priority`、`pass`（累计值，初始 0）；
- 调度器每次选 `pass` 最小的进程，运行后 `pass += stride`。

## 四、关键代码

### stride 调度（manager.rs 的 fetch）

```rust
pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
    if self.ready_queue.is_empty() { return None; }
    let mut min_idx = 0;
    let mut min_pass = usize::MAX;
    for (i, task) in self.ready_queue.iter().enumerate() {
        let pass = task.inner_exclusive_access().pass;
        if pass < min_pass { min_pass = pass; min_idx = i; }
    }
    let task = self.ready_queue.remove(min_idx).unwrap();
    let mut inner = task.inner_exclusive_access();
    inner.pass += inner.stride;   // 被调度后 pass += stride
    drop(inner);
    Some(task)
}
```

### set_priority（task.rs）

```rust
pub fn set_priority(&self, prio: isize) -> isize {
    if prio < 2 { return -1; }   // priority 必须 >= 2
    let mut inner = self.inner_exclusive_access();
    inner.priority = prio as usize;
    inner.stride = BIG_STRIDE / inner.priority;
    inner.pass = 0;
    prio
}
```

### sys_spawn

```rust
pub fn sys_spawn(path: *const u8) -> isize {
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let new_task = Arc::new(TaskControlBlock::new(data));
        let new_pid = new_task.pid.0;
        // 建立父子关系：子进程 parent 指向当前任务，加入当前任务的 children
        let parent = current_task().unwrap();
        new_task.inner_exclusive_access().parent = Some(Arc::downgrade(&parent));
        let mut parent_inner = parent.inner_exclusive_access();
        parent_inner.children.push(new_task.clone());
        drop(parent_inner);
        add_task(new_task);
        new_pid as isize
    } else { -1 }
}
```

## 五、实验结果

`cargo build` 编译通过，官方 checker（`make -C ci-user test CHAPTER=5`）**15/15 全部通过**，lab1/lab2/lab3 三份报告检查通过。

stride 调度验证（count 应正比于 priority）：

```
priority = 6,  ratio = 3693733
priority = 7,  ratio = 3700000
priority = 8,  ratio = 3647150
priority = 9,  ratio = 3675200
priority = 10, ratio = 3707480
```

不同 priority 的 ratio（count/priority）几乎相等（≈370万），证明 count 正比于 priority，stride 调度正确。

## 六、思考与总结

1. **stride 调度原理**：priority 大的进程 stride 小，pass 增长慢，被调度次数多，最终获得的 CPU 时间与 priority 成正比。

2. **spawn vs fork+exec**：spawn 直接加载新程序创建新进程，不复制父进程的地址空间，比 fork+exec 更高效。

3. **进程 vs 任务**：进程有 PID、父子关系、生命周期，可以通过 fork/exec/spawn 动态创建，是「进程管理」的核心。

4. **调试经验：spawn 必须建立父子关系**：最初实现 spawn 时只 `TaskControlBlock::new(data)` 创建进程（parent=None），没建立父子关系，导致父进程 `waitpid` 找不到子进程返回 -1，checker 报「left=1, right=-1」断言失败。修复后设置 parent 并加入 children，问题解决。这体现了「进程的父子关系是 waitpid 正常工作的前提」。

## 七、参考文献

- [rCore-Tutorial-Book-v3 第 5 章](https://rcore-os.github.io/rCore-Tutorial-Book-v3/)
- [rCore-Tutorial-Guide](https://learningos.github.io/rCore-Tutorial-Guide/)
