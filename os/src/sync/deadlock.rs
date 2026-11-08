//! Deadlock detection using the banker's algorithm, stored per process.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;

/// 线程正在等待的资源。
///
/// 与「持有」不同，一个线程同一时刻最多只等一个资源，
/// 所以单独用一个枚举表示等待目标就够。
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum WaitOn {
    /// 等待一把互斥锁（用 id 标识）
    Mutex(usize),
    /// 等待一个信号量（用 id 标识）
    Semaphore(usize),
}

/// 每个进程一个的死锁检测器。
///
/// 不维护大矩阵，只用稀疏表（BTreeMap / BTreeSet）记录：
/// - 每个线程持有哪些 mutex；
/// - 每个线程持有哪些 semaphore（及数量）；
/// - 每个线程正在等待哪个资源。
///
/// 检测时读入各信号量的当前可用数量，运行银行家算法判断
/// 是否存在一种执行顺序，让所有线程都能依次完成并释放资源。
pub struct DeadlockDetector {
    enabled: bool,
    /// 线程 tid -> 持有的 mutex id 集合（一把 mutex 最多持有一个）
    mutex_held: BTreeMap<usize, BTreeSet<usize>>,
    /// 线程 tid -> 持有的 semaphore id -> 数量
    sem_held: BTreeMap<usize, BTreeMap<usize, usize>>,
    /// 线程 tid -> 正在等待的资源
    waiting: BTreeMap<usize, WaitOn>,
}

impl DeadlockDetector {
    /// 创建一个关闭状态、空白的检测器
    pub fn new() -> Self {
        Self {
            enabled: false,
            mutex_held: BTreeMap::new(),
            sem_held: BTreeMap::new(),
            waiting: BTreeMap::new(),
        }
    }

    /// 打开 / 关闭检测
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// 是否已开启检测
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 标记线程 `tid` 开始等待资源 `w`
    pub fn begin_wait(&mut self, tid: usize, w: WaitOn) {
        self.waiting.insert(tid, w);
    }

    /// 清除线程 `tid` 的等待标记
    pub fn clear_wait(&mut self, tid: usize) {
        self.waiting.remove(&tid);
    }

    /// 线程 `tid` 持有一把 mutex
    pub fn hold_mutex(&mut self, tid: usize, mid: usize) {
        self.mutex_held.entry(tid).or_default().insert(mid);
    }

    /// 线程 `tid` 释放一把 mutex
    pub fn drop_mutex(&mut self, tid: usize, mid: usize) {
        let mut remove_tid = false;
        if let Some(held) = self.mutex_held.get_mut(&tid) {
            held.remove(&mid);
            remove_tid = held.is_empty();
        }
        if remove_tid {
            self.mutex_held.remove(&tid);
        }
    }

    /// 线程 `tid` 持有一个信号量单位
    pub fn hold_sem(&mut self, tid: usize, sid: usize) {
        *self.sem_held.entry(tid).or_default().entry(sid).or_insert(0) += 1;
    }

    /// 线程 `tid` 释放一个信号量单位
    pub fn drop_sem(&mut self, tid: usize, sid: usize) {
        let mut remove_tid = false;
        if let Some(held) = self.sem_held.get_mut(&tid) {
            if let Some(c) = held.get_mut(&sid) {
                *c -= 1;
            }
            if held.get(&sid).copied().unwrap_or(0) == 0 {
                held.remove(&sid);
            }
            remove_tid = held.is_empty();
        }
        if remove_tid {
            self.sem_held.remove(&tid);
        }
    }

    /// 银行家算法：当前线程 `cur_tid` 请求 `cur_req` 是否会死锁。
    ///
    /// - `num_tasks`：进程里的线程槽位数（tid 与下标一一对应）。
    /// - `sem_avail`：各信号量当前的可用数量（count 大于 0 取 count，否则 0）。
    ///
    /// 返回 `true` 表示会死锁（不存在安全序列）。
    pub fn would_deadlock(
        &self,
        cur_tid: usize,
        cur_req: WaitOn,
        num_tasks: usize,
        sem_avail: &BTreeMap<usize, usize>,
    ) -> bool {
        // 可用量：mutex 被持有记 0，未被持有（不在表里）默认 1；
        // semaphore 直接用传入的可用数量。
        let mut work_mutex: BTreeMap<usize, usize> = BTreeMap::new();
        for held in self.mutex_held.values() {
            for &m in held {
                work_mutex.insert(m, 0);
            }
        }
        let mut work_sem: BTreeMap<usize, usize> = sem_avail.clone();

        // 反复找「请求可被满足」的线程，让它完成并释放持有的资源。
        let mut finish = vec![false; num_tasks];
        loop {
            let mut found = false;
            for tid in 0..num_tasks {
                if finish[tid] {
                    continue;
                }
                // 当前线程按 cur_req 处理，其余线程看它正在等什么。
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
                    // 释放该线程持有的所有资源。
                    if let Some(held) = self.mutex_held.get(&tid) {
                        for &m in held {
                            *work_mutex.entry(m).or_insert(0) += 1;
                        }
                    }
                    if let Some(held) = self.sem_held.get(&tid) {
                        for (&s, &c) in held {
                            *work_sem.entry(s).or_insert(0) += c;
                        }
                    }
                    finish[tid] = true;
                    found = true;
                }
            }
            if !found {
                break;
            }
        }
        // 有人永远无法完成 -> 存在等待环 -> 死锁
        !finish.iter().all(|&f| f)
    }
}
