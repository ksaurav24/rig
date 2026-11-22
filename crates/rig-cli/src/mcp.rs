//! `rig mcp`: a Model Context Protocol server on stdio.
//!
//! Speaks line-delimited JSON-RPC 2.0. Stdout carries protocol messages and
//! nothing else, so every diagnostic goes to stderr. Tools call the same HTTP
//! API as the rest of the CLI, with the context's API token, so an agent can
//! never do more than that token's role allows. There is deliberately no tool
//! that reveals secret values.

use std::collections::VecDeque;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use rig_api::{App, Deployment, DeploymentStatus, LogLine};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::client::{Client, segment};
use crate::{find_app, short};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const DEPLOY_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const LOG_TIMEOUT: Duration = Duration::from_secs(20);
const DEPLOY_LOG_LINES: usize = 200;
/// A request line longer than this is refused rather than buffered.
const MAX_LINE_BYTES: usize = 1024 * 1024;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

pub async fn serve(api: Client) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let reply = if line.len() > MAX_LINE_BYTES {
            Some(error(Value::Null, INVALID_REQUEST, "request is too large"))
        } else {
            handle(&api, &line).await
        };
        if let Some(reply) = reply {
            let mut bytes = serde_json::to_vec(&reply)?;
            bytes.push(b'\n');
            out.write_all(&bytes).await?;
            out.flush().await?;
        }
    }
    Ok(())
}

/// What one incoming line asks for, decided without touching the network.
#[derive(Debug, PartialEq)]
enum Step {
    /// Nothing to send (a notification).
    Silent,
    Reply(Value),
    Call {
        id: Value,
        tool: &'static Tool,
        args: Map<String, Value>,
    },
}

async fn handle(api: &Client, line: &str) -> Option<Value> {
    match dispatch(line) {
        Step::Silent => None,
        Step::Reply(reply) => Some(reply),
        Step::Call { id, tool, args } => {
            let (payload, is_error) = match run(api, tool.name, &args).await {
                Ok(Outcome { value, failed }) => (value, failed),
                Err(e) => (Value::String(format!("{e:#}")), true),
            };
            let text = match payload {
                Value::String(s) => s,
                other => serde_json::to_string_pretty(&other).unwrap_or_default(),
            };
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "content": [{ "type": "text", "text": text }], "isError": is_error },
            }))
        }
    }
}

fn dispatch(line: &str) -> Step {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Step::Reply(error(
            Value::Null,
            PARSE_ERROR,
            "the line is not valid JSON",
        ));
    };
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        // A response to something we never asked, or not a request at all.
        return match message.get("id") {
            Some(id) if message.get("result").is_none() && message.get("error").is_none() => {
                Step::Reply(error(id.clone(), INVALID_REQUEST, "missing method"))
            }
            None if !message.is_object() => Step::Reply(error(
                Value::Null,
                INVALID_REQUEST,
                "expected a JSON-RPC request object",
            )),
            _ => Step::Silent,
        };
    };
    let Some(id) = message.get("id").cloned() else {
        return Step::Silent;
    };
    let params = message.get("params");

    match method {
        "initialize" => {
            let wanted = params
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str);
            let version = wanted
                .filter(|v| PROTOCOL_VERSIONS.contains(v))
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            Step::Reply(ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "rig", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "Operate a Rig server: inspect projects, apps and databases, deploy, roll back, stop, and manage environment variables. Apps are addressed by name, `project/app`, or id. Secret values are never returned.",
                }),
            ))
        }
        "ping" => Step::Reply(ok(id, json!({}))),
        "tools/list" => Step::Reply(ok(
            id,
            json!({ "tools": TOOLS.iter().map(Tool::describe).collect::<Vec<_>>() }),
        )),
        "tools/call" => {
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(tool) = TOOLS.iter().find(|t| t.name == name) else {
                return Step::Reply(error(id, INVALID_PARAMS, &format!("unknown tool: {name}")));
            };
            let args = match params.and_then(|p| p.get("arguments")) {
                None | Some(Value::Null) => Map::new(),
                Some(Value::Object(map)) => map.clone(),
                Some(_) => {
                    return Step::Reply(error(id, INVALID_PARAMS, "arguments must be an object"));
                }
            };
            match tool.check(&args) {
                Ok(()) => Step::Call { id, tool, args },
                Err(why) => Step::Reply(error(id, INVALID_PARAMS, &why)),
            }
        }
        other => Step::Reply(error(
            id,
            METHOD_NOT_FOUND,
            &format!("method not found: {other}"),
        )),
    }
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Kind {
    Str,
    Bool,
    Int { min: i64, max: i64 },
}

