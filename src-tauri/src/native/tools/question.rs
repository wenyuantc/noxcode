use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct PlanQuestion {
    pub prompt: String,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct PlanQuestionAnswer {
    #[serde(default)]
    pub skipped: bool,
    #[serde(default)]
    pub answers: Vec<String>,
}

const MAX_QUESTIONS: usize = 4;
const MAX_OPTIONS: usize = 6;

#[derive(Deserialize)]
struct AskQuestionArgs {
    questions: Vec<RawQuestion>,
}

#[derive(Deserialize)]
struct RawQuestion {
    prompt: Option<String>,
    #[serde(default)]
    options: Vec<String>,
}

pub fn parse_ask_question_args(arguments: &str) -> Result<Vec<PlanQuestion>, String> {
    let parsed: AskQuestionArgs = serde_json::from_str(arguments)
        .map_err(|_| "AskQuestion 参数必须是 JSON 对象".to_string())?;
    if parsed.questions.is_empty() {
        return Err("AskQuestion.questions 不能为空".to_string());
    }
    if parsed.questions.len() > MAX_QUESTIONS {
        return Err(format!("AskQuestion 一次最多 {MAX_QUESTIONS} 个问题"));
    }
    parsed
        .questions
        .into_iter()
        .map(|item| {
            let prompt = item
                .prompt
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "问题 prompt 不能为空".to_string())?
                .to_string();
            let options: Vec<String> = item
                .options
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect();
            if !options.is_empty() && options.len() < 2 {
                return Err("选项至少 2 个，或不要提供 options".to_string());
            }
            if options.len() > MAX_OPTIONS {
                return Err(format!("每个问题最多 {MAX_OPTIONS} 个选项"));
            }
            Ok(PlanQuestion { prompt, options })
        })
        .collect()
}

pub fn format_ask_question_result(
    questions: &[PlanQuestion],
    answer: &PlanQuestionAnswer,
) -> String {
    if answer.skipped {
        return "用户跳过提问。请自行做合理假设，把假设写入计划，然后输出完整计划；不要再次提问。"
            .to_string();
    }
    let mut lines = vec!["用户回答：".to_string()];
    for (index, question) in questions.iter().enumerate() {
        let reply = answer
            .answers
            .get(index)
            .map(String::as_str)
            .unwrap_or("")
            .trim();
        lines.push(format!("{}. {}", index + 1, question.prompt));
        lines.push(format!(
            "→ {}",
            if reply.is_empty() { "（空）" } else { reply }
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prompt_and_options() {
        let questions = parse_ask_question_args(
            r#"{"questions":[{"prompt":"用哪个入口？","options":["A","B"]}]}"#,
        )
        .expect("parse");
        assert_eq!(questions[0].prompt, "用哪个入口？");
        assert_eq!(questions[0].options, vec!["A", "B"]);
    }

    #[test]
    fn rejects_empty_and_too_many() {
        parse_ask_question_args(r#"{"questions":[]}"#).expect_err("empty");
        parse_ask_question_args(
            r#"{"questions":[{"prompt":"1"},{"prompt":"2"},{"prompt":"3"},{"prompt":"4"},{"prompt":"5"}]}"#,
        )
        .expect_err("too many");
    }

    #[test]
    fn skip_result_tells_model_to_assume() {
        let text = format_ask_question_result(
            &[PlanQuestion {
                prompt: "选哪个？".to_string(),
                options: vec![],
            }],
            &PlanQuestionAnswer {
                skipped: true,
                answers: vec![],
            },
        );
        assert!(text.contains("跳过"));
        assert!(text.contains("不要再次提问"));
    }
}
