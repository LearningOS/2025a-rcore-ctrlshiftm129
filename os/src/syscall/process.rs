//! Process management syscalls
use core::{mem::size_of};

use crate::{
    mm::{translated_byte_buffer, PageTable, VirtAddr},
    task::{
        change_program_brk, current_user_token, exit_current_and_run_next,
        get_current_task_syscall_count,
        suspend_current_and_run_next, mmap, munmap
    },
    timer::get_time_us,
};

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

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    match trace_request {
        // 读取操作
        0 => {
            let va = VirtAddr::from(id);
            let vpn = va.floor();
            let page_table = PageTable::from_token(current_user_token());
            if let Some(pte) = page_table.translate(vpn) {
                if pte.user_accessible() && pte.is_valid() && pte.readable() {
                    let ppn = pte.ppn();
                    return ppn.get_bytes_array()[va.page_offset()] as isize;
                }
            }
            -1
        }
        // 写入操作
        1 => {
            let va = VirtAddr::from(id);
            let vpn = va.floor();
            let page_table = PageTable::from_token(current_user_token());
            if let Some(pte) = page_table.translate(vpn) {
                if pte.user_accessible() && pte.is_valid() && pte.writable() {
                    let ppn = pte.ppn();
                    ppn.get_bytes_array()[va.page_offset()] = data as u8;
                    return 0;
                }
            }
            -1
        }
        // 查询系统调用次数
        2 => {
            get_current_task_syscall_count(id)
        }
        // 其他情况
        _ => -1
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");
    mmap(start, len, prot)
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    munmap(start, len)
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