#[derive(Debug, PartialEq)]
struct Param {
    name: &'static str,
    kind: Kind,
    required: bool,
    about: &'static str,
}

#[derive(Debug, PartialEq)]
struct Tool {
    name: &'static str,
    about: &'static str,
    params: &'static [Param],
    read_only: bool,
    destructive: bool,
}

const APP: Param = Param {
    name: "app",
    kind: Kind::Str,
    required: true,
    about: "App name, `project/app` when the name is not unique, or app id.",
};

const fn read(name: &'static str, about: &'static str, params: &'static [Param]) -> Tool {
    Tool {
        name,
        about,
        params,
        read_only: true,
        destructive: false,
    }
}

const fn write(
    name: &'static str,
    about: &'static str,
    params: &'static [Param],
    destructive: bool,
) -> Tool {
    Tool {
        name,
        about,
        params,
        read_only: false,
        destructive,
    }
}

static TOOLS: [Tool; 12] = [
    read(
        "system_status",
        "Who you are signed in as, plus server version, container engine, proxy state, resource usage and app counts.",
        &[],
    ),
    read("list_projects", "List projects.", &[]),
    read(
        "list_apps",
        "List apps with their status, source and domains.",
        &[Param {
            name: "project",
            kind: Kind::Str,
            required: false,
            about: "Only apps in this project (name or id).",
        }],
    ),
    read(
        "get_app",
        "One app's configuration, status, domains and current deployment.",
        &[APP],
    ),
    write(
        "deploy_app",
        "Build and deploy an app. By default waits for the result and returns the final status with the last log lines; a failed deployment leaves the previous version running.",
        &[
            APP,
            Param {
                name: "wait",
                kind: Kind::Bool,
                required: false,
                about: "Wait for the deployment to finish (default true). False returns as soon as it is queued.",
            },
        ],
        false,
    ),
    read(
        "list_deployments",
        "An app's deployments, newest first.",
        &[APP],
    ),
    read(
        "deployment_logs",
        "The last build and deploy log lines of one deployment.",
        &[
            Param {
                name: "deployment_id",
                kind: Kind::Str,
                required: true,
                about: "Deployment id from list_deployments or deploy_app.",
            },
            Param {
                name: "tail",
                kind: Kind::Int { min: 1, max: 1000 },
                required: false,
                about: "How many lines to return (default 200).",
            },
        ],
    ),
    write(
        "rollback_app",
        "Redeploy an earlier image of an app without rebuilding. Defaults to the previous live deployment.",
        &[
            APP,
            Param {
                name: "deployment_id",
                kind: Kind::Str,
                required: false,
                about: "Deployment to roll back to (id or its last 8 characters).",
            },
        ],
        false,
    ),
    write(
        "stop_app",
        "Stop an app's container. The app stays configured and can be deployed again.",
        &[APP],
        true,
    ),
    read(
        "list_env",
        "An app's environment variables. Secret values are masked and cannot be revealed through MCP.",
        &[APP],
    ),
    write(
        "set_env",
        "Create or replace one environment variable. It takes effect on the next deployment.",
        &[
            APP,
            Param {
                name: "key",
                kind: Kind::Str,
                required: true,
                about: "Variable name, e.g. DATABASE_URL.",
            },
            Param {
                name: "value",
                kind: Kind::Str,
                required: true,
                about: "Variable value.",
            },
            Param {
                name: "secret",
                kind: Kind::Bool,
                required: false,
                about: "Store encrypted and masked (default true).",
            },
        ],
        false,
    ),
    read(
        "list_databases",
        "List managed databases with engine, version and status.",
        &[],
    ),
];

