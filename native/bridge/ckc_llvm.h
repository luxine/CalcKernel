#ifndef CKC_LLVM_H
#define CKC_LLVM_H

#include <stddef.h>
#include <stdint.h>

#define CKC_LLVM_BRIDGE_ABI_VERSION 1u

#ifdef __cplusplus
extern "C" {
#endif

typedef struct CkcLlvmOwnedBytes {
    uint8_t *data;
    size_t len;
} CkcLlvmOwnedBytes;

typedef struct CkcLlvmError {
    int32_t code;
    CkcLlvmOwnedBytes message;
} CkcLlvmError;

typedef struct CkcLlvmBridgeInfo {
    uint32_t abi_version;
    CkcLlvmOwnedBytes llvm_version;
    CkcLlvmOwnedBytes host_triple;
} CkcLlvmBridgeInfo;

#if defined(__cplusplus)
static_assert(sizeof(uint32_t) == 4, "bridge requires 32-bit uint32_t");
static_assert(sizeof(int32_t) == 4, "bridge requires 32-bit int32_t");
static_assert(sizeof(CkcLlvmOwnedBytes) >= sizeof(void *) + sizeof(size_t),
              "owned byte descriptor layout is incomplete");
#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(uint32_t) == 4, "bridge requires 32-bit uint32_t");
_Static_assert(sizeof(int32_t) == 4, "bridge requires 32-bit int32_t");
_Static_assert(sizeof(CkcLlvmOwnedBytes) >= sizeof(void *) + sizeof(size_t),
               "owned byte descriptor layout is incomplete");
#endif

typedef struct CkcLlvmContext CkcLlvmContext;
typedef struct CkcLlvmModule CkcLlvmModule;
typedef struct CkcLlvmObject CkcLlvmObject;
typedef struct CkcLlvmTarget CkcLlvmTarget;
typedef struct CkcLlvmJit CkcLlvmJit;

typedef enum CkcLlvmOrcObjectLayer {
    CKC_LLVM_ORC_JITLINK = 1,
    CKC_LLVM_ORC_RTDYLD_COFF_AARCH64 = 2
} CkcLlvmOrcObjectLayer;

int32_t ckc_llvm_bridge_info(CkcLlvmBridgeInfo *out, CkcLlvmError *error);
int32_t ckc_llvm_test_error(CkcLlvmError *error);
void ckc_llvm_owned_bytes_dispose(CkcLlvmOwnedBytes *bytes);
int32_t ckc_llvm_context_create(CkcLlvmContext **out, CkcLlvmError *error);
void ckc_llvm_context_dispose(CkcLlvmContext *context);
int32_t ckc_llvm_module_create_empty(CkcLlvmContext *context,
                                     CkcLlvmModule **out,
                                     CkcLlvmError *error);
void ckc_llvm_module_dispose(CkcLlvmModule *module);
int32_t ckc_llvm_target_create_host(CkcLlvmTarget **out, CkcLlvmError *error);
void ckc_llvm_target_dispose(CkcLlvmTarget *target);
int32_t ckc_llvm_target_emit_object(CkcLlvmTarget *target,
                                    CkcLlvmModule *module,
                                    CkcLlvmObject **out,
                                    CkcLlvmError *error);
size_t ckc_llvm_object_size(const CkcLlvmObject *object);
void ckc_llvm_object_dispose(CkcLlvmObject *object);
int32_t ckc_llvm_jit_create(CkcLlvmJit **out, CkcLlvmError *error);
uint32_t ckc_llvm_jit_object_layer(const CkcLlvmJit *jit);
void ckc_llvm_jit_dispose(CkcLlvmJit *jit);

#ifdef __cplusplus
}
#endif

#endif
