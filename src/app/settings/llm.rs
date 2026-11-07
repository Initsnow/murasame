use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use slint::ComponentHandle;
use crate::app::{
    SettingsWindow,
    helpers::fetch_models_list,
};

pub fn setup(
    settings_window: &SettingsWindow,
    tokio_rt: &Arc<tokio::runtime::Runtime>,
) {
    let sw_weak_sm = settings_window.as_weak();
    settings_window.on_select_model(move |model_name| {
        if let Some(sw) = sw_weak_sm.upgrade() {
            if sw.get_cfg_api_protocol() == "hermes" {
                sw.set_cfg_hermes_model(model_name);
            } else {
                sw.set_cfg_openai_model(model_name);
            }
        }
    });

    let all_models: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sw_weak_fm = settings_window.as_weak();
    let rt_fm = Arc::clone(&tokio_rt);
    let all_models_fetch = Arc::clone(&all_models);
    settings_window.on_fetch_models_from_api(move |base_url, api_key| {
        let sw_weak = sw_weak_fm.clone();
        let url = base_url.to_string();
        let key = api_key.to_string();
        let all_models_store = Arc::clone(&all_models_fetch);
        if let Some(sw) = sw_weak.upgrade() {
            sw.set_is_fetching_models(true);
            sw.set_fetch_models_status("正在请求模型列表...".into());
        }
        rt_fm.spawn(async move {
            match fetch_models_list(&url, &key).await {
                Ok(models) => {
                    let count = models.len();
                    *all_models_store.lock().unwrap() = models.clone();
                    let slint_models: Vec<slint::SharedString> =
                        models.into_iter().map(Into::into).collect();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(sw) = sw_weak.upgrade() {
                            sw.set_is_fetching_models(false);
                            sw.set_model_presets(slint::ModelRc::new(slint::VecModel::from(
                                slint_models,
                            )));
                            sw.set_fetch_models_status(
                                format!("成功获取到 {count} 个可用模型！").into(),
                            );
                            sw.set_model_dropdown_open(true);
                        }
                    });
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(sw) = sw_weak.upgrade() {
                            sw.set_is_fetching_models(false);
                            sw.set_fetch_models_status(format!("获取失败: {err_msg}").into());
                        }
                    });
                }
            }
        });
    });

    let all_models_filter = Arc::clone(&all_models);
    let sw_weak_filter_model = settings_window.as_weak();
    settings_window.on_filter_models_changed(move |query| {
        if let Some(sw) = sw_weak_filter_model.upgrade() {
            let q = query.to_lowercase();
            let guard = all_models_filter.lock().unwrap();
            let filtered: Vec<slint::SharedString> = guard
                .iter()
                .filter(|m| q.is_empty() || m.to_lowercase().contains(&q))
                .map(|m| m.clone().into())
                .collect();
            sw.set_model_presets(slint::ModelRc::new(slint::VecModel::from(filtered)));
        }
    });

    let sw_weak_test_llm = settings_window.as_weak();
    let rt_test_llm = Arc::clone(&tokio_rt);
    settings_window.on_test_llm_connection(move |base_url, api_key, model| {
        let sw_weak = sw_weak_test_llm.clone();
        let url = base_url.to_string();
        let key = api_key.to_string();
        let model_name = model.to_string();
        if let Some(sw) = sw_weak.upgrade() {
            sw.set_is_testing_llm(true);
            sw.set_llm_test_status("正在测试连通性...".into());
            sw.set_llm_test_is_error(false);
        }
        rt_test_llm.spawn(async move {
            let start = Instant::now();
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default();

            let clean_url = crate::config::normalize_api_base_url(&url);
            let res = if !model_name.trim().is_empty() {
                let chat_url = format!("{clean_url}/chat/completions");
                let body = serde_json::json!({
                    "model": model_name.trim(),
                    "messages": [{"role": "user", "content": "hi"}],
                    "max_tokens": 1
                });
                client
                    .post(&chat_url)
                    .header("Authorization", format!("Bearer {}", key.trim()))
                    .json(&body)
                    .send()
                    .await
            } else {
                let models_url = format!("{clean_url}/models");
                client
                    .get(&models_url)
                    .header("Authorization", format!("Bearer {}", key.trim()))
                    .send()
                    .await
            };

            let elapsed = start.elapsed().as_millis();
            let (status_text, is_err, raw_err) = match res {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        (
                            format!("连接成功 (耗时: {elapsed}ms)"),
                            false,
                            String::new(),
                        )
                    } else {
                        let err_body = resp.text().await.unwrap_or_default();
                        (
                            format!("连接失败 (HTTP {status})"),
                            true,
                            format!("HTTP {status} 响应内容:\n{err_body}"),
                        )
                    }
                }
                Err(e) => (
                    "连接失败: 请求超时或网络错误".to_string(),
                    true,
                    format!("网络错误详情:\n{e}"),
                ),
            };

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(sw) = sw_weak.upgrade() {
                    sw.set_is_testing_llm(false);
                    sw.set_llm_test_status(status_text.into());
                    sw.set_llm_test_is_error(is_err);
                    sw.set_llm_test_raw_error(raw_err.into());
                    sw.set_error_copy_btn_text("复制错误信息".into());
                }
            });
        });
    });
}
