//! Unit tests for the engine's pure logic, grouped by module under test.

mod causal {
    use gitreceipts::causal::order;
    use gitreceipts::schema::Record;

    fn rec(json: &str) -> Record {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn walks_parent_chain_before_timestamps() {
        // b is a's child but has an EARLIER wall clock than orphan c
        let records = vec![
            rec(r#"{"type":"user","uuid":"a","timestamp":"2026-01-01T00:00:00Z"}"#),
            rec(
                r#"{"type":"assistant","uuid":"b","parentUuid":"a","timestamp":"2026-01-01T00:00:01Z"}"#,
            ),
            rec(r#"{"type":"user","uuid":"c","timestamp":"2026-01-01T00:00:00.500Z"}"#),
        ];
        let ordered = order(records);
        let uuids: Vec<&str> = ordered.iter().filter_map(|r| r.uuid.as_deref()).collect();
        assert_eq!(uuids, vec!["a", "b", "c"]);
    }

    #[test]
    fn fractional_second_timestamps_sort_as_instants() {
        // "…00.500Z" string-sorts before "…00Z"; as instants it is later
        let records = vec![
            rec(r#"{"type":"user","uuid":"late","timestamp":"2026-01-01T00:00:00.500Z"}"#),
            rec(r#"{"type":"user","uuid":"early","timestamp":"2026-01-01T00:00:00Z"}"#),
        ];
        let ordered = order(records);
        let uuids: Vec<&str> = ordered.iter().filter_map(|r| r.uuid.as_deref()).collect();
        assert_eq!(uuids, vec!["early", "late"]);
    }

    #[test]
    fn orphans_with_missing_parents_still_appear() {
        let records = vec![
            rec(
                r#"{"type":"user","uuid":"x","parentUuid":"missing","timestamp":"2026-01-01T00:00:02Z"}"#,
            ),
            rec(r#"{"type":"user","uuid":"y","timestamp":"2026-01-01T00:00:01Z"}"#),
        ];
        let ordered = order(records);
        assert_eq!(ordered.len(), 2);
    }
}

mod extract {
    use gitreceipts::extract::{Radius, command_radius, git_subcommands};

    #[test]
    fn git_subcommands_sees_through_global_flags() {
        assert_eq!(git_subcommands("git -C /x commit -m hi"), vec!["commit"]);
        assert_eq!(git_subcommands("git -c a=b push origin main"), vec!["push"]);
        assert_eq!(git_subcommands("ls -la && echo git"), Vec::<String>::new());
    }

    #[test]
    fn git_subcommands_counts_every_invocation() {
        let script = "git add -A\ngit commit -q -F - <<'MSG'\nfirst\nMSG\ngit commit -q -aF - <<'MSG'\nsecond\nMSG";
        let subs = git_subcommands(script);
        assert_eq!(subs.iter().filter(|s| *s == "commit").count(), 2);
    }

    #[test]
    fn radius_orders_by_reach() {
        assert_eq!(command_radius("git status"), None);
        assert_eq!(
            command_radius("git add -A && git commit -m x"),
            Some(Radius::LocalGit)
        );
        assert_eq!(
            command_radius("git commit -m x && git push"),
            Some(Radius::RemoteGit)
        );
        assert_eq!(command_radius("mkdir -p build"), Some(Radius::LocalFs));
        assert_eq!(
            command_radius("curl -s https://example.com"),
            Some(Radius::Network)
        );
    }
}

mod gitio {
    use gitreceipts::gitio::parse_name_status;

    #[test]
    fn parses_modifications_additions_renames() {
        let raw = "M\tSources/App.swift\nA\tdocs/notes.md\nR100\told.rs\tnew.rs\nD\tgone.txt\n";
        let changes = parse_name_status(raw);
        let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["Sources/App.swift", "docs/notes.md", "new.rs", "gone.txt"]
        );
        assert_eq!(changes[2].status, 'R');
    }

    #[test]
    fn skips_noise_lines() {
        assert!(parse_name_status("\ncommit abc\n\n").is_empty());
    }
}

mod reconcile {
    use gitreceipts::reconcile::longest_prefix;

    #[test]
    fn longest_prefix_prefers_deepest_root() {
        let roots = vec!["/a".to_string(), "/a/b".to_string()];
        let (root, rel) = longest_prefix("/a/b/c.txt", &roots).unwrap();
        assert_eq!(root, "/a/b");
        assert_eq!(rel, "c.txt");
    }

    #[test]
    fn longest_prefix_rejects_partial_components() {
        let roots = vec!["/a/repo".to_string()];
        // "/a/repository/f" must not match root "/a/repo"
        assert!(longest_prefix("/a/repository/f", &roots).is_none());
    }
}
