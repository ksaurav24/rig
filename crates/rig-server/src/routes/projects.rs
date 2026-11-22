use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use rig_api::{
    CreateProjectRequest, ErrorBody, Project, ProjectDatabasesOnDelete, UpdateProjectRequest,
};
use serde::Deserialize;
use sqlx::Row;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{Admin, Auth, Developer, Viewer},
    databases, deploy,
    error::{ApiError, ApiResult},
    proxy,
    routes::auth::{slugify, validate_name},
    state::{AppState, new_id, now},
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_projects, create_project))
        .routes(routes!(update_project, delete_project))
}

fn project_from_row(r: &sqlx::sqlite::SqliteRow) -> Project {
    Project {
        id: r.get("id"),
        name: r.get("name"),
        slug: r.get("slug"),
        description: r.get("description"),
        app_count: u32::try_from(r.get::<i64, _>("app_count")).unwrap_or(0),
        created_at: r.get("created_at"),
    }
}

/// Every project in the organization, with how many apps each holds.
#[utoipa::path(get, path = "/api/v1/projects", tag = "projects",
    responses((status = 200, body = Vec<Project>)))]
async fn list_projects(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
) -> ApiResult<Json<Vec<Project>>> {
    let rows = sqlx::query(
        "SELECT p.id, p.name, p.slug, p.description, p.created_at,
                (SELECT COUNT(*) FROM apps a WHERE a.project_id = p.id) AS app_count
           FROM projects p WHERE p.org_id = ? ORDER BY p.name COLLATE NOCASE",
    )
    .bind(&auth.org_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(project_from_row).collect()))
}