impl Tool {
    fn describe(&self) -> Value {
        let mut properties = Map::new();
        for p in self.params {
            let mut schema = match p.kind {
                Kind::Str => json!({ "type": "string" }),
                Kind::Bool => json!({ "type": "boolean" }),
                Kind::Int { min, max } => {
                    json!({ "type": "integer", "minimum": min, "maximum": max })
                }
            };
            schema["description"] = Value::String(p.about.to_string());
            properties.insert(p.name.to_string(), schema);
        }
        let required: Vec<&str> = self
            .params
            .iter()
            .filter(|p| p.required)
            .map(|p| p.name)
            .collect();
        json!({
            "name": self.name,
            "description": self.about,
            "inputSchema": {
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false,
            },
            "annotations": {
                "readOnlyHint": self.read_only,
                "destructiveHint": self.destructive,
                "openWorldHint": false,
            },
        })
    }

    /// Enforces the advertised schema, so `run` can trust the arguments.
    fn check(&self, args: &Map<String, Value>) -> Result<(), String> {
        for key in args.keys() {
            if !self.params.iter().any(|p| p.name == key) {
                return Err(format!("{}: unknown argument `{key}`", self.name));
            }
        }
        for p in self.params {
            let Some(value) = args.get(p.name) else {
                if p.required {
                    return Err(format!("{}: missing argument `{}`", self.name, p.name));
                }
                continue;
            };
            let fits = match p.kind {
                Kind::Str => value
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty() || !p.required),
                Kind::Bool => value.is_boolean(),
                Kind::Int { min, max } => value.as_i64().is_some_and(|n| (min..=max).contains(&n)),
            };
            if !fits {
                let want = match p.kind {
                    Kind::Str => "a non-empty string".to_string(),
                    Kind::Bool => "true or false".to_string(),
                    Kind::Int { min, max } => format!("a whole number from {min} to {max}"),
                };
                return Err(format!("{}: `{}` must be {want}", self.name, p.name));
            }
        }
        Ok(())
    }
}

struct Outcome {
    value: Value,
    /// The call went through but the thing it did failed (a failed deployment).
    failed: bool,
}

impl From<Value> for Outcome {
    fn from(value: Value) -> Self {
        Outcome {
            value,
            failed: false,
        }
    }
}

fn text<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn flag(args: &Map<String, Value>, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
}

async fn app(api: &Client, args: &Map<String, Value>) -> Result<App> {
    find_app(api, text(args, "app").unwrap_or_default()).await
}

