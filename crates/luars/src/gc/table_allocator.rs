use std::alloc::{self, Layout};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr;
use std::rc::Rc;

const MAX_POOLED_ARRAY_BYTES: usize = 1024;
const BLOCKS_PER_PAGE: usize = 128;

type ArrayPoolKey = (usize, usize);

struct SlabPage {
    base: usize,
    free_blocks: usize,
}

struct SlabPool {
    free_list: Vec<usize>,
    pages: Vec<SlabPage>,
    block_size: usize,
    stride: usize,
    align: usize,
}

impl SlabPool {
    fn new(block_size: usize, align: usize) -> Self {
        let stride = align_up(block_size, align);
        Self {
            free_list: Vec::new(),
            pages: Vec::new(),
            block_size,
            stride,
            align,
        }
    }

    fn alloc_zeroed(&mut self) -> *mut u8 {
        if self.free_list.is_empty() {
            self.grow();
        }

        let ptr = self
            .free_list
            .pop()
            .expect("slab pool must contain a free block after grow") as *mut u8;
        let page_index = self
            .page_index_for(ptr)
            .expect("slab block must belong to a page");
        debug_assert!(self.pages[page_index].free_blocks > 0);
        self.pages[page_index].free_blocks -= 1;
        unsafe {
            ptr::write_bytes(ptr, 0, self.block_size);
        }
        ptr
    }

    fn free(&mut self, ptr: *mut u8) {
        let page_index = self
            .page_index_for(ptr)
            .expect("slab block must belong to a page");
        self.pages[page_index].free_blocks += 1;
        self.free_list.push(ptr as usize);
    }

    fn grow(&mut self) {
        let layout = Layout::from_size_align(self.stride * BLOCKS_PER_PAGE, self.align)
            .expect("valid slab page layout");
        let page = unsafe { alloc::alloc_zeroed(layout) };
        if page.is_null() {
            alloc::handle_alloc_error(layout);
        }

        self.pages.push(SlabPage {
            base: page as usize,
            free_blocks: BLOCKS_PER_PAGE,
        });
        for index in 0..BLOCKS_PER_PAGE {
            let block = unsafe { page.add(index * self.stride) };
            self.free_list.push(block as usize);
        }
    }

    fn clear_unused_pages(&mut self) {
        let mut has_fully_free_page = false;
        let mut keep_page = Vec::with_capacity(self.pages.len());
        for page in &self.pages {
            let keep = page.free_blocks != BLOCKS_PER_PAGE;
            if !keep {
                has_fully_free_page = true;
            }
            keep_page.push(keep);
        }

        if !has_fully_free_page {
            return;
        }

        let page_span = self.stride * BLOCKS_PER_PAGE;
        let page_ranges = self
            .pages
            .iter()
            .map(|page| (page.base, page.base + page_span))
            .collect::<Vec<_>>();

        self.free_list.retain(|addr| {
            let addr = *addr;
            page_ranges
                .iter()
                .position(|(start, end)| addr >= *start && addr < *end)
                .is_some_and(|index| keep_page[index])
        });

        let layout = Layout::from_size_align(self.stride * BLOCKS_PER_PAGE, self.align)
            .expect("valid slab page layout");

        let mut new_pages = Vec::with_capacity(self.pages.len());
        for (page, keep) in self.pages.drain(..).zip(keep_page.into_iter()) {
            if keep {
                new_pages.push(page);
            } else {
                unsafe { alloc::dealloc(page.base as *mut u8, layout) };
            }
        }
        self.pages = new_pages;
    }

    fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    fn page_index_for(&self, ptr: *mut u8) -> Option<usize> {
        let ptr = ptr as usize;
        let page_span = self.stride * BLOCKS_PER_PAGE;
        self.pages.iter().position(|page| {
            let start = page.base;
            let end = start + page_span;
            ptr >= start && ptr < end
        })
    }
}

impl Drop for SlabPool {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.stride * BLOCKS_PER_PAGE, self.align)
            .expect("valid slab page layout");
        for page in self.pages.drain(..) {
            unsafe { alloc::dealloc(page.base as *mut u8, layout) };
        }
    }
}

#[derive(Default)]
struct TableAllocInner {
    array_pools: HashMap<ArrayPoolKey, SlabPool>,
}

#[derive(Clone, Default)]
pub struct TableAllocHandle {
    inner: Rc<RefCell<TableAllocInner>>,
}

impl TableAllocHandle {
    #[inline(always)]
    pub fn alloc_hash_nodes<T>(&self, size: usize) -> *mut T {
        let layout = Layout::array::<T>(size).unwrap();
        unsafe { alloc::alloc_zeroed(layout) as *mut T }
    }

    #[inline(always)]
    pub fn free_hash_nodes<T>(&self, ptr: *mut T, size: usize) {
        if size == 0 || ptr.is_null() {
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
            let pool = inner
                .array_pools
                .entry(key)
                .or_insert_with(|| SlabPool::new(total_size, align));
            return pool.alloc_zeroed();
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
            let mut inner = self.inner.borrow_mut();
            let pool = inner
                .array_pools
                .entry(key)
                .or_insert_with(|| SlabPool::new(total_size, align));
            pool.free(ptr);
            return;
        }

        let layout = Layout::from_size_align(total_size, align).unwrap();
        unsafe { alloc::dealloc(ptr, layout) };
    }

    pub fn clear_cached_blocks(&self) {
        let mut inner = self.inner.borrow_mut();

        inner.array_pools.retain(|_, pool| {
            pool.clear_unused_pages();
            !pool.is_empty()
        });
    }
}

#[inline(always)]
fn align_up(size: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (size + (align - 1)) & !(align - 1)
}
