pub(crate) fn embedded_runtime_objects() -> [&'static [u8]; 5] {
    [
        include_bytes!(env!("CKC_RUNTIME_OBJECT_0")),
        include_bytes!(env!("CKC_RUNTIME_OBJECT_1")),
        include_bytes!(env!("CKC_RUNTIME_OBJECT_2")),
        include_bytes!(env!("CKC_RUNTIME_OBJECT_3")),
        include_bytes!(env!("CKC_RUNTIME_OBJECT_4")),
    ]
}

pub(crate) fn embedded_jit_objects() -> Vec<&'static [u8]> {
    let mut objects = Vec::with_capacity(6);
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    objects.push(include_bytes!(env!("CKC_RUNTIME_JIT_SUPPORT")));
    objects.extend(embedded_runtime_objects());
    objects
}

#[cfg(target_os = "windows")]
pub(crate) fn embedded_windows_import_library() -> &'static [u8] {
    include_bytes!(env!("CKC_RUNTIME_PLATFORM_IMPORT"))
}

#[cfg(not(target_os = "windows"))]
pub(crate) const fn embedded_windows_import_library() -> &'static [u8] {
    &[]
}
