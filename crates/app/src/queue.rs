//! In-app job queue.
//!
//! The backend runs one request at a time: `sd-server` holds a single context
//! and one loaded model, and an 8 GB card has room for one of them. This queue
//! captures a request at the moment the user presses Generate, so they can keep
//! composing the next one while earlier jobs run. The app only sends a job to
//! the worker when the previous one has finished, which also makes "remove from
//! queue" honest.
//!
//! Finished jobs leave the queue and land in History like any other result.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

use flux_core::GenerateRequest;
use flux_core::preset::ModelPreset;

use crate::GenerationMode;

/// Maximum number of jobs, including the running one.
pub(crate) const LIMIT: usize = 10;

/// One generation request captured from the sidebar.
#[derive(Clone)]
pub(crate) struct Job {
    /// App-local id, used by the queue rows and notifications.
    pub id: u64,
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
    /// Per-job copies of the references; removed when the job ends.
    pub reference_dir: Option<PathBuf>,
}

impl Job {
    /// A one-line label for the queue rows and notifications.
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
}

impl JobQueue {
    /// Reserve the next job id.
    ///
    /// Called before copying references, so the copies get a unique folder even
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

    /// Drop a waiting job. The running job cannot be removed this way.
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.pending.len();
        self.pending.retain(|job| job.id != id);
        self.pending.len() != before
    }

    /// Drop every waiting job and return how many were removed.
    pub fn clear_pending(&mut self) -> usize {
        let removed = self.pending.len();
        self.pending.clear();
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_core::preset::FLUX2_KLEIN_4B;

    fn job(id: u64) -> Job {
        let mut request = GenerateRequest::new("a prompt", format!("/tmp/{id}.png"));
        request.preset_id = FLUX2_KLEIN_4B.id.into();
        Job {
            id,
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
            reference_dir: None,
        }
    }

    #[test]
    fn runs_jobs_in_order() {
        let mut queue = JobQueue::default();
        queue.push(job(1));
        queue.push(job(2));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.take_next().unwrap().id, 1);
        queue.start(job(1));
        assert_eq!(queue.len(), 2);
        queue.clear_running();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.take_next().unwrap().id, 2);
    }

    #[test]
    fn caps_at_the_limit_including_running() {
        let mut queue = JobQueue::default();
        for id in 0..LIMIT as u64 {
            assert!(!queue.is_full());
            queue.push(job(id));
        }
        assert!(queue.is_full());
        assert_eq!(queue.len(), LIMIT);
        queue.start(job(99));
        assert!(queue.is_full());
    }

    #[test]
    fn removes_waiting_jobs_only() {
        let mut queue = JobQueue::default();
        queue.push(job(1));
        queue.push(job(2));
        queue.start(job(3));
        assert!(queue.remove(1));
        assert!(!queue.remove(1));
        assert!(!queue.remove(3));
        assert_eq!(queue.pending_len(), 1);
        assert_eq!(queue.running().unwrap().job.id, 3);
    }

    #[test]
    fn clears_waiting_jobs_and_keeps_running() {
        let mut queue = JobQueue::default();
        queue.push(job(1));
        queue.push(job(2));
        queue.start(job(3));
        assert_eq!(queue.clear_pending(), 2);
        assert_eq!(queue.pending_len(), 0);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn ids_are_monotonic_and_unique() {
        let mut queue = JobQueue::default();
        assert_eq!(queue.next_id(), 1);
        assert_eq!(queue.next_id(), 2);
    }

    #[test]
    fn snippets_collapse_whitespace_and_limit_length() {
        let mut long = job(1);
        long.prompt = format!("{}\n\n  second line", "word ".repeat(20));
        let snippet = long.snippet();
        assert!(snippet.chars().count() <= 65);
        assert!(snippet.ends_with('…'));
        assert!(!snippet.contains('\n'));
    }
}
