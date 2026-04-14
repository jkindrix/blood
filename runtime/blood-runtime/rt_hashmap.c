// Generic HashMap runtime for Blood
// Type-erased: key_size, val_size, key_kind passed per call.
// key_kind: 0 = primitive (hash/compare raw bytes), 1 = String
//
// HashMap layout: { ptr buckets, i64 len, i64 cap } = 24 bytes
// Bucket layout: [1 byte status][key_size bytes][val_size bytes]
// Status: 0=Empty, 1=Occupied, 2=Deleted

#include <stdint.h>
#include <string.h>
#include <stdlib.h>

#define EMPTY    0
#define OCCUPIED 1
#define DELETED  2
#define MIN_CAP  16

#define FNV_OFF 14695981039346656037ULL
#define FNV_PR  1099511628211ULL

// Field accessors
static int64_t* map_data_ptr(void* m) { return (int64_t*)m; }
static int64_t* map_len_ptr(void* m)  { return (int64_t*)((char*)m + 8); }
static int64_t* map_cap_ptr(void* m)  { return (int64_t*)((char*)m + 16); }

static int64_t stride(int64_t ks, int64_t vs) { return 1 + ks + vs; }

static uint8_t* bucket_at(void* data, int64_t idx, int64_t st) {
    return (uint8_t*)data + idx * st;
}

// Hash
static uint64_t hash_raw(const void* p, int64_t size) {
    uint64_t h = FNV_OFF;
    const uint8_t* b = (const uint8_t*)p;
    for (int64_t i = 0; i < size; i++) {
        h ^= b[i];
        h *= FNV_PR;
    }
    return h;
}

static uint64_t hash_string_key(const void* key_ptr) {
    // String struct: { ptr data, i64 len, i64 cap }
    const char* data = *(const char**)key_ptr;
    int64_t len = *(const int64_t*)((const char*)key_ptr + 8);
    return hash_raw(data, len);
}

static uint64_t hash_key(const void* key, int64_t ks, int32_t kk) {
    return kk == 1 ? hash_string_key(key) : hash_raw(key, ks);
}

// Equality
static int keys_eq(const void* a, const void* b, int64_t ks, int32_t kk) {
    if (kk == 1) {
        const char* ad = *(const char**)a;
        int64_t al = *(const int64_t*)((const char*)a + 8);
        const char* bd = *(const char**)b;
        int64_t bl = *(const int64_t*)((const char*)b + 8);
        if (al != bl) return 0;
        return memcmp(ad, bd, al) == 0;
    }
    return memcmp(a, b, ks) == 0;
}

// Resize
static void do_resize(void* m, int64_t ks, int64_t vs, int32_t kk) {
    int64_t old_cap = *map_cap_ptr(m);
    void* old_data = (void*)*map_data_ptr(m);
    int64_t st = stride(ks, vs);
    int64_t new_cap = old_cap * 2;
    void* new_data = calloc(new_cap, st);

    for (int64_t i = 0; i < old_cap; i++) {
        uint8_t* b = bucket_at(old_data, i, st);
        if (b[0] == OCCUPIED) {
            void* k = b + 1;
            void* v = b + 1 + ks;
            uint64_t h = hash_key(k, ks, kk);
            int64_t idx = (int64_t)(h % (uint64_t)new_cap);
            while (1) {
                uint8_t* nb = bucket_at(new_data, idx, st);
                if (nb[0] == EMPTY) {
                    nb[0] = OCCUPIED;
                    memcpy(nb + 1, k, ks);
                    memcpy(nb + 1 + ks, v, vs);
                    break;
                }
                idx = (idx + 1) % new_cap;
            }
        }
    }
    free(old_data);
    *map_data_ptr(m) = (int64_t)(intptr_t)new_data;
    *map_cap_ptr(m) = new_cap;
}

static void ensure_cap(void* m, int64_t ks, int64_t vs, int32_t kk) {
    int64_t cap = *map_cap_ptr(m);
    if (cap == 0) {
        int64_t st = stride(ks, vs);
        void* data = calloc(MIN_CAP, st);
        *map_data_ptr(m) = (int64_t)(intptr_t)data;
        *map_cap_ptr(m) = MIN_CAP;
        return;
    }
    if (*map_len_ptr(m) * 4 >= cap * 3) {
        do_resize(m, ks, vs, kk);
    }
}

// Exported functions

void hashmap_new(int64_t ks, int64_t vs, int32_t kk, void* out) {
    *map_data_ptr(out) = 0;
    *map_len_ptr(out) = 0;
    *map_cap_ptr(out) = 0;
}

int32_t hashmap_insert(void* m, void* key, void* val,
                       int64_t ks, int64_t vs, int32_t kk,
                       void* old_out) {
    ensure_cap(m, ks, vs, kk);
    void* data = (void*)*map_data_ptr(m);
    int64_t cap = *map_cap_ptr(m);
    int64_t st = stride(ks, vs);
    uint64_t h = hash_key(key, ks, kk);
    int64_t idx = (int64_t)(h % (uint64_t)cap);
    int64_t first_del = -1;

    for (int64_t p = 0; p < cap; p++) {
        uint8_t* b = bucket_at(data, idx, st);
        if (b[0] == EMPTY) {
            uint8_t* ins = first_del >= 0 ? bucket_at(data, first_del, st) : b;
            ins[0] = OCCUPIED;
            memcpy(ins + 1, key, ks);
            memcpy(ins + 1 + ks, val, vs);
            (*map_len_ptr(m))++;
            return 0;
        }
        if (b[0] == DELETED && first_del < 0) first_del = idx;
        if (b[0] == OCCUPIED && keys_eq(b + 1, key, ks, kk)) {
            memcpy(old_out, b + 1 + ks, vs);
            memcpy(b + 1 + ks, val, vs);
            return 1;
        }
        idx = (idx + 1) % cap;
    }
    return 0;
}

