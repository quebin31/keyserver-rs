fn main() {
    prost_build::compile_protos(
        &[
            "src/proto/database.proto",
            "src/proto/keyserver.proto",
            "src/proto/wrapper.proto",
        ],
        &["src/"],
    )
    .unwrap();
}
