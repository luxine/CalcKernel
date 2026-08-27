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
typedef struct CkcLlvmBuilder CkcLlvmBuilder;
typedef struct CkcLlvmType CkcLlvmType;
typedef struct CkcLlvmValue CkcLlvmValue;
typedef struct CkcLlvmFunction CkcLlvmFunction;
typedef struct CkcLlvmBlock CkcLlvmBlock;

typedef struct CkcLlvmBytes {
    const uint8_t *data;
    size_t len;
} CkcLlvmBytes;

typedef enum CkcLlvmBinaryOp {
    CKC_LLVM_ADD = 1,
    CKC_LLVM_SUB = 2,
    CKC_LLVM_MUL = 3,
    CKC_LLVM_SDIV = 4,
    CKC_LLVM_UDIV = 5,
    CKC_LLVM_SREM = 6,
    CKC_LLVM_UREM = 7,
    CKC_LLVM_FADD = 8,
    CKC_LLVM_FSUB = 9,
    CKC_LLVM_FMUL = 10,
    CKC_LLVM_FDIV = 11
} CkcLlvmBinaryOp;

typedef enum CkcLlvmUnaryOp {
    CKC_LLVM_NEG = 1,
    CKC_LLVM_FNEG = 2,
    CKC_LLVM_NOT = 3
} CkcLlvmUnaryOp;

typedef enum CkcLlvmCompareOp {
    CKC_LLVM_ICMP_EQ = 1,
    CKC_LLVM_ICMP_NE = 2,
    CKC_LLVM_ICMP_SLT = 3,
    CKC_LLVM_ICMP_SLE = 4,
    CKC_LLVM_ICMP_SGT = 5,
    CKC_LLVM_ICMP_SGE = 6,
    CKC_LLVM_ICMP_ULT = 7,
    CKC_LLVM_ICMP_ULE = 8,
    CKC_LLVM_ICMP_UGT = 9,
    CKC_LLVM_ICMP_UGE = 10,
    CKC_LLVM_FCMP_OEQ = 11,
    CKC_LLVM_FCMP_UNE = 12,
    CKC_LLVM_FCMP_OLT = 13,
    CKC_LLVM_FCMP_OLE = 14,
    CKC_LLVM_FCMP_OGT = 15,
    CKC_LLVM_FCMP_OGE = 16
} CkcLlvmCompareOp;

typedef enum CkcLlvmCastOp {
    CKC_LLVM_SEXT = 1,
    CKC_LLVM_ZEXT = 2,
    CKC_LLVM_SITOFP = 3,
    CKC_LLVM_UITOFP = 4,
    CKC_LLVM_INTTOPTR = 5
} CkcLlvmCastOp;

typedef enum CkcLlvmCpuPolicy {
    CKC_LLVM_CPU_BASELINE = 1,
    CKC_LLVM_CPU_NATIVE = 2
} CkcLlvmCpuPolicy;

typedef enum CkcLlvmOverflowOp {
    CKC_LLVM_SADD_OVERFLOW = 1,
    CKC_LLVM_UADD_OVERFLOW = 2,
    CKC_LLVM_SSUB_OVERFLOW = 3,
    CKC_LLVM_USUB_OVERFLOW = 4,
    CKC_LLVM_SMUL_OVERFLOW = 5,
    CKC_LLVM_UMUL_OVERFLOW = 6
} CkcLlvmOverflowOp;

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
int32_t ckc_llvm_module_configure(CkcLlvmModule *module,
                                  CkcLlvmTarget *target,
                                  CkcLlvmBytes source_file_name,
                                  CkcLlvmError *error);
int32_t ckc_llvm_module_verify(CkcLlvmModule *module,
                               CkcLlvmError *error);
int32_t ckc_llvm_module_print(CkcLlvmModule *module,
                              CkcLlvmOwnedBytes *out,
                              CkcLlvmError *error);
int32_t ckc_llvm_target_create_host(uint32_t cpu_policy,
                                    CkcLlvmTarget **out,
                                    CkcLlvmError *error);
void ckc_llvm_target_dispose(CkcLlvmTarget *target);
int32_t ckc_llvm_target_triple(CkcLlvmTarget *target,
                               CkcLlvmOwnedBytes *out,
                               CkcLlvmError *error);
int32_t ckc_llvm_target_data_layout(CkcLlvmTarget *target,
                                    CkcLlvmOwnedBytes *out,
                                    CkcLlvmError *error);
