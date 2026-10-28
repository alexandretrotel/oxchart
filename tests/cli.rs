use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("oxchart-cli-{name}"));
    let _ = fs::remove_dir_all(&root);
    for (path, content) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    root
}

fn oxchart(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oxchart"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn app(name: &str) -> PathBuf {
    project(
        name,
        &[
            (
                "src/index.ts",
                "import './a'; import 'react'; import './missing';",
            ),
            ("src/a.ts", "export * from './b';"),
            ("src/b.ts", ""),
            ("node_modules/react/index.js", ""),
        ],
    )
}

#[test]
fn prints_the_graph_and_warnings_separately() {
    let dir = app("graph");
    let out = oxchart(&dir, &["src/index.ts"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let chart = stdout(&out);
    assert!(chart.starts_with("flowchart LR\n"), "{chart}");
    assert!(chart.contains("  m0 -->|re-export| m1\n"), "{chart}");
    assert!(chart.contains("  m2 ==>|static| m0\n"), "{chart}");
    let err = stderr(&out);
    assert!(
        err.contains("warning: src/index.ts: cannot find `./missing`"),
        "{err}"
    );
    assert!(err.contains("3 modules, 2 imports, 1 not found"), "{err}");
}

#[test]
fn quiet_prints_only_the_graph() {
    let dir = app("quiet");
    let out = oxchart(&dir, &["src/index.ts", "--quiet"]);
    assert!(out.status.success());
    assert_eq!(stderr(&out), "");
}

#[test]
fn output_file() {
    let dir = app("output");
    let out = oxchart(&dir, &["src/index.ts", "-o", "graph.mmd", "-q"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "");
    let written = fs::read_to_string(dir.join("graph.mmd")).unwrap();
    assert!(written.contains("  m2 ==>|static| m0\n"), "{written}");
}

#[test]
fn settings_file_is_found_from_a_subfolder_and_flags_win() {
    let dir = app("settings");
    fs::write(
        dir.join("oxchart.toml"),
        "entrypoints = [\"src/index.ts\"]\nexternals = \"leaf\"\n",
    )
    .unwrap();
    let sub = dir.join("src");

    let out = oxchart(&sub, &["-q"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("([\"react\"]):::package"));

    let out = oxchart(&sub, &["-q", "--externals", "hide"]);
    assert!(!stdout(&out).contains("react"));

    let out = oxchart(&sub, &["-q", "--no-config", "index.ts"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("[\"index.ts\"]"));
}

#[test]
fn bad_settings_file_says_what_is_wrong() {
    let dir = app("bad-settings");
    fs::write(dir.join("oxchart.toml"), "externals = \"skip\"\n").unwrap();
    let out = oxchart(&dir, &["src/index.ts"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("invalid `externals`"), "{err}");
    assert!(err.contains("expected one of: hide, leaf, follow"), "{err}");

    fs::write(dir.join("oxchart.toml"), "max_depth = 2\n").unwrap();
    let err = stderr(&oxchart(&dir, &["src/index.ts"]));
    assert!(err.contains("unknown field `max_depth`"), "{err}");
}

#[test]
fn no_entrypoint_explains_what_to_do() {
    let dir = app("no-entry");
    let out = oxchart(&dir, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("Pass the file your app starts from"));
}

#[test]
fn missing_entrypoint_fails() {
    let dir = app("missing-entry");
    let out = oxchart(&dir, &["src/nope.ts"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("entrypoint not found"));
}

#[test]
fn bad_flag_value_is_a_usage_error() {
    let dir = app("bad-flag");
    let out = oxchart(&dir, &["src/index.ts", "--edges", "statik"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("expected one of: static, dynamic"));
}

#[test]
fn modules_without_imports_are_still_listed() {
    let dir = project("alone", &[("index.ts", "")]);
    let out = oxchart(&dir, &["index.ts", "-q"]);
    assert!(stdout(&out).contains("  m0[\"index.ts\"]:::file\n"));
}

#[test]
fn mermaid_format_from_flag_or_settings_file() {
    let dir = app("mermaid");
    let out = oxchart(&dir, &["src/index.ts", "-q", "--format", "mermaid"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let chart = stdout(&out);
    assert!(chart.starts_with("flowchart LR\n"), "{chart}");
    assert!(chart.contains("m1[\"src/b.ts\"]"), "{chart}");

    fs::write(dir.join("oxchart.toml"), "format = \"mermaid\"\n").unwrap();
    let out = oxchart(&dir, &["src/index.ts", "-q"]);
    assert!(stdout(&out).starts_with("flowchart LR\n"));
    let out = oxchart(&dir, &["src/index.ts", "-q", "--format", "svg"]);
    assert!(stdout(&out).starts_with("<svg"));
}

#[test]
fn output_extension_picks_the_format() {
    let dir = app("extension");
    for (file, start) in [
        ("graph.svg", "<svg"),
        ("graph.mmd", "flowchart LR"),
        ("graph.mermaid", "flowchart LR"),
        ("graph.txt", "┌"),
    ] {
        let out = oxchart(&dir, &["src/index.ts", "-q", "-o", file]);
        assert!(out.status.success(), "{}", stderr(&out));
        let written = fs::read_to_string(dir.join(file)).unwrap();
        assert!(written.starts_with(start), "{file}: {written:.80}");
    }
    let out = oxchart(
        &dir,
        &[
            "src/index.ts",
            "-q",
            "-o",
            "graph.svg",
            "--format",
            "mermaid",
        ],
    );
    assert!(out.status.success());
    assert!(
        fs::read_to_string(dir.join("graph.svg"))
            .unwrap()
            .starts_with("flowchart LR")
    );
}

#[test]
fn piped_output_is_mermaid_and_svg_can_be_asked_for() {
    let dir = app("piped");
    assert!(stdout(&oxchart(&dir, &["src/index.ts", "-q"])).starts_with("flowchart LR"));
    let out = oxchart(&dir, &["src/index.ts", "-q", "--format", "svg"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).starts_with("<svg"));
}

#[test]
fn ascii_format_draws_in_the_terminal() {
    let dir = app("ascii");
    let out = oxchart(&dir, &["src/index.ts", "-q", "--format", "ascii"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let drawing = stdout(&out);
    assert!(drawing.contains("│ src/index.ts"), "{drawing}");
    assert!(drawing.contains("re-export"), "{drawing}");
}

#[test]
fn image_and_pdf_formats_from_the_extension() {
    let dir = app("images");
    for (file, start) in [
        ("graph.png", &b"\x89PNG"[..]),
        ("graph.jpg", &[0xFF, 0xD8, 0xFF][..]),
        ("graph.jpeg", &[0xFF, 0xD8, 0xFF][..]),
        ("graph.pdf", &b"%PDF-"[..]),
    ] {
        let out = oxchart(&dir, &["src/index.ts", "-q", "-o", file]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(
            fs::read(dir.join(file)).unwrap().starts_with(start),
            "{file}"
        );
    }
}
