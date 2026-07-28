fn main() {
    pkg_config::probe_library("xcb").unwrap();
}
