#!/usr/bin/env bash
set -euo pipefail

readonly CKC_LLVM_VERSION="22.1.8"
readonly CKC_LLVM_SHA256="922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888"

usage() {
  echo "usage: $0 --archive <llvm-project.tar.xz> --prefix <install-dir> --target <rust-target> [--build-dir <dir>] [--profile release|oracle] [--jobs <n>]" >&2
  exit 2
}

ckc_archive=""
ckc_prefix=""
ckc_target=""
ckc_build_dir=""
ckc_profile="release"
ckc_jobs=""

while (($#)); do
  case "$1" in
    --archive) (($# >= 2)) || usage; ckc_archive="$2"; shift 2 ;;
    --prefix) (($# >= 2)) || usage; ckc_prefix="$2"; shift 2 ;;
    --target) (($# >= 2)) || usage; ckc_target="$2"; shift 2 ;;
    --build-dir) (($# >= 2)) || usage; ckc_build_dir="$2"; shift 2 ;;
    --profile) (($# >= 2)) || usage; ckc_profile="$2"; shift 2 ;;
    --jobs) (($# >= 2)) || usage; ckc_jobs="$2"; shift 2 ;;
    *) usage ;;
  esac
done

[[ -f "$ckc_archive" && -n "$ckc_prefix" && -n "$ckc_target" ]] || usage
[[ "$ckc_profile" == "release" || "$ckc_profile" == "oracle" ]] || usage
[[ ! -e "$ckc_prefix" ]] || {
  echo "refusing to overwrite LLVM prefix: $ckc_prefix" >&2
  exit 1
}

if [[ -z "$ckc_build_dir" ]]; then
  ckc_build_dir="build/llvm/${ckc_target}-${ckc_profile}"
fi
[[ ! -e "$ckc_build_dir" ]] || {
  echo "refusing to overwrite LLVM build directory: $ckc_build_dir" >&2
  exit 1
}

if command -v sha256sum >/dev/null 2>&1; then
  ckc_actual_sha="$(sha256sum "$ckc_archive" | awk '{print $1}')"
else
  ckc_actual_sha="$(shasum -a 256 "$ckc_archive" | awk '{print $1}')"
fi
[[ "$ckc_actual_sha" == "$CKC_LLVM_SHA256" ]] || {
  echo "LLVM source checksum mismatch: expected $CKC_LLVM_SHA256, got $ckc_actual_sha" >&2
  exit 1
}

case "$ckc_target" in
  aarch64-apple-darwin|aarch64-unknown-linux-gnu|aarch64-pc-windows-msvc)
    ckc_llvm_target="AArch64"
    ;;
  x86_64-apple-darwin|x86_64-unknown-linux-gnu|x86_64-pc-windows-msvc)
    ckc_llvm_target="X86"
    ;;
  *)
    echo "unsupported CalcKernel release target: $ckc_target" >&2
    exit 1
    ;;
esac

case "$ckc_target" in
  *-apple-darwin) ckc_lld_driver="MachO" ;;
  *-unknown-linux-gnu) ckc_lld_driver="ELF" ;;
  *-pc-windows-msvc) ckc_lld_driver="COFF" ;;
esac

ckc_projects="lld"
if [[ "$ckc_profile" == "oracle" ]]; then
  ckc_projects="clang;lld"
fi

ckc_platform_args=()
if [[ "$ckc_target" == *-apple-darwin ]]; then
  ckc_platform_args=(-DCMAKE_OSX_DEPLOYMENT_TARGET=11.0)
fi

mkdir -p "$ckc_build_dir/source" "$ckc_build_dir/build"
tar -xf "$ckc_archive" --strip-components=1 -C "$ckc_build_dir/source"

ckc_parallel_args=()
if [[ -n "$ckc_jobs" ]]; then
  ckc_parallel_args=(--parallel "$ckc_jobs")
fi

cmake -S "$ckc_build_dir/source/llvm" -B "$ckc_build_dir/build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$ckc_prefix" \
  -DLLVM_ENABLE_PROJECTS="$ckc_projects" \
  -DLLVM_TARGETS_TO_BUILD="$ckc_llvm_target" \
  -DLLVM_ENABLE_ASSERTIONS=ON \
  -DBUILD_SHARED_LIBS=OFF \
  -DLLVM_BUILD_LLVM_DYLIB=OFF \
  -DLLVM_LINK_LLVM_DYLIB=OFF \
  -DLLVM_ENABLE_RTTI=OFF \
  -DLLVM_ENABLE_EH=OFF \
  -DLLVM_ENABLE_ZLIB=OFF \
  -DLLVM_ENABLE_ZSTD=OFF \
  -DLLVM_ENABLE_LIBXML2=OFF \
  -DLLVM_ENABLE_TERMINFO=OFF \
  -DLLVM_ENABLE_LIBEDIT=OFF \
  -DLLVM_INCLUDE_TESTS=OFF \
  -DLLVM_INCLUDE_BENCHMARKS=OFF \
  -DLLVM_INCLUDE_EXAMPLES=OFF \
  "${ckc_platform_args[@]}"

cmake --build "$ckc_build_dir/build" "${ckc_parallel_args[@]}"
cmake --install "$ckc_build_dir/build"

ckc_llvm_config="$ckc_prefix/bin/llvm-config"
[[ -x "$ckc_llvm_config" ]] || {
  echo "bootstrap did not install llvm-config" >&2
  exit 1
}
[[ "$($ckc_llvm_config --version)" == "$CKC_LLVM_VERSION" ]] || {
  echo "installed llvm-config version mismatch" >&2
  exit 1
}
if find "$ckc_prefix/lib" -maxdepth 1 \( -name 'libLLVM*.so*' -o -name 'libLLVM*.dylib' -o -name 'LLVM*.dll' \) -print -quit | grep -q .; then
  echo "release prefix contains a shared LLVM library" >&2
  exit 1
fi
if [[ "$ckc_profile" == "release" && -e "$ckc_prefix/bin/clang" ]]; then
  echo "release prefix unexpectedly contains Clang" >&2
  exit 1
fi
if [[ "$ckc_profile" == "oracle" && ! -x "$ckc_prefix/bin/clang" ]]; then
  echo "oracle prefix is missing Clang" >&2
  exit 1
fi

ckc_components=(core native orcjit nativecodegen)
ckc_llvm_libs=()
while IFS= read -r ckc_library; do
  ckc_llvm_libs+=("$ckc_library")
done < <("$ckc_llvm_config" --link-static --libnames "${ckc_components[@]}" | tr ' ' '\n' | sed -e 's/^lib//' -e 's/\.a$//' -e '/^$/d')
ckc_lld_libs=("lld${ckc_lld_driver}" lldCommon)
ckc_static_libs=("${ckc_lld_libs[@]}" "${ckc_llvm_libs[@]}")
ckc_system_libs=()
for ckc_flag in $("$ckc_llvm_config" --link-static --system-libs "${ckc_components[@]}"); do
  case "$ckc_flag" in
    -l*) ckc_system_libs+=("${ckc_flag#-l}") ;;
    "") ;;
    *)
      echo "unsupported llvm-config system library flag: $ckc_flag" >&2
      exit 1
      ;;
  esac
done

toml_array() {
  local ckc_first=true
  printf '['
  for ckc_item in "$@"; do
    if [[ "$ckc_first" == true ]]; then
      ckc_first=false
    else
      printf ', '
    fi
    printf '"%s"' "$ckc_item"
  done
  printf ']'
}

mkdir -p "$ckc_prefix/share/ckc"
{
  printf 'schema = 1\n'
  printf 'version = "%s"\n' "$CKC_LLVM_VERSION"
  printf 'target = "%s"\n' "$ckc_target"
  printf 'profile = "%s"\n' "$ckc_profile"
  printf 'source_sha256 = "%s"\n' "$CKC_LLVM_SHA256"
  printf 'static_only = true\n'
  printf 'components = '
  toml_array "${ckc_components[@]}"
  printf '\n'
  printf 'static_libraries = '
  toml_array "${ckc_static_libs[@]}"
  printf '\nsystem_libraries = '
  toml_array "${ckc_system_libs[@]}"
  printf '\n'
} > "$ckc_prefix/share/ckc/llvm-build.toml"

if [[ "$ckc_profile" == "oracle" ]]; then
  printf '%s\n' "$ckc_prefix/bin/clang"
else
  printf '%s\n' "$ckc_prefix"
fi
