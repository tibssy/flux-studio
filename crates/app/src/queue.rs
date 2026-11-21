//! In-app job queue.
//!
//! The backend runs one request at a time: `sd-server` holds a single context
//! and one loaded model, and an 8 GB card has room for one of them. This queue
//! captures a request at the moment the user presses Generate (or Upscale), so
//! they can keep composing while earlier jobs run. The app only starts a job
//! when the previous one has finished, which also makes "remove from queue"
//! honest.
//!
//! Finished jobs leave the queue and land in History like any other result.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

use imagiflume_core::GenerateRequest;
use imagiflume_core::preset::ModelPreset;
use imagiflume_core::upscaler::{UpscaleRequest, UpscalerPreset};

use crate::GenerationMode;

/// Maximum number of jobs, including the running one.
pub(crate) const LIMIT: usize = 10;

/// One text-to-image, edit or effect request.
#[derive(Clone)]
pub(crate) struct GenerateJob {
    pub request: GenerateRequest,
    pub mode: GenerationMode,
    /// Prompt without the style suffix, as recorded in History.
    pub prompt: String,
    pub style: String,
    pub model: &'static ModelPreset,
    pub width: u32,
    pub height: u32,
    pub seed: u64,
    pub steps: u32,
    /// References as selected by the user (not the temporary copies).
    pub reference_paths: Vec<PathBuf>,
    pub source_path: Option<String>,
    /// Seed field contents when the job was queued, so completion only writes
    /// the used seed back if the user has not typed a new one meanwhile.
    pub seed_text: String,
}

/// Where an upscale job gets its input.
#[derive(Clone)]
pub(crate) enum UpscaleTarget {
    /// A real image that exists now; the request holds its copy.
    Displayed,
    /// The output of a generation that is still queued or running.
    Pending(u64),
}

/// One upscale request.
#[derive(Clone)]
pub(crate) struct UpscaleJob {
    pub request: UpscaleRequest,
    pub target: UpscaleTarget,
    pub upscaler: &'static UpscalerPreset,
    pub factor: u32,
    pub input_width: u32,
    pub input_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    /// Original input path for History's parent lookup, once known.
    pub source_path: Option<String>,
}

#[derive(Clone)]
pub(crate) enum JobKind {
    Generate(GenerateJob),
    Upscale(UpscaleJob),
}

/// One queued request.
#[derive(Clone)]
pub(crate) struct Job {
    /// App-local id, used by the queue rows and notifications.
    pub id: u64,
    pub kind: JobKind,
    /// Per-job copies of references or the upscale input; removed when the job
    /// ends or leaves the queue.
    pub temp_dir: Option<PathBuf>,
}

impl Job {
    pub fn is_upscale(&self) -> bool {
        matches!(self.kind, JobKind::Upscale(_))
    }

    pub fn generate(&self) -> Option<&GenerateJob> {
        match &self.kind {
            JobKind::Generate(job) => Some(job),
            JobKind::Upscale(_) => None,
        }
    }

    pub fn upscale(&self) -> Option<&UpscaleJob> {
        match &self.kind {
            JobKind::Upscale(job) => Some(job),
            JobKind::Generate(_) => None,
        }
    }

    pub fn upscale_mut(&mut self) -> Option<&mut UpscaleJob> {
        match &mut self.kind {
            JobKind::Upscale(job) => Some(job),
            JobKind::Generate(_) => None,
        }
    }

    /// A one-line label for the queue rows and notifications.
    pub fn title(&self) -> String {
        match &self.kind {
            JobKind::Generate(job) => job.snippet(),
            JobKind::Upscale(job) => format!("Upscale {}×", job.factor),
        }
    }

    /// Model, size and status-independent details for a waiting row.
    pub fn meta(&self) -> String {
        match &self.kind {
            JobKind::Generate(job) => format!(
                "{} · {} · {} × {}",
                job.mode.queue_label(),
                job.model.label,
                job.width,
                job.height
            ),
            JobKind::Upscale(job) => format!(
                "{} · {} × {} → {} × {}",
                job.upscaler.label,
                job.input_width,
                job.input_height,
                job.output_width,
                job.output_height
            ),
        }
    }
}

