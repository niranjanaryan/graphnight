use std::env;
use std::path::PathBuf;

fn main() {
    // Tell cargo to rerun this script if the Python library changes
    println!("cargo:rerun-if-changed=build.rs");

    // On macOS with abi3, we need to link against the Python framework
    if cfg!(target_os = "macos") {
        // Try to find Python framework
        if let Ok(python_lib) = env::var("PYTHON_LIB") {
            println!("cargo:rustc-link-search={}", python_lib);
        } else {
            // Common locations for Python framework on macOS
            let possible_paths = vec![
                "/Library/Frameworks/Python.framework/Versions/Current/lib",
                "/opt/homebrew/Frameworks/Python.framework/Versions/Current/lib",
                "/usr/local/Frameworks/Python.framework/Versions/Current/lib",
            ];

            for path in possible_paths {
                if PathBuf::from(path).exists() {
                    println!("cargo:rustc-link-search={}", path);
                    break;
                }
            }
        }

        // Link against Python framework
        println!("cargo:rustc-link-lib=framework=Python");
    }
}