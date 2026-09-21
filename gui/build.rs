// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::path::Path;

/// Vendored copy of the official Material 3 component set, taken from
/// `ui-libraries/material/src` of slint-ui/slint at tag v1.18.0. It is versioned
/// in lockstep with Slint, so it moves together with the `slint` dependency.
///
/// Vendored rather than fetched: this is the only component library Slint
/// ships, it has no crates.io package, and a UI that silently changes shape
/// when someone else tags a release is not a UI anyone can review.
const MATERIAL_LIB: &str = "material-1.18.0/material.slint";

fn main() {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR unset");
    let config = slint_build::CompilerConfiguration::new()
        .with_style("material".into())
        .with_library_paths(HashMap::from([(
            "material".to_string(),
            Path::new(&manifest_dir).join(MATERIAL_LIB),
        )]));
    slint_build::compile_with_config("ui/main.slint", config).expect("compiling the Slint UI");
}
