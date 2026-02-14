# Retrieval Abstraction

Date: 2026-02-14

## Purpose

Provide a pluggable retrieval interface that can augment prompts without coupling retrieval implementation to agent/runtime code.

## Interface

Core types implemented in `/Users/jameskaranja/Developer/projects/zavora-cli/src/main.rs`:

- `trait RetrievalService`
- `struct RetrievedChunk`
- `DisabledRetrievalService`
- `LocalFileRetrievalService`

`RetrievalService` contract:
- `backend_name()` for observability/logging
- `retrieve(query, max_chunks)` for returning ranked context chunks

## Backends

- `disabled`:
  - Returns no chunks
  - Preserves existing behavior with no prompt changes

- `local`:
  - Loads a local text file
  - Splits by paragraph blocks
  - Scores chunks by term overlap against the query
  - Returns top `N` chunks

- `semantic` (feature-gated):
  - Enabled with Cargo feature `semantic-search`
  - Loads a local text file
  - Uses semantic similarity ranking (Jaro-Winkler + lexical boost)
  - Returns top `N` chunks

## Integration Points

- Non-interactive commands:
  - `ask`
  - `workflow`
  - `release-plan`

  These use `run_prompt_with_retrieval`, which enriches prompts via `augment_prompt_with_retrieval` before ADK runner execution.

- Interactive command:
  - `chat`

  Each turn uses `run_prompt_streaming_with_retrieval`, applying retrieval augmentation per user message.

## Configuration

CLI/env:
- `--retrieval-backend` / `ZAVORA_RETRIEVAL_BACKEND` (`disabled|local|semantic`)
- `--retrieval-doc-path` / `ZAVORA_RETRIEVAL_DOC_PATH`
- `--retrieval-max-chunks` / `ZAVORA_RETRIEVAL_MAX_CHUNKS`

Profile fields:
- `retrieval_backend`
- `retrieval_doc_path`
- `retrieval_max_chunks`

Feature flag:
- `semantic-search` enables `semantic` backend support

## Safety and Fallback

- Retrieval is disabled by default.
- If retrieval backend is `local` without a doc path, command fails with actionable diagnostics.
- If retrieval backend is `semantic` without feature `semantic-search`, command fails with actionable diagnostics.
- If retrieval returns no matches, prompt is unchanged.
