use std::cell::RefCell;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::rc::Rc;

struct PageSlot<T> {
    occupied: bool,
    value: MaybeUninit<T>,
}

impl<T> PageSlot<T> {
    fn vacant() -> Self {
        Self {
            occupied: false,
            value: MaybeUninit::uninit(),
        }
    }

    #[inline(always)]
    unsafe fn value_ptr(&self) -> *const T {
        self.value.as_ptr()
    }

    #[inline(always)]
    unsafe fn value_mut_ptr(&mut self) -> *mut T {
        self.value.as_mut_ptr()
    }
}

struct PagedPoolInner<T> {
    pages: Vec<Box<[PageSlot<T>]>>,
    free_slots: Vec<NonNull<PageSlot<T>>>,
    next_page_len: usize,
}

impl<T> PagedPoolInner<T> {
    fn with_page_len(page_len: usize) -> Self {
        Self {
            pages: Vec::new(),
            free_slots: Vec::new(),
            next_page_len: page_len.max(1),
        }
    }

    fn grow(&mut self) {
        let page_len = self.next_page_len;
        let mut page = Vec::with_capacity(page_len);
        page.resize_with(page_len, PageSlot::vacant);
        let mut page = page.into_boxed_slice();

        for slot in page.iter_mut().rev() {
            self.free_slots.push(NonNull::from(slot));
        }

        self.pages.push(page);
        self.next_page_len = page_len.saturating_mul(2);
    }
}

pub struct PagedPool<T> {
    inner: Rc<RefCell<PagedPoolInner<T>>>,
}

impl<T> PagedPool<T> {
    pub fn new(page_len: usize) -> Self {
        Self {
            inner: Rc::new(RefCell::new(PagedPoolInner::with_page_len(page_len))),
        }
    }

    pub fn alloc(&mut self, value: T) -> Pooled<T> {
        #[cfg(miri)]
        {
            Pooled::boxed(value)
        }

        #[cfg(not(miri))]
        {
            let slot = {
                let mut inner = self.inner.borrow_mut();
                if inner.free_slots.is_empty() {
                    inner.grow();
                }

                let slot = inner
                    .free_slots
                    .pop()
                    .expect("paged pool must provide a free slot after grow");

                unsafe {
                    let slot_ref = &mut *slot.as_ptr();
                    debug_assert!(!slot_ref.occupied, "paged pool slot already occupied");
                    slot_ref.occupied = true;
                    slot_ref.value_mut_ptr().write(value);
                }

                slot
            };

            Pooled {
                repr: PooledRepr::Slot {
                    slot,
                    pool: Rc::clone(&self.inner),
                },
            }
        }
    }
}

impl<T> Default for PagedPool<T> {
    fn default() -> Self {
        Self::new(256)
    }
}

enum PooledRepr<T> {
    Slot {
        slot: NonNull<PageSlot<T>>,
        pool: Rc<RefCell<PagedPoolInner<T>>>,
    },
    #[cfg(any(miri, feature = "shared-proto"))]
    Boxed(Box<T>),
}

pub struct Pooled<T> {
    repr: PooledRepr<T>,
}

impl<T> Pooled<T> {
    #[cfg(any(miri, feature = "shared-proto"))]
    #[inline(always)]
    pub fn boxed(value: T) -> Self {
        Self {
            repr: PooledRepr::Boxed(Box::new(value)),
        }
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *const T {
        match &self.repr {
            PooledRepr::Slot { slot, .. } => unsafe { (*slot.as_ptr()).value_ptr() },
            #[cfg(any(miri, feature = "shared-proto"))]
            PooledRepr::Boxed(value) => value.as_ref() as *const T,
        }
    }

    #[inline(always)]
    pub fn as_mut_ptr(&self) -> *mut T {
        match &self.repr {
            PooledRepr::Slot { slot, .. } => unsafe { (&mut *slot.as_ptr()).value_mut_ptr() },
            #[cfg(any(miri, feature = "shared-proto"))]
            PooledRepr::Boxed(value) => value.as_ref() as *const T as *mut T,
        }
    }
}

impl<T> AsRef<T> for Pooled<T> {
    fn as_ref(&self) -> &T {
        self.deref()
    }
}

impl<T> AsMut<T> for Pooled<T> {
    fn as_mut(&mut self) -> &mut T {
        self.deref_mut()
    }
}

impl<T> Deref for Pooled<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.as_ptr() }
    }
}

impl<T> DerefMut for Pooled<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.as_mut_ptr() }
    }
}

impl<T> Drop for Pooled<T> {
    fn drop(&mut self) {
        match &mut self.repr {
            PooledRepr::Slot { slot, pool } => {
                let mut inner = pool.borrow_mut();
                unsafe {
                    let slot_ref = &mut *slot.as_ptr();
                    if slot_ref.occupied {
                        std::ptr::drop_in_place(slot_ref.value_mut_ptr());
                        slot_ref.occupied = false;
                        inner.free_slots.push(*slot);
                    }
                }
            }
            #[cfg(any(miri, feature = "shared-proto"))]
            PooledRepr::Boxed(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PagedPool;

    #[test]
    fn stable_addresses_survive_growth() {
        let mut pool = PagedPool::new(2);
        let first = pool.alloc(1u32);
        let first_ptr = first.as_ptr();

        let mut others = Vec::new();
        for i in 0..32 {
            others.push(pool.alloc(i));
        }

        assert_eq!(first_ptr, first.as_ptr());
        assert_eq!(1, *first);
        assert_eq!(32, others.len());
    }

    #[test]
    fn freed_slots_are_reused() {
        let mut pool = PagedPool::new(1);
        let first_ptr = {
            let value = pool.alloc(7u32);
            value.as_ptr()
        };

        let reused = pool.alloc(9u32);
        assert_eq!(first_ptr, reused.as_ptr());
        assert_eq!(9, *reused);
    }
}
