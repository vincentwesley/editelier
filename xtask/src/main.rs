//! `cargo xtask <task>`: check, layers, fetch-media, package [--dev-cert|--portable], accept <id>|--milestone m1.
//! Plain std + serde_json; shells out to cargo, npm, PowerShell and Microsoft's winapp CLI.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, RandomState};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::{env, fs};

use serde_json::Value;

type R<T = ()> = Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let res = match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("layers") => layers(),
        Some("fetch-media") => fetch_media(),
        Some("package") if flag("--portable") => {
            portable().map(|p| println!("ZIP: {}", p.display()))
        }
        Some("package") => package(flag("--dev-cert")).map(|p| println!("MSIX: {}", p.display())),
        Some("accept") => accept(&args[1..]),
        _ => Err("usage: cargo xtask check | layers | fetch-media | package [--dev-cert|--portable] | accept <id>... | accept --milestone m1".into()),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run(cmd: &mut Command) -> R {
    eprintln!("$ {cmd:?}");
    let s = cmd
        .status()
        .map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    if s.success() {
        Ok(())
    } else {
        Err(format!("{:?} failed ({s})", cmd.get_program()))
    }
}

fn output(cmd: &mut Command) -> R<String> {
    let o = cmd
        .output()
        .map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    if !o.status.success() {
        return Err(format!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

fn cargo() -> Command {
    let mut c = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn npm() -> Command {
    let mut c = Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" });
    c.current_dir(root().join("ui"));
    c
}

fn powershell(script: &Path) -> Command {
    let mut c = Command::new("powershell");
    c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script);
    c
}

fn read(p: &Path) -> R<String> {
    fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn copy(from: &Path, to: &Path) -> R {
    fs::copy(from, to)
        .map(drop)
        .map_err(|e| format!("copy {}: {e}", from.display()))
}

// ---------- check / layers / fetch-media ----------

fn check() -> R {
    run(cargo().args(["fmt", "--all", "--check"]))?;
    run(cargo().args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]))?;
    run(cargo().args(["test", "--workspace"]))?;
    run(npm().args(["run", "check"]))
}

fn forbidden(name: &str) -> bool {
    [
        "windows", "winapi", "wgpu", "naga", "tauri", "wry", "tao", "webview2",
    ]
    .iter()
    .any(|p| name == *p || name.strip_prefix(p).is_some_and(|r| r.starts_with('-')))
}

/// Forbidden crates reachable from `krate` over normal (non-dev, non-build) edges.
fn forbidden_deps(meta: &Value, krate: &str) -> R<Vec<String>> {
    let names: HashMap<&str, &str> = meta["packages"]
        .as_array()
        .ok_or("cargo metadata: no packages")?
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p["name"].as_str()?)))
        .collect();
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for n in meta["resolve"]["nodes"]
        .as_array()
        .ok_or("cargo metadata: no resolve")?
    {
        let deps = n["deps"].as_array().into_iter().flatten().filter(|d| {
            d["dep_kinds"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|k| k["kind"].is_null())
        });
        edges.insert(
            n["id"].as_str().unwrap_or(""),
            deps.filter_map(|d| d["pkg"].as_str()).collect(),
        );
    }
    let start = names
        .iter()
        .find(|(_, n)| **n == krate)
        .map(|(id, _)| *id)
        .ok_or(format!("{krate} not in workspace"))?;
    let (mut seen, mut stack, mut bad) = (HashSet::new(), vec![start], Vec::new());
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if forbidden(names.get(id).copied().unwrap_or("")) {
            bad.push(names[id].to_string());
        }
        stack.extend(edges.get(id).into_iter().flatten());
    }
    bad.sort();
    bad.dedup();
    Ok(bad)
}

fn metadata(platform: Option<&str>) -> R<Value> {
    let mut c = cargo();
    c.args(["metadata", "--format-version", "1"]);
    if let Some(p) = platform {
        c.args(["--filter-platform", p]);
    }
    serde_json::from_str(&output(&mut c)?).map_err(|e| format!("cargo metadata: {e}"))
}

