/* 
use core::alloc::{GlobalAlloc, Layout};
use std::sync::{Once, atomic::{AtomicUsize, Ordering}};
use std::ptr;
use log::info;
use tikv_jemallocator::Jemalloc;

mod allocator_bindings {
    include!("umf_allocator_bindings.rs"); // your FFI bindings to UMF
}

/// Hybrid allocator: first DRAM up to a limit, then PMEM
pub struct HybridGlobal;

static INIT: Once = Once::new();
static DRAM_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

impl HybridGlobal {
    //const DRAM_LIMIT: usize = 1024 * 1024 * 1024; // 1 GiB

    const DRAM_LIMIT: usize = 1024 * 1024 * 500; //500 mib 



    /// Should this allocation go to DRAM or PMEM?
    fn should_use_dram(size: usize) -> bool {
        let current = DRAM_ALLOCATED.load(Ordering::Relaxed);
        current + size <= Self::DRAM_LIMIT // leave small buffer
    }
}

/// Header stored before each allocation
#[repr(C)]
struct Header {
    orig_ptr: usize, // pointer returned by backend allocator
    tag: u8,         // 0 = DRAM, 1 = PMEM
    _pad: [u8; 7],   // pad to 16 bytes for alignment
}
const HDR_SIZE: usize = std::mem::size_of::<Header>();

unsafe impl GlobalAlloc for HybridGlobal {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();

        //info!("Hybrid alloc request: size={} align={}", size, align);

        // Extra bytes for header + alignment
        let extra = HDR_SIZE + (align.saturating_sub(1));
        let total_size = size.checked_add(extra).expect("size overflow");

        let (base, tag) = if Self::should_use_dram(size) {
            //info!("Allocating in DRAM");
            let jem = Jemalloc;
            let ptr = jem.alloc(Layout::from_size_align_unchecked(total_size, align));
            if ptr.is_null() { return ptr::null_mut(); }
            DRAM_ALLOCATED.fetch_add(size, Ordering::SeqCst);
            (ptr, 0u8)
        } else {
            //info!("Allocating in PMEM");
            INIT.call_once(|| {
                allocator_bindings::umf_allocator_init(
                    b"/dev/dax0.0\0".as_ptr() as *const i8,
                    266_352_984_064, // example PMEM size
                );
            });
            let ptr = allocator_bindings::umf_alloc(total_size) as *mut u8;
            if ptr.is_null() {
                //info!("PMEM allocation failed, falling back to DRAM");
                let jem = Jemalloc;
                let ptr = jem.alloc(Layout::from_size_align_unchecked(total_size, align));
                if ptr.is_null() { return ptr::null_mut(); }
                DRAM_ALLOCATED.fetch_add(size, Ordering::SeqCst);
                (ptr, 0u8)
            } else {
                (ptr, 1u8)
            }
        };

        // Align user pointer after header
        let base_addr = base as usize;
        let user_addr = ((base_addr + HDR_SIZE + (align - 1)) / align) * align;
        let hdr_ptr = (user_addr - HDR_SIZE) as *mut Header;
        ptr::write(hdr_ptr, Header { orig_ptr: base_addr, tag, _pad: [0;7] });
        /* 
        info!(
            "Allocated: base={:#x} user={:#x} size={} tier={}",
            base_addr, user_addr, size, if tag==0 { "DRAM" } else { "PMEM" }
        );
        */

        user_addr as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() { return; }
        let hdr_ptr = (ptr as usize - HDR_SIZE) as *mut Header;
        let header = ptr::read(hdr_ptr);

        let size = layout.size();
        let align = layout.align();
        let total_size = size + HDR_SIZE + (align.saturating_sub(1));

        if header.tag == 0 {
            //info!("Deallocating DRAM: ptr={:#x} size={}", ptr as usize, size);
            let jem = Jemalloc;
            jem.dealloc(header.orig_ptr as *mut u8, Layout::from_size_align_unchecked(total_size, align));
            DRAM_ALLOCATED.fetch_sub(size, Ordering::SeqCst);
        } else {
            //info!("Deallocating PMEM: ptr={:#x} size={}", ptr as usize, size);
            allocator_bindings::umf_dealloc(header.orig_ptr as *mut std::ffi::c_void);
        }
    }
}
*/


//without alignment

use core::alloc::{GlobalAlloc, Layout};
use std::sync::{Once, atomic::{AtomicUsize, Ordering}};
use std::ptr;
use log::info;
use tikv_jemallocator::Jemalloc;

mod allocator_bindings {
    include!("umf_allocator_bindings.rs"); // your FFI bindings to UMF
}

/// Hybrid allocator: first DRAM up to a limit, then PMEM
pub struct HybridGlobal;

static INIT: Once = Once::new();
static DRAM_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

impl HybridGlobal {
    const DRAM_LIMIT: usize = 1024 * 1024 * 1024 * 2; // 1 GiB
    //const DRAM_LIMIT: usize = 1024 * 1024 * 500; //500 mib
    //const DRAM_LIMIT: usize = 1024 * 1024 * 100; //100 mib
    //const DRAM_LIMIT: usize = 1024 * 1024 * 50; //50 mib
    //const DRAM_LIMIT: usize = 1024 * 1024 * 20; //20 mib
    //const DRAM_LIMIT: usize = 1024 * 1024 * 10; //10 mib
    /// Should this allocation go to DRAM or PMEM?
    fn should_use_dram(size: usize) -> bool {
        let current = DRAM_ALLOCATED.load(Ordering::Relaxed);
        current + size <= Self::DRAM_LIMIT - 1024 * 1024 // leave small buffer
    }
}

