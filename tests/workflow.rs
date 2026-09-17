use std::{fs, path::Path};
use tandem::{
    controller::Controller,
    protocol::*,
    workspace::{self, git},
};
use tempfile::TempDir;
fn fixture() -> (TempDir, Controller) {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q"]).unwrap();
    fs::write(
        root.join("app.txt"),
        "ttl = 10\n\nalpha\nbeta\ngamma\ndelta\n\nfooter\n",
    )
    .unwrap();
    fs::write(root.join("other.txt"), "original\n").unwrap();
    git(&root, &["add", "."]).unwrap();
    git(
        &root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ],
    )
    .unwrap();
    let c = Controller::create(&root, &t.path().join("session")).unwrap();
    (t, c)
}
fn tour(file: &str) -> TourDraft {
    TourDraft {
        title: "Proposal".into(),
        overview: "Explain a complete change".into(),
        stops: vec![
            TourStop {
                title: "First".into(),
                body: "A concept".into(),
                file: file.into(),
                line: 1,
            },
            TourStop {
                title: "Return".into(),
                body: "The next concept".into(),
                file: file.into(),
                line: 1,
            },
        ],
    }
}
fn build(c: &mut Controller, file: &str, content: &str) {
    c.plan("Change the implementation".into()).unwrap();
    c.begin().unwrap();
    fs::write(c.workspace.shadow.join(file), content).unwrap();
    c.complete(tour(file)).unwrap();
}
fn read(root: &Path, file: &str) -> String {
    fs::read_to_string(root.join(file)).unwrap()
}
#[test]
fn stages_versions_tour_and_uncommitted_apply() {
    let (_t, mut c) = fixture();
    let head = git(&c.workspace.real, &["rev-parse", "HEAD"]).unwrap();
    assert!(c.begin().is_err());
    assert!(c.apply().is_err());
    build(&mut c, "other.txt", "proposal one\n");
    assert_eq!(read(&c.workspace.real, "other.txt"), "original\n");
    assert_eq!(c.session.stage, Stage::Tour);
    c.tour_move(1).unwrap();
    c.tour_move(1).unwrap();
    assert_eq!(c.tour().unwrap().current_stop, 2);
    c.tour_move(-1).unwrap();
    assert_eq!(c.tour().unwrap().current_stop, 1);
    c.apply().unwrap();
    assert_eq!(read(&c.workspace.real, "other.txt"), "proposal one\n");
    build(&mut c, "other.txt", "proposal two\n");
    c.apply().unwrap();
    c.switch(1).unwrap();
    assert_eq!(read(&c.workspace.real, "other.txt"), "proposal two\n");
    c.apply().unwrap();
    assert_eq!(read(&c.workspace.real, "other.txt"), "proposal one\n");
    assert_eq!(
        git(&c.workspace.real, &["rev-parse", "HEAD"]).unwrap(),
        head
    );
    assert!(
        git(&c.workspace.real, &["diff", "--cached"])
            .unwrap()
            .is_empty()
    );
}
#[test]
fn tour_refinement_and_navigation_preserve_source_and_reject_stale_positions() {
    let (_t, mut c) = fixture();
    c.present_tour(tour("app.txt")).unwrap();
    let first = c.tour().unwrap().id;
    c.tour_jump(first, 2).unwrap();
    c.tour_jump(first, 0).unwrap();
    assert_eq!(c.tour().unwrap().current_stop, 0);
    assert!(c.tour_jump(first, usize::MAX).is_err());
    c.present_tour(tour("other.txt")).unwrap();
    assert!(c.tour_jump(first, 1).is_err());
    assert_eq!(c.session.stage, Stage::Discuss);
    c.close_tour().unwrap();
    assert!(c.tour_jump(first, 1).is_err());

    build(&mut c, "new.txt", "only in the proposal\n");
    c.present_tour(tour("new.txt")).unwrap();
    assert_eq!(c.tour().unwrap().source, TourSource::Proposal(1));
    assert_eq!(c.session.proposals.len(), 1);
    assert_eq!(c.session.stage, Stage::Tour);
    assert!(!c.workspace.real.join("new.txt").exists());
    let context: serde_json::Value = serde_json::from_str(&c.context().unwrap()).unwrap();
    assert_eq!(context["active_tour"]["id"], c.tour().unwrap().id);
    c.apply().unwrap();
    assert_eq!(read(&c.workspace.real, "new.txt"), "only in the proposal\n");
}
#[test]
fn dirty_staged_and_untracked_baseline_survives() {
    let (t, c) = fixture();
    let root = c.workspace.real.clone();
    drop(c);
    fs::write(root.join("app.txt"), "staged user edit\n").unwrap();
    git(&root, &["add", "app.txt"]).unwrap();
    fs::write(root.join("app.txt"), "unstaged user edit\n").unwrap();
    fs::write(root.join("new.txt"), "untracked user work\n").unwrap();
    let index = git(&root, &["diff", "--cached"]).unwrap();
    let mut c = Controller::create(&root, &t.path().join("dirty-session")).unwrap();
    assert_eq!(read(&c.workspace.shadow, "app.txt"), "unstaged user edit\n");
    assert_eq!(
        read(&c.workspace.shadow, "new.txt"),
        "untracked user work\n"
    );
    build(&mut c, "other.txt", "agent change\n");
    c.apply().unwrap();
    assert_eq!(read(&root, "app.txt"), "unstaged user edit\n");
    assert_eq!(index, git(&root, &["diff", "--cached"]).unwrap());
    assert_eq!(read(&root, "new.txt"), "untracked user work\n");
}
#[test]
fn proposal_on_dirty_file_reverts_to_dirty_baseline() {
    let (_t, mut c) = fixture();
    fs::write(c.workspace.real.join("other.txt"), "human baseline\n").unwrap();
    build(&mut c, "other.txt", "human baseline\nagent addition\n");
    c.apply().unwrap();
    c.revert(0).unwrap();
    assert_eq!(read(&c.workspace.real, "other.txt"), "human baseline\n");
}
#[test]
fn manual_edits_survive_revision_and_overlap_is_conflict() {
    let (_t, mut c) = fixture();
    let original = read(&c.workspace.real, "app.txt");
    build(&mut c, "app.txt", &original.replace("10", "5"));
    c.apply().unwrap();
    fs::write(
        c.workspace.real.join("app.txt"),
        original.replace("10", "2"),
    )
    .unwrap();
    c.plan("Change another file".into()).unwrap();
    c.begin().unwrap();
    assert!(read(&c.workspace.shadow, "app.txt").contains("ttl = 2"));
    fs::write(c.workspace.shadow.join("other.txt"), "revision\n").unwrap();
    c.complete(tour("other.txt")).unwrap();
    c.apply().unwrap();
    assert!(read(&c.workspace.real, "app.txt").contains("ttl = 2"));
    c.plan("Try an overlapping change".into()).unwrap();
    c.begin().unwrap();
    fs::write(
        c.workspace.shadow.join("app.txt"),
        original.replace("10", "7"),
    )
    .unwrap();
    assert!(c.complete(tour("app.txt")).is_err());
    assert!(read(&c.workspace.real, "app.txt").contains("ttl = 2"));
}
#[test]
fn revert_preserves_unrelated_manual_edit_but_refuses_overlap() {
    let (_t, mut c) = fixture();
    let original = read(&c.workspace.real, "app.txt");
    build(&mut c, "app.txt", &original.replace("10", "5"));
    c.apply().unwrap();
    fs::write(
        c.workspace.real.join("app.txt"),
        read(&c.workspace.real, "app.txt").replace("footer", "human footer"),
    )
    .unwrap();
    c.revert(0).unwrap();
    assert_eq!(
        read(&c.workspace.real, "app.txt"),
        original.replace("footer", "human footer")
    );
    let (_t, mut c) = fixture();
    build(&mut c, "app.txt", &original.replace("10", "5"));
    c.apply().unwrap();
    fs::write(
        c.workspace.real.join("app.txt"),
        original.replace("10", "2"),
    )
    .unwrap();
    assert!(c.revert(0).is_err());
    assert!(read(&c.workspace.real, "app.txt").contains("ttl = 2"));
}
#[test]
fn individual_hunks_can_be_reverted_in_any_order() {
    let (_t, mut c) = fixture();
    let original = read(&c.workspace.real, "app.txt");
    build(
        &mut c,
        "app.txt",
        &original
            .replace("10", "5")
            .replace("footer", "agent footer"),
    );
    c.apply().unwrap();
    assert_eq!(c.changes().unwrap().len(), 2);
    c.revert(1).unwrap();
    assert_eq!(
        read(&c.workspace.real, "app.txt"),
        original.replace("10", "5")
    );
    c.revert(0).unwrap();
    assert_eq!(read(&c.workspace.real, "app.txt"), original);
}
#[test]
fn multi_file_conflict_has_no_partial_application() {
    let (_t, mut c) = fixture();
    c.plan("two files".into()).unwrap();
    c.begin().unwrap();
    fs::write(c.workspace.shadow.join("app.txt"), "agent\n").unwrap();
    fs::write(c.workspace.shadow.join("other.txt"), "agent\n").unwrap();
    c.complete(tour("app.txt")).unwrap();
    fs::write(c.workspace.real.join("other.txt"), "human\n").unwrap();
    assert!(c.apply().is_err());
    assert!(read(&c.workspace.real, "app.txt").contains("ttl = 10"));
    assert_eq!(read(&c.workspace.real, "other.txt"), "human\n");
}
#[test]
fn untracked_collision_and_symlink_are_not_overwritten() {
    let (_t, mut c) = fixture();
    build(&mut c, "new.txt", "agent\n");
    fs::write(c.workspace.real.join("new.txt"), "human\n").unwrap();
    assert!(c.apply().is_err());
    assert_eq!(read(&c.workspace.real, "new.txt"), "human\n");
    std::os::unix::fs::symlink("/tmp", c.workspace.real.join("link")).unwrap();
    assert!(workspace::capture(&c.workspace.real).is_err());
}
#[test]
fn unsaved_editor_buffers_gate_writes_and_context_is_serializable() {
    let (_t, mut c) = fixture();
    c.plan("work".into()).unwrap();
    c.editor.dirty.push("app.txt".into());
    assert!(c.begin().is_err());
    c.editor.selection = "a\n\"b".into();
    let value: serde_json::Value = serde_json::from_str(&c.context().unwrap()).unwrap();
    assert_eq!(value["editor"]["selection"], "a\n\"b");
    let req = Request {
        version: VERSION,
        id: 1,
        action: Action::Context {
            context: c.editor.clone(),
        },
    };
    let decoded: Request = serde_json::from_str(&serde_json::to_string(&req).unwrap()).unwrap();
    assert_eq!(decoded.version, VERSION);
}

