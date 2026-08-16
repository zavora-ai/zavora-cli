---
name: docx
description: Create, inspect, edit, and verify Microsoft Word DOCX documents while preserving structure and formatting. Use for Word documents, reports, letters, memos, templates, tracked changes, comments, or any task whose primary input or output is a .docx file.
---

# DOCX workflow

1. Inspect the source document and surrounding files before editing.
2. Prefer a connected DOCX MCP tool when available. Otherwise use an existing project library or an installed local document tool; do not silently install dependencies.
3. Preserve styles, section settings, headers, footers, relationships, tracked changes, and comments unless the request requires changing them.
4. Write the result to the requested path without replacing the source unless explicitly requested.
5. Render or reopen the result and verify page flow, tables, images, headings, and links before reporting completion.
6. State clearly when no executable DOCX tool is connected or installed.