fn layers() -> R {
    // editelier-core: no platform/UI/GPU crate on any target. editelier-media: its Windows
    // backends are `cfg(windows)`; the platform-free part must build clean for a non-Windows target.
    let checks = [
        ("editelier-core", metadata(None)?),
        (
            "editelier-media",
            metadata(Some("x86_64-unknown-linux-gnu"))?,
        ),
    ];
    let mut fail = Vec::new();
    for (krate, meta) in &checks {
        let bad = forbidden_deps(meta, krate)?;
        if bad.is_empty() {
            println!("layers: {krate} ok");
        } else {
            fail.push(format!("{krate} depends on {}", bad.join(", ")));
        }
    }
    if fail.is_empty() {
        Ok(())
    } else {
        Err(fail.join("; "))
    }
}

fn fetch_media() -> R {
    run(&mut powershell(&root().join("scripts/fetch-ffmpeg.ps1")))?;
    run(&mut powershell(&root().join("scripts/make-test-media.ps1")))
}

// ---------- package ----------

struct Identity {
    name: String,
    publisher: String,
    display: String,
}

fn identity() -> R<Identity> {
    let dir = root().join("packaging");
    let store = dir.join("identity.store.json");
    let p = if store.is_file() {
        store
    } else {
        dir.join("identity.dev.json")
    };
    let v: Value = serde_json::from_str(&read(&p)?).map_err(|e| format!("{}: {e}", p.display()))?;
    let f = |k: &str| {
        v[k].as_str()
            .map(str::to_owned)
            .ok_or(format!("{}: missing {k}", p.display()))
    };
    Ok(Identity {
        name: f("name")?,
        publisher: f("publisher")?,
        display: f("publisherDisplayName")?,
    })
}

