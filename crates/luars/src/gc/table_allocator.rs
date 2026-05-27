use std::alloc::{self, Layout};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr;
use std::rc::Rc;

const MAX_POOLED_HASH_NODES: usize = 64;
const MAX_POOLED_ARRAY_BYTES: usize = 64 * 1024;

#[derive(Default)]
struct TableAllocInner {
    hash_free_lists: HashMap<usize, Vec<usize>>,
    array_free_lists: HashMap<usize, Vec<usize>>,
}

#[derive(Clone, Default)]
pub struct TableAllocHandle {
    inner: Rc<RefCell<TableAllocInner>>,
}

impl TableAllocHandle {
    #[inline(always)]
    pub fn alloc_hash_nodes<T>(&self, size: usize) -> *mut T {
        if size <= MAX_POOLED_HASH_NODES {
            let mut inner = self.inner.borrow_mut();
            if let Some(addr) = inner
                .hash_free_lists
                .get_mut(&size)
                .and_then(|free_list| free_list.pop())
            {
                let ptr = addr as *mut T;
                unsafe {
                    ptr::write_bytes(ptr as *mut u8, 0, std::mem::size_of::<T>() * size);
                }
                return ptr;
            }
        }

        let layout = Layout::array::<T>(size).unwrap();
        unsafe { alloc::alloc_zeroed(layout) as *mut T }
    }

    #[inline(always)]
    pub fn free_hash_nodes<T>(&self, ptr: *mut T, size: usize) {
        if size == 0 || ptr.is_null() {
            return;
        }

        if size <= MAX_POOLED_HASH_NODES {
            self.inner
                .borrow_mut()
                .hash_free_lists
                .entry(size)
                .or_default()
                .push(ptr as usize);
            return;
        }

        let layout = Layout::array::<T>(size).unwrap();
        unsafe { alloc::dealloc(ptr as *mut u8, layout) };
    }

    #[inline(always)]
    pub fn alloc_array_bytes(&self, total_size: usize, align: usize) -> *mut u8 {
        if total_size <= MAX_POOLED_ARRAY_BYTES {
            let mut inner = self.inner.borrow_mut();
            if let Some(addr) = inner
                .array_free_lists
                .get_mut(&total_size)
                .and_then(|free_list| free_list.pop())
            {
                let ptr = addr as *mut u8;
                unsafe {
                    ptr::write_bytes(ptr, 0, total_size);
                }
                return ptr;
            }
        }

        let layout = Layout::from_size_align(total_size, align).unwrap();
        unsafe { alloc::alloc_zeroed(layout) }
    }

    #[inline(always)]
    pub fn free_array_bytes(&self, ptr: *mut u8, total_size: usize, align: usize) {
        if ptr.is_null() || total_size == 0 {
            return;
        }

        if total_size <= MAX_POOLED_ARRAY_BYTES {
            self.inner
                .borrow_mut()
                .array_free_lists
                .entry(total_size)
                .or_default()
                .push(ptr as usize);
            return;
        }

        let layout = Layout::from_size_align(total_size, align).unwrap();
        unsafe { alloc::dealloc(ptr, layout) };
    }
}
