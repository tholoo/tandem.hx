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
    pub plan: String,
    pub tours: Vec<Tour>,
    pub active_tour: Option<usize>,
    pub change_index: usize,
    human: Vec<HumanEdit>,
}
pub struct Controller {
    pub workspace: Workspace,
    pub session: Session,
    pub editor: EditorContext,
    pub busy: bool,
    pub generation: u64,
    pub navigation: Option<Location>,
    path: PathBuf,
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
                plan: String::new(),
                tours: vec![],
                active_tour: None,
                change_index: 0,
                human: vec![],
            },
            editor: EditorContext::default(),
            busy: false,
            generation: 0,
            navigation: None,
            path: directory.join("session.json"),
        };
        c.save()?;
        Ok(c)
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
            self.editor.dirty.is_empty(),
            "save your Helix buffers first: {}",
            self.editor.dirty.join(", ")
        );
        Ok(())
    }
    pub fn plan(&mut self, text: String) -> Result<()> {
        ensure!(
            !self.busy && self.session.stage != Stage::Building,
            "agent is busy"
        );
        ensure!(!text.trim().is_empty(), "plan cannot be empty");
        self.session.plan = text;
        self.session.stage = Stage::Plan;
        self.save()
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
    pub fn begin(&mut self) -> Result<()> {
        ensure!(
            !self.busy && self.session.stage == Stage::Plan,
            "Begin requires PLAN and an idle agent"
        );
        self.saved()?;
        let current = workspace::capture(&self.workspace.real)?;
        self.observe_human(&current)?;
        self.workspace.sync(&current)?;
        self.session.pending_base = Some(self.workspace.store(&current)?);
        self.session.stage = Stage::Building;
        self.session.active_tour = None;
        self.busy = true;
        self.save()
    }
    pub fn complete(&mut self, tour: TourDraft) -> Result<()> {
        ensure!(
            self.session.stage == Stage::Building,
            "no build to complete"
        );
        let result = workspace::capture(&self.workspace.shadow)?;
        let base = self
            .session
            .pending_base
            .clone()
            .context("missing build baseline")?;
        let base_files = self.workspace.load(&base)?;
        // Check whether this revision changes a human's prior choice. The reverse merge is
        // a conflict probe only: its result is discarded, never used to undo human work.
        for edit in &self.session.human {
            let before = self.workspace.load(&edit.before)?;
            let after = self.workspace.load(&edit.after)?;
            let reverse = workspace::merge(&after, &base_files, &before)?;
            workspace::merge(&base_files, &result, &reverse)
                .context("revision overlaps a manual edit; proposal was not applied")?;
        }
        validate_tour(&tour, &result, &base_files)?;
        let tree = self.workspace.store(&result)?;
        let id = self.session.proposals.len() + 1;
        self.session.proposals.push(Proposal {
            id,
            base: base.clone(),
            tree,
        });
        self.session.selected = Some(id);
        self.add_tour(TourSource::Proposal(id), tour);
        self.session.change_index = 0;
        self.session.stage = Stage::Tour;
        self.busy = false;
        self.navigate_tour()?;
        self.save()
    }
    pub fn failed(&mut self) -> Result<()> {
        self.busy = false;
        if self.session.stage == Stage::Building {
            self.session.stage = Stage::Plan;
        }
        self.save()
    }
    pub fn switch(&mut self, id: usize) -> Result<()> {
        ensure!(
            !self.busy
                && matches!(
                    self.session.stage,
                    Stage::Review | Stage::Tour | Stage::Plan
                ),
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
        let current = workspace::capture(&self.workspace.real)?;
        self.observe_human(&current)?;
        let mut target = self.workspace.load(&p.tree)?;
        for edit in &self.session.human {
            target = workspace::merge(
                &self.workspace.load(&edit.before)?,
                &target,
                &self.workspace.load(&edit.after)?,
            )
            .context("older proposal conflicts with a saved human choice")?;
        }
        let from = self.workspace.load(&self.session.applied)?;
        let desired = workspace::merge(&from, &current, &target)?;
        self.workspace.sync(&desired)?;
        self.session.pending_base = Some(self.workspace.store(&current)?);
        self.session.selected = Some(id);
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
        // Use the immutable proposal plus explicitly reconciled human choices. Never trust
        // an editable preview buffer as the source of the proposal.
        let p = self.proposal().context("missing proposal")?.clone();
        let mut target = self.workspace.load(&p.tree)?;
        for edit in &self.session.human {
            target = workspace::merge(
                &self.workspace.load(&edit.before)?,
                &target,
                &self.workspace.load(&edit.after)?,
            )?;
        }
        let current = workspace::capture(&self.workspace.real)?;
        let desired = workspace::merge(&base, &current, &target)?;
        workspace::replace(&self.workspace.real, &current, &desired)?;
        self.session.applied = self.workspace.store(&target)?;
        self.session.stage = Stage::Review;
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
            matches!(self.session.stage, Stage::Discuss | Stage::Plan),
            "repository tours are available during DISCUSS/PLAN"
        );
        let files = workspace::capture(&self.workspace.real)?;
        validate_tour(&content, &files, &files)?;
        self.add_tour(TourSource::Repository, content);
        self.navigate_tour()?;
        self.save()
    }
    pub fn close_tour(&mut self) -> Result<()> {
        ensure!(
            self.tour()
                .is_some_and(|t| t.source == TourSource::Repository),
            "finish a proposal tour with Review"
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
        let Some(p) = self.proposal() else {
            return Ok(vec![]);
        };
        Ok(self
            .change_targets(p)?
            .into_iter()
            .enumerate()
            .map(|(id, (file, line, _, label))| Change {
                id,
                file,
                line,
                label,
            })
            .collect())
    }
    // One target per text hunk, or one target for a creation/deletion/binary/mode change.
    fn change_targets(&self, p: &Proposal) -> Result<Vec<(String, usize, Snapshot, String)>> {
        let base = self.workspace.load(&p.base)?;
        let result = self.workspace.load(&p.tree)?;
        let mut targets = Vec::new();
        let paths: std::collections::BTreeSet<_> = base.keys().chain(result.keys()).collect();
        for path in paths {
            if base.get(path) == result.get(path) {
                continue;
            }
            if let (Some(b), Some(r)) = (base.get(path), result.get(path)) {
                if b.executable == r.executable && !b.bytes.contains(&0) && !r.bytes.contains(&0) {
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
                        let mut target = result.clone();
                        target.get_mut(path).unwrap().bytes = bytes;
                        targets.push((path.clone(), rs + 1, target, header.to_owned()));
                    }
                    continue;
                }
            }
            let mut target = result.clone();
            if let Some(b) = base.get(path) {
                target.insert(path.clone(), b.clone());
            } else {
                target.remove(path);
            }
            targets.push((path.clone(), 1, target, "file change".into()));
        }
        Ok(targets)
    }
    pub fn revert(&mut self, id: usize) -> Result<()> {
        ensure!(
            !self.busy && self.session.stage == Stage::Review,
            "revert requires REVIEW"
        );
        self.saved()?;
        let p = self.proposal().context("missing proposal")?.clone();
        let targets = self.change_targets(&p)?;
        let (_, _, target, _) = targets.get(id).context("unknown change")?;
        let current = workspace::capture(&self.workspace.real)?;
        let desired = workspace::merge(&self.workspace.load(&p.tree)?, &current, target)?;
        let expected = workspace::merge(
            &self.workspace.load(&p.tree)?,
            &self.workspace.load(&self.session.applied)?,
            target,
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
        ensure!(self.session.stage == Stage::Review, "not in REVIEW");
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
fn validate_tour(tour: &TourDraft, result: &Snapshot, base: &Snapshot) -> Result<()> {
    ensure!(
        !tour.title.trim().is_empty() && !tour.overview.trim().is_empty(),
        "tour needs a title and overview"
    );
    for stop in &tour.stops {
        workspace::valid_path(&stop.file)?;
        let file = result
            .get(&stop.file)
            .or_else(|| base.get(&stop.file))
            .context("tour references an unknown file")?;
        ensure!(
            stop.line > 0 && stop.line <= file.bytes.split(|b| *b == b'\n').count().max(1),
            "tour line out of bounds: {}:{}",
            stop.file,
            stop.line
        );
    }
    if tour.stops.len() > 200 {
        bail!("tour too large")
    };
    Ok(())
}
