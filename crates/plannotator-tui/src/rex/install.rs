//! `plannotator-tui rex install`: write Rex Annotate, the Lua module that gives Rex its
//! review actions and keys, beside Rex's `init.lua`.
//!
//! Rex has no plugin manifest, but its config file can `require` Lua files stored beside it.
//! The module is embedded in this binary and names it by absolute path, since Rex runs actions
//! with a bare `PATH`; running install again after an upgrade keeps the two in step. Install
//! never edits `init.lua`: it prints the one line to add.

#![allow(clippy::print_stdout, reason = "install reports to the person who ran it")]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

/// The module, with `@BIN@` where the binary's path goes.
const MODULE: &str = include_str!("annotate.lua");
/// What `init.lua` needs to load the module, at `plannotator/annotate.lua` beside it.
pub(crate) const REQUIRE: &str = "require(\"plannotator.annotate\").setup{}";
const MODULE_PATH: [&str; 2] = ["plannotator", "annotate.lua"];

/// What writing the module did, or would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Created,
    Updated,
    Unchanged,
}

pub(crate) fn run(args: &[String]) -> Result<()> {
    let (mut dry_run, mut print) = (false, false);
    for arg in args {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--print" => print = true,
            other => anyhow::bail!(
                "unexpected argument {other:?}\nusage: plannotator-tui rex install [--dry-run | --print]"
            ),
        }
    }
    let exe = std::env::current_exe().context("finding the plannotator-tui binary")?;
    let module = module(&exe)?;
    if print {
        print!("{module}");
        return Ok(());
    }
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let init = init_path(|k| std::env::var(k).ok(), &home);
    let target = module_path(&init);
    let outcome = write(&target, &module, dry_run)?;
    let verb = match (outcome, dry_run) {
        (Outcome::Created, false) => "created",
        (Outcome::Created, true) => "would create",
        (Outcome::Updated, false) => "updated",
        (Outcome::Updated, true) => "would update",
        (Outcome::Unchanged, _) => "unchanged",
    };
    println!("{verb} {}", target.display());
    if loads_module(&init) {
        println!("{} already loads it; run `rex config reload` to pick up changes.", init.display());
    } else {
        println!("Add this line to {}, then run `rex config reload`:\n\n  {REQUIRE}\n", init.display());
    }
    check(&module);
    Ok(())
}

/// The module, naming `exe` as the plannotator-tui its actions run.
pub(crate) fn module(exe: &Path) -> Result<String> {
    anyhow::ensure!(exe.is_absolute(), "the plannotator-tui path {} is not absolute", exe.display());
    let path = exe.to_str().with_context(|| format!("{} is not valid UTF-8", exe.display()))?;
    Ok(MODULE.replacen("@BIN@", &lua_string(path), 1))
}

/// `text` as a double-quoted Lua 5.1 string.
fn lua_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(out, "\\{}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Rex's config file: `$REX_CONFIG`, else `rex/init.lua` in the config directory.
pub(crate) fn init_path(env: impl Fn(&str) -> Option<String>, home: &Path) -> PathBuf {
    if let Some(explicit) = env("REX_CONFIG").filter(|s| !s.is_empty()) {
        return PathBuf::from(explicit);
    }
    #[cfg(windows)]
    if let Some(appdata) = env("APPDATA").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return appdata.join("rex").join("init.lua");
    }
    let xdg = env("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    xdg.unwrap_or_else(|| home.join(".config")).join("rex").join("init.lua")
}

/// Where `require("plannotator.annotate")` finds the module for `init`.
pub(crate) fn module_path(init: &Path) -> PathBuf {
    let dir = init.parent().unwrap_or_else(|| Path::new("."));
    MODULE_PATH.iter().fold(dir.to_path_buf(), |path, part| path.join(part))
}

/// Write `module` to `target` unless it is already there; with `dry_run`, only say so.
pub(crate) fn write(target: &Path, module: &str, dry_run: bool) -> Result<Outcome> {
    let outcome = match std::fs::read_to_string(target) {
        Ok(existing) if existing == module => return Ok(Outcome::Unchanged),
        Ok(_) => Outcome::Updated,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Outcome::Created,
        Err(e) => return Err(e).with_context(|| format!("reading {}", target.display())),
    };
    if !dry_run {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(target, module).with_context(|| format!("writing {}", target.display()))?;
    }
    Ok(outcome)
}

/// Whether `init` already loads the module, so the person has nothing to add.
fn loads_module(init: &Path) -> bool {
    std::fs::read_to_string(init).is_ok_and(|text| text.contains("plannotator.annotate"))
}

/// Ask `rex config check` about a scratch config holding only the module and its `require`
/// line, so the person's own `init.lua` is neither read nor touched. Without `rex`, say so.
fn check(module: &str) {
    let rex = super::command::installed_bin();
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("plannotator-tui-rex-check-{}-{nanos}", std::process::id()));
    let result = scratch_config(&dir, module).and_then(|init| {
        let output = Command::new(&rex)
            .args(["--autostart=false", "config", "check"])
            .arg(&init)
            .output()
            .with_context(|| format!("running {}", rex.display()));
        std::fs::remove_dir_all(&dir).ok();
        output
    });
    match result {
        Ok(output) if output.status.success() => println!("rex config check: the module loads cleanly"),
        Ok(output) => {
            println!("rex config check found problems:");
            print!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        }
        Err(err) => println!("skipped `rex config check`: {err:#}"),
    }
}