int32_t ckc_llvm_target_cpu(CkcLlvmTarget *target,
                            CkcLlvmOwnedBytes *out,
                            CkcLlvmError *error);
int32_t ckc_llvm_target_features(CkcLlvmTarget *target,
                                 CkcLlvmOwnedBytes *out,
                                 CkcLlvmError *error);
int32_t ckc_llvm_module_optimize(CkcLlvmModule *module,
                                 CkcLlvmTarget *target,
                                 uint32_t opt_level,
                                 CkcLlvmError *error);
int32_t ckc_llvm_module_make_invalid_for_test(CkcLlvmModule *module,
                                               CkcLlvmError *error);

int32_t ckc_llvm_type_void(CkcLlvmContext *context, CkcLlvmType **out,
                           CkcLlvmError *error);
int32_t ckc_llvm_type_int(CkcLlvmContext *context, uint32_t bits,
                          CkcLlvmType **out, CkcLlvmError *error);
int32_t ckc_llvm_type_f64(CkcLlvmContext *context, CkcLlvmType **out,
                          CkcLlvmError *error);
int32_t ckc_llvm_type_ptr(CkcLlvmContext *context, CkcLlvmType **out,
                          CkcLlvmError *error);
int32_t ckc_llvm_type_slice(CkcLlvmContext *context, CkcLlvmType **out,
                            CkcLlvmError *error);
int32_t ckc_llvm_type_named_struct(CkcLlvmContext *context, CkcLlvmBytes name,
                                   CkcLlvmType **out,
                                   CkcLlvmError *error);
int32_t ckc_llvm_type_set_struct_body(CkcLlvmType *type,
                                      CkcLlvmType *const *fields,
                                      size_t field_count,
                                      CkcLlvmError *error);

int32_t ckc_llvm_module_add_function(CkcLlvmModule *module,
                                     CkcLlvmBytes name,
                                     CkcLlvmType *return_type,
                                     CkcLlvmType *const *params,
                                     size_t param_count,
                                     uint32_t exported,
                                     CkcLlvmFunction **out,
                                     CkcLlvmError *error);
int32_t ckc_llvm_module_preserve_function(CkcLlvmModule *module,
                                          CkcLlvmFunction *function,
                                          CkcLlvmError *error);
int32_t ckc_llvm_function_param(CkcLlvmFunction *function, size_t index,
                                CkcLlvmBytes name, CkcLlvmValue **out,
                                CkcLlvmError *error);
int32_t ckc_llvm_function_append_block(CkcLlvmFunction *function,
                                       CkcLlvmBytes name,
                                       CkcLlvmBlock **out,
                                       CkcLlvmError *error);

int32_t ckc_llvm_builder_create(CkcLlvmContext *context,
                                CkcLlvmBuilder **out,
                                CkcLlvmError *error);
void ckc_llvm_builder_dispose(CkcLlvmBuilder *builder);
int32_t ckc_llvm_builder_position(CkcLlvmBuilder *builder,
                                  CkcLlvmBlock *block,
                                  CkcLlvmError *error);
int32_t ckc_llvm_builder_alloca(CkcLlvmBuilder *builder, CkcLlvmType *type,
                                CkcLlvmBytes name, CkcLlvmValue **out,
                                CkcLlvmError *error);
int32_t ckc_llvm_builder_load(CkcLlvmBuilder *builder, CkcLlvmType *type,
                              CkcLlvmValue *pointer, CkcLlvmBytes name,
                              CkcLlvmValue **out, CkcLlvmError *error);
int32_t ckc_llvm_builder_store(CkcLlvmBuilder *builder, CkcLlvmValue *value,
                               CkcLlvmValue *pointer,
                               CkcLlvmError *error);
int32_t ckc_llvm_const_int(CkcLlvmType *type, CkcLlvmBytes text,
                           CkcLlvmValue **out, CkcLlvmError *error);
int32_t ckc_llvm_const_float(CkcLlvmType *type, CkcLlvmBytes text,
                             CkcLlvmValue **out, CkcLlvmError *error);
int32_t ckc_llvm_const_bool(CkcLlvmContext *context, uint32_t value,
                            CkcLlvmValue **out, CkcLlvmError *error);
int32_t ckc_llvm_const_undef(CkcLlvmType *type, CkcLlvmValue **out,
                             CkcLlvmError *error);
