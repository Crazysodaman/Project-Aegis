//! Autonomy execution engine.
//!
//! The engine executes scheduled tasks against the real per-Core stores. It
//! never fabricates a document, inventory count, experience, or consent grant:
//! every activity reads or writes durable Core state, and unsupported activity
//! kinds fail rather than pretending to succeed.

use aegis_contracts::living::{ConsentScope, Task, TaskStatus, MAX_DOCUMENT_BODY_CHARS};
use aegis_contracts::personal::MAX_EXPERIENCE_CHARS;
use serde::Serialize;
use serde_json::Value;

use crate::autonomy::{AutonomyStore, MAX_AUTONOMY_LIST};
use crate::consent::ConsentStore;
use crate::personal::PersonalStore;
use crate::possessions::PossessionsStore;
use crate::{CoreSdkContext, SdkError};

const DEFAULT_PERCENT_PER_RUN: u8 = 34;
const MAX_INVENTORY_SCAN: usize = 256;

/// Result of executing one task during a due run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskRun {
    /// Task identifier.
    pub task_id: String,
    /// Resulting task status.
    pub status: TaskStatus,
    /// Persisted progress percentage after the run.
    pub progress_percent: u8,
    /// Bounded human-readable outcome.
    pub message: String,
}

/// Executes due autonomy tasks through the real Core stores.
pub struct AutonomyEngine<'a> {
    autonomy: &'a AutonomyStore,
    possessions: &'a PossessionsStore,
    consent: &'a ConsentStore,
    personal: &'a PersonalStore,
}

impl<'a> AutonomyEngine<'a> {
    /// Build an engine from an opened SDK context bound to one Core.
    pub fn new(context: &'a CoreSdkContext) -> Self {
        Self::from_stores(
            &context.autonomy,
            &context.possessions,
            &context.consent,
            &context.personal,
        )
    }

    /// Build an engine from explicit stores that must all belong to one Core.
    pub fn from_stores(
        autonomy: &'a AutonomyStore,
        possessions: &'a PossessionsStore,
        consent: &'a ConsentStore,
        personal: &'a PersonalStore,
    ) -> Self {
        Self {
            autonomy,
            possessions,
            consent,
            personal,
        }
    }

    /// Execute due pending tasks and previously active/resumed tasks.
    ///
    /// When autonomy is disabled the run is a no-op that returns an empty list
    /// and leaves all task state untouched. Partial work is persisted and the
    /// task is paused so a later [`crate::AutonomyStore::resume_task`] can
    /// continue from the stored progress.
    pub fn run_due(&self, now: &str) -> Result<Vec<TaskRun>, SdkError> {
        let settings = self.autonomy.settings()?;
        if !settings.enabled {
            return Ok(Vec::new());
        }

        let mut candidates = self.autonomy.tick(now)?;
        let known: std::collections::BTreeSet<String> =
            candidates.iter().map(|task| task.task_id.clone()).collect();
        for task in self.autonomy.list_tasks(MAX_AUTONOMY_LIST)? {
            if task.status == TaskStatus::Active && !known.contains(&task.task_id) {
                candidates.push(task);
            }
        }
        candidates.sort_by(|left, right| {
            left.due_at
                .cmp(&right.due_at)
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| left.task_id.cmp(&right.task_id))
        });

