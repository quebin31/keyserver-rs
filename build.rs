fn main() {
    prost_build::compile_protos(
        &[
            "src/proto/metadata/addressmetadata.proto",
            "src/proto/pop/paymentrequest.proto",
        ],
        &["src/"],
    )
    .unwrap();
}
