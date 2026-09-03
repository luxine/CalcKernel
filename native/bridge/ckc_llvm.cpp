#include "ckc_llvm.h"

#include <algorithm>
#include <cctype>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <future>
#include <limits>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <set>
#include <sstream>
#include <string>
#include <string_view>
#include <vector>

#include <llvm-c/Core.h>
#include <llvm-c/TargetMachine.h>
#include <llvm/Config/llvm-config.h>
#include <llvm/ExecutionEngine/JITLink/JITLinkMemoryManager.h>
#include <llvm/ExecutionEngine/JITLink/JITLink.h>
#include <llvm/ExecutionEngine/JITLink/x86_64.h>
#include <llvm/ExecutionEngine/Orc/AbsoluteSymbols.h>
#include <llvm/ExecutionEngine/Orc/LLJIT.h>
#include <llvm/ExecutionEngine/Orc/MapperJITLinkMemoryManager.h>
#include <llvm/ExecutionEngine/Orc/MemoryMapper.h>
#include <llvm/ExecutionEngine/Orc/ObjectLinkingLayer.h>
#include <llvm/ExecutionEngine/Orc/RTDyldObjectLinkingLayer.h>
#include <llvm/ExecutionEngine/SectionMemoryManager.h>
#include <llvm/IR/LLVMContext.h>
#include <llvm/IR/Attributes.h>
#include <llvm/IR/Comdat.h>
#include <llvm/IR/Constants.h>
#include <llvm/IR/DerivedTypes.h>
#include <llvm/IR/Dominators.h>
#include <llvm/IR/Function.h>
#include <llvm/IR/GlobalVariable.h>
#include <llvm/IR/IRBuilder.h>
#include <llvm/IR/Intrinsics.h>
#include <llvm/IR/Instructions.h>
#include <llvm/IR/LegacyPassManager.h>
#include <llvm/IR/MDBuilder.h>
#include <llvm/IR/Metadata.h>
#include <llvm/IR/Module.h>
#include <llvm/IR/Operator.h>
#include <llvm/IR/Verifier.h>
#include <llvm/ADT/SmallVector.h>
#include <llvm/Analysis/CGSCCPassManager.h>
#include <llvm/Analysis/LoopAnalysisManager.h>
#include <llvm/Analysis/LoopInfo.h>
#include <llvm/Analysis/ModuleSummaryAnalysis.h>
#include <llvm/Analysis/TargetLibraryInfo.h>
#include <llvm/Analysis/TargetTransformInfo.h>
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
#include <llvm/Support/Alignment.h>
#include <llvm/Support/MemoryBuffer.h>
#include <llvm/Support/MathExtras.h>
#include <llvm/Support/ModRef.h>
#include <llvm/Support/Process.h>
#include <llvm/Support/TargetSelect.h>
#include <llvm/Support/raw_ostream.h>
#include <llvm/Support/SHA256.h>
#include <llvm/Target/TargetMachine.h>
#include <llvm/Transforms/Utils/Cloning.h>
#include <llvm/Transforms/Utils/ModuleUtils.h>
#include <llvm/Transforms/Utils/PromoteMemToReg.h>
#include <lld/Common/Driver.h>

#if defined(CKC_LLD_DARWIN)
#include <fcntl.h>
#include <pthread.h>
#include <signal.h>
#include <sys/mman.h>
#include <unistd.h>
#elif defined(CKC_LLD_COFF)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
// Keep SDK declarations, but not macros that shadow standard functions or
// LLVM's typed COFF enums. Also handle min/max from a prior SDK include.
#undef min
#undef max
#undef IMAGE_FILE_DLL
#undef IMAGE_FILE_EXECUTABLE_IMAGE
#endif

struct CkcJitMemoryAuditState {
    std::mutex mutex;
    uint64_t allocations = 0;
    uint64_t instruction_cache_finalizations = 0;
    bool saw_relocation_allocation = false;
    bool relocation_write_non_execute = true;
    bool saw_final_code = false;
    bool final_code_read_execute = true;
    bool saw_final_data = false;
    bool final_data_non_execute = true;
    bool darwin_map_jit = false;
    bool darwin_thread_write_protection_supported = false;
    bool darwin_thread_write_protection = false;
};

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
    std::unique_ptr<llvm::LLVMContext> profile_context;
    std::unique_ptr<llvm::Module> profile_module;
    llvm::Function *profile_function = nullptr;
};

struct CkcLlvmJit {
    std::unique_ptr<llvm::orc::LLJIT> value;
    std::shared_ptr<CkcJitMemoryAuditState> memory_audit;
    CkcLlvmOrcObjectLayer object_layer;
    bool executed;
};

struct CkcLlvmBuilder {
    std::unique_ptr<llvm::IRBuilder<>> value;
    llvm::MDNode *alias_domain = nullptr;
    std::map<uint32_t, llvm::MDNode *> alias_scopes;
};

namespace {

constexpr int32_t CKC_LLVM_OK = 0;
constexpr int32_t CKC_LLVM_INVALID_ARGUMENT = 1;
constexpr int32_t CKC_LLVM_OUT_OF_MEMORY = 2;
constexpr int32_t CKC_LLVM_INTERNAL_ERROR = 3;
constexpr size_t CKC_JIT_RESERVATION_GRANULARITY = 512ULL * 1024ULL * 1024ULL;

class CkcInProcessMemoryMapper final : public llvm::orc::MemoryMapper {
public:
    static llvm::Expected<std::unique_ptr<CkcInProcessMemoryMapper>>
    Create(std::shared_ptr<CkcJitMemoryAuditState> audit) {
        auto page_size = llvm::sys::Process::getPageSize();
        if (!page_size) {
            return page_size.takeError();
        }
        return std::make_unique<CkcInProcessMemoryMapper>(
            *page_size, std::move(audit));
    }

    CkcInProcessMemoryMapper(
        size_t page_size, std::shared_ptr<CkcJitMemoryAuditState> audit)
        : page_size_(page_size), audit_(std::move(audit)) {
#if defined(CKC_LLD_DARWIN)
        const bool supported = pthread_jit_write_protect_supported_np() != 0;
        std::lock_guard<std::mutex> lock(audit_->mutex);
        audit_->darwin_thread_write_protection_supported = supported;
#endif
    }

    unsigned int getPageSize() override {
        return static_cast<unsigned int>(page_size_);
    }

    void reserve(size_t byte_count, OnReservedFunction on_reserved) override {
        std::error_code error;
        llvm::sys::MemoryBlock block;
#if defined(CKC_LLD_DARWIN)
        if (uses_darwin_thread_write_protection()) {
            const size_t rounded = llvm::alignTo(byte_count, page_size_);
            void *address = mmap(nullptr, rounded,
                                 PROT_READ | PROT_WRITE | PROT_EXEC,
                                 MAP_PRIVATE | MAP_ANON | MAP_JIT, -1, 0);
            if (address == MAP_FAILED) {
                error = std::error_code(errno, std::generic_category());
                on_reserved(llvm::make_error<llvm::StringError>(
                    "MAP_JIT reservation failed: " + error.message(), error));
                return;
            }
            block = llvm::sys::MemoryBlock(address, rounded);
            set_darwin_write_mode(true);
            {
                std::lock_guard<std::mutex> audit_lock(audit_->mutex);
                audit_->darwin_map_jit = true;
            }
        } else {
            // Darwin page-protection JIT fallback: platforms without
            // per-thread MAP_JIT protection reserve RW/NX pages and finalize
            // each segment with mprotect, exactly like the other JITLink
            // hosts. A page is never writable and executable at once.
            block = llvm::sys::Memory::allocateMappedMemory(
                byte_count, nullptr,
                llvm::sys::Memory::MF_READ | llvm::sys::Memory::MF_WRITE,
                error);
        }
#else
        block = llvm::sys::Memory::allocateMappedMemory(
            byte_count, nullptr,
            llvm::sys::Memory::MF_READ | llvm::sys::Memory::MF_WRITE,
            error);
#endif
        if (error) {
            on_reserved(llvm::make_error<llvm::StringError>(
                "JIT reservation failed: " + error.message(), error));
            return;
        }

        {
            std::lock_guard<std::mutex> lock(mutex_);
            reservations_[block.base()] = {block.allocatedSize(), {}};
        }
        {
            std::lock_guard<std::mutex> lock(audit_->mutex);
            audit_->saw_relocation_allocation = true;
            audit_->relocation_write_non_execute =
                audit_->relocation_write_non_execute &&
#if defined(CKC_LLD_DARWIN)
                (!audit_->darwin_thread_write_protection_supported ||
                 (audit_->darwin_map_jit &&
                  audit_->darwin_thread_write_protection));
#else
                true;
#endif
        }
        on_reserved(llvm::orc::ExecutorAddrRange(
            llvm::orc::ExecutorAddr::fromPtr(block.base()),
            block.allocatedSize()));
    }

    char *prepare(llvm::jitlink::LinkGraph &graph,
                  llvm::orc::ExecutorAddr address,
                  size_t content_size) override {
#if defined(CKC_LLD_DARWIN)
        if (uses_darwin_thread_write_protection()) {
            set_darwin_write_mode(true);
            return graph.allocateBuffer(content_size).data();
        }
#endif
        return address.toPtr<char *>();
    }

    void initialize(AllocInfo &allocation,
                    OnInitializedFunction on_initialized) override {
#if defined(CKC_LLD_DARWIN)
        const bool uses_thread_write_protection =
            uses_darwin_thread_write_protection();
        // Materialization is recursive: a dependency may finalize after this
        // graph's prepare call and restore execute mode. Re-enter write mode
        // at the exact copy boundary for every allocation.
        if (uses_thread_write_protection) {
            set_darwin_write_mode(true);
        }
#endif
        llvm::orc::ExecutorAddr minimum(~0ULL);
        llvm::orc::ExecutorAddr maximum(0);
        bool saw_code = false;
        bool saw_data = false;
        bool code_permissions_valid = true;
        bool data_permissions_valid = true;

        for (auto &segment : allocation.Segments) {
            auto base = allocation.MappingBase + segment.Offset;
            const size_t size = segment.ContentSize + segment.ZeroFillSize;
            if (size == 0) {
                continue;
            }
            minimum = std::min(minimum, base);
            maximum = std::max(maximum, base + size);

            const auto protection = segment.AG.getMemProt();
            const unsigned flags = llvm::orc::toSysMemoryProtectionFlags(
                protection);
            const auto protection_bits = llvm::to_underlying(protection);
            const auto read_bit =
                llvm::to_underlying(llvm::orc::MemProt::Read);
            const auto write_bit =
                llvm::to_underlying(llvm::orc::MemProt::Write);
            const auto execute_bit =
                llvm::to_underlying(llvm::orc::MemProt::Exec);
            const bool executable = (protection_bits & execute_bit) != 0;
#if defined(CKC_LLD_DARWIN)
            if (uses_thread_write_protection) {
                if (!executable) {
                    const size_t mapped_size = llvm::alignTo(size, page_size_);
                    void *mapped =
                        mmap(base.toPtr<void *>(), mapped_size,
                             PROT_READ | PROT_WRITE,
                             MAP_FIXED | MAP_PRIVATE | MAP_ANON, -1, 0);
                    if (mapped == MAP_FAILED) {
                        set_darwin_write_mode(false);
                        const std::error_code error(
                            errno, std::generic_category());
                        on_initialized(llvm::make_error<llvm::StringError>(
                            "JIT data mapping failed: " + error.message(),
                            error));
                        return;
                    }
                }
                std::memcpy(base.toPtr<void *>(), segment.WorkingMem,
                            segment.ContentSize);
                std::memset((base + segment.ContentSize).toPtr<void *>(), 0,
                            segment.ZeroFillSize);
                if (!executable) {
                    if (auto error = llvm::sys::Memory::protectMappedMemory(
                            {base.toPtr<void *>(), size}, flags)) {
                        set_darwin_write_mode(false);
                        on_initialized(llvm::make_error<llvm::StringError>(
                            "JIT data finalization failed: " +
                                error.message(),
                            error));
                        return;
                    }
                }
            } else {
                std::memset((base + segment.ContentSize).toPtr<void *>(), 0,
                            segment.ZeroFillSize);
                if (auto error = llvm::sys::Memory::protectMappedMemory(
                        {base.toPtr<void *>(), size}, flags)) {
                    on_initialized(llvm::make_error<llvm::StringError>(
                        "JIT segment finalization failed: " + error.message(),
                        error));
                    return;
                }
            }
#else
            std::memset((base + segment.ContentSize).toPtr<void *>(), 0,
                        segment.ZeroFillSize);
            if (auto error = llvm::sys::Memory::protectMappedMemory(
                    {base.toPtr<void *>(), size}, flags)) {
                on_initialized(llvm::make_error<llvm::StringError>(
                    "JIT segment finalization failed: " + error.message(),
                    error));
                return;
            }
#endif
            if (executable) {
                saw_code = true;
                code_permissions_valid =
                    code_permissions_valid &&
                    (protection_bits & read_bit) != 0 &&
                    (protection_bits & write_bit) == 0;
                llvm::sys::Memory::InvalidateInstructionCache(
                    base.toPtr<void *>(), size);
                std::lock_guard<std::mutex> lock(audit_->mutex);
                ++audit_->instruction_cache_finalizations;
            } else {
                saw_data = true;
                data_permissions_valid =
                    data_permissions_valid &&
                    (protection_bits & execute_bit) == 0;
            }
        }

#if defined(CKC_LLD_DARWIN)
        if (uses_thread_write_protection) {
            set_darwin_write_mode(false);
        }
#endif
        if (minimum.getValue() == ~0ULL) {
            on_initialized(llvm::make_error<llvm::StringError>(
                "JIT allocation has no material segments",
                llvm::inconvertibleErrorCode()));
            return;
        }

        auto deinitialization_actions =
            llvm::orc::shared::runFinalizeActions(allocation.Actions);
        if (!deinitialization_actions) {
            on_initialized(deinitialization_actions.takeError());
            return;
        }
        {
            std::lock_guard<std::mutex> lock(mutex_);
            auto &record = allocations_[minimum.getValue()];
            record.size = maximum - minimum;
            record.deinitialization_actions =
                std::move(*deinitialization_actions);
            reservations_[allocation.MappingBase.toPtr<void *>()]
                .allocations.push_back(minimum.getValue());
        }
        {
            std::lock_guard<std::mutex> lock(audit_->mutex);
            ++audit_->allocations;
            audit_->saw_final_code = audit_->saw_final_code || saw_code;
            audit_->saw_final_data = audit_->saw_final_data || saw_data;
            audit_->final_code_read_execute =
                audit_->final_code_read_execute && code_permissions_valid;
            audit_->final_data_non_execute =
                audit_->final_data_non_execute && data_permissions_valid;
        }
        on_initialized(minimum);
    }

