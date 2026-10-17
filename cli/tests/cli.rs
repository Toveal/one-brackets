use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;

const BOM: &[u8] = b"\xEF\xBB\xBF";

fn cli() -> Command {
    cargo_bin_cmd!("one-brackets")
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn fixture(name: &str) -> PathBuf {
    repo().join("tests/fixtures").join(name)
}

/// Bracket files written by the platform.
fn samples() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(repo().join("tests/fixtures"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.push(repo().join("examples/cluster/1cv8wsrv.lst"));
    files.sort();
    files
}

fn stdout(command: &mut Command) -> String {
    let output = command.assert().success().get_output().stdout.clone();
    String::from_utf8(output).unwrap()
}

fn stderr(command: &mut Command) -> String {
    let output = command.assert().failure().get_output().stderr.clone();
    String::from_utf8(output).unwrap()
}

#[test]
fn round_trips_samples_byte_for_byte() {
    for file in samples() {
        // Whitespace after the value is not kept.
        let original = fs::read(&file).unwrap().trim_ascii_end().to_vec();
        for format in ["json", "yaml", "xml"] {
            for compact in [false, true] {
                let mut export = cli();
                export
                    .args(["export", "-", "--to", format])
                    .pipe_stdin(&file)
                    .unwrap();
                if compact {
                    export.arg("--compact");
                }
                let exported = export.assert().success().get_output().stdout.clone();
                let mut import = cli();
                import
                    .args(["import", "-", "--from", format])
                    .write_stdin(exported);
                if !original.starts_with(BOM) {
                    import.arg("--no-bom");
                }
                let imported = import.assert().success().get_output().stdout.clone();
                assert!(
                    imported == original,
                    "{} via {format} (compact: {compact})",
                    file.display()
                );
            }
        }
    }
}

#[test]
fn exports_formats() {
    let file = fixture("value-map.txt");
    for format in ["json", "yaml", "xml"] {
        let output = stdout(cli().args(["export", "--to", format]).arg(&file));
        insta::assert_snapshot!(format!("value-map.{format}"), output);
    }
    let output = stdout(cli().args(["export", "--plain", "--compact"]).arg(&file));
    insta::assert_snapshot!("value-map.plain.json", output);
}

#[test]
fn takes_formats_from_extensions() {
    let dir = std::env::temp_dir().join(format!("one-brackets-cli-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = fixture("value-array.txt");
    for extension in ["json", "yml", "xml"] {
        let exported = dir.join(format!("value.{extension}"));
        let restored = dir.join(format!("value.{extension}.txt"));
        cli()
            .arg("export")
            .arg(&source)
            .arg("-o")
            .arg(&exported)
            .assert()
            .success();
        cli()
            .arg("import")
            .arg(&exported)
            .arg("-o")
            .arg(&restored)
            .arg("--no-bom")
            .assert()
            .success();
        assert_eq!(fs::read(&restored).unwrap(), fs::read(&source).unwrap());
    }
    // Without an extension that tells the format, export writes JSON.
    let output = stdout(cli().arg("export").arg(&source));
    assert!(output.starts_with('['), "{output}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn exports_event_log_records() {
    let log = repo().join("examples/v8log/20260908000000.lgp");
    let output = stdout(
        cli()
            .args(["export", "--lines", "--skip-lines", "3"])
            .arg(&log),
    );
    let records: Vec<&str> = output.lines().collect();
    assert!(records.len() > 1);
    assert!(
        records.iter().all(|record| record.starts_with("[2026")),
        "{output}"
    );

    let output = stdout(
        cli()
            .args(["export", "--lines", "--skip-lines", "3", "--to", "yaml"])
            .arg(&log),
    );
    assert_eq!(output.matches("---\n").count(), records.len());

    let output = stdout(
        cli()
            .args(["export", "--lines", "--skip-lines", "3", "--to", "xml"])
            .arg(&log),
    );
    assert!(output.trim_end().ends_with("</values>"));

    let error = stderr(cli().args(["export", "--lines"]).arg(&log));
    assert!(error.starts_with("error: failed to read `"), "{error}");
    // Without skipping the header its text is read as atoms up to the first `,`.
    assert!(error.contains("no list is open"), "{error}");
}

#[test]
fn formats_keeping_the_bom() {
    let file = fixture("form.txt");
    let output = cli()
        .arg("fmt")
        .arg(&file)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(output, fs::read(&file).unwrap());
}

#[test]
fn gets_nodes() {
    let file = fixture("value-string.txt");
    assert_eq!(stdout(cli().arg("get").arg(&file).arg("0")), "\"S\"\n");
    assert_eq!(
        stdout(cli().arg("get").arg(&file).arg("0").arg("--raw")),
        "S\n"
    );
    let output = stdout(cli().arg("get").arg(&file).args(["", "--to", "json"]));
    assert!(
        output.starts_with("[\"S\", \"")
            && output.ends_with(
                "\"]
"
            ),
        "{output}"
    );

    let error = stderr(cli().arg("get").arg(&file).arg("5"));
    assert!(error.starts_with("error: "), "{error}");
    assert!(
        error.contains("out of range") || error.contains("index 5"),
        "{error}"
    );

    let error = stderr(cli().arg("get").arg(&file).arg("a.b"));
    assert_eq!(
        error,
        "error: invalid path `a.b`: expected indices separated by dots: invalid digit found in string\n"
    );

    let error = stderr(cli().arg("get").arg(&file).args(["", "--raw"]));
    assert_eq!(
        error,
        "error: the node at `` is a list: --raw prints strings and atoms\n"
    );
}

#[test]
fn checks_files() {
    let output = stdout(cli().arg("check").arg(fixture("form.txt")));
    assert!(output.ends_with("form.txt: ok\n"), "{output}");

    let assert = cli()
        .args(["check", "-"])
        .write_stdin("{1,\n{2")
        .assert()
        .failure();
    let error = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert_eq!(
        error,
        "-: unexpected end of input at line 2, column 3: list opened at line 2, column 1 is not closed\n"
    );
}

#[test]
fn reports_import_errors() {
    let error = stderr(cli().args(["import", "-"]).write_stdin("[]"));
    assert_eq!(
        error,
        "error: cannot tell the input format from the file extension: pass --from\n"
    );

    let error = stderr(
        cli()
            .args(["import", "-", "--from", "json"])
            .write_stdin("[1,\n{\"atom\": \"a,b\"}]"),
    );
    assert_eq!(
        error,
        "error: failed to read `-`: atom \"a,b\" at line 2, column 1 contains ',' or '}': use a quoted string instead\n"
    );

    let error = stderr(
        cli()
            .args(["export", "-", "--to", "yaml", "--plain"])
            .write_stdin("{}"),
    );
    assert_eq!(error, "error: --plain applies to JSON only\n");
}
