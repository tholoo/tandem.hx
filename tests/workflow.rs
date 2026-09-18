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
                end_line: 1,
            },
            TourStop {
                title: "Return".into(),
                body: "The next concept".into(),
                file: file.into(),
                line: 1,
                end_line: 1,
            },
        ],
    }
}

#[test]
fn build_clarification_restores_the_repository_tour_without_a_proposal() {
    let (_root, mut c) = fixture();
    c.present_tour(tour("app.txt")).unwrap();
    c.tour_move(1).unwrap();
    let before = c.tour().unwrap().clone();
    let files = c.current().unwrap();
    c.begin().unwrap();
    c.finish_build(None).unwrap();
    assert!(!c.busy);
    assert_eq!(c.session.stage, Stage::Discuss);
    assert_eq!(c.tour().unwrap().id, before.id);
    assert_eq!(c.tour().unwrap().current_stop, before.current_stop);
    assert!(c.proposal().is_none());
    assert_eq!(c.current().unwrap(), files);
}

#[test]
fn changed_build_still_requires_a_proposal_tour() {
    let (_root, mut c) = fixture();
    let before = c.current().unwrap();
    c.begin().unwrap();
    fs::write(c.workspace.shadow.join("app.txt"), "changed\n").unwrap();
    assert!(
        c.finish_build(None)
            .unwrap_err()
            .to_string()
            .contains("without providing a proposal tour")
    );
    c.failed().unwrap();
    assert_eq!(c.current().unwrap(), before);
    assert_eq!(
        fs::read(c.workspace.shadow.join("app.txt")).unwrap(),
        fs::read(c.workspace.real.join("app.txt")).unwrap()
    );
    assert!(c.proposal().is_none());
}
fn build(c: &mut Controller, file: &str, content: &str) {
    c.begin().unwrap();
    fs::write(c.workspace.shadow.join(file), content).unwrap();
    c.complete(tour(file)).unwrap();
}

