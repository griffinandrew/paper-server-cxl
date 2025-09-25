use std::alloc::{GlobalAlloc, Layout};

use crate::memkind_bindings::*;

pub struct FarTierAllocator;

unsafe impl GlobalAlloc for FarTierAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        //println!("Allocating {} bytes", layout.size());
        let ptr = memkind_malloc(MEMKIND_DAX_KMEM, layout.size());
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        ptr as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        //println!("Deallocating {} bytes", layout.size());
        memkind_free(MEMKIND_DAX_KMEM, ptr as *mut _);
    }
}