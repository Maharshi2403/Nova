use harness_runtime::broker::command::{CommandPolicy, Decision};
#[test]
fn broad_prefix_does_not_approve_network_destructive_push_or_deploy() {
    for argv in [
        vec!["git", "push"],
        vec!["git", "reset", "--hard"],
        vec!["pip", "install", "package"],
        vec!["kubectl", "apply", "-f", "x"],
    ] {
        let argv: Vec<String> = argv.into_iter().map(str::to_owned).collect();
        let mut policy = CommandPolicy::new(vec![vec![argv[0].clone()]]);
        assert_eq!(policy.decide(&argv), Decision::ApprovalRequired);
        policy.approve(argv.clone());
        assert_eq!(policy.decide(&argv), Decision::Allowed);
    }
}
