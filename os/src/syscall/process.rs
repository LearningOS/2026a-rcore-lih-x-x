//! Process management syscalls
use crate::config::PAGE_SIZE;
use crate::mm::{translated_byte_buffer, MapPermission, PageTable, PTEFlags, PhysAddr, VirtAddr};
use crate::task::{change_program_brk, exit_current_and_run_next, suspend_current_and_run_next};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
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
/// 用 translated_byte_buffer 把 TimeVal 写到用户地址（可能跨页）
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = crate::timer::get_time_us();
    let time = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let token = crate::task::current_user_token();
    let size = core::mem::size_of::<TimeVal>();
    let buffers = translated_byte_buffer(token, ts as *const u8, size);
    let time_bytes = unsafe {
        core::slice::from_raw_parts(&time as *const TimeVal as *const u8, size)
    };
    let mut start = 0;
    for dst in buffers {
        let end = start + dst.len();
        dst.copy_from_slice(&time_bytes[start..end]);
        start = end;
    }
    0
}

// trace request 类型（与 user_lib 的 TraceRequest 枚举对应）
const TRACE_READ: usize = 0;
const TRACE_WRITE: usize = 1;
const TRACE_SYSCALL: usize = 2;

/// 翻译用户虚拟地址，返回物理地址；非法（未映射/非用户/权限不足）返回 None
fn translate_user_va(va: usize, write: bool) -> Option<PhysAddr> {
    let token = crate::task::current_user_token();
    let page_table = PageTable::from_token(token);
    let vpn = VirtAddr::from(va).floor();
    let pte = page_table.translate(vpn)?;
    if !pte.is_valid() || !pte.flags().contains(PTEFlags::U) {
        return None;
    }
    if write && !pte.writable() {
        return None;
    }
    if !write && !pte.readable() {
        return None;
    }
    let pa = PhysAddr::from(pte.ppn()).0 + VirtAddr::from(va).page_offset();
    Some(PhysAddr(pa))
}

/// trace syscall：读内存、写内存、统计系统调用次数三种操作
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    match trace_request {
        // 返回 id 号系统调用的调用次数
        TRACE_SYSCALL => crate::task::get_syscall_count(id) as isize,
        // 读地址 id 处的一个字节（需翻译 + 权限检查）
        TRACE_READ => {
            if let Some(pa) = translate_user_va(id, false) {
                unsafe { *(pa.0 as *const u8) as isize }
            } else {
                -1
            }
        }
        // 往地址 id 写一个字节 data（需翻译 + 写权限检查）
        TRACE_WRITE => {
            if let Some(pa) = translate_user_va(id, true) {
                unsafe {
                    *(pa.0 as *mut u8) = data as u8;
                }
                0
            } else {
                -1
            }
        }
        _ => -1,
    }
}

/// mmap：映射一段虚拟内存
/// port 的 bit 0/1/2 分别表示 R/W/X，bit 3 及以上非法
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    trace!("kernel: sys_mmap");
    // start 必须页对齐
    if start % PAGE_SIZE != 0 {
        return -1;
    }
    // port 合法：不能有 bit 3 及以上（port & !0x7），且至少要有一个权限（port != 0）
    if port & !0x7 != 0 || port == 0 {
        return -1;
    }
    let mut perm = MapPermission::U;
    if port & 1 != 0 {
        perm |= MapPermission::R;
    }
    if port & 2 != 0 {
        perm |= MapPermission::W;
    }
    if port & 4 != 0 {
        perm |= MapPermission::X;
    }
    crate::task::current_mmap(
        VirtAddr::from(start),
        VirtAddr::from(start + len),
        perm,
    )
}

/// munmap：解除一段映射
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    // start 和 len 都必须页对齐
    if start % PAGE_SIZE != 0 || len % PAGE_SIZE != 0 {
        return -1;
    }
    crate::task::current_munmap(VirtAddr::from(start), VirtAddr::from(start + len))
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
