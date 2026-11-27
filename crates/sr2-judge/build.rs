// The judge links at a fixed base far from the game, so the game exe can be mapped at its own base
// 0x400000 in the same process.
fn main() {
    println!("cargo:rustc-link-arg-bins=/DYNAMICBASE:NO");
    println!("cargo:rustc-link-arg-bins=/BASE:0x10000000");
    println!("cargo:rerun-if-changed=build.rs");
}
