use tonk_assess::agent::parse_claude_output;
use tonk_assess::judge::parse_score;
use tonk_assess::types::{JudgeConfig, ProbeFile};

// ── Agent output parsing ────────────────────────────────────────────

const AGENT_SUCCESS_OUTPUT: &str = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":14346,"duration_api_ms":14683,"num_turns":2,"result":"The api-gateway project uses **Vitest** as its test framework (migrated from Jest). Key details:\n\n- `pnpm test` — run vitest\n- `pnpm test:watch` — vitest in watch mode\n- `pnpm test -- --filter auth` — run domain-specific tests\n\nAdditional testing tools:\n- **fast-check** for property-based tests\n- **testcontainers** for integration tests with real PostgreSQL instances","stop_reason":null,"session_id":"267a0b4c-6929-454a-8eb7-58839f035b54","total_cost_usd":0.046569200000000005,"usage":{"input_tokens":4,"cache_creation_input_tokens":436,"cache_read_input_tokens":44676,"output_tokens":282,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":436,"ephemeral_5m_input_tokens":0},"inference_geo":"","iterations":[],"speed":"standard"},"modelUsage":{"claude-sonnet-4-6":{"inputTokens":4,"outputTokens":282,"cacheReadInputTokens":44676,"cacheCreationInputTokens":436,"webSearchRequests":0,"costUSD":0.032133,"contextWindow":200000,"maxOutputTokens":32000},"claude-haiku-4-5-20251001":{"inputTokens":919,"outputTokens":787,"cacheReadInputTokens":43672,"cacheCreationInputTokens":4172,"webSearchRequests":0,"costUSD":0.0144362,"contextWindow":200000,"maxOutputTokens":32000}},"permission_denials":[],"uuid":"9451aacc-1d7b-464e-96df-4f94e63a4d4c"}"#;

#[test]
fn parse_agent_success_output() {
    let metrics =
        parse_claude_output(AGENT_SUCCESS_OUTPUT, 14346, false).expect("should parse successfully");

    assert!(metrics.answer.contains("Vitest"));
    assert!(metrics.answer.contains("migrated from Jest"));
    assert_eq!(metrics.elapsed_ms, 14346);
    assert_eq!(metrics.num_turns, Some(2));
    assert!((metrics.cost_usd.unwrap() - 0.0465692).abs() < 0.0001);
    assert_eq!(metrics.input_tokens, Some(4));
    assert_eq!(metrics.output_tokens, Some(282));
    assert_eq!(metrics.tool_calls, None);
}

#[test]
fn parse_agent_success_output_second_run() {
    let raw = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":20482,"duration_api_ms":22295,"num_turns":2,"result":"The **api-gateway** project uses **Vitest** as its test framework (migrated from Jest).\n\nKey testing details:\n- `pnpm test` — runs vitest\n- **Integration tests** use `testcontainers` for a real Postgres database\n- **Property-based tests** use `fast-check` for parsers and validators\n- `pnpm test:watch` available for watch mode","stop_reason":null,"session_id":"f1f99b0d-7860-4c76-b2af-d89aded47ebf","total_cost_usd":0.05885625,"usage":{"input_tokens":4,"cache_creation_input_tokens":448,"cache_read_input_tokens":44676,"output_tokens":271,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":448,"ephemeral_5m_input_tokens":0},"inference_geo":"","iterations":[],"speed":"standard"},"modelUsage":{"claude-sonnet-4-6":{"inputTokens":4,"outputTokens":271,"cacheReadInputTokens":44676,"cacheCreationInputTokens":448,"webSearchRequests":0,"costUSD":0.031933,"contextWindow":200000,"maxOutputTokens":32000},"claude-haiku-4-5-20251001":{"inputTokens":5051,"outputTokens":1323,"cacheReadInputTokens":72135,"cacheCreationInputTokens":6435,"webSearchRequests":0,"costUSD":0.02692325,"contextWindow":200000,"maxOutputTokens":32000}},"permission_denials":[],"uuid":"ddb05244-cc3f-40bd-bf1f-7ff64a59180e"}"#;

    let metrics = parse_claude_output(raw, 20482, false).expect("should parse successfully");

    assert!(metrics.answer.contains("Vitest"));
    assert_eq!(metrics.num_turns, Some(2));
    assert!((metrics.cost_usd.unwrap() - 0.05885625).abs() < 0.0001);
    assert_eq!(metrics.input_tokens, Some(4));
    assert_eq!(metrics.output_tokens, Some(271));
}

