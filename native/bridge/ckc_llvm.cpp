#include "ckc_llvm.h"

#include <cstdlib>
#include <cstring>
#include <exception>
#include <memory>
#include <mutex>
#include <string>
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
#include <llvm/IR/Attributes.h>
#include <llvm/IR/Constants.h>
#include <llvm/IR/DerivedTypes.h>
#include <llvm/IR/Function.h>
#include <llvm/IR/IRBuilder.h>
#include <llvm/IR/Intrinsics.h>
#include <llvm/IR/LegacyPassManager.h>
#include <llvm/IR/Module.h>
#include <llvm/IR/Verifier.h>
#include <llvm/ADT/SmallVector.h>
#include <llvm/Analysis/CGSCCPassManager.h>
#include <llvm/Analysis/LoopAnalysisManager.h>
#include <llvm/Analysis/ModuleSummaryAnalysis.h>
#include <llvm/Analysis/TargetLibraryInfo.h>
#include <llvm/Object/ObjectFile.h>
#include <llvm/Object/Archive.h>
#include <llvm/Object/ArchiveWriter.h>
#include <llvm/Object/Binary.h>
#include <llvm/Object/COFF.h>
#include <llvm/Object/ELFObjectFile.h>
#include <llvm/Object/MachO.h>
#include <llvm/Passes/OptimizationLevel.h>
#include <llvm/Passes/PassBuilder.h>
#include <llvm/Support/Error.h>
#include <llvm/Support/MemoryBuffer.h>
#include <llvm/Support/TargetSelect.h>
#include <llvm/Support/raw_ostream.h>
#include <llvm/Target/TargetMachine.h>
#include <llvm/Transforms/Utils/ModuleUtils.h>
#include <lld/Common/Driver.h>

#if defined(CKC_LLD_DARWIN)
LLD_HAS_DRIVER(macho)
#elif defined(CKC_LLD_COFF)
LLD_HAS_DRIVER(coff)
#else
LLD_HAS_DRIVER(elf)
#endif

struct CkcLlvmContext {
    std::unique_ptr<llvm::LLVMContext> value;
};

struct CkcLlvmModule {
    std::unique_ptr<llvm::Module> value;
};

struct CkcLlvmObject {
    std::vector<uint8_t> bytes;
};

struct CkcLlvmArchive {
    std::vector<uint8_t> bytes;
    size_t member_count;
    bool has_symbol_index;
};

struct CkcLlvmTarget {
    std::unique_ptr<llvm::TargetMachine> value;
    std::string cpu;
    std::string features;
};

struct CkcLlvmJit {
    std::unique_ptr<llvm::orc::LLJIT> value;
    CkcLlvmOrcObjectLayer object_layer;
};

struct CkcLlvmBuilder {
    std::unique_ptr<llvm::IRBuilder<>> value;
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
    static std::once_flag once;
    static std::string failure;
    std::call_once(once, [] {
        if (llvm::InitializeNativeTarget()) {
            failure = "initializing native LLVM target failed";
            return;
        }
        if (llvm::InitializeNativeTargetAsmPrinter()) {
            failure = "initializing native LLVM assembly printer failed";
        }
    });
    if (!failure.empty()) {
        return llvm::createStringError(failure);
    }
    return llvm::Error::success();
}

llvm::StringRef borrowed_string(CkcLlvmBytes bytes) {
    if (bytes.data == nullptr) {
        return {};
    }
    return {reinterpret_cast<const char *>(bytes.data), bytes.len};
}

llvm::Type *llvm_type(CkcLlvmType *value) {
    return reinterpret_cast<llvm::Type *>(value);
}

CkcLlvmType *bridge_type(llvm::Type *value) {
    return reinterpret_cast<CkcLlvmType *>(value);
}

llvm::Value *llvm_value(CkcLlvmValue *value) {
    return reinterpret_cast<llvm::Value *>(value);
}

CkcLlvmValue *bridge_value(llvm::Value *value) {
    return reinterpret_cast<CkcLlvmValue *>(value);
}

llvm::Function *llvm_function(CkcLlvmFunction *value) {
    return reinterpret_cast<llvm::Function *>(value);
}

CkcLlvmFunction *bridge_function(llvm::Function *value) {
    return reinterpret_cast<CkcLlvmFunction *>(value);
}

llvm::BasicBlock *llvm_block(CkcLlvmBlock *value) {
    return reinterpret_cast<llvm::BasicBlock *>(value);
}

CkcLlvmBlock *bridge_block(llvm::BasicBlock *value) {
    return reinterpret_cast<CkcLlvmBlock *>(value);
}

llvm::Expected<std::string> checked_path(CkcLlvmBytes bytes,
                                         llvm::StringRef description) {
    const llvm::StringRef value = borrowed_string(bytes);
    if (value.empty() || value.contains('\0')) {
        return llvm::createStringError("%s is empty or contains NUL",
                                       description.str().c_str());
    }
    return value.str();
}

llvm::Error validate_link_input(llvm::StringRef path) {
    auto buffer = llvm::MemoryBuffer::getFile(path, false, false);
    if (!buffer) {
        return llvm::errorCodeToError(buffer.getError());
    }
    auto object = llvm::object::ObjectFile::createObjectFile(
        (*buffer)->getMemBufferRef());
    if (!object) {
        return object.takeError();
    }
    if (!(*object)->isRelocatableObject()) {
        return llvm::createStringError("LLD input is not a relocatable object");
    }
    return llvm::Error::success();
}

llvm::Error validate_shared_output(llvm::StringRef path) {
    auto binary = llvm::object::createBinary(path);
    if (!binary) {
        return binary.takeError();
    }
    auto *object = llvm::dyn_cast<llvm::object::ObjectFile>(binary->getBinary());
    if (object == nullptr || object->isRelocatableObject()) {
        return llvm::createStringError("LLD output is not a linked object");
    }
#if defined(CKC_LLD_DARWIN)
    const auto *macho = llvm::dyn_cast<llvm::object::MachOObjectFile>(object);
    if (macho == nullptr || macho->getHeader().filetype != llvm::MachO::MH_DYLIB) {
        return llvm::createStringError("LLD output is not a Mach-O dynamic library");
    }
#elif defined(CKC_LLD_COFF)
    const auto *coff = llvm::dyn_cast<llvm::object::COFFObjectFile>(object);
    if (coff == nullptr ||
        (coff->getCharacteristics() & llvm::COFF::IMAGE_FILE_DLL) == 0) {
        return llvm::createStringError("LLD output is not a PE dynamic library");
    }
#else
    const auto *elf = llvm::dyn_cast<llvm::object::ELFObjectFileBase>(object);
    if (elf == nullptr || elf->getEType() != llvm::ELF::ET_DYN) {
        return llvm::createStringError("LLD output is not an ELF shared object");
    }
#endif
    return llvm::Error::success();
}

