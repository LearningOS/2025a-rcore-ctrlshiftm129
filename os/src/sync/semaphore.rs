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
                })
            },
        }
    }

    /// up operation of semaphore
    pub fn up(&self) {
        trace!("kernel: Semaphore::up");
        // todo 某个线程down信号量后一定结束前会自己把信号量up?目前是这么假定的
        let mut inner = self.inner.exclusive_access();
        inner.count += 1;
        if inner.count <= 0 {
            if let Some(task) = inner.wait_queue.pop_front() {
                wakeup_task(task);
            }
        }
    }

    /// down operation of semaphore
    pub fn down(&self) {
        trace!("kernel: Semaphore::down");
        let mut inner = self.inner.exclusive_access();
        inner.count -= 1;
        if inner.count < 0 {
            inner.wait_queue.push_back(current_task().unwrap());
            drop(inner);
            block_current_and_run_next();
        }
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

    pub fn get_wait_queue_front_tid(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        if let Some(task) = inner.wait_queue.front() {
            // todo 有必要clone吗?
            let task = Arc::clone(task);
            let task_inner = task.inner_exclusive_access();
            let task_res = task_inner.res.as_ref().unwrap();
            let task_tid = task_res.tid;
            Some(task_tid)
        } else {
            None
        }
    }
}