async fn run(api: &Client, tool: &str, args: &Map<String, Value>) -> Result<Outcome> {
    let value: Value = match tool {
        "system_status" => {
            // Built field by field: `Me` carries a CSRF token that has no business here.
            let me: rig_api::Me = api.get("/api/v1/auth/me").await?;
            let system: Value = api.get("/api/v1/system").await?;
            json!({ "server": api.base(), "user": me.user, "organization": me.org, "role": me.role, "system": system })
        }
        "list_projects" => api.get("/api/v1/projects").await?,
        "list_apps" => {
            let apps: Vec<App> = api.get("/api/v1/apps").await?;
            let apps: Vec<App> = match text(args, "project") {
                Some(p) if !p.is_empty() => apps
                    .into_iter()
                    .filter(|a| a.project_id == p || a.project_name.eq_ignore_ascii_case(p))
                    .collect(),
                _ => apps,
            };
            serde_json::to_value(apps)?
        }
        "get_app" => serde_json::to_value(app(api, args).await?)?,
        "deploy_app" => {
            let app = app(api, args).await?;
            let dep: Deployment = api
                .post(&format!("/api/v1/apps/{}/deploy", app.id), None::<&()>)
                .await?;
            if !flag(args, "wait", true) {
                return Ok(
                    json!({ "deployment_id": dep.id, "status": "queued", "app": app.name }).into(),
                );
            }
            return follow(api, &app, &dep.id).await;
        }
        "list_deployments" => {
            let app = app(api, args).await?;
            api.get(&format!("/api/v1/apps/{}/deployments", app.id))
                .await?
        }
        "deployment_logs" => {
            let id = text(args, "deployment_id").unwrap_or_default();
            let tail = args.get("tail").and_then(Value::as_u64).unwrap_or(200) as usize;
            let (lines, status) = collect_logs(api, id, tail, LOG_TIMEOUT).await?;
            json!({
                "deployment_id": id,
                "status": status.unwrap_or_else(|| "in progress".into()),
                "logs": lines,
            })
        }
        "rollback_app" => {
            let app = app(api, args).await?;
            let deployments: Vec<Deployment> = api
                .get(&format!("/api/v1/apps/{}/deployments", app.id))
                .await?;
            let target = match text(args, "deployment_id") {
                Some(wanted) => deployments
                    .iter()
                    .find(|d| d.id == wanted || short(&d.id) == wanted)
                    .ok_or_else(|| {
                        anyhow!(
                            "{} has no deployment {wanted}. See list_deployments",
                            app.name
                        )
                    })?,
                None => deployments
                    .iter()
                    .find(|d| d.status == DeploymentStatus::Superseded && d.image_ref.is_some())
                    .ok_or_else(|| {
                        anyhow!("{} has no earlier deployment to roll back to", app.name)
                    })?,
            };
            let dep: Deployment = api
                .post(
                    &format!("/api/v1/deployments/{}/rollback", segment(&target.id)),
                    None::<&()>,
                )
                .await?;
            return follow(api, &app, &dep.id).await;
        }
        "stop_app" => {
            let app = app(api, args).await?;
            api.post(&format!("/api/v1/apps/{}/stop", app.id), None::<&()>)
                .await?
        }
        "list_env" => {
            let app = app(api, args).await?;
            api.get(&format!("/api/v1/apps/{}/env", app.id)).await?
        }
        "set_env" => {
            let app = app(api, args).await?;
            let body = rig_api::SetEnvVarRequest {
                key: text(args, "key").unwrap_or_default().to_string(),
                value: text(args, "value").unwrap_or_default().to_string(),
                is_secret: flag(args, "secret", true),
            };
            let saved: Value = api
                .put(&format!("/api/v1/apps/{}/env", app.id), &body)
                .await?;
            json!({ "saved": saved, "note": "Takes effect on the next deployment." })
        }
        "list_databases" => api.get("/api/v1/databases").await?,
        other => bail!("unknown tool: {other}"),
    };
    Ok(value.into())
}

/// Waits for a deployment and reports how it ended.
async fn follow(api: &Client, app: &App, deployment_id: &str) -> Result<Outcome> {
    let (lines, status) =
        collect_logs(api, deployment_id, DEPLOY_LOG_LINES, DEPLOY_TIMEOUT).await?;
    let status = status.unwrap_or_else(|| "unknown".into());
    let failed = status != "running";
    let mut value =
        json!({ "deployment_id": deployment_id, "app": app.name, "status": status, "logs": lines });
    if failed {
        value["note"] = Value::String(if status == "unknown" {
            "The log stream ended before the deployment finished. Check list_deployments.".into()
        } else {
            "The previous version, if there was one, is still running.".into()
        });
    } else if let Ok(fresh) = api.get::<App>(&format!("/api/v1/apps/{}", app.id)).await {
        value["domains"] = serde_json::to_value(&fresh.domains)?;
    }
    Ok(Outcome { value, failed })
}

