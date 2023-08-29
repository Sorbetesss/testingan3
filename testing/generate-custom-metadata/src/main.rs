use codec::Encode;

/// can be called from the root of the project with: `cargo run --bin generate-custom-metadata`.
/// Generates a "./artifacts/metadata_with_custom_values.scale" file.
fn main() {
    let metadata_prefixed = generate_custom_metadata::metadata_custom_values_foo();
    let bytes = metadata_prefixed.encode();
    std::fs::write("./artifacts/metadata_with_custom_values.scale", bytes)
        .expect("should be able to write custom metadata");
}
