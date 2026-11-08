# Lab4 实验报告：文件系统与 I/O（ch6）

## 一、实验目的

本章把内核从 ch5 的「纯内存进程管理」升级为「有文件系统 + 磁盘 I/O」：

- 引入块设备（Virtio block 磁盘）；
- 引入文件系统（EasyFileSystem，基于 inode）；
- 进程通过文件描述符（fd）访问文件。

在此基础上完成 lab4：实现文件系统相关的系统调用。

## 二、实验内容与要求

实现 7 个系统调用（4 个重做 + 3 个新增）：

| 任务 | 说明 |
|------|------|
| `sys_get_time` / `sys_mmap` / `sys_munmap` / `sys_spawn` | 重做（ch5 实现过） |
| `sys_fstat` | 获取文件状态（mode/nlink/ino/dev） |
| `sys_linkat` | 创建硬链接（nlink +1） |
| `sys_unlinkat` | 删除链接（nlink -1） |

## 三、实现思路

### 1. 4 个简单 syscall（重做）

- `sys_get_time`：用 `translated_refmut` 写 TimeVal；
- `sys_mmap`/`sys_munmap`：页对齐 + prot 合法 + 重叠检查 + 映射/解除；
- `sys_spawn`：用 `open_file` + `read_all` 加载程序，`TaskControlBlock::new` 创建进程并建立父子关系。

### 2. 硬链接（link/unlink）—— 核心难点

easy_fs 的 `DiskInode` 原本没有 `nlink` 字段，也没有 link/unlink 方法。实现：

1. 给 `DiskInode` 加 `nlink` 字段，`initialize` 时设为 1；
2. 给 `Inode` 加 `link(old_name, new_name)`：目录追加 dirent（new_name → old inode_id），旧 inode 的 nlink +1；
3. 给 `Inode` 加 `unlink(name)`：目录删除 dirent（后面的前移），inode 的 nlink -1。

### 3. fstat

- 给 `File` trait 加 `stat()` 方法；
- `OSInode::stat()` 返回 inode 的 dev/ino/mode/nlink；
- `Stdin`/`Stdout` 返回占位 stat。

## 四、关键代码

### easy_fs：DiskInode 加 nlink

```rust
pub struct DiskInode {
    pub size: u32,
    pub direct: [u32; INODE_DIRECT_COUNT],
    pub indirect1: u32,
    pub indirect2: u32,
    pub nlink: u32,   // 新增
    type_: DiskInodeType,
}
```

### easy_fs：Inode 加 link/unlink

```rust
pub fn link(&self, old_name: &str, new_name: &str) -> bool {
    // 找 old_name 的 inode_id，目录追加 new_name -> inode_id，nlink +1
}
pub fn unlink(&self, name: &str) -> bool {
    // 找 name 的 dirent，删除（后面的前移），nlink -1
}
```

### OS：sys_fstat

```rust
pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        let stat = file.stat();
        drop(inner);
        *translated_refmut(current_user_token(), st) = stat;
        0
    } else { -1 }
}
```

## 五、实验结果

`cargo build` 编译通过。官方 checker 验证结果见评测。

## 六、思考与总结

1. **硬链接原理**：硬链接是「多个目录项指向同一个 inode」，nlink 记录指向该 inode 的目录项数量。link 增加 dirent、nlink+1；unlink 删除 dirent、nlink-1。

2. **文件系统分层**：syscall（fstat/linkat）→ OS 层（File trait）→ easy_fs（Inode）→ 块设备（Virtio block）。每层职责清晰。

3. **fd 表**：进程通过 fd 间接访问 inode，实现了「文件打开状态」与「文件本体」的分离。

## 七、参考文献

- [rCore-Tutorial-Book-v3 第 6 章](https://rcore-os.github.io/rCore-Tutorial-Book-v3/)
- [rCore-Tutorial-Guide](https://learningos.github.io/rCore-Tutorial-Guide/)
