use serde::{Deserialize, Serialize};

// ── Judge types (YAML shapes) ───────────────────────────────────────

/// The `tonk.assess/Judge:` wrapper in YAML — has optional llm/keyword fields.
#[derive(Debug, Clone, Deserialize)]
pub struct JudgeSpecFile {
    #[serde(rename = "tonk.assess/Judge")]
    pub judge: JudgeSpecBody,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JudgeSpecBody {
    pub llm: Option<LlmJudgeFile>,
    pub keyword: Option<KeywordJudgeFile>,
}

/// The `tonk.assess/LlmJudge:` wrapper in YAML.
#[derive(Debug, Clone, Deserialize)]
pub struct LlmJudgeFile {
    #[serde(rename = "tonk.assess/LlmJudge")]
    pub body: LlmJudgeBody,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LlmJudgeBody {
    pub ground_truth: String,
    #[serde(default)]
    pub key_fact: Vec<String>,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
}

/// The `tonk.assess/KeywordJudge:` wrapper in YAML.
#[derive(Debug, Clone, Deserialize)]
pub struct KeywordJudgeFile {
    #[serde(rename = "tonk.assess/KeywordJudge")]
    pub body: KeywordJudgeBody,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct KeywordJudgeBody {
    pub keyword: Vec<ScoredKeywordBody>,
    pub max_score: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScoredKeywordBody {
    pub term: String,
    pub score: u32,
}

// ── Probe (YAML shapes) ────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ProbeFile {
    #[serde(rename = "tonk.assess/Probe")]
    pub probe: ProbeBody,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ProbeBody {
    pub name: Option<String>,
    pub persona: String,
    #[serde(default)]
    pub tag: Vec<String>,
    pub prompt: String,
    pub judge: JudgeSpecFile,
    #[serde(default)]
    pub source_file: Vec<String>,
    pub corpus: Option<String>,
    #[serde(default)]
    pub allowed_tool: Vec<String>,
    pub system_prompt: Option<String>,
    pub max_turns: Option<u32>,
    pub mcp_config: Option<String>,
}

// ── Probe (internal) ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probe {
    pub id: String,
    pub name: Option<String>,
    pub persona: String,
    pub tag: Vec<String>,
    pub prompt: String,
    pub judge: JudgeConfig,
    pub source_files: Vec<String>,
    pub corpus: Option<String>,
    pub allowed_tools: Vec<String>,
    pub system_prompt: Option<String>,
    pub max_turns: Option<u32>,
    pub mcp_config: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JudgeConfig {
    Llm {
        ground_truth: String,
        key_facts: Vec<String>,
        system_prompt: Option<String>,
        model: Option<String>,
    },
    Keyword {
        keywords: Vec<ScoredKeyword>,
        max_score: Option<u32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredKeyword {
    pub term: String,
    pub score: u32,
}

impl JudgeConfig {
    pub fn judge_type(&self) -> &'static str {
        match self {
            JudgeConfig::Llm { .. } => "llm",
            JudgeConfig::Keyword { .. } => "keyword",
        }
    }
}

impl ProbeBody {
    /// Convert to internal Probe, using the given id (derived from filename).
    pub fn into_probe(self, id: String) -> Probe {
        let judge_body = self.judge.judge;
        let judge = if let Some(llm) = judge_body.llm {
            JudgeConfig::Llm {
                ground_truth: llm.body.ground_truth,
                key_facts: llm.body.key_fact,
                system_prompt: llm.body.system_prompt,
                model: llm.body.model,
            }
        } else if let Some(kw) = judge_body.keyword {
            JudgeConfig::Keyword {
                keywords: kw
                    .body
                    .keyword
                    .into_iter()
                    .map(|k| ScoredKeyword {
                        term: k.term,
                        score: k.score,
                    })
                    .collect(),
                max_score: kw.body.max_score,
            }
        } else {
            JudgeConfig::Llm {
                ground_truth: String::new(),
                key_facts: Vec::new(),
                system_prompt: None,
                model: None,
            }
        };

        Probe {
            id,
            name: self.name,
            persona: self.persona,
            tag: self.tag,
            prompt: self.prompt,
            judge,
            source_files: self.source_file,
            corpus: self.corpus,
            allowed_tools: self.allowed_tool,
            system_prompt: self.system_prompt,
            max_turns: self.max_turns,
            mcp_config: self.mcp_config,
        }
    }
}

// ── Run / scoring types ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMetrics {
    pub answer: String,
    pub elapsed_ms: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub tool_calls: Option<u64>,
    pub cost_usd: Option<f64>,
    pub num_turns: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Score {
    pub score: u8,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredRun {
    pub metrics: RunMetrics,
    pub score: Score,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub probe_id: String,
    pub persona: String,
    pub tag: Vec<String>,
    pub prompt: String,
    pub run: ScoredRun,
}
