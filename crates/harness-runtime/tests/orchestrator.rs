use harness_core::{
    domain::{Acceptance, Project, ProjectId, TaskId},
    plan::PlanProposal,
};
use harness_runtime::orchestrator::{attention_tasks, validate};

fn runner(name: &str) -> String {
    if cfg!(windows) {
        format!("C:\\Tools\\{name}.exe")
    } else {
        format!("/usr/bin/{name}")
    }
}

fn fixture() -> (PlanProposal, Project) {
    let plan: PlanProposal = serde_json::from_str(include_str!(
        "../../../tests/fixtures/plans/valid_plan.json"
    ))
    .unwrap();
    let project = Project::new(ProjectId::new("proj-demo").unwrap(), "Demo", "/repo").unwrap();
    (plan, project)
}

fn command(plan: &mut PlanProposal, argv: Vec<String>) {
    for task in &mut plan.tasks {
        task.acceptance = vec![Acceptance::Test {
            command: argv.clone(),
        }];
    }
}

#[test]
fn git_push_rejected_as_test() {
    let (mut plan, project) = fixture();
    command(&mut plan, vec![runner("git"), "push".into()]);
    assert!(validate(&plan, &project, &[runner("git")]).is_err());
}

#[test]
fn npm_publish_rejected() {
    let (mut plan, project) = fixture();
    command(&mut plan, vec![runner("npm"), "publish".into()]);
    assert!(validate(&plan, &project, &[runner("npm")]).is_err());
}

#[test]
fn pytest_allowed() {
    let (mut plan, project) = fixture();
    command(&mut plan, vec![runner("pytest"), "-q".into()]);
    validate(&plan, &project, &[runner("pytest")]).unwrap();
}

#[test]
fn relative_runner_rejected() {
    let (mut plan, project) = fixture();
    command(&mut plan, vec!["pytest".into(), "-q".into()]);
    assert!(validate(&plan, &project, &["pytest".into()]).is_err());
}

#[test]
fn cyclic_plan_rejected() {
    let (mut plan, project) = fixture();
    plan.tasks[0].dependencies = vec![TaskId::new("task-endpoint").unwrap()];
    assert!(validate(&plan, &project, &[runner("python")]).is_err());
}

#[test]
fn absolute_or_parent_path_in_files_rejected() {
    for path in ["/tmp/outside", "../outside", "C:\\outside"] {
        let (mut plan, project) = fixture();
        plan.tasks[0].candidate_paths = vec![path.into()];
        assert!(
            validate(&plan, &project, &[runner("python")]).is_err(),
            "{path}"
        );
    }
}

#[test]
fn package_scripts_flagged_for_review() {
    let (mut plan, project) = fixture();
    command(&mut plan, vec![runner("npm"), "test".into()]);
    validate(&plan, &project, &[runner("npm")]).unwrap();
    assert_eq!(attention_tasks(&plan), vec!["task-model", "task-endpoint"]);
    command(&mut plan, vec![runner("make")]);
    assert!(validate(&plan, &project, &[runner("make")]).is_err());
    for argv in [
        vec![runner("npm"), "exec".into(), "sh".into()],
        vec![runner("npm"), "run".into()],
        vec![runner("yarn"), "dlx".into(), "sh".into()],
    ] {
        command(&mut plan, argv);
        assert!(validate(&plan, &project, &[runner("npm"), runner("yarn")]).is_err());
    }
}
