// SPDX-License-Identifier: GPL-3.0-or-later

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("material".into());
    slint_build::compile_with_config("ui/main.slint", config).expect("compiling the Slint UI");
}