#[test]
fn parse_agent_output_with_null_result() {
    let raw = r#"{"type":"result","subtype":"error_max_turns","is_error":false,"duration_ms":100,"num_turns":10,"result":null,"total_cost_usd":0.01,"usage":{"input_tokens":10,"output_tokens":20}}"#;

    let metrics = parse_claude_output(raw, 100, false).expect("should parse with empty answer");

    assert_eq!(metrics.answer, "");
    assert_eq!(metrics.num_turns, Some(10));
    assert_eq!(metrics.cost_usd, Some(0.01));
}

#[test]
fn parse_agent_output_missing_result_field() {
    let raw = r#"{"type":"result","subtype":"error","is_error":true,"duration_ms":50,"num_turns":0,"total_cost_usd":0.001,"usage":{"input_tokens":5,"output_tokens":0}}"#;

    let metrics = parse_claude_output(raw, 50, false).expect("should parse with empty answer");

    assert_eq!(metrics.answer, "");
    assert_eq!(metrics.cost_usd, Some(0.001));
}

// ── Judge output parsing ────────────────────────────────────────────

#[test]
fn parse_judge_error_max_turns_no_result() {
    let raw = r#"{"type":"result","subtype":"error_max_turns","duration_ms":2802,"duration_api_ms":2747,"is_error":false,"num_turns":2,"stop_reason":null,"session_id":"906fe6ab-df7b-4db2-8e1e-b739ad8d5ddd","total_cost_usd":0.02904175,"usage":{"input_tokens":3,"output_tokens":129},"modelUsage":{},"permission_denials":[],"uuid":"072ae361-a3bf-4ce1-ab16-97412d583ed2","errors":[]}"#;

    let envelope: serde_json::Value = serde_json::from_str(raw).unwrap();
    let result = parse_score(&envelope, false);

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("failed to parse judge score"),
        "error should be descriptive, got: {err_msg}"
    );
}

#[test]
fn parse_judge_direct_json_result() {
    let raw = r#"{"type":"result","subtype":"success","result":"{\"score\":3,\"rationale\":\"Fully correct. Mentions Vitest and migration from Jest.\"}","num_turns":1,"total_cost_usd":0.01}"#;

    let envelope: serde_json::Value = serde_json::from_str(raw).unwrap();
    let score = parse_score(&envelope, false).expect("should parse direct JSON score");

    assert_eq!(score.score, 3);
    assert!(score.rationale.contains("Vitest"));
}

#[test]
fn parse_judge_markdown_wrapped_result() {
    let raw = r#"{"type":"result","subtype":"success","result":"Here is my evaluation:\n\n```json\n{\"score\":2,\"rationale\":\"Mostly correct but missing migration detail.\"}\n```","num_turns":1,"total_cost_usd":0.01}"#;

    let envelope: serde_json::Value = serde_json::from_str(raw).unwrap();
    let score = parse_score(&envelope, false).expect("should parse markdown-wrapped score");

    assert_eq!(score.score, 2);
    assert!(score.rationale.contains("migration"));
}

#[test]
fn parse_judge_json_embedded_in_text() {
    let raw = r#"{"type":"result","subtype":"success","result":"Based on my analysis, the score is: {\"score\":1,\"rationale\":\"Partially correct, only mentions Vitest without jest migration.\"} That's my assessment.","num_turns":1,"total_cost_usd":0.01}"#;

    let envelope: serde_json::Value = serde_json::from_str(raw).unwrap();
    let score = parse_score(&envelope, false).expect("should extract embedded JSON score");

    assert_eq!(score.score, 1);
    assert!(score.rationale.contains("Partially correct"));
}

#[test]
fn parse_judge_score_zero() {
    let raw = r#"{"type":"result","subtype":"success","result":"{\"score\":0,\"rationale\":\"Completely wrong answer.\"}","num_turns":1,"total_cost_usd":0.005}"#;

    let envelope: serde_json::Value = serde_json::from_str(raw).unwrap();
    let score = parse_score(&envelope, false).expect("should parse score 0");

    assert_eq!(score.score, 0);
}

#[test]
fn parse_judge_empty_result_string() {
    let raw = r#"{"type":"result","subtype":"error_max_turns","result":"","num_turns":2,"total_cost_usd":0.02}"#;

    let envelope: serde_json::Value = serde_json::from_str(raw).unwrap();
    let result = parse_score(&envelope, false);

    assert!(result.is_err());
}

#[test]
fn parse_judge_result_with_extra_whitespace() {
    let raw = r#"{"type":"result","subtype":"success","result":"  \n{\"score\":3,\"rationale\":\"Perfect answer.\"}\n  ","num_turns":1,"total_cost_usd":0.01}"#;

    let envelope: serde_json::Value = serde_json::from_str(raw).unwrap();
    let score = parse_score(&envelope, false).expect("should handle whitespace around JSON");

    assert_eq!(score.score, 3);
}

