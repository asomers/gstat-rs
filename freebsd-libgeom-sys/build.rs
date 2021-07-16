// vim: tw=80

#[cfg(target_os = "freebsd")]
fn main() {
    use std::env;
    use std::path::PathBuf;

    println!("cargo:rustc-link-lib=geom");
    let bindings = bindgen::Builder::default()
        .header("/usr/include/libgeom.h")
        .header("/usr/include/sys/devicestat.h")
        .whitelist_function("geom_.*")
        .whitelist_function("gctl_.*")
        .whitelist_function("g_.*")
        .whitelist_type("devstat_trans_flags")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .generate()
        .expect("Unable to generate bindings");
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}

#[cfg(not(target_os = "freebsd"))]
fn main() {
    // If we're building not on FreeBSD, there's no way the build can succeed.
    // This probably means we're building docs on docs.rs, so set this config
    // variable.  We'll use it to stub out the crate well enough that
    // freebsd-libgeom's docs can build.
    println!("cargo:rustc-cfg=crossdocs");
}