fn version() -> R<String> {
    let conf: Value = serde_json::from_str(&read(&root().join("src-tauri/tauri.conf.json"))?)
        .map_err(|e| e.to_string())?;
    let v = conf["version"]
        .as_str()
        .ok_or("tauri.conf.json: no version")?;
    Ok(format!("{v}.0"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn manifest() -> R<String> {
    let id = identity()?;
    Ok(read(&root().join("packaging/AppxManifest.xml"))?
        .replace("{{NAME}}", &xml_escape(&id.name))
        .replace("{{PUBLISHER_DISPLAY_NAME}}", &xml_escape(&id.display))
        .replace("{{PUBLISHER}}", &xml_escape(&id.publisher))
        .replace("{{VERSION}}", &version()?))
}

/// A-MSIX-01. `ponytail:` substring checks, not a schema validation; `winapp package` validates the schema.
fn lint_manifest(m: &str) -> R {
    let mut bad = Vec::new();
    for (ok, what) in [
        (m.contains(r#"Name="runFullTrust""#), "runFullTrust missing"),
        (
            !m.contains("broadFileSystemAccess"),
            "broadFileSystemAccess declared",
        ),
        (
            m.contains(r#"MinVersion="10.0.22000.0""#),
            "MinVersion is not 10.0.22000.0",
        ),
        (
            m.contains("windows.fileTypeAssociation") && m.contains(".editelier<"),
            "file associations missing",
        ),
        (!m.contains("{{"), "unfilled placeholder"),
    ] {
        if !ok {
            bad.push(what);
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("manifest: {}", bad.join(", ")))
    }
}

/// Our LGPLv2.1 build (`packaging/build-ffmpeg.sh`) when present, else the dev-only BtbN build.
fn ffmpeg_dir() -> PathBuf {
    let lgpl = root().join("third_party/ffmpeg-lgpl");
    if lgpl.join("bin/ffmpeg.exe").is_file() {
        lgpl
    } else {
        root().join("third_party/ffmpeg")
    }
}

/// Release build + the payload shared by the MSIX and the portable ZIP: exe, FFmpeg helpers
/// and DLLs, `licences/`. Fonts are embedded in the exe; only their OFL.txt ships.
fn payload(lay: &Path) -> R {
    let root = root();
    let ff = ffmpeg_dir();
    if !ff.join("bin/ffmpeg.exe").is_file() {
        return Err("no FFmpeg: run packaging/build-ffmpeg.sh in WSL (or `cargo xtask fetch-media` for dev)".into());
    }
    if !ff.ends_with("ffmpeg-lgpl") {
        eprintln!(
            "warning: packaging the dev-only BtbN FFmpeg; A-LEGAL-01 fails on it (run packaging/build-ffmpeg.sh)"
        );
    }
    run(npm().args(["run", "build"]))?;
    run(cargo().args([
        "build",
        "--release",
        "-p",
        "editelier",
        "--features",
        "custom-protocol",
    ]))?;

    let _ = fs::remove_dir_all(lay);
    fs::create_dir_all(lay.join("licences/FFmpeg")).map_err(|e| e.to_string())?;
    copy(
        &root.join("target/release/editelier.exe"),
        &lay.join("editelier.exe"),
    )?;
    // FFmpeg helper exes + their DLLs, original names, beside the app (D3 amendment). No ffplay.
    for e in fs::read_dir(ff.join("bin"))
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.ends_with(".dll") || n == "ffmpeg.exe" || n == "ffprobe.exe" {
            copy(&e.path(), &lay.join(&n))?;
        }
    }
    let lic = lay.join("licences");
    for f in ["LICENSE", "NOTICE.md", "THIRD_PARTY_NOTICES.md"] {
        copy(&root.join(f), &lic.join(f))?;
    }
    copy(
        &root.join("crates/editelier-render/fonts/OFL.txt"),
        &lic.join("OFL.txt"),
    )?;
    copy(&ff.join("LICENSE.txt"), &lic.join("FFmpeg/LICENSE.txt"))?;
    let conf =
        output(Command::new(ff.join("bin/ffmpeg.exe")).args(["-hide_banner", "-buildconf"]))?;
    let pinned = read(&ff.join("PINNED.txt")).unwrap_or_default();
    fs::write(
        lic.join("FFmpeg/BUILD-FFMPEG.txt"),
        format!(
            "FFmpeg (https://ffmpeg.org) is used under the GNU LGPL (see LICENSE.txt); Vin's Atelier does not own it.\n\
             Build: {}\nSource: https://ffmpeg.org (exact source archives and build script attached to the GitHub Release).\n\n{conf}",
            pinned.trim()
        ),
    )
    .map_err(|e| e.to_string())
}

fn package(dev_cert: bool) -> R<PathBuf> {
    let m = manifest()?;
    lint_manifest(&m)?;
    let out = root().join("target/msix");
    let lay = out.join("layout");
    payload(&lay)?;
    fs::create_dir_all(lay.join("Assets")).map_err(|e| e.to_string())?;
    for e in fs::read_dir(root().join("packaging/Assets"))
        .map_err(|e| e.to_string())?
        .flatten()
    {
        copy(&e.path(), &lay.join("Assets").join(e.file_name()))?;
    }
    let man = lay.join("AppxManifest.xml");
    fs::write(&man, m).map_err(|e| e.to_string())?;

    let msix = out.join(format!("Editelier_{}_x64.msix", version()?));
    let _ = fs::remove_file(&msix);
    let mut c = winapp();
    c.arg("package")
        .arg(&lay)
        .arg("--manifest")
        .arg(&man)
        .arg("--output")
        .arg(&msix);
    if dev_cert {
        let (pfx, pw) = dev_cert_files(&identity()?.publisher)?;
        c.arg("--cert").arg(pfx).arg("--cert-password").arg(pw);
    } else {
        c.arg("--no-sign");
    }
    run(&mut c)?;
    Ok(msix)
}

/// Portable ZIP for GitHub Releases: the MSIX payload minus manifest/Assets, in a writable
/// folder so users can swap the LGPL FFmpeg files (D9, `packaging/PORTABLE.md`).
fn portable() -> R<PathBuf> {
    let v = version()?;
    let name = format!(
        "Editelier-{}-portable-x64",
        v.strip_suffix(".0").unwrap_or(&v)
    );
    let out = root().join("target/package");
    let dir = out.join(&name);
    payload(&dir)?;
    copy(
        &root().join("packaging/PORTABLE.md"),
        &dir.join("README-PORTABLE.txt"),
    )?;
    let zip = out.join(format!("{name}.zip"));
    let _ = fs::remove_file(&zip);
    // No zip crate in the lockfile: Compress-Archive, paths passed via env (no quoting).
    run(Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Compress-Archive -Path $env:XT_SRC -DestinationPath $env:XT_ZIP",
        ])
        .env("XT_SRC", &dir)
        .env("XT_ZIP", &zip))?;
    Ok(zip)
}

fn winapp() -> Command {
    let mut c = Command::new("winapp");
    c.env("WINAPP_CLI_TELEMETRY_OPTOUT", "1"); // no telemetry, matching the app's own stance
    c
}

/// Self-signed dev cert in `%LOCALAPPDATA%\Editelier-dev\` (never in the repo), created once.
fn dev_cert_files(publisher: &str) -> R<(PathBuf, String)> {
    let dir = PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unset")?)
        .join("Editelier-dev");
    let (pfx, pass, cer) = (
        dir.join("editelier-dev.pfx"),
        dir.join("editelier-dev.pass"),
        dir.join("editelier-dev.cer"),
    );
    if !pfx.is_file() {
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let s = RandomState::new();
        let pw: String = (0..2u8)
            .map(|i| format!("{:016x}", s.hash_one(i)))
            .collect();
        fs::write(&pass, &pw).map_err(|e| e.to_string())?;
        run(winapp()
            .args([
                "cert",
                "generate",
                "--publisher",
                publisher,
                "--export-cer",
                "--password",
                &pw,
                "--output",
            ])
            .arg(&pfx))?;
    }
    let pw = read(&pass)?.trim().to_string();
    println!(
        "Dev cert: {}\nTrust it once (admin PowerShell):\n  Import-Certificate -FilePath \"{}\" -CertStoreLocation Cert:\\LocalMachine\\TrustedPeople",
        pfx.display(),
        cer.display()
    );
    Ok((pfx, pw))
}

// ---------- accept ----------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    Cargo(&'static [&'static str]),
    Npm(&'static [&'static str]),
    ManifestLint,
    Msix,
    Legal01,
    Legal04,
    Legal05,
    /// `--measure` with 60 s phases; per-id verdict from its table.
    Measure60,
    /// `export_bench` (60 s timelines); per-id verdict from its BENCH lines.
    ExportBench,
    /// Off-screen real-app WebdriverIO suite (`e2e-app/`); per-id verdict from tagged test titles.
    E2e,
}
const QUALITY: Act = Act::Cargo(&[
    "test",
    "--release",
    "-p",
    "editelier-export",
    "--test",
    "mf_export",
    "quality_and_speed_1080p_4k",
    "--",
    "--ignored",
]);

/// Existing automation per ACCEPTANCE id, with how much of the id it covers.
fn plan(id: &str) -> Option<(Vec<Act>, &'static str)> {
    use Act::*;
    let core = |t: &'static [&'static str]| Cargo(t);
    // Lossless path: model cut -> MF decode -> FFV1/MKV via the helper -> framemd5 vs oracle trim.
    let lossless = |t: &'static str| {
        Cargo(match t {
            "01" => &[
                "test",
                "-p",
                "editelier-media",
                "--test",
                "cut_lossless",
                "a_cut_01",
            ],
            "02" => &[
                "test",
                "-p",
                "editelier-media",
                "--test",
                "cut_lossless",
                "a_cut_02",
            ],
            "03" => &[
                "test",
                "-p",
                "editelier-media",
                "--test",
                "cut_lossless",
                "a_cut_03",
            ],
            _ => &[
                "test",
                "-p",
                "editelier-media",
                "--test",
                "cut_lossless",
                "a_cut_04",
            ],
        })
    };
    Some(match id {
        "A-CUT-01" => (
            vec![
                core(&[
                    "test",
                    "-p",
                    "editelier-core",
                    "--test",
                    "core",
                    "trims_snap_to_frames_and_clamp_to_media",
                ]),
                lossless("01"),
            ],
            "full (720p GOP-250 fixture, 20 seeded pairs)",
        ),
        "A-CUT-02" => (
            vec![
                core(&[
                    "test",
                    "-p",
                    "editelier-core",
                    "--test",
                    "core",
                    "random_splits_then_joins_round_trip",
                ]),
                lossless("02"),
            ],
            "full (av_offset fixture: B-frames, elst, AAC priming)",
        ),
        "A-CUT-03" => (
            vec![
                core(&[
                    "test",
                    "-p",
                    "editelier-core",
                    "--test",
                    "core",
                    "join_refuses_non_contiguous_clips",
                ]),
                lossless("03"),
            ],
            "partial: video framemd5 across the join; audio sample count not checked",
        ),
        "A-CUT-04" => (
            vec![
                core(&[
                    "test",
                    "-p",
                    "editelier-core",
                    "--test",
                    "core",
                    "ripple_delete_closes_the_gap",
                ]),
                lossless("04"),
            ],
            "full",
        ),
        "A-CUT-05" => (
            vec![
                core(&[
                    "test",
                    "-p",
                    "editelier-media",
                    "--test",
                    "media",
                    "seek_exact_vfr",
                ]),
                core(&["test", "-p", "editelier", "pts_lookup_is_sample_and_hold"]),
            ],
            "partial: synthetic VFR, no C-phone-vfr",
        ),
        "A-CUT-06" => (
            vec![core(&[
                "test",
                "-p",
                "editelier-core",
                "--test",
                "core",
                "undo_all_and_redo_all_are_byte_exact",
            ])],
            "full",
        ),
        "A-CUT-07" => (
            vec![core(&[
                "test",
                "-p",
                "editelier-media",
                "--test",
                "media",
                "seek_exact_",
            ])],
            "full on synthetic fixtures",
        ),
        "A-QUAL-01" | "A-QUAL-02" => (vec![QUALITY], "synthetic 5 s pattern"),
        "A-PERF-03" | "A-PERF-04" => (
            vec![ExportBench],
            "60 s timeline (bench20 clip x3 + title + audio), YouTube preset",
        ),
        "A-QUAL-03" => (
            vec![core(&[
                "test",
                "-p",
                "editelier-render",
                "--test",
                "render",
                "title_golden_frame",
            ])],
            "partial: title golden only",
        ),
        "A-QUAL-06" => (
            vec![core(&[
                "test",
                "-p",
                "editelier-export",
                "--lib",
                "loudness",
            ])],
            "partial: unit gain maths, no ebur128 oracle",
        ),
        "A-QUAL-07" => (
            vec![core(&[
                "test",
                "-p",
                "editelier-export",
                "--test",
                "mf_export",
                "every_encoder_writes_conformant_mp4",
            ])],
            "full",
        ),
        "A-COL-01" => (
            vec![core(&[
                "test",
                "-p",
                "editelier-render",
                "--test",
                "render",
                "neutral_grade_round_trips",
            ])],
            "full",
        ),
        "A-PERF-01" => (
            vec![Measure60],
            "partial: 60 s, uploaded fps >= 29.5; no PresentMon drop count or render p95",
        ),
        "A-PERF-02" => (
            vec![Measure60],
            "partial: 4K60 via proxy, 60 s >= 30 fps; no HEVC source",
        ),
        "A-IMPORT-01" => (
            vec![core(&[
                "test",
                "-p",
                "editelier-media",
                "--test",
                "media",
                "probe_corpus",
            ])],
            "partial: local corpus only",
        ),
        "A-IMPORT-02" => (
            vec![core(&[
                "test",
                "-p",
                "editelier-media",
                "--test",
                "media",
                "audio_decode_and_waveform",
            ])],
            "partial",
        ),
        "A-UI-01" => (vec![E2e], "full (Short and YouTube paths)"),
        "A-UI-02" => (
            vec![E2e],
            "partial: import, split, title, music, export probed; no ripple delete/fade/5 checkpoint hashes",
        ),
        "A-UI-03" => (vec![E2e], "full"),
        "A-UI-04" => (
            vec![E2e],
            "partial: keys + toolbar buttons; no menu or History panel in M1",
        ),
        "A-UI-05" => (
            vec![Npm(&["run", "test:e2e"]), E2e],
            "partial: WebView2 axe on welcome/editor (both themes), shortcuts, export, About (dark); no settings dialog in M1",
        ),
        "A-UI-06" => (vec![E2e], "full"),
        "A-MSIX-01" => (vec![ManifestLint], "full except 'version bumped'"),
        "A-MSIX-02" | "A-MSIX-04" => (vec![Msix], "full"),
        "A-MSIX-03" => (
            vec![Msix],
            "partial: alias launch with --measure, no automation ping / sample project",
        ),
        "A-LEGAL-01" => (
            vec![Legal01],
            "full (BUILD-FFMPEG.txt match only on a packaged layout)",
        ),
        "A-LEGAL-04" => (vec![Legal04], "full (release exe)"),
        "A-LEGAL-05" => (
            vec![Legal05],
            "partial: HTTP crates + CSP; naming scan not automated",
        ),
        _ => return None,
    })
}

