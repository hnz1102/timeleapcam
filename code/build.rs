extern crate cc;
extern crate cmake;

fn main() -> anyhow::Result<()> {

    // let _ = git2::Repository::clone("https://github.com/espressif/esp32-camera.git", "esp32-camera");
    let compiler = "xtensa-esp32s3-elf-gcc";
    let compiler_path = std::env::var("DEP_ESP_IDF_EMBUILD_ENV_PATH").expect("Failed to get compiler path");
    let mut build = cc::Build::new();

    // set env var
    std::env::set_var("PATH", compiler_path);
    build.target("xtensa-esp32s3-none-elf");
    build.compiler(compiler);

    // build.file("ESP32-OV5640-AF/src/ESP32_OV5640_AF.cpp");
    // build.flag_if_supported("-Wall");
    // build.flag_if_supported("-Iesp-camera-rs/esp32-camera/driver/include");
    // build.flag_if_supported("-I.embuild/espressif/esp-idf/v5.2.1/components/esp_common/include");
    // build.compile("autofocus");
    // println!("cargo:rustc-link-lib=autofocus");
    // println!("cargo:rerun-if-changed=ESP32-OV5640-AF/src/ESP32_OV5640_AF.cpp");

    embuild::build::CfgArgs::output_propagated("ESP_IDF")?;
    embuild::build::LinkArgs::output_propagated("ESP_IDF")
}