/// Reads a deployment's log stream, keeping the last `tail` lines. Returns the
/// final status when the stream reported one before `limit` ran out.
async fn collect_logs(
    api: &Client,
    id: &str,
    tail: usize,
    limit: Duration,
) -> Result<(Vec<String>, Option<String>)> {
    let mut lines: VecDeque<String> = VecDeque::with_capacity(tail.min(256));
    let mut status = None;
    let path = format!("/api/v1/deployments/{}/logs", segment(id));
    let stream = api.events(&path, |event, data| match event {
        "log" => {
            let line = serde_json::from_str::<LogLine>(data)
                .map(|l| l.line)
                .unwrap_or_else(|_| data.to_string());
            if lines.len() == tail {
                lines.pop_front();
            }
            lines.push_back(line);
        }
        "end" => status = Some(data.trim_matches('"').to_string()),
        _ => {}
    });
    // Running out of time is not an error: return what arrived so far.
    if let Ok(result) = tokio::time::timeout(limit, stream).await {
        result?;
    }
    Ok((lines.into(), status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(line: &str) -> Value {
        match dispatch(line) {
            Step::Reply(v) => v,
            other => panic!("expected a reply, got {other:?}"),
        }
    }

    #[test]
    fn initialize_negotiates_the_version() {
        let known = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
        );
        assert_eq!(known["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(known["result"]["serverInfo"]["name"], "rig");
        assert_eq!(known["result"]["capabilities"], json!({ "tools": {} }));
        let unknown = reply(
            r#"{"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
        );
        assert_eq!(unknown["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(unknown["id"], "a");
    }

    #[test]
    fn notifications_and_stray_responses_get_no_reply() {
        assert_eq!(
            dispatch(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Step::Silent
        );
        assert_eq!(
            dispatch(r#"{"jsonrpc":"2.0","method":"no/such/notification"}"#),
            Step::Silent
        );
        assert_eq!(
            dispatch(r#"{"jsonrpc":"2.0","id":4,"result":{}}"#),
            Step::Silent
        );
    }

    #[test]
    fn protocol_errors_use_the_standard_codes() {
        assert_eq!(reply("{nope")["error"]["code"], PARSE_ERROR);
        assert_eq!(reply("{nope")["id"], Value::Null);
        assert_eq!(reply("[1,2]")["error"]["code"], INVALID_REQUEST);
        assert_eq!(
            reply(r#"{"jsonrpc":"2.0","id":2}"#)["error"]["code"],
            INVALID_REQUEST
        );
        assert_eq!(
            reply(r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#)["error"]["code"],
            METHOD_NOT_FOUND
        );
        assert_eq!(
            reply(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#)["result"],
            json!({})
        );
    }

    #[test]
    fn tools_list_is_strict_and_annotated() {
        let listed = reply(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), TOOLS.len());
        for tool in tools {
            assert_eq!(
                tool["inputSchema"]["additionalProperties"], false,
                "{}",
                tool["name"]
            );
            assert!(!tool["description"].as_str().unwrap().is_empty());
        }
        let stop = tools.iter().find(|t| t["name"] == "stop_app").unwrap();
        assert_eq!(stop["annotations"]["destructiveHint"], true);
        assert_eq!(stop["inputSchema"]["required"], json!(["app"]));
        let env = tools.iter().find(|t| t["name"] == "list_env").unwrap();
        assert_eq!(env["annotations"]["readOnlyHint"], true);
        assert!(
            !tools
                .iter()
                .any(|t| t["name"].as_str().unwrap().contains("reveal"))
        );
    }

    fn call(name: &str, args: &str) -> Step {
        dispatch(&format!(
            r#"{{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{{"name":"{name}","arguments":{args}}}}}"#
        ))
    }

    #[test]
    fn tool_arguments_are_validated_before_any_request() {
        let bad = |step: Step| match step {
            Step::Reply(v) => {
                assert_eq!(v["error"]["code"], INVALID_PARAMS, "{v}");
                v["error"]["message"].as_str().unwrap().to_string()
            }
            other => panic!("expected an error, got {other:?}"),
        };
        assert!(bad(call("nope", "{}")).contains("unknown tool"));
        assert!(bad(call("get_app", "{}")).contains("missing argument `app`"));
        assert!(bad(call("get_app", r#"{"app":"  "}"#)).contains("non-empty"));
        assert!(
            bad(call("get_app", r#"{"app":"web","extra":1}"#)).contains("unknown argument `extra`")
        );
        assert!(bad(call("deploy_app", r#"{"app":"web","wait":"yes"}"#)).contains("true or false"));
        assert!(
            bad(call("deployment_logs", r#"{"deployment_id":"d","tail":0}"#)).contains("1 to 1000")
        );
        assert!(
            bad(call(
                "deployment_logs",
                r#"{"deployment_id":"d","tail":1.5}"#
            ))
            .contains("whole number")
        );
        assert!(bad(call("list_apps", "[]")).contains("must be an object"));

        assert!(
            matches!(call("list_apps", "{}"), Step::Call { tool, .. } if tool.name == "list_apps")
        );
        assert!(matches!(call("list_apps", "null"), Step::Call { .. }));
        assert!(matches!(
            call(
                "set_env",
                r#"{"app":"web","key":"A","value":"","secret":false}"#
            ),
            Step::Reply(_)
        ));
        assert!(matches!(
            call("deployment_logs", r#"{"deployment_id":"d","tail":1000}"#),
            Step::Call { .. }
        ));
    }
}
