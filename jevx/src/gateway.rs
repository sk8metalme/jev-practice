use std::time::Instant;

use async_trait::async_trait;
use serde::Deserialize;

use crate::config::Config;
use crate::error::JevxError;
use crate::types::{Judge, JudgeRequest, JudgeResponse, Usage};

pub struct GatewayJudge {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    timeout: std::time::Duration,
}

impl GatewayJudge {
    pub fn from_config(config: &Config) -> Result<Self, JevxError> {
        let Some(api_key) = config.api_key.clone().filter(|key| !key.trim().is_empty()) else {
            return Err(JevxError::MissingApiKey);
        };
        let client = reqwest::Client::new();
        Ok(Self {
            client,
            endpoint: config.endpoint.clone(),
            api_key,
            timeout: config.timeout,
        })
    }
}

#[derive(Debug, Deserialize)]
struct GatewayPayload {
    answers: Option<Answers>,
    usage: Option<Usage>,
    error: Option<GatewayErrorPayload>,
}

#[derive(Debug, Deserialize)]
struct Answers {
    skill: Option<SkillAnswer>,
}

#[derive(Debug, Deserialize)]
struct SkillAnswer {
    choice: Option<String>,
    probabilities: Option<std::collections::BTreeMap<String, f64>>,
}

#[derive(Debug, Deserialize)]
struct GatewayErrorPayload {
    message: Option<String>,
}

#[async_trait]
impl Judge for GatewayJudge {
    async fn evaluate(&self, request: JudgeRequest) -> Result<JudgeResponse, JevxError> {
        let started = Instant::now();
        let criteria = request
            .candidates
            .iter()
            .map(|candidate| (candidate.id.clone(), candidate.description.clone()))
            .chain([(
                String::from("none"),
                String::from("候補Skillのどれも現在の依頼に適合しない"),
            )])
            .collect::<std::collections::BTreeMap<_, _>>();
        let body = serde_json::json!({
            "model": crate::MODEL_ID,
            "state": request.state,
            "questions": {
                "skill": {
                    "type": "choice",
                    "instructions": "現在の依頼に最も適したSkillを1つ選んでください。適合するSkillがなければnoneを選んでください。",
                    "criteria": criteria
                }
            }
        });
        let response = match self
            .client
            .post(&self.endpoint)
            .timeout(self.timeout)
            .bearer_auth(self.api_key.trim())
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) if error.is_timeout() => return Err(JevxError::Timeout),
            Err(_) => {
                return Err(JevxError::Provider(
                    "Jevへの接続に失敗しました。".to_owned(),
                ));
            }
        };

        let status = response.status();
        let payload: GatewayPayload = response
            .json()
            .await
            .map_err(|_| JevxError::Provider("Jevの応答を読み取れませんでした。".to_owned()))?;
        if !status.is_success() {
            let message = payload
                .error
                .and_then(|error| error.message)
                .unwrap_or_else(|| "Jevの評価に失敗しました。".to_owned());
            return Err(JevxError::Provider(message));
        }
        let Some(answer) = payload.answers.and_then(|answers| answers.skill) else {
            return Err(JevxError::Provider(
                "Jevの応答にskillの回答がありません。".to_owned(),
            ));
        };
        Ok(JudgeResponse {
            choice: answer.choice,
            probabilities: answer.probabilities.unwrap_or_default(),
            response_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
            usage: payload.usage,
        })
    }
}
