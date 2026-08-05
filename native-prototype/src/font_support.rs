use std::path::PathBuf;

/// Candidate CJK font files supplied by the host operating system.
///
/// The native client intentionally does not ship a large CJK font collection.
/// These paths cover the standard font locations on the three supported
/// desktop platforms and can be overridden for testing or custom installs.
pub fn cjk_font_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(path) = std::env::var_os("LITETERM_CJK_FONT").map(PathBuf::from) {
        paths.push(if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map(|directory| directory.join(&path))
                .unwrap_or(path)
        });
    }

    #[cfg(target_os = "windows")]
    {
        let windows_root = std::env::var_os("WINDIR")
            .or_else(|| std::env::var_os("SYSTEMROOT"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let system_fonts = windows_root.join("Fonts");
        for name in [
            "msyh.ttc",
            "msyhl.ttc",
            "msyh.ttf",
            "simsun.ttc",
            "simhei.ttf",
            "Deng.ttf",
        ] {
            paths.push(system_fonts.join(name));
        }

        if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
            for directory in [
                "AppData\\Local\\Microsoft\\Windows\\Fonts",
                "AppData\\Roaming\\Microsoft\\Windows\\Fonts",
            ] {
                let directory = profile.join(directory);
                for name in ["msyh.ttc", "msyhl.ttc", "msyh.ttf", "simsun.ttc"] {
                    paths.push(directory.join(name));
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        for path in [
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/Library/Fonts/Arial Unicode.ttf",
        ] {
            paths.push(PathBuf::from(path));
        }
    }

    #[cfg(target_os = "linux")]
    {
        for path in [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttf",
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttf",
            "/usr/share/fonts/truetype/arphic/uming.ttc",
            "/usr/share/fonts/truetype/arphic/ukai.ttc",
        ] {
            paths.push(PathBuf::from(path));
        }
        if let Some(home) = dirs::home_dir() {
            for directory in [home.join(".local/share/fonts"), home.join(".fonts")] {
                for name in [
                    "NotoSansCJK-Regular.ttc",
                    "NotoSansCJK-Regular.ttf",
                    "wqy-zenhei.ttc",
                    "wqy-zenhei.ttf",
                ] {
                    paths.push(directory.join(name));
                }
            }
        }
    }

    paths
}

/// Find a readable CJK font file for UI font registration or explicit
/// terminal fallback. The first existing file wins, keeping startup bounded.
pub fn find_cjk_font() -> Option<PathBuf> {
    cjk_font_candidates()
        .into_iter()
        .find(|path| path.is_file())
}

/// Common CJK family names used by Windows, macOS and Linux installations.
/// The list is intentionally broader than cosmic-text's platform defaults:
/// Windows images in particular may expose `Microsoft YaHei` instead of the
/// `Microsoft YaHei UI` family name.
pub fn cjk_fallback_families() -> &'static [&'static str] {
    &[
        "Microsoft YaHei UI",
        "Microsoft YaHei",
        "SimSun",
        "SimHei",
        "DengXian",
        "PingFang SC",
        "PingFang TC",
        "Hiragino Sans GB",
        "Noto Sans CJK SC",
        "Noto Sans CJK TC",
        "Noto Sans CJK JP",
        "Noto Sans Mono CJK SC",
        "WenQuanYi Zen Hei",
        "WenQuanYi Micro Hei",
        "AR PL UMing CN",
        "Arial Unicode MS",
    ]
}

/// Return whether the database already exposes one of our known CJK families.
/// This avoids loading the same system font a second time on Linux/macOS.
pub fn database_has_known_cjk_family(db: &cosmic_text::fontdb::Database) -> bool {
    db.faces().any(|face| {
        face.families.iter().any(|(family, _)| {
            cjk_fallback_families()
                .iter()
                .any(|candidate| family == candidate)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_family_catalog_contains_windows_and_linux_names() {
        let families = cjk_fallback_families();
        assert!(families.contains(&"Microsoft YaHei"));
        assert!(families.contains(&"Noto Sans CJK SC"));
    }

    #[test]
    fn candidate_paths_are_non_empty_and_absolute() {
        let paths = cjk_font_candidates();
        assert!(!paths.is_empty());
        assert!(paths.iter().all(|path| path.is_absolute()));
    }
}
