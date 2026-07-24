fn main() {
    println!("cargo:rerun-if-changed=assets/minutes-graph-worker-Info.plist");
    println!("cargo:rerun-if-changed=assets/minutes-audio-worker-Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let manifest_dir = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    for worker in ["minutes-graph-worker", "minutes-audio-worker"] {
        let template_path = manifest_dir.join(format!("assets/{worker}-Info.plist"));
        let template = std::fs::read_to_string(&template_path)
            .unwrap_or_else(|_| panic!("{worker} Info.plist must be readable"));
        let marker = "__MINUTES_VERSION__";
        assert_eq!(
            template.matches(marker).count(),
            2,
            "{worker} Info.plist must contain exactly two version markers"
        );
        let plist = out_dir.join(format!("{worker}-Info.plist"));
        std::fs::write(
            &plist,
            template.replace(marker, &std::env::var("CARGO_PKG_VERSION").unwrap()),
        )
        .unwrap_or_else(|_| panic!("{worker} Info.plist must be generated"));
        println!(
            "cargo:rustc-link-arg-bin={worker}=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
}
