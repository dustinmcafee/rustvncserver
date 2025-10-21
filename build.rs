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
            // On Linux, turbojpeg is typically available via system package manager
            // (libjpeg-turbo8-dev on Ubuntu/Debian)
            // The library is usually in standard system paths, so we just need to link it
            println!("cargo:rustc-link-lib=turbojpeg");
        }
        "windows" => {
            // On Windows, turbojpeg linking is typically handled differently
            // This is a placeholder for future Windows support
            println!("cargo:rustc-link-lib=turbojpeg");
        }
        _ => {
            // For other platforms, attempt standard linking
            println!("cargo:rustc-link-lib=turbojpeg");
        }
    }
}
