int umf_allocator_init(const char *dax_path, size_t dax_size);
void *umf_alloc(size_t size);
void umf_dealloc(void *ptr);
void umf_allocator_finalize(void);