/// Header stored before each allocation
#[repr(C)]
struct Header {
    orig_ptr: usize, // pointer returned by backend allocator
    tag: u8,         // 0 = DRAM, 1 = PMEM
    //_pad: [u8; 7],   // pad to 16 bytes for alignment
}
const HDR_SIZE: usize = std::mem::size_of::<Header>();

unsafe impl GlobalAlloc for HybridGlobal {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();

        // Total size includes header
        let total_size = size + HDR_SIZE;

        let (base, tag) = if Self::should_use_dram(size) {
            let jem = Jemalloc;
            let ptr = jem.alloc(Layout::from_size_align_unchecked(total_size, align));
            if ptr.is_null() { return ptr::null_mut(); }
            DRAM_ALLOCATED.fetch_add(size, Ordering::SeqCst);
            (ptr, 0u8)
        } else {
            INIT.call_once(|| {
                allocator_bindings::umf_allocator_init(
                    b"/dev/dax0.0\0".as_ptr() as *const i8,
                    266_352_984_064, // example PMEM size
                );
            });
            let ptr = allocator_bindings::umf_alloc(total_size) as *mut u8;
            if ptr.is_null() {
                // fallback to DRAM


                //let jem = Jemalloc;
                //let ptr = jem.alloc(Layout::from_size_align_unchecked(total_size, align));
                //if ptr.is_null() { return ptr::null_mut(); }
                //DRAM_ALLOCATED.fetch_add(size, Ordering::SeqCst);
                panic!("UMF allocation failed and no fallback");
                (ptr, 0u8)
            } else {
                (ptr, 1u8)
            }
        };

        // Store header directly at base
        let hdr_ptr = base as *mut Header;
        //ptr::write(hdr_ptr, Header { orig_ptr: base as usize, tag, _pad: [0;7] });
        ptr::write(hdr_ptr, Header { orig_ptr: base as usize, tag});

        // Return pointer immediately after header
        (base as *mut u8).add(HDR_SIZE)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() { return; }

        // Retrieve header from just before the user pointer
        let hdr_ptr = (ptr as *mut u8).sub(HDR_SIZE) as *mut Header;
        let header = ptr::read(hdr_ptr);

        let size = layout.size();
        let align = layout.align(); 
        let total_size = size + HDR_SIZE;

        //not sure if this should be total size or just requete size ... 
        //i kinda think it should be total size since that is what was allocated
        if header.tag == 0 {
            let jem = Jemalloc;
            jem.dealloc(header.orig_ptr as *mut u8, Layout::from_size_align_unchecked(total_size, align));
            DRAM_ALLOCATED.fetch_sub(size, Ordering::SeqCst);
        } else {
            allocator_bindings::umf_dealloc(header.orig_ptr as *mut std::ffi::c_void);
        }
    }
}





/*use core::alloc::{GlobalAlloc, Layout};
use std::sync::{Once, atomic::{AtomicUsize, Ordering}};
use log::info;
//use jemallocator::Jemalloc;

//#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

// Import your FFI bindings.
mod allocator_bindings {
    include!("umf_allocator_bindings.rs");
}

pub struct HybridGlobal;

static INIT: Once = Once::new();

static DRAM_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

impl HybridGlobal {
    const DRAM_LIMIT: usize = 1024 * 1024 * 1024; // 1 GB added an other 1024 so not 1gb....

    //const DRAM_LIMIT: usize = 500 * 1024 * 1024; // 500 MB

    // Tracks total allocated bytes from DRAM
    //static DRAM_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

    fn should_use_dram(size: usize) -> bool {
        let current = DRAM_ALLOCATED.load(Ordering::Relaxed);
        current + size <= Self::DRAM_LIMIT - 1024*1024 // leave 1MB buffer for UMF allocations
    }
}

unsafe impl GlobalAlloc for HybridGlobal {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        info!("Requesting allocation of size: {}", layout.size());
        if HybridGlobal::should_use_dram(layout.size()) {
            let ptr = Jemalloc.alloc(layout);
            DRAM_ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
            ptr
        } else {
            INIT.call_once(|| {
                allocator_bindings::umf_allocator_init(b"/dev/dax0.0\0".as_ptr() as *const i8, 266352984064);
            });
            let ret = allocator_bindings::umf_alloc(layout.size()) as *mut u8;
            if ret.is_null() {
                info!("UMF allocation failed");
                //Jemalloc.alloc(layout)

                ret
            } else {
                ret
            }
            
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let current = DRAM_ALLOCATED.load(Ordering::Relaxed);
        // If DRAM_ALLOCATED is still above the limit, assume PMEM, else DRAM
        if current <= HybridGlobal::DRAM_LIMIT {
            Jemalloc.dealloc(ptr, layout);
            //not sure if this should be dynamic or static? like hard allocate 1GB or dynamic based on max ram
            //DRAM_ALLOCATED.fetch_sub(layout.size(), Ordering::Relaxed);
        } else {
            allocator_bindings::umf_dealloc(ptr as *mut std::ffi::c_void);
        }
    }
}


//#[global_allocator]
//static GLOBAL: HybridGlobal = HybridGlobal;
 */