llvm::Error validate_import_archive(llvm::StringRef path) {
    auto buffer = llvm::MemoryBuffer::getFile(path, false, false);
    if (!buffer) {
        return llvm::errorCodeToError(buffer.getError());
    }
    auto archive = llvm::object::Archive::create((*buffer)->getMemBufferRef());
    if (!archive) {
        return archive.takeError();
    }
    if (!(*archive)->hasSymbolTable() || (*archive)->isEmpty()) {
        return llvm::createStringError(
            "LLD import library is empty or lacks a symbol index");
    }
    return llvm::Error::success();
}

template <typename Action>
int32_t guarded(CkcLlvmError *error, std::string_view action,
                Action &&body) noexcept {
    clear_error(error);
    try {
        return body();
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        std::string message("unknown C++ exception ");
        message.append(action);
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, message);
    }
}

int32_t invalid(CkcLlvmError *error, std::string_view message) noexcept {
    return set_error(error, CKC_LLVM_INVALID_ARGUMENT, message);
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

extern "C" int32_t ckc_llvm_module_configure(
    CkcLlvmModule *module, CkcLlvmTarget *target,
    CkcLlvmBytes source_file_name, CkcLlvmError *error) {
    return guarded(error, "configuring LLVM module", [&] {
        if (module == nullptr || module->value == nullptr || target == nullptr ||
            target->value == nullptr) {
            return invalid(error, "LLVM module configuration input is null");
        }
        module->value->setTargetTriple(target->value->getTargetTriple());
        module->value->setDataLayout(target->value->createDataLayout());
        module->value->setSourceFileName(borrowed_string(source_file_name));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_module_verify(CkcLlvmModule *module,
                                             CkcLlvmError *error) {
    return guarded(error, "verifying LLVM module", [&] {
        if (module == nullptr || module->value == nullptr) {
            return invalid(error, "LLVM module verification input is null");
        }
        std::string message;
        llvm::raw_string_ostream stream(message);
        if (llvm::verifyModule(*module->value, &stream)) {
            stream.flush();
            return set_error(error, CKC_LLVM_INTERNAL_ERROR, message);
        }
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_module_print(CkcLlvmModule *module,
                                            CkcLlvmOwnedBytes *out,
                                            CkcLlvmError *error) {
    return guarded(error, "printing LLVM module", [&] {
        if (module == nullptr || module->value == nullptr || out == nullptr) {
            return invalid(error, "LLVM module print input or output is null");
        }
        clear_bytes(out);
        std::string text;
        llvm::raw_string_ostream stream(text);
        module->value->print(stream, nullptr);
        stream.flush();
        if (!copy_bytes(text, out)) {
            return set_error(error, CKC_LLVM_OUT_OF_MEMORY,
                             "allocating printed LLVM module failed");
        }
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_target_create_host(uint32_t cpu_policy,
                                                 CkcLlvmTarget **out,
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
        if (cpu_policy == CKC_LLVM_CPU_BASELINE) {
            switch (builder->getTargetTriple().getArch()) {
            case llvm::Triple::x86_64:
                builder->setCPU("x86-64");
                break;
            case llvm::Triple::aarch64:
                builder->setCPU("generic");
                break;
            default:
                return invalid(error, "host architecture has no CK baseline CPU");
            }
            builder->setFeatures("");
        } else if (cpu_policy != CKC_LLVM_CPU_NATIVE) {
            return invalid(error, "unknown LLVM CPU policy");
        }
        builder->setRelocationModel(llvm::Reloc::PIC_);
        auto target_machine = builder->createTargetMachine();
        if (!target_machine) {
            return set_llvm_error(error, target_machine.takeError());
        }
        auto target = std::make_unique<CkcLlvmTarget>();
        target->value = std::move(*target_machine);
        target->cpu = target->value->getTargetCPU().str();
        target->features = target->value->getTargetFeatureString().str();
        *out = target.release();
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception creating LLVM target");
    }
}

extern "C" int32_t ckc_llvm_target_cpu(CkcLlvmTarget *target,
                                          CkcLlvmOwnedBytes *out,
                                          CkcLlvmError *error) {
    return guarded(error, "reading LLVM target CPU", [&] {
        if (target == nullptr || target->value == nullptr || out == nullptr) {
            return invalid(error, "LLVM target CPU input or output is null");
        }
        return copy_bytes(target->cpu, out)
                   ? CKC_LLVM_OK
                   : set_error(error, CKC_LLVM_OUT_OF_MEMORY,
                               "allocating LLVM target CPU failed");
    });
}

extern "C" int32_t ckc_llvm_target_features(CkcLlvmTarget *target,
                                               CkcLlvmOwnedBytes *out,
                                               CkcLlvmError *error) {
    return guarded(error, "reading LLVM target features", [&] {
        if (target == nullptr || target->value == nullptr || out == nullptr) {
            return invalid(error, "LLVM target features input or output is null");
        }
        return copy_bytes(target->features, out)
                   ? CKC_LLVM_OK
                   : set_error(error, CKC_LLVM_OUT_OF_MEMORY,
                               "allocating LLVM target features failed");
    });
}

extern "C" int32_t ckc_llvm_module_optimize(
    CkcLlvmModule *module, CkcLlvmTarget *target, uint32_t opt_level,
    CkcLlvmError *error) {
    return guarded(error, "optimizing LLVM module", [&] {
        if (module == nullptr || module->value == nullptr || target == nullptr ||
            target->value == nullptr || opt_level > 3) {
            return invalid(error, "LLVM optimization input is invalid");
        }
        llvm::OptimizationLevel level = llvm::OptimizationLevel::O0;
        switch (opt_level) {
        case 0: level = llvm::OptimizationLevel::O0; break;
        case 1: level = llvm::OptimizationLevel::O1; break;
        case 2: level = llvm::OptimizationLevel::O2; break;
        case 3: level = llvm::OptimizationLevel::O3; break;
        default: llvm_unreachable("validated optimization level");
        }

        llvm::LoopAnalysisManager loop_analyses;
        llvm::FunctionAnalysisManager function_analyses;
        llvm::CGSCCAnalysisManager cgscc_analyses;
        llvm::ModuleAnalysisManager module_analyses;
        llvm::PassBuilder passes(target->value.get());
        passes.registerModuleAnalyses(module_analyses);
        passes.registerCGSCCAnalyses(cgscc_analyses);
        passes.registerFunctionAnalyses(function_analyses);
        passes.registerLoopAnalyses(loop_analyses);
        passes.crossRegisterProxies(loop_analyses, function_analyses,
                                    cgscc_analyses, module_analyses);
        auto pipeline = passes.buildPerModuleDefaultPipeline(level);
        pipeline.run(*module->value, module_analyses);

        std::string message;
        llvm::raw_string_ostream stream(message);
        if (llvm::verifyModule(*module->value, &stream)) {
            stream.flush();
            return set_error(error, CKC_LLVM_INTERNAL_ERROR, message);
        }
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_module_make_invalid_for_test(
    CkcLlvmModule *module, CkcLlvmError *error) {
    return guarded(error, "creating invalid LLVM test module", [&] {
        if (module == nullptr || module->value == nullptr) {
            return invalid(error, "invalid test module input is null");
        }
        auto &context = module->value->getContext();
        auto *type = llvm::FunctionType::get(llvm::Type::getVoidTy(context), false);
        auto *function = llvm::Function::Create(
            type, llvm::GlobalValue::ExternalLinkage, "invalid_test",
            *module->value);
        llvm::BasicBlock::Create(context, "entry", function);
        return CKC_LLVM_OK;
    });
}

extern "C" void ckc_llvm_target_dispose(CkcLlvmTarget *target) {
    delete target;
}

extern "C" int32_t ckc_llvm_target_triple(CkcLlvmTarget *target,
                                             CkcLlvmOwnedBytes *out,
                                             CkcLlvmError *error) {
    return guarded(error, "reading LLVM target triple", [&] {
        if (target == nullptr || target->value == nullptr || out == nullptr) {
            return invalid(error, "LLVM target triple input or output is null");
        }
        clear_bytes(out);
        const std::string text = target->value->getTargetTriple().str();
        return copy_bytes(text, out)
                   ? CKC_LLVM_OK
                   : set_error(error, CKC_LLVM_OUT_OF_MEMORY,
                               "allocating LLVM target triple failed");
    });
}

extern "C" int32_t ckc_llvm_target_data_layout(CkcLlvmTarget *target,
                                                  CkcLlvmOwnedBytes *out,
                                                  CkcLlvmError *error) {
    return guarded(error, "reading LLVM target data layout", [&] {
        if (target == nullptr || target->value == nullptr || out == nullptr) {
            return invalid(error,
                           "LLVM target data layout input or output is null");
        }
        clear_bytes(out);
        const std::string text = target->value->createDataLayout().getStringRepresentation();
        return copy_bytes(text, out)
                   ? CKC_LLVM_OK
                   : set_error(error, CKC_LLVM_OUT_OF_MEMORY,
                               "allocating LLVM target data layout failed");
    });
}

extern "C" int32_t ckc_llvm_type_void(CkcLlvmContext *context,
                                         CkcLlvmType **out,
                                         CkcLlvmError *error) {
    return guarded(error, "creating void type", [&] {
        if (context == nullptr || context->value == nullptr || out == nullptr) {
            return invalid(error, "void type input or output is null");
        }
        *out = bridge_type(llvm::Type::getVoidTy(*context->value));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_type_int(CkcLlvmContext *context, uint32_t bits,
                                        CkcLlvmType **out,
                                        CkcLlvmError *error) {
    return guarded(error, "creating integer type", [&] {
        if (context == nullptr || context->value == nullptr || out == nullptr ||
            bits == 0) {
            return invalid(error, "integer type input or output is invalid");
        }
        *out = bridge_type(llvm::IntegerType::get(*context->value, bits));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_type_f64(CkcLlvmContext *context,
                                        CkcLlvmType **out,
                                        CkcLlvmError *error) {
    return guarded(error, "creating f64 type", [&] {
        if (context == nullptr || context->value == nullptr || out == nullptr) {
            return invalid(error, "f64 type input or output is null");
        }
        *out = bridge_type(llvm::Type::getDoubleTy(*context->value));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_type_ptr(CkcLlvmContext *context,
                                        CkcLlvmType **out,
                                        CkcLlvmError *error) {
    return guarded(error, "creating pointer type", [&] {
        if (context == nullptr || context->value == nullptr || out == nullptr) {
            return invalid(error, "pointer type input or output is null");
        }
        *out = bridge_type(llvm::PointerType::get(*context->value, 0));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_type_slice(CkcLlvmContext *context,
                                          CkcLlvmType **out,
                                          CkcLlvmError *error) {
    return guarded(error, "creating slice type", [&] {
        if (context == nullptr || context->value == nullptr || out == nullptr) {
            return invalid(error, "slice type input or output is null");
        }
        llvm::Type *fields[] = {llvm::PointerType::get(*context->value, 0),
                                llvm::Type::getInt32Ty(*context->value)};
        *out = bridge_type(llvm::StructType::get(
            *context->value, llvm::ArrayRef<llvm::Type *>(fields)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_type_array(CkcLlvmType *element, uint32_t count,
                                          CkcLlvmType **out,
                                          CkcLlvmError *error) {
    return guarded(error, "creating array type", [&] {
        if (element == nullptr || count == 0 || out == nullptr) {
            return invalid(error, "array type input or output is invalid");
        }
        *out = bridge_type(llvm::ArrayType::get(llvm_type(element), count));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_type_struct(
    CkcLlvmContext *context, CkcLlvmType *const *fields, size_t field_count,
    CkcLlvmType **out, CkcLlvmError *error) {
    return guarded(error, "creating literal struct type", [&] {
        if (context == nullptr || context->value == nullptr || out == nullptr ||
            (field_count != 0 && fields == nullptr)) {
            return invalid(error, "literal struct type input or output is invalid");
        }
        std::vector<llvm::Type *> body;
        body.reserve(field_count);
        for (size_t index = 0; index < field_count; ++index) {
            if (fields[index] == nullptr) {
                return invalid(error, "literal struct type has a null field");
            }
            body.push_back(llvm_type(fields[index]));
        }
        *out = bridge_type(llvm::StructType::get(*context->value, body));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_type_named_struct(
    CkcLlvmContext *context, CkcLlvmBytes name, CkcLlvmType **out,
    CkcLlvmError *error) {
    return guarded(error, "creating named struct type", [&] {
        if (context == nullptr || context->value == nullptr || out == nullptr) {
            return invalid(error, "named struct type input or output is null");
        }
        *out = bridge_type(
            llvm::StructType::create(*context->value, borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_type_set_struct_body(
    CkcLlvmType *type, CkcLlvmType *const *fields, size_t field_count,
    CkcLlvmError *error) {
    return guarded(error, "setting named struct body", [&] {
        auto *structure = llvm::dyn_cast_or_null<llvm::StructType>(llvm_type(type));
        if (structure == nullptr || (field_count != 0 && fields == nullptr)) {
            return invalid(error, "struct body input is invalid");
        }
        std::vector<llvm::Type *> body;
        body.reserve(field_count);
        for (size_t index = 0; index < field_count; ++index) {
            if (fields[index] == nullptr) {
                return invalid(error, "struct body contains a null field type");
            }
            body.push_back(llvm_type(fields[index]));
        }
        structure->setBody(body);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_module_add_function(
    CkcLlvmModule *module, CkcLlvmBytes name, CkcLlvmType *return_type,
    CkcLlvmType *const *params, size_t param_count, uint32_t exported,
    CkcLlvmFunction **out, CkcLlvmError *error) {
    return guarded(error, "adding LLVM function", [&] {
        if (module == nullptr || module->value == nullptr ||
            return_type == nullptr || (param_count != 0 && params == nullptr) ||
            out == nullptr) {
            return invalid(error, "LLVM function input or output is invalid");
        }
        std::vector<llvm::Type *> types;
        types.reserve(param_count);
        for (size_t index = 0; index < param_count; ++index) {
            if (params[index] == nullptr) {
                return invalid(error, "LLVM function contains a null parameter type");
            }
            types.push_back(llvm_type(params[index]));
        }
        auto *function_type =
            llvm::FunctionType::get(llvm_type(return_type), types, false);
        auto linkage = exported != 0 ? llvm::GlobalValue::ExternalLinkage
                                     : llvm::GlobalValue::InternalLinkage;
        auto *function = llvm::Function::Create(
            function_type, linkage, borrowed_string(name), *module->value);
        *out = bridge_function(function);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_module_preserve_function(
    CkcLlvmModule *module, CkcLlvmFunction *function,
    CkcLlvmError *error) {
    return guarded(error, "preserving LLVM function", [&] {
        auto *value = llvm_function(function);
        if (module == nullptr || module->value == nullptr || value == nullptr ||
            value->getParent() != module->value.get()) {
            return invalid(error, "LLVM preserved function input is invalid");
        }
        llvm::appendToCompilerUsed(*module->value, {value});
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_function_param(
    CkcLlvmFunction *function, size_t index, CkcLlvmBytes name,
    CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "reading LLVM function parameter", [&] {
        auto *value = llvm_function(function);
        if (value == nullptr || index >= value->arg_size() || out == nullptr) {
            return invalid(error, "LLVM function parameter input is invalid");
        }
        auto *argument = value->getArg(index);
        argument->setName(borrowed_string(name));
        *out = bridge_value(argument);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_function_append_block(
    CkcLlvmFunction *function, CkcLlvmBytes name, CkcLlvmBlock **out,
    CkcLlvmError *error) {
    return guarded(error, "appending LLVM block", [&] {
        auto *value = llvm_function(function);
        if (value == nullptr || out == nullptr) {
            return invalid(error, "LLVM block input or output is null");
        }
        *out = bridge_block(llvm::BasicBlock::Create(
            value->getContext(), borrowed_string(name), value));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_function_add_attribute(
    CkcLlvmFunction *function, uint32_t kind, uint32_t return_attribute,
    size_t param_index, CkcLlvmType *pointee_type, uint32_t alignment,
    CkcLlvmError *error) {
    return guarded(error, "adding LLVM function attribute", [&] {
        auto *value = llvm_function(function);
        if (value == nullptr ||
            (return_attribute == 0 && param_index >= value->arg_size())) {
            return invalid(error, "LLVM function attribute location is invalid");
        }
        if (return_attribute != 0 &&
            kind != CKC_LLVM_ATTR_ZEROEXT && kind != CKC_LLVM_ATTR_SIGNEXT) {
            return invalid(error, "only extension attributes may apply to a return");
        }

        llvm::Attribute attribute;
        switch (kind) {
        case CKC_LLVM_ATTR_ZEROEXT:
            attribute = llvm::Attribute::get(value->getContext(), llvm::Attribute::ZExt);
            break;
        case CKC_LLVM_ATTR_SIGNEXT:
            attribute = llvm::Attribute::get(value->getContext(), llvm::Attribute::SExt);
            break;
        case CKC_LLVM_ATTR_SRET:
            if (pointee_type == nullptr || alignment == 0) {
                return invalid(error, "sret requires a pointee type and alignment");
            }
            attribute = llvm::Attribute::getWithStructRetType(
                value->getContext(), llvm_type(pointee_type));
            break;
        case CKC_LLVM_ATTR_BYVAL:
            if (pointee_type == nullptr || alignment == 0) {
                return invalid(error, "byval requires a pointee type and alignment");
            }
            attribute = llvm::Attribute::getWithByValType(
                value->getContext(), llvm_type(pointee_type));
            break;
        default:
            return invalid(error, "unknown LLVM function attribute kind");
        }

        if (return_attribute != 0) {
            value->addRetAttr(attribute);
        } else {
            value->addParamAttr(param_index, attribute);
            if (alignment != 0 &&
                (kind == CKC_LLVM_ATTR_SRET || kind == CKC_LLVM_ATTR_BYVAL)) {
                value->addParamAttr(
                    param_index,
                    llvm::Attribute::getWithAlignment(value->getContext(),
                                                      llvm::Align(alignment)));
            }
        }
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_function_set_dll_export(
    CkcLlvmFunction *function, CkcLlvmError *error) {
    return guarded(error, "setting LLVM DLL export storage", [&] {
        auto *value = llvm_function(function);
        if (value == nullptr) {
            return invalid(error, "LLVM DLL export function is null");
        }
        value->setDLLStorageClass(llvm::GlobalValue::DLLExportStorageClass);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_create(CkcLlvmContext *context,
                                              CkcLlvmBuilder **out,
                                              CkcLlvmError *error) {
    return guarded(error, "creating LLVM builder", [&] {
        if (context == nullptr || context->value == nullptr || out == nullptr) {
            return invalid(error, "LLVM builder input or output is null");
        }
        auto builder = std::make_unique<CkcLlvmBuilder>();
        builder->value = std::make_unique<llvm::IRBuilder<>>(*context->value);
        *out = builder.release();
        return CKC_LLVM_OK;
    });
}

extern "C" void ckc_llvm_builder_dispose(CkcLlvmBuilder *builder) {
    delete builder;
}

extern "C" int32_t ckc_llvm_builder_position(CkcLlvmBuilder *builder,
                                                CkcLlvmBlock *block,
                                                CkcLlvmError *error) {
    return guarded(error, "positioning LLVM builder", [&] {
        if (builder == nullptr || builder->value == nullptr || block == nullptr) {
            return invalid(error, "LLVM builder position input is null");
        }
        builder->value->SetInsertPoint(llvm_block(block));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_alloca(
    CkcLlvmBuilder *builder, CkcLlvmType *type, CkcLlvmBytes name,
    CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "building LLVM alloca", [&] {
        if (builder == nullptr || builder->value == nullptr || type == nullptr ||
            out == nullptr) {
            return invalid(error, "LLVM alloca input or output is null");
        }
        *out = bridge_value(
            builder->value->CreateAlloca(llvm_type(type), nullptr,
                                         borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_load(
    CkcLlvmBuilder *builder, CkcLlvmType *type, CkcLlvmValue *pointer,
    CkcLlvmBytes name, CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "building LLVM load", [&] {
        if (builder == nullptr || builder->value == nullptr || type == nullptr ||
            pointer == nullptr || out == nullptr) {
            return invalid(error, "LLVM load input or output is null");
        }
        *out = bridge_value(builder->value->CreateLoad(
            llvm_type(type), llvm_value(pointer), borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_store(CkcLlvmBuilder *builder,
                                             CkcLlvmValue *value,
                                             CkcLlvmValue *pointer,
                                             CkcLlvmError *error) {
    return guarded(error, "building LLVM store", [&] {
        if (builder == nullptr || builder->value == nullptr || value == nullptr ||
            pointer == nullptr) {
            return invalid(error, "LLVM store input is null");
        }
        builder->value->CreateStore(llvm_value(value), llvm_value(pointer));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_const_int(CkcLlvmType *type,
                                         CkcLlvmBytes text,
                                         CkcLlvmValue **out,
                                         CkcLlvmError *error) {
    return guarded(error, "creating LLVM integer constant", [&] {
        auto *integer = llvm::dyn_cast_or_null<llvm::IntegerType>(llvm_type(type));
        if (integer == nullptr || out == nullptr) {
            return invalid(error, "LLVM integer constant input is invalid");
        }
        llvm::APInt value(integer->getBitWidth(), borrowed_string(text), 10);
        *out = bridge_value(llvm::ConstantInt::get(integer, value));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_const_float(CkcLlvmType *type,
                                           CkcLlvmBytes text,
                                           CkcLlvmValue **out,
                                           CkcLlvmError *error) {
    return guarded(error, "creating LLVM floating constant", [&] {
        if (type == nullptr || out == nullptr || !llvm_type(type)->isDoubleTy()) {
            return invalid(error, "LLVM floating constant input is invalid");
        }
        *out = bridge_value(
            llvm::ConstantFP::get(llvm_type(type), borrowed_string(text)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_const_bool(CkcLlvmContext *context,
                                          uint32_t value,
                                          CkcLlvmValue **out,
                                          CkcLlvmError *error) {
    return guarded(error, "creating LLVM bool constant", [&] {
        if (context == nullptr || context->value == nullptr || out == nullptr) {
            return invalid(error, "LLVM bool constant input or output is null");
        }
        *out = bridge_value(llvm::ConstantInt::getBool(*context->value, value != 0));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_const_undef(CkcLlvmType *type,
                                           CkcLlvmValue **out,
                                           CkcLlvmError *error) {
    return guarded(error, "creating LLVM undef value", [&] {
        if (type == nullptr || out == nullptr) {
            return invalid(error, "LLVM undef input or output is null");
        }
        *out = bridge_value(llvm::UndefValue::get(llvm_type(type)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_binary(
    CkcLlvmBuilder *builder, uint32_t op, CkcLlvmValue *left,
    CkcLlvmValue *right, CkcLlvmBytes name, CkcLlvmValue **out,
    CkcLlvmError *error) {
    return guarded(error, "building LLVM binary instruction", [&] {
        if (builder == nullptr || builder->value == nullptr || left == nullptr ||
            right == nullptr || out == nullptr) {
            return invalid(error, "LLVM binary input or output is null");
        }
        llvm::Value *result = nullptr;
        switch (op) {
        case CKC_LLVM_ADD: result = builder->value->CreateAdd(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_SUB: result = builder->value->CreateSub(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_MUL: result = builder->value->CreateMul(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_SDIV: result = builder->value->CreateSDiv(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_UDIV: result = builder->value->CreateUDiv(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_SREM: result = builder->value->CreateSRem(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_UREM: result = builder->value->CreateURem(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_FADD: result = builder->value->CreateFAdd(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_FSUB: result = builder->value->CreateFSub(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_FMUL: result = builder->value->CreateFMul(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_FDIV: result = builder->value->CreateFDiv(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        default: return invalid(error, "unknown LLVM binary opcode");
        }
        *out = bridge_value(result);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_overflow(
    CkcLlvmBuilder *builder, uint32_t op, CkcLlvmValue *left,
    CkcLlvmValue *right, CkcLlvmBytes name, CkcLlvmValue **out,
    CkcLlvmError *error) {
    return guarded(error, "building LLVM overflow intrinsic", [&] {
        if (builder == nullptr || builder->value == nullptr || left == nullptr ||
            right == nullptr || out == nullptr) {
            return invalid(error, "LLVM overflow intrinsic input or output is null");
        }
        llvm::Intrinsic::ID intrinsic;
        switch (op) {
        case CKC_LLVM_SADD_OVERFLOW: intrinsic = llvm::Intrinsic::sadd_with_overflow; break;
        case CKC_LLVM_UADD_OVERFLOW: intrinsic = llvm::Intrinsic::uadd_with_overflow; break;
        case CKC_LLVM_SSUB_OVERFLOW: intrinsic = llvm::Intrinsic::ssub_with_overflow; break;
        case CKC_LLVM_USUB_OVERFLOW: intrinsic = llvm::Intrinsic::usub_with_overflow; break;
        case CKC_LLVM_SMUL_OVERFLOW: intrinsic = llvm::Intrinsic::smul_with_overflow; break;
        case CKC_LLVM_UMUL_OVERFLOW: intrinsic = llvm::Intrinsic::umul_with_overflow; break;
        default: return invalid(error, "unknown LLVM overflow intrinsic opcode");
        }
        *out = bridge_value(builder->value->CreateBinaryIntrinsic(
            intrinsic, llvm_value(left), llvm_value(right), nullptr,
            borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_unary(
    CkcLlvmBuilder *builder, uint32_t op, CkcLlvmValue *value,
    CkcLlvmBytes name, CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "building LLVM unary instruction", [&] {
        if (builder == nullptr || builder->value == nullptr || value == nullptr ||
            out == nullptr) {
            return invalid(error, "LLVM unary input or output is null");
        }
        llvm::Value *result = nullptr;
        switch (op) {
        case CKC_LLVM_NEG: result = builder->value->CreateNeg(llvm_value(value), borrowed_string(name)); break;
        case CKC_LLVM_FNEG: result = builder->value->CreateFNeg(llvm_value(value), borrowed_string(name)); break;
        case CKC_LLVM_NOT: result = builder->value->CreateNot(llvm_value(value), borrowed_string(name)); break;
        default: return invalid(error, "unknown LLVM unary opcode");
        }
        *out = bridge_value(result);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_compare(
    CkcLlvmBuilder *builder, uint32_t op, CkcLlvmValue *left,
    CkcLlvmValue *right, CkcLlvmBytes name, CkcLlvmValue **out,
    CkcLlvmError *error) {
    return guarded(error, "building LLVM compare instruction", [&] {
        if (builder == nullptr || builder->value == nullptr || left == nullptr ||
            right == nullptr || out == nullptr) {
            return invalid(error, "LLVM compare input or output is null");
        }
        llvm::Value *result = nullptr;
        switch (op) {
        case CKC_LLVM_ICMP_EQ: result = builder->value->CreateICmpEQ(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_ICMP_NE: result = builder->value->CreateICmpNE(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_ICMP_SLT: result = builder->value->CreateICmpSLT(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_ICMP_SLE: result = builder->value->CreateICmpSLE(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_ICMP_SGT: result = builder->value->CreateICmpSGT(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_ICMP_SGE: result = builder->value->CreateICmpSGE(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_ICMP_ULT: result = builder->value->CreateICmpULT(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_ICMP_ULE: result = builder->value->CreateICmpULE(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_ICMP_UGT: result = builder->value->CreateICmpUGT(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_ICMP_UGE: result = builder->value->CreateICmpUGE(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_FCMP_OEQ: result = builder->value->CreateFCmpOEQ(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_FCMP_UNE: result = builder->value->CreateFCmpUNE(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_FCMP_OLT: result = builder->value->CreateFCmpOLT(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_FCMP_OLE: result = builder->value->CreateFCmpOLE(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_FCMP_OGT: result = builder->value->CreateFCmpOGT(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        case CKC_LLVM_FCMP_OGE: result = builder->value->CreateFCmpOGE(llvm_value(left), llvm_value(right), borrowed_string(name)); break;
        default: return invalid(error, "unknown LLVM compare opcode");
        }
        *out = bridge_value(result);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_cast(
    CkcLlvmBuilder *builder, uint32_t op, CkcLlvmValue *value,
    CkcLlvmType *target_type, CkcLlvmBytes name, CkcLlvmValue **out,
    CkcLlvmError *error) {
    return guarded(error, "building LLVM cast instruction", [&] {
        if (builder == nullptr || builder->value == nullptr || value == nullptr ||
            target_type == nullptr || out == nullptr) {
            return invalid(error, "LLVM cast input or output is null");
        }
        llvm::Value *result = nullptr;
        switch (op) {
        case CKC_LLVM_SEXT: result = builder->value->CreateSExt(llvm_value(value), llvm_type(target_type), borrowed_string(name)); break;
        case CKC_LLVM_ZEXT: result = builder->value->CreateZExt(llvm_value(value), llvm_type(target_type), borrowed_string(name)); break;
        case CKC_LLVM_SITOFP: result = builder->value->CreateSIToFP(llvm_value(value), llvm_type(target_type), borrowed_string(name)); break;
        case CKC_LLVM_UITOFP: result = builder->value->CreateUIToFP(llvm_value(value), llvm_type(target_type), borrowed_string(name)); break;
        case CKC_LLVM_INTTOPTR: result = builder->value->CreateIntToPtr(llvm_value(value), llvm_type(target_type), borrowed_string(name)); break;
        default: return invalid(error, "unknown LLVM cast opcode");
        }
        *out = bridge_value(result);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_gep(
    CkcLlvmBuilder *builder, CkcLlvmType *element_type,
    CkcLlvmValue *pointer, CkcLlvmValue *const *indices,
    size_t index_count, CkcLlvmBytes name, CkcLlvmValue **out,
    CkcLlvmError *error) {
    return guarded(error, "building LLVM getelementptr", [&] {
        if (builder == nullptr || builder->value == nullptr ||
            element_type == nullptr || pointer == nullptr || out == nullptr ||
            (index_count != 0 && indices == nullptr)) {
            return invalid(error, "LLVM getelementptr input or output is invalid");
        }
        std::vector<llvm::Value *> values;
        values.reserve(index_count);
        for (size_t index = 0; index < index_count; ++index) {
            if (indices[index] == nullptr) {
                return invalid(error, "LLVM getelementptr has a null index");
            }
            values.push_back(llvm_value(indices[index]));
        }
        *out = bridge_value(builder->value->CreateGEP(
            llvm_type(element_type), llvm_value(pointer), values,
            borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_extract_value(
    CkcLlvmBuilder *builder, CkcLlvmValue *aggregate, uint32_t index,
    CkcLlvmBytes name, CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "building LLVM extractvalue", [&] {
        if (builder == nullptr || builder->value == nullptr ||
            aggregate == nullptr || out == nullptr) {
            return invalid(error, "LLVM extractvalue input or output is null");
        }
        *out = bridge_value(builder->value->CreateExtractValue(
            llvm_value(aggregate), {index}, borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_insert_value(
    CkcLlvmBuilder *builder, CkcLlvmValue *aggregate, CkcLlvmValue *value,
    uint32_t index, CkcLlvmBytes name, CkcLlvmValue **out,
    CkcLlvmError *error) {
    return guarded(error, "building LLVM insertvalue", [&] {
        if (builder == nullptr || builder->value == nullptr ||
            aggregate == nullptr || value == nullptr || out == nullptr) {
            return invalid(error, "LLVM insertvalue input or output is null");
        }
        *out = bridge_value(builder->value->CreateInsertValue(
            llvm_value(aggregate), llvm_value(value), {index},
            borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_select(
    CkcLlvmBuilder *builder, CkcLlvmValue *condition,
    CkcLlvmValue *then_value, CkcLlvmValue *else_value, CkcLlvmBytes name,
    CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "building LLVM select", [&] {
        if (builder == nullptr || builder->value == nullptr ||
            condition == nullptr || then_value == nullptr ||
            else_value == nullptr || out == nullptr) {
            return invalid(error, "LLVM select input or output is null");
        }
        *out = bridge_value(builder->value->CreateSelect(
            llvm_value(condition), llvm_value(then_value),
            llvm_value(else_value), borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_call(
    CkcLlvmBuilder *builder, CkcLlvmFunction *function,
    CkcLlvmValue *const *args, size_t arg_count, CkcLlvmBytes name,
    CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "building LLVM call", [&] {
        auto *callee = llvm_function(function);
        if (builder == nullptr || builder->value == nullptr || callee == nullptr ||
            out == nullptr || (arg_count != 0 && args == nullptr)) {
            return invalid(error, "LLVM call input or output is invalid");
        }
        std::vector<llvm::Value *> values;
        values.reserve(arg_count);
        for (size_t index = 0; index < arg_count; ++index) {
            if (args[index] == nullptr) {
                return invalid(error, "LLVM call has a null argument");
            }
            values.push_back(llvm_value(args[index]));
        }
        *out = bridge_value(builder->value->CreateCall(
            callee->getFunctionType(), callee, values, borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_return_void(CkcLlvmBuilder *builder,
                                                   CkcLlvmError *error) {
    return guarded(error, "building LLVM void return", [&] {
        if (builder == nullptr || builder->value == nullptr) {
            return invalid(error, "LLVM return builder is null");
        }
        builder->value->CreateRetVoid();
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_return(CkcLlvmBuilder *builder,
                                              CkcLlvmValue *value,
                                              CkcLlvmError *error) {
    return guarded(error, "building LLVM return", [&] {
        if (builder == nullptr || builder->value == nullptr || value == nullptr) {
            return invalid(error, "LLVM return input is null");
        }
        builder->value->CreateRet(llvm_value(value));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_branch(CkcLlvmBuilder *builder,
                                              CkcLlvmBlock *target,
                                              CkcLlvmError *error) {
    return guarded(error, "building LLVM branch", [&] {
        if (builder == nullptr || builder->value == nullptr || target == nullptr) {
            return invalid(error, "LLVM branch input is null");
        }
        builder->value->CreateBr(llvm_block(target));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_cond_branch(
    CkcLlvmBuilder *builder, CkcLlvmValue *condition,
    CkcLlvmBlock *then_block, CkcLlvmBlock *else_block,
    CkcLlvmError *error) {
    return guarded(error, "building LLVM conditional branch", [&] {
        if (builder == nullptr || builder->value == nullptr ||
            condition == nullptr || then_block == nullptr || else_block == nullptr) {
            return invalid(error, "LLVM conditional branch input is null");
        }
        builder->value->CreateCondBr(llvm_value(condition), llvm_block(then_block),
                                     llvm_block(else_block));
        return CKC_LLVM_OK;
    });
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

        llvm::MemoryBufferRef buffer(
            llvm::StringRef(storage.data(), storage.size()), "ckc-object");
        auto parsed = llvm::object::ObjectFile::createObjectFile(buffer);
        if (!parsed) {
            return set_llvm_error(error, parsed.takeError());
        }

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

extern "C" const uint8_t *ckc_llvm_object_data(const CkcLlvmObject *object) {
    return object == nullptr || object->bytes.empty() ? nullptr
                                                      : object->bytes.data();
}

extern "C" void ckc_llvm_object_dispose(CkcLlvmObject *object) {
    delete object;
}

extern "C" int32_t ckc_llvm_archive_create(const CkcLlvmObject *object,
                                             uint32_t kind,
                                             CkcLlvmArchive **out,
                                             CkcLlvmError *error) {
    clear_error(error);
    if (object == nullptr || object->bytes.empty() || out == nullptr) {
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "LLVM archive input or output is null");
    }
    *out = nullptr;
    try {
        llvm::object::Archive::Kind archive_kind;
        switch (kind) {
        case CKC_LLVM_ARCHIVE_GNU:
            archive_kind = llvm::object::Archive::K_GNU;
            break;
        case CKC_LLVM_ARCHIVE_DARWIN:
            archive_kind = llvm::object::Archive::K_DARWIN;
            break;
        case CKC_LLVM_ARCHIVE_COFF:
            archive_kind = llvm::object::Archive::K_COFF;
            break;
        default:
            return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                             "unknown LLVM archive kind");
        }

        const llvm::StringRef object_bytes(
            reinterpret_cast<const char *>(object->bytes.data()),
            object->bytes.size());
        auto object_buffer = llvm::MemoryBuffer::getMemBufferCopy(
            object_bytes, "ck_module.o");
        llvm::NewArchiveMember member(object_buffer->getMemBufferRef());
        member.MemberName = "ck_module.o";
        const llvm::NewArchiveMember members[] = {std::move(member)};
        auto written = llvm::writeArchiveToBuffer(
            members, llvm::SymtabWritingMode::NormalSymtab, archive_kind,
            true, false, [](llvm::Error warning) { llvm::consumeError(std::move(warning)); });
        if (!written) {
            return set_llvm_error(error, written.takeError());
        }

        auto parsed = llvm::object::Archive::create((*written)->getMemBufferRef());
        if (!parsed) {
            return set_llvm_error(error, parsed.takeError());
        }
        size_t member_count = 0;
        llvm::Error child_error = llvm::Error::success();
        for (const auto &child : (*parsed)->children(child_error)) {
            (void)child;
            ++member_count;
        }
        if (child_error) {
            return set_llvm_error(error, std::move(child_error));
        }
        if (member_count != 1 || !(*parsed)->hasSymbolTable()) {
            return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                             "LLVM produced an invalid one-member indexed archive");
        }

        auto archive = std::make_unique<CkcLlvmArchive>();
        const llvm::StringRef archive_bytes = (*written)->getBuffer();
        archive->bytes.assign(archive_bytes.bytes_begin(), archive_bytes.bytes_end());
        archive->member_count = member_count;
        archive->has_symbol_index = true;
        *out = archive.release();
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception creating LLVM archive");
    }
}

extern "C" size_t ckc_llvm_archive_size(const CkcLlvmArchive *archive) {
    return archive == nullptr ? 0 : archive->bytes.size();
}

extern "C" const uint8_t *ckc_llvm_archive_data(const CkcLlvmArchive *archive) {
    return archive == nullptr || archive->bytes.empty() ? nullptr
                                                        : archive->bytes.data();
}

extern "C" size_t
ckc_llvm_archive_member_count(const CkcLlvmArchive *archive) {
    return archive == nullptr ? 0 : archive->member_count;
}

extern "C" uint32_t
ckc_llvm_archive_has_symbol_index(const CkcLlvmArchive *archive) {
    return archive != nullptr && archive->has_symbol_index ? 1u : 0u;
}

extern "C" void ckc_llvm_archive_dispose(CkcLlvmArchive *archive) {
    delete archive;
}

extern "C" int32_t ckc_lld_link_shared(
    CkcLlvmBytes object_path_bytes, CkcLlvmBytes output_path_bytes,
    CkcLlvmBytes import_library_path_bytes, const CkcLlvmBytes *exports,
    size_t export_count, CkcLlvmError *error) {
    clear_error(error);
    try {
        auto object_path = checked_path(object_path_bytes, "LLD object path");
        if (!object_path) {
            return set_llvm_error(error, object_path.takeError());
        }
        auto output_path = checked_path(output_path_bytes, "LLD output path");
        if (!output_path) {
            return set_llvm_error(error, output_path.takeError());
        }
        if (export_count != 0 && exports == nullptr) {
            return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                             "LLD export list is null");
        }
        if (auto validation = validate_link_input(*object_path)) {
            return set_llvm_error(error, std::move(validation));
        }

        std::vector<std::string> arguments;
        arguments.reserve(14 + export_count);
#if defined(CKC_LLD_DARWIN)
        arguments.emplace_back("ld64.lld");
        arguments.emplace_back("-dylib");
        arguments.emplace_back("-arch");
#if defined(__aarch64__) || defined(__arm64__)
        arguments.emplace_back("arm64");
#else
        arguments.emplace_back("x86_64");
#endif
        arguments.emplace_back("-platform_version");
        arguments.emplace_back("macos");
        arguments.emplace_back("11.0");
        arguments.emplace_back("11.0");
        arguments.emplace_back("-adhoc_codesign");
        for (size_t index = 0; index < export_count; ++index) {
            const llvm::StringRef name = borrowed_string(exports[index]);
            if (name.empty() || name.contains('\0') || name.contains(',')) {
                return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                                 "invalid LLD export symbol");
            }
            arguments.emplace_back("-exported_symbol");
            arguments.emplace_back("_" + name.str());
        }
#elif defined(CKC_LLD_COFF)
        arguments.emplace_back("lld-link");
        arguments.emplace_back("/dll");
        arguments.emplace_back("/noentry");
        arguments.emplace_back("/nodefaultlib");
        auto import_path = checked_path(import_library_path_bytes,
                                        "LLD import library path");
        if (!import_path) {
            return set_llvm_error(error, import_path.takeError());
        }
        arguments.emplace_back("/implib:" + *import_path);
        for (size_t index = 0; index < export_count; ++index) {
            const llvm::StringRef name = borrowed_string(exports[index]);
            if (name.empty() || name.contains('\0') || name.contains(',') ||
                name.contains(':')) {
                return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                                 "invalid LLD export symbol");
            }
            arguments.emplace_back("/export:" + name.str());
        }
#else
        arguments.emplace_back("ld.lld");
        arguments.emplace_back("-shared");
        arguments.emplace_back("--no-undefined");
        for (size_t index = 0; index < export_count; ++index) {
            const llvm::StringRef name = borrowed_string(exports[index]);
            if (name.empty() || name.contains('\0') || name.contains(',')) {
                return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                                 "invalid LLD export symbol");
            }
        }
#endif
        arguments.emplace_back("-o");
        arguments.emplace_back(*output_path);
        arguments.emplace_back(*object_path);

        std::vector<const char *> raw_arguments;
        raw_arguments.reserve(arguments.size());
        for (const auto &argument : arguments) {
            raw_arguments.push_back(argument.c_str());
        }
        std::string stdout_text;
        std::string stderr_text;
        llvm::raw_string_ostream stdout_stream(stdout_text);
        llvm::raw_string_ostream stderr_stream(stderr_text);
#if defined(CKC_LLD_DARWIN)
        const lld::DriverDef drivers[] = {{lld::Darwin, &lld::macho::link}};
#elif defined(CKC_LLD_COFF)
        const lld::DriverDef drivers[] = {{lld::WinLink, &lld::coff::link}};
#else
        const lld::DriverDef drivers[] = {{lld::Gnu, &lld::elf::link}};
#endif
        static std::mutex lld_mutex;
        lld::Result result;
        {
            std::lock_guard<std::mutex> lock(lld_mutex);
            result = lld::lldMain(raw_arguments, stdout_stream, stderr_stream,
                                  drivers);
        }
        stdout_stream.flush();
        stderr_stream.flush();
        if (result.retCode != 0) {
            std::string message = stderr_text.empty() ? stdout_text : stderr_text;
            if (message.empty()) {
                message = "LLD returned a non-zero status";
            }
            return set_error(error, CKC_LLVM_INTERNAL_ERROR, message);
        }
        if (!result.canRunAgain) {
            return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                             "LLD completed but cannot safely run again");
        }
        if (auto validation = validate_shared_output(*output_path)) {
            return set_llvm_error(error, std::move(validation));
        }
#if defined(CKC_LLD_COFF)
        if (auto validation = validate_import_archive(*import_path)) {
            return set_llvm_error(error, std::move(validation));
        }
#endif
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception linking shared library");
    }
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
