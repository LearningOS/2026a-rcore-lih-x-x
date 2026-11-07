# Lab2 实验报告：地址空间（ch4）

## 一、实验目的

本章把内核从 ch3 的「用户程序直接用物理地址（无隔离）」升级为「每个任务拥有独立的虚拟地址空间（页表隔离）」。

核心机制：

- 每个任务拥有独立的 `MemorySet`（地址空间），通过**页表**把虚拟地址翻译成物理地址；
- 用户程序传进来的指针是**虚拟地址**，内核访问前必须先翻译 + 权限检查。

在此基础上完成 lab2 实验：在地址空间隔离的背景下，重新实现并新增一系列系统调用。

## 二、实验内容与要求

实现以下 5 项：

| 任务 | 要求 |
|------|------|
| syscall 计数 | 按任务统计每个系统调用的调用次数 |
| `sys_trace` | Read/Write 要翻译地址 + 权限检查（V/U/R/W），Syscall 返回计数 |
| `sys_get_time` | 把 `TimeVal` 写到用户地址（可能跨页） |
| `sys_mmap` | 页对齐 + prot 合法 + 不重叠 + 映射 |
| `sys_munmap` | 页对齐 + 解除映射 |

## 三、实现思路

### 核心：虚拟地址翻译

ch4 有了地址空间，用户指针是虚拟地址，不能像 ch3 那样直接解引用。为此实现了一个辅助函数 `translate_user_va`：

1. 用当前任务的 `token` 构造页表；
2. 用虚拟页号查 PTE；
3. 检查 V（有效）、U（用户）、R/W（读写权限）；
4. 翻译成物理地址。

### 1. 系统调用计数（按任务）

给 TCB 加 `syscall_times: [u32; MAX_SYSCALL_NUM]` 字段，在 `syscall()` 分发**之前**计数（这样 `sys_trace` 调用本身也被计入）。

### 2. sys_trace

- `TRACE_SYSCALL`：返回计数；
- `TRACE_READ`/`TRACE_WRITE`：先 `translate_user_va` 翻译 + 权限检查，非法返回 -1。

### 3. sys_get_time

用 `translated_byte_buffer` 把内核里的 `TimeVal` 复制到用户地址（处理可能跨页的情况）。

### 4. sys_mmap / sys_munmap

- mmap：检查页对齐、prot 合法性（bit 0/1/2 = R/W/X），转换权限，检查不重叠后映射；
- munmap：检查页对齐，找到完全匹配的 area 解除映射。

## 四、关键代码与说明

### translate_user_va（虚拟地址翻译）

```rust
fn translate_user_va(va: usize, write: bool) -> Option<PhysAddr> {
    let token = crate::task::current_user_token();
    let page_table = PageTable::from_token(token);
    let vpn = VirtAddr::from(va).floor();
    let pte = page_table.translate(vpn)?;               // 未映射 → None
    if !pte.is_valid() || !pte.flags().contains(PTEFlags::U) {
        return None;                                    // 无效/非用户 → None
    }
    if write && !pte.writable() { return None; }        // 写但无 W
    if !write && !pte.readable() { return None; }       // 读但无 R
    let pa = PhysAddr::from(pte.ppn()).0 + VirtAddr::from(va).page_offset();
    Some(PhysAddr(pa))
}
```

关键点：检查 `U` 位是隔离的体现——内核地址没有 U 位，所以 `trace_read(0x80200000)` 返回 None。

### MemorySet 新增 overlap 和 remove_area

```rust
pub fn overlap(&self, start_vpn: VirtPageNum, end_vpn: VirtPageNum) -> bool {
    self.areas.iter().any(|area| {
        let a_start = area.vpn_range.get_start();
        let a_end = area.vpn_range.get_end();
        start_vpn < a_end && a_start < end_vpn   // 区间重叠判断
    })
}

pub fn remove_area(&mut self, start: VirtAddr, end: VirtAddr) -> bool {
    // 找到 vpn_range 完全匹配的 area，unmap 并从 areas 删除
    ...
}
```

### sys_mmap 的检查

```rust
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    if start % PAGE_SIZE != 0 { return -1; }           // 页对齐
    if port & !0x7 != 0 || port == 0 { return -1; }    // prot 合法
    let mut perm = MapPermission::U;
    if port & 1 != 0 { perm |= MapPermission::R; }
    if port & 2 != 0 { perm |= MapPermission::W; }
    if port & 4 != 0 { perm |= MapPermission::X; }
    crate::task::current_mmap(VirtAddr::from(start), VirtAddr::from(start + len), perm)
}
```

## 五、实验结果

`make -C os run BASE=0` 输出：

```
Test 04_1 OK!        ← ch4_mmap0（mmap 读写）
Test trace_1 OK!     ← ch4_trace1（trace 地址翻译 + 只读页权限）
Test 04_5 ummap OK!  ← ch4_unmap（munmap）
Test 04_6 ummap2 OK! ← ch4_unmap2（munmap 页对齐）
Test 04_4 test OK!   ← ch4_mmap3（mmap 错误处理）
Test trace OK!       ← ch3_trace（重新实现的 sys_trace）
Test sleep OK!       ← ch3_sleep
```

（ch4_mmap1/2 是「应触发访存异常被杀死」的测试，不打印 OK 属预期）

## 六、思考与总结

1. **ch3 和 ch4 的本质区别**：ch3 里 `trace_read` 直接 `*(id as *const u8)` 就能读任何地址（无隔离）；ch4 里必须先翻译虚拟地址 + 检查 U/R/W 权限，读内核地址会返回 None。这就是「地址空间隔离」的直接体现。

2. **为什么检查 U 位**：`U` 标志表示「用户可访问」，内核页没有 U 位。检查 U 位才能阻止用户程序通过 `trace_read` 偷窥内核内存。

3. **translated_byte_buffer 处理跨页**：一个缓冲区可能跨越两个物理页，所以翻译结果是 `Vec<&mut [u8]>`（多个切片），需要逐段复制。

4. **mmap 的重叠检查**：映射新区域前必须检查不与已有区域重叠，否则两个区域映射到冲突的虚拟页号，会导致错误。

## 七、参考文献

- [rCore-Tutorial-Book-v3 第 4 章](https://rcore-os.github.io/rCore-Tutorial-Book-v3/)
- [rCore-Tutorial-Guide](https://learningos.github.io/rCore-Tutorial-Guide/)
