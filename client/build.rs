use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/vmi.rc");
    println!("cargo:rerun-if-changed=assets/vmi.ico");

    let target = env::var("TARGET").expect("Cargo sets TARGET");
    if !target.ends_with("windows-msvc") {
        return;
    }

    let resource =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR")).join("vmi.res");
    let mut compiler = find_msvc_tools::find(&target, "rc.exe")
        .expect("Install the Windows SDK resource compiler to build the client with MSVC");
    let status = compiler
        .current_dir("assets")
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg("vmi.rc")
        .status()
        .expect("Could not run the Windows resource compiler");
    assert!(status.success(), "Could not compile the client icon");
    println!(
        "cargo:rustc-link-arg-bin=vmi-analysis-client={}",
        resource.display()
    );
}