impl GenerateJob {
    /// A one-line prompt snippet.
    pub fn snippet(&self) -> String {
        const MAX_CHARS: usize = 64;
        let prompt = self.prompt.split_whitespace().collect::<Vec<_>>().join(" ");
        if prompt.chars().count() <= MAX_CHARS {
            return prompt;
        }
        let mut snippet: String = prompt.chars().take(MAX_CHARS).collect();
        snippet.push('…');
        snippet
    }

    pub fn reference_strings(&self) -> Vec<String> {
        self.reference_paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }
}

#[derive(Clone)]
pub(crate) struct Running {
    pub job: Job,
    pub started: Instant,
    /// Last step announced by the backend, for the queue subtitle.
    pub step: Option<(u32, u32)>,
    /// The user asked to cancel; the backend has not answered yet.
    pub cancelling: bool,
}

#[derive(Default)]
pub(crate) struct JobQueue {
    pending: VecDeque<Job>,
    running: Option<Running>,
    next_id: u64,
    /// Recent generation outputs, so an upscale whose dialog was open while
    /// the generation finished can still find its input.
    completed: HashMap<u64, PathBuf>,
    /// The user paused the queue; the running job continues, but no new job
    /// starts until it is resumed.
    paused: bool,
}

impl JobQueue {
    /// Reserve the next job id.
    ///
    /// Called before copying files, so the copies get a unique folder even
    /// when the job is never pushed (a failed copy must not consume a slot).
    pub fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Number of jobs in the queue, including the running one.
    pub fn len(&self) -> usize {
        self.pending.len() + usize::from(self.running.is_some())
    }

    pub fn is_full(&self) -> bool {
        self.len() >= LIMIT
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn pending(&self) -> impl Iterator<Item = &Job> {
        self.pending.iter()
    }

    pub fn pending_job_mut(&mut self, id: u64) -> Option<&mut Job> {
        self.pending.iter_mut().find(|job| job.id == id)
    }

    pub fn pending_job(&self, id: u64) -> Option<&Job> {
        self.pending.iter().find(|job| job.id == id)
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Whether a waiting job can move by `delta` without crossing a pending
    /// upscale's dependency.
    pub fn can_move_waiting(&self, id: u64, delta: isize) -> bool {
        let Some(index) = self.pending.iter().position(|job| job.id == id) else {
            return false;
        };
        let target = (index as isize + delta).clamp(0, self.pending.len() as isize - 1) as usize;
        if target == index {
            return false;
        }
        let mut moved = self.pending.clone();
        let job = moved.remove(index).expect("pending index exists");
        moved.insert(target, job);
        for (position, job) in moved.iter().enumerate() {
            let Some(upscale) = job.upscale() else {
                continue;
            };
            let UpscaleTarget::Pending(target_id) = upscale.target else {
                continue;
            };
            if let Some(dependency) = moved.iter().position(|candidate| candidate.id == target_id)
                && dependency > position
            {
                return false;
            }
        }
        true
    }

    /// Move a waiting job by `delta` positions. Returns whether it moved.
    pub fn move_waiting(&mut self, id: u64, delta: isize) -> bool {
        if !self.can_move_waiting(id, delta) {
            return false;
        }
        let index = self
            .pending
            .iter()
            .position(|job| job.id == id)
            .expect("pending index exists");
        let target = (index as isize + delta).clamp(0, self.pending.len() as isize - 1) as usize;
        let job = self.pending.remove(index).expect("pending index exists");
        self.pending.insert(target, job);
        true
    }

    pub fn running(&self) -> Option<&Running> {
        self.running.as_ref()
    }

    pub fn running_mut(&mut self) -> Option<&mut Running> {
        self.running.as_mut()
    }

    pub fn push(&mut self, job: Job) {
        self.pending.push_back(job);
    }

    /// Pop the oldest waiting job. The caller starts it immediately.
    pub fn take_next(&mut self) -> Option<Job> {
        self.pending.pop_front()
    }

    pub fn start(&mut self, job: Job) {
        self.running = Some(Running {
            job,
            started: Instant::now(),
            step: None,
            cancelling: false,
        });
    }

    pub fn clear_running(&mut self) {
        self.running = None;
    }

    pub fn remember_output(&mut self, id: u64, path: PathBuf) {
        const KEEP: usize = 32;
        if self.completed.len() >= KEEP
            && let Some(oldest) = self.completed.keys().min().copied()
        {
            self.completed.remove(&oldest);
        }
        self.completed.insert(id, path);
    }

    pub fn completed_output(&self, id: u64) -> Option<&Path> {
        self.completed.get(&id).map(PathBuf::as_path)
    }

    /// Whether any running or waiting job uses `path` as a reference or as an
    /// upscale source.
    pub fn uses_source(&self, path: &Path) -> bool {
        let matches = |value: Option<&str>| value.is_some_and(|value| Path::new(value) == path);
        let job_uses = |job: &Job| match &job.kind {
            JobKind::Generate(job) => {
                job.reference_paths
                    .iter()
                    .any(|candidate| candidate == path)
                    || matches(job.source_path.as_deref())
            }
            JobKind::Upscale(job) => matches(job.source_path.as_deref()),
        };
        self.running
            .as_ref()
            .is_some_and(|running| job_uses(&running.job))
            || self.pending.iter().any(job_uses)
    }

    /// Whether `id` is still running or waiting.
    pub fn is_active(&self, id: u64) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.job.id == id)
            || self.pending.iter().any(|job| job.id == id)
    }

