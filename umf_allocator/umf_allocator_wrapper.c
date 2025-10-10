#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <umf/providers/provider_devdax_memory.h>
#include <umf/pools/pool_jemalloc.h>
#include <umf/memory_pool.h>
#include <umf/memory_provider.h>
#include <pthread.h>
#include <string.h>

static pthread_mutex_t pool_lock = PTHREAD_MUTEX_INITIALIZER;
static umf_memory_pool_handle_t pool = NULL;
static umf_memory_provider_handle_t dax_provider = NULL;
static umf_devdax_memory_provider_params_handle_t dax_params = NULL;


void umf_allocator_finalize(void) {
    if (pool) {
        umfPoolDestroy(pool);
        pool = NULL;
    }
    if (dax_provider) {
        umfMemoryProviderDestroy(dax_provider);
        dax_provider = NULL;
    }
    if (dax_params) {
        umfDevDaxMemoryProviderParamsDestroy(dax_params);
        dax_params = NULL;
    }
}

int umf_allocator_init(const char *dax_path, size_t dax_size) {
    umf_jemalloc_pool_params_handle_t jemalloc_params = NULL;
    umf_result_t res;

    res = umfDevDaxMemoryProviderParamsCreate(dax_path, dax_size, &dax_params);
    if (res != UMF_RESULT_SUCCESS) {
        fprintf(stderr, "Failed to create DAX params: %d\n", res);
        return 1;
    }

    res = umfMemoryProviderCreate(umfDevDaxMemoryProviderOps(), dax_params, &dax_provider);
    if (res != UMF_RESULT_SUCCESS) {
        fprintf(stderr, "Failed to create DAX provider: %d\n", res);
        return 2;
    }

    res = umfJemallocPoolParamsCreate(&jemalloc_params);
    if (res != UMF_RESULT_SUCCESS) {
        fprintf(stderr, "Failed to create jemalloc pool params: %d\n", res);
        return 3;
    }

    res = umfPoolCreate(umfJemallocPoolOps(), dax_provider, jemalloc_params, 0, &pool);
    umfJemallocPoolParamsDestroy(jemalloc_params);

    if (res != UMF_RESULT_SUCCESS) {
        fprintf(stderr, "Failed to create memory pool: %d\n", res);
        return 4;
    }

    // Zero all memory in the pool
    // in case of persistence.. dont think it matters for devdax tho.. 
    size_t pool_size = dax_size;
    void *base = umfPoolMalloc(pool, pool_size);
    if (base) {
        memset(base, 0, pool_size);
        umfPoolFree(pool, base);
    }

    atexit(umf_allocator_finalize);
    return 0;
}


void *umf_alloc(size_t size) {
    //pthread_mutex_lock(&pool_lock);
    if (!pool || size == 0) {
        //pthread_mutex_unlock(&pool_lock);
        fprintf(stderr, "Invalid allocation request: pool is NULL or size is 0, size=%zu\n", size);
        return NULL;
    }
    void *ptr = umfPoolMalloc(pool, size);
    //pthread_mutex_unlock(&pool_lock);
    return ptr;
}

void umf_dealloc(void *ptr) {
    //pthread_mutex_lock(&pool_lock);
    if (!pool || !ptr) {
        //pthread_mutex_unlock(&pool_lock);
        return;
    }
    umfPoolFree(pool, ptr);
    //pthread_mutex_unlock(&pool_lock);
}
