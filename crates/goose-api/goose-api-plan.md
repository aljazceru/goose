# Plan for `goose-api` Review and Improvements

This document outlines the plan to address the user's request regarding `goose-api`'s interaction with `goose-cli`, session sharing, and reported resource exhaustion/memory leaks. All changes will be confined to the `crates/goose-api` crate.

## Summary of Findings

### Session Sharing
*   Both `goose-api` and `goose-cli` leverage the `goose` crate's session management, storing sessions as `.jsonl` files in a common directory (`~/.local/share/goose/sessions` by default).
*   `goose-api` generates a `Uuid` for each new session and returns it. This UUID is used as the session name for file persistence.
*   `goose-cli`'s `session resume` command can accept a session name or path. Therefore, the UUID returned by `goose-api` can be used directly with `goose-cli session --resume --name <UUID>`.

### Resource Exhaustion and Memory Leaks
*   **Primary Suspect: Partial Stream Consumption in `agent.reply`:** In `crates/goose-api/src/handlers.rs`, both `start_session_handler` and `reply_session_handler` only consume the *first* item from the `BoxStream` returned by `agent.reply`. If `agent.reply` produces a stream of multiple messages (common for LLM interactions), the remaining messages and associated resources are not consumed or released, leading to memory accumulation. This is highly likely to be the root cause of single-session resource exhaustion.
*   **Per-Session `Agent` Instances:** `goose-api` creates a new `Agent` instance for each session and stores it in an in-memory `DashMap` (`SESSIONS`). While this provides session isolation, it means more `Agent` instances (each with its own internal state and resources) are held in memory.
*   **Session Cleanup:** `cleanup_expired_sessions()` is called to remove inactive sessions from the `DashMap` after `SESSION_TIMEOUT_SECS` (currently 1 hour). If this timeout is too long, or if `Agent` instances don't fully release resources upon being dropped, memory can accumulate.
*   **LLM Calls for Summarization:** `generate_description` (in `goose::session::storage`) and `agent.summarize_context` (in `goose` crate) involve additional LLM calls, which are resource-intensive operations.
*   **Extension Management:** `Stdio` extensions can spawn external processes. If these processes are not properly terminated when their associated `Agent` is dropped, they could contribute to leaks.

## Detailed Plan

### Phase 1: Address Immediate Resource Leak (Critical)

1.  **Fully Consume `agent.reply` Stream in `crates/goose-api/src/handlers.rs`:**
    *   **Action:** Modify `start_session_handler` and `reply_session_handler` to iterate through the entire `BoxStream<anyhow::Result<Message>>` returned by `agent.reply`. All messages from the stream will be collected and concatenated to form the complete response. This ensures all resources associated with the stream are properly released.

    *   **Mermaid Diagram for Stream Consumption:**
        ```mermaid
        graph TD
            A[Call agent.reply()] --> B{Receive BoxStream<Message>};
            B --> C{Loop: stream.try_next().await};
            C -- Has Message --> D[Append Message to history];
            C -- No More Messages / Error --> E[Process complete response];
            D --> C;
        ```

### Phase 2: Improve Session Sharing (Documentation within `goose-api`)

1.  **Clarify Session ID Usage in `crates/goose-api/README.md`:**
    *   **Action:** Add a clear note or example in the "Session Management" section of `crates/goose-api/README.md` demonstrating that the `session_id` (UUID) returned by the API can be directly used with `goose-cli session --resume --name <UUID>`.

### Phase 3: Investigate and Mitigate Potential Resource Issues (within `goose-api` only)

1.  **Review `ApiSession` and `cleanup_expired_sessions` in `crates/goose-api/src/api_sessions.rs`:**
    *   **Action:** No code change is immediately required.
    *   **Recommendation (for user consideration):** The `SESSION_TIMEOUT_SECS` constant (currently 1 hour) is a critical parameter. If resource issues persist after Phase 1, reducing this timeout (e.g., to 5-15 minutes) would cause inactive `Agent` instances to be dropped more quickly, freeing up their resources. This would be a configuration/tuning step.

2.  **Monitor `generate_description` and `summarize_context` calls:**
    *   **Action:** No direct code change in `goose-api` is possible for the implementation of these functions as they reside in the `goose` crate.
    *   **Recommendation (for user consideration):** These LLM calls add to the overall load. If resource issues are observed, especially during summarization, it might indicate a bottleneck in the LLM provider interaction or the `goose` crate's handling of large contexts.

3.  **Extension Management:**
    *   **Action:** No direct code change in `goose-api` is possible to fix potential leaks within the `goose` crate's `ExtensionManager`.
    *   **Recommendation (for user consideration):** If specific `Stdio` extensions are identified as problematic, the user might need to investigate their implementation or consider if `goose-api` could offer a way to explicitly terminate processes associated with a session's `Agent` when the session expires.