    /// Drop a waiting job and return it so its copies can be removed.
    pub fn remove(&mut self, id: u64) -> Option<Job> {
        let index = self.pending.iter().position(|job| job.id == id)?;
        self.pending.remove(index)
    }

    /// Drop every waiting job and return them for cleanup.
    pub fn clear_pending(&mut self) -> Vec<Job> {
        self.pending.drain(..).collect()
    }

    /// Ids of waiting upscales that depend on `target_id`.
    pub fn pending_upscales_for(&self, target_id: u64) -> Vec<u64> {
        self.pending
            .iter()
            .filter(|job| {
                matches!(
                    job.upscale().map(|upscale| &upscale.target),
                    Some(UpscaleTarget::Pending(id)) if *id == target_id
                )
            })
            .map(|job| job.id)
            .collect()
    }

    /// Point a waiting upscale at the finished generation's output.
    pub fn resolve_upscale(&mut self, id: u64, input: PathBuf, source_path: String) {
        let Some(job) = self.pending.iter_mut().find(|job| job.id == id) else {
            return;
        };
        let Some(upscale) = job.upscale_mut() else {
            return;
        };
        upscale.target = UpscaleTarget::Displayed;
        upscale.request.input = input;
        upscale.source_path = Some(source_path);
    }

    /// Drop waiting upscales whose generation failed or was cancelled.
    pub fn drop_pending_upscales(&mut self, target_id: u64) -> Vec<Job> {
        let mut dropped = Vec::new();
        let mut kept = VecDeque::with_capacity(self.pending.len());
        while let Some(job) = self.pending.pop_front() {
            let matches = matches!(
                job.upscale().map(|upscale| &upscale.target),
                Some(UpscaleTarget::Pending(id)) if *id == target_id
            );
            if matches {
                dropped.push(job);
            } else {
                kept.push_back(job);
            }
        }
        self.pending = kept;
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imagiflume_core::preset::FLUX2_KLEIN_4B;
    use imagiflume_core::upscaler::MOSR_4X;

    fn generate_job(id: u64) -> Job {
        let mut request = GenerateRequest::new("a prompt", format!("/tmp/{id}.png"));
        request.preset_id = FLUX2_KLEIN_4B.id.into();
        Job {
            id,
            kind: JobKind::Generate(GenerateJob {
                request,
                mode: GenerationMode::Generate,
                prompt: "a prompt".into(),
                style: "none".into(),
                model: &FLUX2_KLEIN_4B,
                width: 608,
                height: 608,
                seed: 42,
                steps: 4,
                reference_paths: Vec::new(),
                source_path: None,
                seed_text: String::new(),
            }),
            temp_dir: None,
        }
    }

    fn upscale_job(id: u64, target: UpscaleTarget) -> Job {
        Job {
            id,
            kind: JobKind::Upscale(UpscaleJob {
                request: UpscaleRequest {
                    input: PathBuf::new(),
                    output: format!("/tmp/{id}-up.png").into(),
                    upscaler_id: MOSR_4X.id.into(),
                    factor: 2,
                    tile_size: 0,
                },
                target,
                upscaler: &MOSR_4X,
                factor: 2,
                input_width: 608,
                input_height: 608,
                output_width: 1216,
                output_height: 1216,
                source_path: None,
            }),
            temp_dir: None,
        }
    }

    #[test]
    fn runs_jobs_in_order() {
        let mut queue = JobQueue::default();
        queue.push(generate_job(1));
        queue.push(generate_job(2));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.take_next().unwrap().id, 1);
        queue.start(generate_job(1));
        assert_eq!(queue.len(), 2);
        queue.clear_running();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.take_next().unwrap().id, 2);
    }

    #[test]
    fn reorder_respects_pending_upscale_dependencies() {
        let mut queue = JobQueue::default();
        queue.push(generate_job(1));
        queue.push(upscale_job(2, UpscaleTarget::Pending(1)));
        queue.push(generate_job(3));

        assert!(
            !queue.can_move_waiting(2, -1),
            "upscale before its generation"
        );
        assert!(
            !queue.can_move_waiting(1, 1),
            "generation after its upscale"
        );
        assert!(queue.can_move_waiting(3, -1));
        assert!(!queue.can_move_waiting(1, 0));

        assert!(!queue.move_waiting(2, -1));
        assert_eq!(
            queue.pending().map(|job| job.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn reorders_waiting_jobs() {
        let mut queue = JobQueue::default();
        for id in 1..=3 {
            queue.push(generate_job(id));
        }
        assert!(queue.move_waiting(3, -1));
        assert_eq!(
            queue.pending().map(|job| job.id).collect::<Vec<_>>(),
            vec![1, 3, 2]
        );
        assert!(queue.move_waiting(3, -1));
        assert_eq!(
            queue.pending().map(|job| job.id).collect::<Vec<_>>(),
            vec![3, 1, 2]
        );
        assert!(!queue.move_waiting(3, -1), "already first");
        assert!(!queue.move_waiting(2, 1), "already last");
        assert!(!queue.move_waiting(99, -1), "unknown job");
    }

    #[test]
    fn pauses_without_dropping_jobs() {
        let mut queue = JobQueue::default();
        assert!(!queue.is_paused());
        queue.push(generate_job(1));
        queue.set_paused(true);
        assert!(queue.is_paused());
        assert_eq!(queue.pending_len(), 1);
        queue.set_paused(false);
        assert_eq!(queue.take_next().unwrap().id, 1);
    }

    #[test]
    fn caps_at_the_limit_including_running() {
        let mut queue = JobQueue::default();
        for id in 0..LIMIT as u64 {
            assert!(!queue.is_full());
            queue.push(generate_job(id));
        }
        assert!(queue.is_full());
        assert_eq!(queue.len(), LIMIT);
        queue.start(generate_job(99));
        assert!(queue.is_full());
    }

    #[test]
    fn removes_waiting_jobs_only() {
        let mut queue = JobQueue::default();
        queue.push(generate_job(1));
        queue.push(generate_job(2));
        queue.start(generate_job(3));
        assert!(queue.remove(1).is_some());
        assert!(queue.remove(1).is_none());
        assert!(queue.remove(3).is_none());
        assert_eq!(queue.pending_len(), 1);
        assert_eq!(queue.running().unwrap().job.id, 3);
    }

    #[test]
    fn clears_waiting_jobs_and_keeps_running() {
        let mut queue = JobQueue::default();
        queue.push(generate_job(1));
        queue.push(generate_job(2));
        queue.start(generate_job(3));
        assert_eq!(queue.clear_pending().len(), 2);
        assert_eq!(queue.pending_len(), 0);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn resolves_upscales_that_depend_on_a_generation() {
        let mut queue = JobQueue::default();
        queue.push(upscale_job(1, UpscaleTarget::Pending(7)));
        queue.push(upscale_job(2, UpscaleTarget::Pending(8)));
        queue.push(upscale_job(3, UpscaleTarget::Displayed));

        assert_eq!(queue.pending_upscales_for(7), vec![1]);
        queue.resolve_upscale(1, "/tmp/out.png".into(), "/tmp/out.png".into());
        let job = queue.pending().find(|job| job.id == 1).unwrap();
        let upscale = job.upscale().unwrap();
        assert!(matches!(&upscale.target, UpscaleTarget::Displayed));
        assert_eq!(upscale.request.input, PathBuf::from("/tmp/out.png"));

        let dropped = queue.drop_pending_upscales(8);
        assert_eq!(dropped.len(), 1);
        assert_eq!(queue.pending_len(), 2);
    }

    #[test]
    fn keeps_recent_outputs_for_upscales() {
        let mut queue = JobQueue::default();
        queue.push(generate_job(7));
        assert!(queue.is_active(7));
        queue.remember_output(7, "/tmp/out.png".into());
        assert_eq!(queue.completed_output(7), Some(Path::new("/tmp/out.png")));
        queue.clear_pending();
        assert!(!queue.is_active(7));
        assert_eq!(queue.completed_output(7), Some(Path::new("/tmp/out.png")));
    }

    #[test]
    fn reports_sources_used_by_queued_jobs() {
        let mut queue = JobQueue::default();
        let mut generate = generate_job(1);
        if let JobKind::Generate(job) = &mut generate.kind {
            job.reference_paths = vec!["/tmp/ref.png".into()];
        }
        queue.push(generate);

        let mut upscale = upscale_job(2, UpscaleTarget::Displayed);
        if let Some(job) = upscale.upscale_mut() {
            job.source_path = Some("/tmp/source.png".into());
        }
        queue.push(upscale);

        assert!(queue.uses_source(Path::new("/tmp/ref.png")));
        assert!(queue.uses_source(Path::new("/tmp/source.png")));
        assert!(!queue.uses_source(Path::new("/tmp/other.png")));

        let running = queue.take_next().unwrap();
        queue.start(running);
        assert!(queue.uses_source(Path::new("/tmp/ref.png")));
    }

    #[test]
    fn ids_are_monotonic_and_unique() {
        let mut queue = JobQueue::default();
        assert_eq!(queue.next_id(), 1);
        assert_eq!(queue.next_id(), 2);
    }

    #[test]
    fn snippets_collapse_whitespace_and_limit_length() {
        let mut job = generate_job(1);
        if let JobKind::Generate(generate) = &mut job.kind {
            generate.prompt = format!("{}\n\n  second line", "word ".repeat(20));
        }
        let snippet = job.title();
        assert!(snippet.chars().count() <= 65);
        assert!(snippet.ends_with('…'));
        assert!(!snippet.contains('\n'));
    }

    #[test]
    fn labels_upscales_and_generations() {
        let generate = generate_job(1);
        assert_eq!(generate.title(), "a prompt");
        let upscale = upscale_job(2, UpscaleTarget::Displayed);
        assert_eq!(upscale.title(), "Upscale 2×");
        assert!(upscale.meta().contains("→ 1216 × 1216"));
    }
}
