//! Register bundled public fonts for this process without changing desktop fonts.
use std::{ffi::CString, path::PathBuf};

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcConfigGetCurrent() -> *mut std::ffi::c_void;
    fn FcConfigAppFontAddFile(config: *mut std::ffi::c_void, file: *const u8) -> libc::c_int;
}

pub fn register() {
    let directory = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
        .map(|p| p.join("safeincloud-linux/fonts"));
    let Some(directory) = directory else {
        return;
    };
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    let fonts: &[(&str, &[u8])] = &[
        (
            "AtkinsonHyperlegibleNext.ttf",
            include_bytes!("../../assets/fonts/AtkinsonHyperlegibleNext.ttf"),
        ),
        (
            "Spectral-Regular.ttf",
            include_bytes!("../../assets/fonts/Spectral-Regular.ttf"),
        ),
        (
            "Spectral-Light.ttf",
            include_bytes!("../../assets/fonts/Spectral-Light.ttf"),
        ),
        (
            "RedHatMono.ttf",
            include_bytes!("../../assets/fonts/RedHatMono.ttf"),
        ),
    ];
    for (name, bytes) in fonts {
        let path = directory.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(*bytes)
            && std::fs::write(&path, bytes).is_err()
        {
            continue;
        }
        if let Ok(path) = CString::new(path.as_os_str().as_encoded_bytes()) {
            unsafe {
                FcConfigAppFontAddFile(FcConfigGetCurrent(), path.as_ptr().cast());
            }
        }
    }
}