int32_t ckc_llvm_builder_binary(CkcLlvmBuilder *builder, uint32_t op,
                                CkcLlvmValue *left, CkcLlvmValue *right,
                                CkcLlvmBytes name, CkcLlvmValue **out,
                                CkcLlvmError *error);
int32_t ckc_llvm_builder_overflow(CkcLlvmBuilder *builder, uint32_t op,
                                  CkcLlvmValue *left,
                                  CkcLlvmValue *right,
                                  CkcLlvmBytes name,
                                  CkcLlvmValue **out,
                                  CkcLlvmError *error);
int32_t ckc_llvm_builder_unary(CkcLlvmBuilder *builder, uint32_t op,
                               CkcLlvmValue *value, CkcLlvmBytes name,
                               CkcLlvmValue **out, CkcLlvmError *error);
int32_t ckc_llvm_builder_compare(CkcLlvmBuilder *builder, uint32_t op,
                                 CkcLlvmValue *left, CkcLlvmValue *right,
                                 CkcLlvmBytes name, CkcLlvmValue **out,
                                 CkcLlvmError *error);
int32_t ckc_llvm_builder_cast(CkcLlvmBuilder *builder, uint32_t op,
                              CkcLlvmValue *value, CkcLlvmType *target_type,
                              CkcLlvmBytes name, CkcLlvmValue **out,
                              CkcLlvmError *error);
int32_t ckc_llvm_builder_gep(CkcLlvmBuilder *builder,
                             CkcLlvmType *element_type,
                             CkcLlvmValue *pointer,
                             CkcLlvmValue *const *indices,
                             size_t index_count, CkcLlvmBytes name,
                             CkcLlvmValue **out, CkcLlvmError *error);
int32_t ckc_llvm_builder_extract_value(CkcLlvmBuilder *builder,
                                       CkcLlvmValue *aggregate,
                                       uint32_t index, CkcLlvmBytes name,
                                       CkcLlvmValue **out,
                                       CkcLlvmError *error);
int32_t ckc_llvm_builder_insert_value(CkcLlvmBuilder *builder,
                                      CkcLlvmValue *aggregate,
                                      CkcLlvmValue *value, uint32_t index,
                                      CkcLlvmBytes name, CkcLlvmValue **out,
                                      CkcLlvmError *error);
int32_t ckc_llvm_builder_select(CkcLlvmBuilder *builder,
                                CkcLlvmValue *condition,
                                CkcLlvmValue *then_value,
                                CkcLlvmValue *else_value,
                                CkcLlvmBytes name, CkcLlvmValue **out,
                                CkcLlvmError *error);
int32_t ckc_llvm_builder_call(CkcLlvmBuilder *builder,
                              CkcLlvmFunction *function,
                              CkcLlvmValue *const *args, size_t arg_count,
                              CkcLlvmBytes name, CkcLlvmValue **out,
                              CkcLlvmError *error);
int32_t ckc_llvm_builder_return_void(CkcLlvmBuilder *builder,
                                     CkcLlvmError *error);
int32_t ckc_llvm_builder_return(CkcLlvmBuilder *builder,
                                CkcLlvmValue *value,
                                CkcLlvmError *error);
int32_t ckc_llvm_builder_branch(CkcLlvmBuilder *builder, CkcLlvmBlock *target,
                                CkcLlvmError *error);
int32_t ckc_llvm_builder_cond_branch(CkcLlvmBuilder *builder,
                                     CkcLlvmValue *condition,
                                     CkcLlvmBlock *then_block,
                                     CkcLlvmBlock *else_block,
                                     CkcLlvmError *error);
int32_t ckc_llvm_target_emit_object(CkcLlvmTarget *target,
                                    CkcLlvmModule *module,
                                    CkcLlvmObject **out,
                                    CkcLlvmError *error);
size_t ckc_llvm_object_size(const CkcLlvmObject *object);
const uint8_t *ckc_llvm_object_data(const CkcLlvmObject *object);
void ckc_llvm_object_dispose(CkcLlvmObject *object);
int32_t ckc_llvm_jit_create(CkcLlvmJit **out, CkcLlvmError *error);
uint32_t ckc_llvm_jit_object_layer(const CkcLlvmJit *jit);
void ckc_llvm_jit_dispose(CkcLlvmJit *jit);

#ifdef __cplusplus
}
#endif

#endif
