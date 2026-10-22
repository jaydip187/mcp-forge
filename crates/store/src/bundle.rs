//! Bulk import of auth configs and tools (the YAML bundle), all or nothing.

use std::collections::HashMap;

use forge_core::{AuthConfigInput, ToolInput, ToolKind, ValidationErrors};
use serde::Serialize;
use uuid::Uuid;

use crate::tools::{apply_input, insert_tool, new_tool_def, write_tool};
use crate::{now, to_json, Store, StoreError};

/// What to do when an item with the same name already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Abort the whole import.
    #[default]
    Fail,
    /// Keep the existing item.
    Skip,
    /// Overwrite the existing item.
    Replace,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BundleReport {
    pub auth_created: Vec<String>,
    pub auth_updated: Vec<String>,
    /// Existing configs kept (identical, or skipped by policy).
    pub auth_kept: Vec<String>,
    pub tools_created: Vec<String>,
    pub tools_updated: Vec<String>,
    pub tools_skipped: Vec<String>,
}

impl Store {
    /// Imports auth configs then tools. A tool's `auth` names an auth config
    /// from the bundle or already in the project. New tools are manual tools.
    pub async fn import_bundle(
        &self,
        project_id: Uuid,
        auths: &[AuthConfigInput],
        tools: &[(ToolInput, Option<String>)],
        policy: ConflictPolicy,
    ) -> Result<BundleReport, StoreError> {
        self.ensure_project(project_id).await?;
        let mut errs = ValidationErrors::default();
        for (i, a) in auths.iter().enumerate() {
            if let Err(e) = a.validate() {
                for f in e.errors {
                    errs.push(format!("auth_configs[{i}].{}", f.field), f.message);
                }
            }
        }
        let existing_auth: HashMap<String, _> = self
            .list_auth_configs(project_id)
            .await?
            .into_iter()
            .map(|a| (a.name.clone(), a))
            .collect();
        let existing_tools: HashMap<String, _> = self
            .list_tools(project_id)
            .await?
            .into_iter()
            .map(|t| (t.name.clone(), t))
            .collect();

        let mut report = BundleReport::default();
        let mut auth_ids: HashMap<String, Uuid> = existing_auth
            .iter()
            .map(|(n, a)| (n.clone(), a.id))
            .collect();
        let mut tx = self.pool.begin().await?;

        for a in auths {
            match existing_auth.get(&a.name) {
                Some(cur) if cur.kind == a.kind && cur.secret_ref == a.secret_ref => {
                    report.auth_kept.push(a.name.clone());
                }
                Some(_) if policy == ConflictPolicy::Fail => {
                    return Err(StoreError::Conflict(format!(
                        "an auth config named `{}` already exists with different settings",
                        a.name
                    )));
                }
                Some(_) if policy == ConflictPolicy::Skip => report.auth_kept.push(a.name.clone()),
                Some(cur) => {
                    sqlx::query(
                        "UPDATE auth_configs SET kind = ?, secret_ref = ?, updated_at = ? WHERE id = ?",
                    )
                    .bind(to_json(&a.kind))
                    .bind(a.secret_ref.as_str())
                    .bind(now())
                    .bind(cur.id.to_string())
                    .execute(&mut *tx)
                    .await?;
                    report.auth_updated.push(a.name.clone());
                }
                None => {
                    let id = Uuid::new_v4();
                    let ts = now();
                    sqlx::query(
                        "INSERT INTO auth_configs (id, project_id, name, kind, secret_ref, created_at, updated_at) \
                         VALUES (?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(id.to_string())
                    .bind(project_id.to_string())
                    .bind(&a.name)
                    .bind(to_json(&a.kind))
                    .bind(a.secret_ref.as_str())
                    .bind(ts)
                    .bind(ts)
                    .execute(&mut *tx)
                    .await?;
                    auth_ids.insert(a.name.clone(), id);
                    report.auth_created.push(a.name.clone());
                }
            }
        }

        let mut resolved = Vec::with_capacity(tools.len());
        for (i, (t, auth)) in tools.iter().enumerate() {
            let mut t = t.clone();
            let ToolKind::Http(h) = &mut t.kind;
            h.auth_ref = match auth {
                None => None,
                Some(name) => match auth_ids.get(name) {
                    Some(id) => Some(*id),
                    None => {
                        errs.push(
                            format!("tools[{i}].auth"),
                            format!("no auth config named `{name}`"),
                        );
                        None
                    }
                },
            };
            if let Err(e) = t.validate() {
                for f in e.errors {
                    errs.push(format!("tools[{i}].{}", f.field), f.message);
                }
            }
            resolved.push(t);
        }
        if !errs.is_empty() {
            return Err(errs.into());
        }

        for t in resolved {
            match existing_tools.get(&t.name) {
                Some(_) if policy == ConflictPolicy::Fail => {
                    return Err(StoreError::Conflict(format!(
                        "a tool named `{}` already exists in this project",
                        t.name
                    )));
                }
                Some(_) if policy == ConflictPolicy::Skip => {
                    report.tools_skipped.push(t.name.clone())
                }
                Some(cur) => {
                    let mut cur = cur.clone();
                    apply_input(&mut cur, &t);
                    write_tool(&mut *tx, &cur).await?;
                    report.tools_updated.push(t.name.clone());
                }
                None => {
                    let tool = new_tool_def(project_id, &t, forge_core::Source::Manual);
                    insert_tool(&mut *tx, &tool).await?;
                    report.tools_created.push(t.name.clone());
                }
            }
        }
        tx.commit().await?;
        Ok(report)
    }
}