fn m1_ids() -> Vec<String> {
    let mut v = Vec::new();
    for (p, n) in [
        ("CUT", 7),
        ("QUAL", 7),
        ("PERF", 7),
        ("UI", 6),
        ("IMPORT", 3),
        ("MSIX", 5),
        ("LEGAL", 5),
    ] {
        v.extend((1..=n).map(|i| format!("A-{p}-{i:02}")));
    }
    v.push("A-COL-01".into());
    v
}

fn accept(args: &[String]) -> R {
    let ids = match args {
        [f, m] if f == "--milestone" && m == "m1" => m1_ids(),
        [f, ..] if f == "--milestone" => return Err("only --milestone m1 is defined".into()),
        [] => return Err("accept <id>... | --milestone m1".into()),
        ids => ids.to_vec(),
    };
    let dir = root().join("qa-output/accept");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut done: HashMap<Act, R<String>> = HashMap::new();
    let (mut fails, mut todo) = (Vec::new(), Vec::new());
    for id in &ids {
        let Some((acts, cover)) = plan(id) else {
            todo.push(id.clone());
            fs::write(
                dir.join(format!("{id}.json")),
                format!("{{\"id\":\"{id}\",\"status\":\"not-automated\"}}\n"),
            )
            .ok();
            continue;
        };
        let mut err = None;
        for a in acts {
            let r = done.entry(a).or_insert_with(|| act(a)).clone();
            if let Err(e) = r.and_then(|log| verdict(id, a, &log)) {
                err = Some(e);
            }
        }
        let status = if err.is_some() { "fail" } else { "pass" };
        println!(
            "{id}: {status} ({cover}){}",
            err.as_ref().map(|e| format!(": {e}")).unwrap_or_default()
        );
        let json =
            serde_json::json!({ "id": id, "status": status, "coverage": cover, "error": err });
        fs::write(dir.join(format!("{id}.json")), format!("{json}\n")).ok();
        if status == "fail" {
            fails.push(id.clone());
        }
    }
    if !todo.is_empty() {
        println!("not yet automated: {}", todo.join(", "));
    }
    println!(
        "note: media/GPU tests print SKIP and pass when fixtures or hardware are missing (run `cargo xtask fetch-media`)."
    );
    if fails.is_empty() {
        Ok(())
    } else {
        Err(format!("failed: {}", fails.join(", ")))
    }
}

