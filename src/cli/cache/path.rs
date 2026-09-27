use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CachePlatform {
    Linux,
    Darwin,
    Windows,
}

pub(super) fn resolve_cache_root() -> Option<PathBuf> {
    let platform = if cfg!(target_os = "windows") {
        CachePlatform::Windows
    } else if cfg!(target_os = "macos") {
        CachePlatform::Darwin
    } else {
        CachePlatform::Linux
    };
    resolve_cache_root_with(platform, |name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string_lossy().into_owned())
    })
}

fn resolve_cache_root_with(
    platform: CachePlatform,
    mut environment: impl FnMut(&str) -> Option<String>,
) -> Option<PathBuf> {
    let mut value = |name| environment(name).filter(|value| !value.is_empty());
    match platform {
        CachePlatform::Linux => value("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .map(|base| base.join("ckc"))
            .or_else(|| {
                value("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".cache/ckc"))
            }),
        CachePlatform::Darwin => value("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Caches/ckc")),
        CachePlatform::Windows => value("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|base| base.join("CalcKernel/cache")),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use super::{CachePlatform, resolve_cache_root_with};

    fn resolve(platform: CachePlatform, values: &[(&str, &str)]) -> Option<PathBuf> {
        let values = values.iter().copied().collect::<HashMap<_, _>>();
        resolve_cache_root_with(platform, |name| values.get(name).map(ToString::to_string))
    }

    #[test]
    fn cache_roots_should_follow_all_three_platform_contracts() {
        assert_eq!(
            resolve(CachePlatform::Linux, &[("XDG_CACHE_HOME", "/cache")]),
            Some(PathBuf::from("/cache/ckc"))
        );
        assert_eq!(
            resolve(CachePlatform::Linux, &[("HOME", "/home/me")]),
            Some(PathBuf::from("/home/me/.cache/ckc"))
        );
        assert_eq!(
            resolve(CachePlatform::Darwin, &[("HOME", "/Users/me")]),
            Some(PathBuf::from("/Users/me/Library/Caches/ckc"))
        );
        assert_eq!(
            resolve(
                CachePlatform::Windows,
                &[("LOCALAPPDATA", r"C:\Users\me\AppData\Local")]
            ),
            Some(PathBuf::from(r"C:\Users\me\AppData\Local").join("CalcKernel/cache"))
        );
    }

    #[test]
    fn missing_or_empty_required_base_should_disable_cache() {
        for platform in [
            CachePlatform::Linux,
            CachePlatform::Darwin,
            CachePlatform::Windows,
        ] {
            assert_eq!(resolve(platform, &[]), None);
        }
        assert_eq!(
            resolve(
                CachePlatform::Linux,
                &[("XDG_CACHE_HOME", ""), ("HOME", "/home/me")]
            ),
            Some(PathBuf::from("/home/me/.cache/ckc"))
        );
    }
}
