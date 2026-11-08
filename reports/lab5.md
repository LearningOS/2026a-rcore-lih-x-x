# Lab5 实验报告：并发与同步（ch8）

## 一、实验目的

本章引入并发与同步机制，解决多线程访问共享资源时的竞态条件：

- 三个同步原语：互斥锁（mutex）、信号量（semaphore）、条件变量（condvar）；
- 线程（thread）；
- **死锁检测**（deadlock detection）。

在此基础上完成 lab5：实现死锁检测，并在 `mutex_lock`/`semaphore_down` 时若判定会死锁则返回 `-0xdead`。

## 二、实验内容与要求

### 1. 重做 8 个 syscall

`get_time`/`mmap`/`munmap`/`spawn`/`set_priority` + `fstat`/`linkat`/`unlinkat`（ch5/ch6 实现过，ch8 重构后重做）。

### 2. 死锁检测（核心）

实现 `sys_enable_deadlock_detect`，并在 `mutex_lock`/`semaphore_down` 时检测死锁，若会死锁则返回 `-0xdead`。

## 三、实现思路

### 死锁检测：银行家算法 + 每个进程一个检测器

不维护全局大矩阵，也不把状态塞进线程控制块，而是**每个进程放一个 `DeadlockDetector`**，用稀疏表记录「谁持有谁、谁在等谁」：

- `mutex_held`：线程 tid → 持有的 mutex id 集合；
- `sem_held`：线程 tid → 持有的 semaphore id → 数量；
- `waiting`：线程 tid → 正在等待的资源（用 `WaitOn` 枚举区分 mutex / semaphore）；
- `enabled`：检测开关。

```rust
pub enum WaitOn { Mutex(usize), Semaphore(usize) }
```

### 银行家算法核心：判断是否存在「安全序列」

检测线程请求资源 `w` 时：

1. 先读各信号量当前的可用数量（`count > 0 ? count : 0`）；mutex 的可用量则是「被持有记 0，未持有记 1」。
2. 反复寻找「请求可被满足」的线程，让它「完成」并释放其持有的资源，直到没有线程能推进。
3. 全部完成 → 安全（不死锁）；有人永远完不成 → 存在等待环 → 死锁。

### barrier 信号量的排除

约定 **id=0 的信号量是 barrier**，只用于线程同步（每个线程都会 acquire 再 release），永不参与死锁环。syscall 层对 `sem_id == 0` 直接跳过检测器的记录（不记持有、不记等待），只做原始的 down/up。

### 集成点

- `sys_mutex_lock` / `sys_semaphore_down`：先 `begin_wait` 标记等待 → `deadlock_detect` 检测 → 死锁则清标记返回 `-0xdead`；安全则**保留等待标记** → 真正 `lock`/`down`（可能阻塞，阻塞期间别人仍能看到本线程在等）→ `clear_wait` + 记录持有。
- `sys_mutex_unlock` / `sys_semaphore_up`：先 `drop_mutex`/`drop_sem` 解除持有记录，再真正 `unlock`/`up`。
- `sys_enable_deadlock_detect`：设置检测器的 `enabled` 开关。

## 四、关键代码

### 检测器（`os/src/sync/deadlock.rs`）

