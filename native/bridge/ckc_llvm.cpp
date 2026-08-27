#include "ckc_llvm.h"

#include <cstdlib>
#include <cstring>
#include <exception>
#include <memory>
#include <string_view>
#include <vector>

#include <llvm-c/Core.h>
#include <llvm-c/TargetMachine.h>
#include <llvm/Config/llvm-config.h>
#include <llvm/ExecutionEngine/JITLink/JITLinkMemoryManager.h>
#include <llvm/ExecutionEngine/Orc/LLJIT.h>
#include <llvm/ExecutionEngine/Orc/ObjectLinkingLayer.h>
#include <llvm/ExecutionEngine/Orc/RTDyldObjectLinkingLayer.h>
#include <llvm/ExecutionEngine/SectionMemoryManager.h>
#include <llvm/IR/LLVMContext.h>
#include <llvm/IR/LegacyPassManager.h>
#include <llvm/IR/Module.h>
#include <llvm/IR/Verifier.h>
#include <llvm/ADT/SmallVector.h>
#include <llvm/Support/Error.h>
#include <llvm/Support/MemoryBuffer.h>
#include <llvm/Support/TargetSelect.h>
#include <llvm/Support/raw_ostream.h>
#include <llvm/Target/TargetMachine.h>

struct CkcLlvmContext {
    std::unique_ptr<llvm::LLVMContext> value;
};

struct CkcLlvmModule {
    std::unique_ptr<llvm::Module> value;
};

struct CkcLlvmObject {
    std::vector<uint8_t> bytes;
};

struct CkcLlvmTarget {
    std::unique_ptr<llvm::TargetMachine> value;
};

struct CkcLlvmJit {
    std::unique_ptr<llvm::orc::LLJIT> value;
    CkcLlvmOrcObjectLayer object_layer;
};

namespace {

constexpr int32_t CKC_LLVM_OK = 0;
constexpr int32_t CKC_LLVM_INVALID_ARGUMENT = 1;
constexpr int32_t CKC_LLVM_OUT_OF_MEMORY = 2;
constexpr int32_t CKC_LLVM_INTERNAL_ERROR = 3;

void clear_bytes(CkcLlvmOwnedBytes *bytes) noexcept {
    if (bytes != nullptr) {
        bytes->data = nullptr;
        bytes->len = 0;
    }
}

bool copy_bytes(std::string_view source, CkcLlvmOwnedBytes *out) noexcept {
    clear_bytes(out);
    if (source.empty()) {
        return true;
    }
    auto *data = static_cast<uint8_t *>(std::malloc(source.size()));
    if (data == nullptr) {
        return false;
    }
    std::memcpy(data, source.data(), source.size());
    out->data = data;
    out->len = source.size();
    return true;
}

int32_t set_error(CkcLlvmError *error, int32_t code,
                  std::string_view message) noexcept {
    if (error != nullptr) {
        error->code = code;
        if (!copy_bytes(message, &error->message)) {
            error->code = CKC_LLVM_OUT_OF_MEMORY;
            clear_bytes(&error->message);
            return CKC_LLVM_OUT_OF_MEMORY;
        }
    }
    return code;
}

void clear_error(CkcLlvmError *error) noexcept {
    if (error != nullptr) {
        error->code = CKC_LLVM_OK;
        clear_bytes(&error->message);
    }
}

int32_t set_llvm_error(CkcLlvmError *error, llvm::Error value) noexcept {
    return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                     llvm::toString(std::move(value)));
}

llvm::Error initialize_host_target() {
    if (llvm::InitializeNativeTarget()) {
        return llvm::createStringError("initializing native LLVM target failed");
    }
    if (llvm::InitializeNativeTargetAsmPrinter()) {
        return llvm::createStringError(
            "initializing native LLVM assembly printer failed");
    }
    return llvm::Error::success();
}

} // namespace