#[test]
fn repository_tour_is_independent_of_proposals_and_discussion_stage() {
    let (_t, mut c) = fixture();
    let before = workspace::capture(&c.workspace.real).unwrap();
    c.repository_tour(tour("app.txt")).unwrap();
    assert_eq!(c.session.stage, Stage::Discuss);
    assert!(c.proposal().is_none());
    assert_eq!(c.tour().unwrap().source, TourSource::Repository);
    c.tour_move(2).unwrap();
    assert!(
        c.navigation
            .as_ref()
            .unwrap()
            .file
            .starts_with(c.workspace.real.to_str().unwrap())
    );
    let ctx: serde_json::Value = serde_json::from_str(&c.context().unwrap()).unwrap();
    assert_eq!(ctx["active_tour"]["current_stop"], 2);
    assert_eq!(ctx["tour_stop"]["title"], "Return");
    c.close_tour().unwrap();
    assert!(c.tour().is_none());
    assert_eq!(c.session.stage, Stage::Discuss);
    assert_eq!(before, workspace::capture(&c.workspace.real).unwrap());
}

#[test]
fn new_ignore_rules_do_not_delete_baseline_untracked_work() {
    let (t, c) = fixture();
    let root = c.workspace.real.clone();
    drop(c);
    fs::write(root.join("notes.txt"), "human notes\n").unwrap();
    let mut c = Controller::create(&root, &t.path().join("ignored-session")).unwrap();
    build(&mut c, ".gitignore", "notes.txt\n");
    c.apply().unwrap();
    assert_eq!(read(&root, "notes.txt"), "human notes\n");
    c.plan("another change".into()).unwrap();
    c.begin().unwrap();
    assert_eq!(read(&c.workspace.shadow, "notes.txt"), "human notes\n");
    fs::write(c.workspace.shadow.join("other.txt"), "another proposal\n").unwrap();
    c.complete(tour("other.txt")).unwrap();
    c.apply().unwrap();
    assert_eq!(read(&root, "notes.txt"), "human notes\n");
}
#[test]
fn history_switch_preserves_manual_edits_or_reports_conflicts() {
    let (_t, mut c) = fixture();
    let original = read(&c.workspace.real, "app.txt");
    build(&mut c, "app.txt", &original.replace("10", "5"));
    c.apply().unwrap();
    fs::write(
        c.workspace.real.join("app.txt"),
        original.replace("10", "2"),
    )
    .unwrap();
    build(&mut c, "other.txt", "revision\n");
    c.apply().unwrap();
    c.switch(1).unwrap();
    c.apply().unwrap();
    assert!(read(&c.workspace.real, "app.txt").contains("ttl = 2"));
    assert_eq!(read(&c.workspace.real, "other.txt"), "original\n");
}
#[test]
fn binary_mode_and_deleted_file_reverts_preserve_git_index() {
    use std::os::unix::fs::PermissionsExt;
    let (_t, mut c) = fixture();
    c.plan("binary and executable changes".into()).unwrap();
    c.begin().unwrap();
    fs::write(c.workspace.shadow.join("binary.bin"), [0, 1, 2, 3]).unwrap();
    fs::set_permissions(
        c.workspace.shadow.join("app.txt"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::remove_file(c.workspace.shadow.join("other.txt")).unwrap();
    c.complete(tour("app.txt")).unwrap();
    c.apply().unwrap();
    assert_eq!(c.changes().unwrap().len(), 3);
    for id in (0..3).rev() {
        c.revert(id).unwrap();
    }
    assert!(!c.workspace.real.join("binary.bin").exists());
    assert_eq!(read(&c.workspace.real, "other.txt"), "original\n");
    assert_eq!(
        fs::metadata(c.workspace.real.join("app.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert!(
        git(&c.workspace.real, &["diff", "--cached"])
            .unwrap()
            .is_empty()
    );
}
#[test]
fn monorepo_subdirectory_resolves_root_and_reuses_worktree() {
    let (_t, mut c) = fixture();
    let shadow = c.workspace.shadow.clone();
    fs::create_dir_all(c.workspace.real.join("packages/service")).unwrap();
    fs::write(
        c.workspace.real.join("packages/service/file.txt"),
        "workspace\n",
    )
    .unwrap();
    build(&mut c, "other.txt", "first\n");
    c.apply().unwrap();
    build(&mut c, "other.txt", "second\n");
    c.apply().unwrap();
    assert_eq!(shadow, c.workspace.shadow);
    assert_eq!(read(&shadow, "packages/service/file.txt"), "workspace\n");
    let session = tempfile::tempdir().unwrap();
    let nested = Controller::create(
        &c.workspace.real.join("packages/service"),
        &session.path().join("nested"),
    )
    .unwrap();
    assert_eq!(nested.workspace.real, c.workspace.real);
}

#[test]
fn applying_content_preserves_private_file_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let (_t, mut c) = fixture();
    fs::set_permissions(
        c.workspace.real.join("other.txt"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    build(&mut c, "other.txt", "new contents\n");
    c.apply().unwrap();
    assert_eq!(
        fs::metadata(c.workspace.real.join("other.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn session_creation_refuses_project_storage_and_existing_session_data() {
    let (t, c) = fixture();
    let before = workspace::capture(&c.workspace.real).unwrap();
    assert!(Controller::create(&c.workspace.real, &c.workspace.real.join("runtime")).is_err());
    assert!(!c.workspace.real.join("runtime").exists());
    let link = t.path().join("project-link");
    std::os::unix::fs::symlink(&c.workspace.real, &link).unwrap();
    assert!(Controller::create(&c.workspace.real, &link.join("runtime")).is_err());
    assert!(!c.workspace.real.join("runtime").exists());
    let existing = t.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep"), "user data").unwrap();
    assert!(Controller::create(&c.workspace.real, &existing).is_err());
    assert_eq!(read(&existing, "keep"), "user data");
    assert_eq!(before, workspace::capture(&c.workspace.real).unwrap());
}

#[test]
fn later_human_choices_supersede_earlier_edits_across_revisions() {
    let (_t, mut c) = fixture();
    let original = read(&c.workspace.real, "app.txt");
    build(&mut c, "app.txt", &original.replace("10", "5"));
    c.apply().unwrap();
    fs::write(
        c.workspace.real.join("app.txt"),
        original.replace("10", "2"),
    )
    .unwrap();
    build(&mut c, "other.txt", "second proposal\n");
    c.apply().unwrap();
    fs::write(
        c.workspace.real.join("app.txt"),
        original.replace("10", "3"),
    )
    .unwrap();
    build(&mut c, "other.txt", "third proposal\n");
    c.apply().unwrap();
    assert!(read(&c.workspace.real, "app.txt").contains("ttl = 3"));
    c.switch(1).unwrap();
    c.apply().unwrap();
    assert!(read(&c.workspace.real, "app.txt").contains("ttl = 3"));
}

#[test]
fn revising_an_unapplied_proposal_keeps_its_implementation_in_shadow() {
    let (_t, mut c) = fixture();
    build(&mut c, "other.txt", "first implementation\n");
    c.plan("extend the proposal before applying it".into())
        .unwrap();
    c.begin().unwrap();
    assert_eq!(
        read(&c.workspace.shadow, "other.txt"),
        "first implementation\n"
    );
    fs::write(c.workspace.shadow.join("app.txt"), "extension\n").unwrap();
    c.complete(tour("app.txt")).unwrap();
    assert_eq!(read(&c.workspace.real, "other.txt"), "original\n");
    c.apply().unwrap();
    assert_eq!(
        read(&c.workspace.real, "other.txt"),
        "first implementation\n"
    );
    assert_eq!(read(&c.workspace.real, "app.txt"), "extension\n");
}
