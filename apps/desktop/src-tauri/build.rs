fn main() {
    // Menor privilégio: cada comando exposto precisa de permissão explícita na capability.
    let app = tauri_build::AppManifest::new().commands(&[
        "store_status",
        "detect_local_gateway",
        "approve_executable",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(app))
        .expect("falha no tauri-build");
}