#[test]
fn parse_judge_structured_output_field() {
    let raw = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":3494,"duration_api_ms":3405,"num_turns":2,"result":"","stop_reason":null,"session_id":"ed211acd-5a26-4a7b-a7a3-5909dcd51fbb","total_cost_usd":0.041221999999999995,"usage":{"input_tokens":4,"cache_creation_input_tokens":2664,"cache_read_input_tokens":42904,"output_tokens":124,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":2664,"ephemeral_5m_input_tokens":0},"inference_geo":"","iterations":[],"speed":"standard"},"modelUsage":{"claude-sonnet-4-6":{"inputTokens":4,"outputTokens":124,"cacheReadInputTokens":42904,"cacheCreationInputTokens":2664,"webSearchRequests":0,"costUSD":0.041221999999999995,"contextWindow":200000,"maxOutputTokens":32000}},"permission_denials":[],"structured_output":{"score":3,"rationale":"The answer correctly identifies Vitest as the test framework and mentions the migration from Jest, covering both key facts. The additional details about fast-check, testcontainers, and test commands are supplementary information that doesn't contradict the ground truth."},"uuid":"784f4b01-ab2a-4720-8e25-2f7ad5027a6c"}"#;

    let envelope: serde_json::Value = serde_json::from_str(raw).unwrap();
    let score = parse_score(&envelope, false).expect("should parse structured_output field");

    assert_eq!(score.score, 3);
    assert!(score.rationale.contains("Vitest"));
    assert!(score.rationale.contains("migration from Jest"));
}

#[test]
fn parse_judge_structured_output_takes_priority() {
    let raw = r#"{"type":"result","subtype":"success","result":"{\"score\":1,\"rationale\":\"wrong\"}","structured_output":{"score":3,"rationale":"correct"},"num_turns":1}"#;

    let envelope: serde_json::Value = serde_json::from_str(raw).unwrap();
    let score = parse_score(&envelope, false).expect("structured_output should take priority");

    assert_eq!(score.score, 3);
    assert_eq!(score.rationale, "correct");
}

// ── Probe YAML parsing ──────────────────────────────────────────────

#[test]
fn parse_probe_with_llm_judge() {
    let yaml = r#"
tonk.assess/Probe:
  persona: marcus
  tag: [synthesis]
  prompt: "What CSS approach does Marcus use?"
  judge:
    tonk.assess/Judge:
      llm:
        tonk.assess/LlmJudge:
          ground-truth: "Tailwind everywhere."
          key-fact:
            - Tailwind in dashboard-ui
            - Tailwind in personal-site
  source-file:
    - projects/dashboard-ui/CLAUDE.md
    - projects/personal-site/CLAUDE.md
"#;

    let file: ProbeFile = serde_yaml::from_str(yaml).expect("should parse LLM judge probe");
    let probe = file.probe.into_probe("marcus-synthesis-01".to_string());

    assert_eq!(probe.id, "marcus-synthesis-01");
    assert_eq!(probe.persona, "marcus");
    assert_eq!(probe.tag, vec!["synthesis"]);
    assert_eq!(probe.prompt, "What CSS approach does Marcus use?");
    assert_eq!(probe.source_files.len(), 2);
    assert!(probe.allowed_tools.is_empty());
    assert!(probe.system_prompt.is_none());

    match &probe.judge {
        JudgeConfig::Llm {
            ground_truth,
            key_facts,
            system_prompt,
            model,
        } => {
            assert_eq!(ground_truth, "Tailwind everywhere.");
            assert_eq!(key_facts.len(), 2);
            assert!(system_prompt.is_none());
            assert!(model.is_none());
        }
        _ => panic!("expected LLM judge"),
    }
}

#[test]
fn parse_probe_with_keyword_judge() {
    let yaml = r#"
tonk.assess/Probe:
  persona: marcus
  tag: [lookup]
  prompt: "What test framework does the api-gateway project use?"
  judge:
    tonk.assess/Judge:
      keyword:
        tonk.assess/KeywordJudge:
          keyword:
            - term: vitest
              score: 2
            - term: jest
              score: 1
  source-file:
    - projects/api-gateway/CLAUDE.md
"#;

    let file: ProbeFile = serde_yaml::from_str(yaml).expect("should parse keyword judge probe");
    let probe = file.probe.into_probe("marcus-lookup-01".to_string());

    assert_eq!(probe.id, "marcus-lookup-01");
    assert_eq!(probe.tag, vec!["lookup"]);

    match &probe.judge {
        JudgeConfig::Keyword {
            keywords,
            max_score,
        } => {
            assert_eq!(keywords.len(), 2);
            assert_eq!(keywords[0].term, "vitest");
            assert_eq!(keywords[0].score, 2);
            assert_eq!(keywords[1].term, "jest");
            assert_eq!(keywords[1].score, 1);
            assert!(max_score.is_none());
        }
        _ => panic!("expected Keyword judge"),
    }
}

