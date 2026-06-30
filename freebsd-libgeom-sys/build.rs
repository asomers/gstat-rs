// vim: tw=80

#[cfg(target_os = "freebsd")]
fn main() {
    println!("cargo::rustc-check-cfg=cfg(crossdocs)");
    println!("cargo:rustc-link-lib=geom");
}

#[cfg(not(target_os = "freebsd"))]
fn main() {
    println!("cargo::rustc-check-cfg=cfg(crossdocs)");
    // If we're building not on FreeBSD, there's no way the build can succeed.
    // This probably means we're building docs on docs.rs, so set this config
    // variable.  We'll use it to stub out the crate well enough that
    // freebsd-libgeom's docs can build.
    println!("cargo:rustc-cfg=crossdocs");
}
