fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=worker.manifest");
        println!("cargo:rerun-if-changed=worker.manifest.rc");
        embed_resource::compile("worker.manifest.rc", embed_resource::NONE)
            .manifest_required()
            .expect("the worker manifest is required");
    }
}
