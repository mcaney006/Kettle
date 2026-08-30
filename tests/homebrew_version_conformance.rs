use kettle::domain::version_cmp;
use std::{cmp::Ordering, process::Command};

const CORPUS: &[&str] = &[
    "0.28.0",
    "1.0",
    "1.0rc1",
    "1.0_1",
    "1.2.3_10",
    "1.3.30-stable",
    "2.36.34",
    "26.825.41651",
    "20260817.0",
    "999999999999999999999999999999999999.2",
    "1..2---rc01",
];

#[test]
#[ignore = "optional conformance probe; requires Homebrew"]
fn local_installed_directory_order_matches_homebrew_for_corpus() {
    let Some(brew) = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
    else {
        eprintln!("Homebrew is not installed; skipping conformance probe");
        return;
    };
    let script = r#"require "pkg_version"; ARGV.each_slice(2) { |a, b| puts(PkgVersion.parse(a) <=> PkgVersion.parse(b)) }"#;
    let mut command = Command::new(brew);
    command.args(["ruby", "-e", script]);
    for &left in CORPUS {
        for &right in CORPUS {
            command.args([left, right]);
        }
    }
    let output = command.output().expect("invoke brew ruby directly");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let authoritative: Vec<Ordering> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| match line {
            "-1" => Ordering::Less,
            "0" => Ordering::Equal,
            "1" => Ordering::Greater,
            other => panic!("unexpected Homebrew comparison {other:?}"),
        })
        .collect();
    let mut index = 0;
    for &left in CORPUS {
        for &right in CORPUS {
            assert_eq!(
                version_cmp(left, right),
                authoritative[index],
                "Kettle and Homebrew disagree for {left:?} versus {right:?}"
            );
            index += 1;
        }
    }
}
