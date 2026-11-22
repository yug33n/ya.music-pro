const APP_NAME: &str = "ya.music-pro";

fn main() {
    println!("cargo:rustc-env=APP_NAME={APP_NAME}");
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .unwrap();
}
