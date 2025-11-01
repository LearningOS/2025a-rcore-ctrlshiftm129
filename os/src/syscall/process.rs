//! Process management syscalls
//!
use alloc::sync::Arc;
use core::mem::size_of;

use crate::{
    fs::{open_file, OpenFlags},
    mm::{
        translated_byte_buffer, translated_refmut, translated_str, MapPermission, VPNRange,
        VirtAddr,
    },
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next, TaskControlBlock,
    },
    timer::get_time_us,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let task = current_task().unwrap();
        task.exec(all_data.as_slice());
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!(
        "kernel::pid[{}] sys_waitpid [{}]",
        current_task().unwrap().pid.0,
        pid
    );
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        // todo exit_code_ptr跨页怎么办?
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    // Translate&Copy a ptru8 array with LENGTH len to a mutable u8 Vec through page table
    let mut buffer =
        translated_byte_buffer(current_user_token(), ts as *const u8, size_of::<TimeVal>());
    let us = get_time_us();
    let tv = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let tv_bytes = unsafe {
        core::slice::from_raw_parts(&tv as *const TimeVal as *const u8, size_of::<TimeVal>())
    };
    let mut offset = 0;
    for chunk in buffer.iter_mut() {
        let copy_len = (tv_bytes.len() - offset).min(chunk.len());
        if copy_len == 0 {
            break;
        }
        chunk[..copy_len].copy_from_slice(&tv_bytes[offset..offset + copy_len]);
        offset += copy_len;
        if offset >= tv_bytes.len() {
            break;
        }
    }
    0
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel:pid[{}] sys_mmap", current_task().unwrap().pid.0);
    if prot & !0x7 != 0 || prot & 0x7 == 0 {
        return -1;
    }
    // 不存在只能写不能读
    if (prot & 0x2) != 0 && (prot & 0x1) == 0 {
        return -1;
    }
    let current_task = current_task().unwrap();
    let mut inner = current_task.inner_exclusive_access();
    let current_memset = &mut inner.memory_set;
    // start 需要映射的虚存起始地址，要求按页对齐
    let start_va = VirtAddr::from(start);
    if !start_va.aligned() {
        return -1;
    }
    let mut permission = MapPermission::U;
    if prot & 0x1 != 0 {
        permission |= MapPermission::R;
    }
    if prot & 0x2 != 0 {
        permission |= MapPermission::W;
    }
    if prot & 0x4 != 0 {
        permission |= MapPermission::X;
    }
    let end_va = VirtAddr::from(start + len);
    let start_vpn = start_va.floor();
    let end_vpn = end_va.ceil();
    let vpn_range = VPNRange::new(start_vpn, end_vpn);
    // 确保[start, start + len) 中不存在已经被映射的页
    for vpn in vpn_range {
        if let Some(pte) = current_memset.translate(vpn) {
            if pte.is_valid() {
                return -1;
            }
        }
    }
    current_memset.insert_framed_area(start_va, end_va, permission);
    0
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_munmap", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let mut inner = current_task.inner_exclusive_access();
    let current_memset = &mut inner.memory_set;
    // start 需要映射的虚存起始地址，要求按页对齐
    let start_va = VirtAddr::from(start);
    if !start_va.aligned() {
        return -1;
    }
    let end_va = VirtAddr::from(start + len);
    let start_vpn = start_va.floor();
    let end_vpn = end_va.ceil();
    let vpn_range = VPNRange::new(start_vpn, end_vpn);
    // 确保[start, start + len) 中不存在未被映射的页
    for vpn in vpn_range {
        if let Some(vpn) = current_memset.translate(vpn) {
            if !vpn.is_valid() {
                return -1;
            }
        } else {
            return -1;
        }
    }
    current_memset.munmap_vpn_range(start_vpn, end_vpn);
    0
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_spawn", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let data = app_inode.read_all();
        let current_task = current_task().unwrap();
        let new_task = Arc::new(TaskControlBlock::new(data.as_slice()));
        let new_pid = new_task.pid.0;

        let mut parrent_inner = current_task.inner_exclusive_access();
        parrent_inner.children.push(new_task.clone());
        drop(parrent_inner);
        let mut child_inner = new_task.inner_exclusive_access();
        child_inner.parent = Some(Arc::downgrade(&current_task));
        drop(child_inner);

        add_task(new_task);
        new_pid as isize
    } else {
        -1
    }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority",
        current_task().unwrap().pid.0
    );
    if prio < 2 {
        return -1;
    }
    let current_task = current_task().unwrap();
    let mut inner = current_task.inner_exclusive_access();
    inner.pass = prio as usize;
    prio
}
