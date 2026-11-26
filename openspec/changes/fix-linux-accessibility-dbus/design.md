## Context

The AccessKit worker drives its asynchronous D-Bus connection with futures-lite on a plain thread. Cargo unifies features of the same zbus package across the accessibility and capture dependency paths. The Tokio feature makes zbus require a reactor there, even if async-io is also enabled.

## Decisions

Enable ashpd's async-io feature with its existing ScreenCast and camera features. The publishing task can await these futures from its existing Tokio runtime. The accessibility worker keeps its existing executor. Cover the combined application's dependency graph with a Linux integration test that opens a missing socket from the worker's executor.

## Verification

Compare the delivered application and the fixed build using the same virtual display, accessibility registry and session bus. Exercise the application publishing path separately and identify the compositor, Portal service and any controlled response in the evidence.

AI 撰写：gpt-6.2-astra
