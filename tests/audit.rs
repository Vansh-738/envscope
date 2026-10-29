use envscope::{Options, Report, manifest_names, scan};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn project(source: &str, example: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("app.py"), source).unwrap();
    fs::write(dir.path().join(".env.example"), example).unwrap();
    dir
}

fn options(dir: &TempDir) -> Options {
    Options {
        root: dir.path().to_path_buf(),
        example: PathBuf::from(".env.example"),
        excludes: vec![],
        ignore_keys: BTreeSet::new(),
        template: false,
    }
}

fn audit(source: &str) -> Report {
    let dir = project(source, "");
    scan(&options(&dir))
}

fn names(report: &Report) -> Vec<&str> {
    report.missing.keys().map(String::as_str).collect()
}

#[test]
fn finds_real_lookups_and_locations_without_matching_strings() {
    let report = audit(
        "import os\n# os.getenv('COMMENT')\ns = \"os.environ['STRING']\"\na = os.environ['TOKEN']\nb = os.getenv('PORT', '8000')\nc = os.environ.get('HOST')\n",
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(names(&report), ["HOST", "PORT", "TOKEN"]);
    assert_eq!(report.missing["TOKEN"][0].line, 4);
    assert_eq!(report.missing["TOKEN"][0].column, 5);
}

#[test]
fn import_aliases_and_multiline_calls() {
    let report = audit(
        "import os as system\nfrom os import environ as env, getenv as read\na = system.getenv(\n 'A'\n)\nb = env['B']\nc = read(key='C')\nd = env.get(key='D')\n",
    );
    assert_eq!(names(&report), ["A", "B", "C", "D"]);
}

#[test]
fn does_not_confuse_other_modules_or_unimported_names() {
    assert!(
        audit("from other import getenv\ngetenv('NO')\n")
            .accesses
            .is_empty()
    );
    assert!(audit("os.getenv('NO')\n").accesses.is_empty());
}

#[test]
fn respects_function_scopes_and_parameters() {
    let report = audit(
        "import os\ndef real():\n return os.getenv('YES')\ndef shadow(os):\n return os.getenv('NO')\ndef local():\n from os import getenv as read\n return read('LOCAL')\ndef unrelated():\n return read('NO')\n",
    );
    assert_eq!(names(&report), ["LOCAL", "YES"]);
}

#[test]
fn function_assignment_shadows_import_for_entire_scope() {
    let report = audit(
        "import os\ndef f():\n value = os.getenv('NO')\n os = object()\n return value\nvalue = os.getenv('YES')\n",
    );
    assert_eq!(names(&report), ["YES"]);
}

#[test]
fn class_attributes_do_not_shadow_module_names_inside_methods() {
    let report = audit(
        "import os\nclass App:\n os = object()\n def run(self):\n  return os.getenv('YES')\n",
    );
    assert_eq!(names(&report), ["YES"]);
}

#[test]
fn dynamic_keys_are_visible_without_printing_the_expression() {
    let report = audit(
        "import os\nkey = 'TOKEN'\na = os.getenv(key)\nb = os.environ[f'PREFIX_{key}']\nc = os.getenv('API' + '_KEY')\n",
    );
    assert!(report.accesses.is_empty());
    assert_eq!(report.unresolved.len(), 3);
    assert!(!serde_json::to_string(&report).unwrap().contains("PREFIX"));
}

#[test]
fn ordinary_writes_and_deletes_are_not_reads() {
    let report = audit(
        "import os\nos.environ['WRITE'] = 'x'\ndel os.environ['DELETE']\nos.environ['UPDATE'] += 'x'\n",
    );
    assert_eq!(names(&report), ["UPDATE"]);
}

#[test]
fn comprehension_and_lambda_parameters_shadow_imports() {
    let report = audit(
        "import os\na = [os.getenv('NO') for os in []]\nb = lambda os: os.getenv('ALSO_NO')\nc = os.getenv('YES')\n",
    );
    assert_eq!(names(&report), ["YES"]);
}

#[test]
fn accepts_plain_raw_unicode_and_triple_quoted_names() {
    let report = audit(
        "import os\na = os.getenv(r'RAW')\nb = os.getenv(u'UNICODE')\nc = os.getenv(\"\"\"TRIPLE\"\"\")\nd = os.getenv('ESC\\x41PED')\n",
    );
    assert_eq!(names(&report), ["RAW", "TRIPLE", "UNICODE"]);
    assert_eq!(report.unresolved.len(), 1);
}

#[test]
fn example_supports_exports_bom_crlf_and_multiline_values() {
    let names = manifest_names("\u{feff}# comment\r\nexport TOKEN = anything\r\nCERT=\"line one\nFAKE_KEY=inside value\nlast line\" # ignored\nPORT=8000\n").unwrap();
    assert_eq!(
        names.into_iter().collect::<Vec<_>>(),
        ["CERT", "PORT", "TOKEN"]
    );
}

#[test]
fn manifest_errors_never_echo_values() {
    for input in [
        "bad-key=private-value",
        "private-value",
        "TOKEN=\"private-value",
    ] {
        let error = manifest_names(input).unwrap_err();
        assert!(!error.contains("private-value"));
    }
}

#[test]
fn compares_unique_names_and_keeps_every_location() {
    let dir = project(
        "import os\na = os.getenv('KNOWN')\nb = os.getenv('MISSING')\nc = os.environ['MISSING']\n",
        "KNOWN=private-value\n",
    );
    let report = scan(&options(&dir));
    assert_eq!(names(&report), ["MISSING"]);
    assert_eq!(report.missing["MISSING"].len(), 2);
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("private-value")
    );
}

