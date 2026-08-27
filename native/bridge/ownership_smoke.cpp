#include "ckc_llvm.h"

#include <cstdio>

#ifndef CKC_OWNERSHIP_SMOKE_ITERATIONS
#define CKC_OWNERSHIP_SMOKE_ITERATIONS 32
#endif

namespace {

bool succeeded(int32_t status, CkcLlvmError &error, const char *operation) {
    if (status == 0) {
        return true;
    }
    std::fprintf(stderr, "%s failed with code %d: %.*s\n", operation,
                 error.code, static_cast<int>(error.message.len),
                 error.message.data == nullptr
                     ? ""
                     : reinterpret_cast<const char *>(error.message.data));
    ckc_llvm_owned_bytes_dispose(&error.message);
    return false;
}

} // namespace

int main() {
    CkcLlvmError error{};
    CkcLlvmBridgeInfo info{};
    if (!succeeded(ckc_llvm_bridge_info(&info, &error), error,
                   "bridge info")) {
        return 1;
    }
    ckc_llvm_owned_bytes_dispose(&info.host_triple);
    ckc_llvm_owned_bytes_dispose(&info.llvm_version);

    if (ckc_llvm_context_create(nullptr, &error) != 1) {
        std::fputs("null context output was not rejected\n", stderr);
        return 2;
    }
    ckc_llvm_owned_bytes_dispose(&error.message);

    for (int iteration = 0; iteration < CKC_OWNERSHIP_SMOKE_ITERATIONS;
         ++iteration) {
        CkcLlvmContext *context = nullptr;
        CkcLlvmModule *module = nullptr;
        CkcLlvmTarget *target = nullptr;
        CkcLlvmObject *object = nullptr;
        CkcLlvmJit *jit = nullptr;
        if (!succeeded(ckc_llvm_context_create(&context, &error), error,
                       "context create") ||
            !succeeded(ckc_llvm_module_create_empty(context, &module, &error),
                       error, "module create") ||
            !succeeded(ckc_llvm_target_create_host(&target, &error), error,
                       "target create") ||
            !succeeded(ckc_llvm_target_emit_object(target, module, &object,
                                                   &error),
                       error, "object emit") ||
            !succeeded(ckc_llvm_jit_create(&jit, &error), error,
                       "JIT create")) {
            ckc_llvm_jit_dispose(jit);
            ckc_llvm_object_dispose(object);
            ckc_llvm_module_dispose(module);
            ckc_llvm_target_dispose(target);
            ckc_llvm_context_dispose(context);
            return 3;
        }
        if (ckc_llvm_object_size(object) == 0) {
            std::fputs("empty target object\n", stderr);
            return 4;
        }
        ckc_llvm_jit_dispose(jit);
        ckc_llvm_object_dispose(object);
        ckc_llvm_module_dispose(module);
        ckc_llvm_target_dispose(target);
        ckc_llvm_context_dispose(context);
    }

    return 0;
}
