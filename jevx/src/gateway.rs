use std::collections::BTreeMap;
use std::time::Instant;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::{Duration, sleep};

use crate::Config;
use crate::decision::{
    DecisionJudge, DecisionRequest, DecisionResponse, QuestionSpec, TypedAnswer,
};
use crate::error::JevxError;
use crate::redaction::redact;
use crate::types::{Judge, JudgeEvaluation, JudgeRequest, JudgeResponse, Usage};

pub struct GatewayJudge {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    timeout: Duration,
    max_retries: usize,
    retry_backoff: Duration,
}

impl GatewayJudge {
    pub fn from_config(config: &Config) -> Result<Self, JevxError> {
        let Some(api_key) = config.api_key.clone().filter(|key| !key.trim().is_empty()) else {
            return Err(JevxError::MissingApiKey);
        };
        Ok(Self {
            client: reqwest::Client::new(),
            endpoint: config.endpoint.clone(),
            api_key,
            timeout: config.timeout,
            max_retries: config.max_retries,
            retry_backoff: Duration::from_millis(config.retry_backoff_ms),
        })
    }

    async fn evaluate_request(
        &self,
        request: DecisionRequest,
    ) -> Result<DecisionResponse, JevxError> {
        let started = Instant::now();
        let questions = request.questions.clone();
        for question in questions.values() {
            question.validate()?;
        }
        let body = json!({
            "model": crate::MODEL_ID,
            "state": request.state,
            "questions": questions,
        });
        // `timeout` はリトライを含む呼び出し全体の予算。Hook（5秒）の中でも上限を超えない。
        let deadline = started + self.timeout;
        let mut attempts = 0_u32;
        loop {
            attempts += 1;
            let remaining = deadline.saturating_duration_since(Instant::now());
            let response = match self
                .client
                .post(&self.endpoint)
                .timeout(remaining.max(Duration::from_millis(1)))
                .bearer_auth(self.api_key.trim())
                .json(&body)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) if error.is_timeout() => {
                    return Err(JevxError::TimeoutWithMetrics {
                        calls: attempts,
                        retries: attempts.saturating_sub(1),
                        response_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                    });
                }
                Err(_) => {
                    return Err(JevxError::Provider(
                        "Jevへの接続に失敗しました。".to_owned(),
                    ));
                }
            };
            let status = response.status();
            let text = response.text().await.map_err(|_| {
                provider_failure(
                    "Jevの応答を読み取れませんでした。".to_owned(),
                    &started,
                    attempts,
                )
            })?;

            if is_retryable(status) && (attempts as usize) <= self.max_retries {
                let backoff = self
                    .retry_backoff
                    .saturating_mul(2_u32.saturating_pow(attempts - 1));
                if Instant::now() + backoff < deadline {
                    sleep(backoff).await;
                    continue;
                }
            }
            let payload: GatewayPayload = serde_json::from_str(&text).map_err(|_| {
                provider_failure(
                    "Jevの応答を読み取れませんでした。".to_owned(),
                    &started,
                    attempts,
                )
            })?;
            if !status.is_success() {
                let message = payload
                    .error
                    .and_then(|error| error.message)
                    .map(|message| redact(&message))
                    .unwrap_or_else(|| "Jevの評価に失敗しました。".to_owned());
                return Err(provider_failure(message, &started, attempts));
            }

            let raw_answers = payload.answers.unwrap_or_default();
            let mut answers = BTreeMap::new();
            for (question_id, question) in &questions {
                if let Some(raw_answer) = raw_answers.get(question_id) {
                    answers.insert(
                        question_id.clone(),
                        parse_typed_answer(question, raw_answer).map_err(|error| {
                            provider_failure(error.to_string(), &started, attempts)
                        })?,
                    );
                }
            }
            return Ok(DecisionResponse {
                answers,
                response_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                usage: payload.usage,
                calls: attempts,
                retries: attempts.saturating_sub(1),
            });
        }
    }
}

#[derive(Debug, Deserialize)]
struct GatewayPayload {
    answers: Option<BTreeMap<String, Value>>,
    usage: Option<Usage>,
    error: Option<GatewayErrorPayload>,
}

#[derive(Debug, Deserialize)]
struct GatewayErrorPayload {
    message: Option<String>,
}