extern "C" int32_t ckc_llvm_bridge_info(CkcLlvmBridgeInfo *out,
                                          CkcLlvmError *error) {
    clear_error(error);
    if (out == nullptr) {
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "bridge info output is null");
    }
    out->abi_version = 0;
    clear_bytes(&out->llvm_version);
    clear_bytes(&out->host_triple);

    try {
        if (!copy_bytes(LLVM_VERSION_STRING, &out->llvm_version)) {
            return set_error(error, CKC_LLVM_OUT_OF_MEMORY,
                             "allocating LLVM version failed");
        }

        char *triple = LLVMGetDefaultTargetTriple();
        if (triple == nullptr) {
            ckc_llvm_owned_bytes_dispose(&out->llvm_version);
            return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                             "LLVM returned no host target triple");
        }
        const bool copied = copy_bytes(triple, &out->host_triple);
        LLVMDisposeMessage(triple);
        if (!copied) {
            ckc_llvm_owned_bytes_dispose(&out->llvm_version);
            return set_error(error, CKC_LLVM_OUT_OF_MEMORY,
                             "allocating LLVM host triple failed");
        }

        out->abi_version = CKC_LLVM_BRIDGE_ABI_VERSION;
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        ckc_llvm_owned_bytes_dispose(&out->llvm_version);
        ckc_llvm_owned_bytes_dispose(&out->host_triple);
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        ckc_llvm_owned_bytes_dispose(&out->llvm_version);
        ckc_llvm_owned_bytes_dispose(&out->host_triple);
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception in LLVM bridge");
    }
}

extern "C" int32_t ckc_llvm_test_error(CkcLlvmError *error) {
    clear_error(error);
    try {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "injected LLVM bridge failure");
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception in LLVM bridge test hook");
    }
}

extern "C" void ckc_llvm_owned_bytes_dispose(CkcLlvmOwnedBytes *bytes) {
    if (bytes == nullptr) {
        return;
    }
    std::free(bytes->data);
    clear_bytes(bytes);
}

extern "C" int32_t ckc_llvm_context_create(CkcLlvmContext **out,
                                             CkcLlvmError *error) {
    clear_error(error);
    if (out == nullptr) {
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "LLVM context output is null");
    }
    *out = nullptr;
    try {
        auto context = std::make_unique<CkcLlvmContext>();
        context->value = std::make_unique<llvm::LLVMContext>();
        *out = context.release();
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception creating LLVM context");
    }
}

extern "C" void ckc_llvm_context_dispose(CkcLlvmContext *context) {
    delete context;
}

extern "C" int32_t ckc_llvm_module_create_empty(CkcLlvmContext *context,
                                                  CkcLlvmModule **out,
                                                  CkcLlvmError *error) {
    clear_error(error);
    if (context == nullptr || context->value == nullptr || out == nullptr) {
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "LLVM module input or output is null");
    }
    *out = nullptr;
    try {
        auto module = std::make_unique<CkcLlvmModule>();
        module->value =
            std::make_unique<llvm::Module>("ckc", *context->value);
        *out = module.release();
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception creating LLVM module");
    }
}

extern "C" void ckc_llvm_module_dispose(CkcLlvmModule *module) {
    delete module;
}

extern "C" int32_t ckc_llvm_target_create_host(CkcLlvmTarget **out,
                                                 CkcLlvmError *error) {
    clear_error(error);
    if (out == nullptr) {
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "LLVM target output is null");
    }
    *out = nullptr;
    try {
        if (auto init_error = initialize_host_target()) {
            return set_llvm_error(error, std::move(init_error));
        }
        auto builder = llvm::orc::JITTargetMachineBuilder::detectHost();
        if (!builder) {
            return set_llvm_error(error, builder.takeError());
        }
        auto target_machine = builder->createTargetMachine();
        if (!target_machine) {
            return set_llvm_error(error, target_machine.takeError());
        }
        auto target = std::make_unique<CkcLlvmTarget>();
        target->value = std::move(*target_machine);
        *out = target.release();
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception creating LLVM target");
    }
}

extern "C" void ckc_llvm_target_dispose(CkcLlvmTarget *target) {
    delete target;
}

extern "C" int32_t ckc_llvm_target_emit_object(CkcLlvmTarget *target,
                                                 CkcLlvmModule *module,
                                                 CkcLlvmObject **out,
                                                 CkcLlvmError *error) {
    clear_error(error);
    if (target == nullptr || target->value == nullptr || module == nullptr ||
        module->value == nullptr || out == nullptr) {
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "LLVM object emission input or output is null");
    }
    *out = nullptr;
    try {
        module->value->setTargetTriple(target->value->getTargetTriple());
        module->value->setDataLayout(target->value->createDataLayout());
        std::string verification_message;
        llvm::raw_string_ostream verification_stream(verification_message);
        if (llvm::verifyModule(*module->value, &verification_stream)) {
            verification_stream.flush();
            return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                             verification_message);
        }

        llvm::SmallVector<char, 0> storage;
        llvm::raw_svector_ostream output(storage);
        llvm::legacy::PassManager passes;
        if (target->value->addPassesToEmitFile(
                passes, output, nullptr, llvm::CodeGenFileType::ObjectFile)) {
            return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                             "host target cannot emit an object file");
        }
        passes.run(*module->value);

        auto object = std::make_unique<CkcLlvmObject>();
        object->bytes.assign(storage.begin(), storage.end());
        *out = object.release();
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception emitting LLVM object");
    }
}

