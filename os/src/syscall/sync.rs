use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore, WaitOn};
use crate::task::{block_current_and_run_next, current_process, current_task};
use crate::timer::{add_timer, get_time_ms};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;

/// barrier 信号量 id：只用于线程间同步，永不参与死锁环。
const BARRIER_SEM: usize = 0;

/// 获取当前线程的 tid
fn current_tid() -> usize {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid
}

/// 判断当前线程请求 `w` 是否会死锁。
///
/// 先读取各信号量当前的可用数量，再交给本进程的死锁检测器
/// 用银行家算法判断是否存在安全序列。
fn deadlock_detect(w: WaitOn) -> bool {
    // barrier 永不参与死锁环。
    if w == WaitOn::Semaphore(BARRIER_SEM) {
        return false;
    }
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    if !process_inner.deadlock_detector.is_enabled() {
        return false;
    }
    let cur_tid = current_tid();
    let num_tasks = process_inner.tasks.len();
    // 收集各信号量的可用数量（count 大于 0 时取 count，否则 0）。
    let mut sem_avail: BTreeMap<usize, usize> = BTreeMap::new();
    for (sid, slot) in process_inner.semaphore_list.iter().enumerate() {
        if let Some(sem) = slot {
            let count = sem.inner.exclusive_access().count;
            sem_avail.insert(sid, if count > 0 { count as usize } else { 0 });
        }
    }
    process_inner
        .deadlock_detector
        .would_deadlock(cur_tid, w, num_tasks, &sem_avail)
}
/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_list.len() as isize - 1
    }
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let tid = current_tid();
    let w = WaitOn::Mutex(mutex_id);
    // 先假设本线程要等这把锁，检测会不会成环；安全则保留等待标记直到真正拿到，
    // 这样在 mutex.lock() 阻塞期间，其他线程的检测仍能看到本线程在等这把锁。
    current_process()
        .inner_exclusive_access()
        .deadlock_detector
        .begin_wait(tid, w);
    if deadlock_detect(w) {
        current_process()
            .inner_exclusive_access()
            .deadlock_detector
            .clear_wait(tid);
        return -0xdead;
    }
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    mutex.lock();
    // 成功持有：清除等待标记，记录持有。
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.deadlock_detector.clear_wait(tid);
    process_inner.deadlock_detector.hold_mutex(tid, mutex_id);
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    // 先解除本线程对这把锁的持有记录，再真正解锁。
    let tid = current_tid();
    current_process()
        .inner_exclusive_access()
        .deadlock_detector
        .drop_mutex(tid, mutex_id);
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    mutex.unlock();
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_list.len() - 1
    };
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    // barrier 不参与检测；其余信号量先解除持有记录，再执行 up（V 操作）。
    let tid = current_tid();
    if sem_id != BARRIER_SEM {
        current_process()
            .inner_exclusive_access()
            .deadlock_detector
            .drop_sem(tid, sem_id);
    }
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    sem.up();
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    // barrier 不参与检测；其余信号量与 mutex_lock 同理：先假设要等它，
    // 若会成环则立即拒绝；安全则保留等待标记直到真正持有后清除。
    let tid = current_tid();
    if sem_id != BARRIER_SEM {
        let w = WaitOn::Semaphore(sem_id);
        current_process()
            .inner_exclusive_access()
            .deadlock_detector
            .begin_wait(tid, w);
        if deadlock_detect(w) {
            current_process()
                .inner_exclusive_access()
                .deadlock_detector
                .clear_wait(tid);
            return -0xdead;
        }
    }
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    sem.down();
    // 成功持有：清除等待标记，记录持有。
    if sem_id != BARRIER_SEM {
        let process = current_process();
        let mut process_inner = process.inner_exclusive_access();
        process_inner.deadlock_detector.clear_wait(tid);
        process_inner.deadlock_detector.hold_sem(tid, sem_id);
    }
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    trace!("kernel: sys_enable_deadlock_detect");
    current_process()
        .inner_exclusive_access()
        .deadlock_detector
        .set_enabled(enabled == 1);
    0
}
