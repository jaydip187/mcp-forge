use super::*;

fn parse(args: &[&str]) -> Cli {
    Cli::try_parse_from(args).unwrap()
}

#[test]
fn serve_defaults() {
    let Command::Serve(s) = parse(&["mcp-forge", "serve"]).command else {
        panic!("expected serve");
    };
    assert_eq!(s.bind, DEFAULT_BIND.parse().unwrap());
}

#[test]
fn serve_flags() {
    let Command::Serve(s) = parse(&[
        "mcp-forge",
        "serve",
        "--bind",
        "0.0.0.0:9000",
        "--data-dir",
        "/tmp/x",
    ])
    .command
    else {
        panic!("expected serve");
    };
    assert_eq!(s.bind, "0.0.0.0:9000".parse().unwrap());
    assert_eq!(
        s.common.resolve_data_dir().unwrap(),
        PathBuf::from("/tmp/x")
    );
}

#[test]
fn non_loopback_requires_token() {
    let serve = |args: &[&str]| {
        let mut full = vec!["mcp-forge", "serve"];
        full.extend_from_slice(args);
        match parse(&full).command {
            Command::Serve(s) => s,
            _ => unreachable!(),
        }
    };
    let local = serve(&[]).security().unwrap();
    assert!(local.loopback_hosts_only && local.admin_token.is_none());
    assert!(serve(&["--bind", "0.0.0.0:7410"]).security().is_err());
    assert!(serve(&["--bind", "0.0.0.0:7410", "--admin-token", "short"])
        .security()
        .is_err());
    let open = serve(&[
        "--bind",
        "0.0.0.0:7410",
        "--admin-token",
        "0123456789abcdef",
    ])
    .security()
    .unwrap();
    assert!(!open.loopback_hosts_only);
    assert_eq!(open.admin_token.as_deref(), Some("0123456789abcdef"));
}

#[test]
fn export_import_args() {
    let Command::Export(e) = parse(&["mcp-forge", "export", "--project", "demo"]).command else {
        panic!("expected export");
    };
    assert_eq!((e.project.as_str(), e.out), ("demo", None));
    let Command::Import(i) = parse(&[
        "mcp-forge",
        "import",
        "--project",
        "demo",
        "tools.yaml",
        "--on-conflict",
        "replace",
    ])
    .command
    else {
        panic!("expected import");
    };
    assert_eq!(i.file, Some(PathBuf::from("tools.yaml")));
    assert_eq!(i.on_conflict, OnConflict::Replace);
    assert!(Cli::try_parse_from(["mcp-forge", "import"]).is_err());
}

#[test]
fn stdio_requires_project() {
    assert!(Cli::try_parse_from(["mcp-forge", "stdio"]).is_err());
    let Command::Stdio(s) = parse(&["mcp-forge", "stdio", "--project", "demo"]).command else {
        panic!("expected stdio");
    };
    assert_eq!(s.project, "demo");
}

#[test]
fn data_dir_defaults_under_home() {
    let args = CommonArgs { data_dir: None };
    if let Ok(dir) = args.resolve_data_dir() {
        assert!(dir.ends_with(DATA_DIR_NAME));
    }
}
