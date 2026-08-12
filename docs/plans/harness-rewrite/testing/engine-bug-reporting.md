# Agent Arena Engine Bug Reporting

This procedure applies when harness work finds a defect in the Agent Arena engine or MCP gateway.

## Target repository

Report confirmed defects to `Kadajett/agentArena` on GitHub.

Do not report a defect only because the harness received an unexpected result. First determine whether the cause is in the harness, the documented contract, the MCP gateway, or the game engine.

## Confirmation rules

A report must have these items:

1. A repeatable production or local-backend failure.
2. The expected behavior from code, schema, or tool documentation.
3. The actual behavior.
4. A minimal sequence of MCP calls or game actions.
5. A timestamp and environment name.
6. A sanitized trace or captured fixture.
7. A statement that explains why the harness is not the cause.

If the result is intermittent, include the number of attempts and failures.

## Issue content

Use this structure:

```text
Title

Summary
Environment
Expected behavior
Actual behavior
Steps to reproduce
Frequency
Impact on the harness
Evidence
Possible source location
Regression test suggestion
```

Use exact tool names and exact field names. Include the backend error code when one exists. Link a harness regression fixture when the fixture does not contain private data.

## Safety

Never include an API key, authorization header, MCP session ID, backend agent ID, private dialogue, or raw production payload in a GitHub issue.

Do not include another player's private data. Use stable harness character names only when the name is relevant to reproduction.

## Follow-up

Add the issue URL to the applicable phase document and regression fixture. Keep the harness compatibility behavior explicit until the engine fix is deployed and verified in production.
