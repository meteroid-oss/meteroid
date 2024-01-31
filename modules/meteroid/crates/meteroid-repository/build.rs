fn main() -> Result<(), cornucopia_build::CornucopiaBuildError> {
    println!("cargo:rerun-if-changed=refinery");
    println!("cargo:rerun-if-changed=queries");

    cornucopia_build::generate()
}