/// `dir/init.lua` with the `require` line, and the module beside it. `dir` must not exist yet: it
/// is created private to this user, so nobody else can plant or swap the Lua that Rex then runs.
fn scratch_config(dir: &Path, module: &str) -> Result<PathBuf> {
    let builder = &mut std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(builder, 0o700);
    builder.create(dir).with_context(|| format!("creating {}", dir.display()))?;
    let init = dir.join("init.lua");
    let written = write(&module_path(&init), module, false).and_then(|_| {
        std::fs::write(&init, format!("{REQUIRE}\n")).with_context(|| format!("writing {}", init.display()))
    });
    if written.is_err() {
        std::fs::remove_dir_all(dir).ok();
    }
    written.map(|()| init)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn scratch(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("plannotator-tui-install-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Scratch(dir)
    }

    #[cfg(unix)]
    fn lookup(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        |k| vars.iter().find(|(name, _)| *name == k).map(|(_, v)| (*v).to_owned())
    }

    // Unix paths: on Windows `/home/me` is not absolute. Rex runs on macOS.
    #[cfg(unix)]
    #[test]
    fn the_module_goes_where_rex_reads_its_config_from() {
        let home = Path::new("/home/me");
        let init = init_path(lookup(&[]), home);
        assert_eq!(init, Path::new("/home/me/.config/rex/init.lua"));
        assert_eq!(module_path(&init), Path::new("/home/me/.config/rex/plannotator/annotate.lua"));
        let xdg = lookup(&[("XDG_CONFIG_HOME", "/xdg"), ("REX_CONFIG", "")]);
        assert_eq!(init_path(xdg, home), Path::new("/xdg/rex/init.lua"));
        assert_eq!(
            init_path(lookup(&[("XDG_CONFIG_HOME", "relative")]), home),
            init,
            "a relative XDG dir is ignored"
        );
        let explicit =
            init_path(lookup(&[("REX_CONFIG", "/etc/rex/main.lua"), ("XDG_CONFIG_HOME", "/xdg")]), home);
        assert_eq!(module_path(&explicit), Path::new("/etc/rex/plannotator/annotate.lua"));
    }

    // Unix paths: on Windows `/home/me` is not absolute. Rex runs on macOS.
    #[cfg(unix)]
    #[test]
    fn the_module_names_this_binary_as_a_lua_string() {
        let text = module(Path::new("/opt/a \"b\"\\c/plannotator-tui")).expect("module");
        assert!(text.contains(r#"M.bin = "/opt/a \"b\"\\c/plannotator-tui""#), "{text}");
        assert!(!text.contains("@BIN@"));
        assert!(module(Path::new("plannotator-tui")).is_err(), "a relative path would break under Rex");
        assert_eq!(lua_string("a\nb"), "\"a\\10b\"");
    }

    #[test]
    fn install_creates_then_leaves_alone_then_updates_and_a_dry_run_writes_nothing() {
        let dir = scratch("write");
        let target = module_path(&dir.0.join("init.lua"));
        assert_eq!(write(&target, "one", true).expect("dry run"), Outcome::Created);
        assert!(!target.exists(), "a dry run writes nothing");
        assert_eq!(write(&target, "one", false).expect("write"), Outcome::Created);
        assert_eq!(write(&target, "one", false).expect("again"), Outcome::Unchanged);
        assert_eq!(write(&target, "two", true).expect("dry run"), Outcome::Updated);
        assert_eq!(std::fs::read_to_string(&target).expect("read"), "one");
        assert_eq!(write(&target, "two", false).expect("update"), Outcome::Updated);
        assert_eq!(std::fs::read_to_string(&target).expect("read"), "two");
    }

    #[test]
    fn a_config_that_already_requires_the_module_needs_no_new_line() {
        let dir = scratch("loads");
        let init = scratch_config(&dir.0, "return {}").expect("scratch config");
        assert!(loads_module(&init));
        std::fs::write(&init, "rex.bind(\"cmd+d\", \"pane.split\")\n").expect("write");
        assert!(!loads_module(&init));
        assert!(!loads_module(&dir.0.join("missing.lua")));
    }

    #[test]
    fn the_scratch_config_never_writes_into_a_directory_it_did_not_create() {
        let dir = scratch("planted");
        std::fs::create_dir_all(&dir.0).expect("plant");
        assert!(scratch_config(&dir.0, "return {}").is_err());
        assert!(!dir.0.join("init.lua").exists(), "nothing is written into a planted directory");
    }

    #[cfg(unix)]
    #[test]
    fn the_scratch_config_is_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("private");
        scratch_config(&dir.0, "return {}").expect("scratch config");
        let mode = std::fs::metadata(&dir.0).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}
