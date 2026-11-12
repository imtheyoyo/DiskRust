//! Script de construction : intègre le manifeste Windows (uniquement sous Windows).
//! Sur les autres systèmes, ce script ne fait rien.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=windows/diskrust-gui.rc");
    println!("cargo:rerun-if-changed=windows/diskrust-gui.manifest");
    #[cfg(windows)]
    {
        // Ne lie la ressource qu'au binaire `diskrust-gui`.
        embed_resource::compile_for(
            "windows/diskrust-gui.rc",
            ["diskrust-gui"],
            embed_resource::NONE,
        )
        .manifest_required()
        .expect("échec de l'intégration du manifeste Windows");
    }
}