```rust
pub struct DeadlockDetector {
    enabled: bool,
    mutex_held: BTreeMap<usize, BTreeSet<usize>>,        // tid -> mutex 集合
    sem_held: BTreeMap<usize, BTreeMap<usize, usize>>,   // tid -> sem -> 数量
    waiting: BTreeMap<usize, WaitOn>,                    // tid -> 正在等的资源
}

impl DeadlockDetector {
    pub fn would_deadlock(
        &self,
        cur_tid: usize,
        cur_req: WaitOn,
        num_tasks: usize,
        sem_avail: &BTreeMap<usize, usize>,
    ) -> bool {
        // mutex 可用量：被持有记 0，未持有（不在表里）默认 1
        let mut work_mutex = BTreeMap::new();
        for held in self.mutex_held.values() {
            for &m in held { work_mutex.insert(m, 0); }
        }
        let mut work_sem = sem_avail.clone();

        // 银行家算法：反复找「请求可被满足」的线程，让它完成并释放资源
        let mut finish = vec![false; num_tasks];
        loop {
            let mut found = false;
            for tid in 0..num_tasks {
                if finish[tid] { continue; }
                let req = if tid == cur_tid {
                    Some(cur_req)
                } else {
                    self.waiting.get(&tid).copied()
                };
                let satisfiable = match req {
                    Some(WaitOn::Mutex(m)) => work_mutex.get(&m).copied().unwrap_or(1) >= 1,
                    Some(WaitOn::Semaphore(s)) => work_sem.get(&s).copied().unwrap_or(0) >= 1,
                    None => true,
                };
                if satisfiable {
                    if let Some(held) = self.mutex_held.get(&tid) {
                        for &m in held { *work_mutex.entry(m).or_insert(0) += 1; }
                    }
                    if let Some(held) = self.sem_held.get(&tid) {
                        for (&s, &c) in held { *work_sem.entry(s).or_insert(0) += c; }
                    }
                    finish[tid] = true;
                    found = true;
                }
            }
            if !found { break; }
        }
        !finish.iter().all(|&f| f)
    }
}
```

### syscall 侧（`os/src/syscall/sync.rs`）

```rust
fn deadlock_detect(w: WaitOn) -> bool {
    if w == WaitOn::Semaphore(BARRIER_SEM) { return false; }
    let process_inner = current_process().inner_exclusive_access();
    if !process_inner.deadlock_detector.is_enabled() { return false; }

    // 读各信号量当前的可用数量
    let mut sem_avail = BTreeMap::new();
    for (sid, slot) in process_inner.semaphore_list.iter().enumerate() {
        if let Some(sem) = slot {
            let count = sem.inner.exclusive_access().count;
            sem_avail.insert(sid, if count > 0 { count as usize } else { 0 });
        }
    }
    process_inner.deadlock_detector.would_deadlock(
        current_tid(), w, process_inner.tasks.len(), &sem_avail,
    )
}
```

## 五、调试过程（踩坑记录）

1. **StorePageFault（栈溢出）**：最初用全局 `DeadLockDetect` 结构体，内含 `128×256` 的 `allocation` 数组（约 256KB），在 `new()` 时超出 8KB 内核栈，触发缺页。→ 改为动态集合 `BTreeSet`/`BTreeMap`，只在检测时临时分配。
2. **越界 panic**：固定数组下标未做边界检查，`index 16` 越界。→ 加入边界判断。
3. **全局检测器冲突（21/25）**：全局 `DEADLOCK_DETECT` 让不同进程的资源 id 混在一起，导致 `ch8b_sem1/sem2` 误判、condvar 互斥断言失败。→ 改为**每个进程一个 `DeadlockDetector`**（放进 `ProcessControlBlockInner`），状态互不串扰。
4. **等待标记过早清除导致漏检**：若在 `lock`/`down` 之前就清掉 `waiting`，阻塞期间其他线程看不到它，环检测失效，测试会整体挂起。→ 改为**成功持有后才 `clear_wait`**。

## 六、实验结果

`cargo build` 编译通过；官方 checker 验证 **25/25** 全部通过。

## 七、思考与总结

1. **死锁的本质**：多个线程循环等待资源，形成「等待环」。四个必要条件：互斥、持有并等待、不可剥夺、循环等待。
2. **银行家算法 vs 环检测**：银行家算法判断「是否存在安全序列」，比单纯的 DFS 环检测更通用（能正确处理信号量的多计数）。
3. **状态放哪很重要**：死锁状态应放在**进程级**（每个进程一个检测器），而不是塞进一个全局大表，否则会跨进程串扰、也容易栈溢出。
4. **barrier 要排除**：仅用于同步的信号量（每个线程都 acquire/release）不能参与死锁环判定，否则必然误报。

## 八、参考文献

- [rCore-Tutorial-Book-v3 第 8 章](https://rcore-os.github.io/rCore-Tutorial-Book-v3/)
- [rCore-Tutorial-Guide](https://learningos.github.io/rCore-Tutorial-Guide/)
