pub(crate) fn embedded_runtime_objects() -> [&'static [u8]; 5] {
    [
        include_bytes!(env!("CKC_RUNTIME_OBJECT_0")),
        include_bytes!(env!("CKC_RUNTIME_OBJECT_1")),
        include_bytes!(env!("CKC_RUNTIME_OBJECT_2")),
        include_bytes!(env!("CKC_RUNTIME_OBJECT_3")),
        include_bytes!(env!("CKC_RUNTIME_OBJECT_4")),
    ]
}

#[cfg(target_os = "windows")]
pub(crate) fn embedded_windows_import_library() -> &'static [u8] {
    include_bytes!(env!("CKC_RUNTIME_PLATFORM_IMPORT"))
}

#[cfg(not(target_os = "windows"))]
pub(crate) const fn embedded_windows_import_library() -> &'static [u8] {
    &[]
}