extern "C" size_t ckc_llvm_object_size(const CkcLlvmObject *object) {
    return object == nullptr ? 0 : object->bytes.size();
}

extern "C" void ckc_llvm_object_dispose(CkcLlvmObject *object) {
    delete object;
}

extern "C" int32_t ckc_llvm_jit_create(CkcLlvmJit **out,
                                        CkcLlvmError *error) {
    clear_error(error);
    if (out == nullptr) {
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "LLVM JIT output is null");
    }
    *out = nullptr;
    try {
        if (auto init_error = initialize_host_target()) {
            return set_llvm_error(error, std::move(init_error));
        }
        auto target_builder = llvm::orc::JITTargetMachineBuilder::detectHost();
        if (!target_builder) {
            return set_llvm_error(error, target_builder.takeError());
        }

        const llvm::Triple triple = target_builder->getTargetTriple();
        const bool use_coff_aarch64_rtdyld =
            triple.getArch() == llvm::Triple::aarch64 &&
            triple.isOSBinFormatCOFF();

        llvm::orc::LLJITBuilder builder;
        builder.setJITTargetMachineBuilder(std::move(*target_builder));
        builder.setLinkProcessSymbolsByDefault(false);
        builder.setProcessSymbolsJITDylibSetup(
            [](llvm::orc::LLJIT &jit)
                -> llvm::Expected<llvm::orc::JITDylibSP> {
                // LLVM's native platform setup requires a process-symbols
                // JITDylib. Keep the structural JITDylib, but deliberately do
                // not attach an EPCDynamicLibrarySearchGenerator: CK code may
                // only see host symbols that ckc explicitly registers later.
                return &jit.getExecutionSession().createBareJITDylib(
                    "<CK Process Symbols>");
            });
        if (use_coff_aarch64_rtdyld) {
            builder.setObjectLinkingLayerCreator(
                [](llvm::orc::ExecutionSession &session)
                    -> llvm::Expected<
                        std::unique_ptr<llvm::orc::ObjectLayer>> {
                    auto memory_manager = [](const llvm::MemoryBuffer &) {
                        return std::make_unique<llvm::SectionMemoryManager>(
                            nullptr, true);
                    };
                    return std::unique_ptr<llvm::orc::ObjectLayer>(
                        std::make_unique<llvm::orc::RTDyldObjectLinkingLayer>(
                            session, std::move(memory_manager)));
                });
        } else {
            builder.setObjectLinkingLayerCreator(
                [](llvm::orc::ExecutionSession &session)
                    -> llvm::Expected<
                        std::unique_ptr<llvm::orc::ObjectLayer>> {
                    auto memory_manager =
                        llvm::jitlink::InProcessMemoryManager::Create();
                    if (!memory_manager) {
                        return memory_manager.takeError();
                    }
                    return std::unique_ptr<llvm::orc::ObjectLayer>(
                        std::make_unique<llvm::orc::ObjectLinkingLayer>(
                            session, std::move(*memory_manager)));
                });
        }

        auto jit_value = builder.create();
        if (!jit_value) {
            return set_llvm_error(error, jit_value.takeError());
        }
        auto jit = std::make_unique<CkcLlvmJit>();
        jit->value = std::move(*jit_value);
        jit->object_layer = use_coff_aarch64_rtdyld
                                ? CKC_LLVM_ORC_RTDYLD_COFF_AARCH64
                                : CKC_LLVM_ORC_JITLINK;
        *out = jit.release();
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception creating LLVM JIT");
    }
}

extern "C" uint32_t ckc_llvm_jit_object_layer(const CkcLlvmJit *jit) {
    if (jit == nullptr) {
        return 0;
    }
    return static_cast<uint32_t>(jit->object_layer);
}

extern "C" void ckc_llvm_jit_dispose(CkcLlvmJit *jit) { delete jit; }
