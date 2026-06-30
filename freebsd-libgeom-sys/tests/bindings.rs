// vim: tw=80

use std::{
    process::{Command, Stdio},
    str::FromStr,
};

/// Skip a test.
// Copied from nix.  Sure would be nice if the test harness knew about "skipped"
// tests as opposed to "passed" or "failed".
#[macro_export]
macro_rules! skip {
    ($($reason: expr),+) => {
        use ::std::io::{self, Write};

        let stderr = io::stderr();
        let mut handle = stderr.lock();
        writeln!(handle, $($reason),+).unwrap();
        return;
    }
}

#[macro_export]
macro_rules! require_command {
    ($command:expr) => {
        if ::std::process::Command::new($command)
            .arg("--version")
            .output()
            .is_err()
        {
            skip!("{} not available.  Skipping test", $command);
        }
    };
}

/// If bindgen is installed, run it to check that the libgeom bindings in the OS
/// haven't changed in a backwards-incompatible way.  It's important to run this
/// test on FreeBSD current every so often, but that's difficult to do in Github
/// Workflows.
#[test]
fn stable_bindings() {
    // Check for required commands
    require_command!("bindgen");
    require_command!("difft");

    // Don't bother running test on older OS Releases.  We know that we're
    // backwards compatible with them, but they lack newer constants that
    // confuse difft.
    let output = Command::new("uname")
        .arg("-U")
        .output()
        .expect("Failed to run uname");
    assert!(output.status.success(), "uname failed");
    let release =
        u32::from_str(String::from_utf8_lossy(&output.stdout).trim()).unwrap();
    if release <= 1501000 {
        // freebsd-libgeom-sys's libgeom bindings are known to be backwards
        // compatible with this OS version
        return;
    }

    let ffi_fname = match usize::BITS {
        32 => "ffi32.rs",
        64 => "ffi64.rs",
        _ => unimplemented!(),
    };
    let tf = tempfile::NamedTempFile::new().unwrap();
    let root_dir = env!("CARGO_MANIFEST_DIR");
    let output = Command::new("sh")
        .arg(format!("{}/bindgen/bindgen.sh", root_dir))
        .arg(tf.path())
        .stderr(Stdio::inherit())
        .stdout(Stdio::inherit())
        .output()
        .expect("failed to run bindgen");
    assert!(output.status.success(), "bindgen failed");
    let output = Command::new("difft")
        .arg("--exit-code")
        .arg("--ignore-comments")
        .arg(format!("{}/src/{}", root_dir, ffi_fname))
        .arg(tf.path())
        .stderr(Stdio::inherit())
        .output()
        .expect("failed to run difft");
    assert!(
        output.status.success(),
        "FFI definitions need refreshing: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
