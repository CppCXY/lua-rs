use std::alloc::{self, Layout};
use std::cell::RefCell;
use std::ptr;
use std::rc::Rc;

const POOLED_HASH_NODES: usize = 4;

#[derive(Default)]
struct TableAllocInner {
    small_hash_free_list: Vec<usize>,
}

#[derive(Clone, Default)]
pub struct TableAllocHandle {
    inner: Rc<RefCell<TableAllocInner>>,
}

impl TableAllocHandle {
    #[inline(always)]
    pub fn alloc_hash_nodes<T>(&self, size: usize) -> *mut T {
        if size == POOLED_HASH_NODES
            && let Some(addr) = self.inner.borrow_mut().small_hash_free_list.pop()
        {
            let ptr = addr as *mut T;
            unsafe {
                ptr::write_bytes(ptr as *mut u8, 0, std::mem::size_of::<T>() * size);
            }
            return ptr;
        }

        let layout = Layout::array::<T>(size).unwrap();
        unsafe { alloc::alloc_zeroed(layout) as *mut T }
    }

    #[inline(always)]
    pub fn free_hash_nodes<T>(&self, ptr: *mut T, size: usize) {
        if size == 0 || ptr.is_null() {
            return;
        }

        if size == POOLED_HASH_NODES {
            self.inner.borrow_mut().small_hash_free_list.push(ptr as usize);
            return;
        }

        let layout = Layout::array::<T>(size).unwrap();
        unsafe { alloc::dealloc(ptr as *mut u8, layout) };
    }
}
