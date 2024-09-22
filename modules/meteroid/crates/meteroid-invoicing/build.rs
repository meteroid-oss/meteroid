use fluent_static_codegen::{generate, FunctionPerMessageCodeGenerator};

pub fn main() {
    generate!(
        "./l10n",
        FunctionPerMessageCodeGenerator::new("en-US"),
        "l10n.rs"
    );
}
