fn main() {
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .expect("slint");
    println!("cargo:rerun-if-changed=../../icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../icon.ico");
        res.set("ProductName", "To Picture");
        res.set("FileDescription", "To Picture");
        res.compile().expect("winresource");
    }
}