/// Runs `cmd`, echoing and returning stdout + stderr whatever the exit status (the per-id
/// verdict reads the log); Err only when it can't start.
fn capture(cmd: &mut Command) -> R<String> {
    eprintln!("$ {cmd:?}");
    let o = cmd
        .output()
        .map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    println!("{log}");
    Ok(log)
}

/// PIDs of the processes the off-screen e2e can leave behind.
fn e2e_pids() -> HashSet<u32> {
    let mut pids = HashSet::new();
    for exe in ["tauri-driver.exe", "msedgedriver.exe", "editelier.exe"] {
        let Ok(o) = Command::new("tasklist")
            .args(["/FO", "CSV", "/NH", "/FI", &format!("IMAGENAME eq {exe}")])
            .output()
        else {
            continue;
        };
        pids.extend(
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.split("\",\"").nth(1)?.parse::<u32>().ok()),
        );
    }
    pids
}

/// Ensures `npm ci --ignore-scripts` ran in `dir` (worktrees start without node_modules).
fn npm_ci(dir: &Path) -> R {
    if dir.join("node_modules").is_dir() {
        return Ok(());
    }
    run(npm().current_dir(dir).args(["ci", "--ignore-scripts"]))
}

fn e2e() -> R<String> {
    let root = root();
    npm_ci(&root.join("ui"))?;
    run(npm().args(["run", "build"]))?;
    run(cargo().args([
        "build",
        "-p",
        "editelier",
        "--features",
        "agent,custom-protocol",
    ]))?;
    let dir = root.join("e2e-app");
    npm_ci(&dir)?;
    let before = e2e_pids();
    let log = capture(npm().current_dir(&dir).arg("test"));
    // tauri-driver / msedgedriver (and a stuck app) can outlive the run: stop only the new PIDs.
    for pid in e2e_pids().difference(&before) {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
    log
}

/// Per-id verdict from a shared log (bench tables, tagged e2e titles); other acts pass as run.
fn verdict(id: &str, a: Act, log: &str) -> R {
    let num = |s: Option<&str>| s.and_then(|s| s.trim().parse::<f64>().ok());
    match a {
        Act::Measure60 => {
            let (row, min) = match id {
                "A-PERF-01" => ("1080p30 original", 29.5),
                _ => ("4K60 via proxy", 30.0),
            };
            // Columns after the phase name: secs, video fps, ...
            let fps = log
                .lines()
                .find_map(|l| l.strip_prefix(row))
                .and_then(|r| num(r.split_whitespace().nth(1)))
                .ok_or(format!("no `{row}` row in the --measure output"))?;
            if fps >= min {
                Ok(())
            } else {
                Err(format!("{row}: {fps:.1} fps < {min}"))
            }
        }
        Act::ExportBench => {
            let (clip, budget) = match id {
                "A-PERF-03" => ("bench20_1080p30.mp4", 30.0),
                _ => ("bench20_2160p30.mp4", 80.0),
            };
            let line = log
                .lines()
                .find(|l| l.contains(&format!("BENCH {clip}:")))
                .ok_or(format!(
                    "no BENCH line for {clip} (fixture or GPU missing?)"
                ))?;
            let secs = num(line
                .split("frames in ")
                .nth(1)
                .and_then(|r| r.split(" s").next()))
            .ok_or(format!("unreadable: {line}"))?;
            if secs <= budget {
                Ok(())
            } else {
                Err(format!("{clip}: {secs:.1} s > {budget} s ({line})"))
            }
        }
        Act::E2e => {
            let tagged = |mark: &str| {
                log.lines()
                    .filter(|l| l.contains(mark) && l.contains(id))
                    .map(str::trim)
                    .collect::<Vec<_>>()
            };
            let failed = tagged("✖");
            if !failed.is_empty() {
                Err(format!("e2e failed: {}", failed.join(" | ")))
            } else if tagged("✓").is_empty() {
                Err("no passing e2e test tagged with this id (suite did not run?)".into())
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

/// Log-producing acts return their output; the rest an empty log.
fn act(a: Act) -> R<String> {
    match a {
        Act::Measure60 => capture(
            cargo()
                .args([
                    "run",
                    "--release",
                    "-p",
                    "editelier",
                    "--features",
                    "custom-protocol",
                    "--",
                    "--measure",
                ])
                .env("EDITELIER_MEASURE_SECS", "60"),
        ),
        Act::ExportBench => capture(cargo().args([
            "test",
            "--release",
            "-p",
            "editelier",
            "export_bench",
            "--",
            "--ignored",
            "--test-threads=1",
            "--nocapture",
        ])),
        Act::E2e => e2e(),
        other => act_checks(other).map(|()| String::new()),
    }
}

fn act_checks(a: Act) -> R {
    match a {
        Act::Cargo(args) => run(cargo().args(args)),
        Act::Npm(args) => run(npm().args(args)),
        Act::Measure60 | Act::ExportBench | Act::E2e => unreachable!("log acts run in act()"),
        Act::ManifestLint => lint_manifest(&manifest()?),
        Act::Msix => {
            let msix = root().join(format!("target/msix/Editelier_{}_x64.msix", version()?));
            if !msix.is_file() {
                return Err("no MSIX: run `cargo xtask package --dev-cert` first".into());
            }
            let cer = PathBuf::from(env::var_os("LOCALAPPDATA").unwrap_or_default())
                .join("Editelier-dev/editelier-dev.cer");
            run(powershell(&root().join("packaging/accept-msix.ps1"))
                .arg("-Msix")
                .arg(msix)
                .arg("-Cer")
                .arg(cer))
        }
        Act::Legal01 => {
            let lay = root().join("target/msix/layout/ffmpeg.exe");
            let exe = if lay.is_file() {
                lay
            } else {
                ffmpeg_dir().join("bin/ffmpeg.exe")
            };
            let q = |f: &str| output(Command::new(&exe).args(["-hide_banner", f]));
            let mut bad = Vec::new();
            let conf = q("-buildconf")?;
            bad.extend(
                ["--enable-gpl", "--enable-nonfree"]
                    .into_iter()
                    .filter(|f| conf.contains(f))
                    .map(str::to_owned),
            );
            for (list, names) in [
                ("-decoders", ["h264", "hevc", "aac", "aac_fixed"]),
                ("-encoders", ["h264", "hevc", "aac", "aac_fixed"]),
            ] {
                let text = q(list)?;
                for l in text.lines() {
                    if let Some(n) = l.split_whitespace().nth(1).filter(|n| names.contains(n)) {
                        bad.push(format!("{list} has {n}"));
                    }
                }
            }
            let build = exe.with_file_name("licences/FFmpeg/BUILD-FFMPEG.txt");
            if build.is_file() && !read(&build)?.contains(conf.trim()) {
                bad.push("BUILD-FFMPEG.txt does not match -buildconf".into());
            }
            if bad.is_empty() {
                Ok(())
            } else {
                Err(format!("{}: {}", exe.display(), bad.join(", ")))
            }
        }
        Act::Legal04 => {
            let exe = root().join("target/release/editelier.exe");
            let bytes = fs::read(&exe)
                .map_err(|e| format!("{}: {e} (run `cargo xtask package` first)", exe.display()))?;
            let needle = b"editelier-automation";
            if bytes.windows(needle.len()).any(|w| w == needle) {
                Err("release exe contains editelier-automation".into())
            } else {
                Ok(())
            }
        }
        Act::Legal05 => {
            let tree = output(cargo().args([
                "tree",
                "-p",
                "editelier",
                "-e",
                "normal",
                "--prefix",
                "none",
            ]))?;
            let http = [
                "reqwest",
                "hyper",
                "ureq",
                "curl",
                "isahc",
                "attohttpc",
                "surf",
                "minreq",
            ];
            let mut bad: Vec<String> = tree
                .lines()
                .filter_map(|l| l.split_whitespace().next())
                .filter(|n| http.contains(n))
                .map(str::to_owned)
                .collect();
            let conf = read(&root().join("src-tauri/tauri.conf.json"))?;
            let csp = conf
                .split("connect-src")
                .nth(1)
                .unwrap_or("")
                .split([';', '"'])
                .next()
                .unwrap_or("");
            bad.extend(
                csp.split_whitespace()
                    .filter(|s| !["'self'", "ipc:", "http://ipc.localhost"].contains(s))
                    .map(|s| format!("CSP connect-src {s}")),
            );
            bad.sort();
            bad.dedup();
            if bad.is_empty() {
                Ok(())
            } else {
                Err(bad.join(", "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_names() {
        assert!(
            forbidden("windows")
                && forbidden("windows-core")
                && forbidden("wgpu-hal")
                && forbidden("tauri-build")
        );
        assert!(!forbidden("windowfunctions") && !forbidden("serde") && !forbidden("taolib"));
    }

    #[test]
    fn layer_walk_finds_the_shells_gpu_and_windows_deps() {
        let bad = forbidden_deps(&metadata(None).unwrap(), "editelier").unwrap();
        assert!(bad.iter().any(|n| n == "wgpu") && bad.iter().any(|n| n == "tauri"));
    }

    #[test]
    fn dev_manifest_passes_lint() {
        lint_manifest(&manifest().unwrap()).unwrap();
        assert!(lint_manifest("<Package/>").is_err());
    }

    #[test]
    fn verdicts_read_the_logs() {
        let m = "phase secs video fps\n1080p30 original    60.0      29.9\n4K60 via proxy    60.0      28.0\n";
        assert!(verdict("A-PERF-01", Act::Measure60, m).is_ok());
        assert!(verdict("A-PERF-02", Act::Measure60, m).is_err());
        let b = "BENCH bench20_1080p30.mp4: 1800 frames in 12.50 s = 4.80x realtime on QSV\n";
        assert!(verdict("A-PERF-03", Act::ExportBench, b).is_ok());
        assert!(verdict("A-PERF-04", Act::ExportBench, b).is_err());
        let e = "  ✓ A-UI-01 A-UI-05: welcome\n  ✖ A-UI-05 A-UI-06: About\n";
        assert!(verdict("A-UI-01", Act::E2e, e).is_ok());
        assert!(verdict("A-UI-06", Act::E2e, e).is_err());
        assert!(verdict("A-UI-03", Act::E2e, e).is_err());
    }

    #[test]
    fn m1_ids_are_known_shape() {
        let ids = m1_ids();
        assert_eq!(ids.len(), 41);
        assert!(ids.contains(&"A-MSIX-05".to_string()) && plan("A-MSIX-05").is_none());
    }
}