#[utoipa::path(post, path = "/api/v1/projects", tag = "projects",
    request_body = CreateProjectRequest,
    responses((status = 201, body = Project), (status = 409, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn create_project(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Json(req): Json<CreateProjectRequest>,
) -> ApiResult<(StatusCode, Json<Project>)> {
    let name = validate_name("project name", &req.name, 64)?;
    let description = req.description.trim();
    if description.len() > 500 {
        return Err(ApiError::validation(
            "description must be 500 characters or fewer",
        ));
    }
    let project = Project {
        id: new_id(),
        slug: slugify(&name),
        name,
        description: description.to_string(),
        app_count: 0,
        created_at: now(),
    };
    sqlx::query(
        "INSERT INTO projects (id, org_id, name, slug, description, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&project.id)
    .bind(&auth.org_id)
    .bind(&project.name)
    .bind(&project.slug)
    .bind(&project.description)
    .bind(&project.created_at)
    .execute(&state.db)
    .await?;
    auth.audit(
        &state,
        "project.create",
        Some(&project.id),
        Some(&project.name),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(project)))
}

async fn load_project(state: &AppState, org_id: &str, id: &str) -> ApiResult<Project> {
    sqlx::query(
        "SELECT p.id, p.name, p.slug, p.description, p.created_at,
                (SELECT COUNT(*) FROM apps a WHERE a.project_id = p.id) AS app_count
           FROM projects p WHERE p.id = ? AND p.org_id = ?",
    )
    .bind(id)
    .bind(org_id)
    .fetch_optional(&state.db)
    .await?
    .as_ref()
    .map(project_from_row)
    .ok_or(ApiError::NotFound("project"))
}

/// Rename a project or change its description.
#[utoipa::path(patch, path = "/api/v1/projects/{id}", tag = "projects",
    params(("id" = String, Path)),
    request_body = UpdateProjectRequest,
    responses((status = 200, body = Project), (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn update_project(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProjectRequest>,
) -> ApiResult<Json<Project>> {
    let current = load_project(&state, &auth.org_id, &id).await?;
    let name = match req.name.as_deref() {
        Some(n) => validate_name("project name", n, 64)?,
        None => current.name.clone(),
    };
    let description = match req.description.as_deref() {
        Some(d) => d.trim().to_string(),
        None => current.description.clone(),
    };
    if description.len() > 500 {
        return Err(ApiError::validation(
            "description must be 500 characters or fewer",
        ));
    }
    let taken: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM projects WHERE org_id = ? AND id != ? AND name = ? COLLATE NOCASE",
    )
    .bind(&auth.org_id)
    .bind(&id)
    .bind(&name)
    .fetch_one(&state.db)
    .await?;
    if taken > 0 {
        return Err(ApiError::Conflict(format!(
            "another project is already called {name}"
        )));
    }
    sqlx::query("UPDATE projects SET name = ?, description = ? WHERE id = ? AND org_id = ?")
        .bind(&name)
        .bind(&description)
        .bind(&id)
        .bind(&auth.org_id)
        .execute(&state.db)
        .await?;
    if name != current.name {
        auth.audit(&state, "project.rename", Some(&id), Some(&name))
            .await?;
    }
    load_project(&state, &auth.org_id, &id).await.map(Json)
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct DeleteProjectQuery {
    /// Required when the project has databases: move them to `target`, or
    /// delete them with their data and backups.
    #[param(inline)]
    databases: Option<ProjectDatabasesOnDelete>,
    /// The project that receives the databases when `databases=move`.
    target: Option<String>,
}

/// Delete a project, its apps and their containers. Its databases are moved
/// to another project or deleted, as the caller chooses.
#[utoipa::path(delete, path = "/api/v1/projects/{id}", tag = "projects",
    params(("id" = String, Path), DeleteProjectQuery),
    responses((status = 204), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody),
        (status = 422, body = ErrorBody)))]
async fn delete_project(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(id): Path<String>,
    Query(q): Query<DeleteProjectQuery>,
) -> ApiResult<StatusCode> {
    let name: Option<String> =
        sqlx::query_scalar("SELECT name FROM projects WHERE id = ? AND org_id = ?")
            .bind(&id)
            .bind(&auth.org_id)
            .fetch_optional(&state.db)
            .await?;
    let Some(name) = name else {
        return Err(ApiError::NotFound("project"));
    };

    // Decide what happens to the databases before anything is touched.
    let db_rows = sqlx::query("SELECT id, name, slug, status FROM databases WHERE project_id = ?")
        .bind(&id)
        .fetch_all(&state.db)
        .await?;
    let move_to = match (db_rows.is_empty(), q.databases) {
        (true, _) | (false, Some(ProjectDatabasesOnDelete::Delete)) => None,
        (false, None) => {
            return Err(ApiError::Conflict(format!(
                "this project has {} database{}. Choose whether to move {} to another project or delete {}",
                db_rows.len(),
                if db_rows.len() == 1 { "" } else { "s" },
                if db_rows.len() == 1 { "it" } else { "them" },
                if db_rows.len() == 1 { "it" } else { "them" }
            )));
        }
        (false, Some(ProjectDatabasesOnDelete::Move)) => {
            let Some(target) = q.target.as_deref().filter(|t| *t != id) else {
                return Err(ApiError::validation(
                    "choose the project the databases move to",
                ));
            };
            let target = load_project(&state, &auth.org_id, target).await?;
            for r in &db_rows {
                let (name, slug): (String, String) = (r.get("name"), r.get("slug"));
                let clash: i64 = sqlx::query_scalar(
                    "SELECT (SELECT COUNT(*) FROM databases WHERE project_id = ?1 AND (slug = ?2 OR name = ?3))
                          + (SELECT COUNT(*) FROM apps WHERE project_id = ?1 AND slug = ?2)",
                )
                .bind(&target.id)
                .bind(&slug)
                .bind(&name)
                .fetch_one(&state.db)
                .await?;
                if clash > 0 {
                    return Err(ApiError::Conflict(format!(
                        "{} already has a service called {name}. Rename one of them first",
                        target.name
                    )));
                }
            }
            Some(target)
        }
    };

    let busy: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM deployments d JOIN apps a ON a.id = d.app_id
          WHERE a.project_id = ? AND d.status IN ('queued', 'building', 'deploying')",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await?;
    if busy > 0 {
        return Err(ApiError::Conflict(
            "a deployment in this project is still in progress. Wait for it to finish, then delete"
                .into(),
        ));
    }

    let apps: Vec<String> = sqlx::query_scalar("SELECT id FROM apps WHERE project_id = ?")
        .bind(&id)
        .fetch_all(&state.db)
        .await?;
    // Containers go first: once the rows are gone nothing would remember them.
    for app_id in &apps {
        deploy::remove_app_containers(&state, app_id)
            .await
            .map_err(|e| {
                ApiError::Unavailable(format!(
                    "could not remove the project's containers ({e:#}). Nothing was deleted"
                ))
            })?;
        deploy::remove_app_volumes(&state, app_id)
            .await
            .map_err(|e| {
                ApiError::Unavailable(format!(
                    "could not remove the project's volumes ({e:#}). Nothing was deleted"
                ))
            })?;
    }
    let starting: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM databases WHERE project_id = ? AND status = 'creating'",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await?;
    if starting > 0 {
        return Err(ApiError::Conflict(
            "a database in this project is still starting. Wait for it to finish, then delete"
                .into(),
        ));
    }
    if let Some(target) = &move_to {
        for r in &db_rows {
            let db_id: String = r.get("id");
            let running = r.get::<String, _>("status") == "running";
            databases::remove_containers(&state, &db_id)
                .await
                .map_err(|e| {
                    ApiError::Unavailable(format!(
                        "could not move the project's databases ({e:#}). The project was not deleted"
                    ))
                })?;
            // A running database comes back up on the new project's network.
            sqlx::query(
                "UPDATE databases SET project_id = ?, container_id = NULL, host_port = NULL,
                        status = CASE WHEN ? THEN 'creating' ELSE status END
                  WHERE id = ?",
            )
            .bind(&target.id)
            .bind(running)
            .bind(&db_id)
            .execute(&state.db)
            .await?;
            if running {
                tokio::spawn(databases::provision(state.clone(), db_id.clone()));
            }
            auth.audit(&state, "database.move", Some(&db_id), Some(&target.name))
                .await?;
        }
    }
    databases::destroy_project(&state, &id).await.map_err(|e| {
        ApiError::Unavailable(format!(
            "could not remove the project's databases ({e:#}). The project was not deleted"
        ))
    })?;
    sqlx::query("DELETE FROM projects WHERE id = ? AND org_id = ?")
        .bind(&id)
        .bind(&auth.org_id)
        .execute(&state.db)
        .await?;
    if let Err(e) = proxy::sync(&state).await {
        tracing::warn!(error = %format!("{e:#}"), "proxy not updated after deleting a project");
    }
    auth.audit(&state, "project.delete", Some(&id), Some(&name))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
