use crate::{
    protocol::*,
    workspace::{self, Snapshot, Workspace},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
struct HumanEdit {
    before: String,
    after: String,
}
#[derive(Serialize, Deserialize)]
pub struct Session {
    pub stage: Stage,
    pub proposals: Vec<Proposal>,
    pub selected: Option<usize>,
    pub baseline: String,
    pub applied: String,
    pub pending_base: Option<String>,
    pub pending_proposal: bool,
    pub drafts: std::collections::BTreeMap<usize, String>,
    pub tours: Vec<Tour>,
    pub active_tour: Option<usize>,
    pub change_index: usize,
    human: Vec<HumanEdit>,
}
struct ChangeTarget {
    file: String,
    line: usize,
    replacement: Option<workspace::File>,
    label: String,
}
struct ProposalTurn {
    tree: String,
    stage: Stage,
    tour: Option<usize>,
    refinement: bool,
}
pub struct Controller {
    pub workspace: Workspace,
    pub session: Session,
    pub editor: EditorContext,
    pub editor_connected: Option<bool>,
    pub busy: bool,
    pub generation: u64,
    pub navigation: Option<Location>,
    turn: Option<ProposalTurn>,
    path: PathBuf,
    change_cache: std::cell::RefCell<Option<(usize, Vec<Change>)>>,
}
impl Controller {
    pub fn create(project: &Path, directory: &Path) -> Result<Self> {
        let workspace = Workspace::create(project, directory)?;
        let baseline = workspace.store(&workspace::capture(&workspace.real)?)?;
        let c = Self {
            workspace,
            session: Session {
                stage: Stage::Discuss,
                proposals: vec![],
                selected: None,
                baseline: baseline.clone(),
                applied: baseline,
                pending_base: None,
                pending_proposal: false,
                drafts: std::collections::BTreeMap::new(),
                tours: vec![],
                active_tour: None,
                change_index: 0,
                human: vec![],
            },
            editor: EditorContext::default(),
            editor_connected: None,
            busy: false,
            generation: 0,
            navigation: None,
            change_cache: std::cell::RefCell::new(None),
            turn: None,
            path: directory.join("session.json"),
        };
        c.save()?;
        Ok(c)
    }
    pub fn current(&self) -> Result<Snapshot> {
        let mut known = self.workspace.load(&self.session.baseline)?;
        known.extend(self.workspace.load(&self.session.applied)?);
        workspace::capture_known(&self.workspace.real, &known)
    }
    pub fn save(&self) -> Result<()> {
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&self.session)?)?;
        fs::rename(tmp, &self.path)?;
        Ok(())
    }
    pub fn proposal(&self) -> Option<&Proposal> {
        self.session
            .selected
            .and_then(|i| self.session.proposals.iter().find(|p| p.id == i))
    }
    pub fn view(&self) -> Result<View> {
        Ok(View {
            stage: self.session.stage,
            busy: self.busy,
            real: self.workspace.real.display().to_string(),
            shadow: self.workspace.shadow.display().to_string(),
            proposal: self.session.selected,
            proposals: self.session.proposals.iter().map(|p| p.id).collect(),
            tour: self.tour().cloned(),
            changes: self.changes()?,
            change_index: self.session.change_index,
            editor: self.editor.clone(),
            navigation: self.navigation.clone(),
            generation: self.generation,
        })
    }
    fn saved(&self) -> Result<()> {
        ensure!(
            self.editor_connected != Some(false),
            "editor disconnected; run :tandem in Helix to reconnect first"
        );
        ensure!(
            self.editor.dirty.is_empty(),
            "save your Helix buffers first: {}",
            self.editor.dirty.join(", ")
        );
        Ok(())
    }
    fn effective_proposal(&self) -> Option<Proposal> {
        self.proposal().cloned().map(|mut p| {
            if let Some(tree) = self.session.drafts.get(&p.id) {
                p.tree = tree.clone();
            }
            p
        })
    }
    fn preview(&self) -> Result<Snapshot> {
        let proposal = self.effective_proposal().context("missing proposal")?;
        workspace::capture_known(
            &self.workspace.shadow,
            &self.workspace.load(&proposal.tree)?,
        )
    }
    fn remember_preview(&mut self) -> Result<Snapshot> {
        let files = self.preview()?;
        let id = self.session.selected.context("missing proposal")?;
        self.session
            .drafts
            .insert(id, self.workspace.store(&files)?);
        *self.change_cache.borrow_mut() = None;
        Ok(files)
    }
    pub fn diff(&self) -> Result<String> {
        let p = self.effective_proposal().context("no proposal")?;
        let tree = if self.session.pending_proposal && !self.busy {
            self.workspace.store(&self.preview()?)?
        } else {
            p.tree
        };
        self.workspace.diff(&p.base, &tree, 3)
    }
    pub fn refining(&self) -> bool {
        self.turn.as_ref().is_some_and(|t| t.refinement)
    }
    /// Both saved versions, positioned at the changed block under the cursor.
    pub fn peek(&self) -> Result<Comparison> {
        ensure!(!self.busy, "wait for the agent before comparing a change");
        let p = self
            .effective_proposal()
            .context("no proposal to compare")?;
        let path = Path::new(
            self.editor
                .file
                .as_deref()
                .context("no code file selected")?,
        );
        ensure!(
            !self
                .editor
                .dirty
                .iter()
                .any(|dirty| Path::new(dirty) == path),
            "save this buffer before comparing its changes"
        );
        let relative = path
            .strip_prefix(&self.workspace.shadow)
            .or_else(|_| path.strip_prefix(&self.workspace.real))
            .context("selected file is outside this repository")?;
        let file = relative.to_str().context("unsupported filename")?;
        workspace::valid_path(file)?;
        let root = if self.session.pending_proposal {
            &self.workspace.shadow
        } else {
            &self.workspace.real
        };
        ensure!(
            !self
                .editor
                .dirty
                .iter()
                .any(|dirty| Path::new(dirty) == root.join(file)),
            "save this buffer before comparing its changes"
        );
        let files = if self.session.pending_proposal {
            self.preview()?
        } else {
            self.current()?
        };
        let baseline = self.workspace.load(&p.base)?;
        let content = |snapshot: &Snapshot| -> Result<Option<String>> {
            snapshot
                .get(file)
                .map(|f| {
                    ensure!(
                        !f.bytes.contains(&0),
                        "binary files cannot be compared in the editor"
                    );
                    Ok(std::str::from_utf8(&f.bytes)
                        .context("comparison requires UTF-8 text")?
                        .to_owned())
                })
                .transpose()
        };
        let old = content(&baseline)?;
        let current = content(&files)?;
        ensure!(
            old.as_ref().map_or(0, String::len) + current.as_ref().map_or(0, String::len)
                < MAX_FRAME / 8,
            "file is too large for an editor comparison"
        );
        let tree = self.workspace.store(&files)?;
        let diff = String::from_utf8_lossy(&workspace::git(
            &self.workspace.storage,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--unified=3",
                &p.base,
                &tree,
                "--",
                file,
            ],
        )?)
        .into_owned();
        for hunk in diff.lines().filter(|line| line.starts_with("@@ ")) {
            let fields: Vec<_> = hunk.split_whitespace().collect();
            let (os, _) = range(fields[1])?;
            let (rs, rn) = range(fields[2])?;
            let line = self.editor.line.saturating_sub(1);
            if line >= rs && line < rs + rn.max(1) {
                return Ok(Comparison {
                    file: file.into(),
                    current_file: root.join(file).display().to_string(),
                    old,
                    current,
                    old_line: os + 1,
                    current_line: rs + 1,
                });
            }
        }
        bail!("no text change at this location")
    }
    /// A pending proposal is already authorized for iterative shadow edits.
    /// Questions can complete without changes or a replacement tour.
    pub fn refine(&mut self) -> Result<()> {
        ensure!(
            !self.busy && self.session.stage == Stage::Tour,
            "no idle proposal to refine"
        );
        self.saved()?;
        let files = self.remember_preview()?;
        self.turn = Some(ProposalTurn {
            tree: self.workspace.store(&files)?,
            stage: self.session.stage,
            tour: self.session.active_tour,
            refinement: true,
        });
        self.busy = true;
        self.save()
    }
    pub fn finish_refinement(&mut self, tour: Option<TourDraft>) -> Result<()> {
        let turn = self.turn.as_ref().context("no refinement in progress")?;
        ensure!(turn.refinement, "not a refinement");
        let changed = self.preview()? != self.workspace.load(&turn.tree)?;
        if changed {
            self.complete(tour.context("changed proposal needs a tour")?)
        } else {
            if let Some(tour) = tour {
                self.present_tour(tour)?;
            }
            self.turn = None;
            self.busy = false;
            self.save()
        }
    }
    fn observe_human(&mut self, current: &Snapshot) -> Result<()> {
        let tree = self.workspace.store(current)?;
        if !self.session.proposals.is_empty() && tree != self.session.applied {
            self.session.human.push(HumanEdit {
                before: self.session.applied.clone(),
                after: tree,
            });
        }
        Ok(())
    }
    /// Undo recorded human deltas in memory, newest first. This computes their
    /// cumulative effect while allowing a later human choice to supersede an earlier one.
    /// The returned snapshot is only a merge ancestor; it is never written to disk.
    fn without_human(&self, current: &Snapshot) -> Result<Snapshot> {
        let mut ancestor = current.clone();
        for edit in self.session.human.iter().rev() {
            ancestor = workspace::merge(
                &self.workspace.load(&edit.after)?,
                &ancestor,
                &self.workspace.load(&edit.before)?,
            )?;
        }
        Ok(ancestor)
    }
    fn preserve_human(&self, current: &Snapshot, target: &Snapshot) -> Result<Snapshot> {
        workspace::merge(&self.without_human(current)?, target, current)
            .context("proposal conflicts with the current human choices")
    }
    pub fn begin(&mut self) -> Result<()> {
        ensure!(
            !self.busy && self.session.stage != Stage::Building,
            "Begin requires an idle agent"
        );
        self.saved()?;
        let current = self.current()?;
        self.observe_human(&current)?;
        let starting = if self.session.pending_proposal {
            let draft = self.remember_preview()?;
            let pending_base = self.workspace.load(
                self.session
                    .pending_base
                    .as_ref()
                    .context("missing pending baseline")?,
            )?;
            let merged = workspace::merge(&pending_base, &current, &draft)?;
            self.preserve_human(&current, &merged)?
        } else {
            current.clone()
        };
        self.workspace.sync(&starting)?;
        self.turn = Some(ProposalTurn {
            tree: self.workspace.store(&starting)?,
            stage: self.session.stage,
            tour: self.session.active_tour,
            refinement: false,
        });
        self.session.pending_base = Some(self.workspace.store(&current)?);
        self.session.stage = Stage::Building;
        self.session.active_tour = None;
        self.busy = true;
        self.save()
    }
    pub fn finish_build(&mut self, tour: Option<TourDraft>) -> Result<()> {
        ensure!(self.session.stage == Stage::Building, "no build to finish");
        if let Some(tour) = tour {
            self.complete(tour)
        } else {
            let turn = self.turn.as_ref().context("missing build starting point")?;
            let starting = self.workspace.load(&turn.tree)?;
            ensure!(
                workspace::capture_known(&self.workspace.shadow, &starting)? == starting,
                "implementation changed code without providing a proposal tour"
            );
            // A clarification leaves the prior conversation and tour in place.
            self.failed()
        }
    }
    pub fn complete(&mut self, tour: TourDraft) -> Result<()> {
        ensure!(
            self.session.stage == Stage::Building || self.refining(),
            "no build to complete"
        );
        let mut known = self.workspace.load(
            self.session
                .pending_base
                .as_ref()
                .context("missing build baseline")?,
        )?;
        if let Some(turn) = &self.turn {
            known.extend(self.workspace.load(&turn.tree)?);
        }
        let result = workspace::capture_known(&self.workspace.shadow, &known)?;
        let base = self
            .session
            .pending_base
            .clone()
            .context("missing build baseline")?;
        let base_files = self.workspace.load(&base)?;
        // Check whether this revision changes a human's prior choice. The reverse merge is
        // a conflict probe only: its result is discarded, never used to undo human work.
        let ancestor = self.without_human(&base_files)?;
        workspace::merge(&base_files, &result, &ancestor)
            .context("revision overlaps a manual edit; proposal was not applied")?;
        validate_tour(&tour, &result)?;
        let tree = self.workspace.store(&result)?;
        let id = self.session.proposals.len() + 1;
        self.session.proposals.push(Proposal {
            id,
            base: base.clone(),
            tree,
        });
        self.session.selected = Some(id);
        self.session.pending_proposal = true;
        self.add_tour(TourSource::Proposal(id), tour);
        self.session.change_index = 0;
        self.session.stage = Stage::Tour;
        self.busy = false;
        self.turn = None;
        self.navigate_tour()?;
        self.save()
    }
    pub fn failed(&mut self) -> Result<()> {
        self.busy = false;
        if let Some(turn) = self.turn.take() {
            self.workspace.sync(&self.workspace.load(&turn.tree)?)?;
            self.session.stage = turn.stage;
            self.session.active_tour = turn.tour;
            if self.session.pending_proposal {
                self.remember_preview()?;
            }
            self.generation += 1;
        }
        self.save()
    }
    pub fn switch(&mut self, id: usize) -> Result<()> {
        ensure!(
            !self.busy && matches!(self.session.stage, Stage::Applied | Stage::Tour),
            "cannot switch now"
        );
        self.saved()?;
        let p = self
            .session
            .proposals
            .iter()
            .find(|p| p.id == id)
            .context("unknown proposal")?
            .clone();
        if self.session.pending_proposal {
            self.remember_preview()?;
        }
        let current = self.current()?;
        self.observe_human(&current)?;
        let tree = self.session.drafts.get(&id).unwrap_or(&p.tree);
        let target = self.preserve_human(&current, &self.workspace.load(tree)?)?;
        let from = self.workspace.load(&self.session.applied)?;
        let desired = workspace::merge(&from, &current, &target)?;
        self.workspace.sync(&desired)?;
        self.session.pending_base = Some(self.workspace.store(&current)?);
        self.session.selected = Some(id);
        self.session.pending_proposal = true;
        self.session.stage = Stage::Tour;
        self.session.active_tour = self
            .session
            .tours
            .iter()
            .rposition(|t| t.source == TourSource::Proposal(id));
        if let Some(i) = self.session.active_tour {
            self.session.tours[i].current_stop = 0;
        }
        self.navigate_tour()?;
        self.save()
    }
    pub fn apply(&mut self) -> Result<()> {
        ensure!(
            !self.busy && self.session.stage == Stage::Tour,
            "application requires TOUR"
        );
        self.saved()?;
        let base = self.workspace.load(
            self.session
                .pending_base
                .as_ref()
                .context("missing application baseline")?,
        )?;
        // Saved preview edits are part of the proposal the human is applying.
        let draft = self.remember_preview()?;
        let target = self.preserve_human(&base, &draft)?;
        let current = self.current()?;
        let desired = workspace::merge(&base, &current, &target)?;
        workspace::replace(&self.workspace.real, &current, &desired)?;
        self.session.applied = self.workspace.store(&target)?;
        self.session.stage = Stage::Applied;
        self.session.pending_proposal = false;
        self.session.pending_base = None;
        self.session.active_tour = None;
        self.session.change_index = 0;
        self.navigate_change()?;
        self.save()
    }
    pub fn tour(&self) -> Option<&Tour> {
        self.session
            .active_tour
            .and_then(|i| self.session.tours.get(i))
    }
    fn add_tour(&mut self, source: TourSource, content: TourDraft) {
        let id = self.session.tours.len() + 1;
        self.session.tours.push(Tour {
            id,
            source,
            current_stop: 0,
            content,
        });
        self.session.active_tour = Some(id - 1);
    }
    pub fn repository_tour(&mut self, content: TourDraft) -> Result<()> {
        ensure!(
            self.session.stage == Stage::Discuss,
            "repository tours are available during DISCUSS"
        );
        let files = self.current()?;
        validate_tour(&content, &files)?;
        self.add_tour(TourSource::Repository, content);
        self.navigate_tour()?;
        self.save()
    }
    /// A conversational refinement retains the active proposal's source and stage.
    pub fn present_tour(&mut self, content: TourDraft) -> Result<()> {
        if self.session.stage != Stage::Tour {
            return self.repository_tour(content);
        }
        let source = self.tour().context("no active tour")?.source.clone();
        validate_tour(&content, &workspace::capture(&self.workspace.shadow)?)?;
        self.add_tour(source, content);
        self.navigate_tour()?;
        self.save()
    }
    pub fn tour_jump(&mut self, tour_id: usize, index: usize) -> Result<()> {
        let tour = self.tour().context("no active tour")?;
        ensure!(
            tour.id == tour_id,
            "active tour changed; navigation was ignored"
        );
        ensure!(index <= tour.content.stops.len(), "unknown tour stop");
        self.tour_move(index as isize - tour.current_stop as isize)
    }
    pub fn close_tour(&mut self) -> Result<()> {
        ensure!(
            self.tour()
                .is_some_and(|t| t.source == TourSource::Repository),
            "finish a proposal tour with /apply"
        );
        self.session.active_tour = None;
        self.navigation = None;
        self.generation += 1;
        self.save()
    }
    pub fn tour_move(&mut self, offset: isize) -> Result<()> {
        let i = self.session.active_tour.context("no active tour")?;
        let t = &mut self.session.tours[i];
        t.current_stop = t
            .current_stop
            .saturating_add_signed(offset)
            .min(t.content.stops.len());
        self.navigate_tour()?;
        self.save()
    }
    fn navigate_tour(&mut self) -> Result<()> {
        self.navigation = self.tour().and_then(|t| {
            t.current_stop
                .checked_sub(1)
                .and_then(|i| t.content.stops.get(i))
                .map(|s| {
                    let root = if t.source == TourSource::Repository {
                        &self.workspace.real
                    } else {
                        &self.workspace.shadow
                    };
                    Location {
                        file: root.join(&s.file).display().to_string(),
                        line: s.line,
                    }
                })
        });
        self.generation += 1;
        Ok(())
    }
    pub fn changes(&self) -> Result<Vec<Change>> {
        let Some(p) = self.effective_proposal() else {
            return Ok(vec![]);
        };
        if let Some((id, changes)) = &*self.change_cache.borrow()
            && *id == p.id
        {
            return Ok(changes.clone());
        }
        let changes: Vec<_> = self
            .change_targets(&p)?
            .into_iter()
            .enumerate()
            .map(|(id, change)| Change {
                id,
                file: change.file,
                line: change.line,
                label: change.label,
            })
            .collect();
        *self.change_cache.borrow_mut() = Some((p.id, changes.clone()));
        Ok(changes)
    }
    // One target per text hunk, or one target for a creation/deletion/binary/mode change.
    fn change_targets(&self, p: &Proposal) -> Result<Vec<ChangeTarget>> {
        let base = self.workspace.load(&p.base)?;
        let result = self.workspace.load(&p.tree)?;
        let mut targets = Vec::new();
        let paths: std::collections::BTreeSet<_> = base.keys().chain(result.keys()).collect();
        for path in paths {
            if base.get(path) == result.get(path) {
                continue;
            }
            if let (Some(b), Some(r)) = (base.get(path), result.get(path))
                && b.executable == r.executable
                && !b.bytes.contains(&0)
                && !r.bytes.contains(&0)
            {
                let diff = workspace::git(
                    &self.workspace.storage,
                    &[
                        "diff",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--unified=0",
                        &p.base,
                        &p.tree,
                        "--",
                        path,
                    ],
                )?;
                let bl: Vec<_> = b.bytes.split_inclusive(|x| *x == b'\n').collect();
                let rl: Vec<_> = r.bytes.split_inclusive(|x| *x == b'\n').collect();
                for header in String::from_utf8_lossy(&diff)
                    .lines()
                    .filter(|l| l.starts_with("@@ "))
                {
                    let fields: Vec<_> = header.split_whitespace().collect();
                    let (bs, bn) = range(fields[1])?;
                    let (rs, rn) = range(fields[2])?;
                    let mut bytes = Vec::new();
                    for line in &rl[..rs] {
                        bytes.extend_from_slice(line)
                    }
                    for line in &bl[bs..bs + bn] {
                        bytes.extend_from_slice(line)
                    }
                    for line in &rl[rs + rn..] {
                        bytes.extend_from_slice(line)
                    }
                    let target = workspace::File {
                        bytes,
                        executable: r.executable,
                    };
                    targets.push(ChangeTarget {
                        file: path.clone(),
                        line: rs + 1,
                        replacement: Some(target),
                        label: header.to_owned(),
                    });
                }
                continue;
            }
            targets.push(ChangeTarget {
                file: path.clone(),
                line: 1,
                replacement: base.get(path).cloned(),
                label: "file change".into(),
            });
        }
        Ok(targets)
    }
    pub fn revert(&mut self, id: usize) -> Result<()> {
        ensure!(
            !self.busy && self.session.stage == Stage::Applied,
            "revert requires an applied proposal"
        );
        self.saved()?;
        let p = self.effective_proposal().context("missing proposal")?;
        let targets = self.change_targets(&p)?;
        let change = targets.get(id).context("unknown change")?;
        let path = &change.file;
        let replacement = &change.replacement;
        let mut target = self.workspace.load(&p.tree)?;
        if let Some(file) = replacement {
            target.insert(path.clone(), file.clone());
        } else {
            target.remove(path);
        }
        let current = self.current()?;
        let desired = workspace::merge(&self.workspace.load(&p.tree)?, &current, &target)?;
        let expected = workspace::merge(
            &self.workspace.load(&p.tree)?,
            &self.workspace.load(&self.session.applied)?,
            &target,
        )?;
        workspace::replace(&self.workspace.real, &current, &desired)?;
        // Revert is a human decision too; preserve it in later revisions/history switches.
        self.session.human.push(HumanEdit {
            before: self.session.applied.clone(),
            after: self.workspace.store(&expected)?,
        });
        self.session.applied = self.workspace.store(&expected)?;
        self.generation += 1;
        self.save()
    }
    pub fn change_move(&mut self, offset: isize) -> Result<()> {
        ensure!(self.session.stage == Stage::Applied, "no applied proposal");
        self.session.change_index = self
            .session
            .change_index
            .saturating_add_signed(offset)
            .min(self.changes()?.len().saturating_sub(1));
        self.navigate_change()?;
        self.save()
    }
    fn navigate_change(&mut self) -> Result<()> {
        self.navigation = self
            .changes()?
            .get(self.session.change_index)
            .map(|c| Location {
                file: self.workspace.real.join(&c.file).display().to_string(),
                line: c.line,
            });
        self.generation += 1;
        Ok(())
    }
    pub fn context(&self) -> Result<String> {
        Ok(serde_json::to_string(
            &serde_json::json!({"stage":self.session.stage,"proposal":self.session.selected,"active_tour":self.tour(),"tour_stop":self.tour().and_then(|t|t.current_stop.checked_sub(1).and_then(|i|t.content.stops.get(i))),"editor":self.editor}),
        )?)
    }
}
fn range(s: &str) -> Result<(usize, usize)> {
    let (start, count) = s[1..].split_once(',').unwrap_or((&s[1..], "1"));
    let start: usize = start.parse()?;
    let count: usize = count.parse()?;
    Ok((
        if count == 0 {
            start
        } else {
            start.saturating_sub(1)
        },
        count,
    ))
}
fn validate_tour(tour: &TourDraft, result: &Snapshot) -> Result<()> {
    ensure!(
        !tour.title.trim().is_empty() && !tour.overview.trim().is_empty(),
        "tour needs a title and overview"
    );
    for stop in &tour.stops {
        workspace::valid_path(&stop.file)?;
        let file = result
            .get(&stop.file)
            .context("tour references an unknown file")?;
        ensure!(
            stop.line > 0
                && stop.end_line >= stop.line
                && stop.end_line <= file.bytes.split(|b| *b == b'\n').count().max(1),
            "tour range out of bounds: {}:{}–{}",
            stop.file,
            stop.line,
            stop.end_line
        );
    }
    if tour.stops.len() > 200 {
        bail!("tour too large")
    };
    Ok(())
}
