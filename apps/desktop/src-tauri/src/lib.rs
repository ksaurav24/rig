//! Rig Desktop native Rust core (Tauri 2).
//!
//! Responsibilities:
//! - Multi-server registry & read-only cache (`registry`)
//! - OS Keychain token storage (`keychain`)
//! - Per-server `/api/v1/events` SSE lifecycle + OS notifications (`connection`)
//! - High-throughput Rust log ring buffer, ANSI parser & 60Hz batcher (`log_engine`)
//! - Local `.env` parser/diff (`env_diff`), local Git deploy (`local_git`),
//!   and container terminal WebSocket bridge (`terminal`)

pub mod commands;
pub mod connection;
pub mod env_diff;
pub mod github_oauth;
pub mod keychain;
pub mod local_git;
pub mod log_engine;
pub mod registry;
pub mod terminal;

use std::sync::Arc;
use std::time::Instant;

use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::RwLock;

use crate::commands::DesktopState;
use crate::connection::ConnectionManager;
use crate::github_oauth::GithubConnectController;
use crate::keychain::TokenStore;
use crate::log_engine::LogEngine;
use crate::registry::ServerRegistry;
use crate::terminal::TerminalManager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let started_at = Instant::now();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,rig_desktop_lib=debug".into()),
        )
        .init();

    let registry = ServerRegistry::load(ServerRegistry::default_path())
        .unwrap_or_else(|e| {
            tracing::warn!("loading server registry failed, starting fresh: {e:#}");
            ServerRegistry::load(ServerRegistry::default_path().with_extension("recovery.json"))
                .expect("creating empty registry")
        });

    let initial_servers = registry.list().to_vec();
    let registry = Arc::new(RwLock::new(registry));
    let tokens = Arc::new(TokenStore::default());
    let connections = ConnectionManager::new();
    let logs = LogEngine::new();
    let terminals = TerminalManager::new();
    let github_connect = Arc::new(GithubConnectController::new());

    let boot_registry = Arc::clone(&registry);
    let boot_tokens = Arc::clone(&tokens);
    let boot_connections = Arc::clone(&connections);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(DesktopState {
            started_at,
            registry,
            tokens,
            connections,
            logs,
            terminals,
            github_connect,
        })
        .setup(move |app| {
            // System tray / menu-bar icon with Show & Quit actions (§8).
            if let Ok(show_item) = MenuItem::with_id(app, "show", "Show Rig Desktop", true, None::<&str>)
                && let Ok(quit_item) = MenuItem::with_id(app, "quit", "Quit Rig", true, None::<&str>)
                && let Ok(menu) = Menu::with_items(app, &[&show_item, &quit_item])
            {
                let mut builder = TrayIconBuilder::with_id("rig-tray")
                    .tooltip("Rig Desktop")
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            let app = tray.app_handle();
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                        }
                    });
                if let Some(icon) = app.default_window_icon() {
                    builder = builder.icon(icon.clone());
                }
                let _ = builder.build(app);
            }

            // Start lightweight /api/v1/events SSE watchers for every paired server.
            let handle = app.handle().clone();
            let reminder_handle = app.handle().clone();
            let reminder_registry = Arc::clone(&boot_registry);
            tauri::async_runtime::spawn(async move {
                for entry in initial_servers {
                    boot_connections
                        .ensure_watcher(
                            handle.clone(),
                            entry,
                            Arc::clone(&boot_tokens),
                            Arc::clone(&boot_registry),
                        )
                        .await;
                }
            });

            // Periodic snapshot backup reminder ticker: checks shortly after startup, then every 15 minutes.
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                let mut ticker = tokio::time::interval(std::time::Duration::from_secs(15 * 60));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    ticker.tick().await;
                    let now = chrono::Utc::now();
                    let due_servers: Vec<(String, String, u32, Option<String>)> = {
                        let reg = reminder_registry.read().await;
                        reg.list()
                            .iter()
                            .filter(|s| s.backup_reminder.should_notify(now))
                            .map(|s| {
                                (
                                    s.id.clone(),
                                    s.name.clone(),
                                    s.backup_reminder.interval_days,
                                    s.backup_reminder.last_snapshot_at.clone(),
                                )
                            })
                            .collect()
                    };
                    for (server_id, server_name, interval_days, last_snapshot_at) in due_servers {
                        let body = match &last_snapshot_at {
                            Some(_) => format!(
                                "It has been more than {interval_days} day(s) since your last snapshot backup of {server_name}."
                            ),
                            None => format!(
                                "Time to download your first full snapshot backup of {server_name}."
                            ),
                        };
                        let _ = reminder_handle
                            .notification()
                            .builder()
                            .title(format!("Rig Backup Reminder · {server_name}"))
                            .body(&body)
                            .show();
                        let _ = reminder_handle.emit(
                            "rig://backup-reminder-due",
                            serde_json::json!({
                                "server_id": server_id,
                                "server_name": server_name,
                                "interval_days": interval_days,
                                "last_snapshot_at": last_snapshot_at,
                            }),
                        );
                        let _ = reminder_registry
                            .write()
                            .await
                            .mark_reminder_notified(&server_id);
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_servers,
            commands::probe_server,
            commands::init_server_setup,
            commands::pair_server,
            commands::login_server,
            commands::select_server,
            commands::remove_server,
            commands::reorder_servers,
            commands::update_server,
            commands::get_server_snapshot,
            commands::get_app_detail,
            commands::create_project,
            commands::update_project,
            commands::delete_project,
            commands::create_app,
            commands::update_app,
            commands::delete_app,
            commands::app_action,
            commands::rollback_deployment,
            commands::cancel_deployment,
            commands::add_domain,
            commands::delete_domain,
            commands::list_database_engines,
            commands::create_database,
            commands::delete_database,
            commands::database_action,
            commands::reveal_database,
            commands::list_backups,
            commands::create_backup,
            commands::restore_backup,
            commands::delete_backup,
            commands::get_server_settings,
            commands::update_organization,
            commands::add_member,
            commands::update_member,
            commands::remove_member,
            commands::reset_member_password,
            commands::create_alert_channel,
            commands::test_alert_channel,
            commands::delete_alert_channel,
            commands::create_token,
            commands::revoke_token,
            commands::get_github_status,
            commands::list_github_repos,
            commands::list_github_branches,
            commands::connect_github_app,
            commands::cancel_github_connect,
            commands::disconnect_github,
            commands::list_devices,
            commands::create_pairing_code,
            commands::revoke_device,
            commands::start_app_logs,
            commands::start_deploy_logs,
            commands::start_log_benchmark,
            commands::stop_log_stream,
            commands::query_log_buffer,
            commands::get_perf_diagnostics,
            commands::get_app_info,
            commands::check_desktop_updates,
            commands::set_env_var,
            commands::reveal_env_var,
            commands::delete_env_var,
            commands::preview_env_diff,
            commands::apply_env_diff,
            commands::discover_local_repos,
            commands::inspect_local_git,
            commands::deploy_local_git,
            commands::open_terminal,
            commands::write_terminal,
            commands::resize_terminal,
            commands::close_terminal,
            commands::get_metrics,
            commands::get_app_webhook,
            commands::rotate_app_webhook,
            commands::download_backup,
            commands::upload_org_logo,
            commands::delete_org_logo,
            commands::fetch_media_data_url,
            commands::update_profile,
            commands::upload_avatar,
            commands::delete_avatar,
            commands::change_password,
            commands::get_totp_status,
            commands::totp_setup,
            commands::totp_enable,
            commands::totp_disable,
            commands::totp_regenerate_recovery_codes,
            commands::save_text_file,
            commands::export_server_snapshot,
            commands::inspect_server_snapshot,
            commands::restore_server_snapshot,
            commands::set_backup_reminder,
            commands::snooze_backup_reminder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rig Desktop");
}
