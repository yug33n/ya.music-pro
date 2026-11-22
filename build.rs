const APP_NAME: &str = "ya.music-pro";

fn main() {
    println!("cargo:rustc-env=APP_NAME={APP_NAME}");
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .unwrap();
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        resources::embed();
    }
}

/// Ресурсы exe: описание и значок из icons/app.ico
#[cfg(windows)]
mod resources {
    use super::APP_NAME;
    use std::path::{Path, PathBuf};

    pub fn embed() {
        let Some(rc) = find_rc() else {
            println!("cargo:warning=rc.exe не найден, exe без значка и описания");
            return;
        };
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("icons").join("app.ico");
        println!("cargo:rerun-if-changed={}", icon.display());
        std::fs::copy(&icon, out.join("app.ico")).expect("icons/app.ico");
        let script = format!("#pragma code_page(65001)\n1 ICON \"app.ico\"\n{}", version_info());
        std::fs::write(out.join("app.rc"), script).unwrap();

        let res = out.join("app.res");
        let ok = std::process::Command::new(rc)
            .current_dir(&out)
            .args(["/nologo", "/fo"])
            .arg(&res)
            .arg("app.rc")
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            println!("cargo:rustc-link-arg-bins={}", res.display());
        } else {
            println!("cargo:warning=rc.exe завершился с ошибкой, exe без значка и описания");
        }
    }

    fn version_info() -> String {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let exe = std::env::var("CARGO_PKG_NAME").unwrap();
        let nums: Vec<&str> = version.split(['.', '-']).take(3).collect();
        let num = format!("{},0", nums.join(","));
        format!(
            r#"1 VERSIONINFO
FILEVERSION {num}
PRODUCTVERSION {num}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "041904B0"
    BEGIN
      VALUE "FileDescription", "{APP_NAME}"
      VALUE "ProductName", "{APP_NAME}"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "InternalName", "{exe}"
      VALUE "OriginalFilename", "{exe}.exe"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x419, 1200
  END
END
"#
        )
    }

    fn find_rc() -> Option<PathBuf> {
        let kits = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
        let mut vers: Vec<PathBuf> = std::fs::read_dir(kits).ok()?.filter_map(|e| e.ok().map(|e| e.path())).collect();
        vers.sort();
        vers.into_iter().rev().map(|v| v.join("x64").join("rc.exe")).find(|p| p.exists())
    }
}
