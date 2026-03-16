use serde_json::{json, Value};

use crate::app_config::{AppType, McpServer};
use crate::cli::i18n::texts;
use crate::cli::tui::form::strip_common_config_from_settings;
use crate::error::AppError;
use crate::provider::Provider;
use crate::services::{McpService, PromptService, ProviderService};
use crate::settings::{set_webdav_sync_settings, WebDavSyncSettings};

use super::super::app::{EditorSubmit, Overlay, TextViewState, ToastKind};
use super::super::data::{load_state, UiData};
use super::super::form::FormState;
use super::helpers::run_external_editor_for_current_editor;
use super::RuntimeActionContext;

pub(super) fn open_external(ctx: &mut RuntimeActionContext<'_>) -> Result<(), AppError> {
    ctx.terminal.with_terminal_restored(|| {
        run_external_editor_for_current_editor(ctx.app, crate::cli::editor::open_external_editor)
    })
}

pub(super) fn submit(
    ctx: &mut RuntimeActionContext<'_>,
    submit: EditorSubmit,
    content: String,
) -> Result<(), AppError> {
    match submit {
        EditorSubmit::PromptEdit { id } => submit_prompt_edit(ctx, id, content),
        EditorSubmit::ProviderFormApplyJson => submit_provider_form_apply_json(ctx, content),
        EditorSubmit::ProviderFormApplyCodexAuth => {
            submit_provider_form_apply_codex_auth(ctx, content)
        }
        EditorSubmit::ProviderFormApplyCodexConfigToml => {
            submit_provider_form_apply_codex_config_toml(ctx, content)
        }
        EditorSubmit::ProviderAdd => submit_provider_add(ctx, content),
        EditorSubmit::ProviderEdit { id } => submit_provider_edit(ctx, id, content),
        EditorSubmit::McpAdd => submit_mcp_add(ctx, content),
        EditorSubmit::McpEdit { id } => submit_mcp_edit(ctx, id, content),
        EditorSubmit::ConfigCommonSnippet { app_type } => {
            submit_config_common_snippet(ctx, app_type, content)
        }
        EditorSubmit::ConfigWebDavSettings => submit_webdav_settings(ctx, content),
    }
}

