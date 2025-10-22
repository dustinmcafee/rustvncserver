// Copyright 2025 Dustin McAfee
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::env;
use std::path::PathBuf;

fn main() {
    // Only configure linking if turbojpeg feature is enabled
    if env::var("CARGO_FEATURE_TURBOJPEG").is_err() {
        return;
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();

    match target_os.as_str() {
        "macos" => {
            // On macOS, turbojpeg is typically installed via Homebrew
            // We need to add the Homebrew library path to the linker search path
            let homebrew_paths = vec![
                "/opt/homebrew/opt/jpeg-turbo/lib", // Apple Silicon (M1/M2/M3)
                "/usr/local/opt/jpeg-turbo/lib",    // Intel Macs
            ];

            for path in homebrew_paths {
                let path_buf = PathBuf::from(path);
                if path_buf.exists() {
                    println!("cargo:rustc-link-search=native={}", path);
                    println!("cargo:rustc-link-lib=turbojpeg");
                    // Found the library, no need to check other paths
                    return;
                }
            }

            // If neither path exists, still try to link (might be in system path)
            println!("cargo:rustc-link-lib=turbojpeg");
        }
        "linux" => {
            // Skip host library paths when cross-compiling for Android
            // Android target uses RUSTFLAGS from build.gradle to specify library paths
            if let Ok(target_env) = env::var("CARGO_CFG_TARGET_ENV") {
                if target_env == "gnu" {
                    // Check if building for Android (will have TARGET containing "android")
                    if let Ok(target) = env::var("TARGET") {
                        if target.contains("android") {
                            // For Android, let build.gradle's RUSTFLAGS handle library paths
                            // Don't add host PC library paths
                            return;
                        }
                    }
                }
            }

            // On Linux (non-Android), turbojpeg is typically available via system package manager
            // (libjpeg-turbo8-dev on Ubuntu/Debian)
            // Add common library paths for different architectures
            if let Ok(target_arch) = env::var("CARGO_CFG_TARGET_ARCH") {
                let lib_path = match target_arch.as_str() {
                    "x86_64" => "/usr/lib/x86_64-linux-gnu",
                    "aarch64" => "/usr/lib/aarch64-linux-gnu",
                    "arm" => "/usr/lib/arm-linux-gnueabihf",
                    _ => "/usr/lib",
                };
                println!("cargo:rustc-link-search=native={}", lib_path);
            }
            println!("cargo:rustc-link-lib=turbojpeg");
        }
        "windows" => {
            // On Windows, turbojpeg is typically installed via vcpkg
            // Check for VCPKG_ROOT environment variable
            if let Ok(vcpkg_root) = env::var("VCPKG_ROOT") {
                let lib_path = format!("{}\\installed\\x64-windows\\lib", vcpkg_root);
                println!("cargo:rustc-link-search=native={}", lib_path);
            } else if let Ok(vcpkg_root) = env::var("VCPKG_INSTALLATION_ROOT") {
                let lib_path = format!("{}\\installed\\x64-windows\\lib", vcpkg_root);
                println!("cargo:rustc-link-search=native={}", lib_path);
            }
            println!("cargo:rustc-link-lib=turbojpeg");
        }
        _ => {
            // For other platforms, attempt standard linking
            println!("cargo:rustc-link-lib=turbojpeg");
        }
    }
}
