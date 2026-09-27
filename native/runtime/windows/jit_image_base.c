/*
 * COFF x86-64 JITLink resolves IMAGE_REL_AMD64_ADDR32NB relocations against
 * __ImageBase. This data-only object supplies that private anchor inside the
 * JIT reservation; it is never linked into CK AOT artifacts.
 */
#if !defined(_M_X64)
#error "the CK JIT image-base anchor is x86-64-only"
#endif

unsigned char __ImageBase = 0;
