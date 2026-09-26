fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // GPUI's debug-mode window rendering can exceed the Windows executable's
    // small default main-thread stack. This reserves virtual address space;
    // pages are committed only as the stack grows.
    let arg = match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => "/STACK:16777216",
        Ok("gnu") => "-Wl,--stack,16777216",
        _ => return,
    };
    println!("cargo:rustc-link-arg-bin=browserctl-rs={arg}");
}
