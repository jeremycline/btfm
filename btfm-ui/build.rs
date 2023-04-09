fn main() {
    glib_build_tools::compile_resources(
        &["src"],
        "src/btfm-ui.gresource.xml",
        "compiled.gresource",
    );
}
