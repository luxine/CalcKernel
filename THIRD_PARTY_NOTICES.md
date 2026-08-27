# CalcKernel Third-Party Notices

This index is embedded byte-for-byte in every `ckc` executable. Source archive
and crate hashes are immutable acquisition identities; repository license paths
name the exact notice bytes emitted by `ckc licenses`. CalcKernel chooses the
Apache-2.0 option where a Cargo dependency offers `MIT OR Apache-2.0`, except
for the two explicitly named MIT copies below. Build-only crates are included
because they participate in producing the distributed executable.

## Compiler and native runtime

- Rust standard library 1.90.0 — rust-src SHA-256 `cde088d57064d151b2236f4619aea4a8207e0709eb3035ddc6617d609ab7d453` — `third_party/licenses/RUST-COPYRIGHT`, `third_party/licenses/RUST-LICENSE-MIT`
- LLVM 22.1.8 — source SHA-256 `922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888` — `native/llvm/LICENSE.TXT`
- LLD 22.1.8 — source SHA-256 `922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888` — `native/llvm/LLD-LICENSE.TXT`
- LLVM Support BLAKE3 — LLVM source SHA-256 `922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888` — `native/llvm/third-party/BLAKE3-LICENSE`
- LLVM Support regex — LLVM source SHA-256 `922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888` — `native/llvm/third-party/COPYRIGHT.regex`
- Ryu 59661c3f883dfd39cef6dc8eaf2fcbaae53597e8 — vendored source hashes in `native/runtime/provenance.toml` — `native/runtime/vendor/ryu/LICENSE-Apache2`, `native/runtime/vendor/ryu/LICENSE-Boost`

## Cargo source and build dependency closure

- block-buffer 0.10.4 — crates.io SHA-256 `3078c7629b62d3f0439517fa394996acacc5cbc91c5a20d8c658e77abd503a71` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- bumpalo 3.20.3 — crates.io SHA-256 `72f5acc6cb2ba439de613abc23857ec3d78374d8ed5ac84e9d11336e87da8649` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- cc 1.4.4 — crates.io SHA-256 `0ad534f4357a5264cce5019c989cf66a4f0dc4e0d1b1d15f8aacec0ff7360273` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- cfg-if 1.0.4 — crates.io SHA-256 `9330f8b2ff13f34540b44e946ef35111825727b38d33286ef986142615121801` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- cpufeatures 0.2.17 — crates.io SHA-256 `59ed5838eebb26a2bb2e58f6d5b5316989ae9d08bab10e0e6d103e656d1b0280` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- crypto-common 0.1.7 — crates.io SHA-256 `78c8292055d1c1df0cce5d180393dc8cce0abec0a7102adb6c7b1eef6016d60a` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- digest 0.10.7 — crates.io SHA-256 `9ed9a281f7bc9b7576e61468ba615a66a5c8cfdff42420a70aa82701a3b1e292` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- find-msvc-tools 0.1.11 — crates.io SHA-256 `d45db016d36b838f563236e9193d0ee6ce38f3f68b6c94e914b4929c96bbb890` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- generic-array 0.14.7 — crates.io SHA-256 `85649ca51fd72272d7821adaf274ad91c288277713d9c18820d8499a7ff69e9a` — `third_party/licenses/generic-array-MIT.txt`
- leb128fmt 0.1.0 — crates.io SHA-256 `09edd9e8b54e49e587e4f6295a7d29c3ea94d469cb40ab8ca70b288248a81db2` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- libc 0.2.186 — crates.io SHA-256 `68ab91017fe16c622486840e4c83c9a37afeff978bd239b5293d61ece587de66` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- memchr 2.8.2 — crates.io SHA-256 `88904434abc2901f197fe8cc55f0445e7ded921dba5911dad2e2b39b48e663c4` — `third_party/licenses/memchr-MIT.txt`
- proc-macro2 1.0.106 — crates.io SHA-256 `8fd00f0bb2e90d81d1044c2b32617f68fcb9fa3bb7640c23e9c748e53fb30934` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- quote 1.0.46 — crates.io SHA-256 `dfbc457d0c7a0759a614551b11a6409e5951f6c7537be1f1b7682b9ae9230368` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- sha2 0.10.9 — crates.io SHA-256 `a7507d819769d01a365ab707794a4084392c824f54a7a6a7862f8c3d0892b283` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- shlex 2.0.1 — crates.io SHA-256 `f8fadd59c855ef2080decdef8ff161eb6661b86933c9d82e5ba29dc602a55aba` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- syn 2.0.118 — crates.io SHA-256 `1b9ae57f904213ebb649ce6895b8a66c66f0203b9319718f69a5612a065b1422` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- thiserror 2.0.18 — crates.io SHA-256 `4288b5bcbc7920c07a1149a35cf9590a2aa808e0bc1eafaade0b80947865fbc4` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- thiserror-impl 2.0.18 — crates.io SHA-256 `ebc4ee7f67670e9b64d05fa4253e753e016c6c95ff35b89b7941d6b856dec1d5` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- typenum 1.20.1 — crates.io SHA-256 `b6f5e870be6c3b371b77fe0ee0bafb859fa4964b4404c27de1d380043c4dda20` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- unicode-ident 1.0.24 — crates.io SHA-256 `e6e4313cd5fcd3dad5cafa179702e2b244f760991f45397d14d4ebf38247da75` — `native/runtime/vendor/ryu/LICENSE-Apache2`, `third_party/licenses/LICENSE-UNICODE`
- unicode-width 0.2.2 — crates.io SHA-256 `b4ac048d71ede7ee76d585517add45da530660ef4390e49b098733c6e897f254` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- version_check 0.9.5 — crates.io SHA-256 `0b928f33d975fc6ad9f86c8f283853ad26bdd5b10b7f1542aa2fa15e2289105a` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- wasm-encoder 0.252.0 — crates.io SHA-256 `8185ae345fa5687c054626ff9a50e7089797a343d9904d1dc9820eb4c4d3196f` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- wast 252.0.0 — crates.io SHA-256 `942a3449d6a593fccc111a6241c8df52bda168af30e40bf9580d4394d7374c65` — `native/runtime/vendor/ryu/LICENSE-Apache2`
- wat 1.252.0 — crates.io SHA-256 `c72a4ba7088f7bac94cf516e49882bdf97068904a563768cf249efc839ec42cb` — `native/runtime/vendor/ryu/LICENSE-Apache2`

The complete license texts follow the index in `ckc licenses`; duplicate
Apache-2.0 text is emitted once. Development-only test dependencies are not
linked into or used to build the distributed executable and are excluded.