fn submit_prompt_edit(
    ctx: &mut RuntimeActionContext<'_>,
    id: String,
    content: String,
) -> Result<(), AppError> {
    let state = load_state()?;
    let prompts = PromptService::get_prompts(&state, ctx.app.app_type.clone())?;
    let Some(mut prompt) = prompts.get(&id).cloned() else {
        ctx.app
            .push_toast(texts::tui_toast_prompt_not_found(&id), ToastKind::Error);
        return Ok(());
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    prompt.content = content;
    prompt.updated_at = Some(timestamp);

    if let Err(err) = PromptService::upsert_prompt(&state, ctx.app.app_type.clone(), &id, prompt) {
        ctx.app.push_toast(err.to_string(), ToastKind::Error);
        return Ok(());
    }

    ctx.app.editor = None;
    ctx.app
        .push_toast(texts::tui_toast_prompt_edit_finished(), ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

fn submit_provider_form_apply_json(
    ctx: &mut RuntimeActionContext<'_>,
    content: String,
) -> Result<(), AppError> {
    let mut settings_value: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };

    if !settings_value.is_object() {
        ctx.app
            .push_toast(texts::tui_toast_json_must_be_object(), ToastKind::Error);
        return Ok(());
    }

    let provider_value = match ctx.app.form.as_ref() {
        Some(FormState::ProviderAdd(form)) => {
            if form.include_common_config {
                if let Err(err) = strip_common_config_from_settings(
                    &form.app_type,
                    &mut settings_value,
                    &ctx.data.config.common_snippet,
                ) {
                    ctx.app.push_toast(err, ToastKind::Error);
                    return Ok(());
                }
            }

            let mut provider_value = form.to_provider_json_value();
            if let Some(obj) = provider_value.as_object_mut() {
                obj.insert("settingsConfig".to_string(), settings_value.clone());
            }
            Some(provider_value)
        }
        _ => None,
    };

    if let Some(provider_value) = provider_value {
        let apply_result = match ctx.app.form.as_mut() {
            Some(FormState::ProviderAdd(form)) => {
                form.apply_provider_json_value_to_fields(provider_value)
            }
            _ => Ok(()),
        };

        if let Err(err) = apply_result {
            ctx.app.push_toast(err, ToastKind::Error);
            return Ok(());
        }
    }
    ctx.app.editor = None;
    Ok(())
}

fn submit_provider_form_apply_codex_auth(
    ctx: &mut RuntimeActionContext<'_>,
    content: String,
) -> Result<(), AppError> {
    let auth_value: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };

    if !auth_value.is_object() {
        ctx.app
            .push_toast(texts::tui_toast_json_must_be_object(), ToastKind::Error);
        return Ok(());
    }

    let provider_value = match ctx.app.form.as_ref() {
        Some(FormState::ProviderAdd(form)) => {
            let mut provider_value = form.to_provider_json_value();
            if let Some(settings_value) = provider_value
                .as_object_mut()
                .and_then(|obj| obj.get_mut("settingsConfig"))
            {
                if !settings_value.is_object() {
                    *settings_value = json!({});
                }
                if let Some(settings_obj) = settings_value.as_object_mut() {
                    settings_obj.insert("auth".to_string(), auth_value);
                }
            }
            Some(provider_value)
        }
        _ => None,
    };

    if let Some(provider_value) = provider_value {
        let apply_result = match ctx.app.form.as_mut() {
            Some(FormState::ProviderAdd(form)) => {
                form.apply_provider_json_value_to_fields(provider_value)
            }
            _ => Ok(()),
        };

        if let Err(err) = apply_result {
            ctx.app.push_toast(err, ToastKind::Error);
            return Ok(());
        }
    }

    ctx.app.editor = None;
    Ok(())
}

fn submit_provider_form_apply_codex_config_toml(
    ctx: &mut RuntimeActionContext<'_>,
    content: String,
) -> Result<(), AppError> {
    use toml_edit::DocumentMut;

    let config_text = if content.trim().is_empty() {
        String::new()
    } else {
        let doc: DocumentMut = match content.parse() {
            Ok(doc) => doc,
            Err(e) => {
                ctx.app.push_toast(
                    texts::common_config_snippet_invalid_toml(&e.to_string()),
                    ToastKind::Error,
                );
                return Ok(());
            }
        };
        doc.to_string()
    };

    let provider_value = match ctx.app.form.as_ref() {
        Some(FormState::ProviderAdd(form)) => {
            let mut provider_value = form.to_provider_json_value();
            if let Some(settings_value) = provider_value
                .as_object_mut()
                .and_then(|obj| obj.get_mut("settingsConfig"))
            {
                if !settings_value.is_object() {
                    *settings_value = json!({});
                }
                if let Some(settings_obj) = settings_value.as_object_mut() {
                    settings_obj.insert("config".to_string(), Value::String(config_text));
                }
            }
            Some(provider_value)
        }
        _ => None,
    };

    if let Some(provider_value) = provider_value {
        let apply_result = match ctx.app.form.as_mut() {
            Some(FormState::ProviderAdd(form)) => {
                form.apply_provider_json_value_to_fields(provider_value)
            }
            _ => Ok(()),
        };

        if let Err(err) = apply_result {
            ctx.app.push_toast(err, ToastKind::Error);
            return Ok(());
        }
    }

    ctx.app.editor = None;
    Ok(())
}

fn submit_provider_add(
    ctx: &mut RuntimeActionContext<'_>,
    content: String,
) -> Result<(), AppError> {
    let mut provider: Provider = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };

    if provider.name.trim().is_empty() {
        ctx.app.push_toast(
            texts::tui_toast_provider_add_missing_fields(),
            ToastKind::Warning,
        );
        return Ok(());
    }

    let state = load_state()?;
    let existing_ids = {
        let config = state.config.read().map_err(AppError::from)?;
        config
            .get_manager(&ctx.app.app_type)
            .map(|manager| manager.providers.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let Some(provider_id) = crate::cli::tui::form::resolve_provider_id_for_submit(
        &provider.name,
        &provider.id,
        &existing_ids,
    ) else {
        ctx.app.push_toast(
            texts::tui_toast_provider_add_missing_fields(),
            ToastKind::Warning,
        );
        return Ok(());
    };
    provider.id = provider_id;

    match ProviderService::add(&state, ctx.app.app_type.clone(), provider) {
        Ok(true) => {
            ctx.app.editor = None;
            ctx.app.form = None;
            ctx.app
                .push_toast(texts::tui_toast_provider_add_finished(), ToastKind::Success);
            *ctx.data = UiData::load(&ctx.app.app_type)?;
        }
        Ok(false) => {
            ctx.app
                .push_toast(texts::tui_toast_provider_add_failed(), ToastKind::Error);
        }
        Err(err) => {
            ctx.app.push_toast(err.to_string(), ToastKind::Error);
        }
    }

    Ok(())
}

fn submit_provider_edit(
    ctx: &mut RuntimeActionContext<'_>,
    id: String,
    content: String,
) -> Result<(), AppError> {
    let mut provider: Provider = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };
    provider.id = id.clone();

    if provider.name.trim().is_empty() {
        ctx.app
            .push_toast(texts::tui_toast_provider_missing_name(), ToastKind::Warning);
        return Ok(());
    }

    let state = load_state()?;
    if let Err(err) = ProviderService::update(&state, ctx.app.app_type.clone(), provider) {
        ctx.app.push_toast(err.to_string(), ToastKind::Error);
        return Ok(());
    }

    ctx.app.editor = None;
    ctx.app.form = None;
    ctx.app.push_toast(
        texts::tui_toast_provider_edit_finished(),
        ToastKind::Success,
    );
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

fn submit_mcp_add(ctx: &mut RuntimeActionContext<'_>, content: String) -> Result<(), AppError> {
    let server: McpServer = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };

    if server.id.trim().is_empty() || server.name.trim().is_empty() {
        ctx.app
            .push_toast(texts::tui_toast_mcp_missing_fields(), ToastKind::Warning);
        return Ok(());
    }

    let state = load_state()?;
    if let Err(err) = McpService::upsert_server(&state, server) {
        ctx.app.push_toast(err.to_string(), ToastKind::Error);
        return Ok(());
    }

    ctx.app.editor = None;
    ctx.app.form = None;
    ctx.app
        .push_toast(texts::tui_toast_mcp_upserted(), ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

fn submit_mcp_edit(
    ctx: &mut RuntimeActionContext<'_>,
    id: String,
    content: String,
) -> Result<(), AppError> {
    let mut server: McpServer = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            ctx.app.push_toast(
                texts::tui_toast_invalid_json(&e.to_string()),
                ToastKind::Error,
            );
            return Ok(());
        }
    };
    server.id = id.clone();

    if server.name.trim().is_empty() {
        ctx.app
            .push_toast(texts::tui_toast_mcp_missing_fields(), ToastKind::Warning);
        return Ok(());
    }

    let state = load_state()?;
    if let Err(err) = McpService::upsert_server(&state, server) {
        ctx.app.push_toast(err.to_string(), ToastKind::Error);
        return Ok(());
    }

    ctx.app.editor = None;
    ctx.app.form = None;
    ctx.app
        .push_toast(texts::tui_toast_mcp_upserted(), ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

fn submit_config_common_snippet(
    ctx: &mut RuntimeActionContext<'_>,
    app_type: AppType,
    content: String,
) -> Result<(), AppError> {
    let edited = content.trim().to_string();
    let (next_snippet, toast) = if edited.is_empty() {
        (None, texts::common_config_snippet_cleared())
    } else if matches!(app_type, AppType::Codex) {
        let doc: toml_edit::DocumentMut = match edited.parse() {
            Ok(v) => v,
            Err(e) => {
                ctx.app.push_toast(
                    texts::common_config_snippet_invalid_toml(&e.to_string()),
                    ToastKind::Error,
                );
                return Ok(());
            }
        };
        let canonical = doc.to_string().trim().to_string();
        (Some(canonical), texts::common_config_snippet_saved())
    } else {
        let value: Value = match serde_json::from_str(&edited) {
            Ok(v) => v,
            Err(e) => {
                ctx.app.push_toast(
                    texts::common_config_snippet_invalid_json(&e.to_string()),
                    ToastKind::Error,
                );
                return Ok(());
            }
        };

        if !value.is_object() {
            ctx.app
                .push_toast(texts::common_config_snippet_not_object(), ToastKind::Error);
            return Ok(());
        }

        let pretty = match serde_json::to_string_pretty(&value) {
            Ok(v) => v,
            Err(e) => {
                ctx.app.push_toast(
                    texts::failed_to_serialize_json(&e.to_string()),
                    ToastKind::Error,
                );
                return Ok(());
            }
        };

        (Some(pretty), texts::common_config_snippet_saved())
    };

    let state = load_state()?;
    let service_result = if let Some(snippet) = next_snippet.clone() {
        ProviderService::set_common_config_snippet(&state, app_type.clone(), Some(snippet))
    } else {
        ProviderService::clear_common_config_snippet(&state, app_type.clone())
    };
    if let Err(err) = service_result {
        ctx.app.push_toast(err.to_string(), ToastKind::Error);
        return Ok(());
    }

    ctx.app.editor = None;
    ctx.app.push_toast(toast, ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;

    let snippet = next_snippet.unwrap_or_else(|| {
        texts::tui_default_common_snippet_for_app(app_type.as_str()).to_string()
    });
    ctx.app.overlay = Overlay::CommonSnippetView {
        app_type: app_type.clone(),
        view: TextViewState {
            title: texts::tui_common_snippet_title(app_type.as_str()),
            lines: snippet.lines().map(|s| s.to_string()).collect(),
            scroll: 0,
            action: None,
        },
    };
    Ok(())
}

fn submit_webdav_settings(
    ctx: &mut RuntimeActionContext<'_>,
    content: String,
) -> Result<(), AppError> {
    let edited = content.trim();
    if edited.is_empty() {
        set_webdav_sync_settings(None)?;
        ctx.app.editor = None;
        ctx.app.push_toast(
            texts::tui_toast_webdav_settings_cleared(),
            ToastKind::Success,
        );
        *ctx.data = UiData::load(&ctx.app.app_type)?;
        return Ok(());
    }

    let cfg: WebDavSyncSettings = serde_json::from_str(edited)
        .map_err(|e| AppError::Message(texts::tui_toast_invalid_json(&e.to_string())))?;
    set_webdav_sync_settings(Some(cfg))?;

    ctx.app.editor = None;
    ctx.app
        .push_toast(texts::tui_toast_webdav_settings_saved(), ToastKind::Success);
    *ctx.data = UiData::load(&ctx.app.app_type)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use serial_test::serial;
    use tempfile::TempDir;

    use super::*;
    use crate::cli::tui::app::{App, Toast};
    use crate::cli::tui::runtime_systems::RequestTracker;
    use crate::cli::tui::terminal::TuiTerminal;

    struct EnvGuard {
        old_home: Option<OsString>,
        old_userprofile: Option<OsString>,
    }

    impl EnvGuard {
        fn set_home(home: &Path) -> Self {
            let old_home = std::env::var_os("HOME");
            let old_userprofile = std::env::var_os("USERPROFILE");
            std::env::set_var("HOME", home);
            std::env::set_var("USERPROFILE", home);
            Self {
                old_home,
                old_userprofile,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.old_userprofile {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
            }
        }
    }

    fn runtime_ctx(
        app_type: AppType,
    ) -> (
        TempDir,
        EnvGuard,
        TuiTerminal,
        App,
        UiData,
        RequestTracker,
        RequestTracker,
        RequestTracker,
    ) {
        let temp_home = TempDir::new().expect("create temp home");
        let env = EnvGuard::set_home(temp_home.path());

        let terminal = TuiTerminal::new_for_test().expect("create test terminal");
        let app = App::new(Some(app_type.clone()));
        let data = UiData::load(&app_type).expect("load ui data");
        (
            temp_home,
            env,
            terminal,
            app,
            data,
            RequestTracker::default(),
            RequestTracker::default(),
            RequestTracker::default(),
        )
    }

    #[test]
    #[serial]
    fn submit_provider_add_generates_id_when_name_is_valid() {
        let (
            _temp_home,
            _env,
            mut terminal,
            mut app,
            mut data,
            mut proxy_loading,
            mut webdav_loading,
            mut update_check,
        ) = runtime_ctx(AppType::Claude);

        let mut ctx = RuntimeActionContext {
            terminal: &mut terminal,
            app: &mut app,
            data: &mut data,
            speedtest_req_tx: None,
            stream_check_req_tx: None,
            skills_req_tx: None,
            proxy_req_tx: None,
            proxy_loading: &mut proxy_loading,
            local_env_req_tx: None,
            webdav_req_tx: None,
            webdav_loading: &mut webdav_loading,
            update_req_tx: None,
            update_check: &mut update_check,
            model_fetch_req_tx: None,
        };

        submit_provider_add(
            &mut ctx,
            r#"{"id":"","name":"Provider One","settingsConfig":{"env":{"ANTHROPIC_BASE_URL":"https://example.com"}}}"#
                .to_string(),
        )
        .expect("submit should succeed");

        let refreshed = UiData::load(&AppType::Claude).expect("reload ui data");
        assert!(
            refreshed
                .providers
                .rows
                .iter()
                .any(|row| row.id == "provider-one"),
            "runtime submit should auto-generate and persist an id"
        );
    }

    #[test]
    #[serial]
    fn submit_provider_add_rejects_name_that_cannot_generate_id() {
        let (
            _temp_home,
            _env,
            mut terminal,
            mut app,
            mut data,
            mut proxy_loading,
            mut webdav_loading,
            mut update_check,
        ) = runtime_ctx(AppType::Claude);

        let mut ctx = RuntimeActionContext {
            terminal: &mut terminal,
            app: &mut app,
            data: &mut data,
            speedtest_req_tx: None,
            stream_check_req_tx: None,
            skills_req_tx: None,
            proxy_req_tx: None,
            proxy_loading: &mut proxy_loading,
            local_env_req_tx: None,
            webdav_req_tx: None,
            webdav_loading: &mut webdav_loading,
            update_req_tx: None,
            update_check: &mut update_check,
            model_fetch_req_tx: None,
        };

        submit_provider_add(
            &mut ctx,
            r#"{"id":"","name":"!!!","settingsConfig":{"env":{"ANTHROPIC_BASE_URL":"https://example.com"}}}"#
                .to_string(),
        )
        .expect("submit should return without crashing");

        let refreshed = UiData::load(&AppType::Claude).expect("reload ui data");
        assert!(
            refreshed.providers.rows.is_empty(),
            "runtime submit should refuse names that still yield an empty id"
        );
        assert!(matches!(
            ctx.app.toast.as_ref(),
            Some(Toast {
                kind: ToastKind::Warning,
                ..
            })
        ));
    }

    #[test]
    #[serial]
    fn submit_provider_form_apply_json_keeps_common_snippet_out_of_raw_submit_payload() {
        let (
            _temp_home,
            _env,
            mut terminal,
            mut app,
            mut data,
            mut proxy_loading,
            mut webdav_loading,
            mut update_check,
        ) = runtime_ctx(AppType::Claude);

        data.config.common_snippet = r#"{
            "alwaysThinkingEnabled": false,
            "env": {
                "COMMON_FLAG": "1"
            }
        }"#
        .to_string();

        let mut form = crate::cli::tui::form::ProviderAddFormState::new(AppType::Claude);
        form.id.set("p1");
        form.name.set("Provider One");
        form.include_common_config = true;
        form.claude_base_url.set("https://provider.example");
        app.form = Some(FormState::ProviderAdd(form));

        let mut ctx = RuntimeActionContext {
            terminal: &mut terminal,
            app: &mut app,
            data: &mut data,
            speedtest_req_tx: None,
            stream_check_req_tx: None,
            skills_req_tx: None,
            proxy_req_tx: None,
            proxy_loading: &mut proxy_loading,
            local_env_req_tx: None,
            webdav_req_tx: None,
            webdav_loading: &mut webdav_loading,
            update_req_tx: None,
            update_check: &mut update_check,
            model_fetch_req_tx: None,
        };

        submit_provider_form_apply_json(
            &mut ctx,
            r#"{
                "alwaysThinkingEnabled": false,
                "env": {
                    "ANTHROPIC_BASE_URL": "https://edited.example",
                    "COMMON_FLAG": "1",
                    "EXTRA_FIELD": "kept"
                }
            }"#
            .to_string(),
        )
        .expect("apply should succeed");

        let FormState::ProviderAdd(form) = ctx
            .app
            .form
            .as_ref()
            .expect("provider form should remain open")
        else {
            panic!("expected provider form");
        };
        let settings = form
            .to_provider_json_value()
            .get("settingsConfig")
            .cloned()
            .expect("settingsConfig should exist");

        assert!(
            settings.get("alwaysThinkingEnabled").is_none(),
            "applying preview JSON should not persist top-level common snippet keys into raw form payload"
        );
        assert!(
            settings["env"].get("COMMON_FLAG").is_none(),
            "applying preview JSON should not persist nested common snippet keys into raw form payload"
        );
        assert_eq!(
            settings["env"]["ANTHROPIC_BASE_URL"], "https://edited.example",
            "provider-specific edits from the preview editor should still be preserved"
        );
        assert_eq!(
            settings["env"]["EXTRA_FIELD"], "kept",
            "non-common keys introduced in the preview editor should still be preserved"
        );
    }
}