    void deinitialize(llvm::ArrayRef<llvm::orc::ExecutorAddr> bases,
                      OnDeinitializedFunction on_deinitialized) override {
        llvm::Error combined = llvm::Error::success();
#if defined(CKC_LLD_DARWIN)
        const bool uses_thread_write_protection =
            uses_darwin_thread_write_protection();
        if (uses_thread_write_protection) {
            set_darwin_write_mode(true);
        }
#endif
        std::lock_guard<std::mutex> lock(mutex_);
        for (auto base : llvm::reverse(bases)) {
            auto found = allocations_.find(base.getValue());
            if (found == allocations_.end()) {
                continue;
            }
            if (auto error = llvm::orc::shared::runDeallocActions(
                    found->second.deinitialization_actions)) {
                combined = llvm::joinErrors(std::move(combined),
                                            std::move(error));
            }
#if defined(CKC_LLD_DARWIN)
            if (!uses_thread_write_protection) {
                if (auto error = llvm::sys::Memory::protectMappedMemory(
                        {base.toPtr<void *>(), found->second.size},
                        llvm::sys::Memory::MF_READ |
                            llvm::sys::Memory::MF_WRITE)) {
                    combined = llvm::joinErrors(
                        std::move(combined), llvm::errorCodeToError(error));
                }
            }
#else
            if (auto error = llvm::sys::Memory::protectMappedMemory(
                    {base.toPtr<void *>(), found->second.size},
                    llvm::sys::Memory::MF_READ |
                        llvm::sys::Memory::MF_WRITE)) {
                combined = llvm::joinErrors(
                    std::move(combined), llvm::errorCodeToError(error));
            }
#endif
            allocations_.erase(found);
        }
#if defined(CKC_LLD_DARWIN)
        if (uses_thread_write_protection) {
            set_darwin_write_mode(false);
        }
#endif
        on_deinitialized(std::move(combined));
    }

    void release(llvm::ArrayRef<llvm::orc::ExecutorAddr> bases,
                 OnReleasedFunction on_released) override {
        llvm::Error combined = llvm::Error::success();
        for (auto base : bases) {
            Reservation reservation;
            {
                std::lock_guard<std::mutex> lock(mutex_);
                auto found = reservations_.find(base.toPtr<void *>());
                if (found == reservations_.end()) {
                    continue;
                }
                reservation = std::move(found->second);
                reservations_.erase(found);
            }

            std::promise<llvm::MSVCPError> promise;
            auto future = promise.get_future();
            std::vector<llvm::orc::ExecutorAddr> allocation_bases;
            allocation_bases.reserve(reservation.allocations.size());
            for (uint64_t allocation_base : reservation.allocations) {
                allocation_bases.emplace_back(allocation_base);
            }
            deinitialize(allocation_bases, [&](llvm::Error error) {
                promise.set_value(std::move(error));
            });
            if (auto error = future.get()) {
                combined = llvm::joinErrors(std::move(combined),
                                            std::move(error));
            }

            llvm::sys::MemoryBlock block(base.toPtr<void *>(),
                                         reservation.size);
            if (auto error = llvm::sys::Memory::releaseMappedMemory(block)) {
                combined = llvm::joinErrors(
                    std::move(combined), llvm::errorCodeToError(error));
            }
        }
        on_released(std::move(combined));
    }

    ~CkcInProcessMemoryMapper() override {
        std::vector<llvm::orc::ExecutorAddr> bases;
        {
            std::lock_guard<std::mutex> lock(mutex_);
            bases.reserve(reservations_.size());
            for (const auto &[base, _] : reservations_) {
                bases.push_back(llvm::orc::ExecutorAddr::fromPtr(base));
            }
        }
        if (bases.empty()) {
            return;
        }
        std::promise<llvm::MSVCPError> promise;
        auto future = promise.get_future();
        release(bases, [&](llvm::Error error) {
            promise.set_value(std::move(error));
        });
        if (auto error = future.get()) {
            llvm::consumeError(std::move(error));
        }
    }

private:
    struct Allocation {
        size_t size = 0;
        std::vector<llvm::orc::shared::WrapperFunctionCall>
            deinitialization_actions;
    };

    struct Reservation {
        size_t size = 0;
        std::vector<uint64_t> allocations;
    };

#if defined(CKC_LLD_DARWIN)
    bool uses_darwin_thread_write_protection() const {
        std::lock_guard<std::mutex> lock(audit_->mutex);
        return audit_->darwin_thread_write_protection_supported;
    }

    void set_darwin_write_mode(bool writable) {
        if (!uses_darwin_thread_write_protection()) {
            return;
        }
        pthread_jit_write_protect_np(writable ? 0 : 1);
        std::lock_guard<std::mutex> lock(audit_->mutex);
        audit_->darwin_thread_write_protection = true;
    }
#endif

