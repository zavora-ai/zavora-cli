---
name: xlsx
description: Create, inspect, edit, analyze, and verify spreadsheet files including XLSX, XLSM, CSV, and TSV. Use for formulas, formatting, tables, charts, financial models, data cleanup, or any task whose primary input or output is a spreadsheet.
---

# Spreadsheet workflow

1. Inspect workbook sheets, used ranges, formulas, named ranges, charts, and existing formatting.
2. Prefer a connected spreadsheet MCP tool. Otherwise use an existing project library or installed workbook tool without silently adding dependencies.
3. Preserve formulas, number formats, merged cells, validation, hidden sheets, and macros unless changes are requested.
4. Use formulas for derived values rather than replacing calculations with constants.
5. Recalculate or reopen the workbook and verify that it contains no formula errors or corrupted relationships.
6. State clearly when no executable spreadsheet tool is connected or installed.
