fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("protoc binary not found");
    std::env::set_var("PROTOC", protoc);

    prost_build::compile_protos(&["proto/zmqcontainer.proto"], &["proto/"])
        .expect("failed to compile protos");
    println!("cargo:rerun-if-changed=proto/zmqcontainer.proto");
}