    size_t page_size_;
    std::shared_ptr<CkcJitMemoryAuditState> audit_;
    std::mutex mutex_;
    std::map<void *, Reservation> reservations_;
    std::map<uint64_t, Allocation> allocations_;
};

#if defined(CKC_LLD_COFF) && \
    (defined(_M_X64) || defined(__x86_64__))
bool is_allowed_coff_x64_process_symbol(const llvm::jitlink::Symbol &symbol) {
    if (!symbol.hasName() || !symbol.isExternal()) {
        return false;
    }
    const llvm::StringRef name = *symbol.getName();
    return name == "GetStdHandle" || name == "WriteFile" ||
           name == "ExitProcess";
}

llvm::Error add_coff_x64_process_stubs(llvm::jitlink::LinkGraph &G) {
    std::vector<llvm::jitlink::Edge *> process_calls;
    for (auto *block : G.blocks()) {
        for (auto &edge : block->edges()) {
            if (!is_allowed_coff_x64_process_symbol(edge.getTarget())) {
                continue;
            }
            if (G.getEdgeKindName(edge.getKind()) !=
                llvm::StringRef("PCRel32")) {
                return llvm::createStringError(
                    "COFF x64 process symbol has a non-PCRel32 relocation");
            }
            const auto content = block->getContent();
            if (edge.getOffset() == 0 || edge.getOffset() > content.size() ||
                static_cast<unsigned char>(content[edge.getOffset() - 1]) !=
                    0xe8u) {
                return llvm::createStringError(
                    "COFF x64 process symbol PCRel32 is not a direct call opcode");
            }
            process_calls.push_back(&edge);
        }
    }
    if (process_calls.empty()) {
        return llvm::Error::success();
    }
    if (G.findSectionByName("$__CKC_PROCESS_GOT") != nullptr ||
        G.findSectionByName("$__CKC_PROCESS_STUBS") != nullptr) {
        return llvm::createStringError(
            "COFF x64 object defines a reserved process-stub section");
    }

    auto &pointer_section = G.createSection(
        "$__CKC_PROCESS_GOT", llvm::orc::MemProt::Read);
    const auto stub_protection = static_cast<llvm::orc::MemProt>(
        llvm::to_underlying(llvm::orc::MemProt::Read) |
        llvm::to_underlying(llvm::orc::MemProt::Exec));
    auto &stub_section = G.createSection(
        "$__CKC_PROCESS_STUBS", stub_protection);
    std::map<llvm::jitlink::Symbol *, llvm::jitlink::Symbol *> stubs;
    for (auto *edge : process_calls) {
        auto *target = &edge->getTarget();
        auto found = stubs.find(target);
        if (found == stubs.end()) {
            auto &pointer = llvm::jitlink::x86_64::createAnonymousPointer(
                G, pointer_section, target);
            if (pointer.getBlock().edges_size() != 1 ||
                pointer.getBlock().edges().begin()->getKind() !=
                    llvm::jitlink::x86_64::Pointer64) {
                return llvm::createStringError(
                    "COFF x64 process pointer did not use Pointer64");
            }
            auto &stub =
                llvm::jitlink::x86_64::createAnonymousPointerJumpStub(
                    G, stub_section, pointer);
            found = stubs.emplace(target, &stub).first;
        }
        edge->setTarget(*found->second);
    }
    return llvm::Error::success();
}

class CkcCoffX64ProcessStubsPlugin final
    : public llvm::orc::LinkGraphLinkingLayer::Plugin {
public:
    void modifyPassConfig(
        llvm::orc::MaterializationResponsibility &,
        llvm::jitlink::LinkGraph &G,
        llvm::jitlink::PassConfiguration &Config) override {
        if (G.getTargetTriple().getArch() == llvm::Triple::x86_64 &&
            G.getTargetTriple().isOSBinFormatCOFF()) {
            Config.PostPrunePasses.push_back(
                add_coff_x64_process_stubs);
        }
    }

    llvm::Error notifyFailed(
        llvm::orc::MaterializationResponsibility &) override {
        return llvm::Error::success();
    }

    llvm::Error notifyRemovingResources(
        llvm::orc::JITDylib &, llvm::orc::ResourceKey) override {
        return llvm::Error::success();
    }

    void notifyTransferringResources(
        llvm::orc::JITDylib &, llvm::orc::ResourceKey,
        llvm::orc::ResourceKey) override {}
};
#endif

class CkcSectionMemoryMapper
    : public llvm::SectionMemoryManager::MemoryMapper {
public:
    explicit CkcSectionMemoryMapper(
        std::shared_ptr<CkcJitMemoryAuditState> audit)
        : audit_(std::move(audit)) {}

    llvm::sys::MemoryBlock allocateMappedMemory(
        llvm::SectionMemoryManager::AllocationPurpose purpose,
        size_t byte_count, const llvm::sys::MemoryBlock *near_block,
        unsigned flags, std::error_code &error) override {
        auto block = llvm::sys::Memory::allocateMappedMemory(
            byte_count, near_block, flags, error);
        if (error || block.base() == nullptr) {
            return block;
        }

        const bool writable =
            (flags & llvm::sys::Memory::MF_WRITE) != 0;
        const bool executable =
            (flags & llvm::sys::Memory::MF_EXEC) != 0;
        {
            std::lock_guard<std::mutex> lock(audit_->mutex);
            ++audit_->allocations;
            audit_->saw_relocation_allocation = true;
            audit_->relocation_write_non_execute =
                audit_->relocation_write_non_execute && writable &&
                !executable;
            saw_data_allocation_ =
                saw_data_allocation_ ||
                purpose !=
                    llvm::SectionMemoryManager::AllocationPurpose::Code;
        }
        return block;
    }

    std::error_code protectMappedMemory(
        const llvm::sys::MemoryBlock &block, unsigned flags) override {
        auto error = llvm::sys::Memory::protectMappedMemory(block, flags);
        if (error) {
            return error;
        }

        const bool readable =
            (flags & llvm::sys::Memory::MF_READ) != 0;
        const bool writable =
            (flags & llvm::sys::Memory::MF_WRITE) != 0;
        const bool executable =
            (flags & llvm::sys::Memory::MF_EXEC) != 0;
        std::lock_guard<std::mutex> lock(audit_->mutex);
        if (executable) {
            // RuntimeDyld finalizes relocations before requesting RX. Flush
            // the exact protected code range here; LLVM 22's base
            // SectionMemoryManager clears its pending range before its later
            // invalidateInstructionCache callback.
            llvm::sys::Memory::InvalidateInstructionCache(
                block.base(), block.allocatedSize());
            ++audit_->instruction_cache_finalizations;
            audit_->saw_final_code = true;
            audit_->final_code_read_execute =
                audit_->final_code_read_execute && readable && !writable;
        } else {
            audit_->saw_final_data = true;
            audit_->final_data_non_execute =
                audit_->final_data_non_execute && !executable;
        }
        return error;
    }

    std::error_code releaseMappedMemory(
        llvm::sys::MemoryBlock &block) override {
        return llvm::sys::Memory::releaseMappedMemory(block);
    }

protected:
    void record_successful_finalization() {
        std::lock_guard<std::mutex> lock(audit_->mutex);
        if (saw_data_allocation_) {
            // With reservation enabled RuntimeDyld requests one initially-RW,
            // non-executable allocation and applies RX only to its code
            // subrange. The remaining data pages therefore stay NX.
            audit_->saw_final_data = true;
        }
    }

private:
    std::shared_ptr<CkcJitMemoryAuditState> audit_;
    bool saw_data_allocation_ = false;
};

// Base order is intentional: SectionMemoryManager's destructor releases
// mappings through the mapper, so the mapper base must be constructed first
// and destroyed last.
class CkcAuditedSectionMemoryManager final
    : private CkcSectionMemoryMapper,
      public llvm::SectionMemoryManager {
public:
    explicit CkcAuditedSectionMemoryManager(
        std::shared_ptr<CkcJitMemoryAuditState> audit)
        : CkcSectionMemoryMapper(std::move(audit)),
          llvm::SectionMemoryManager(
              static_cast<CkcSectionMemoryMapper *>(this), true) {}

    bool finalizeMemory(std::string *error_message = nullptr) override {
        const bool failed =
            llvm::SectionMemoryManager::finalizeMemory(error_message);
        if (!failed) {
            record_successful_finalization();
        }
        return failed;
    }
};

std::mutex &lld_driver_mutex() {
    static std::mutex mutex;
    return mutex;
}

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

int32_t finish_target_machine(llvm::orc::JITTargetMachineBuilder &builder,
                              CkcLlvmTarget **out,
                              CkcLlvmError *error) {
    builder.setRelocationModel(llvm::Reloc::PIC_);
    if (builder.getTargetTriple().isOSBinFormatMachO()) {
        // JIT defaults to Large on x86-64, whose Mach-O calls still use
        // absolute text relocations even with PIC. The same object must also
        // be loadable by dyld without writing its executable pages.
        builder.setCodeModel(llvm::CodeModel::Small);
    }
    auto target_machine = builder.createTargetMachine();
    if (!target_machine) {
        return set_llvm_error(error, target_machine.takeError());
    }
    auto target = std::make_unique<CkcLlvmTarget>();
    target->value = std::move(*target_machine);
    target->cpu = target->value->getTargetCPU().str();
    target->features = target->value->getTargetFeatureString().str();
    target->profile_context = std::make_unique<llvm::LLVMContext>();
    target->profile_module = std::make_unique<llvm::Module>(
        "ckc.target.profile", *target->profile_context);
    target->profile_module->setTargetTriple(target->value->getTargetTriple());
    target->profile_module->setDataLayout(target->value->createDataLayout());
    auto *profile_function_type = llvm::FunctionType::get(
        llvm::Type::getVoidTy(*target->profile_context), false);
    target->profile_function = llvm::Function::Create(
        profile_function_type, llvm::GlobalValue::InternalLinkage,
        "__ck_target_profile_probe", *target->profile_module);
    target->profile_function->addFnAttr("target-cpu", target->cpu);
    target->profile_function->addFnAttr("target-features", target->features);
    *out = target.release();
    return CKC_LLVM_OK;
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

llvm::Expected<std::unique_ptr<llvm::MemoryBuffer>> validated_object_buffer(
    CkcLlvmBytes bytes, llvm::StringRef name, llvm::Triple::ArchType arch) {
    if (bytes.data == nullptr || bytes.len == 0) {
        return llvm::createStringError("%s is empty", name.str().c_str());
    }
    auto buffer = llvm::MemoryBuffer::getMemBufferCopy(borrowed_string(bytes), name);
    auto object = llvm::object::ObjectFile::createObjectFile(
        buffer->getMemBufferRef());
    if (!object) {
        return object.takeError();
    }
    if (!(*object)->isRelocatableObject() || (*object)->getArch() != arch) {
        return llvm::createStringError(
            "%s is not a host-architecture relocatable object",
            name.str().c_str());
    }
#if defined(CKC_LLD_DARWIN)
    if (!llvm::isa<llvm::object::MachOObjectFile>(object->get())) {
        return llvm::createStringError("%s is not a Mach-O object",
                                       name.str().c_str());
    }
#elif defined(CKC_LLD_COFF)
    if (!llvm::isa<llvm::object::COFFObjectFile>(object->get())) {
        return llvm::createStringError("%s is not a COFF object",
                                       name.str().c_str());
    }
#else
    if (!llvm::isa<llvm::object::ELFObjectFileBase>(object->get())) {
        return llvm::createStringError("%s is not an ELF object",
                                       name.str().c_str());
    }
#endif
    return std::move(buffer);
}

llvm::Expected<std::vector<std::string>> defined_linker_symbols(
    const llvm::MemoryBuffer &buffer) {
    auto object = llvm::object::ObjectFile::createObjectFile(
        buffer.getMemBufferRef());
    if (!object) {
        return object.takeError();
    }
    std::vector<std::string> names;
    for (const auto &symbol : (*object)->symbols()) {
        auto flags = symbol.getFlags();
        if (!flags) {
            return flags.takeError();
        }
        if ((*flags & llvm::object::BasicSymbolRef::SF_Undefined) != 0 ||
            (*flags & llvm::object::BasicSymbolRef::SF_Global) == 0 ||
            (*flags & llvm::object::BasicSymbolRef::SF_FormatSpecific) != 0) {
            continue;
        }
        auto name = symbol.getName();
        if (!name) {
            return name.takeError();
        }
        if (!name->empty()) {
            names.push_back(name->str());
        }
    }
    return names;
}

llvm::Error define_allowed_process_symbols(llvm::orc::LLJIT &jit) {
    llvm::orc::SymbolMap symbols;
#if defined(CKC_LLD_DARWIN)
    symbols[jit.mangleAndIntern("fcntl")] =
        llvm::orc::ExecutorSymbolDef::fromPtr(
            &::fcntl, llvm::JITSymbolFlags::Exported);
    symbols[jit.mangleAndIntern("signal")] =
        llvm::orc::ExecutorSymbolDef::fromPtr(
            &::signal, llvm::JITSymbolFlags::Exported);
    symbols[jit.mangleAndIntern("write")] =
        llvm::orc::ExecutorSymbolDef::fromPtr(
            &::write, llvm::JITSymbolFlags::Exported);
    symbols[jit.mangleAndIntern("_exit")] =
        llvm::orc::ExecutorSymbolDef::fromPtr(
            &::_exit, llvm::JITSymbolFlags::Exported);
#elif defined(CKC_LLD_COFF)
    symbols[jit.mangleAndIntern("GetStdHandle")] =
        llvm::orc::ExecutorSymbolDef::fromPtr(
            &::GetStdHandle, llvm::JITSymbolFlags::Exported);
    symbols[jit.mangleAndIntern("WriteFile")] =
        llvm::orc::ExecutorSymbolDef::fromPtr(
            &::WriteFile, llvm::JITSymbolFlags::Exported);
    symbols[jit.mangleAndIntern("ExitProcess")] =
        llvm::orc::ExecutorSymbolDef::fromPtr(
            &::ExitProcess, llvm::JITSymbolFlags::Exported);
#endif
    if (symbols.empty()) {
        return llvm::Error::success();
    }
    return jit.getMainJITDylib().define(
        llvm::orc::absoluteSymbols(std::move(symbols)));
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

llvm::Error validate_executable_output(llvm::StringRef path) {
    auto binary = llvm::object::createBinary(path);
    if (!binary) {
        return binary.takeError();
    }
    auto *object = llvm::dyn_cast<llvm::object::ObjectFile>(binary->getBinary());
    if (object == nullptr || object->isRelocatableObject()) {
        return llvm::createStringError("LLD output is not a linked executable");
    }
#if defined(CKC_LLD_DARWIN)
    const auto *macho = llvm::dyn_cast<llvm::object::MachOObjectFile>(object);
    if (macho == nullptr || macho->getHeader().filetype != llvm::MachO::MH_EXECUTE) {
        return llvm::createStringError("LLD output is not a Mach-O executable");
    }
#elif defined(CKC_LLD_COFF)
    const auto *coff = llvm::dyn_cast<llvm::object::COFFObjectFile>(object);
    if (coff == nullptr ||
        (coff->getCharacteristics() & llvm::COFF::IMAGE_FILE_EXECUTABLE_IMAGE) == 0 ||
        (coff->getCharacteristics() & llvm::COFF::IMAGE_FILE_DLL) != 0) {
        return llvm::createStringError("LLD output is not a PE executable");
    }
#else
    const auto *elf = llvm::dyn_cast<llvm::object::ELFObjectFileBase>(object);
    if (elf == nullptr || elf->getEType() != llvm::ELF::ET_EXEC) {
        return llvm::createStringError("LLD output is not an ELF executable");
    }
#endif
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

llvm::Type *profile_lane_type(llvm::LLVMContext &context, uint32_t lane) {
    switch (lane) {
    case 1:
    case 3:
        return llvm::Type::getInt32Ty(context);
    case 2:
    case 4:
        return llvm::Type::getInt64Ty(context);
    case 5:
        return llvm::Type::getDoubleTy(context);
    default:
        return nullptr;
    }
}

llvm::Type *profile_value_type(llvm::LLVMContext &context, uint32_t lane,
                               uint32_t lanes) {
    llvm::Type *element = profile_lane_type(context, lane);
    if (element == nullptr || lanes == 0) {
        return nullptr;
    }
    if (lanes == 1) {
        return element;
    }
    return llvm::FixedVectorType::get(element, lanes);
}

llvm::InstructionCost profile_operation_cost(
    const llvm::TargetTransformInfo &tti,
    const CkcLlvmTargetProfileQuery &query, llvm::Type *value_type) {
    using TTI = llvm::TargetTransformInfo;
    constexpr auto kind = TTI::TCK_RecipThroughput;
    const bool floating = query.lane == 5;
    const bool unsigned_integer = query.lane == 3 || query.lane == 4;
    auto *vector_type = llvm::dyn_cast<llvm::VectorType>(value_type);
    llvm::Type *mask_type = llvm::Type::getInt1Ty(value_type->getContext());
    if (query.lanes != 1) {
        mask_type = llvm::FixedVectorType::get(mask_type, query.lanes);
    }
    switch (query.operation) {
    case 1: // splat
        if (vector_type == nullptr) {
            return llvm::InstructionCost::getInvalid();
        }
        return tti.getShuffleCost(TTI::SK_Broadcast, vector_type, vector_type,
                                  {}, kind);
    case 2: // add
        return tti.getArithmeticInstrCost(
            floating ? llvm::Instruction::FAdd : llvm::Instruction::Add,
            value_type, kind);
    case 3: // subtract
        return tti.getArithmeticInstrCost(
            floating ? llvm::Instruction::FSub : llvm::Instruction::Sub,
            value_type, kind);
    case 4: // multiply
        return tti.getArithmeticInstrCost(
            floating ? llvm::Instruction::FMul : llvm::Instruction::Mul,
            value_type, kind);
    case 5: // divide
        return tti.getArithmeticInstrCost(
            floating ? llvm::Instruction::FDiv
                     : (unsigned_integer ? llvm::Instruction::UDiv
                                         : llvm::Instruction::SDiv),
            value_type, kind);
    case 6: // remainder
        if (floating) {
            return llvm::InstructionCost::getInvalid();
        }
        return tti.getArithmeticInstrCost(
            unsigned_integer ? llvm::Instruction::URem
                             : llvm::Instruction::SRem,
            value_type, kind);
    case 7: // negate
        return tti.getArithmeticInstrCost(
            floating ? llvm::Instruction::FSub : llvm::Instruction::Sub,
            value_type, kind);
    case 8: // mask not
        if (query.lane != 1) {
            return llvm::InstructionCost::getInvalid();
        }
        return tti.getArithmeticInstrCost(llvm::Instruction::Xor, mask_type,
                                          kind);
    case 9: // bit and
    case 10: // bit or
    case 11: // bit xor
        if (floating) {
            return llvm::InstructionCost::getInvalid();
        }
        return tti.getArithmeticInstrCost(
            query.operation == 9   ? llvm::Instruction::And
            : query.operation == 10 ? llvm::Instruction::Or
                                    : llvm::Instruction::Xor,
            value_type, kind);
    case 12: // shift left
    case 13: // shift right
        if (floating) {
            return llvm::InstructionCost::getInvalid();
        }
        return tti.getArithmeticInstrCost(
            query.operation == 12
                ? llvm::Instruction::Shl
                : (unsigned_integer ? llvm::Instruction::LShr
                                    : llvm::Instruction::AShr),
            value_type, kind);
    case 14: // compare
        return tti.getCmpSelInstrCost(
            floating ? llvm::Instruction::FCmp : llvm::Instruction::ICmp,
            value_type, mask_type,
            floating ? llvm::CmpInst::FCMP_OLT
                     : (unsigned_integer ? llvm::CmpInst::ICMP_ULT
                                         : llvm::CmpInst::ICMP_SLT),
            kind);
    case 15: // select
        return tti.getCmpSelInstrCost(llvm::Instruction::Select, value_type,
                                      mask_type, llvm::CmpInst::BAD_ICMP_PREDICATE,
                                      kind);
    case 16: { // i32/u32 to f64 cast
        if (query.lane != 1 && query.lane != 3) {
            return llvm::InstructionCost::getInvalid();
        }
        llvm::Type *destination = profile_value_type(
            value_type->getContext(), 5, query.lanes);
        return tti.getCastInstrCost(
            query.lane == 1 ? llvm::Instruction::SIToFP
                            : llvm::Instruction::UIToFP,
            destination, value_type, TTI::CastContextHint::None, kind);
    }
    case 17: // insert
    case 18: // extract
        if (vector_type == nullptr) {
            return llvm::InstructionCost::getInvalid();
        }
        return tti.getVectorInstrCost(
            query.operation == 17 ? llvm::Instruction::InsertElement
                                  : llvm::Instruction::ExtractElement,
            vector_type, kind, 0);
    case 19: // load
    case 20: // store
        if (query.alignment == 0 || !llvm::isPowerOf2_32(query.alignment)) {
            return llvm::InstructionCost::getInvalid();
        }
        return tti.getMemoryOpCost(
            query.operation == 19 ? llvm::Instruction::Load
                                  : llvm::Instruction::Store,
            value_type, llvm::Align(query.alignment), 0, kind);
    case 21: // reduce add
        if (vector_type == nullptr || floating) {
            return llvm::InstructionCost::getInvalid();
        }
        return tti.getArithmeticReductionCost(llvm::Instruction::Add,
                                              vector_type, std::nullopt, kind);
    case 22: // reduce min
    case 23: { // reduce max
        if (vector_type == nullptr || floating) {
            return llvm::InstructionCost::getInvalid();
        }
        llvm::Intrinsic::ID id;
        if (query.operation == 22) {
            id = unsigned_integer ? llvm::Intrinsic::vector_reduce_umin
                                  : llvm::Intrinsic::vector_reduce_smin;
        } else {
            id = unsigned_integer ? llvm::Intrinsic::vector_reduce_umax
                                  : llvm::Intrinsic::vector_reduce_smax;
        }
        return tti.getMinMaxReductionCost(id, vector_type, {}, kind);
    }
    case 24: // branch
        return tti.getCFInstrCost(llvm::Instruction::Br, kind);
    case 25: // runtime predicate = compare plus branch
        return tti.getCmpSelInstrCost(
                   floating ? llvm::Instruction::FCmp
                            : llvm::Instruction::ICmp,
                   value_type, mask_type,
                   floating ? llvm::CmpInst::FCMP_OLT
                            : (unsigned_integer ? llvm::CmpInst::ICMP_ULT
                                                : llvm::CmpInst::ICMP_SLT),
                   kind) +
               tti.getCFInstrCost(llvm::Instruction::Br, kind);
    case 26: // reduce multiply
        if (vector_type == nullptr || floating) {
            return llvm::InstructionCost::getInvalid();
        }
        return tti.getArithmeticReductionCost(llvm::Instruction::Mul,
                                              vector_type, std::nullopt, kind);
    default:
        return llvm::InstructionCost::getInvalid();
    }
}

std::string profile_legalized_type(llvm::Type *value_type, uint32_t lanes,
                                   uint32_t parts) {
    llvm::Type *legalized = value_type;
    if (lanes > 1 && parts > 1) {
        if (parts >= lanes || lanes % parts != 0 || lanes / parts <= 1) {
            return {};
        }
        auto *vector_type = llvm::cast<llvm::VectorType>(value_type);
        legalized = llvm::FixedVectorType::get(vector_type->getElementType(),
                                               lanes / parts);
    }
    std::string text;
    llvm::raw_string_ostream stream(text);
    legalized->print(stream);
    stream.flush();
    return text;
}

struct CkcLateLayoutDirective {
    std::string function;
    std::vector<std::string> blocks;
};

void sha256_text(std::string_view text, uint8_t output[32]) {
    llvm::SHA256 digest;
    digest.update(llvm::StringRef(text.data(), text.size()));
    const auto bytes = digest.final();
    std::copy(bytes.begin(), bytes.end(), output);
}

std::string instruction_text(const llvm::Instruction &instruction) {
    std::string text;
    llvm::raw_string_ostream stream(text);
    instruction.print(stream);
    stream.flush();
    return text;
}

std::string late_layout_snapshot(const llvm::Module &module,
                                 bool structural) {
    std::vector<std::string> functions;
    for (const llvm::Function &function : module) {
        if (function.isDeclaration()) {
            continue;
        }
        std::vector<std::string> blocks;
        for (const llvm::BasicBlock &block : function) {
            std::string block_text = "block\t" + block.getName().str() + "\n";
            for (const llvm::Instruction &instruction : block) {
                block_text += instruction_text(instruction);
                block_text.push_back('\n');
            }
            blocks.push_back(std::move(block_text));
        }
        if (structural) {
            std::sort(blocks.begin(), blocks.end());
        }
        std::string function_text = "function\t" + function.getName().str() + "\n";
        for (const std::string &block : blocks) {
            function_text += block;
        }
        functions.push_back(std::move(function_text));
    }
    if (structural) {
        std::sort(functions.begin(), functions.end());
    }
    std::string snapshot = "CK-LATE-LAYOUT-SNAPSHOT-1\n";
    for (const std::string &function : functions) {
        snapshot += function;
    }
    return snapshot;
}

std::optional<std::vector<CkcLateLayoutDirective>>
parse_late_layout_plan(CkcLlvmBytes plan, std::string &failure) {
    if (plan.len != 0 && plan.data == nullptr) {
        failure = "late layout plan data is null";
        return std::nullopt;
    }
    std::string input(reinterpret_cast<const char *>(plan.data), plan.len);
    if (input.find('\0') != std::string::npos) {
        failure = "late layout plan contains NUL";
        return std::nullopt;
    }
    std::istringstream stream(input);
    std::string line;
    if (!std::getline(stream, line) || line != "CKLAYOUT1") {
        failure = "late layout plan has invalid schema";
        return std::nullopt;
    }
    std::vector<CkcLateLayoutDirective> directives;
    std::map<std::string, size_t> functions;
    std::set<std::pair<std::string, std::string>> blocks;
    while (std::getline(stream, line)) {
        if (line.empty()) {
            continue;
        }
        if (line.rfind("B\t", 0) != 0) {
            failure = "late layout plan has an unknown record";
            return std::nullopt;
        }
        const size_t separator = line.find('\t', 2);
        if (separator == std::string::npos || separator == 2 ||
            separator + 1 == line.size()) {
            failure = "late layout block record is malformed";
            return std::nullopt;
        }
        const std::string function = line.substr(2, separator - 2);
        const std::string block = line.substr(separator + 1);
        if (!blocks.insert({function, block}).second) {
            failure = "late layout block record is duplicated";
            return std::nullopt;
        }
        auto [position, inserted] = functions.emplace(function, directives.size());
        if (inserted) {
            directives.push_back({function, {}});
        }
        directives[position->second].blocks.push_back(block);
    }
    return directives;
}

bool late_layout_target_supported(const llvm::Triple &triple) {
    const bool architecture = triple.getArch() == llvm::Triple::x86_64 ||
                              triple.getArch() == llvm::Triple::aarch64;
    const bool format = triple.isOSBinFormatELF() || triple.isOSBinFormatMachO() ||
                        triple.isOSBinFormatCOFF();
    return architecture && format;
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
        const auto &triple = target->value->getTargetTriple();
        const bool needs_fltused = triple.isWindowsMSVCEnvironment() &&
                                   triple.getArch() == llvm::Triple::x86_64;
        if (needs_fltused &&
            module->value->getNamedGlobal("_fltused") == nullptr) {
            auto *type = llvm::Type::getInt32Ty(module->value->getContext());
            auto *helper = new llvm::GlobalVariable(
                *module->value, type, false,
                llvm::GlobalValue::WeakODRLinkage,
                llvm::ConstantInt::get(type, 0), "_fltused");
            auto *comdat = module->value->getOrInsertComdat("_fltused");
            comdat->setSelectionKind(llvm::Comdat::Any);
            helper->setComdat(comdat);
            helper->setDSOLocal(true);
            helper->setUnnamedAddr(llvm::GlobalValue::UnnamedAddr::Global);
            helper->setAlignment(llvm::Align(4));
        }
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
        return finish_target_machine(*builder, out, error);
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception creating LLVM target");
    }
}

extern "C" int32_t ckc_llvm_target_create_explicit(
    CkcLlvmBytes triple_bytes, CkcLlvmBytes cpu_bytes,
    CkcLlvmBytes feature_bytes, CkcLlvmTarget **out,
    CkcLlvmError *error) {
    clear_error(error);
    if (out == nullptr) {
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "LLVM explicit target output is null");
    }
    *out = nullptr;
    try {
        if (auto init_error = initialize_host_target()) {
            return set_llvm_error(error, std::move(init_error));
        }
        auto triple = llvm::Triple::normalize(borrowed_string(triple_bytes));
        auto cpu = borrowed_string(cpu_bytes).str();
        auto features = borrowed_string(feature_bytes).str();
        if (triple.empty() || cpu.empty()) {
            return invalid(error,
                           "LLVM explicit target triple or CPU is empty");
        }
        auto detected_builder = llvm::orc::JITTargetMachineBuilder::detectHost();
        if (!detected_builder) {
            return set_llvm_error(error, detected_builder.takeError());
        }
        auto requested = llvm::Triple(triple);
        auto detected = detected_builder->getTargetTriple();
        if (requested.getArch() != detected.getArch() ||
            requested.getOS() != detected.getOS() ||
            requested.getEnvironment() != detected.getEnvironment()) {
            return invalid(
                error,
                "explicit LLVM feature target must match the build host ABI");
        }
        llvm::orc::JITTargetMachineBuilder builder(std::move(requested));
        builder.setCPU(cpu);
        builder.setFeatures(features);
        return finish_target_machine(builder, out, error);
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception creating explicit LLVM target");
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

extern "C" int32_t ckc_llvm_target_layout(CkcLlvmTarget *target,
                                             uint32_t *pointer_width_bits,
                                             uint32_t *little_endian,
                                             CkcLlvmError *error) {
    return guarded(error, "reading LLVM target layout", [&] {
        if (target == nullptr || target->value == nullptr ||
            pointer_width_bits == nullptr || little_endian == nullptr) {
            return invalid(error, "LLVM target layout input or output is null");
        }
        const llvm::DataLayout layout = target->value->createDataLayout();
        *pointer_width_bits = layout.getPointerSizeInBits();
        *little_endian = layout.isLittleEndian() ? 1u : 0u;
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_target_profile_query(
    CkcLlvmTarget *target, const CkcLlvmTargetProfileQuery *query,
    CkcLlvmTargetProfileResult *out, CkcLlvmError *error) {
    return guarded(error, "querying LLVM target profile", [&] {
        if (target == nullptr || target->value == nullptr ||
            target->profile_context == nullptr ||
            target->profile_module == nullptr ||
            target->profile_function == nullptr || query == nullptr ||
            out == nullptr) {
            return invalid(error,
                           "LLVM target profile input or output is null");
        }
        out->available = 0;
        out->cost = 0;
        out->legalization_parts = 0;
        out->maximum_interleave_factor = 1;
        clear_bytes(&out->legalized_type);
        if (query->lanes == 0 || query->lanes > 16 || query->alignment > 64) {
            return invalid(error, "LLVM target profile query is out of range");
        }

        llvm::LLVMContext &context = *target->profile_context;
        const llvm::TargetTransformInfo tti =
            target->value->getTargetTransformInfo(*target->profile_function);
        llvm::Type *value_type =
            profile_value_type(context, query->lane, query->lanes);
        if (value_type == nullptr) {
            return invalid(error, "LLVM target profile lane is invalid");
        }
        llvm::Type *legalization_type = value_type;
        if (query->operation == 16) {
            legalization_type = profile_value_type(context, 5, query->lanes);
        } else if (query->operation == 8) {
            llvm::Type *mask_element = llvm::Type::getInt1Ty(context);
            legalization_type = query->lanes == 1
                                    ? mask_element
                                    : llvm::FixedVectorType::get(mask_element,
                                                                 query->lanes);
        }
        const unsigned parts = tti.getNumberOfParts(legalization_type);
        const unsigned interleave = tti.getMaxInterleaveFactor(
            llvm::ElementCount::getFixed(query->lanes));
        out->maximum_interleave_factor = std::max(1u, interleave);
        if (parts == 0 || parts > std::numeric_limits<uint32_t>::max()) {
            return CKC_LLVM_OK;
        }
        const std::string legalized =
            profile_legalized_type(legalization_type, query->lanes, parts);
        if (legalized.empty()) {
            return CKC_LLVM_OK;
        }
        const llvm::InstructionCost cost =
            profile_operation_cost(tti, *query, value_type);
        if (!cost.isValid()) {
            return CKC_LLVM_OK;
        }
        const int64_t numeric = cost.getValue();
        if (numeric < 0 ||
            static_cast<uint64_t>(numeric) >
                std::numeric_limits<uint32_t>::max()) {
            return CKC_LLVM_OK;
        }
        if (!copy_bytes(legalized, &out->legalized_type)) {
            return set_error(error, CKC_LLVM_OUT_OF_MEMORY,
                             "allocating LLVM legalized type failed");
        }
        out->available = 1;
        // CK's closed cost model never treats emitted structural work as free.
        // LLVM legitimately reports zero-throughput inserts/extracts on some
        // targets, so normalize those legal operations to one structural unit.
        out->cost = static_cast<uint32_t>(std::max<int64_t>(1, numeric));
        out->legalization_parts = parts;
        return CKC_LLVM_OK;
    });
}

namespace {

constexpr uint32_t CKC_X86_REDUCTION_INTERLEAVE = 8;

bool is_integer_memory_reduction(const llvm::Loop &loop) {
    const auto *header = loop.getHeader();
    for (const llvm::PHINode &phi : header->phis()) {
        if (!phi.getType()->isIntegerTy()) {
            continue;
        }
        for (unsigned index = 0; index < phi.getNumIncomingValues(); ++index) {
            if (!loop.contains(phi.getIncomingBlock(index))) {
                continue;
            }
            const auto *binary = llvm::dyn_cast<llvm::BinaryOperator>(
                phi.getIncomingValue(index));
            if (binary == nullptr ||
                (binary->getOpcode() != llvm::Instruction::Add &&
                 binary->getOpcode() != llvm::Instruction::Mul)) {
                continue;
            }
            const llvm::Value *other = nullptr;
            if (binary->getOperand(0) == &phi) {
                other = binary->getOperand(1);
            } else if (binary->getOperand(1) == &phi) {
                other = binary->getOperand(0);
            }
            if (llvm::isa_and_nonnull<llvm::LoadInst>(other)) {
                return true;
            }
        }
    }
    return false;
}

bool may_contain_nonlocal_load(const llvm::Function &function) {
    for (const llvm::BasicBlock &block : function) {
        for (const llvm::Instruction &instruction : block) {
            const auto *load = llvm::dyn_cast<llvm::LoadInst>(&instruction);
            if (load != nullptr &&
                !llvm::isa<llvm::AllocaInst>(
                    load->getPointerOperand()->stripPointerCasts())) {
                return true;
            }
        }
    }
    return false;
}

void attach_x86_integer_reduction_interleave(
    llvm::Module &module, const llvm::TargetMachine &target) {
    if (target.getTargetTriple().getArch() != llvm::Triple::x86_64) {
        return;
    }
    llvm::SmallVector<llvm::Function *, 16> production_functions;
    for (llvm::Function &function : module) {
        production_functions.push_back(&function);
    }
    for (llvm::Function *function : production_functions) {
        if (function->isDeclaration() || function->empty() ||
            !may_contain_nonlocal_load(*function)) {
            continue;
        }
        llvm::ValueToValueMapTy clone_map;
        llvm::Function *attached_clone =
            llvm::CloneFunction(function, clone_map);
        llvm::SmallVector<llvm::AllocaInst *, 16> allocas;
        for (llvm::Instruction &instruction :
             attached_clone->getEntryBlock()) {
            auto *alloca = llvm::dyn_cast<llvm::AllocaInst>(&instruction);
            if (alloca != nullptr && llvm::isAllocaPromotable(alloca)) {
                allocas.push_back(alloca);
            }
        }
        llvm::DominatorTree clone_dominators(*attached_clone);
        if (!allocas.empty()) {
            llvm::PromoteMemToReg(allocas, clone_dominators);
        }
        llvm::LoopInfo clone_loops(clone_dominators);
        llvm::DominatorTree production_dominators(*function);
        llvm::LoopInfo production_loops(production_dominators);
        for (llvm::Loop *loop : production_loops.getLoopsInPreorder()) {
            auto *clone_header = llvm::dyn_cast_or_null<llvm::BasicBlock>(
                clone_map.lookup(loop->getHeader()));
            if (clone_header == nullptr) {
                continue;
            }
            llvm::Loop *clone_loop = clone_loops.getLoopFor(clone_header);
            if (clone_loop == nullptr ||
                clone_loop->getHeader() != clone_header ||
                !is_integer_memory_reduction(*clone_loop)) {
                continue;
            }
            auto &context = module.getContext();
            auto *count = llvm::MDNode::get(
                context,
                {llvm::MDString::get(context, "llvm.loop.interleave.count"),
                 llvm::ConstantAsMetadata::get(llvm::ConstantInt::get(
                     llvm::Type::getInt32Ty(context),
                     CKC_X86_REDUCTION_INTERLEAVE))});
            llvm::Metadata *operands[] = {nullptr, count};
            auto *loop_id = llvm::MDNode::getDistinct(context, operands);
            loop_id->replaceOperandWith(0, loop_id);
            llvm::SmallVector<llvm::BasicBlock *, 4> latches;
            loop->getLoopLatches(latches);
            for (llvm::BasicBlock *latch : latches) {
                latch->getTerminator()->setMetadata(llvm::LLVMContext::MD_loop,
                                                    loop_id);
            }
        }
        attached_clone->eraseFromParent();
    }
}

} // namespace

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

        if (level == llvm::OptimizationLevel::O3) {
            attach_x86_integer_reduction_interleave(*module->value,
                                                     *target->value);
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

extern "C" int32_t ckc_llvm_module_apply_late_layout(
    CkcLlvmModule *module, CkcLlvmTarget *target, CkcLlvmBytes plan,
    CkcLlvmLateLayoutReport *out, CkcLlvmError *error) {
    clear_error(error);
    if (out != nullptr) {
        std::memset(out, 0, sizeof(*out));
        clear_bytes(&out->reason);
    }
    return guarded(error, "applying CK late profile layout", [&] {
        if (module == nullptr || module->value == nullptr || target == nullptr ||
            target->value == nullptr || out == nullptr) {
            return invalid(error, "late profile layout input is invalid");
        }
        const std::string pre_layout = late_layout_snapshot(*module->value, false);
        const std::string pre_structural = late_layout_snapshot(*module->value, true);
        sha256_text(pre_layout, out->pre_layout_digest);
        sha256_text(pre_structural, out->pre_structural_digest);
        std::copy(std::begin(out->pre_layout_digest),
                  std::end(out->pre_layout_digest), out->post_layout_digest);
        std::copy(std::begin(out->pre_structural_digest),
                  std::end(out->pre_structural_digest),
                  out->post_structural_digest);

        std::string parse_failure;
        auto parsed = parse_late_layout_plan(plan, parse_failure);
        if (!parsed) {
            return invalid(error, parse_failure);
        }
        const llvm::Triple &triple = target->value->getTargetTriple();
        if (!late_layout_target_supported(triple)) {
            if (!copy_bytes("unsupported-target-repair", &out->reason)) {
                return set_error(error, CKC_LLVM_OUT_OF_MEMORY,
                                 "allocating late layout reason failed");
            }
            return CKC_LLVM_OK;
        }
        if (parsed->empty()) {
            if (!copy_bytes("no-layout-authority", &out->reason)) {
                return set_error(error, CKC_LLVM_OUT_OF_MEMORY,
                                 "allocating late layout reason failed");
            }
            return CKC_LLVM_OK;
        }

        std::vector<std::pair<llvm::Function *, std::vector<llvm::BasicBlock *>>>
            resolved;
        for (const CkcLateLayoutDirective &directive : *parsed) {
            llvm::Function *function =
                module->value->getFunction(directive.function);
            if (function == nullptr || function->isDeclaration() ||
                function->empty()) {
                return invalid(error, "late layout names an unknown function");
            }
            std::vector<llvm::BasicBlock *> blocks;
            for (const std::string &name : directive.blocks) {
                llvm::BasicBlock *found = nullptr;
                for (llvm::BasicBlock &block : *function) {
                    if (block.getName() == name) {
                        found = &block;
                        break;
                    }
                }
                if (found == nullptr) {
                    return invalid(error, "late layout names an unknown block");
                }
                if (found == &function->getEntryBlock()) {
                    return invalid(error,
                                   "late layout cannot move the IR entry block");
                }
                blocks.push_back(found);
            }
            resolved.push_back({function, std::move(blocks)});
        }

        for (auto &[function, blocks] : resolved) {
            llvm::BasicBlock *anchor = &function->getEntryBlock();
            for (llvm::BasicBlock *block : blocks) {
                block->moveAfter(anchor);
                anchor = block;
            }
        }
        const std::string post_layout = late_layout_snapshot(*module->value, false);
        const std::string post_structural = late_layout_snapshot(*module->value, true);
        sha256_text(post_layout, out->post_layout_digest);
        sha256_text(post_structural, out->post_structural_digest);
        if (pre_structural != post_structural) {
            return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                             "late layout changed non-layout structure");
        }
        out->accepted = 1;
        out->changed = pre_layout != post_layout;
        // The target emission pipeline performs these closed repairs after the
        // verified permutation; no additional optimizing pass is introduced.
        out->repair_mask = 1u | 8u;
        out->repair_mask |= triple.getArch() == llvm::Triple::aarch64 ? 2u : 4u;
        const std::string_view reason =
            out->changed != 0 ? "accepted" : "accepted-no-order-delta";
        if (!copy_bytes(reason, &out->reason)) {
            return set_error(error, CKC_LLVM_OUT_OF_MEMORY,
                             "allocating late layout reason failed");
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

extern "C" int32_t ckc_llvm_module_test_inject_untracked_strengthening(
    CkcLlvmModule *module, CkcLlvmError *error) {
    return guarded(error, "injecting untracked LLVM strengthening", [&] {
        if (module == nullptr || module->value == nullptr) {
            return invalid(error, "untracked-strengthening test module is null");
        }
        auto &context = module->value->getContext();
        auto *pointer = llvm::PointerType::get(context, 0);
        auto *type = llvm::FunctionType::get(llvm::Type::getVoidTy(context),
                                             {pointer}, false);
        auto *probe = llvm::Function::Create(
            type, llvm::GlobalValue::ExternalLinkage, "__ck_untracked_probe",
            *module->value);
        probe->addParamAttr(
            0, llvm::Attribute::get(context, llvm::Attribute::NoAlias));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_module_test_inject_untracked_flag(
    CkcLlvmModule *module, CkcLlvmError *error) {
    return guarded(error, "injecting untracked LLVM flag", [&] {
        if (module == nullptr || module->value == nullptr) {
            return invalid(error, "untracked-flag test module is null");
        }
        auto &context = module->value->getContext();
        auto *i32 = llvm::Type::getInt32Ty(context);
        auto *type = llvm::FunctionType::get(i32, {i32}, false);
        auto *probe = llvm::Function::Create(
            type, llvm::GlobalValue::ExternalLinkage, "__ck_untracked_flag_probe",
            *module->value);
        auto *entry = llvm::BasicBlock::Create(context, "entry", probe);
        llvm::IRBuilder<> builder(entry);
        auto *incremented = llvm::cast<llvm::BinaryOperator>(
            builder.CreateAdd(probe->getArg(0), llvm::ConstantInt::get(i32, 1),
                              "untracked.nuw"));
        incremented->setHasNoUnsignedWrap(true);
        builder.CreateRet(incremented);
        return CKC_LLVM_OK;
    });
}

static bool ckc_alias_scope_id(llvm::Metadata *metadata, uint32_t *out) {
    auto *node = llvm::dyn_cast_or_null<llvm::MDNode>(metadata);
    if (node == nullptr || node->getNumOperands() < 3 || out == nullptr) {
        return false;
    }
    auto *name = llvm::dyn_cast_or_null<llvm::MDString>(node->getOperand(2).get());
    if (name == nullptr) {
        return false;
    }
    auto text = name->getString();
    if (!text.consume_front("ck.alias.scope.")) {
        return false;
    }
    uint32_t value = 0;
    if (text.getAsInteger(10, value) || value == 0) {
        return false;
    }
    *out = value;
    return true;
}

extern "C" int32_t ckc_llvm_module_fact_audit_counts(
    CkcLlvmModule *module, CkcLlvmFactAuditCounts *out,
    CkcLlvmError *error) {
    return guarded(error, "enumerating LLVM strengthenings", [&] {
        if (module == nullptr || module->value == nullptr || out == nullptr) {
            return invalid(error, "LLVM strengthening audit input or output is null");
        }
        *out = {};
        std::set<std::pair<uint32_t, uint32_t>> alias_pairs;
        for (auto &function : *module->value) {
            for (unsigned index = 0; index < function.arg_size(); ++index) {
                if (function.hasParamAttribute(index, llvm::Attribute::NoAlias)) {
                    ++out->parameter_noalias;
                }
                if (function.hasParamAttribute(index, llvm::Attribute::ReadOnly)) {
                    ++out->readonly_count;
                }
                if (function.hasParamAttribute(index, llvm::Attribute::WriteOnly)) {
                    ++out->writeonly_count;
                }
                const bool aggregate_abi =
                    function.hasParamAttribute(index, llvm::Attribute::StructRet) ||
                    function.hasParamAttribute(index, llvm::Attribute::ByVal);
                if (!aggregate_abi &&
                    function.hasParamAttribute(index, llvm::Attribute::Alignment)) {
                    ++out->alignment;
                }
            }
            if (!function.isDeclaration()) {
                auto effects = function.getMemoryEffects();
                if (effects == llvm::MemoryEffects::none() ||
                    effects == llvm::MemoryEffects::readOnly() ||
                    effects == llvm::MemoryEffects::writeOnly()) {
                    ++out->memory_effects;
                }
            }
            for (auto &block : function) {
                for (auto &instruction : block) {
                    if (auto *binary =
                            llvm::dyn_cast<llvm::OverflowingBinaryOperator>(&instruction)) {
                        if (binary->hasNoUnsignedWrap()) {
                            ++out->no_unsigned_wrap;
                        }
                        if (binary->hasNoSignedWrap()) {
                            ++out->no_signed_wrap;
                        }
                    }
                    if (auto *call = llvm::dyn_cast<llvm::CallBase>(&instruction)) {
                        auto *callee = call->getCalledFunction();
                        if (callee != nullptr &&
                            callee->getIntrinsicID() == llvm::Intrinsic::assume) {
                            ++out->assume_count;
                            ++out->range;
                        }
                    }
                    auto *aliases = instruction.getMetadata(
                        llvm::LLVMContext::MD_alias_scope);
                    auto *noaliases = instruction.getMetadata(
                        llvm::LLVMContext::MD_noalias);
                    if (aliases == nullptr || noaliases == nullptr) {
                        continue;
                    }
                    for (const auto &alias_operand : aliases->operands()) {
                        uint32_t alias = 0;
                        if (!ckc_alias_scope_id(alias_operand.get(), &alias)) {
                            continue;
                        }
                        for (const auto &noalias_operand : noaliases->operands()) {
                            uint32_t noalias = 0;
                            if (!ckc_alias_scope_id(noalias_operand.get(), &noalias) ||
                                alias == noalias) {
                                continue;
                            }
                            alias_pairs.emplace(std::min(alias, noalias),
                                                std::max(alias, noalias));
                        }
                    }
                }
            }
        }
        out->alias_scope = alias_pairs.size();
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_module_expose_hidden_function(
    CkcLlvmModule *module, CkcLlvmBytes function_name,
    CkcLlvmError *error) {
    return guarded(error, "exposing hidden multiversion function", [&] {
        if (module == nullptr || module->value == nullptr) {
            return invalid(error, "multiversion module is null");
        }
        const auto name = borrowed_string(function_name);
        auto *function = module->value->getFunction(name);
        if (name.empty() || function == nullptr || function->isDeclaration()) {
            return invalid(error,
                           "multiversion hidden function is missing or undefined");
        }
        function->setLinkage(llvm::GlobalValue::ExternalLinkage);
        function->setVisibility(llvm::GlobalValue::HiddenVisibility);
        function->setDSOLocal(true);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_module_add_multiversion_dispatch(
    CkcLlvmModule *module, CkcLlvmBytes public_name_bytes,
    CkcLlvmBytes implementation_name_bytes,
    CkcLlvmBytes baseline_hidden_name_bytes,
    CkcLlvmBytes dispatch_namespace_bytes,
    const CkcLlvmBytes *variant_name_bytes,
    const uint32_t *required_capabilities, size_t variant_count,
    CkcLlvmError *error) {
    return guarded(error, "building LLVM multiversion dispatcher", [&] {
        if (module == nullptr || module->value == nullptr ||
            (variant_count != 0 &&
             (variant_name_bytes == nullptr || required_capabilities == nullptr))) {
            return invalid(error, "multiversion dispatch input is null");
        }
        auto &llvm_module = *module->value;
        auto &context = llvm_module.getContext();
        const auto public_name = borrowed_string(public_name_bytes);
        const auto implementation_name =
            borrowed_string(implementation_name_bytes);
        const auto baseline_hidden_name =
            borrowed_string(baseline_hidden_name_bytes);
        const auto dispatch_namespace =
            borrowed_string(dispatch_namespace_bytes);
        if (public_name.empty() || implementation_name.empty() ||
            baseline_hidden_name.empty() || dispatch_namespace.empty() ||
            implementation_name == baseline_hidden_name ||
            llvm_module.getNamedValue(baseline_hidden_name) != nullptr) {
            return invalid(error, "multiversion dispatch names are invalid or collide");
        }
        auto *public_thunk = llvm_module.getFunction(public_name);
        auto *baseline = llvm_module.getFunction(implementation_name);
        if (public_thunk == nullptr || public_thunk->isDeclaration() ||
            baseline == nullptr || baseline->isDeclaration()) {
            return invalid(error,
                           "multiversion public thunk or baseline implementation is missing");
        }

        auto *function_type = baseline->getFunctionType();
        baseline->setName(baseline_hidden_name);
        baseline->setLinkage(llvm::GlobalValue::ExternalLinkage);
        baseline->setVisibility(llvm::GlobalValue::HiddenVisibility);
        baseline->setDSOLocal(true);

        std::vector<llvm::Function *> variants;
        variants.reserve(variant_count);
        std::set<std::string> names;
        for (size_t index = 0; index < variant_count; ++index) {
            const auto name = borrowed_string(variant_name_bytes[index]);
            if (name.empty() || name == baseline_hidden_name ||
                !names.insert(name.str()).second ||
                llvm_module.getNamedValue(name) != nullptr) {
                return invalid(error,
                               "multiversion variant name is empty, duplicated, or collides");
            }
            auto *variant = llvm::Function::Create(
                function_type, llvm::GlobalValue::ExternalLinkage, name,
                llvm_module);
            variant->setVisibility(llvm::GlobalValue::HiddenVisibility);
            variant->setDSOLocal(true);
            variant->setCallingConv(baseline->getCallingConv());
            variants.push_back(variant);
        }

        const std::string stem = "__ck_mv_" + dispatch_namespace.str() +
                                 "_" + public_name.str();
        auto *pointer_type = llvm::PointerType::get(context, 0);
        auto *null_pointer = llvm::ConstantPointerNull::get(pointer_type);
        auto *slot = new llvm::GlobalVariable(
            llvm_module, pointer_type, false,
            llvm::GlobalValue::InternalLinkage, null_pointer,
            stem + "_slot");
        slot->setAlignment(llvm::Align(8));

        auto *i32_type = llvm::Type::getInt32Ty(context);
        auto *detector_type = llvm::FunctionType::get(i32_type, false);
        auto detector = llvm_module.getOrInsertFunction(
            "__ck_dispatch_detect_capabilities", detector_type);
        if (auto *detector_function =
                llvm::dyn_cast<llvm::Function>(detector.getCallee())) {
            detector_function->setVisibility(
                llvm::GlobalValue::HiddenVisibility);
        }

        auto *resolver_type = llvm::FunctionType::get(pointer_type, false);
        auto *resolver = llvm::Function::Create(
            resolver_type, llvm::GlobalValue::InternalLinkage,
            stem + "_resolve", llvm_module);
        resolver->addFnAttr(llvm::Attribute::NoInline);
        resolver->addFnAttr(llvm::Attribute::Cold);
        auto *resolver_entry =
            llvm::BasicBlock::Create(context, "entry", resolver);
        llvm::IRBuilder<> resolver_builder(resolver_entry);
        auto *capabilities = resolver_builder.CreateCall(
            detector_type, detector.getCallee(), {}, "ck.capabilities");
        llvm::Value *selected = baseline;
        for (size_t reverse = variant_count; reverse > 0; --reverse) {
            const size_t index = reverse - 1;
            const uint32_t required = required_capabilities[index];
            if (required == 0u || (required & ~0x0fu) != 0u) {
                return invalid(error,
                               "multiversion requirement is baseline or outside the closed set");
            }
            auto *required_value = llvm::ConstantInt::get(i32_type, required);
            auto *masked = resolver_builder.CreateAnd(
                capabilities, required_value, "ck.capability.mask");
            auto *compatible = resolver_builder.CreateICmpEQ(
                masked, required_value, "ck.capability.compatible");
            selected = resolver_builder.CreateSelect(
                compatible, variants[index], selected, "ck.selected");
        }
        auto *publication = resolver_builder.CreateAtomicCmpXchg(
            slot, null_pointer, selected, llvm::Align(8),
            llvm::AtomicOrdering::AcquireRelease,
            llvm::AtomicOrdering::Acquire);
        publication->setWeak(false);
        auto *winner = resolver_builder.CreateExtractValue(
            publication, 0, "ck.published.pointer");
        auto *published = resolver_builder.CreateExtractValue(
            publication, 1, "ck.publication.won");
        auto *resolved = resolver_builder.CreateSelect(
            published, selected, winner, "ck.resolved.pointer");
        resolver_builder.CreateRet(resolved);

        auto *dispatcher = llvm::Function::Create(
            function_type, llvm::GlobalValue::InternalLinkage,
            implementation_name, llvm_module);
        dispatcher->setCallingConv(baseline->getCallingConv());
        dispatcher->setAttributes(baseline->getAttributes());
        dispatcher->addFnAttr(llvm::Attribute::AlwaysInline);
        auto *entry = llvm::BasicBlock::Create(context, "entry", dispatcher);
        auto *slow = llvm::BasicBlock::Create(context, "resolve", dispatcher);
        auto *invoke = llvm::BasicBlock::Create(context, "invoke", dispatcher);
        llvm::IRBuilder<> dispatch_builder(entry);
        auto *cached = dispatch_builder.CreateLoad(pointer_type, slot,
                                                   "ck.dispatch.pointer");
        cached->setAtomic(llvm::AtomicOrdering::Acquire);
        cached->setAlignment(llvm::Align(8));
        auto *uninitialized = dispatch_builder.CreateICmpEQ(
            cached, null_pointer, "ck.dispatch.uninitialized");
        dispatch_builder.CreateCondBr(uninitialized, slow, invoke);
        dispatch_builder.SetInsertPoint(slow);
        auto *fresh = dispatch_builder.CreateCall(resolver, {}, "ck.dispatch.fresh");
        dispatch_builder.CreateBr(invoke);
        dispatch_builder.SetInsertPoint(invoke);
        auto *pointer = dispatch_builder.CreatePHI(pointer_type, 2,
                                                  "ck.dispatch.target");
        pointer->addIncoming(cached, entry);
        pointer->addIncoming(fresh, slow);
        std::vector<llvm::Value *> arguments;
        arguments.reserve(dispatcher->arg_size());
        for (auto &argument : dispatcher->args()) {
            arguments.push_back(&argument);
        }
        auto *call = function_type->getReturnType()->isVoidTy()
                         ? dispatch_builder.CreateCall(function_type, pointer, arguments)
                         : dispatch_builder.CreateCall(function_type, pointer, arguments,
                                                       "ck.dispatch.call");
        call->setCallingConv(baseline->getCallingConv());
        call->setAttributes(baseline->getAttributes());
        call->setTailCallKind(llvm::CallInst::TCK_MustTail);
        if (function_type->getReturnType()->isVoidTy()) {
            dispatch_builder.CreateRetVoid();
        } else {
            dispatch_builder.CreateRet(call);
        }

        size_t rewritten = 0;
        for (auto &block : *public_thunk) {
            for (auto &instruction : block) {
                auto *call_base = llvm::dyn_cast<llvm::CallBase>(&instruction);
                if (call_base != nullptr &&
                    call_base->getCalledOperand()->stripPointerCasts() == baseline) {
                    call_base->setCalledFunction(dispatcher);
                    ++rewritten;
                }
            }
        }
        if (rewritten != 1) {
            return invalid(error,
                           "multiversion public thunk must contain exactly one baseline call");
        }
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

extern "C" int32_t ckc_llvm_type_fixed_vector(CkcLlvmType *element,
                                                 uint32_t count,
                                                 CkcLlvmType **out,
                                                 CkcLlvmError *error) {
    return guarded(error, "creating LLVM fixed vector type", [&] {
        if (element == nullptr || out == nullptr || count < 2 || count > 16) {
            return invalid(error, "LLVM fixed vector type input is invalid");
        }
        llvm::Type *lane = llvm_type(element);
        if (!lane->isIntegerTy() && !lane->isDoubleTy()) {
            return invalid(error, "LLVM fixed vector lane type is invalid");
        }
        *out = bridge_type(llvm::FixedVectorType::get(lane, count));
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
        if (exported != 0) {
            function->setAlignment(llvm::Align(64));
        }
        *out = bridge_function(function);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_module_add_global_bytes(
    CkcLlvmModule *module, CkcLlvmBytes name, const uint8_t *bytes,
    size_t byte_count, uint32_t mutable_storage, uint32_t alignment,
    CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "adding LLVM byte global", [&] {
        if (module == nullptr || module->value == nullptr || out == nullptr ||
            byte_count == 0 || bytes == nullptr || alignment == 0 ||
            !llvm::isPowerOf2_32(alignment)) {
            return invalid(error, "LLVM byte global input or output is invalid");
        }
        auto initializer = llvm::ConstantDataArray::get(
            module->value->getContext(), llvm::ArrayRef<uint8_t>(bytes, byte_count));
        auto *global = new llvm::GlobalVariable(
            *module->value, initializer->getType(), mutable_storage == 0,
            llvm::GlobalValue::InternalLinkage, initializer, borrowed_string(name));
        global->setAlignment(llvm::Align(alignment));
        global->setUnnamedAddr(mutable_storage == 0
                                   ? llvm::GlobalValue::UnnamedAddr::Global
                                   : llvm::GlobalValue::UnnamedAddr::None);
        *out = bridge_value(global);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_module_add_global_u32_array(
    CkcLlvmModule *module, CkcLlvmBytes name, const uint32_t *values,
    size_t value_count, uint32_t alignment, CkcLlvmValue **out,
    CkcLlvmError *error) {
    return guarded(error, "adding LLVM u32 global", [&] {
        if (module == nullptr || module->value == nullptr || out == nullptr ||
            value_count == 0 || values == nullptr || alignment == 0 ||
            !llvm::isPowerOf2_32(alignment)) {
            return invalid(error, "LLVM u32 global input or output is invalid");
        }
        auto initializer = llvm::ConstantDataArray::get(
            module->value->getContext(), llvm::ArrayRef<uint32_t>(values, value_count));
        auto *global = new llvm::GlobalVariable(
            *module->value, initializer->getType(), true,
            llvm::GlobalValue::InternalLinkage, initializer, borrowed_string(name));
        global->setAlignment(llvm::Align(alignment));
        global->setUnnamedAddr(llvm::GlobalValue::UnnamedAddr::Global);
        *out = bridge_value(global);
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
        case CKC_LLVM_ATTR_NOALIAS:
            attribute = llvm::Attribute::get(value->getContext(), llvm::Attribute::NoAlias);
            break;
        case CKC_LLVM_ATTR_READONLY:
            attribute = llvm::Attribute::get(value->getContext(), llvm::Attribute::ReadOnly);
            break;
        case CKC_LLVM_ATTR_WRITEONLY:
            attribute = llvm::Attribute::get(value->getContext(), llvm::Attribute::WriteOnly);
            break;
        case CKC_LLVM_ATTR_ALIGN:
            if (alignment == 0 || (alignment & (alignment - 1)) != 0) {
                return invalid(error, "align requires a nonzero power-of-two alignment");
            }
            attribute = llvm::Attribute::getWithAlignment(value->getContext(),
                                                          llvm::Align(alignment));
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

extern "C" int32_t ckc_llvm_function_set_memory_effects(
    CkcLlvmFunction *function, uint32_t effects, CkcLlvmError *error) {
    return guarded(error, "setting LLVM function memory effects", [&] {
        auto *value = llvm_function(function);
        if (value == nullptr) {
            return invalid(error, "LLVM memory-effects function is null");
        }
        switch (effects) {
        case CKC_LLVM_MEMORY_NONE:
            value->setMemoryEffects(llvm::MemoryEffects::none());
            break;
        case CKC_LLVM_MEMORY_READ:
            value->setMemoryEffects(llvm::MemoryEffects::readOnly());
            break;
        case CKC_LLVM_MEMORY_WRITE:
            value->setMemoryEffects(llvm::MemoryEffects::writeOnly());
            break;
        default:
            return invalid(error, "unknown LLVM function memory effects");
        }
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_function_set_profile(
    CkcLlvmFunction *function, uint64_t entry_count, uint32_t hot,
    uint32_t cold, CkcLlvmError *error) {
    return guarded(error, "setting checked LLVM function profile", [&] {
        auto *value = llvm_function(function);
        if (value == nullptr) {
            return invalid(error, "LLVM profile function is null");
        }
        if (hot > 1 || cold > 1 || (hot != 0 && cold != 0)) {
            return invalid(error, "LLVM function profile classification is invalid");
        }
        value->setEntryCount(entry_count, llvm::Function::PCT_Real);
        if (hot != 0) {
            value->addFnAttr(llvm::Attribute::Hot);
        }
        if (cold != 0) {
            value->addFnAttr(llvm::Attribute::Cold);
            // CK has already rejected size-increasing inline materialization
            // for a checked profile-cold call path. Preserve that verified
            // decision across LLVM's independent inliner while retaining the
            // complete generic function as the semantic fallback.
            value->addFnAttr(llvm::Attribute::NoInline);
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

static llvm::MDNode *ckc_alias_scope(CkcLlvmBuilder *builder, uint32_t id) {
    auto existing = builder->alias_scopes.find(id);
    if (existing != builder->alias_scopes.end()) {
        return existing->second;
    }
    auto &context = builder->value->getContext();
    if (builder->alias_domain == nullptr) {
        builder->alias_domain = llvm::MDNode::getDistinct(
            context, {nullptr, llvm::MDString::get(context, "ck.alias.domain")});
        builder->alias_domain->replaceOperandWith(0, builder->alias_domain);
    }
    auto *scope = llvm::MDNode::getDistinct(
        context,
        {nullptr, builder->alias_domain,
         llvm::MDString::get(context, "ck.alias.scope." + std::to_string(id))});
    scope->replaceOperandWith(0, scope);
    builder->alias_scopes.emplace(id, scope);
    return scope;
}

static int32_t ckc_set_alias_metadata(
    CkcLlvmBuilder *builder, llvm::Instruction *instruction,
    const uint32_t *alias_scopes, size_t alias_scope_count,
    const uint32_t *noalias_scopes, size_t noalias_scope_count,
    CkcLlvmError *error) {
    if ((alias_scope_count != 0 && alias_scopes == nullptr) ||
        (noalias_scope_count != 0 && noalias_scopes == nullptr)) {
        return invalid(error, "LLVM scoped alias metadata array is null");
    }
    auto &context = builder->value->getContext();
    llvm::SmallVector<llvm::Metadata *, 4> aliases;
    llvm::SmallVector<llvm::Metadata *, 4> noaliases;
    for (size_t index = 0; index < alias_scope_count; ++index) {
        if (alias_scopes[index] == 0) {
            return invalid(error, "LLVM alias scope identity must be nonzero");
        }
        aliases.push_back(ckc_alias_scope(builder, alias_scopes[index]));
    }
    for (size_t index = 0; index < noalias_scope_count; ++index) {
        if (noalias_scopes[index] == 0) {
            return invalid(error, "LLVM noalias scope identity must be nonzero");
        }
        noaliases.push_back(ckc_alias_scope(builder, noalias_scopes[index]));
    }
    if (!aliases.empty()) {
        instruction->setMetadata(llvm::LLVMContext::MD_alias_scope,
                                 llvm::MDNode::get(context, aliases));
    }
    if (!noaliases.empty()) {
        instruction->setMetadata(llvm::LLVMContext::MD_noalias,
                                 llvm::MDNode::get(context, noaliases));
    }
    return CKC_LLVM_OK;
}

extern "C" int32_t ckc_llvm_builder_load_scoped_alias(
    CkcLlvmBuilder *builder, CkcLlvmType *type, CkcLlvmValue *pointer,
    const uint32_t *alias_scopes, size_t alias_scope_count,
    const uint32_t *noalias_scopes, size_t noalias_scope_count,
    CkcLlvmBytes name, CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "building LLVM scoped-alias load", [&] {
        if (builder == nullptr || builder->value == nullptr || type == nullptr ||
            pointer == nullptr || out == nullptr) {
            return invalid(error, "LLVM scoped-alias load input or output is null");
        }
        auto *load = builder->value->CreateLoad(
            llvm_type(type), llvm_value(pointer), borrowed_string(name));
        auto status = ckc_set_alias_metadata(
            builder, load, alias_scopes, alias_scope_count, noalias_scopes,
            noalias_scope_count, error);
        if (status != CKC_LLVM_OK) {
            load->eraseFromParent();
            return status;
        }
        *out = bridge_value(load);
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

extern "C" int32_t ckc_llvm_builder_store_scoped_alias(
    CkcLlvmBuilder *builder, CkcLlvmValue *value, CkcLlvmValue *pointer,
    const uint32_t *alias_scopes, size_t alias_scope_count,
    const uint32_t *noalias_scopes, size_t noalias_scope_count,
    CkcLlvmError *error) {
    return guarded(error, "building LLVM scoped-alias store", [&] {
        if (builder == nullptr || builder->value == nullptr || value == nullptr ||
            pointer == nullptr) {
            return invalid(error, "LLVM scoped-alias store input is null");
        }
        auto *store = builder->value->CreateStore(llvm_value(value), llvm_value(pointer));
        auto status = ckc_set_alias_metadata(
            builder, store, alias_scopes, alias_scope_count, noalias_scopes,
            noalias_scope_count, error);
        if (status != CKC_LLVM_OK) {
            store->eraseFromParent();
            return status;
        }
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_vector_load(
    CkcLlvmBuilder *builder, CkcLlvmType *type, CkcLlvmValue *pointer,
    uint32_t alignment, CkcLlvmBytes name, CkcLlvmValue **out,
    CkcLlvmError *error) {
    return guarded(error, "building LLVM vector load", [&] {
        if (builder == nullptr || builder->value == nullptr || type == nullptr ||
            pointer == nullptr || out == nullptr || alignment == 0 ||
            !llvm::isPowerOf2_32(alignment) ||
            !llvm_type(type)->isVectorTy()) {
            return invalid(error, "LLVM vector load input is invalid");
        }
        *out = bridge_value(builder->value->CreateAlignedLoad(
            llvm_type(type), llvm_value(pointer), llvm::Align(alignment),
            borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_vector_store(
    CkcLlvmBuilder *builder, CkcLlvmValue *value, CkcLlvmValue *pointer,
    uint32_t alignment, CkcLlvmError *error) {
    return guarded(error, "building LLVM vector store", [&] {
        if (builder == nullptr || builder->value == nullptr || value == nullptr ||
            pointer == nullptr || alignment == 0 ||
            !llvm::isPowerOf2_32(alignment) ||
            !llvm_value(value)->getType()->isVectorTy()) {
            return invalid(error, "LLVM vector store input is invalid");
        }
        builder->value->CreateAlignedStore(llvm_value(value), llvm_value(pointer),
                                           llvm::Align(alignment));
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
    CkcLlvmValue *right, uint32_t no_unsigned_wrap,
    uint32_t no_signed_wrap, CkcLlvmBytes name,
    CkcLlvmValue **out, CkcLlvmError *error) {
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
        if (no_unsigned_wrap != 0 || no_signed_wrap != 0) {
            auto *binary = llvm::dyn_cast<llvm::BinaryOperator>(result);
            if (binary == nullptr) {
                return invalid(error, "wrap flags require an integer binary instruction");
            }
            if (no_unsigned_wrap != 0) {
                binary->setHasNoUnsignedWrap(true);
            }
            if (no_signed_wrap != 0) {
                binary->setHasNoSignedWrap(true);
            }
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
        case CKC_LLVM_PTRTOINT: result = builder->value->CreatePtrToInt(llvm_value(value), llvm_type(target_type), borrowed_string(name)); break;
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

extern "C" int32_t ckc_llvm_builder_vector_splat(
    CkcLlvmBuilder *builder, uint32_t lanes, CkcLlvmValue *scalar,
    CkcLlvmBytes name, CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "building LLVM vector splat", [&] {
        if (builder == nullptr || builder->value == nullptr || scalar == nullptr ||
            out == nullptr || lanes < 2 || lanes > 16) {
            return invalid(error, "LLVM vector splat input is invalid");
        }
        *out = bridge_value(builder->value->CreateVectorSplat(
            lanes, llvm_value(scalar), borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_vector_insert(
    CkcLlvmBuilder *builder, CkcLlvmValue *vector, CkcLlvmValue *scalar,
    uint32_t lane_index, CkcLlvmBytes name, CkcLlvmValue **out,
    CkcLlvmError *error) {
    return guarded(error, "building LLVM vector insert", [&] {
        auto *value = llvm_value(vector);
        auto *vector_type = value == nullptr
                                ? nullptr
                                : llvm::dyn_cast<llvm::FixedVectorType>(
                                      value->getType());
        if (builder == nullptr || builder->value == nullptr ||
            vector_type == nullptr || scalar == nullptr || out == nullptr ||
            lane_index >= vector_type->getNumElements()) {
            return invalid(error, "LLVM vector insert input is invalid");
        }
        *out = bridge_value(builder->value->CreateInsertElement(
            value, llvm_value(scalar), lane_index, borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_vector_extract(
    CkcLlvmBuilder *builder, CkcLlvmValue *vector, uint32_t lane_index,
    CkcLlvmBytes name, CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "building LLVM vector extract", [&] {
        auto *value = llvm_value(vector);
        auto *vector_type = value == nullptr
                                ? nullptr
                                : llvm::dyn_cast<llvm::FixedVectorType>(
                                      value->getType());
        if (builder == nullptr || builder->value == nullptr ||
            vector_type == nullptr || out == nullptr ||
            lane_index >= vector_type->getNumElements()) {
            return invalid(error, "LLVM vector extract input is invalid");
        }
        *out = bridge_value(builder->value->CreateExtractElement(
            value, lane_index, borrowed_string(name)));
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_vector_reduce(
    CkcLlvmBuilder *builder, uint32_t reduction, CkcLlvmValue *vector,
    CkcLlvmBytes name, CkcLlvmValue **out, CkcLlvmError *error) {
    return guarded(error, "building LLVM vector reduction", [&] {
        auto *value = llvm_value(vector);
        if (builder == nullptr || builder->value == nullptr || value == nullptr ||
            !value->getType()->isIntOrIntVectorTy() ||
            !value->getType()->isVectorTy() || out == nullptr) {
            return invalid(error, "LLVM vector reduction input is invalid");
        }
        llvm::Value *result = nullptr;
        switch (reduction) {
        case 1:
            result = builder->value->CreateAddReduce(value);
            break;
        case 2:
            result = builder->value->CreateIntMinReduce(value, true);
            break;
        case 3:
            result = builder->value->CreateIntMinReduce(value, false);
            break;
        case 4:
            result = builder->value->CreateIntMaxReduce(value, true);
            break;
        case 5:
            result = builder->value->CreateIntMaxReduce(value, false);
            break;
        case 6:
            result = builder->value->CreateMulReduce(value);
            break;
        default:
            return invalid(error, "unknown LLVM vector reduction");
        }
        if (auto *instruction = llvm::dyn_cast<llvm::Instruction>(result)) {
            instruction->setName(borrowed_string(name));
        }
        *out = bridge_value(result);
        return CKC_LLVM_OK;
    });
}

extern "C" int32_t ckc_llvm_builder_assume(CkcLlvmBuilder *builder,
                                             CkcLlvmValue *condition,
                                             CkcLlvmError *error) {
    return guarded(error, "building LLVM assume", [&] {
        auto *ir = builder == nullptr ? nullptr : builder->value.get();
        auto *value = llvm_value(condition);
        if (ir == nullptr || value == nullptr || ir->GetInsertBlock() == nullptr ||
            !value->getType()->isIntegerTy(1)) {
            return invalid(error,
                           "LLVM assume requires an active builder and i1 condition");
        }
        auto *intrinsic = llvm::Intrinsic::getOrInsertDeclaration(
            ir->GetInsertBlock()->getModule(), llvm::Intrinsic::assume);
        ir->CreateCall(intrinsic, {value});
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
        auto *call = builder->value->CreateCall(callee->getFunctionType(), callee,
                                                values);
        if (!callee->getReturnType()->isVoidTy() && name.len != 0) {
            call->setName(borrowed_string(name));
        }
        *out = bridge_value(call);
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
        llvm::BranchInst *branch = builder->value->CreateCondBr(
            llvm_value(condition), llvm_block(then_block), llvm_block(else_block));
        if (llvm_block(then_block)->getName().starts_with("checked.failure") &&
            llvm_block(else_block)->getName().starts_with("checked.continue")) {
            llvm::MDBuilder metadata(builder->value->getContext());
            branch->setMetadata(llvm::LLVMContext::MD_prof,
                                metadata.createBranchWeights(1, 2000));
        }
        return CKC_LLVM_OK;
    });
}

static std::pair<uint32_t, uint32_t> ckc_branch_weights(uint64_t then_count,
                                                        uint64_t else_count) {
    uint64_t maximum = std::max(then_count, else_count);
    while (maximum > std::numeric_limits<uint32_t>::max()) {
        then_count >>= 1;
        else_count >>= 1;
        maximum >>= 1;
    }
    // A zero observation is not an unreachable proof. Keep both successors
    // possible while preserving the checked profile ratio as closely as the
    // LLVM metadata schema permits.
    return {static_cast<uint32_t>(std::max<uint64_t>(then_count, 1)),
            static_cast<uint32_t>(std::max<uint64_t>(else_count, 1))};
}

extern "C" int32_t ckc_llvm_builder_cond_branch_weighted(
    CkcLlvmBuilder *builder, CkcLlvmValue *condition,
    CkcLlvmBlock *then_block, CkcLlvmBlock *else_block,
    uint64_t then_count, uint64_t else_count, CkcLlvmError *error) {
    return guarded(error, "building checked LLVM weighted branch", [&] {
        if (builder == nullptr || builder->value == nullptr ||
            condition == nullptr || then_block == nullptr || else_block == nullptr) {
            return invalid(error, "LLVM weighted branch input is null");
        }
        auto *branch = builder->value->CreateCondBr(
            llvm_value(condition), llvm_block(then_block), llvm_block(else_block));
        const auto [then_weight, else_weight] =
            ckc_branch_weights(then_count, else_count);
        llvm::MDBuilder metadata(branch->getContext());
        branch->setMetadata(
            llvm::LLVMContext::MD_prof,
            metadata.createBranchWeights(then_weight, else_weight));
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

extern "C" int32_t ckc_llvm_target_parse_object(
    CkcLlvmTarget *target, CkcLlvmBytes object_bytes, CkcLlvmObject **out,
    CkcLlvmError *error) {
    clear_error(error);
    if (target == nullptr || target->value == nullptr || out == nullptr) {
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "LLVM cached object input or output is null");
    }
    *out = nullptr;
    try {
        auto validated = validated_object_buffer(
            object_bytes, "ckc-cache-object.o",
            target->value->getTargetTriple().getArch());
        if (!validated) {
            return set_llvm_error(error, validated.takeError());
        }
        auto object = std::make_unique<CkcLlvmObject>();
        const llvm::StringRef bytes = (*validated)->getBuffer();
        object->bytes.assign(bytes.bytes_begin(), bytes.bytes_end());
        *out = object.release();
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception parsing cached LLVM object");
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

extern "C" int32_t ckc_llvm_archive_create(
    const CkcLlvmObject *const *objects, const CkcLlvmBytes *member_names,
    size_t object_count, uint32_t kind,
                                             CkcLlvmArchive **out,
                                             CkcLlvmError *error) {
    clear_error(error);
    if (objects == nullptr || member_names == nullptr || object_count == 0 ||
        out == nullptr) {
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

        std::vector<std::unique_ptr<llvm::MemoryBuffer>> buffers;
        std::vector<llvm::NewArchiveMember> members;
        std::set<std::string> unique_names;
        buffers.reserve(object_count);
        members.reserve(object_count);
        for (size_t index = 0; index < object_count; ++index) {
            if (objects[index] == nullptr || objects[index]->bytes.empty()) {
                return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                                 "LLVM archive contains an empty object");
            }
            const llvm::StringRef object_bytes(
                reinterpret_cast<const char *>(objects[index]->bytes.data()),
                objects[index]->bytes.size());
            const std::string member_name = borrowed_string(member_names[index]).str();
            if (member_name.empty() || member_name.size() > 255 ||
                member_name.front() == '.' ||
                !std::all_of(member_name.begin(), member_name.end(), [](char value) {
                    const auto byte = static_cast<unsigned char>(value);
                    return std::isalnum(byte) != 0 || value == '.' || value == '_' ||
                           value == '-';
                }) ||
                !unique_names.insert(member_name).second) {
                return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                                 "LLVM archive member name is invalid or duplicated");
            }
            buffers.push_back(
                llvm::MemoryBuffer::getMemBufferCopy(object_bytes, member_name));
            llvm::NewArchiveMember member(buffers.back()->getMemBufferRef());
            member.MemberName = buffers.back()->getBufferIdentifier();
            members.push_back(std::move(member));
        }
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
        if (member_count != object_count || !(*parsed)->hasSymbolTable()) {
            return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                             "LLVM produced an invalid indexed archive");
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
    const CkcLlvmBytes *object_path_bytes, size_t object_count,
    CkcLlvmBytes output_path_bytes, CkcLlvmBytes import_library_path_bytes,
    CkcLlvmBytes platform_input_path_bytes, const CkcLlvmBytes *exports,
    size_t export_count, CkcLlvmError *error) {
    clear_error(error);
    try {
        if (object_count == 0 || object_path_bytes == nullptr) {
            return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                             "LLD shared object list is empty");
        }
        auto output_path = checked_path(output_path_bytes, "LLD output path");
        if (!output_path) {
            return set_llvm_error(error, output_path.takeError());
        }
        if (export_count != 0 && exports == nullptr) {
            return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                             "LLD export list is null");
        }
        std::vector<std::string> object_paths;
        object_paths.reserve(object_count);
        for (size_t index = 0; index < object_count; ++index) {
            auto object_path = checked_path(object_path_bytes[index], "LLD object path");
            if (!object_path) {
                return set_llvm_error(error, object_path.takeError());
            }
            if (auto validation = validate_link_input(*object_path)) {
                return set_llvm_error(error, std::move(validation));
            }
            object_paths.push_back(std::move(*object_path));
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
        arguments.emplace_back("-install_name");
        arguments.emplace_back("@rpath/module.dylib");
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
#if defined(CKC_LLD_COFF)
        arguments.emplace_back("/out:" + *output_path);
#else
        arguments.emplace_back("-o");
        arguments.emplace_back(*output_path);
#endif
        arguments.insert(arguments.end(), object_paths.begin(), object_paths.end());
        if (object_count > 1) {
#if defined(CKC_LLD_DARWIN) || defined(CKC_LLD_COFF)
            auto platform_input = checked_path(platform_input_path_bytes,
                                               "LLD shared platform input path");
            if (!platform_input) {
                return set_llvm_error(error, platform_input.takeError());
            }
            arguments.emplace_back(*platform_input);
#endif
        }

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
        lld::Result result;
        {
            std::lock_guard<std::mutex> lock(lld_driver_mutex());
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

extern "C" int32_t ckc_lld_link_executable(
    const CkcLlvmBytes *object_path_bytes, size_t object_count,
    CkcLlvmBytes output_path_bytes, CkcLlvmBytes platform_input_path_bytes,
    CkcLlvmError *error) {
    clear_error(error);
    try {
        if (object_count == 0 || object_path_bytes == nullptr) {
            return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                             "LLD executable object list is empty");
        }
        auto output_path = checked_path(output_path_bytes, "LLD executable output path");
        if (!output_path) {
            return set_llvm_error(error, output_path.takeError());
        }
        std::vector<std::string> object_paths;
        object_paths.reserve(object_count);
        for (size_t index = 0; index < object_count; ++index) {
            auto path = checked_path(object_path_bytes[index], "LLD executable object path");
            if (!path) {
                return set_llvm_error(error, path.takeError());
            }
            if (auto validation = validate_link_input(*path)) {
                return set_llvm_error(error, std::move(validation));
            }
            object_paths.push_back(std::move(*path));
        }

        std::vector<std::string> arguments;
        arguments.reserve(18 + object_count);
#if defined(CKC_LLD_DARWIN)
        auto platform_input = checked_path(platform_input_path_bytes,
                                           "LLD libSystem stub path");
        if (!platform_input) {
            return set_llvm_error(error, platform_input.takeError());
        }
        arguments.emplace_back("ld64.lld");
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
        arguments.emplace_back("-dead_strip");
        arguments.emplace_back("-e");
        arguments.emplace_back("_main");
#elif defined(CKC_LLD_COFF)
        auto platform_input = checked_path(platform_input_path_bytes,
                                           "LLD kernel32 import path");
        if (!platform_input) {
            return set_llvm_error(error, platform_input.takeError());
        }
        arguments.emplace_back("lld-link");
        arguments.emplace_back("/subsystem:console");
        arguments.emplace_back("/entry:mainCRTStartup");
        arguments.emplace_back("/nodefaultlib");
#else
        arguments.emplace_back("ld.lld");
        arguments.emplace_back("-static");
        arguments.emplace_back("--gc-sections");
        arguments.emplace_back("-z");
        arguments.emplace_back("noexecstack");
        arguments.emplace_back("-e");
        arguments.emplace_back("_start");
#endif
#if defined(CKC_LLD_COFF)
        arguments.emplace_back("/out:" + *output_path);
#else
        arguments.emplace_back("-o");
        arguments.emplace_back(*output_path);
#endif
        arguments.insert(arguments.end(), object_paths.begin(), object_paths.end());
#if defined(CKC_LLD_DARWIN) || defined(CKC_LLD_COFF)
        arguments.emplace_back(*platform_input);
#endif

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
        lld::Result result{1, false};
        {
            std::lock_guard<std::mutex> lock(lld_driver_mutex());
            result = lld::lldMain(raw_arguments, stdout_stream, stderr_stream,
                                  drivers);
        }
        stdout_stream.flush();
        stderr_stream.flush();
        if (result.retCode != 0) {
            std::string message = stderr_text.empty() ? stdout_text : stderr_text;
            if (message.empty()) {
                message = "LLD executable link returned a non-zero status";
            }
            return set_error(error, CKC_LLVM_INTERNAL_ERROR, message);
        }
        if (!result.canRunAgain) {
            return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                             "LLD completed but cannot safely run again");
        }
        if (auto validation = validate_executable_output(*output_path)) {
            return set_llvm_error(error, std::move(validation));
        }
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception linking executable");
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
        auto memory_audit = std::make_shared<CkcJitMemoryAuditState>();

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
                [memory_audit](llvm::orc::ExecutionSession &session)
                    -> llvm::Expected<
                        std::unique_ptr<llvm::orc::ObjectLayer>> {
                    auto memory_manager =
                        [memory_audit](const llvm::MemoryBuffer &) {
                        return std::make_unique<
                            CkcAuditedSectionMemoryManager>(memory_audit);
                    };
                    auto object_layer = std::make_unique<
                        llvm::orc::RTDyldObjectLinkingLayer>(
                        session, std::move(memory_manager));
                    object_layer->setOverrideObjectFlagsWithResponsibilityFlags(
                        true);
                    object_layer->setAutoClaimResponsibilityForObjectSymbols(
                        true);
                    return std::unique_ptr<llvm::orc::ObjectLayer>(
                        std::move(object_layer));
                });
        } else {
            builder.setObjectLinkingLayerCreator(
                [memory_audit](llvm::orc::ExecutionSession &session)
                    -> llvm::Expected<
                        std::unique_ptr<llvm::orc::ObjectLayer>> {
                    auto mapper =
                        CkcInProcessMemoryMapper::Create(memory_audit);
                    if (!mapper) {
                        return mapper.takeError();
                    }
                    auto memory_manager = std::make_unique<
                        llvm::orc::MapperJITLinkMemoryManager>(
                        CKC_JIT_RESERVATION_GRANULARITY,
                        std::move(*mapper));
                    auto object_layer =
                        std::make_unique<llvm::orc::ObjectLinkingLayer>(
                            session, std::move(memory_manager));
#if defined(CKC_LLD_COFF) && \
    (defined(_M_X64) || defined(__x86_64__))
                    object_layer->addPlugin(std::make_shared<
                        CkcCoffX64ProcessStubsPlugin>());
#endif
                    return std::unique_ptr<llvm::orc::ObjectLayer>(
                        std::move(object_layer));
                });
        }

        auto jit_value = builder.create();
        if (!jit_value) {
            return set_llvm_error(error, jit_value.takeError());
        }
        auto jit = std::make_unique<CkcLlvmJit>();
        jit->value = std::move(*jit_value);
        jit->memory_audit = std::move(memory_audit);
        if (auto symbol_error = define_allowed_process_symbols(*jit->value)) {
            return set_llvm_error(error, std::move(symbol_error));
        }
        jit->object_layer = use_coff_aarch64_rtdyld
                                ? CKC_LLVM_ORC_RTDYLD_COFF_AARCH64
                                : CKC_LLVM_ORC_JITLINK;
        jit->executed = false;
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

extern "C" int32_t ckc_llvm_jit_execute(
    CkcLlvmJit *jit, CkcLlvmBytes program_object,
    const CkcLlvmBytes *runtime_objects, size_t runtime_object_count,
    int32_t *exit_status, CkcLlvmError *error) {
    clear_error(error);
#if defined(CKC_LLD_COFF) && \
    (defined(_M_X64) || defined(__x86_64__))
    if (jit == nullptr || jit->value == nullptr || exit_status == nullptr ||
        runtime_objects == nullptr || runtime_object_count != 7) {
#else
    if (jit == nullptr || jit->value == nullptr || exit_status == nullptr ||
        runtime_objects == nullptr || runtime_object_count != 6) {
#endif
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "LLVM JIT execution input is invalid");
    }
    *exit_status = 0;
    if (jit->executed) {
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "native JIT instance already executed an object");
    }
    jit->executed = true;
    try {
        const auto arch = jit->value->getTargetTriple().getArch();
        std::vector<std::unique_ptr<llvm::MemoryBuffer>> buffers;
        std::set<std::string> linker_symbols;
        buffers.reserve(runtime_object_count + 1);
        for (size_t index = 0; index < runtime_object_count; ++index) {
            auto buffer = validated_object_buffer(
                runtime_objects[index],
                index == 0 && runtime_object_count == 7
                    ? "ckc-jit-image-base.o"
                    : "ckc-runtime-" +
                          std::to_string(index - (runtime_object_count == 7)) +
                          ".o",
                arch);
            if (!buffer) {
                return set_llvm_error(error, buffer.takeError());
            }
            auto symbols = defined_linker_symbols(**buffer);
            if (!symbols) {
                return set_llvm_error(error, symbols.takeError());
            }
#if defined(CKC_LLD_COFF) && \
    (defined(_M_X64) || defined(__x86_64__))
            const bool defines_image_base =
                std::find(symbols->begin(), symbols->end(), "__ImageBase") !=
                symbols->end();
            if (index == 0 && !defines_image_base) {
                return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                                 "invalid COFF x64 JIT image-base anchor");
            }
            if (index != 0 && defines_image_base) {
                return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                                 "runtime object defines reserved __ImageBase");
            }
#endif
            linker_symbols.insert(symbols->begin(), symbols->end());
            buffers.push_back(std::move(*buffer));
        }
        auto program =
            validated_object_buffer(program_object, "ckc-program.o", arch);
        if (!program) {
            return set_llvm_error(error, program.takeError());
        }
        auto program_symbols = defined_linker_symbols(**program);
        if (!program_symbols) {
            return set_llvm_error(error, program_symbols.takeError());
        }
#if defined(CKC_LLD_COFF) && \
    (defined(_M_X64) || defined(__x86_64__))
        if (std::find(program_symbols->begin(), program_symbols->end(),
                      "__ImageBase") != program_symbols->end()) {
            return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                             "program object defines reserved __ImageBase");
        }
#endif
        linker_symbols.insert(program_symbols->begin(),
                              program_symbols->end());
        buffers.push_back(std::move(*program));

#if defined(CKC_LLD_COFF) && \
    (defined(_M_X64) || defined(__x86_64__))
        if (auto add_error =
                jit->value->addObjectFile(std::move(buffers.front()))) {
            return set_llvm_error(error, std::move(add_error));
        }
        auto image_base_address =
            jit->value->lookupLinkerMangled("__ImageBase");
        if (!image_base_address) {
            return set_llvm_error(error, image_base_address.takeError());
        }
        for (size_t index = 1; index < buffers.size(); ++index) {
            if (auto add_error =
                    jit->value->addObjectFile(std::move(buffers[index]))) {
                return set_llvm_error(error, std::move(add_error));
            }
        }
#else
        for (auto &buffer : buffers) {
            if (auto add_error = jit->value->addObjectFile(std::move(buffer))) {
                return set_llvm_error(error, std::move(add_error));
            }
        }
#endif
        llvm::orc::ExecutorAddr entry_address;
        const std::string main_linker_name =
            jit->value->mangle("main");
        for (const auto &symbol_name : linker_symbols) {
            auto address =
                jit->value->lookupLinkerMangled(symbol_name);
            if (!address) {
                return set_llvm_error(error, address.takeError());
            }
            if (symbol_name == main_linker_name) {
                entry_address = *address;
            }
        }
        if (!entry_address) {
            return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                             "LLVM JIT object has no main entry symbol");
        }
        using EntryFunction = int32_t();
        auto *entry_function = entry_address.toPtr<EntryFunction>();
        if (entry_function == nullptr) {
            return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                             "LLVM JIT entry lookup returned null");
        }
        *exit_status = entry_function();
        return CKC_LLVM_OK;
    } catch (const std::exception &exception) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR, exception.what());
    } catch (...) {
        return set_error(error, CKC_LLVM_INTERNAL_ERROR,
                         "unknown C++ exception executing LLVM JIT object");
    }
}

extern "C" int32_t ckc_llvm_jit_memory_audit(
    const CkcLlvmJit *jit, CkcLlvmJitMemoryAudit *out,
    CkcLlvmError *error) {
    clear_error(error);
    if (jit == nullptr || jit->memory_audit == nullptr || out == nullptr) {
        return set_error(error, CKC_LLVM_INVALID_ARGUMENT,
                         "LLVM JIT memory audit input is invalid");
    }
    std::lock_guard<std::mutex> lock(jit->memory_audit->mutex);
    out->allocations = jit->memory_audit->allocations;
    out->instruction_cache_finalizations =
        jit->memory_audit->instruction_cache_finalizations;
    out->relocation_write_non_execute =
        jit->memory_audit->saw_relocation_allocation &&
        jit->memory_audit->relocation_write_non_execute;
    out->final_code_read_execute =
        jit->memory_audit->saw_final_code &&
        jit->memory_audit->final_code_read_execute;
    out->final_data_non_execute =
        jit->memory_audit->saw_final_data &&
        jit->memory_audit->final_data_non_execute;
    out->darwin_map_jit = jit->memory_audit->darwin_map_jit;
    out->darwin_thread_write_protection_supported =
        jit->memory_audit->darwin_thread_write_protection_supported;
    out->darwin_thread_write_protection =
        jit->memory_audit->darwin_thread_write_protection;
    return CKC_LLVM_OK;
}

extern "C" void ckc_llvm_jit_dispose(CkcLlvmJit *jit) { delete jit; }
