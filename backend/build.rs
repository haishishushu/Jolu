fn main() {
    println!("cargo:rerun-if-env-changed=JOLU_UPDATES_ENABLED");
    tauri_build::build()
}
