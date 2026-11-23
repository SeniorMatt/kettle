//! Compiles `src/overlay.wgsl` to SPIR-V for the Vulkan layer (no shader compiler needed on the build machine).

fn main() {
    println!("cargo:rerun-if-changed=src/overlay.wgsl");
    let src = std::fs::read_to_string("src/overlay.wgsl").expect("shader source");
    let module = naga::front::wgsl::parse_str(&src).expect("shader parses");
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .expect("shader is valid");
    // Positions are already in Vulkan's clip space (y down): naga must not flip them.
    let mut options = naga::back::spv::Options::default();
    options.flags.remove(naga::back::spv::WriterFlags::ADJUST_COORDINATE_SPACE);
    let words = naga::back::spv::write_vec(&module, &info, &options, None).expect("shader compiles to SPIR-V");
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("overlay.spv");
    std::fs::write(out, bytes).expect("write SPIR-V");
}
