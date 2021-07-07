// vim: tw=80

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-link-lib=geom");
    let bindings = bindgen::Builder::default()
        .header("/usr/include/libgeom.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .generate()
        .expect("Unable to generate bindings");
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}

