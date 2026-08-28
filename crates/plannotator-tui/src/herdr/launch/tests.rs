#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use super::*;
use crate::config::Config;
use crate::herdr::context::HerdrContext;

fn env(pane_id: Option<&str>, context: Option<HerdrContext>) -> HerdrEnv {
    HerdrEnv { in_herdr: true, pane_id: pane_id.map(str::to_owned), context, ..HerdrEnv::default() }
}

fn temp_folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plannotator-tui-launch-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("docs")).expect("temp dir");
    std::fs::write(dir.join("docs/plan.md"), "# plan\n").expect("file");
    dir
}

#[test]
fn human_keybind_opens_the_focused_folder_and_delivers_to_its_agent() {
    let root = temp_folder("keybind");
    let context = HerdrContext {
        focused_pane_id: Some("w1:p1".into()),
        focused_pane_cwd: Some(root.display().to_string()),
        focused_pane_agent: Some("claude".into()),
        workspace_cwd: Some("/elsewhere".into()),
        invocation_source: Some("keybinding".into()),
        ..HerdrContext::default()
    };
    let launch = plan(&env(None, Some(context)), &Config::default(), OpenArgs::default(), Path::new("/"))
        .expect("plans");
    assert_eq!(launch.file, root);
    assert_eq!(launch.cwd, root, "a folder is its own cwd");
    assert_eq!(launch.placement, Placement::Overlay);
    assert_eq!(launch.deliver, Some(Target { pane: "w1:p1".into(), agent: Some("claude".into()) }));
    assert_eq!(
        argv(&launch),
        vec![
            "plugin",
            "pane",
            "open",
            "--plugin",
            "plannotator-tui",
            "--entrypoint",
            "doc",
            "--placement",
            "overlay",
            "--focus",
            "--cwd",
            &root.display().to_string(),
            "--env",
            &format!("PLANNOTATOR_TUI_FILE={}", root.display()),
            "--env",
            "PLANNOTATOR_TUI_DELIVER_TO=w1:p1",
            "--env",
            "PLANNOTATOR_TUI_DELIVER_AGENT=claude",
        ]
    );
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn agent_skill_delivers_to_and_splits_beside_the_calling_pane() {
    let root = temp_folder("skill");
    let config = Config::parse("[herdr]\nplacement = \"split\"\n").expect("config");
    let args = OpenArgs { path: Some(PathBuf::from("docs/plan.md")), ..OpenArgs::default() };
    let launch = plan(&env(Some("w2:p4"), None), &config, args, &root).expect("plans");
    assert_eq!(launch.file, root.join("docs/plan.md"), "relative paths resolve against the caller's cwd");
    assert_eq!(launch.cwd, root.join("docs"), "a file's cwd is its folder");
    assert_eq!(launch.deliver, Some(Target { pane: "w2:p4".into(), agent: None }));
    assert_eq!(launch.target_pane.as_deref(), Some("w2:p4"));
    let args = argv(&launch);
    let after_placement: Vec<&str> = args.iter().map(String::as_str).skip(7).take(6).collect();
    assert_eq!(after_placement, ["--placement", "split", "--direction", "right", "--target-pane", "w2:p4"]);
    assert!(args.iter().any(|a| a == "PLANNOTATOR_TUI_DELIVER_TO=w2:p4"));
    assert!(!args.iter().any(|a| a.starts_with("PLANNOTATOR_TUI_DELIVER_AGENT")));
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn ctrl_click_opens_the_linked_file() {
    let root = temp_folder("click");
    let url = format!("file://{}/docs/plan.md", root.display());
    let context = HerdrContext {
        focused_pane_id: Some("w1:p1".into()),
        focused_pane_agent: Some("claude".into()),
        focused_pane_cwd: Some("/should/not/win".into()),
        clicked_url: Some(url),
        ..HerdrContext::default()
    };
    let launch = plan(&env(None, Some(context)), &Config::default(), OpenArgs::default(), Path::new("/"))
        .expect("plans");
    assert_eq!(launch.file, root.join("docs/plan.md"));
    assert_eq!(launch.cwd, root.join("docs"));
    assert_eq!(launch.deliver.map(|t| t.pane), Some("w1:p1".into()));
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn only_file_urls_are_opened() {
    assert_eq!(file_url_path("file:///a/b%20c.md"), Some(PathBuf::from("/a/b c.md")));
    assert_eq!(file_url_path("file://localhost/a.md"), Some(PathBuf::from("/a.md")));
    assert_eq!(file_url_path("https://github.com/x/README.md"), None);
    assert_eq!(file_url_path("file://host/a.md"), None);
}

#[test]
fn popup_placement_emits_size_and_no_target_pane() {
    let config = Config::parse("[herdr]\npopup_width = \"100%\"\npopup_height = \"100%\"\n").expect("config");
    let args = OpenArgs { placement: Some(Placement::Popup), deliver_to: Some("w1:p1".into()), path: None };
    let launch = plan(&env(Some("w1:p9"), None), &config, args, Path::new("/tmp")).expect("plans");
    assert_eq!(launch.deliver, Some(Target { pane: "w1:p1".into(), agent: None }), "--deliver-to wins");
    let args = argv(&launch);
    assert!(args.windows(2).any(|w| w == ["--width", "100%"]));
    assert!(args.windows(2).any(|w| w == ["--height", "100%"]));
    assert!(!args.iter().any(|a| a == "--target-pane"));
}

#[test]
fn env_placement_beats_config_and_arg_beats_env() {
    let config = Config::parse("[herdr]\nplacement = \"popup\"\n").expect("config");
    let mut env = env(None, None);
    env.placement = Some("split".into());
    let launch = plan(&env, &config, OpenArgs::default(), Path::new("/tmp")).expect("plans");
    assert_eq!(launch.placement, Placement::Split);
    let args = OpenArgs { placement: Some(Placement::Overlay), ..OpenArgs::default() };
    assert_eq!(plan(&env, &config, args, Path::new("/tmp")).expect("plans").placement, Placement::Overlay);
    env.placement = Some("floating".into());
    assert!(plan(&env, &config, OpenArgs::default(), Path::new("/tmp")).is_err());
}

#[test]
fn run_refuses_outside_herdr() {
    let launch = plan(&HerdrEnv::default(), &Config::default(), OpenArgs::default(), Path::new("/tmp"))
        .expect("plans");
    let err = run(&HerdrEnv::default(), &launch).expect_err("refused");
    assert!(err.to_string().contains("not inside Herdr"));
}

#[test]
fn a_human_in_a_plain_shell_pane_gets_no_agent_target() {
    // A manifest action always carries a context; a focused pane without an agent is not a
    // target, and HERDR_PANE_ID (set for the action too) must not become one either.
    let context = HerdrContext { focused_pane_id: Some("w1:p2".into()), ..HerdrContext::default() };
    let env = env(Some("w1:p2"), Some(context));
    let launch = plan(&env, &Config::default(), OpenArgs::default(), Path::new("/tmp")).expect("plan");
    assert_eq!(launch.deliver, None);
    assert_eq!(launch.target_pane.as_deref(), Some("w1:p2"), "a split still opens beside the caller");
}

#[test]
fn the_plugin_id_comes_from_the_environment() {
    let env = HerdrEnv { in_herdr: true, plugin_id: Some("annotate".into()), ..HerdrEnv::default() };
    let launch = plan(&env, &Config::default(), OpenArgs::default(), Path::new("/tmp")).expect("plan");
    let args = argv(&launch);
    let plugin = args.iter().position(|a| a == "--plugin").and_then(|i| args.get(i + 1));
    assert_eq!(plugin.map(String::as_str), Some("annotate"));
}
