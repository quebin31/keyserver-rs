fn main() {
    prost_build::compile_protos(&["src/proto/database.proto"], &["src/"]).unwrap();
}
