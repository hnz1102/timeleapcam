extern crate cc;
extern crate cmake;

fn main() -> anyhow::Result<()> {

    let compiler = "xtensa-esp32s3-elf-gcc";
    let compiler_path = std::env::var("DEP_ESP_IDF_EMBUILD_ENV_PATH").expect("Failed to get compiler path");
    let mut build = cc::Build::new();

    std::env::set_var("PATH", compiler_path);
    build.target("xtensa-esp32s3-none-elf");
    build.compiler(compiler);

    embuild::build::CfgArgs::output_propagated("ESP_IDF")?;
    embuild::build::LinkArgs::output_propagated("ESP_IDF")
}
