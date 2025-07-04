use axum::{Json, response::IntoResponse};
use serde::Serialize;

use crate::routes::ApiAuth;

#[derive(Serialize)]
pub struct SelfResponse {
    pub id: i32,
    pub github_login: Option<String>,
    pub github_id: Option<i64>,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
    pub github_email: Option<String>,
    pub telegram_chat_id: Option<i64>,
}

pub async fn user_self(ApiAuth(user): ApiAuth) -> impl IntoResponse {
    (
        [("Cache-Control", "private, no-store")],
        Json(SelfResponse {
            id: user.id,
            github_login: user.github_login,
            github_id: user.github_id,
            github_name: user.github_name,
            github_avatar_url: user.github_avatar_url,
            github_email: user.github_email,
            telegram_chat_id: user.telegram_chat_id,
        }),
    )
}
