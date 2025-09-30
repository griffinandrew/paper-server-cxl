use core::alloc::{GlobalAlloc, Layout};
use std::sync::Once;
use std::ptr::null_mut;

// Import your FFI bindings.
mod allocator_bindings {
    include!("umf_allocator_bindings.rs");
}


pub struct UmfGlobal;

static INIT: Once = Once::new();

unsafe impl GlobalAlloc for UmfGlobal {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        INIT.call_once(|| {
            // Set your actual dax path and size here
            allocator_bindings::umf_allocator_init(b"/dev/dax0.0\0".as_ptr() as *const i8, 266352984064);
        });
        allocator_bindings::umf_alloc(layout.size()) as *mut u8
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        allocator_bindings::umf_dealloc(ptr as *mut std::ffi::c_void)
    }
}