#[test]
fn gitignore_hidden_dirs_exclusions_and_ignored_keys() {
    let dir = project(
        "import os\nos.getenv('HOME')\nos.getenv('KEEP')\n",
        "KEEP=\n",
    );
    fs::write(dir.path().join(".gitignore"), "generated/\n").unwrap();
    for folder in ["generated", ".venv", "tests"] {
        fs::create_dir(dir.path().join(folder)).unwrap();
        fs::write(
            dir.path().join(folder).join("x.py"),
            "import os\nos.getenv('SKIP')\n",
        )
        .unwrap();
    }
    let mut opts = options(&dir);
    opts.excludes.push("tests/**".into());
    opts.ignore_keys.insert("HOME".into());
    let report = scan(&opts);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.missing.is_empty(), "{:?}", report.missing);
    assert_eq!(report.files_scanned, 1);
}

#[test]
fn syntax_errors_fail_instead_of_claiming_a_clean_scan() {
    let report = audit("import os\ndef broken(:\n os.getenv('TOKEN')\n");
    assert_eq!(report.errors.len(), 1);
    assert!(report.accesses.is_empty());
}

#[test]
fn missing_manifest_and_bad_root_are_errors() {
    let dir = project("", "");
    fs::remove_file(dir.path().join(".env.example")).unwrap();
    assert_eq!(scan(&options(&dir)).errors.len(), 1);
    let mut opts = options(&dir);
    opts.root = dir.path().join("missing");
    assert_eq!(scan(&opts).errors.len(), 1);
}

#[test]
fn cli_exit_codes_json_template_and_dynamic_policy() {
    let dir = project("import os\nos.getenv('TOKEN')\nos.getenv(key)\n", "");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_envscope"))
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let missing = run(&["--format", "json"]);
    assert_eq!(missing.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert!(json["missing"].get("TOKEN").is_some());
    fs::write(dir.path().join(".env.example"), "TOKEN=\n").unwrap();
    assert_eq!(run(&[]).status.code(), Some(0));
    assert_eq!(run(&["--deny-dynamic"]).status.code(), Some(1));
    fs::remove_file(dir.path().join(".env.example")).unwrap();
    let template = run(&["--format", "template"]);
    assert_eq!(template.status.code(), Some(0));
    assert_eq!(String::from_utf8(template.stdout).unwrap(), "TOKEN=\n");
    assert_eq!(run(&[]).status.code(), Some(2));
}

#[test]
fn template_is_empty_when_any_file_has_syntax_errors() {
    let dir = project("import os\nos.getenv('TOKEN')\n", "");
    fs::write(dir.path().join("broken.py"), "def broken(:\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_envscope"))
        .arg(dir.path())
        .args(["--format", "template"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn does_not_follow_source_symlinks() {
    let dir = project("", "");
    let outside = project("import os\nos.getenv('OUTSIDE')\n", "");
    std::os::unix::fs::symlink(outside.path().join("app.py"), dir.path().join("link.py")).unwrap();
    let report = scan(&options(&dir));
    assert_eq!(report.files_scanned, 1);
    assert!(report.accesses.is_empty());
}

#[test]
fn tuple_subscripts_are_unresolved_instead_of_using_the_first_name() {
    let report = audit("import os\nos.environ['A', 'B']\n");
    assert!(report.accesses.is_empty());
    assert_eq!(report.unresolved.len(), 1);
}

#[test]
fn example_handles_escaped_quotes_and_export_whitespace() {
    let names =
        manifest_names("export\tTOKEN=\nCERT='first\\' line\nFAKE=inside\nlast'\nPORT=\n").unwrap();
    assert_eq!(
        names.into_iter().collect::<Vec<_>>(),
        ["CERT", "PORT", "TOKEN"]
    );
}
