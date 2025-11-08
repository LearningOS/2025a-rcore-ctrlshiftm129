//! Semaphore

use crate::sync::UPSafeCell;
use crate::task::{block_current_and_run_next, current_task, wakeup_task, TaskControlBlock};
use alloc::vec::Vec;
use alloc::{collections::VecDeque, sync::Arc};

/// semaphore structure
pub struct Semaphore {
    /// semaphore inner
    pub inner: UPSafeCell<SemaphoreInner>,
}

pub struct SemaphoreInner {
    pub count: isize,
    pub wait_queue: VecDeque<Arc<TaskControlBlock>>,
    pub owners: Vec<usize>,
}

impl Semaphore {
    /// Create a new semaphore
    pub fn new(res_count: usize) -> Self {
        trace!("kernel: Semaphore::new");
        Self {
            inner: unsafe {
                UPSafeCell::new(SemaphoreInner {
                    count: res_count as isize,
                    wait_queue: VecDeque::new(),
                    owners: Vec::with_capacity(res_count),
                })
            },
        }
    }

    /// up operation of semaphore
    pub fn up(&self) {
        trace!("kernel: Semaphore::up");
        let mut inner = self.inner.exclusive_access();
        inner.count += 1;
        let tid = current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid;
        // 可能一个线程会持有多个相同的信号量,每次只删除第一个
        let index = inner.owners.iter().position(|t| *t == tid);
        if let Some(index) = index {
            println!("find owner{}", index);
            inner.owners.remove(index);
        } else {
            println!("not find owner");
        }
        if inner.count <= 0 {
            if let Some(task) = inner.wait_queue.pop_front() {
                let w_task = Arc::clone(&task);
                let w_task_inner = w_task.inner_exclusive_access();
                let w_task_res = w_task_inner.res.as_ref().unwrap();
                let w_task_tid = w_task_res.tid;
                inner.owners.push(w_task_tid);
                wakeup_task(task);
            }
        }
    }

    /// down operation of semaphore
    pub fn down(&self) {
        let mut inner = self.inner.exclusive_access();
        println!("kernel: Semaphore::down count{} owners{:?}", inner.count, inner.owners);
        inner.count -= 1;
        if inner.count < 0 {
            inner.wait_queue.push_back(current_task().unwrap());
            drop(inner);
            block_current_and_run_next();
        } else {
            let tid = current_task()
                .unwrap()
                .inner_exclusive_access()
                .res
                .as_ref()
                .unwrap()
                .tid;
            inner.owners.push(tid);
        }
    }

    pub fn get_owners(&self) -> Vec<usize> {
        let inner = self.inner.exclusive_access();
        inner.owners.clone()
    }

    pub fn get_count(&self) -> isize {
        let inner = self.inner.exclusive_access();
        inner.count
    }

    pub fn get_wait_tid_list(&self) -> Vec<usize> {
        let inner = self.inner.exclusive_access();
        let mut tid_list = Vec::new();
        for task in &inner.wait_queue {
            let task = Arc::clone(task);
            let task_inner = task.inner_exclusive_access();
            let task_res = task_inner.res.as_ref().unwrap();
            let task_tid = task_res.tid;
            tid_list.push(task_tid);
        }
        tid_list
    }
}
