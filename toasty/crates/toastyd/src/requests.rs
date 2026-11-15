//! Conversion at the boundary between agent requests and client-facing forms.
use toasty_backend as agent;
use toasty_broker as broker;

pub enum AgentRequest {
    Permission(agent::Permission),
    Question(agent::Question),
}
impl AgentRequest {
    pub fn key(&self) -> &str {
        match self {
            Self::Permission(p) => &p.key.0,
            Self::Question(q) => &q.key.0,
        }
    }
    pub fn blocking(&self) -> bool {
        match self {
            Self::Permission(_) => true,
            Self::Question(q) => q.blocking,
        }
    }
    pub fn payload(&self) -> broker::RequestPayload {
        match self {
            Self::Permission(p) => {
                let (diff, command) = match &p.detail {
                    Some(agent::Detail::Diff { path, old, new }) => (
                        Some(format!(
                            "{path}\n---\n{}\n+++\n{new}",
                            old.as_deref().unwrap_or("")
                        )),
                        None,
                    ),
                    Some(agent::Detail::Command(cmd)) => (None, Some(cmd.clone())),
                    None => (None, None),
                };
                broker::RequestPayload::Permission(broker::Permission {
                    title: p.title.clone(),
                    kind: p.kind.clone().unwrap_or_else(|| "tool".into()),
                    diff,
                    command,
                    options: p
                        .options
                        .iter()
                        .map(|o| broker::PermissionOption {
                            id: o.id.clone(),
                            label: o.label.clone(),
                            kind: match o.kind {
                                agent::OptionKind::AllowOnce => {
                                    broker::PermissionOptionKind::AllowOnce
                                }
                                agent::OptionKind::AllowAlways => {
                                    broker::PermissionOptionKind::AllowAlways
                                }
                                agent::OptionKind::RejectOnce => {
                                    broker::PermissionOptionKind::RejectOnce
                                }
                                agent::OptionKind::RejectAlways => {
                                    broker::PermissionOptionKind::RejectAlways
                                }
                            },
                        })
                        .collect(),
                })
            }
            Self::Question(q) => broker::RequestPayload::Question(broker::Question {
                message: q.message.clone(),
                blocking: q.blocking,
                form: match &q.form {
                    agent::Form::Url(url) => broker::QuestionForm::Url(url.clone()),
                    agent::Form::Fields(fields) => broker::QuestionForm::Fields(
                        fields
                            .iter()
                            .map(|f| broker::QuestionField {
                                id: f.id.clone(),
                                title: f.title.clone(),
                                description: f.description.clone(),
                                required: f.required,
                                kind: match &f.kind {
                                    agent::FieldKind::Text => broker::FieldKind::Text,
                                    agent::FieldKind::Boolean => broker::FieldKind::Boolean,
                                    agent::FieldKind::Number { integer: true } => {
                                        broker::FieldKind::Integer
                                    }
                                    agent::FieldKind::Number { integer: false } => {
                                        broker::FieldKind::Number
                                    }
                                    agent::FieldKind::Choice(c) => {
                                        broker::FieldKind::Choice(choices(c))
                                    }
                                    agent::FieldKind::MultiChoice(c) => {
                                        broker::FieldKind::MultiChoice(choices(c))
                                    }
                                },
                            })
                            .collect(),
                    ),
                },
            }),
        }
    }
    pub fn answer(self, answer: broker::Answer) -> Result<(), broker::BrokerError> {
        match (self, answer) {
            (Self::Permission(p), broker::Answer::Permission(a)) => p.answer(match a {
                broker::PermissionAnswer::Selected(id) => agent::PermissionAnswer::Selected(id),
                broker::PermissionAnswer::Cancelled => agent::PermissionAnswer::Cancelled,
            }),
            (Self::Question(q), broker::Answer::Question(a)) => q.answer(match a {
                broker::QuestionAnswer::Accept(values) => {
                    agent::QuestionAnswer::Accept(values.into_iter().collect())
                }
                broker::QuestionAnswer::Decline => agent::QuestionAnswer::Decline,
                broker::QuestionAnswer::Cancel => agent::QuestionAnswer::Cancel,
            }),
            _ => return Err(broker::BrokerError::WrongAnswerKind),
        }
        Ok(())
    }
}
fn choices(values: &[agent::Choice]) -> Vec<broker::Choice> {
    values
        .iter()
        .map(|c| broker::Choice {
            value: c.value.clone(),
            label: c.label.clone(),
            description: c.description.clone(),
        })
        .collect()
}
