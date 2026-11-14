//! `--headless --print` against this repo's own examples.

use std::process::Command;

#[test]
fn print_marks_the_start_project_active() {
    let hello = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hello");
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .args(["--headless", "--print", "-d", hello])
        .env("FUNCTOR_APP_HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("\nactive: examples/hello\n"), "{text}");
    assert!(
        text.contains("\n* examples/hello (hello): entry game.fun\n"),
        "{text}"
    );
}

#[test]
fn print_lists_paired_devices_without_their_secrets() {
    let hello = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hello");
    let home = tempfile::tempdir().unwrap();
    let secret = "ab".repeat(32);
    let pairing = format!(
        r#"[{{"id":"0a1b2c3d4e5f60718293a4b5c6d7e8f9","name":"Old Quest","secret":"{secret}","first_seen":1,"last_seen":2}}]"#
    );
    std::fs::write(home.path().join("devices.json"), pairing).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .args(["--headless", "--print", "-d", hello])
        .env("FUNCTOR_APP_HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("\nheadset: away (link off)\n  Old Quest 0a1b2c3d: away\n"),
        "{text}"
    );
    assert!(!text.contains(&secret), "{text}");
}

/// `--print` says what is wrong with the `functor` it found (D40).
#[cfg(unix)]
#[test]
fn print_says_the_functor_is_too_old() {
    use std::os::unix::fs::PermissionsExt;
    let hello = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hello");
    let home = tempfile::tempdir().unwrap();
    let old = home.path().join("functor");
    let script = "#!/bin/sh\necho \"error: unrecognized subcommand '$1'\" >&2\nexit 2\n";
    std::fs::write(&old, script).unwrap();
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .args(["--headless", "--print", "-d", hello, "--functor"])
        .arg(&old)
        .env("FUNCTOR_APP_HOME", home.path())
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("\ncli: The functor CLI at ~/functor was built before this app, "),
        "{text}"
    );
}
