# Resume AI Agent

The AI agent can analyze, modify, and compile user resumes through a set of specialized tools. This enables an AI-driven workflow where the agent maintains resume quality on behalf of the user.

## Agent Tools

### Resume Tools

Registered in `crates/workers/src/tools/services/resume_tools.rs`.

#### `list_resumes`

List all resumes with optional source filter.

**Parameters:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `source` | string | No | Filter: `manual`, `ai_generated`, or `optimized` |

**Returns:** Array of resume summaries (id, title, source, tags, version_tag).

#### `get_resume_content`

Retrieve full resume content by ID.

**Parameters:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `resume_id` | string | Yes | UUID of the resume |

**Returns:** Complete resume fields including content text.

#### `analyze_resume`

Analyze a resume and produce an optimization report. Uses `ResumeAnalyzerAgent` with a specialized system prompt for professional resume consulting.

**Parameters:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `resume_id` | string | Yes | UUID of the resume to analyze |

**Analysis dimensions:**

1. **Content completeness** — contact info, work experience, education, skills
2. **Wording quality** — action verbs, quantified achievements, clarity
3. **Structure and format** — section organization, length, readability
4. **ATS compatibility** — keyword coverage, format simplicity
5. **Job match** — skill alignment with target job (when `target_job_id` is set)

### Typst Tools

Registered in `crates/workers/src/tools/services/typst_tools.rs`.

#### `list_typst_projects`

List all Typst projects.

**Parameters:** None.

**Returns:** Array of projects (id, name, main_file, git_url).

#### `list_typst_files`

List all files in a Typst project.

**Parameters:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `project_id` | string | Yes | UUID of the project |

**Returns:** Array of file paths.

#### `read_typst_file`

Read file content from a Typst project.

**Parameters:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `project_id` | string | Yes | UUID of the project |
| `file_path` | string | Yes | Relative file path (e.g., `main.typ`) |

**Returns:** Full file text content.

#### `update_typst_file`

Modify file content in a Typst project.

**Parameters:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `project_id` | string | Yes | UUID of the project |
| `file_path` | string | Yes | Relative file path |
| `content` | string | Yes | New file content |

**Returns:** Update confirmation.

#### `compile_typst_project`

Compile a Typst project to PDF.

**Parameters:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `project_id` | string | Yes | UUID of the project |

**Returns:** Compilation result (render_id, page_count, file_size, source_hash).

## AI-Driven Workflow

The intended workflow for resume maintenance:

```mermaid
sequenceDiagram
    participant User
    participant Agent as AI Agent
    participant Typst as TypstService
    participant S3

    User->>Agent: "Optimize my resume for the SWE role at Google"
    Agent->>Agent: list_typst_projects
    Agent->>Agent: list_typst_files (find project files)
    Agent->>Agent: read_typst_file (read main.typ)
    Agent->>Agent: analyze_resume (multi-dimensional analysis)
    Agent->>Agent: update_typst_file (apply improvements)
    Agent->>Typst: compile_typst_project
    Typst->>S3: Upload PDF
    Typst-->>Agent: render result (pages, size)
    Agent-->>User: "Done. Changed X, Y, Z. New PDF ready."
    User->>User: Review in editor / download PDF
```

### Example Conversation

```
User: 帮我优化简历中的工作经验部分，加上量化数据

Agent: 让我先查看你的简历项目。
→ [calls list_typst_projects]
→ [calls list_typst_files for project "my-resume"]
→ [calls read_typst_file for "main.typ"]

Agent: 我已经阅读了你的简历，发现以下可优化的点：
1. "负责后端开发" → 缺少量化成果
2. "优化系统性能" → 未指明提升幅度
3. "参与项目管理" → 缺少团队规模和项目范围

让我帮你修改：
→ [calls update_typst_file with improved content]
→ [calls compile_typst_project]

Agent: 已完成修改并重新编译。主要改动：
- "负责后端开发" → "主导后端微服务架构设计，支撑日均 500 万请求"
- "优化系统性能" → "将 API 响应时间从 800ms 降至 120ms（提升 85%）"
- "参与项目管理" → "带领 6 人团队交付 3 个季度 OKR 项目"

新 PDF 已生成（2 页，42KB），可在编辑器中预览。
```

## Implementation Files

| File | Purpose |
|------|---------|
| `crates/workers/src/tools/services/resume_tools.rs` | Resume agent tools |
| `crates/workers/src/tools/services/typst_tools.rs` | Typst agent tools |
| `crates/ai/src/agents/resume_analyzer.rs` | Resume analysis AI agent |
| `crates/workers/src/worker_state.rs` | Tool registration |