        let max_active = usize::from(settings.max_active_tasks);
        let mut started = 0usize;
        let mut runs = Vec::new();
        for task in candidates {
            let is_new_start = task.status != TaskStatus::Active;
            if is_new_start && started >= max_active {
                continue;
            }
            if is_new_start {
                started += 1;
            }
            let run = self.execute_task(task, now)?;
            runs.push(run);
        }
        Ok(runs)
    }

    fn execute_task(&self, task: Task, now: &str) -> Result<TaskRun, SdkError> {
        let task = if task.status == TaskStatus::Active {
            task
        } else {
            self.autonomy.start_task(&task.task_id)?
        };

        match task.activity_kind.as_str() {
            "read_document" => self.read_document(&task),
            "write_journal" => self.write_journal(&task),
            "organize_inventory" => self.organize_inventory(&task),
            "social_interaction" => self.social_interaction(&task, now),
            "reflect" => self.reflect(&task),
            _ => self.fail(task.task_id.as_str(), "unsupported_activity"),
        }
    }

    fn read_document(&self, task: &Task) -> Result<TaskRun, SdkError> {
        let Some(document_id) = field_str(&task.payload, "document_id") else {
            return self.fail(&task.task_id, "invalid_payload");
        };
        let Some(document) = self.possessions.get_document(&document_id)? else {
            return self.fail(&task.task_id, "document_not_found");
        };
        let step = field_percent(&task.payload).unwrap_or(DEFAULT_PERCENT_PER_RUN);
        let progress = task.progress_percent.saturating_add(step).min(100);
        self.personal
            .record_experience(&bounded_experience(&format!(
                "Read document {}",
                document.title
            )))?;
        self.persist_progress(task, progress, &format!("read document {}", document.title))
    }

    fn write_journal(&self, task: &Task) -> Result<TaskRun, SdkError> {
        let Some(document_id) = field_str(&task.payload, "document_id") else {
            return self.fail(&task.task_id, "invalid_payload");
        };
        let Some(append) = field_str(&task.payload, "append") else {
            return self.fail(&task.task_id, "invalid_payload");
        };
        let Some(document) = self.possessions.get_document(&document_id)? else {
            return self.fail(&task.task_id, "document_not_found");
        };
        let body = if document.body.is_empty() {
            append.clone()
        } else {
            format!("{}\n{}", document.body, append)
        };
        if body.chars().count() > MAX_DOCUMENT_BODY_CHARS {
            return self.fail(&task.task_id, "document_too_large");
        }
        self.possessions.update_document(
            &document.document_id,
            &document.title,
            &body,
            document.format,
            document.visibility,
        )?;
        let step = field_percent(&task.payload).unwrap_or(100);
        let progress = task.progress_percent.saturating_add(step).min(100);
        self.personal
            .record_experience(&bounded_experience(&format!(
                "Wrote journal entry to {}",
                document.title
            )))?;
        self.persist_progress(
            task,
            progress,
            &format!("appended to journal {}", document.title),
        )
    }

    fn organize_inventory(&self, task: &Task) -> Result<TaskRun, SdkError> {
        let inventory = self.possessions.list_inventory(MAX_INVENTORY_SCAN)?;
        let count = inventory.len();
        self.personal
            .record_experience(&bounded_experience(&format!(
                "Organized inventory ({count} items)"
            )))?;
        self.complete(task, 100, &format!("organized inventory ({count} items)"))
    }

    fn social_interaction(&self, task: &Task, now: &str) -> Result<TaskRun, SdkError> {
        let Some(participant) = field_str(&task.payload, "participant") else {
            return self.fail(&task.task_id, "invalid_payload");
        };
        let Some(capability) = field_str(&task.payload, "capability") else {
            return self.fail(&task.task_id, "invalid_payload");
        };
        let Some(scope) = field_str(&task.payload, "scope").and_then(|s| parse_scope(&s)) else {
            return self.fail(&task.task_id, "invalid_payload");
        };
        if !self.consent.check(scope, &capability, &participant, now) {
            return self.fail(&task.task_id, "consent_required");
        }
        self.personal
            .record_experience(&bounded_experience(&format!(
                "Social interaction: {capability} with {participant}"
            )))?;
        self.complete(
            task,
            100,
            &format!("completed {capability} with {participant}"),
        )
    }

    fn reflect(&self, task: &Task) -> Result<TaskRun, SdkError> {
        let reflection = field_str(&task.payload, "summary").unwrap_or_else(|| task.title.clone());
        self.personal
            .record_experience(&bounded_experience(&format!("Reflected: {reflection}")))?;
        self.complete(task, 100, "recorded reflection")
    }

    fn persist_progress(
        &self,
        task: &Task,
        progress: u8,
        message: &str,
    ) -> Result<TaskRun, SdkError> {
        if progress >= 100 {
            return self.complete(task, 100, message);
        }
        let mut updated = task.clone();
        updated.progress_percent = progress;
        let updated = self.autonomy.update_task(&updated)?;
        let paused = self.autonomy.pause_task(&updated.task_id)?;
        Ok(TaskRun {
            task_id: paused.task_id,
            status: paused.status,
            progress_percent: paused.progress_percent,
            message: message.to_owned(),
        })
    }

    fn complete(&self, task: &Task, progress: u8, message: &str) -> Result<TaskRun, SdkError> {
        let completed = self.autonomy.complete_task(&task.task_id, progress)?;
        Ok(TaskRun {
            task_id: completed.task_id,
            status: completed.status,
            progress_percent: completed.progress_percent,
            message: message.to_owned(),
        })
    }

    fn fail(&self, task_id: &str, reason: &str) -> Result<TaskRun, SdkError> {
        let failed = self.autonomy.fail_task(task_id, reason)?;
        Ok(TaskRun {
            task_id: failed.task_id,
            status: failed.status,
            progress_percent: failed.progress_percent,
            message: reason.to_owned(),
        })
    }
}

fn field_str(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn field_percent(payload: &Value) -> Option<u8> {
    payload
        .get("percent_per_run")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .map(|value| value.clamp(1, 100))
}

fn parse_scope(value: &str) -> Option<ConsentScope> {
    Some(match value {
        "conversation" => ConsentScope::Conversation,
        "affection" => ConsentScope::Affection,
        "romantic" => ConsentScope::Romantic,
        "adult" => ConsentScope::Adult,
        "object" => ConsentScope::Object,
        "shared_project" => ConsentScope::SharedProject,
        _ => return None,
    })
}

fn bounded_experience(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_EXPERIENCE_CHARS)
        .collect();
    if cleaned.trim().is_empty() {
        "Autonomy activity".to_owned()
    } else {
        cleaned
    }
}
