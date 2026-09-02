fn main() {
    println!("cargo:rerun-if-changed=assets/icons/moqcast.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/moqcast.ico")
            .compile()
            .expect("failed to embed the MoQCast application icon");
    }
}