#[async_trait]
impl DecisionJudge for GatewayJudge {
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResponse, JevxError> {
        self.evaluate_request(request).await
    }
}

#[async_trait]
impl Judge for GatewayJudge {
    async fn evaluate(&self, request: JudgeRequest) -> Result<JudgeResponse, JevxError> {
        Ok(self.evaluate_legacy_request(request).await?.response)
    }

    async fn evaluate_with_metrics(
        &self,
        request: JudgeRequest,
    ) -> Result<JudgeEvaluation, JevxError> {
        self.evaluate_legacy_request(request).await
    }
}

impl GatewayJudge {
    async fn evaluate_legacy_request(
        &self,
        request: JudgeRequest,
    ) -> Result<JudgeEvaluation, JevxError> {
        let criteria = request
            .candidates
            .iter()
            .map(|candidate| (candidate.id.clone(), candidate.description.clone()))
            .chain([(
                String::from("none"),
                String::from("候補Skillのどれも現在の依頼に適合しない"),
            )])
            .collect::<BTreeMap<_, _>>();
        let response = self
            .evaluate_request(DecisionRequest {
                state: request.state,
                questions: BTreeMap::from([(
                    "skill".to_owned(),
                    QuestionSpec::Choice {
                        instructions: "現在の依頼に最も適したSkillを1つ選んでください。適合するSkillがなければnoneを選んでください。".to_owned(),
                        criteria,
                    },
                )]),
            })
            .await?;
        let Some(TypedAnswer::Choice {
            choice,
            probabilities,
            ..
        }) = response.answers.get("skill").cloned()
        else {
            return Err(JevxError::Provider(
                "Jevの応答にskillの回答がありません。".to_owned(),
            ));
        };
        let calls = response.calls.max(1);
        let retries = response.retries;
        let response = JudgeResponse {
            choice: Some(choice),
            probabilities,
            response_ms: response.response_ms,
            usage: response.usage,
        };
        Ok(JudgeEvaluation {
            response,
            calls,
            retries,
        })
    }
}

fn parse_typed_answer(question: &QuestionSpec, value: &Value) -> Result<TypedAnswer, JevxError> {
    match question {
        QuestionSpec::Choice { .. } => {
            let choice = value
                .get("choice")
                .and_then(Value::as_str)
                .ok_or_else(|| JevxError::Provider("Jevのchoice回答が不正です。".to_owned()))?
                .to_owned();
            let probabilities = parse_probabilities(value)?;
            Ok(TypedAnswer::Choice {
                choice,
                probabilities,
                confidence: value.get("confidence").and_then(Value::as_f64),
            })
        }
        QuestionSpec::Score { .. } => {
            let score = value
                .get("score")
                .and_then(Value::as_f64)
                .ok_or_else(|| JevxError::Provider("Jevのscore回答が不正です。".to_owned()))?;
            let probabilities = parse_probabilities(value)?;
            let confidence = value
                .get("confidence")
                .and_then(Value::as_f64)
                .ok_or_else(|| JevxError::Provider("Jevのconfidence回答が不正です。".to_owned()))?;
            Ok(TypedAnswer::Score {
                score,
                probabilities,
                confidence,
            })
        }
        QuestionSpec::Predicate { .. } => {
            let noul = value
                .get("noul")
                .and_then(Value::as_f64)
                .ok_or_else(|| JevxError::Provider("Jevのnoul回答が不正です。".to_owned()))?;
            Ok(TypedAnswer::Predicate { noul })
        }
    }
}

fn parse_probabilities(value: &Value) -> Result<BTreeMap<String, f64>, JevxError> {
    value
        .get("probabilities")
        .and_then(Value::as_object)
        .ok_or_else(|| JevxError::Provider("Jevのprobabilities回答が不正です。".to_owned()))?
        .iter()
        .map(|(key, value)| {
            value
                .as_f64()
                .map(|probability| (key.clone(), probability))
                .ok_or_else(|| JevxError::Provider("Jevのprobabilities回答が不正です。".to_owned()))
        })
        .collect()
}

fn provider_failure(message: String, started: &Instant, calls: u32) -> JevxError {
    JevxError::ProviderWithMetrics {
        message,
        calls,
        retries: calls.saturating_sub(1),
        response_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
    }
}

fn is_retryable(status: reqwest::StatusCode) -> bool {
    status.as_u16() == 429 || status.as_u16() == 529
}
