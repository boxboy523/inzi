use std::env;
use std::path::PathBuf;

fn main() {
    let dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let lib_path = PathBuf::from(dir).join("lib");
    println!("cargo:rustc-link-search=native={}", lib_path.display());

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-link-lib=Fwlib64");
        println!("cargo:rerun-if-changed=lib/Fwlib64.h");
        let bindings = bindgen::Builder::default()
            .header("lib/Fwlib64.h")
            .parse_callbacks(Box::new(bindgen::CargoCallbacks))
            .generate()
            .expect("Unable to generate bindings");
        let out_path = PathBuf::from("src");
        bindings
            .write_to_file(out_path.join("bindings_window.rs"))
            .expect("Couldn't write bindings!");
    }
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-link-lib=fwlib32");
        println!("cargo:rerun-if-changed=lib/Fwlib64.h");
        let bindings = bindgen::Builder::default()
            .header("lib/Fwlib64.h")
            .clang_arg("-DTCHAR=char")
            .parse_callbacks(Box::new(bindgen::CargoCallbacks))
            .generate()
            .expect("Unable to generate bindings");
        let out_path = PathBuf::from("src");
        bindings
            .write_to_file(out_path.join("bindings_linux.rs"))
            .expect("Couldn't write bindings!");
    }
    tauri_build::build()
}
