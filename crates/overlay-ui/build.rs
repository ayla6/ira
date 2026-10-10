fn main() {
    glib_build_tools::compile_resources(
        &["resources"],
        "resources/ira-overlay.gresource.xml",
        "ira-overlay.gresource",
    );
}
