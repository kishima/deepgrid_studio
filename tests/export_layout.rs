//! Integration test for `scripts/export-game.sh` (plan11 受け入れ基準:
//! 成果物に saves/ と *.ron.bak が含まれない)。The script is bash+coreutils
//! only, so the test drives it directly with a dummy binary via
//! `DEEPGRID_EXPORT_BIN`.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn export_excludes_saves_and_backups() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = std::env::temp_dir().join(format!("deepgrid_export_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);

    // A fixture project littered with play-time artifacts.
    let fixture = tmp.join("fixgame");
    write(&fixture.join("project.ron"), "(\n    name: \"Fixture Game\",\n    version: 8,\n)\n");
    write(&fixture.join("levels/level00.ron"), "()");
    write(&fixture.join("levels/level00.ron.bak"), "old");
    write(&fixture.join("project.ron.bak"), "old");
    write(&fixture.join("saves/slot1.ron"), "(save)");

    let bin = tmp.join("dummy_bin");
    write(&bin, "#!/bin/sh\n");

    let out = tmp.join("out");
    let status = Command::new("bash")
        .arg(repo.join("scripts/export-game.sh"))
        .arg(&fixture)
        .arg(&out)
        .env("DEEPGRID_EXPORT_BIN", &bin)
        .output()
        .expect("run export-game.sh");
    assert!(
        status.status.success(),
        "export failed: {}\n{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );

    let exported = out.join("assets/projects/fixgame");
    assert!(exported.join("project.ron").is_file(), "project copied");
    assert!(!exported.join("saves").exists(), "saves/ must be excluded");

    // No *.ron.bak anywhere in the artifact.
    fn no_baks(dir: &Path) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                no_baks(&p);
            } else {
                assert!(
                    !p.to_string_lossy().ends_with(".ron.bak"),
                    "backup leaked: {}",
                    p.display()
                );
            }
        }
    }
    no_baks(&out);

    // Only the exported project ships.
    let projects: Vec<_> = fs::read_dir(out.join("assets/projects"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(projects, vec!["fixgame"], "only the target project ships");

    // Launch config pins the project in play_only mode; docs ship too.
    let launch = fs::read_to_string(out.join("deepgrid.ron")).unwrap();
    assert!(launch.contains("play_only: true"), "{launch}");
    assert!(launch.contains("assets/projects/fixgame"), "{launch}");
    assert!(out.join("CREDITS.md").is_file());
    let readme = fs::read_to_string(out.join("README.md")).unwrap();
    assert!(readme.contains("Fixture Game"), "game name in the player README");

    let _ = fs::remove_dir_all(&tmp);
}
