//! Process management syscalls
use crate::{
    task::{exit_current_and_run_next, suspend_current_and_run_next},
    timer::get_time_us,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("[kernel] Application exited with code {}", exit_code);
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// get time with second and microsecond
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    unsafe {
        *ts = TimeVal {
            sec: us / 1_000_000,
            usec: us % 1_000_000,
        };
    }
    0
}

// trace request 类型（与 user_lib 的 TraceRequest 枚举对应）
const TRACE_READ: usize = 0;
const TRACE_WRITE: usize = 1;
const TRACE_SYSCALL: usize = 2;

/// trace syscall：支持读内存、写内存、统计系统调用次数三种操作
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    match trace_request {
        // 返回 id 号系统调用的调用次数
        TRACE_SYSCALL => crate::task::get_syscall_count(id) as isize,
        // 读地址 id 处的一个字节
        TRACE_READ => unsafe { *(id as *const u8) as isize },
        // 往地址 id 写一个字节 data
        TRACE_WRITE => {
            unsafe {
                *(id as *mut u8) = data as u8;
            }
            0
        }
        _ => -1,
    }
}