int32_t hashmap_get(void* m, void* key,
                    int64_t ks, int64_t vs, int32_t kk,
                    void* val_out) {
    int64_t cap = *map_cap_ptr(m);
    if (cap == 0) return 0;
    void* data = (void*)*map_data_ptr(m);
    int64_t st = stride(ks, vs);
    uint64_t h = hash_key(key, ks, kk);
    int64_t idx = (int64_t)(h % (uint64_t)cap);

    for (int64_t p = 0; p < cap; p++) {
        uint8_t* b = bucket_at(data, idx, st);
        if (b[0] == EMPTY) return 0;
        if (b[0] == OCCUPIED && keys_eq(b + 1, key, ks, kk)) {
            memcpy(val_out, b + 1 + ks, vs);
            return 1;
        }
        idx = (idx + 1) % cap;
    }
    return 0;
}

int32_t hashmap_contains_key(void* m, void* key,
                             int64_t ks, int64_t vs, int32_t kk) {
    int64_t cap = *map_cap_ptr(m);
    if (cap == 0) return 0;
    void* data = (void*)*map_data_ptr(m);
    int64_t st = stride(ks, vs);
    uint64_t h = hash_key(key, ks, kk);
    int64_t idx = (int64_t)(h % (uint64_t)cap);

    for (int64_t p = 0; p < cap; p++) {
        uint8_t* b = bucket_at(data, idx, st);
        if (b[0] == EMPTY) return 0;
        if (b[0] == OCCUPIED && keys_eq(b + 1, key, ks, kk)) return 1;
        idx = (idx + 1) % cap;
    }
    return 0;
}

int64_t hashmap_len(void* m) { return *map_len_ptr(m); }
int32_t hashmap_is_empty(void* m) { return *map_len_ptr(m) == 0 ? 1 : 0; }

int32_t hashmap_remove(void* m, void* key,
                       int64_t ks, int64_t vs, int32_t kk,
                       void* val_out) {
    int64_t cap = *map_cap_ptr(m);
    if (cap == 0) return 0;
    void* data = (void*)*map_data_ptr(m);
    int64_t st = stride(ks, vs);
    uint64_t h = hash_key(key, ks, kk);
    int64_t idx = (int64_t)(h % (uint64_t)cap);

    for (int64_t p = 0; p < cap; p++) {
        uint8_t* b = bucket_at(data, idx, st);
        if (b[0] == EMPTY) return 0;
        if (b[0] == OCCUPIED && keys_eq(b + 1, key, ks, kk)) {
            memcpy(val_out, b + 1 + ks, vs);
            b[0] = DELETED;
            (*map_len_ptr(m))--;
            return 1;
        }
        idx = (idx + 1) % cap;
    }
    return 0;
}

// Iterator support
// Iterator layout: { ptr map, i64 index } = 16 bytes
// map: pointer to the HashMap struct (not the bucket array)
// index: current scan position in the bucket array

static void** iter_map_ptr(void* it) { return (void**)it; }
static int64_t* iter_idx_ptr(void* it) { return (int64_t*)((char*)it + 8); }

void hashmap_keys_new(void* map, int64_t ks, int64_t vs, int32_t kk, void* out) {
    *iter_map_ptr(out) = map;
    *iter_idx_ptr(out) = 0;
}

int32_t hashmap_keys_next(void* it, int64_t ks, int64_t vs, int32_t kk, void* key_out) {
    void* map = *iter_map_ptr(it);
    int64_t* idx = iter_idx_ptr(it);
    int64_t cap = *map_cap_ptr(map);
    if (cap == 0) return 0;
    void* data = (void*)*map_data_ptr(map);
    int64_t st = stride(ks, vs);

    while (*idx < cap) {
        uint8_t* b = bucket_at(data, *idx, st);
        (*idx)++;
        if (b[0] == OCCUPIED) {
            memcpy(key_out, b + 1, ks);
            return 1;
        }
    }
    return 0;
}

void hashmap_values_new(void* map, int64_t ks, int64_t vs, int32_t kk, void* out) {
    *iter_map_ptr(out) = map;
    *iter_idx_ptr(out) = 0;
}

int32_t hashmap_values_next(void* it, int64_t ks, int64_t vs, int32_t kk, void* val_out) {
    void* map = *iter_map_ptr(it);
    int64_t* idx = iter_idx_ptr(it);
    int64_t cap = *map_cap_ptr(map);
    if (cap == 0) return 0;
    void* data = (void*)*map_data_ptr(map);
    int64_t st = stride(ks, vs);

    while (*idx < cap) {
        uint8_t* b = bucket_at(data, *idx, st);
        (*idx)++;
        if (b[0] == OCCUPIED) {
            memcpy(val_out, b + 1 + ks, vs);
            return 1;
        }
    }
    return 0;
}

void hashmap_clone(void* src, int64_t ks, int64_t vs, int32_t kk, void* out) {
    int64_t cap = *map_cap_ptr(src);
    int64_t len = *map_len_ptr(src);
    if (cap == 0) {
        *map_data_ptr(out) = 0;
        *map_len_ptr(out) = 0;
        *map_cap_ptr(out) = 0;
        return;
    }
    int64_t st = stride(ks, vs);
    void* new_data = malloc(cap * st);
    memcpy(new_data, (void*)*map_data_ptr(src), cap * st);
    *map_data_ptr(out) = (int64_t)(intptr_t)new_data;
    *map_len_ptr(out) = len;
    *map_cap_ptr(out) = cap;
}