#[test]
fn parse_probe_with_custom_system_prompt() {
    let yaml = r#"
tonk.assess/Probe:
  persona: marcus
  tag: [inference]
  prompt: "Complex question"
  judge:
    tonk.assess/Judge:
      llm:
        tonk.assess/LlmJudge:
          ground-truth: "The answer"
          system-prompt: "You are a strict technical evaluator."
          model: claude-haiku-4-5-20251001
"#;

    let file: ProbeFile = serde_yaml::from_str(yaml).expect("should parse custom system prompt");
    let probe = file.probe.into_probe("custom-prompt-probe".to_string());

    match &probe.judge {
        JudgeConfig::Llm {
            system_prompt,
            model,
            ..
        } => {
            assert_eq!(
                system_prompt.as_deref(),
                Some("You are a strict technical evaluator.")
            );
            assert_eq!(model.as_deref(), Some("claude-haiku-4-5-20251001"));
        }
        _ => panic!("expected LLM judge"),
    }
}

#[test]
fn parse_probe_with_keyword_max_score() {
    let yaml = r#"
tonk.assess/Probe:
  persona: marcus
  tag: [lookup]
  prompt: "Simple question"
  judge:
    tonk.assess/Judge:
      keyword:
        tonk.assess/KeywordJudge:
          keyword:
            - term: foo
              score: 5
            - term: bar
              score: 3
          max-score: 10
"#;

    let file: ProbeFile = serde_yaml::from_str(yaml).expect("should parse keyword with max-score");
    let probe = file.probe.into_probe("max-score-probe".to_string());

    match &probe.judge {
        JudgeConfig::Keyword {
            keywords,
            max_score,
        } => {
            assert_eq!(keywords.len(), 2);
            assert_eq!(*max_score, Some(10));
        }
        _ => panic!("expected Keyword judge"),
    }
}

#[test]
fn parse_probe_no_source_files() {
    let yaml = r#"
tonk.assess/Probe:
  persona: marcus
  tag: [lookup]
  prompt: "Minimal question"
  judge:
    tonk.assess/Judge:
      keyword:
        tonk.assess/KeywordJudge:
          keyword:
            - term: answer
              score: 1
"#;

    let file: ProbeFile = serde_yaml::from_str(yaml).unwrap();
    let probe = file.probe.into_probe("minimal-probe".to_string());

    assert!(probe.source_files.is_empty());
}

#[test]
fn parse_probe_with_name_and_corpus() {
    let yaml = r#"
tonk.assess/Probe:
  name: "Test framework lookup"
  persona: marcus
  corpus: ../personas/marcus/artifacts
  tag: [lookup]
  prompt: "What test framework?"
  judge:
    tonk.assess/Judge:
      keyword:
        tonk.assess/KeywordJudge:
          keyword:
            - term: vitest
              score: 1
"#;

    let file: ProbeFile = serde_yaml::from_str(yaml).unwrap();
    let probe = file.probe.into_probe("named-probe".to_string());

    assert_eq!(probe.id, "named-probe");
    assert_eq!(probe.name.as_deref(), Some("Test framework lookup"));
    assert_eq!(
        probe.corpus.as_deref(),
        Some("../personas/marcus/artifacts")
    );
}

#[test]
fn parse_probe_with_agent_config() {
    let yaml = r#"
tonk.assess/Probe:
  persona: marcus
  corpus: ../personas/marcus/artifacts
  tag: [lookup]
  prompt: "What test framework?"
  allowed-tool:
    - Bash
  system-prompt: prompts/carry.md
  max-turns: 5
  judge:
    tonk.assess/Judge:
      keyword:
        tonk.assess/KeywordJudge:
          keyword:
            - term: vitest
              score: 1
"#;

    let file: ProbeFile = serde_yaml::from_str(yaml).unwrap();
    let probe = file.probe.into_probe("carry-probe".to_string());

    assert_eq!(probe.allowed_tools, vec!["Bash"]);
    assert_eq!(probe.system_prompt.as_deref(), Some("prompts/carry.md"));
    assert_eq!(probe.max_turns, Some(5));
}

#[test]
fn judge_type_method() {
    let llm = JudgeConfig::Llm {
        ground_truth: String::new(),
        key_facts: vec![],
        system_prompt: None,
        model: None,
    };
    assert_eq!(llm.judge_type(), "llm");

    let kw = JudgeConfig::Keyword {
        keywords: vec![],
        max_score: None,
    };
    assert_eq!(kw.judge_type(), "keyword");
}
