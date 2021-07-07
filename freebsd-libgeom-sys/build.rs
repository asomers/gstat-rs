// vim: tw=80

use std::env;
use std::path::PathBuf;

fn main() {
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