#[test]
fn tour_reading_ranges_are_explicit_inclusive_and_validated() {
    let (_temp, mut c) = fixture();
    let mut draft = tour("app.txt");
    draft.stops[0].line = 1;
    draft.stops[0].end_line = 2;
    c.repository_tour(draft.clone()).unwrap();
    assert_eq!(c.view().unwrap().tour.unwrap().content.stops[0].end_line, 2);
    draft.stops[0].line = 2;
    draft.stops[0].end_line = 1;
    assert!(c.repository_tour(draft.clone()).is_err());
    draft.stops[0].end_line = usize::MAX;
    assert!(c.repository_tour(draft).is_err());
    assert!(
        serde_json::from_value::<TourStop>(serde_json::json!({
            "title": "Missing boundary", "body": "Read this", "file": "app.txt", "line": 1
        }))
        .is_err()
    );
}
fn read(root: &Path, file: &str) -> String {
    fs::read_to_string(root.join(file)).unwrap()
}
#[test]
fn stages_versions_tour_and_uncommitted_apply() {
    let (_t, mut c) = fixture();
    let head = git(&c.workspace.real, &["rev-parse", "HEAD"]).unwrap();
    c.begin().unwrap();
    assert!(c.busy);
    c.failed().unwrap();
    assert_eq!(c.session.stage, Stage::Discuss);
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
    c.begin().unwrap();
    assert!(read(&c.workspace.shadow, "app.txt").contains("ttl = 2"));
    fs::write(c.workspace.shadow.join("other.txt"), "revision\n").unwrap();
    c.complete(tour("other.txt")).unwrap();
    c.apply().unwrap();
    assert!(read(&c.workspace.real, "app.txt").contains("ttl = 2"));
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

#[test]
fn saved_preview_edits_are_applied_and_included_in_diff() {
    let (_t, mut c) = fixture();
    build(&mut c, "other.txt", "agent draft\n");
    fs::write(c.workspace.shadow.join("other.txt"), "human adjustment\n").unwrap();
    fs::write(c.workspace.shadow.join("extra.txt"), "human file\n").unwrap();
    assert!(c.diff().unwrap().contains("human adjustment"));
    assert_eq!(read(&c.workspace.real, "other.txt"), "original\n");
    c.apply().unwrap();
    assert_eq!(read(&c.workspace.real, "other.txt"), "human adjustment\n");
    assert_eq!(read(&c.workspace.real, "extra.txt"), "human file\n");
    assert_eq!(c.changes().unwrap().len(), 2);
}
#[test]
fn proposal_questions_keep_the_tour_and_revisions_start_with_saved_edits() {
    let (_t, mut c) = fixture();
    build(&mut c, "other.txt", "agent draft\n");
    c.tour_move(2).unwrap();
    let tour_id = c.tour().unwrap().id;
    fs::write(c.workspace.shadow.join("other.txt"), "human adjustment\n").unwrap();
    c.refine().unwrap();
    c.finish_refinement(None).unwrap();
    assert_eq!(c.session.proposals.len(), 1);
    assert_eq!(c.tour().unwrap().id, tour_id);
    assert_eq!(c.tour().unwrap().current_stop, 2);
    c.refine().unwrap();
    assert_eq!(read(&c.workspace.shadow, "other.txt"), "human adjustment\n");
    fs::write(c.workspace.shadow.join("new.txt"), "agent revision\n").unwrap();
    c.finish_refinement(Some(tour("new.txt"))).unwrap();
    assert_eq!(c.session.selected, Some(2));
    assert_eq!(read(&c.workspace.real, "other.txt"), "original\n");
    assert!(!c.workspace.real.join("new.txt").exists());
    c.apply().unwrap();
    assert_eq!(read(&c.workspace.real, "other.txt"), "human adjustment\n");
    assert_eq!(read(&c.workspace.real, "new.txt"), "agent revision\n");
}
#[test]
fn failed_refinements_restore_saved_preview_and_tour() {
    let (_t, mut c) = fixture();
    build(&mut c, "other.txt", "agent draft\n");
    fs::write(c.workspace.shadow.join("other.txt"), "saved human edit\n").unwrap();
    c.tour_move(1).unwrap();
    let tour_id = c.tour().unwrap().id;
    c.refine().unwrap();
    fs::write(c.workspace.shadow.join("other.txt"), "partial work\n").unwrap();
    fs::write(c.workspace.shadow.join("partial.txt"), "incomplete\n").unwrap();
    assert!(c.finish_refinement(None).is_err());
    c.failed().unwrap();
    assert_eq!(c.session.stage, Stage::Tour);
    assert_eq!(c.session.selected, Some(1));
    assert_eq!(c.tour().unwrap().id, tour_id);
    assert_eq!(c.tour().unwrap().current_stop, 1);
    assert_eq!(read(&c.workspace.shadow, "other.txt"), "saved human edit\n");
    assert!(!c.workspace.shadow.join("partial.txt").exists());
    c.apply().unwrap();
    assert_eq!(read(&c.workspace.real, "other.txt"), "saved human edit\n");
}
#[test]
fn switching_proposals_keeps_each_saved_draft() {
    let (_t, mut c) = fixture();
    build(&mut c, "other.txt", "first\n");
    fs::write(
        c.workspace.shadow.join("other.txt"),
        "first with human edit\n",
    )
    .unwrap();
    c.refine().unwrap();
    fs::write(c.workspace.shadow.join("other.txt"), "second\n").unwrap();
    c.finish_refinement(Some(tour("other.txt"))).unwrap();
    fs::write(
        c.workspace.shadow.join("other.txt"),
        "second with human edit\n",
    )
    .unwrap();
    c.switch(1).unwrap();
    assert_eq!(
        read(&c.workspace.shadow, "other.txt"),
        "first with human edit\n"
    );
    c.switch(2).unwrap();
    assert_eq!(
        read(&c.workspace.shadow, "other.txt"),
        "second with human edit\n"
    );
    c.apply().unwrap();
    assert_eq!(
        read(&c.workspace.real, "other.txt"),
        "second with human edit\n"
    );
}

#[test]
fn peek_uses_the_saved_draft_and_current_hunk_without_writing_real_files() {
    let (_t, mut c) = fixture();
    let original =
        read(&c.workspace.real, "app.txt").replace("footer", "one\ntwo\nthree\nfour\nfive\nfooter");
    fs::write(c.workspace.real.join("app.txt"), &original).unwrap();
    build(
        &mut c,
        "app.txt",
        &original
            .replace("10", "5")
            .replace("footer", "agent footer"),
    );
    fs::write(
        c.workspace.shadow.join("app.txt"),
        original
            .replace("10", "2")
            .replace("footer", "human footer"),
    )
    .unwrap();
    c.editor.file = Some(c.workspace.shadow.join("app.txt").display().to_string());
    c.editor.line = 1;
    let peek = c.peek().unwrap();
    assert_eq!(peek.old.as_deref(), Some(original.as_str()));
    assert!(peek.current.as_ref().unwrap().contains("ttl = 2"));
    assert_eq!((peek.old_line, peek.current_line), (1, 1));
    c.editor.line = 13;
    assert!(c.peek().unwrap().current_line > 1);
    c.editor.dirty.push(c.editor.file.clone().unwrap());
    assert!(
        c.peek()
            .unwrap_err()
            .to_string()
            .contains("save this buffer")
    );
    c.editor.dirty.clear();
    assert_eq!(read(&c.workspace.real, "app.txt"), original);
    c.editor.file = Some("/tmp/outside.txt".into());
    assert!(c.peek().is_err());
}
#[test]
fn peek_handles_deletions_and_unchanged_locations() {
    let (_t, mut c) = fixture();
    let original = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n";
    fs::write(c.workspace.real.join("app.txt"), original).unwrap();
    build(&mut c, "app.txt", &original.replace("a\n", ""));
    c.editor.file = Some(c.workspace.shadow.join("app.txt").display().to_string());
    c.editor.line = 1;
    let comparison = c.peek().unwrap();
    assert_eq!(comparison.old.as_deref(), Some(original));
    assert_eq!(
        comparison.current.as_deref(),
        Some(original.trim_start_matches("a\n"))
    );
    c.editor.line = 10;
    assert!(c.peek().unwrap_err().to_string().contains("no text change"));
}

#[test]
fn comparison_handles_added_deleted_binary_and_applied_files() {
    let (_t, mut c) = fixture();
    build(&mut c, "new.txt", "new content\n");
    c.editor.file = Some(c.workspace.shadow.join("new.txt").display().to_string());
    c.editor.line = 1;
    assert!(c.peek().unwrap().old.is_none());
    c.apply().unwrap();
    c.editor.file = Some(c.workspace.real.join("new.txt").display().to_string());
    fs::write(c.workspace.real.join("new.txt"), "human edit\n").unwrap();
    assert_eq!(c.peek().unwrap().current.as_deref(), Some("human edit\n"));
    let (_t, mut c) = fixture();
    build(&mut c, "app.txt", "replacement\n");
    c.editor.file = Some(c.workspace.shadow.join("app.txt").display().to_string());
    c.editor.line = 1;
    fs::remove_file(c.workspace.shadow.join("app.txt")).unwrap();
    assert!(c.peek().unwrap().current.is_none());
    fs::write(c.workspace.shadow.join("app.txt"), b"binary\0content").unwrap();
    assert!(c.peek().unwrap_err().to_string().contains("binary"));
}
