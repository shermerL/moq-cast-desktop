fn main() {
    println!("cargo:rerun-if-env-changed=MOQCAST_PROVENANCE_FILE");
    assert!(
        std::env::var_os("MOQCAST_PROVENANCE_FILE").is_none(),
        "MOQCAST_PROVENANCE_FILE is no longer an input; use MOQCAST_PROVENANCE_OUTPUT for the generated record"
    );
    moqcast_build_provenance::generate().expect("failed to generate build provenance");
}
