use std::alloc::{self, Layout};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr;
use std::rc::Rc;

const MAX_POOLED_HASH_NODES: usize = 64;
const MAX_POOLED_ARRAY_BYTES: usize = 64 * 1024;

type HashPoolKey = (usize, usize, usize);
type ArrayPoolKey = (usize, usize);

#[derive(Default)]
struct TableAllocInner {
    hash_free_lists: HashMap<HashPoolKey, Vec<usize>>,
    array_free_lists: HashMap<ArrayPoolKey, Vec<usize>>,
}

impl Drop for TableAllocInner {
    fn drop(&mut self) {
        for ((count, elem_size, elem_align), free_list) in self.hash_free_lists.drain() {
            let layout = Layout::from_size_align(count * elem_size, elem_align)
                .expect("valid pooled hash layout");
            for addr in free_list {
                unsafe { alloc::dealloc(addr as *mut u8, layout) };
            }
        }

        for ((total_size, align), free_list) in self.array_free_lists.drain() {
            let layout =
                Layout::from_size_align(total_size, align).expect("valid pooled array layout");
            for addr in free_list {
                unsafe { alloc::dealloc(addr as *mut u8, layout) };
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct TableAllocHandle {
    inner: Rc<RefCell<TableAllocInner>>,
}

impl TableAllocHandle {
    #[inline(always)]
    pub fn alloc_hash_nodes<T>(&self, size: usize) -> *mut T {
        if size <= MAX_POOLED_HASH_NODES {
            let key = (size, std::mem::size_of::<T>(), std::mem::align_of::<T>());
            let mut inner = self.inner.borrow_mut();
            if let Some(addr) = inner
                .hash_free_lists
                .get_mut(&key)
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
            let key = (size, std::mem::size_of::<T>(), std::mem::align_of::<T>());
            self.inner
                .borrow_mut()
                .hash_free_lists
                .entry(key)
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
            let key = (total_size, align);
            let mut inner = self.inner.borrow_mut();
            if let Some(addr) = inner
                .array_free_lists
                .get_mut(&key)
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
            let key = (total_size, align);
            self.inner
                .borrow_mut()
                .array_free_lists
                .entry(key)
                .or_default()
                .push(ptr as usize);
            return;
        }

        let layout = Layout::from_size_align(total_size, align).unwrap();
        unsafe { alloc::dealloc(ptr, layout) };
    }
}
