// Los schemas vienen en librespot-protocol, pero su build.rs no los compila.
// Generar la copia mínima sin modificar esa dependencia ni necesitar protoc.
fn main() {
    for name in ["track_descriptors", "collection2v2"] {
        println!("cargo:rerun-if-changed=proto/{name}.proto");
        protobuf_codegen::Codegen::new()
            .pure()
            .cargo_out_dir(name)
            .include("proto")
            .input(format!("proto/{name}.proto"))
            .run_from_script();
    }
}
