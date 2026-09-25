//! Branch diagnostics for the inspector.
//!
//! The inspector's debug panel shows a repository's local metadata and, on
//! request, the branch classified against its upstream. It asserts
//! [`InspectBranch`] on the branch it inspects, and the worker answers on
//! that branch's [`BranchInspection`] overlay row, which the panel reads
//! with a query once the row carries its stamp.
//!
//! [`InspectBranch`]: tonk_schema::command::InspectBranch
//! [`BranchInspection`]: tonk_schema::BranchInspection

use super::sync::{SyncPath, check_status};

/// Run [`InspectBranch`] on the branch it was asserted on.
///
/// [`InspectBranch`]: tonk_schema::command::InspectBranch
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::InspectBranch>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::InspectBranch) {
        use tonk_schema::BranchInspection;
        use tonk_schema::domain::branch_inspection::{AnsweredAt, Failure, Repository, Status};

        let origin = self.origin().clone();
        let mut failures = Vec::new();
        let repository =
            match super::repository::load_repository_info(self.state(), &origin.repo).await {
                Ok(info) => serde_json::to_string(&info).unwrap_or_default(),
                Err(error) => {
                    failures.push(format!("repository: {error}"));
                    String::new()
                }
            };
        let status = if command.probe.0 {
            let params = SyncPath {
                repo: origin.repo.clone(),
                branch: origin.branch.clone(),
            };
            match check_status(self.state(), &params).await {
                Ok(status) => serde_json::to_string(&status).unwrap_or_default(),
                Err(error) => {
                    failures.push(format!("upstream: {error}"));
                    String::new()
                }
            }
        } else {
            String::new()
        };

        let Ok(this) = BranchInspection::ENTITY.parse() else {
            return;
        };
        let tonk = self.state().read().await;
        let written = tonk
            .reactor
            .repository(&origin.repo)
            .branch(&origin.branch)
            .overlay()
            .assert(BranchInspection {
                this,
                answered_at: AnsweredAt(command.at.0),
                repository: Repository(repository),
                status: Status(status),
                failure: Failure(failures.join("; ")),
            })
            .write()
            .perform(&tonk.operator)
            .await;
        if let Err(error) = written {
            tonk_common::log!("branch inspection not published: {error}");
        }
    }
}
