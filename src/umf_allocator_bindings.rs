
unsafe extern "C" {
    pub fn umf_allocator_init(dax_path: *const libc::c_char, dax_size: usize) -> libc::c_int;
    pub fn umf_alloc(size: usize) -> *mut libc::c_void;
    pub fn umf_dealloc(ptr: *mut libc::c_void);
}

