fn main() {
    println!("cargo:rerun-if-changed=lowbattery.rc");
    println!("cargo:rerun-if-changed=../../assets/lowbattery.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("lowbattery.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
