#include <stddef.h>
#include <umf/providers/provider_devdax_memory.h>
#include <umf/pools/pool_jemalloc.h>
#include <umf/memory_pool.h>
#include <umf/memory_provider.h>

static umf_memory_pool_handle_t pool = NULL;

int umf_allocator_init(const char *dax_path, size_t dax_size) {
    umf_devdax_memory_provider_params_handle_t dax_params = NULL;
    umf_memory_provider_handle_t dax_provider = NULL;
    umf_jemalloc_pool_params_handle_t jemalloc_params = NULL;
    umf_result_t res;

    res = umfDevDaxMemoryProviderParamsCreate(dax_path, dax_size, &dax_params);
    if (res != UMF_RESULT_SUCCESS) return 1;
    res = umfMemoryProviderCreate(umfDevDaxMemoryProviderOps(), dax_params, &dax_provider);
    if (res != UMF_RESULT_SUCCESS) return 2;
    res = umfJemallocPoolParamsCreate(&jemalloc_params);
    if (res != UMF_RESULT_SUCCESS) return 3;
    res = umfPoolCreate(umfJemallocPoolOps(), dax_provider, jemalloc_params, 0, &pool);
    if (res != UMF_RESULT_SUCCESS) return 4;
    return 0;
}

void *umf_alloc(size_t size) {
    if (!pool) return NULL;
    return umfPoolMalloc(pool, size);
}

void umf_dealloc(void *ptr) {
    if (!pool) return;
    umfPoolFree(pool, ptr);
}